//! Where the world puts things — and, above all, the opening beat.
//!
//! Design 02 §4.1 is the sharpest instruction in the brief:
//!
//! > A **home town** near the map centre … a **second destination within about
//! > eight to twelve tiles**, across terrain that poses a small, legible question
//! > — a stream to bridge, a low rise to skirt. … Scattering the starting anchors
//! > to the map's extremes produces the opposite: a long, expensive, unrewarded
//! > haul as the first thing the player ever does.
//!
//! Farthest-point sampling — which is what anchor placement does today — has
//! *maximum separation* as its objective, and §4.1 names that as the worst
//! possible opening. The generator cannot fix the sampler from here, but it does
//! know which two tiles the game should open on, so it says so: [`choose_sites`]
//! picks the pair and hands it over in [`crate::MapFeatures`].
//!
//! The rest of the module is §4.2: sites on sensible ground, clear of the map
//! edge, and industries where their resource makes sense — a quarry against rock,
//! a harbour on a bay.

use rail_sim::ids::TileCoord;
use rand::rngs::StdRng;
use rand::Rng;

use crate::features::{SiteHint, SiteKind, Surface};
use crate::options::MapGenOptions;

use super::field::{salt, stream, Canvas, ROCK_BAND};

/// Closest a site may sit to the map edge (§4.2: "room to build around them").
const EDGE_MARGIN: i32 = 4;

/// The opening pair's separation, in tiles (§4.1).
pub(crate) const OPENING_MIN: f32 = 8.0;
pub(crate) const OPENING_MAX: f32 = 12.0;

/// Everything [`choose_sites`] found.
pub(crate) struct Sites {
    pub(crate) home: Option<TileCoord>,
    pub(crate) near: Option<TileCoord>,
    pub(crate) hints: Vec<SiteHint>,
}

struct Ground {
    /// Buildable-land component id per cell, `u32::MAX` for everything else.
    component: Vec<u32>,
    /// The component with the most tiles in it.
    mainland: u32,
    /// 4-neighbours sharing this cell's band: 4 is properly flat.
    level: Vec<u8>,
}

fn survey(canvas: &Canvas) -> Ground {
    let len = canvas.len();
    let mut component = vec![u32::MAX; len];
    let buildable =
        |i: usize| canvas.surface[i] == Surface::Land && canvas.band[i] < ROCK_BAND;

    let mut next = 0u32;
    let mut mainland = u32::MAX;
    let mut best = 0usize;
    for start in 0..len {
        if component[start] != u32::MAX || !buildable(start) {
            continue;
        }
        let id = next;
        next += 1;
        let mut stack = vec![start];
        component[start] = id;
        let mut size = 0usize;
        while let Some(i) = stack.pop() {
            size += 1;
            let x = (i % canvas.w) as i32;
            let y = (i / canvas.w) as i32;
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let Some(n) = canvas.idx(x + dx, y + dy) else {
                    continue;
                };
                if component[n] == u32::MAX && buildable(n) {
                    component[n] = id;
                    stack.push(n);
                }
            }
        }
        if size > best {
            best = size;
            mainland = id;
        }
    }

    let mut level = vec![0u8; len];
    for y in 0..canvas.h as i32 {
        for x in 0..canvas.w as i32 {
            let i = canvas.at(x, y);
            level[i] = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .filter(|(dx, dy)| {
                    canvas
                        .idx(x + dx, y + dy)
                        .is_some_and(|n| canvas.band[n] == canvas.band[i])
                })
                .count() as u8;
        }
    }

    Ground {
        component,
        mainland,
        level,
    }
}

/// Tiles a station could stand on without looking like a bug (§4.2).
fn sensible(canvas: &Canvas, ground: &Ground, i: usize) -> bool {
    let x = (i % canvas.w) as i32;
    let y = (i / canvas.w) as i32;
    x >= EDGE_MARGIN
        && y >= EDGE_MARGIN
        && x < canvas.w as i32 - EDGE_MARGIN
        && y < canvas.h as i32 - EDGE_MARGIN
        && ground.component[i] == ground.mainland
        && ground.level[i] >= 3
}

/// Tiles on the straight line between two points (Bresenham).
fn segment(a: TileCoord, b: TileCoord) -> Vec<TileCoord> {
    let mut out = Vec::new();
    let (mut x, mut y) = (a.x, a.y);
    let dx = (b.x - a.x).abs();
    let dy = -(b.y - a.y).abs();
    let sx = if a.x < b.x { 1 } else { -1 };
    let sy = if a.y < b.y { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        out.push(TileCoord { x, y });
        if x == b.x && y == b.y {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
    out
}

/// How good a question the ground between two sites asks (§4.1).
///
/// The prize goes to a short stretch of bridgeable water — "a stream to bridge"
/// — then to a rise the player can climb or skirt. A dead-flat, dry line scores
/// negative: that is the failure state the brief names, terrain as wallpaper.
fn question_score(canvas: &Canvas, from: TileCoord, to: TileCoord) -> i32 {
    let line = segment(from, to);
    let mut water = 0i32;
    let mut run = 0i32;
    let mut worst_run = 0i32;
    let mut steps = 0i32;
    let mut previous: Option<i8> = None;

    for tile in &line {
        let Some(i) = canvas.idx(tile.x, tile.y) else {
            continue;
        };
        if canvas.surface[i].is_water() {
            water += 1;
            run += 1;
            worst_run = worst_run.max(run);
        } else {
            run = 0;
        }
        if let Some(p) = previous {
            if canvas.band[i] != p {
                steps += 1;
            }
        }
        previous = Some(canvas.band[i]);
    }

    let mut score = 0;
    if water > 0 {
        // A crossing the player can actually afford in minute one.
        score += if worst_run <= rail_sim::MAX_BRIDGE_SPAN as i32 {
            45 - water * 2
        } else {
            -60
        };
    }
    score += (steps.min(4)) * 9;
    if water == 0 && steps == 0 {
        score -= 30;
    }
    score
}

/// Pick the opening pair and the sites the world can grow into.
pub(crate) fn choose_sites(canvas: &Canvas, seed: u64, options: MapGenOptions) -> Sites {
    let mut rng = stream(seed, salt::SITES);
    let ground = survey(canvas);
    if ground.mainland == u32::MAX {
        return Sites {
            home: None,
            near: None,
            hints: Vec::new(),
        };
    }

    let cx = (canvas.w as i32 - 1) / 2;
    let cy = (canvas.h as i32 - 1) / 2;
    let coord = |i: usize| TileCoord {
        x: (i % canvas.w) as i32,
        y: (i / canvas.w) as i32,
    };

    // Home: flat, low, buildable, and as near the middle as that allows.
    let mut home: Option<(i32, usize)> = None;
    for i in 0..canvas.len() {
        if !sensible(canvas, &ground, i) {
            continue;
        }
        let c = coord(i);
        let from_centre = (c.x - cx).abs().max((c.y - cy).abs());
        if from_centre > canvas.w.min(canvas.h) as i32 / 4 {
            continue;
        }
        let score = 60 - from_centre * 4 + ground.level[i] as i32 * 6
            - canvas.band[i] as i32 * 8
            + rng.gen_range(0..5);
        if home.is_none_or(|(b, _)| score > b) {
            home = Some((score, i));
        }
    }
    let Some((_, home_index)) = home else {
        return Sites {
            home: None,
            near: None,
            hints: Vec::new(),
        };
    };
    let home_tile = coord(home_index);

    // Near: eight to twelve tiles out, across the most interesting small
    // question the map has to offer at that range.
    let mut near: Option<(i32, usize)> = None;
    for i in 0..canvas.len() {
        if !sensible(canvas, &ground, i) {
            continue;
        }
        let c = coord(i);
        let d = (((c.x - home_tile.x).pow(2) + (c.y - home_tile.y).pow(2)) as f32).sqrt();
        if !(OPENING_MIN..=OPENING_MAX).contains(&d) {
            continue;
        }
        let score = question_score(canvas, home_tile, c) + ground.level[i] as i32 * 4
            - canvas.band[i] as i32 * 5
            + rng.gen_range(0..5);
        if near.is_none_or(|(b, _)| score > b) {
            near = Some((score, i));
        }
    }
    let near_tile = near.map(|(_, i)| coord(i));

    let hints = growth_sites(canvas, &ground, options, home_tile, near_tile, &mut rng);

    Sites {
        home: Some(home_tile),
        near: near_tile,
        hints,
    }
}

/// Sites the world can grow into: towns spread by the Resources option, plus
/// industries placed where their resource makes sense (§4.2, §4.3).
fn growth_sites(
    canvas: &Canvas,
    ground: &Ground,
    options: MapGenOptions,
    home: TileCoord,
    near: Option<TileCoord>,
    rng: &mut StdRng,
) -> Vec<SiteHint> {
    let coord = |i: usize| TileCoord {
        x: (i % canvas.w) as i32,
        y: (i / canvas.w) as i32,
    };
    let candidates: Vec<usize> = (0..canvas.len())
        .filter(|&i| sensible(canvas, ground, i))
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }

    // Clustered draws every town from a handful of districts; Scattered spreads
    // them over the whole landmass. §4.2 wants a *distribution*, so neither is a
    // grid — this is greedy spacing inside whichever region is allowed.
    let mut allowed = candidates.clone();
    if let Some(clusters) = options.resources.clusters() {
        let mut centres: Vec<TileCoord> = Vec::new();
        for _ in 0..clusters {
            let pick = candidates[rng.gen_range(0..candidates.len())];
            centres.push(coord(pick));
        }
        let radius = canvas.w.min(canvas.h) as i32 / 4;
        allowed.retain(|&i| {
            let c = coord(i);
            centres
                .iter()
                .any(|k| (k.x - c.x).abs() + (k.y - c.y).abs() <= radius)
        });
        if allowed.len() < 4 {
            allowed = candidates.clone();
        }
    }

    let mut chosen: Vec<TileCoord> = vec![home];
    chosen.extend(near);
    let mut hints: Vec<SiteHint> = Vec::new();

    // Greedy spacing, but capped: a *distribution* of gaps, not an extremum
    // (§4.2). Each pick is the best-spaced tile among a shortlist, so the map
    // gets a few close pairs and a few far ones instead of five corners.
    for _ in 0..6 {
        let mut best: Option<(i32, usize)> = None;
        for &i in &allowed {
            let c = coord(i);
            let apart = chosen
                .iter()
                .map(|p| (p.x - c.x).abs() + (p.y - c.y).abs())
                .min()
                .unwrap_or(i32::MAX);
            if apart < 8 {
                continue;
            }
            let score = apart.min(24) * 3 + ground.level[i] as i32 * 4
                - canvas.band[i] as i32 * 4
                + rng.gen_range(0..12);
            if best.is_none_or(|(b, _)| score > b) {
                best = Some((score, i));
            }
        }
        let Some((_, index)) = best else { break };
        let tile = coord(index);
        chosen.push(tile);
        hints.push(SiteHint {
            tile,
            kind: SiteKind::Town,
        });
    }

    // Industries where their reason is visible from the map.
    let count_near = |i: usize, radius: i32, pred: &dyn Fn(usize) -> bool| -> i32 {
        let c = coord(i);
        let mut n = 0;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if let Some(k) = canvas.idx(c.x + dx, c.y + dy) {
                    if pred(k) {
                        n += 1;
                    }
                }
            }
        }
        n
    };

    let add_best = |kind: SiteKind,
                        score: &dyn Fn(usize) -> Option<i32>,
                        chosen: &mut Vec<TileCoord>,
                        hints: &mut Vec<SiteHint>| {
        let mut best: Option<(i32, usize)> = None;
        for &i in &candidates {
            let c = coord(i);
            if chosen
                .iter()
                .any(|p| (p.x - c.x).abs() + (p.y - c.y).abs() < 6)
            {
                continue;
            }
            if let Some(s) = score(i) {
                if best.is_none_or(|(b, _)| s > b) {
                    best = Some((s, i));
                }
            }
        }
        if let Some((_, index)) = best {
            let tile = coord(index);
            chosen.push(tile);
            hints.push(SiteHint { tile, kind });
        }
    };

    let is_rock = |i: usize| canvas.surface[i] == Surface::Land && canvas.band[i] >= ROCK_BAND;
    let is_sea = |i: usize| canvas.surface[i] == Surface::Sea;

    add_best(
        SiteKind::Quarry,
        &|i| {
            let rock = count_near(i, 2, &is_rock);
            (rock >= 3).then_some(rock * 5)
        },
        &mut chosen,
        &mut hints,
    );
    add_best(
        SiteKind::Harbour,
        &|i| {
            let sea = count_near(i, 3, &is_sea);
            (sea >= 6 && count_near(i, 1, &is_sea) >= 1).then_some(sea * 2)
        },
        &mut chosen,
        &mut hints,
    );
    for _ in 0..2 {
        add_best(
            SiteKind::Forest,
            &|i| {
                (canvas.band[i] <= 2 && count_near(i, 2, &is_rock) == 0)
                    .then_some(40 - canvas.band[i] as i32 * 10 + ground.level[i] as i32 * 3)
            },
            &mut chosen,
            &mut hints,
        );
    }

    hints
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_walks_from_end_to_end() {
        let line = segment(TileCoord { x: 0, y: 0 }, TileCoord { x: 3, y: 1 });
        assert_eq!(line.first(), Some(&TileCoord { x: 0, y: 0 }));
        assert_eq!(line.last(), Some(&TileCoord { x: 3, y: 1 }));
        assert_eq!(line.len(), 4);
    }

    #[test]
    fn flat_and_dry_scores_worse_than_a_stream() {
        let mut canvas = Canvas::new(16, 3);
        let flat = question_score(
            &canvas,
            TileCoord { x: 1, y: 1 },
            TileCoord { x: 12, y: 1 },
        );
        for y in 0..3i32 {
            let i = canvas.at(6, y);
            canvas.surface[i] = Surface::River;
        }
        let stream = question_score(
            &canvas,
            TileCoord { x: 1, y: 1 },
            TileCoord { x: 12, y: 1 },
        );
        assert!(flat < 0, "a dead flat dry line is wallpaper: {flat}");
        assert!(stream > flat + 40, "stream {stream} vs flat {flat}");
    }

    #[test]
    fn an_unbridgeable_channel_is_not_an_opening_question() {
        let mut canvas = Canvas::new(16, 3);
        for y in 0..3i32 {
            for x in 5..11i32 {
                let i = canvas.at(x, y);
                canvas.surface[i] = Surface::River;
            }
        }
        let score = question_score(
            &canvas,
            TileCoord { x: 1, y: 1 },
            TileCoord { x: 14, y: 1 },
        );
        assert!(score < -30, "a six-wide river is a wall, not a beat: {score}");
    }
}
