//! Coastline and landforms — the *placed* half of generation.
//!
//! Design 02 §2.2 is explicit about the method: "the generator should place
//! these as **features** and then let noise decorate them, rather than hoping
//! features emerge from octaves. A blurred multi-octave field with a radial bias
//! produces blobs; blobs produce no passes, no corridors, and no decisions."
//!
//! So nothing here is a threshold on noise. A ridge is a spine with a crest
//! profile and a named set of passes. A plateau is a disc with a flat top and a
//! short flank. A bay is a lobe walked inward from a chosen point on the
//! perimeter. Noise is the last term in the sum and is worth less than half a
//! band — it roughens what was placed and never decides where anything is.

use rand::rngs::StdRng;
use rand::Rng;

use crate::features::Surface;
use crate::options::MapGenOptions;

use super::field::{salt, stream, Canvas, Grain, TOP_BAND};

/// Distance from every cell to the nearest seed, carrying that seed's payload.
///
/// 3-4 chamfer: orthogonal steps cost 3 and diagonal 4, so the result is within
/// a few percent of Euclidean instead of the diamond a Manhattan sweep gives —
/// which matters, because a diamond-section ridge reads as a zigzag.
fn chamfer(w: usize, h: usize, seeds: &[(usize, f32)]) -> (Vec<f32>, Vec<f32>) {
    const FAR: i32 = 1 << 20;
    let mut dist = vec![FAR; w * h];
    let mut payload = vec![0.0f32; w * h];
    for &(index, value) in seeds {
        if index < dist.len() {
            dist[index] = 0;
            payload[index] = value;
        }
    }

    let relax = |dist: &mut Vec<i32>, payload: &mut Vec<f32>, i: usize, j: usize, cost: i32| {
        let candidate = dist[j] + cost;
        if candidate < dist[i] {
            dist[i] = candidate;
            payload[i] = payload[j];
        }
    };

    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if x > 0 {
                relax(&mut dist, &mut payload, i, i - 1, 3);
            }
            if y > 0 {
                relax(&mut dist, &mut payload, i, i - w, 3);
                if x > 0 {
                    relax(&mut dist, &mut payload, i, i - w - 1, 4);
                }
                if x + 1 < w {
                    relax(&mut dist, &mut payload, i, i - w + 1, 4);
                }
            }
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            if x + 1 < w {
                relax(&mut dist, &mut payload, i, i + 1, 3);
            }
            if y + 1 < h {
                relax(&mut dist, &mut payload, i, i + w, 3);
                if x + 1 < w {
                    relax(&mut dist, &mut payload, i, i + w + 1, 4);
                }
                if x > 0 {
                    relax(&mut dist, &mut payload, i, i + w - 1, 4);
                }
            }
        }
    }

    (
        dist.into_iter().map(|d| d as f32 / 3.0).collect(),
        payload,
    )
}

/// Rasterise a polyline into `(cell, arc position)` seeds for [`chamfer`].
fn spine_seeds(w: usize, h: usize, points: &[(f32, f32)]) -> Vec<(usize, f32)> {
    let mut seeds = Vec::with_capacity(points.len() * 2);
    let last = (points.len().max(2) - 1) as f32;
    for (i, &(px, py)) in points.iter().enumerate() {
        let x = px.round() as i32;
        let y = py.round() as i32;
        if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
            continue;
        }
        seeds.push((y as usize * w + x as usize, i as f32 / last));
    }
    seeds
}

/// Walk a wandering line across the map from `start` in `heading`.
///
/// Spines are launched from **outside** the frame so they arrive at the map edge
/// already committed to a direction — a ridge that begins in the middle of the
/// map is a lump, not a barrier. The walk therefore only stops once it has been
/// inside and come out the far side.
fn walk(
    w: usize,
    h: usize,
    start: (f32, f32),
    heading: f32,
    step: f32,
    wander: f32,
    rng: &mut StdRng,
) -> Vec<(f32, f32)> {
    let margin = step * 1.5;
    let inside = |p: (f32, f32)| {
        p.0 >= -margin && p.1 >= -margin && p.0 <= w as f32 + margin && p.1 <= h as f32 + margin
    };

    let mut points = vec![start];
    let mut p = start;
    let mut angle = heading;
    let mut entered = inside(start);
    // Enough steps to arrive from off-map and cross, capped so a pathological
    // turn cannot spin forever.
    let limit = ((w + h) as f32 * 2.0 / step) as usize + 8;
    for _ in 0..limit {
        angle += rng.gen_range(-wander..wander);
        p = (p.0 + angle.cos() * step, p.1 + angle.sin() * step);
        points.push(p);
        if inside(p) {
            entered = true;
        } else if entered {
            break;
        }
    }
    points
}

/// Where a ray from the map centre at `angle` leaves the frame.
fn border_point(w: usize, h: usize, angle: f32) -> (f32, f32) {
    let cx = (w - 1) as f32 * 0.5;
    let cy = (h - 1) as f32 * 0.5;
    let (dx, dy) = (angle.cos(), angle.sin());
    let tx = if dx.abs() > 1e-4 {
        cx / dx.abs()
    } else {
        f32::MAX
    };
    let ty = if dy.abs() > 1e-4 {
        cy / dy.abs()
    } else {
        f32::MAX
    };
    let t = tx.min(ty);
    (cx + dx * t, cy + dy * t)
}

// ---------------------------------------------------------------------------
// Coast
// ---------------------------------------------------------------------------

/// Cut this map's sea, if it has one at all.
///
/// **A Rail Town map is inland countryside by default.** Playtest replaced brief
/// 02 §2.1's 6–12% sea with 0–4%, most maps none: the reference is Locomotion and
/// RCT, where the constraint is the shape of the ground rather than a coastline
/// hemming the player in. So there is no edge bias here — that bias is exactly
/// what made every map an island — and a landlocked map's land runs to the
/// border, portals and all.
///
/// Where a coast is rolled it is a single inlet walked in from one edge, sized by
/// bisection to the sea target. That gives a harbour and a shoreline to look at
/// without spending the frame on open blue.
///
/// Returns whether this map ended up with a coast.
pub(crate) fn carve_sea(canvas: &mut Canvas, seed: u64, options: MapGenOptions, scale: f32) -> bool {
    let w = canvas.w;
    let h = canvas.h;
    let short = w.min(h) as f32;
    if !options.is_coastal(seed) || short < 24.0 || options.water.bays() == 0 {
        return false;
    }

    let mut rng = stream(seed, salt::COAST);
    let inlets = ((options.water.bays() as f32) * scale).round().clamp(1.0, 3.0) as usize;
    let target = options.water.sea_target() / 100.0 * canvas.len() as f32;

    // Each inlet is a lobe walked inward from a mouth on the border. Sharing one
    // heading keeps them on the same side of the map, so the result reads as a
    // coast the land meets rather than as lakes that happen to touch the edge.
    let coast = rng.gen_range(0.0..std::f32::consts::TAU);
    let mut lobes: Vec<(Vec<(f32, f32)>, f32)> = Vec::new();
    for _ in 0..inlets {
        let angle = coast + rng.gen_range(-0.5..0.5);
        let mouth = border_point(w, h, angle);
        let inward = angle + std::f32::consts::PI;
        let points = walk(w, h, mouth, inward, 1.5, 0.20, &mut rng);
        let reach = short * rng.gen_range(0.14..0.26);
        let steps = ((reach / 1.5) as usize).max(3).min(points.len());
        lobes.push((points[..steps].to_vec(), rng.gen_range(2.2..3.4)));
    }

    let mark = |canvas: &Canvas, gain: f32, out: &mut [bool]| {
        out.fill(false);
        for (points, mouth_radius) in &lobes {
            let steps = points.len().max(1);
            for (k, &(px, py)) in points.iter().enumerate() {
                // Taper from a wide mouth to a narrow head, so the water reads as
                // an inlet and the land either side reads as a headland.
                let t = k as f32 / steps as f32;
                let radius = mouth_radius * gain * (1.0 - t * 0.7);
                if radius <= 0.0 {
                    continue;
                }
                let r = radius.ceil() as i32;
                let (bx, by) = (px.round() as i32, py.round() as i32);
                for dy in -r..=r {
                    for dx in -r..=r {
                        if (dx * dx + dy * dy) as f32 > radius * radius {
                            continue;
                        }
                        if let Some(index) = canvas.idx(bx + dx, by + dy) {
                            out[index] = true;
                        }
                    }
                }
            }
        }
    };

    // Solve the lobe width for the sea share.
    let mut scratch = vec![false; canvas.len()];
    let mut lo = 0.05f32;
    let mut hi = 3.0f32;
    for _ in 0..12 {
        let mid = 0.5 * (lo + hi);
        mark(canvas, mid, &mut scratch);
        if (scratch.iter().filter(|s| **s).count() as f32) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    mark(canvas, 0.5 * (lo + hi), &mut scratch);
    let mut any = false;
    for (index, wet) in scratch.iter().enumerate() {
        if *wet {
            canvas.surface[index] = Surface::Sea;
            any = true;
        }
    }
    any
}

// ---------------------------------------------------------------------------
// Landforms
// ---------------------------------------------------------------------------

/// A ridge as the design describes it: a continuous barrier whose crest is high
/// enough to be rock, broken at a named handful of saddles.
struct Ridge {
    points: Vec<(f32, f32)>,
    half_width: f32,
    crest: f32,
    /// Arc positions, in `0..1`, where the crest drops to a buildable saddle.
    passes: Vec<f32>,
    /// Half-width of a saddle, in arc units.
    notch: f32,
}

impl Ridge {
    /// How strongly a pass claims this arc position: 1 at a saddle's centre,
    /// easing to 0 over the notch. A cosine, so the saddle has shoulders rather
    /// than being a slot cut in a wall.
    fn notch_at(&self, arc: f32) -> f32 {
        let mut weight = 0.0f32;
        for &pass in &self.passes {
            let d = (arc - pass).abs() / self.notch;
            if d < 1.0 {
                weight = weight.max(0.5 * (1.0 + (std::f32::consts::PI * d).cos()));
            }
        }
        weight
    }
}

/// How far a ridge spine keeps clear of open sea, in tiles. See [`add_ridges`].
const SEA_STANDOFF: u16 = 10;

/// Half-width of a saddle, in spine steps.
///
/// Arc position is carried by [`chamfer`] from the nearest spine point, so it
/// arrives quantised to whole steps: a notch narrower than a step or so can miss
/// every cell on the spine and cut nothing at all. At 1.6 the point nearest a
/// designed pass always lands inside the flat of the col, whatever fraction of a
/// step the pass itself sits at.
const NOTCH_STEPS: f32 = 1.6;

/// How sharply the saddle's flat bottom gives way to its shoulders.
const NOTCH_SHOULDER: f32 = 1.6;

/// Elevation a saddle is held down to, in bands.
///
/// Band 3: hills, buildable, and two clear bands below the rock crest either
/// side, so a pass reads as a gap from directly above. It is a **ceiling**, not a
/// subtraction, and that is the point — a pass that closed when the terrain style
/// got rugged would make "a ridge with two passes" a claim the map does not keep.
const SADDLE_CEILING: f32 = 2.7;

/// The elevation field, in band units, split into the part that is fixed and the
/// ridges — which are scaled to hit the rock target.
///
/// Keeping ridges separate is the whole trick. Lifting the *whole* field to make
/// more rock turns every high-noise pixel into a wall, which is confetti; scaling
/// the ridge term instead makes the existing ridges wider and taller, which is a
/// bigger wall in the same place. Rock stays where a landform put it.
pub(crate) struct Elevation {
    base: Vec<f32>,
    ridge: Vec<f32>,
    /// Height cap at the passes. `f32::MAX` everywhere else.
    ceiling: Vec<f32>,
    /// Cells where a ridge was deliberately notched — the passes, as generation
    /// meant them rather than as a shape-detector guesses them back out.
    pub(crate) saddles: Vec<usize>,
}

impl Elevation {
    #[inline]
    fn at(&self, index: usize, gain: f32) -> f32 {
        (self.base[index] + self.ridge[index] * gain).min(self.ceiling[index])
    }

    /// The ground as a continuous surface, before it is cut into bands **and
    /// before the passes are notched out of it**.
    ///
    /// Water runs over this, not over the drawn bands (see
    /// [`super::hydro::carve_rivers`]). Two things follow, and both of them are
    /// the point. A dry valley half a band deep is level ground to the eye and
    /// to the cost model, and still the line a river takes — which is how the map
    /// gets a watercourse that reads as a deliberate corridor without paying a
    /// band boundary for it. And a saddle, which is the *cheapest* way across a
    /// massif, is not offered to the water at all: a pass is a gap cut for track,
    /// and a river that took one would pin its own shore apron down through the
    /// crest and leave a gorge with the massif in two pieces either side.
    pub(crate) fn relief(&self, gain: f32) -> Vec<f32> {
        (0..self.base.len())
            .map(|i| self.base[i] + self.ridge[i] * gain)
            .collect()
    }
}

/// Build the elevation field from placed landforms plus a little grain.
pub(crate) fn landform_field(
    canvas: &Canvas,
    seed: u64,
    options: MapGenOptions,
    scale: f32,
) -> Elevation {
    let w = canvas.w;
    let h = canvas.h;
    // **The plain.** Band 0, dead flat, and it is the whole map until something
    // is placed on it.
    //
    // It used to be band 1, with the low ground cut back down to band 0 around
    // every watercourse. That put two band boundaries either side of every river
    // on the map — the floodplain edge — plus two more down every dry valley,
    // and those alone crossed a random 30-tile sight line more than once. Sitting
    // the plain on the floor instead costs the ridge one extra band of climb and
    // buys back every one of those boundaries: a river now runs *in* the plain
    // rather than in a trench cut through it, and the shore apron
    // ([`Canvas::clamp_shores`]) only draws a valley wall where the water really
    // does cut through high ground.
    let mut base = vec![0.0f32; canvas.len()];
    let mut ridge = vec![0.0f32; canvas.len()];
    let mut ceiling = vec![f32::MAX; canvas.len()];

    let Ridges { saddles, tails } =
        add_ridges(&mut ridge, &mut ceiling, canvas, seed, options, scale);
    // What a ridge already claims, nothing may dig out again. A valley that cut
    // clean through a crest would leave a hole the generator never chose, and a
    // wall with unplanned holes in it is not a wall — it is a texture.
    let shield: Vec<f32> = ridge.iter().map(|r| (r / 1.5).clamp(0.0, 1.0)).collect();
    add_plateaus(&mut base, canvas, seed, options, scale, &tails);
    add_basins(&mut base, canvas, seed, options, scale, &shield);
    add_valleys(&mut base, canvas, seed, scale, &shield);

    // Noise decorates; it does not decide. Design 02 §2.2.
    let mut rng = stream(seed, salt::GRAIN);
    let grain = Grain::new(&mut rng);
    let amp = options.terrain.grain();
    for y in 0..h {
        for x in 0..w {
            let u = x as f32 / (w.max(2) - 1) as f32;
            let v = y as f32 / (h.max(2) - 1) as f32;
            base[y * w + x] += grain.sample(u, v) * amp;
        }
    }

    Elevation {
        base,
        ridge,
        ceiling,
        saddles,
    }
}

/// Keep the stretch of a walked spine that is actually on the map.
///
/// Arc position is measured along *this* stretch, not the whole walk. Get that
/// wrong and the passes are placed off the edge of the world, leaving a wall with
/// no way through it — which is the one thing a ridge must never be.
fn clip_to_map(w: usize, h: usize, points: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let inside = |&(x, y): &(f32, f32)| {
        x >= -0.5 && y >= -0.5 && x <= w as f32 - 0.5 && y <= h as f32 - 0.5
    };
    let first = points.iter().position(inside);
    let last = points.iter().rposition(inside);
    match (first, last) {
        (Some(a), Some(b)) if b >= a => points[a..=b].to_vec(),
        _ => Vec::new(),
    }
}

/// What ridge placement wrote down for the phases that come after it.
struct Ridges {
    /// Cells where a crest was deliberately notched — the passes.
    saddles: Vec<usize>,
    /// The inland end of each spine. [`add_plateaus`] seats a tableland here so
    /// the map's high ground is one system rather than two.
    tails: Vec<(f32, f32)>,
}

fn add_ridges(
    field: &mut [f32],
    ceiling: &mut [f32],
    canvas: &Canvas,
    seed: u64,
    options: MapGenOptions,
    scale: f32,
) -> Ridges {
    let w = canvas.w;
    let h = canvas.h;
    let mut rng = stream(seed, salt::RIDGE);
    let count = ((options.terrain.ridges() as f32) * scale).round().max(1.0) as usize;
    let short = w.min(h) as f32;
    let mut saddles = Vec::new();
    let mut tails = Vec::new();
    if short < 12.0 {
        return Ridges { saddles, tails };
    }

    // Distance to open sea, on the maps that have any. A massif standing in the
    // water is a massif the shoreline planes back down — every seed that came up
    // short of §2.1's rock share was one where the ridge and the inlet had been
    // rolled onto the same corner. Mountains go inland; the coast is for looking
    // at.
    let from_sea = canvas
        .surface
        .contains(&Surface::Sea)
        .then(|| canvas.distance_to(|i| canvas.surface[i] == Surface::Sea));

    for i in 0..count {
        // Start off one edge, so the massif arrives already committed to a
        // direction rather than being a lump someone dropped in a field — and
        // then **stop**, `reach` tiles in.
        //
        // A spine that ran right across the map dragged its four-tile flank
        // across with it: a fifth of a 64² map was ground that changed height,
        // whichever way you crossed it. The same rock run in from one edge
        // keeps a chunk of its apron off-frame, leaves the far side of the map
        // open, and still asks the only question a ridge is for — round the end,
        // or through a pass.
        let run = ((short * options.terrain.reach()) / 1.5).round() as usize;
        let mut best: Option<(usize, Vec<(f32, f32)>)> = None;
        for _ in 0..3 {
            let angle = rng.gen_range(0.0..std::f32::consts::TAU);
            let offset = rng.gen_range(-0.26..0.26) * short;
            let cx = (w - 1) as f32 * 0.5 + (angle + std::f32::consts::FRAC_PI_2).cos() * offset;
            let cy = (h - 1) as f32 * 0.5 + (angle + std::f32::consts::FRAC_PI_2).sin() * offset;
            let launch = ((w * w + h * h) as f32).sqrt() * 0.5 + 3.0;
            let start = (cx - angle.cos() * launch, cy - angle.sin() * launch);
            let mut points = clip_to_map(w, h, &walk(w, h, start, angle, 1.5, 0.09, &mut rng));
            if points.len() < 8 {
                continue;
            }
            // Truncation keeps the entry end, which is the end that is anchored
            // to the frame.
            points.truncate(run.max(8).min(points.len()));
            let wet = from_sea.as_ref().map_or(0, |sea| {
                points
                    .iter()
                    .filter(|&&(px, py)| {
                        canvas
                            .idx(px.round() as i32, py.round() as i32)
                            .is_some_and(|k| sea[k] < SEA_STANDOFF)
                    })
                    .count()
            });
            let better = best.as_ref().is_none_or(|(worst, _)| wet < *worst);
            if better {
                best = Some((wet, points));
            }
            if wet == 0 {
                break;
            }
        }
        let Some((_, points)) = best else { continue };

        let wanted = ((options.terrain.passes_per_ridge() as f32) * scale)
            .round()
            .clamp(2.0, 4.0) as usize;
        // Passes sit inside the span and never on top of each other: a ridge with
        // two passes is a decision (§2.2), two passes three tiles apart is one.
        let span = 0.76;
        let lead = (1.0 - span) * 0.5;
        let slot = span / wanted as f32;
        let passes: Vec<f32> = (0..wanted)
            .map(|k| lead + slot * (k as f32 + rng.gen_range(0.3..0.7)))
            .collect();

        let ridge = Ridge {
            // A saddle a couple of tiles either side of the spine: wide enough
            // to lay track through, narrow enough that the wall still reads as
            // a wall.
            notch: (NOTCH_STEPS / (points.len() - 1) as f32).clamp(0.02, 0.2),
            points,
            // Wide enough that the crest survives relaxation. Bands may only
            // step one at a time, so a crest five bands above the plain needs
            // five tiles of flank beneath it on each side — draw a narrower
            // ridge and it is planed down to a hill however tall it was meant to
            // be. Stout rather than thin, too: for a given amount of rock, the
            // squarer the massif the less apron there is round it, and the apron
            // is what the player feels.
            //
            // Note this is a *ceiling* on the massif, not its size: the crest
            // reaches whatever radius [`solve_ridge_gain`] needs for §2.1's rock
            // share, and relaxation lays the same four-tile apron round it
            // whatever this says. What a too-small half-width does is cap the
            // rock below its target on the smaller map sizes, where four tiles
            // of apron eat a much larger share of the landform.
            half_width: (short * rng.gen_range(0.16..0.21)).clamp(10.0, 14.0),
            crest: options.terrain.crest() * if i == 0 { 1.0 } else { rng.gen_range(0.82..1.0) },
            passes,
        };

        let seeds = spine_seeds(w, h, &ridge.points);
        if seeds.is_empty() {
            continue;
        }
        if let Some(&tail) = ridge.points.last() {
            tails.push(tail);
        }
        // Remember where the saddles landed on the spine.
        for &pass in &ridge.passes {
            let k = (pass * (ridge.points.len() - 1) as f32).round() as usize;
            let (px, py) = ridge.points[k.min(ridge.points.len() - 1)];
            if let Some(index) = canvas.idx(px.round() as i32, py.round() as i32) {
                saddles.push(index);
            }
        }

        let (dist, arc) = chamfer(w, h, &seeds);
        for index in 0..field.len() {
            let u = dist[index] / ridge.half_width;
            if u >= 1.0 {
                continue;
            }
            // Cosine section: flanks that ease out instead of ending in a step.
            let profile = 0.5 * (1.0 + (std::f32::consts::PI * u).cos());
            field[index] += ridge.crest * profile;
            // A saddle is a col **right across** the ridge, so the cap follows
            // the arc alone and not the crest profile.
            //
            // Weighting it by the profile as well used to be right: when a ridge
            // was a thin wall, a notch that faded out towards the flanks still
            // reached both sides of it. A massif is three times as wide, and the
            // same notch became a dimple in the middle of a rock plateau with
            // crest all the way round it — a pass you could walk into and not
            // out of, which the verifier in `gen.rs` correctly refused to call a
            // pass at all. Off the crest the cap is above the ground anyway, so
            // the col only shows where there is something to cut through.
            let notch = ridge.notch_at(arc[index]);
            if notch > 0.05 {
                // Ramp the cap out over the shoulders. Gentle enough that the
                // saddle is a few tiles of buildable ground and not a keyhole.
                let cap = SADDLE_CEILING + (1.0 - NOTCH_SHOULDER * notch).max(0.0) * 5.0;
                ceiling[index] = ceiling[index].min(cap);
            }
        }
    }
    Ridges { saddles, tails }
}

/// Tablelands: broad, flat-topped uplands one band above the plain.
///
/// §2.2's plateau exactly — "flat on top, expensive to reach" — and now the map's
/// other shade of grass. Wide and low beats small and tall for the same reason a
/// massif beats a wall: the player pays at the *rim* and builds freely on the
/// top, so a table twelve tiles across is one 6× decision and a hundred and fifty
/// tiles of open country, while three tables four tiles across are three
/// decisions and nowhere to put anything.
fn add_plateaus(
    field: &mut [f32],
    canvas: &Canvas,
    seed: u64,
    options: MapGenOptions,
    scale: f32,
    tails: &[(f32, f32)],
) {
    let w = canvas.w;
    let h = canvas.h;
    let mut rng = stream(seed, salt::PLATEAU);
    let count = ((options.terrain.plateaus() as f32) * scale).round() as usize;
    let short = w.min(h) as f32;
    if short < 16.0 {
        return;
    }

    for k in 0..count {
        // The lift is held close to a whole band. Half a band either way and the
        // grain would decide the table's band for it, tile by tile, and a
        // tableland whose edge is a rash of single steps is the texture this
        // whole pass exists to remove.
        let lift = options.terrain.plateau_lift() * rng.gen_range(0.94..1.06);
        // A table that stands higher takes less of the map: its rim costs more
        // to cross, and an upland the player pays two bands to climb onto had
        // better be a mesa rather than half the countryside.
        let radius = (short * rng.gen_range(0.17..0.24) / lift.max(1.0)).clamp(4.0, 18.0);
        let lo = radius + 2.0;
        let hi = |limit: usize| (limit as f32 - lo).max(lo + 1.0);
        let mut px = rng.gen_range(lo..hi(w));
        let mut py = rng.gen_range(lo..hi(h));
        if let Some(&(tx, ty)) = tails.get(k) {
            // Seated against the inland end of a ridge, just clear of the crest:
            // the table is the massif's shoulder, not ground buried under its
            // flank, and the two share an outline instead of each paying for one.
            let bearing = rng.gen_range(0.0..std::f32::consts::TAU);
            px = (tx + bearing.cos() * radius * 0.55).clamp(lo, hi(w));
            py = (ty + bearing.sin() * radius * 0.55).clamp(lo, hi(h));
        }
        let flank = 2.5f32;
        let (x0, x1) = span(px, radius + flank, w);
        let (y0, y1) = span(py, radius + flank, h);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = ((x as f32 - px).powi(2) + (y as f32 - py).powi(2)).sqrt();
                if d > radius + flank {
                    continue;
                }
                let t = ((radius + flank - d) / flank).clamp(0.0, 1.0);
                // An upper envelope, not a sum. Two tables that touch are one
                // wider table at one height; added together they made a seam of
                // hills along the join that nothing had placed and nothing
                // explained — landform arithmetic doing exactly what §2.2 says
                // noise must not.
                let i = y * w + x;
                field[i] = field[i].max(lift * t);
            }
        }
    }
}

/// Hollows: low ground scooped out of whatever is standing above the plain.
///
/// The raised rim this used to carry is gone. On a band-1 map a rim was a ring of
/// hills round a dip and read as a crater; on a band-0 plain it would be a ring of
/// single-tile steps round nothing at all — the map's most obvious piece of
/// texture-for-its-own-sake. What is left digs a vale into a tableland, seats a
/// lake ([`super::hydro::place_lakes`]) in ground that explains it, and does
/// nothing whatsoever out on the open plain, which is the right amount.
fn add_basins(
    field: &mut [f32],
    canvas: &Canvas,
    seed: u64,
    options: MapGenOptions,
    scale: f32,
    shield: &[f32],
) {
    let w = canvas.w;
    let h = canvas.h;
    let mut rng = stream(seed, salt::BASIN);
    let count = ((options.terrain.basins() as f32) * scale).round() as usize;
    let short = w.min(h) as f32;
    if short < 16.0 {
        return;
    }

    for _ in 0..count {
        let radius = (short * rng.gen_range(0.07..0.11)).clamp(3.0, 9.0);
        let px = rng.gen_range(radius + 3.0..(w as f32 - radius - 3.0).max(radius + 4.0));
        let py = rng.gen_range(radius + 3.0..(h as f32 - radius - 3.0).max(radius + 4.0));
        let depth = rng.gen_range(1.2..2.0);
        let flank = 3.0f32;
        let (x0, x1) = span(px, radius + flank, w);
        let (y0, y1) = span(py, radius + flank, h);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = ((x as f32 - px).powi(2) + (y as f32 - py).powi(2)).sqrt();
                if d > radius + flank {
                    continue;
                }
                let i = y * w + x;
                let t = ((radius + flank - d) / flank).clamp(0.0, 1.0);
                field[i] -= depth * t * (1.0 - shield[i]);
            }
        }
    }
}

/// Row / column window a disc of `radius` around `centre` can touch.
fn span(centre: f32, radius: f32, limit: usize) -> (usize, usize) {
    let lo = (centre - radius).floor().max(0.0) as usize;
    let hi = ((centre + radius).ceil() as usize + 1).min(limit);
    (lo.min(limit), hi)
}

/// Dry valleys: the corridors that tell the player — and the water — where the
/// obvious route goes (§2.2).
///
/// Cut deeper than they used to be, and mostly invisible, which is the point.
/// Out on the plain a valley floor a band and a half down still quantises to
/// band 0, so it draws no step and costs nothing to build along; what it does is
/// tilt the ground under [`super::hydro::carve_rivers`], which reads the
/// continuous relief rather than the drawn bands. So the trunk river lies in a
/// valley for the whole of its length without the valley itself ever being a
/// thing the player has to climb out of. Where the same corridor crosses a
/// tableland it *is* drawn, as the gap in the escarpment — which is where a
/// valley earns being visible.
fn add_valleys(field: &mut [f32], canvas: &Canvas, seed: u64, scale: f32, shield: &[f32]) {
    let w = canvas.w;
    let h = canvas.h;
    let mut rng = stream(seed, salt::VALLEY);
    let short = w.min(h) as f32;
    if short < 16.0 {
        return;
    }
    let count = scale.round().max(1.0) as usize;

    for _ in 0..count {
        let angle = rng.gen_range(0.0..std::f32::consts::TAU);
        let reach = ((w * w + h * h) as f32).sqrt() * 0.5 + 3.0;
        let cx = rng.gen_range(short * 0.25..w as f32 - short * 0.25);
        let cy = rng.gen_range(short * 0.25..h as f32 - short * 0.25);
        let start = (cx - angle.cos() * reach, cy - angle.sin() * reach);
        let points = clip_to_map(w, h, &walk(w, h, start, angle, 1.5, 0.13, &mut rng));
        let seeds = spine_seeds(w, h, &points);
        if seeds.is_empty() {
            continue;
        }
        let (dist, _) = chamfer(w, h, &seeds);
        let half_width = (short * 0.06).clamp(2.0, 6.0);
        let depth = rng.gen_range(1.6..2.6);
        for index in 0..field.len() {
            let u = dist[index] / half_width;
            if u >= 1.0 {
                continue;
            }
            field[index] -=
                depth * 0.5 * (1.0 + (std::f32::consts::PI * u).cos()) * (1.0 - shield[index]);
        }
    }
}

/// Quantise the elevation to bands at a given ridge gain, then make it legal:
/// shorelines pinned low and no step bigger than one band.
pub(crate) fn apply_field(canvas: &mut Canvas, elevation: &Elevation, gain: f32) {
    for i in 0..canvas.len() {
        canvas.band[i] = elevation.at(i, gain).round().clamp(0.0, TOP_BAND as f32) as i8;
    }
    canvas.clamp_shores();
    canvas.relax_bands();
    canvas.tidy_rock();
}

/// Solve the ridge gain that lands the impassable-rock share on its target.
///
/// Bisection on a monotone quantity: ridges only ever add height, so a bigger
/// gain can only add rock. Scaling the ridges rather than the whole field is what
/// keeps rock in walls — see [`Elevation`].
pub(crate) fn solve_ridge_gain(canvas: &mut Canvas, elevation: &Elevation, target_pct: f32) -> f32 {
    let target = target_pct / 100.0 * canvas.len() as f32;
    let rock_at = |canvas: &mut Canvas, gain: f32| -> f32 {
        apply_field(canvas, elevation, gain);
        canvas
            .band
            .iter()
            .zip(&canvas.surface)
            .filter(|(b, s)| **b >= super::field::ROCK_BAND && **s == Surface::Land)
            .count() as f32
    };

    let mut lo = 0.10f32;
    let mut hi = 9.0f32;
    for _ in 0..11 {
        let mid = 0.5 * (lo + hi);
        if rock_at(canvas, mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    // Rock is a step function of the gain, so the bracket's midpoint can sit a
    // long way off on a small map where one tile is a tenth of a percent. Take
    // whichever end of the bracket actually lands nearer the target.
    let low = (rock_at(canvas, lo) - target).abs();
    let high = (rock_at(canvas, hi) - target).abs();
    let gain = if low <= high { lo } else { hi };
    apply_field(canvas, elevation, gain);
    grow_rock_to(canvas, target);
    gain
}

/// Thicken the existing walls until the rock share reaches its target.
///
/// A map whose ridges the valley floors planed down can run short of rock however
/// far the ridge gain is pushed. Growing outward from rock that is already there
/// — never seeding new specks — keeps the shortfall from turning into confetti,
/// and keeps the wall in the place a landform chose.
///
/// Growth is band by band: a tile may only be raised to `b` when every orthogonal
/// neighbour has already reached `b - 1`, so the one-band-per-tile rule that makes
/// the whole map climbable is never broken. When the crest has no room left to
/// spread, its apron is raised first and the crest tries again next round — and
/// each of those recoveries costs a round, which is why there are as many as
/// there are. A massif is stouter than the old map-long wall and its crest runs
/// out of room sooner.
fn grow_rock_to(canvas: &mut Canvas, target: f32) {
    let rock = super::field::ROCK_BAND;
    let mut have = canvas
        .band
        .iter()
        .zip(&canvas.surface)
        .filter(|(b, s)| **b >= rock && **s == Surface::Land)
        .count() as f32;

    for _ in 0..24 {
        if have >= target {
            return;
        }
        let room = (target - have).ceil() as usize;
        let grown = lift_to(canvas, rock, room);
        have += grown as f32;
        if grown == 0 && lift_to(canvas, rock - 1, usize::MAX) == 0 {
            return;
        }
    }
}

/// Raise land one band below `to` that already touches `to`, up to `budget`
/// tiles, keeping every step at one band.
fn lift_to(canvas: &mut Canvas, to: i8, budget: usize) -> usize {
    if to <= 0 || budget == 0 {
        return 0;
    }
    let mut raise: Vec<usize> = Vec::new();
    for y in 0..canvas.h as i32 {
        for x in 0..canvas.w as i32 {
            let i = canvas.at(x, y);
            if canvas.surface[i] != Surface::Land || canvas.band[i] != to - 1 {
                continue;
            }
            let neighbours = [(1, 0), (-1, 0), (0, 1), (0, -1)];
            let seated = neighbours
                .iter()
                .all(|(dx, dy)| canvas.idx(x + dx, y + dy).is_none_or(|k| canvas.band[k] >= to - 1));
            let touching = neighbours
                .iter()
                .any(|(dx, dy)| canvas.idx(x + dx, y + dy).is_some_and(|k| canvas.band[k] >= to));
            if seated && touching {
                raise.push(i);
            }
        }
    }
    let n = raise.len().min(budget);
    for &i in &raise[..n] {
        canvas.band[i] = to;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chamfer_beats_a_manhattan_diamond() {
        let (dist, _) = chamfer(9, 9, &[(4 * 9 + 4, 1.0)]);
        // Straight out is exact…
        assert!((dist[4 * 9 + 8] - 4.0).abs() < 0.01);
        // …and diagonal is near √2 · 4 ≈ 5.66, not 8.
        let diagonal = dist[8 * 9 + 8];
        assert!(
            (5.0..6.2).contains(&diagonal),
            "diagonal distance {diagonal} is not Euclidean-ish"
        );
    }

    #[test]
    fn chamfer_carries_the_nearest_seed_payload() {
        let (_, arc) = chamfer(9, 1, &[(0, 0.0), (8, 1.0)]);
        assert_eq!(arc[1], 0.0);
        assert_eq!(arc[7], 1.0);
    }

    #[test]
    fn a_ridge_notch_is_a_saddle_not_a_slot() {
        let ridge = Ridge {
            points: vec![(0.0, 0.0); 40],
            half_width: 4.0,
            crest: 5.0,
            passes: vec![0.5],
            notch: 0.1,
        };
        assert_eq!(ridge.notch_at(0.0), 0.0, "the crest is untouched away from a pass");
        assert!((ridge.notch_at(0.5) - 1.0).abs() < 1e-5, "the saddle is fully notched");
        let shoulder = ridge.notch_at(0.55);
        assert!(
            (0.0..1.0).contains(&shoulder) && shoulder > 0.0,
            "the notch must have shoulders, got {shoulder}"
        );
    }

    #[test]
    fn a_pass_stays_buildable_however_rugged_the_map_gets() {
        // The saddle is a ceiling, not a subtraction, so scaling the ridges up
        // cannot close it. `SADDLE_CEILING` rounds to a band below the rock band.
        assert!(SADDLE_CEILING.round() < super::super::field::ROCK_BAND as f32);
        let elevation = Elevation {
            base: vec![1.15],
            ridge: vec![6.0],
            ceiling: vec![SADDLE_CEILING],
            saddles: Vec::new(),
        };
        for gain in [0.5f32, 1.0, 3.0, 9.0] {
            assert_eq!(elevation.at(0, gain), SADDLE_CEILING);
        }
    }

    #[test]
    fn clipping_measures_arc_along_the_visible_stretch() {
        let walked = [
            (-9.0, 4.0),
            (-3.0, 4.0),
            (3.0, 4.0),
            (7.0, 4.0),
            (14.0, 4.0),
        ];
        let clipped = clip_to_map(10, 10, &walked);
        assert_eq!(clipped, vec![(3.0, 4.0), (7.0, 4.0)]);
    }

    #[test]
    fn a_sparse_map_is_landlocked() {
        // Playtest: "the default map should read as inland countryside."
        let options = MapGenOptions {
            water: crate::options::WaterStyle::Sparse,
            ..MapGenOptions::standard()
        };
        let mut canvas = Canvas::new(64, 64);
        assert!(!carve_sea(&mut canvas, 42, options, 1.0));
        assert!(canvas.surface.iter().all(|s| *s == Surface::Land));
    }

    #[test]
    fn a_coastal_map_gets_an_inlet_not_a_frame() {
        let options = MapGenOptions {
            water: crate::options::WaterStyle::Riverlands,
            ..MapGenOptions::standard()
        };
        let mut canvas = Canvas::new(64, 64);
        assert!(carve_sea(&mut canvas, 42, options, 1.0));
        let sea = canvas.surface.iter().filter(|s| **s == Surface::Sea).count() as f32 * 100.0
            / canvas.len() as f32;
        assert!(
            (sea - options.water.sea_target()).abs() < 1.0,
            "sea {sea:.1}% missed target {:.1}%",
            options.water.sea_target()
        );
        // The old generator drowned the border all the way round. Count how much
        // of the frame is still dry: an inlet leaves nearly all of it.
        let dry = (0..64i32)
            .flat_map(|k| [(k, 0), (k, 63), (0, k), (63, k)])
            .filter(|&(x, y)| canvas.surface[canvas.at(x, y)] == Surface::Land)
            .count();
        assert!(dry > 220, "only {dry} of 256 border tiles stayed dry");
    }
}
