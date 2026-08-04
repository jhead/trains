//! Rivers and lakes — generated deliberately, never as a height threshold.
//!
//! Design 02 §2.1: "**Rivers are the best terrain feature in the game** and
//! should be generated deliberately rather than falling out of a height
//! threshold. A river is a continuous line the player must cross somewhere, and
//! choosing *where* to cross is a real decision."
//!
//! That sentence sets the whole shape of this module. A river here is a *route*
//! first — a least-cost course from a source in the high ground down to the sea,
//! which is why it lies in the valleys and why its banks are a corridor. Its
//! width is then authored along that course: wide enough to refuse a *cheap*
//! bridge everywhere except at two to four **named narrows**, whose spans
//! deliberately differ so the player is choosing between a cheap bridge with a
//! long detour and an expensive one with a short detour. §2.1 calls that pair "a
//! complete design problem on its own".
//!
//! A trunk is not a wall. It is `TRUNK_WIDTH` across, inside `rail_sim`'s span
//! limit, so a player who wants to cross *here* can always buy their way over at
//! the premium end of the bridge ladder. That is the third option in the same
//! decision, not an escape from it: paying 30x a tile to avoid a detour is
//! exactly the trade §2.1 is asking the player to weigh.
//!
//! The widths are not merely intended, they are **enforced**: after carving,
//! anything outside a named crossing that turns out to be *cheaply* bridgeable
//! gets widened until it is not. Otherwise a lucky bend quietly becomes a fifth
//! narrows and the decision evaporates.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use rail_sim::CHEAP_BRIDGE_SPAN;
use rand::rngs::StdRng;
use rand::Rng;

use crate::features::{RiverCrossing, Surface};
use crate::options::MapGenOptions;

use super::field::{salt, stream, Canvas, Grain, ROCK_BAND};

/// Width of a river where it is not meant to be crossed cheaply.
///
/// One tile past `CHEAP_BRIDGE_SPAN`: the narrowest width that refuses a *cheap*
/// bridge, so the water between the narrows is never more water than it has to
/// be. It stays inside `MAX_BRIDGE_SPAN`, so the trunk is crossable at the
/// premium rate — the expensive answer to "I want to cross right here".
const TRUNK_WIDTH: u32 = CHEAP_BRIDGE_SPAN + 1;

/// Crossing spans in the order they are handed out along a course.
///
/// Adjacent crossings always differ, and both ends of the cheap tier of
/// `rail_sim`'s bridge cost ladder appear: span 1 is 8× base, span 3 is 20× and
/// already a decision (§3.4). Anything wider is trunk, and trunk prices at 30×
/// and up.
const CROSSING_SPANS: [u32; 4] = [1, 3, 2, 3];

/// How far from a named crossing the width enforcement stops caring.
const CROSSING_GUARD: i32 = 5;

/// Most of a map, as a fraction of its tiles, that standing water may take
/// however much of the inland budget the rivers left unspent.
///
/// "Narrow rivers, generously placed, beat big lakes" — so the leftover is not
/// an invitation to flood a basin, and what the rivers did not spend simply
/// stays open land.
const LAKE_SHARE_DENOM: usize = 32;

/// What one river laid down.
pub(crate) struct River {
    /// The course, source first, as row-major cell indices.
    pub(crate) course: Vec<usize>,
    /// Named, verified narrows along it.
    pub(crate) crossings: Vec<RiverCrossing>,
}

/// How far below the plain a valley floor may lie, in bands. See [`flow_to_sea`].
const RELIEF_FLOOR: f32 = 3.0;

/// Cost of entering a tile, per *squared* band of height above the valley floor.
///
/// Squared, because linear was not enough to keep water off the high ground.
/// A source is chosen for being expensive to reach, which puts it *behind* the
/// massif as often as not, and under a linear cost the cheapest way to the sea
/// from there was straight over the top: twenty-odd tiles of climb still added
/// up to less than going round. The river then pinned its shore apron down
/// through the crest and cut the massif in half — every seed that missed §2.1's
/// rock share was this. Squaring makes the detour win by a distance, which is
/// what water does anyway.
const RELIEF_CLIMB: f32 = 6.0;

/// Least-cost descent to an outlet for every land tile.
///
/// One Dijkstra from every outlet at once, over a cost dominated by elevation, so
/// following `downhill` from anywhere traces the course water would take: along
/// valleys, around ridges, through a pass only when going round is genuinely
/// worse.
///
/// The elevation it reads is the **continuous relief**, not the drawn bands.
/// That distinction is what lets the map be calm and still have a river worth
/// crossing: the plain is one flat band from edge to edge, so bands alone would
/// leave the water nothing to follow but noise, and a river that wanders twenty
/// tiles to the nearest border is not the "continuous line the player must cross
/// somewhere" §2.1 asks for. Under the bands the dry valleys are still there,
/// a band or two deep, and the trunk lies in one for its whole length.
///
/// An **outlet** is the sea where there is one and the map border where there is
/// not. Most Rail Town maps are landlocked, and a river that runs off the edge is
/// exactly right for one: the world carries on past the frame, which is what the
/// edge portals are for.
struct Flow {
    cost: Vec<u32>,
    downhill: Vec<u32>,
}

const NO_STEP: u32 = u32::MAX;

fn flow_to_sea(canvas: &Canvas, seed: u64, relief: &[f32]) -> Flow {
    let w = canvas.w;
    let h = canvas.h;
    let mut rng = stream(seed, salt::RIVER);
    let meander = Grain::new(&mut rng);

    // Entering a tile costs its climb plus a wandering term, so a river on flat
    // ground meanders instead of ruling a straight line to the coast. Baked once:
    // Dijkstra asks for it several times per tile and the noise is not cheap.
    let enter: Vec<u32> = (0..canvas.len())
        .map(|i| {
            let x = (i % w) as f32 / (w.max(2) - 1) as f32;
            let y = (i / w) as f32 / (h.max(2) - 1) as f32;
            let wobble = ((meander.sample(x, y) + 1.0) * 2.5) as u32;
            let ground = relief
                .get(i)
                .map_or(canvas.band[i].max(0) as f32, |r| r + RELIEF_FLOOR)
                .max(0.0);
            4 + (ground * ground * RELIEF_CLIMB) as u32 + wobble
        })
        .collect();

    let mut cost = vec![u32::MAX; canvas.len()];
    let mut downhill = vec![NO_STEP; canvas.len()];
    let mut heap: BinaryHeap<Reverse<(u32, usize)>> = BinaryHeap::new();

    let has_sea = canvas.surface.contains(&Surface::Sea);
    for (i, cell) in canvas.surface.iter().enumerate() {
        let x = (i % w) as i32;
        let y = (i / w) as i32;
        let border = x == 0 || y == 0 || x == w as i32 - 1 || y == h as i32 - 1;
        let outlet = if has_sea {
            *cell == Surface::Sea
        } else {
            border && *cell == Surface::Land
        };
        if outlet {
            cost[i] = 0;
            heap.push(Reverse((0, i)));
        }
    }

    while let Some(Reverse((c, i))) = heap.pop() {
        if c > cost[i] {
            continue;
        }
        let x = (i % w) as i32;
        let y = (i / w) as i32;
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let Some(n) = canvas.idx(x + dx, y + dy) else {
                continue;
            };
            if canvas.surface[n] != Surface::Land {
                continue;
            }
            let next = c.saturating_add(enter[n]);
            if next < cost[n] {
                cost[n] = next;
                downhill[n] = i as u32;
                heap.push(Reverse((next, n)));
            }
        }
    }

    Flow { cost, downhill }
}

/// Follow the descent from `start` until the sea (or existing water) is reached.
fn course_from(flow: &Flow, canvas: &Canvas, start: usize) -> Vec<usize> {
    let mut course = vec![start];
    let mut at = start;
    for _ in 0..canvas.len() {
        let step = flow.downhill[at];
        if step == NO_STEP {
            // An outlet: either open water, or the map border on a landlocked map.
            break;
        }
        let next = step as usize;
        course.push(next);
        if canvas.surface[next].is_water() {
            break;
        }
        at = next;
    }
    course
}

/// Carve one course, with an authored width along it.
///
/// Widths are perpendicular to flow and exact: courses are 4-connected, so the
/// normal is always an axis and "four tiles across" means four tiles across.
fn carve(canvas: &mut Canvas, course: &[usize], width_at: impl Fn(usize) -> u32) {
    let w = canvas.w as i32;
    for (k, &cell) in course.iter().enumerate() {
        let x = (cell % canvas.w) as i32;
        let y = (cell / canvas.w) as i32;
        let ahead = course[(k + 1).min(course.len() - 1)];
        let behind = course[k.saturating_sub(1)];
        let dx = (ahead % canvas.w) as i32 - (behind % canvas.w) as i32;
        let dy = (ahead / canvas.w) as i32 - (behind / canvas.w) as i32;
        // Normal to the flow; a diagonal step (a corner) takes the x normal.
        let (nx, ny) = if dx.abs() >= dy.abs() { (0, 1) } else { (1, 0) };

        let width = width_at(k).max(1) as i32;
        for o in -(width / 2)..=((width - 1) / 2) {
            let Some(index) = canvas.idx(x + nx * o, y + ny * o) else {
                continue;
            };
            if canvas.surface[index] == Surface::Land {
                canvas.surface[index] = Surface::River;
            }
        }
        let _ = w;
    }
}

/// Contiguous water run through a cell on one axis, if both ends are land.
fn axis_span(canvas: &Canvas, x: i32, y: i32, dx: i32, dy: i32) -> Option<u32> {
    let mut span = 1u32;
    let (mut lx, mut ly) = (x - dx, y - dy);
    while canvas.is_water(lx, ly) {
        span += 1;
        lx -= dx;
        ly -= dy;
    }
    let (mut hx, mut hy) = (x + dx, y + dy);
    while canvas.is_water(hx, hy) {
        span += 1;
        hx += dx;
        hy += dy;
    }
    if !canvas.is_land(lx, ly) || !canvas.is_land(hx, hy) {
        return None;
    }
    Some(span)
}

/// Contiguous water run through a cell on one axis, ends ignored.
fn run_len(canvas: &Canvas, x: i32, y: i32, dx: i32, dy: i32) -> u32 {
    let mut span = 1u32;
    let (mut lx, mut ly) = (x - dx, y - dy);
    while canvas.is_water(lx, ly) {
        span += 1;
        lx -= dx;
        ly -= dy;
    }
    let (mut hx, mut hy) = (x + dx, y + dy);
    while canvas.is_water(hx, hy) {
        span += 1;
        hx += dx;
        hy += dy;
    }
    span
}

/// Shortest run through a water cell a *cheap* bridge could cover, or `None`.
///
/// The question is deliberately the cheap tier rather than `MAX_BRIDGE_SPAN`:
/// every trunk tile is bridgeable now, so "can this be bridged" no longer picks
/// out anything, whereas "can this be bridged without a mid-game budget" is
/// precisely the narrows §2.1 asks the generator to author.
fn cheap_span(canvas: &Canvas, index: usize) -> Option<u32> {
    let x = (index % canvas.w) as i32;
    let y = (index / canvas.w) as i32;
    let span = [(1, 0), (0, 1)]
        .into_iter()
        .filter_map(|(dx, dy)| axis_span(canvas, x, y, dx, dy))
        .min()?;
    (span <= CHEAP_BRIDGE_SPAN).then_some(span)
}

/// Widen every cheaply bridgeable spot that is not one of the named crossings.
///
/// A river whose narrows are wherever the rasteriser happened to pinch is a
/// river with no decision in it. This is the pass that makes the authored
/// crossings the *only* crossings.
fn enforce_widths(canvas: &mut Canvas, named: &[usize]) {
    for _ in 0..3 {
        let mut widen: Vec<(i32, i32, i32, i32)> = Vec::new();
        for index in 0..canvas.len() {
            if canvas.surface[index] != Surface::River {
                continue;
            }
            let x = (index % canvas.w) as i32;
            let y = (index / canvas.w) as i32;
            if named.iter().any(|&c| {
                let cx = (c % canvas.w) as i32;
                let cy = (c / canvas.w) as i32;
                (cx - x).abs() <= CROSSING_GUARD && (cy - y).abs() <= CROSSING_GUARD
            }) {
                continue;
            }
            if cheap_span(canvas, index).is_none() {
                continue;
            }
            // Push out the shorter axis, one tile each side.
            let horizontal = axis_span(canvas, x, y, 1, 0).unwrap_or(u32::MAX);
            let vertical = axis_span(canvas, x, y, 0, 1).unwrap_or(u32::MAX);
            let (dx, dy) = if horizontal <= vertical { (1, 0) } else { (0, 1) };
            widen.push((x, y, dx, dy));
        }
        if widen.is_empty() {
            return;
        }
        for (x, y, dx, dy) in widen {
            let (mut lx, mut ly) = (x - dx, y - dy);
            while canvas.is_water(lx, ly) {
                lx -= dx;
                ly -= dy;
            }
            let (mut hx, mut hy) = (x + dx, y + dy);
            while canvas.is_water(hx, hy) {
                hx += dx;
                hy += dy;
            }
            for (cx, cy) in [(lx, ly), (hx, hy)] {
                if let Some(index) = canvas.idx(cx, cy) {
                    if canvas.surface[index] == Surface::Land {
                        canvas.surface[index] = Surface::River;
                    }
                }
            }
        }
    }
}

/// Make every designated narrows actually cheap to cross.
///
/// A meander can leave an authored crossing one tile too wide, and a narrows
/// that prices like the trunk either side of it is not a decision — it is a
/// name on a map. So the run is trimmed back to a cheap-tier width from
/// whichever end is further from the crossing itself.
fn force_crossings(canvas: &mut Canvas, crossings: &[usize]) {
    for &cell in crossings {
        if cheap_span(canvas, cell).is_some() {
            continue;
        }
        let x = (cell % canvas.w) as i32;
        let y = (cell / canvas.w) as i32;
        let (dx, dy) = if run_len(canvas, x, y, 1, 0) <= run_len(canvas, x, y, 0, 1) {
            (1, 0)
        } else {
            (0, 1)
        };
        for _ in 0..12 {
            if run_len(canvas, x, y, dx, dy) <= CHEAP_BRIDGE_SPAN
                && cheap_span(canvas, cell).is_some()
            {
                break;
            }
            let (mut lx, mut ly) = (x, y);
            while canvas.is_water(lx - dx, ly - dy) {
                lx -= dx;
                ly -= dy;
            }
            let (mut hx, mut hy) = (x, y);
            while canvas.is_water(hx + dx, hy + dy) {
                hx += dx;
                hy += dy;
            }
            let low = (lx - x).abs() + (ly - y).abs();
            let high = (hx - x).abs() + (hy - y).abs();
            let (tx, ty) = if high >= low { (hx, hy) } else { (lx, ly) };
            let Some(index) = canvas.idx(tx, ty) else { break };
            if canvas.surface[index] != Surface::River || (tx, ty) == (x, y) {
                break;
            }
            canvas.surface[index] = Surface::Land;
        }
    }
}

/// Make sure the map offers crossings at more than one price.
///
/// §2.1 asks for narrows "of differing width", because that is the whole
/// decision: a cheap bridge with a long detour against an expensive one with a
/// short detour. A meander can quietly rasterise two authored widths into the
/// same measured span, so if every crossing ended up the same, one is nudged.
fn diversify_spans(canvas: &mut Canvas, named: &[usize]) {
    for attempt in 0..4 {
        let spans: Vec<(usize, u32)> = named
            .iter()
            .filter_map(|&c| cheap_span(canvas, c).map(|s| (c, s)))
            .collect();
        if spans.len() < 2 || spans.iter().any(|(_, s)| *s != spans[0].1) {
            return;
        }
        // Work through the crossings rather than hammering the first: its far
        // bank may be sea, or already as narrow as water gets.
        let (cell, span) = spans[attempt % spans.len()];
        let x = (cell % canvas.w) as i32;
        let y = (cell / canvas.w) as i32;
        let horizontal = axis_span(canvas, x, y, 1, 0).unwrap_or(u32::MAX);
        let (dx, dy) = if horizontal <= axis_span(canvas, x, y, 0, 1).unwrap_or(u32::MAX) {
            (1, 0)
        } else {
            (0, 1)
        };
        // Walk to the far end of the run and take a tile off it (or add one, when
        // the run is already as narrow as water gets).
        let (mut ex, mut ey) = (x, y);
        while canvas.is_water(ex + dx, ey + dy) {
            ex += dx;
            ey += dy;
        }
        if span > 1 {
            if let Some(index) = canvas.idx(ex, ey) {
                if canvas.surface[index] == Surface::River {
                    canvas.surface[index] = Surface::Land;
                }
            }
        } else if let Some(index) = canvas.idx(ex + dx, ey + dy) {
            if canvas.surface[index] == Surface::Land {
                canvas.surface[index] = Surface::River;
            }
        } else {
            return;
        }
    }
}

/// Pick a source: high, far inland, and away from anything already chosen.
fn pick_source(canvas: &Canvas, flow: &Flow, taken: &[usize], rng: &mut StdRng) -> Option<usize> {
    let mut best: Option<(i64, usize)> = None;
    let jitter = rng.gen_range(0..64) as i64;
    for i in 0..canvas.len() {
        if canvas.surface[i] != Surface::Land || flow.downhill[i] == NO_STEP {
            continue;
        }
        // A spring rises in the hills, not off the top of a cliff. Left to
        // itself the score below picks the highest, most expensive ground on the
        // map, which is the crest of the massif — and a course starting there
        // runs straight down through the wall, taking its band-0 shore apron
        // with it and leaving a gorge where §2.2 wanted a barrier.
        if canvas.band[i] >= ROCK_BAND - 1 {
            continue;
        }
        let x = (i % canvas.w) as i32;
        let y = (i / canvas.w) as i32;
        // A spring wants to be inland. On a landlocked map the border *is* the
        // outlet, so a source near it would produce a three-tile river.
        let inset = (canvas.w.min(canvas.h) / 5).max(3) as i32;
        if x < inset || y < inset || x >= canvas.w as i32 - inset || y >= canvas.h as i32 - inset {
            continue;
        }
        // A long expensive climb from the sea, and high up — but not on top of
        // another source.
        let apart = taken
            .iter()
            .map(|&t| {
                let tx = (t % canvas.w) as i32;
                let ty = (t / canvas.w) as i32;
                ((tx - x).abs() + (ty - y).abs()) as i64
            })
            .min()
            .unwrap_or(i64::MAX);
        if apart < (canvas.w.min(canvas.h) as i64) / 3 {
            continue;
        }
        let score = flow.cost[i] as i64 / 5
            + canvas.band[i] as i64 * 6
            + ((i as i64).wrapping_mul(2_654_435_761) ^ jitter).rem_euclid(7);
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, i));
        }
    }
    best.map(|(_, i)| i)
}

/// Lay every river system the options ask for.
///
/// Returns the rivers in the order they were cut: the trunk first, then its
/// tributaries. §2.1's "two to four viable crossing points" is a claim about
/// the trunk, and the trunk is `rivers[0]`.
pub(crate) fn carve_rivers(
    canvas: &mut Canvas,
    seed: u64,
    options: MapGenOptions,
    scale: f32,
    relief: &[f32],
) -> Vec<River> {
    let mut rng = stream(seed, salt::RIVER ^ 0x5151);
    let short = canvas.w.min(canvas.h);
    if short < 20 {
        return Vec::new();
    }

    let trunks = ((options.water.rivers() as f32) * scale).round().max(1.0) as usize;
    let tributaries = ((options.water.tributaries() as f32) * scale).round() as usize;
    let mut sources: Vec<usize> = Vec::new();
    let mut rivers: Vec<River> = Vec::new();
    // Cells the width enforcement must leave alone: the authored narrows, and the
    // springs, which are genuinely fordable and would otherwise be widened away.
    let mut named: Vec<usize> = Vec::new();
    // The subset that is a *crossing decision* — a spring is where a river starts,
    // not one of the places §2.1 asks the player to choose between.
    let mut crossings: Vec<usize> = Vec::new();

    // Water is an accent: the whole inland-water share is a handful of percent,
    // and a river four tiles wide spends it quickly. Courses are therefore
    // trimmed at the *source* end to fit the budget — a river that starts further
    // down the valley is still a river, whereas one that never reaches its outlet
    // is a puddle. A slice is held back for lakes.
    let budget = (options.water.inland_target() / 100.0 * canvas.len() as f32) as usize;
    let river_budget = budget * 94 / 100;
    let mut spent = 0usize;
    let trim = |course: &mut Vec<usize>, spent: usize| {
        let room = river_budget.saturating_sub(spent) / TRUNK_WIDTH as usize;
        if course.len() > room {
            course.drain(..course.len() - room);
        }
    };

    for trunk in 0..trunks {
        // Recomputed per trunk: a tributary should descend into water that is
        // already there, which means the flow field has to know about it.
        let flow = flow_to_sea(canvas, seed ^ (trunk as u64) << 32, relief);
        // Try a few springs and keep the **longest** course, not the first one
        // that clears the bar.
        //
        // §2.1 wants "a continuous line the player must cross somewhere", and on
        // a plain that is one flat band from edge to edge every border tile is an
        // outlet — so the highest-scoring spring is quite often one with an
        // outlet twenty tiles away, and twenty tiles of river is something you
        // walk round rather than a crossing decision. Taking the best of four
        // costs nothing and reliably finds the course that runs the length of a
        // valley.
        let least = (short / 4).max(8);
        let mut course: Vec<usize> = Vec::new();
        for _ in 0..4 {
            let Some(source) = pick_source(canvas, &flow, &sources, &mut rng) else {
                break;
            };
            sources.push(source);
            let mut candidate = course_from(&flow, canvas, source);
            trim(&mut candidate, spent);
            if candidate.len() > course.len() {
                course = candidate;
            }
        }
        if course.len() < least {
            continue;
        }
        spent += course.len() * TRUNK_WIDTH as usize;

        let wanted = options.water.crossings().clamp(2, CROSSING_SPANS.len());
        let picks = crossing_positions(course.len(), wanted, &mut rng);
        let spans: Vec<u32> = (0..picks.len()).map(|k| CROSSING_SPANS[k % 4]).collect();
        carve(canvas, &course, |k| width_at(k, course.len(), &picks, &spans));
        for &p in &picks {
            named.push(course[p]);
            crossings.push(course[p]);
        }
        named.push(course[0]);
        rivers.push(River {
            course,
            crossings: Vec::new(),
        });

        // Tributaries descend from their own source and stop where they meet
        // water — which is what makes a river system rather than parallel lines.
        for _ in 0..tributaries {
            let flow = flow_to_sea(canvas, seed ^ 0xbeef ^ (sources.len() as u64) << 40, relief);
            let Some(source) = pick_source(canvas, &flow, &sources, &mut rng) else {
                break;
            };
            sources.push(source);
            let mut course = course_from(&flow, canvas, source);
            trim(&mut course, spent);
            if course.len() < 8 {
                continue;
            }
            spent += course.len() * TRUNK_WIDTH as usize;
            let picks = crossing_positions(course.len(), 1, &mut rng);
            let spans = vec![CROSSING_SPANS[rng.gen_range(0..3)]];
            carve(canvas, &course, |k| width_at(k, course.len(), &picks, &spans));
            for &p in &picks {
                named.push(course[p]);
                crossings.push(course[p]);
            }
            named.push(course[0]);
            rivers.push(River {
                course,
                crossings: Vec::new(),
            });
        }
    }

    enforce_widths(canvas, &named);
    force_crossings(canvas, &crossings);
    diversify_spans(canvas, &crossings);

    // Record the crossings that actually survived the width enforcement, with
    // the span a bridge would really have to cover.
    for river in &mut rivers {
        for &cell in &river.course {
            if !crossings.contains(&cell) {
                continue;
            }
            if let Some(span) = cheap_span(canvas, cell) {
                river.crossings.push(RiverCrossing {
                    tile: rail_sim::ids::TileCoord {
                        x: (cell % canvas.w) as i32,
                        y: (cell / canvas.w) as i32,
                    },
                    span,
                });
            }
        }
    }
    rivers
}

/// Where along a course the narrows go: inside the span, well separated.
fn crossing_positions(len: usize, wanted: usize, rng: &mut StdRng) -> Vec<usize> {
    if len < 8 || wanted == 0 {
        return Vec::new();
    }
    // Keep clear of the source (where the river is a trickle anyway) and of the
    // mouth (where it meets the sea and the span question is meaningless).
    let lo = (len as f32 * 0.18) as usize;
    let hi = (len as f32 * 0.86) as usize;
    if hi <= lo {
        return Vec::new();
    }
    let slot = (hi - lo) / wanted.max(1);
    if slot < 4 {
        return vec![(lo + hi) / 2];
    }
    (0..wanted)
        .map(|k| lo + slot * k + rng.gen_range(slot / 4..slot * 3 / 4))
        .collect()
}

/// River width at course position `k`.
///
/// The trunk width everywhere, except: a three-cell narrows at each named
/// crossing (so the span the player measures is the span that was authored, not
/// whatever a bend happened to leave), easing back out over two more cells; and a
/// tapered head, because a river that begins as a four-tile channel out of
/// nowhere reads as a canal.
fn width_at(k: usize, _len: usize, picks: &[usize], spans: &[u32]) -> u32 {
    let mut width = TRUNK_WIDTH;
    for (i, &pick) in picks.iter().enumerate() {
        let span = spans.get(i).copied().unwrap_or(1);
        let away = k.abs_diff(pick) as u32;
        if away <= 1 {
            width = width.min(span);
        } else if away <= 3 {
            width = width.min(span + away - 1);
        }
    }
    width.min(HEAD_TAPER.get(k).copied().unwrap_or(TRUNK_WIDTH))
}

/// Width of the first few cells of a course: a spring, not a canal mouth.
const HEAD_TAPER: [u32; 3] = [1, 2, 3];

/// Fill what is left of the §2.1 inland-water budget with lakes in the low ground.
///
/// Lakes go where a basin already is — cheap inside, costly to enter (§2.2) —
/// so the water reads as something the landscape explains rather than as blue
/// paint applied to hit a number. "Where a basin is" now means the continuous
/// relief rather than the drawn bands: on an open plain every tile is band 0 and
/// the bands have nothing to say about which hollow is the lowest.
pub(crate) fn place_lakes(canvas: &mut Canvas, seed: u64, quota: usize, relief: &[f32]) {
    if quota == 0 {
        return;
    }
    let quota = quota.min(canvas.len() / LAKE_SHARE_DENOM);
    let mut rng = stream(seed, salt::LAKE);
    let from_water = canvas.distance_to(|i| canvas.surface[i].is_water());
    let ground = |i: usize| relief.get(i).copied().unwrap_or(canvas.band[i] as f32);
    let mut remaining = quota;

    for _ in 0..3 {
        if remaining == 0 {
            break;
        }
        // A basin floor: low, and a long way from anything already wet.
        let mut best: Option<(i64, usize)> = None;
        for (i, &distance) in from_water.iter().enumerate() {
            if canvas.surface[i] != Surface::Land || distance < 5 {
                continue;
            }
            let x = (i % canvas.w) as i32;
            let y = (i / canvas.w) as i32;
            if x < 4 || y < 4 || x >= canvas.w as i32 - 4 || y >= canvas.h as i32 - 4 {
                continue;
            }
            let score = from_water[i] as i64 * 2 - (ground(i) * 8.0) as i64
                + rng.gen_range(0..3) as i64;
            if best.is_none_or(|(b, _)| score > b) {
                best = Some((score, i));
            }
        }
        let Some((_, centre)) = best else { break };
        let (cx, cy) = ((centre % canvas.w) as i32, (centre / canvas.w) as i32);

        // Flood the low ground outward from the centre until the quota is met.
        let size = remaining.min(quota / 2 + 8).max(6);
        let mut filled = 0usize;
        let mut frontier: BinaryHeap<Reverse<(i32, usize)>> = BinaryHeap::new();
        frontier.push(Reverse((0, centre)));
        let mut seen = vec![false; canvas.len()];
        seen[centre] = true;
        while let Some(Reverse((_, i))) = frontier.pop() {
            if filled >= size {
                break;
            }
            if canvas.surface[i] != Surface::Land {
                continue;
            }
            let x = (i % canvas.w) as i32;
            let y = (i / canvas.w) as i32;
            // Never let a lake reach the sea: that would make it a bay.
            if [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .any(|(dx, dy)| {
                    canvas
                        .idx(x + dx, y + dy)
                        .is_some_and(|n| canvas.surface[n] == Surface::Sea)
                })
            {
                continue;
            }
            canvas.surface[i] = Surface::Lake;
            filled += 1;
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                if let Some(n) = canvas.idx(x + dx, y + dy) {
                    if !seen[n] && canvas.surface[n] == Surface::Land {
                        seen[n] = true;
                        // Lowest ground first, and *nearest* to break the tie.
                        // Without the second term a lake on flat ground grew in
                        // whatever order the cells happened to be numbered in,
                        // which on an open plain is one tile high and fifty
                        // long — a worm along a row, not a pool.
                        let reach = (x + dx - cx).abs() + (y + dy - cy).abs();
                        frontier.push(Reverse(((ground(n) * 64.0) as i32 + reach, n)));
                    }
                }
            }
        }
        remaining = remaining.saturating_sub(filled);
        if filled == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_crossing_narrows_and_the_rest_does_not() {
        let picks = [20usize];
        let spans = [1u32];
        // Three cells at the authored span, so the width the player measures is
        // the width that was chosen rather than whatever a bend left behind.
        assert_eq!(width_at(19, 60, &picks, &spans), 1);
        assert_eq!(width_at(20, 60, &picks, &spans), 1);
        assert_eq!(width_at(21, 60, &picks, &spans), 1);
        assert_eq!(width_at(22, 60, &picks, &spans), 2);
        assert_eq!(width_at(23, 60, &picks, &spans), 3);
        assert_eq!(width_at(24, 60, &picks, &spans), TRUNK_WIDTH);
        assert_eq!(width_at(10, 60, &picks, &spans), TRUNK_WIDTH);
        // …and a spring rather than a canal mouth at the head.
        assert_eq!(width_at(0, 60, &picks, &spans), 1);
    }

    /// The trunk used to be sized to refuse a bridge outright. It is now sized
    /// to refuse a *cheap* one: a wall is not a decision, and a player who
    /// wants to cross away from the narrows should be able to buy their way
    /// over at the premium end of the ladder.
    #[test]
    fn the_trunk_width_is_the_narrowest_that_refuses_a_cheap_bridge() {
        assert_eq!(TRUNK_WIDTH, CHEAP_BRIDGE_SPAN + 1);
        assert!(CROSSING_SPANS.iter().all(|s| *s <= CHEAP_BRIDGE_SPAN));
        // …and the trunk itself is still bridgeable, at a price.
        let spans: Vec<u32> = (1..=rail_sim::MAX_BRIDGE_SPAN).collect();
        assert!(
            spans.contains(&TRUNK_WIDTH),
            "a trunk nobody can cross is a wall, not a decision"
        );
        assert!(
            rail_sim::bridge_cost_for_span(TRUNK_WIDTH)
                > rail_sim::bridge_cost_for_span(CHEAP_BRIDGE_SPAN),
            "crossing the trunk has to cost more than crossing a narrows"
        );
        // Adjacent crossings differ, so a player always has two prices to weigh.
        for pair in CROSSING_SPANS.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
    }

    #[test]
    fn crossings_are_spread_along_the_course() {
        let mut rng = stream(7, 0);
        let picks = crossing_positions(80, 3, &mut rng);
        assert_eq!(picks.len(), 3);
        for pair in picks.windows(2) {
            assert!(pair[1] > pair[0] + 4, "crossings bunched: {picks:?}");
        }
        assert!(picks[0] >= 14 && *picks.last().unwrap() <= 69, "{picks:?}");
    }

    /// Four wide is trunk: bridgeable, but only on the premium rungs, so the
    /// generator does not count it as one of the authored narrows.
    #[test]
    fn a_four_wide_channel_refuses_a_cheap_bridge_and_a_three_wide_accepts_one() {
        let mut canvas = Canvas::new(12, 3);
        for y in 0..3i32 {
            for x in 4..8i32 {
                let i = canvas.at(x, y);
                canvas.surface[i] = Surface::River;
            }
        }
        assert_eq!(cheap_span(&canvas, canvas.at(5, 1)), None);
        let i = canvas.at(7, 1);
        canvas.surface[i] = Surface::Land;
        assert_eq!(cheap_span(&canvas, canvas.at(5, 1)), Some(3));
    }
}
