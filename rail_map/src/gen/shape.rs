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
    // A base of one band: open, buildable countryside is the default surface and
    // everything else is something placed on top of it.
    let mut base = vec![1.15f32; canvas.len()];
    let mut ridge = vec![0.0f32; canvas.len()];
    let mut ceiling = vec![f32::MAX; canvas.len()];

    let saddles = add_ridges(&mut ridge, &mut ceiling, canvas, seed, options, scale);
    // What a ridge already claims, nothing may dig out again. A valley that cut
    // clean through a crest would leave a hole the generator never chose, and a
    // wall with unplanned holes in it is not a wall — it is a texture.
    let shield: Vec<f32> = ridge.iter().map(|r| (r / 1.5).clamp(0.0, 1.0)).collect();
    add_plateaus(&mut base, canvas, seed, options, scale);
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

fn add_ridges(
    field: &mut [f32],
    ceiling: &mut [f32],
    canvas: &Canvas,
    seed: u64,
    options: MapGenOptions,
    scale: f32,
) -> Vec<usize> {
    let w = canvas.w;
    let h = canvas.h;
    let mut rng = stream(seed, salt::RIDGE);
    let count = ((options.terrain.ridges() as f32) * scale).round().max(1.0) as usize;
    let short = w.min(h) as f32;
    let mut saddles = Vec::new();
    if short < 12.0 {
        return saddles;
    }

    for i in 0..count {
        // Start off one edge and cross the map, so the ridge is a barrier rather
        // than a lump in the middle of a field.
        let angle = rng.gen_range(0.0..std::f32::consts::TAU);
        let offset = rng.gen_range(-0.26..0.26) * short;
        let cx = (w - 1) as f32 * 0.5 + (angle + std::f32::consts::FRAC_PI_2).cos() * offset;
        let cy = (h - 1) as f32 * 0.5 + (angle + std::f32::consts::FRAC_PI_2).sin() * offset;
        let reach = ((w * w + h * h) as f32).sqrt() * 0.5 + 3.0;
        let start = (cx - angle.cos() * reach, cy - angle.sin() * reach);
        let points = clip_to_map(w, h, &walk(w, h, start, angle, 1.5, 0.09, &mut rng));
        if points.len() < 8 {
            continue;
        }

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
            // A saddle about three tiles along the crest: wide enough to lay
            // track through, narrow enough that the wall still reads as a wall.
            notch: (2.0 / (points.len() - 1) as f32).clamp(0.02, 0.14),
            points,
            // Wide enough that the crest survives relaxation. Bands may only
            // step one at a time, so a crest five bands above the surrounding
            // country needs five tiles of flank beneath it on each side — draw a
            // narrower ridge and it is planed down to a hill however tall it was
            // meant to be. The flanks are open buildable hill either way; only
            // the last tile or two of crest is the wall.
            half_width: (short * rng.gen_range(0.10..0.13)).clamp(6.0, 9.0),
            crest: options.terrain.crest() * if i == 0 { 1.0 } else { rng.gen_range(0.82..1.0) },
            passes,
        };

        let seeds = spine_seeds(w, h, &ridge.points);
        if seeds.is_empty() {
            continue;
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
            // A saddle holds the crest down where it crosses the spine, and lets
            // go out on the flanks where there is nothing to hold down.
            let notch = ridge.notch_at(arc[index]) * profile;
            if notch > 0.05 {
                // Ramp the cap out over the shoulders. Gentle enough that the
                // saddle is a few tiles of buildable ground and not a keyhole.
                let cap = SADDLE_CEILING + (1.0 - notch) * 5.0;
                ceiling[index] = ceiling[index].min(cap);
            }
        }
    }
    saddles
}

fn add_plateaus(field: &mut [f32], canvas: &Canvas, seed: u64, options: MapGenOptions, scale: f32) {
    let w = canvas.w;
    let h = canvas.h;
    let mut rng = stream(seed, salt::PLATEAU);
    let count = ((options.terrain.plateaus() as f32) * scale).round() as usize;
    let short = w.min(h) as f32;
    if short < 16.0 {
        return;
    }

    for _ in 0..count {
        let radius = (short * rng.gen_range(0.08..0.12)).clamp(3.0, 10.0);
        let px = rng.gen_range(radius + 2.0..(w as f32 - radius - 2.0).max(radius + 3.0));
        let py = rng.gen_range(radius + 2.0..(h as f32 - radius - 2.0).max(radius + 3.0));
        // Flat on top, expensive to reach: the lift is constant inside and the
        // flank is short, so the whole cost of the plateau is at its edge (§2.2).
        let lift = rng.gen_range(1.7..2.6);
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
                field[y * w + x] += lift * t;
            }
        }
    }
}

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
        let rim = rng.gen_range(0.5..1.1);
        let (x0, x1) = span(px, radius + 3.0, w);
        let (y0, y1) = span(py, radius + 3.0, h);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = ((x as f32 - px).powi(2) + (y as f32 - py).powi(2)).sqrt();
                if d > radius + 3.0 {
                    continue;
                }
                let i = y * w + x;
                if d <= radius {
                    // Cheap inside…
                    field[i] -= depth * (1.0 - shield[i]);
                } else {
                    // …costly to enter or leave.
                    field[i] += rim * (1.0 - (d - radius) / 3.0);
                }
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

/// Dry valleys: corridors that are cheap to build along and that tell the player
/// where the obvious route goes (§2.2). Rivers cut their own.
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
        let half_width = (short * 0.05).clamp(2.0, 5.0);
        let depth = rng.gen_range(1.0..1.8);
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
/// A map whose ridges were planed down by the shoreline relaxation can run short
/// of rock however far the gain is pushed. Growing outward from rock that is
/// already there — never seeding new specks — keeps the shortfall from turning
/// into confetti, and keeps the wall in the place a landform chose.
fn grow_rock_to(canvas: &mut Canvas, target: f32) {
    let rock = super::field::ROCK_BAND;
    let count = |canvas: &Canvas| {
        canvas
            .band
            .iter()
            .zip(&canvas.surface)
            .filter(|(b, s)| **b >= rock && **s == Surface::Land)
            .count()
    };
    let mut have = count(canvas) as f32;
    if have >= target {
        return;
    }

    for _ in 0..6 {
        let mut promote: Vec<usize> = Vec::new();
        for y in 0..canvas.h as i32 {
            for x in 0..canvas.w as i32 {
                let i = canvas.at(x, y);
                if canvas.surface[i] != Surface::Land || canvas.band[i] != rock - 1 {
                    continue;
                }
                // Only where the step stays legal: every orthogonal neighbour is
                // already within one band of rock.
                if ![(1, 0), (-1, 0), (0, 1), (0, -1)].iter().all(|(dx, dy)| {
                    canvas
                        .idx(x + dx, y + dy)
                        .is_none_or(|k| canvas.band[k] >= rock - 1)
                }) {
                    continue;
                }
                let touching = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .filter(|(dx, dy)| {
                        canvas
                            .idx(x + dx, y + dy)
                            .is_some_and(|k| canvas.band[k] >= rock)
                    })
                    .count();
                if touching > 0 {
                    promote.push(i);
                }
            }
        }
        if promote.is_empty() {
            return;
        }
        for i in promote {
            if have >= target {
                return;
            }
            canvas.band[i] = rock;
            have += 1.0;
        }
    }
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
