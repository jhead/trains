//! Measuring a finished map against the design's own numbers.
//!
//! Design 02 §2.1 states a composition target and §2.1/§2.2 state what a river
//! and a ridge have to offer. Those are testable claims, so they are measured
//! here rather than asserted in prose — the generator's tests read this module,
//! and so can the New Map screen's readouts.
//!
//! Every function works on a bare [`MapGrid`]. Where [`MapFeatures`] is present
//! it is believed (generation knows a river from a bay); where it is absent the
//! measurement falls back to geometry, so a grid rebuilt from a save still reads.

use rail_sim::ids::TileCoord;
use rail_sim::{CHEAP_BRIDGE_SPAN, MAX_BRIDGE_SPAN, MOUNTAIN_HEIGHT_MIN};

use crate::features::{RiverCrossing, Surface};
use crate::grid::MapGrid;

/// The four surface shares of a map, in percent of all tiles.
///
/// Targets are the **playtest revision** of brief 02 §2.1 — a broad, open,
/// mostly buildable landscape in the shape of Locomotion and RCT, where the
/// constraint is the ground rather than a coastline. See
/// [`crate::MapGenOptions::composition_targets`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Composition {
    /// Land a track may be laid on: **85–92%**.
    pub buildable: f32,
    /// Rivers and lakes — the crossing decision: **4–8%**.
    pub inland_water: f32,
    /// Open sea: **0–4%**, and most maps have none at all.
    pub sea: f32,
    /// Impassable rock — hard walls that force real detours: **4–8%**.
    pub rock: f32,
}

impl Composition {
    /// Whether every row sits inside its band.
    pub fn meets_design_targets(&self) -> bool {
        (85.0..=92.0).contains(&self.buildable)
            && (4.0..=8.0).contains(&self.inland_water)
            && (0.0..=4.0).contains(&self.sea)
            && (4.0..=8.0).contains(&self.rock)
    }
}

/// Land tiles at or above this height are impassable rock.
///
/// Taken from `rail_sim` rather than restated, so terrain that *looks* like a
/// wall and terrain that *is* a wall can never drift apart.
pub const ROCK_HEIGHT_MIN: i8 = MOUNTAIN_HEIGHT_MIN;

/// Measure a map against §2.1.
pub fn composition(map: &MapGrid) -> Composition {
    let total = map.tiles().len();
    if total == 0 {
        return Composition::default();
    }
    let classes = surfaces(map);
    let mut buildable = 0usize;
    let mut inland = 0usize;
    let mut sea = 0usize;
    let mut rock = 0usize;

    for (index, tile) in map.tiles().iter().enumerate() {
        if tile.water {
            match classes[index] {
                Surface::Sea => sea += 1,
                _ => inland += 1,
            }
        } else if tile.height >= ROCK_HEIGHT_MIN {
            rock += 1;
        } else {
            buildable += 1;
        }
    }

    let pct = |n: usize| (n as f32) * 100.0 / (total as f32);
    Composition {
        buildable: pct(buildable),
        inland_water: pct(inland),
        sea: pct(sea),
        rock: pct(rock),
    }
}

/// Per-tile surface class: the generator's own record when it kept one, and a
/// border flood fill (water touching the frame is sea) when it did not.
pub fn surfaces(map: &MapGrid) -> Vec<Surface> {
    let len = map.tiles().len();
    if map.features().describes(len) {
        return map.features().surface.clone();
    }

    let w = map.width as i32;
    let h = map.height as i32;
    let mut out = vec![Surface::Land; len];
    for (index, tile) in map.tiles().iter().enumerate() {
        if tile.water {
            out[index] = Surface::Lake;
        }
    }

    // Flood sea inward from every border water tile.
    let idx = |x: i32, y: i32| (y * w + x) as usize;
    let mut stack: Vec<(i32, i32)> = Vec::new();
    for x in 0..w {
        for y in [0, h - 1] {
            if out[idx(x, y)] == Surface::Lake {
                out[idx(x, y)] = Surface::Sea;
                stack.push((x, y));
            }
        }
    }
    for y in 0..h {
        for x in [0, w - 1] {
            if out[idx(x, y)] == Surface::Lake {
                out[idx(x, y)] = Surface::Sea;
                stack.push((x, y));
            }
        }
    }
    while let Some((cx, cy)) = stack.pop() {
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (cx + dx, cy + dy);
            if nx < 0 || ny < 0 || nx >= w || ny >= h {
                continue;
            }
            if out[idx(nx, ny)] == Surface::Lake {
                out[idx(nx, ny)] = Surface::Sea;
                stack.push((nx, ny));
            }
        }
    }
    out
}

/// Whether track may be laid on this tile at all (§3.1's last row).
#[inline]
pub fn is_buildable(map: &MapGrid, coord: TileCoord) -> bool {
    map.get(coord)
        .is_some_and(|t| !t.water && t.height < ROCK_HEIGHT_MIN)
}

/// Shortest bridgeable run through a water tile, with the land it would join.
///
/// Returns the span in tiles when one axis crosses in [`MAX_BRIDGE_SPAN`] or
/// fewer water tiles and lands on buildable ground both sides — the same
/// question `rail_sim`'s placement rules ask.
pub fn crossing_span(map: &MapGrid, coord: TileCoord) -> Option<u32> {
    if !map.get(coord).is_some_and(|t| t.water) {
        return None;
    }
    [(1, 0), (0, 1)]
        .into_iter()
        .filter_map(|(dx, dy)| axis_span(map, coord, dx, dy))
        .min()
}

/// [`crossing_span`], restricted to spans on the cheap rungs of the bridge
/// ladder ([`CHEAP_BRIDGE_SPAN`]).
///
/// This is the one that answers "where can the player cross", because the wide
/// answer is now "anywhere on the trunk, for a mid-game sum". Scouting is about
/// the narrows.
pub fn cheap_crossing_span(map: &MapGrid, coord: TileCoord) -> Option<u32> {
    crossing_span(map, coord).filter(|span| *span <= CHEAP_BRIDGE_SPAN)
}

/// Water run through `coord` along one axis, if both ends are buildable land.
fn axis_span(map: &MapGrid, coord: TileCoord, dx: i32, dy: i32) -> Option<u32> {
    let mut span = 1u32;
    let mut lo = TileCoord {
        x: coord.x - dx,
        y: coord.y - dy,
    };
    while map.get(lo).is_some_and(|t| t.water) {
        span += 1;
        lo = TileCoord {
            x: lo.x - dx,
            y: lo.y - dy,
        };
    }
    let mut hi = TileCoord {
        x: coord.x + dx,
        y: coord.y + dy,
    };
    while map.get(hi).is_some_and(|t| t.water) {
        span += 1;
        hi = TileCoord {
            x: hi.x + dx,
            y: hi.y + dy,
        };
    }
    if span > MAX_BRIDGE_SPAN || !is_buildable(map, lo) || !is_buildable(map, hi) {
        return None;
    }
    Some(span)
}

/// Every distinct place the water on this map can be bridged *cheaply*.
///
/// Adjacent narrow tiles are one crossing, not four: what the player chooses
/// between is *places*, and a four-tile-wide ford is one place.
///
/// The cheap tier is the filter because the premium tier is not a place — a
/// trunk is bridgeable end to end now, so listing every spot that admits a deck
/// would list the river. What is scarce, and therefore worth scouting, is the
/// narrows.
pub fn river_crossings(map: &MapGrid) -> Vec<RiverCrossing> {
    let w = map.width as i32;
    let h = map.height as i32;
    let idx = |x: i32, y: i32| (y * w + x) as usize;

    let mut span_at = vec![0u32; map.tiles().len()];
    for y in 0..h {
        for x in 0..w {
            let i = idx(x, y);
            if let Some(span) = cheap_crossing_span(map, TileCoord { x, y }) {
                span_at[i] = span;
            }
        }
    }

    let mut seen = vec![false; map.tiles().len()];
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if seen[idx(x, y)] || span_at[idx(x, y)] == 0 {
                continue;
            }
            // One crossing = one 8-connected clump of bridgeable water.
            let mut stack = vec![(x, y)];
            seen[idx(x, y)] = true;
            let mut best = (u32::MAX, TileCoord { x, y });
            while let Some((cx, cy)) = stack.pop() {
                let span = span_at[idx(cx, cy)];
                if span < best.0 {
                    best = (span, TileCoord { x: cx, y: cy });
                }
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (nx, ny) = (cx + dx, cy + dy);
                        if nx < 0 || ny < 0 || nx >= w || ny >= h {
                            continue;
                        }
                        if seen[idx(nx, ny)] || span_at[idx(nx, ny)] == 0 {
                            continue;
                        }
                        seen[idx(nx, ny)] = true;
                        stack.push((nx, ny));
                    }
                }
            }
            out.push(RiverCrossing {
                tile: best.1,
                span: best.0,
            });
        }
    }
    out
}

/// How far either side of a tile rock still counts as walling it in.
///
/// A designed saddle is a few tiles across plus the shoulders the crest sheds
/// around it, so a two-tile reach — which is what the New Map readout uses —
/// misses most real passes while still catching every one-tile nick in a crest.
pub const PASS_REACH: i32 = 3;

/// Gaps through high ground: buildable tiles pinched between impassable rock on
/// opposite sides, grouped so a wide saddle reads as one pass.
pub fn ridge_passes(map: &MapGrid) -> Vec<TileCoord> {
    let w = map.width as i32;
    let h = map.height as i32;
    let idx = |x: i32, y: i32| (y * w + x) as usize;
    let blocked = |x: i32, y: i32| {
        map.get(TileCoord { x, y })
            .is_some_and(|t| !t.water && t.height >= ROCK_HEIGHT_MIN)
    };

    let mut is_pass = vec![false; map.tiles().len()];
    for y in 0..h {
        for x in 0..w {
            if !is_buildable(map, TileCoord { x, y }) {
                continue;
            }
            // Four axes, not two: a ridge runs at whatever angle it was drawn
            // at, and a saddle in a diagonal wall is still a saddle.
            let pinched = [(1, 0), (0, 1), (1, 1), (1, -1)].into_iter().any(|(dx, dy)| {
                (1..=PASS_REACH).any(|d| blocked(x - dx * d, y - dy * d))
                    && (1..=PASS_REACH).any(|d| blocked(x + dx * d, y + dy * d))
            });
            if pinched {
                is_pass[idx(x, y)] = true;
            }
        }
    }

    let mut seen = vec![false; map.tiles().len()];
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if seen[idx(x, y)] || !is_pass[idx(x, y)] {
                continue;
            }
            let mut stack = vec![(x, y)];
            seen[idx(x, y)] = true;
            let (mut sx, mut sy, mut n) = (0i64, 0i64, 0i64);
            while let Some((cx, cy)) = stack.pop() {
                sx += cx as i64;
                sy += cy as i64;
                n += 1;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (nx, ny) = (cx + dx, cy + dy);
                        if nx < 0 || ny < 0 || nx >= w || ny >= h {
                            continue;
                        }
                        if seen[idx(nx, ny)] || !is_pass[idx(nx, ny)] {
                            continue;
                        }
                        seen[idx(nx, ny)] = true;
                        stack.push((nx, ny));
                    }
                }
            }
            out.push(TileCoord {
                x: (sx / n) as i32,
                y: (sy / n) as i32,
            });
        }
    }
    out
}

// -- Relief: how *often* the ground changes height ---------------------------

/// Land at or above this height reads as hills — `rail_sim`'s 3× band.
pub const HILL_HEIGHT_MIN: i8 = 7;

/// A hill clump smaller than this is a bump, not a system.
const SYSTEM_MIN_TILES: usize = 8;

/// Length, in tiles, of the sight lines [`relief`] samples.
const SIGHT_LINE: i32 = 30;

/// How many sight lines are walked. Enough that the mean is stable to ~0.02.
const SIGHT_SAMPLES: usize = 600;

/// How often the ground changes height, as opposed to how much of it is high.
///
/// Playtest: *"I'm constantly fighting terrain … just mountains and up/down
/// everywhere."* That is a complaint about **frequency**, and the composition
/// numbers in [`Composition`] cannot see it — a map can hit every share in §2.1
/// and still be a rash of one-tile steps. These four are what a player feels:
/// how much ground is uninterrupted, how many separate things are in the way,
/// how often a straight line runs into one, and where the height actually sits.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Relief {
    /// Share of land tiles with no height step to any of their eight
    /// neighbours — open country you can build across without thinking.
    pub flat_share: f32,
    /// Connected hill-or-higher systems of real size: the ridges and massifs.
    pub systems: usize,
    /// Hill clumps too small to be one — scattered bumps.
    pub specks: usize,
    /// Mean band boundaries a straight [`SIGHT_LINE`]-tile line over land
    /// crosses. Under one means the ground is usually just ground.
    pub crossings: f32,
    /// Share of *land* in each elevation band, band 0 first.
    pub bands: [f32; 6],
}

/// Which elevation band a tile height belongs to.
///
/// `rail_map` generates only the six band heights, so anything else — a save
/// from a hand-built test grid — takes the nearest band below.
fn band_of(height: i8) -> usize {
    match height {
        h if h >= 16 => 5,
        h if h >= 13 => 4,
        h if h >= 10 => 3,
        h if h >= 7 => 2,
        h if h >= 4 => 1,
        _ => 0,
    }
}

/// Measure a map's relief — see [`Relief`].
pub fn relief(map: &MapGrid) -> Relief {
    let w = map.width as i32;
    let h = map.height as i32;
    if w < 2 || h < 2 {
        return Relief::default();
    }
    let idx = |x: i32, y: i32| (y * w + x) as usize;
    let dry = |x: i32, y: i32| map.get(TileCoord { x, y }).is_some_and(|t| !t.water);

    // 1. Flat share, and the band histogram, in one sweep.
    let mut land = 0usize;
    let mut flat = 0usize;
    let mut bands = [0usize; 6];
    for y in 0..h {
        for x in 0..w {
            let tile = map.tile(TileCoord { x, y });
            if tile.water {
                continue;
            }
            land += 1;
            bands[band_of(tile.height)] += 1;
            let level = (-1i32..=1)
                .flat_map(|dy| (-1i32..=1).map(move |dx| (dx, dy)))
                .filter(|&(dx, dy)| (dx, dy) != (0, 0))
                .all(|(dx, dy)| {
                    map.get(TileCoord {
                        x: x + dx,
                        y: y + dy,
                    })
                    .is_none_or(|n| n.height == tile.height)
                });
            if level {
                flat += 1;
            }
        }
    }

    // 2. Hill systems: 8-connected clumps of hill-or-higher ground.
    let hill = |x: i32, y: i32| {
        map.get(TileCoord { x, y })
            .is_some_and(|t| !t.water && t.height >= HILL_HEIGHT_MIN)
    };
    let mut seen = vec![false; (w * h) as usize];
    let mut systems = 0usize;
    let mut specks = 0usize;
    for y in 0..h {
        for x in 0..w {
            if seen[idx(x, y)] || !hill(x, y) {
                continue;
            }
            let mut stack = vec![(x, y)];
            seen[idx(x, y)] = true;
            let mut size = 0usize;
            while let Some((cx, cy)) = stack.pop() {
                size += 1;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (nx, ny) = (cx + dx, cy + dy);
                        if nx < 0 || ny < 0 || nx >= w || ny >= h {
                            continue;
                        }
                        if seen[idx(nx, ny)] || !hill(nx, ny) {
                            continue;
                        }
                        seen[idx(nx, ny)] = true;
                        stack.push((nx, ny));
                    }
                }
            }
            if size >= SYSTEM_MIN_TILES {
                systems += 1;
            } else {
                specks += 1;
            }
        }
    }

    // 3. Sight lines. Drawn from the map's own seed so the number is a property
    //    of the world, not of whoever measured it.
    let mut state = map.seed ^ 0x5ee5_1941_7000;
    let mut roll = |n: u32| -> i32 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((state >> 33) % n as u64) as i32
    };
    let mut walked = 0usize;
    let mut steps = 0usize;
    for _ in 0..SIGHT_SAMPLES * 8 {
        if walked >= SIGHT_SAMPLES {
            break;
        }
        let ax = roll(w as u32);
        let ay = roll(h as u32);
        // A direction on the SIGHT_LINE-radius circle, quantised to whole tiles.
        let turn = roll(360) as f32 * std::f32::consts::TAU / 360.0;
        let bx = ax + (turn.cos() * SIGHT_LINE as f32).round() as i32;
        let by = ay + (turn.sin() * SIGHT_LINE as f32).round() as i32;
        if bx < 0 || by < 0 || bx >= w || by >= h || !dry(ax, ay) || !dry(bx, by) {
            continue;
        }
        walked += 1;
        let mut previous: Option<i8> = None;
        for point in line(TileCoord { x: ax, y: ay }, TileCoord { x: bx, y: by }) {
            let Some(tile) = map.get(point) else { continue };
            if tile.water {
                previous = None;
                continue;
            }
            if previous.is_some_and(|p| p != tile.height) {
                steps += 1;
            }
            previous = Some(tile.height);
        }
    }

    let share = |n: usize| n as f32 * 100.0 / land.max(1) as f32;
    Relief {
        flat_share: share(flat),
        systems,
        specks,
        crossings: steps as f32 / walked.max(1) as f32,
        bands: std::array::from_fn(|b| share(bands[b])),
    }
}

/// Tiles on the straight line between two points (Bresenham).
fn line(a: TileCoord, b: TileCoord) -> Vec<TileCoord> {
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

/// Size of the largest 4-connected run of buildable land.
pub fn largest_buildable_region(map: &MapGrid) -> usize {
    let w = map.width as i32;
    let h = map.height as i32;
    let idx = |x: i32, y: i32| (y * w + x) as usize;
    let mut seen = vec![false; map.tiles().len()];
    let mut best = 0usize;

    for y in 0..h {
        for x in 0..w {
            if seen[idx(x, y)] || !is_buildable(map, TileCoord { x, y }) {
                continue;
            }
            let mut stack = vec![(x, y)];
            seen[idx(x, y)] = true;
            let mut size = 0usize;
            while let Some((cx, cy)) = stack.pop() {
                size += 1;
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, ny) = (cx + dx, cy + dy);
                    if nx < 0 || ny < 0 || nx >= w || ny >= h {
                        continue;
                    }
                    if seen[idx(nx, ny)] || !is_buildable(map, TileCoord { x: nx, y: ny }) {
                        continue;
                    }
                    seen[idx(nx, ny)] = true;
                    stack.push((nx, ny));
                }
            }
            best = best.max(size);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tile::{TerrainKind, Tile};

    fn grid(width: u32, height: u32, f: impl Fn(i32, i32) -> Tile) -> MapGrid {
        let mut map = MapGrid::empty(width, height, 0);
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                *map.get_mut(TileCoord { x, y }).expect("in bounds") = f(x, y);
            }
        }
        map
    }

    fn land(height: i8) -> Tile {
        Tile {
            height,
            water: false,
            kind: TerrainKind::Plains,
        }
    }

    fn water() -> Tile {
        Tile {
            height: -1,
            water: true,
            kind: TerrainKind::Water,
        }
    }

    #[test]
    fn border_water_reads_as_sea_without_generator_notes() {
        // A ring of water plus a puddle in the middle.
        let map = grid(9, 9, |x, y| {
            let border = x == 0 || y == 0 || x == 8 || y == 8;
            let puddle = x == 4 && y == 4;
            if border || puddle {
                water()
            } else {
                land(0)
            }
        });
        let composition = composition(&map);
        assert!(composition.sea > 0.0);
        assert!((composition.inland_water - 100.0 / 81.0).abs() < 0.01);
    }

    #[test]
    fn a_narrow_river_is_crossable_and_a_wide_one_is_not() {
        // Column x=4 is a four-wide river except at y=2 where it pinches to one.
        let map = grid(11, 9, |x, y| {
            let wide = x == 4 || ((5..=7).contains(&x) && y != 2);
            if wide {
                water()
            } else {
                land(0)
            }
        });
        let crossings = river_crossings(&map);
        assert_eq!(crossings.len(), 1, "one pinch is one crossing: {crossings:?}");
        assert_eq!(crossings[0].span, 1);
        assert_eq!(crossings[0].tile, TileCoord { x: 4, y: 2 });

        // The four-wide stretch is bridgeable — at the premium end of the
        // ladder, which is why it is not one of the places worth scouting.
        let trunk = TileCoord { x: 5, y: 5 };
        assert_eq!(crossing_span(&map, trunk), Some(4));
        assert_eq!(cheap_crossing_span(&map, trunk), None);
    }

    /// Wide water is a premium crossing; water wider than the span limit is
    /// still genuinely impassable, and both answers have to be distinguishable.
    #[test]
    fn water_past_the_span_limit_refuses_a_bridge_at_any_price() {
        let widest = MAX_BRIDGE_SPAN as i32;
        for (width, expected) in [(widest, Some(widest as u32)), (widest + 1, None)] {
            let map = grid((width + 6) as u32, 5, |x, _| {
                if x >= 3 && x < 3 + width {
                    water()
                } else {
                    land(0)
                }
            });
            assert_eq!(
                crossing_span(&map, TileCoord { x: 3, y: 2 }),
                expected,
                "a {width}-wide channel"
            );
            // Neither width is a cheap crossing, so neither is worth scouting.
            assert!(river_crossings(&map).is_empty());
        }
    }

    #[test]
    fn a_pass_is_the_gap_and_not_the_wall() {
        // A rock wall across the map with a two-tile saddle at y = 3..=4.
        let map = grid(9, 13, |x, y| {
            if x == 4 && !(5..=7).contains(&y) {
                land(ROCK_HEIGHT_MIN + 2)
            } else {
                land(0)
            }
        });
        let passes = ridge_passes(&map);
        assert_eq!(passes.len(), 1, "one saddle is one pass: {passes:?}");
        assert_eq!(passes[0].x, 4);
    }

    #[test]
    fn relief_counts_the_ground_that_does_not_move() {
        // A 20² plain with one 6² hill stamped on it. Everything but the hill and
        // the ring of plain around it is flat, and the hill is one system.
        let map = grid(20, 20, |x, y| {
            if (7..13).contains(&x) && (7..13).contains(&y) {
                land(HILL_HEIGHT_MIN)
            } else {
                land(0)
            }
        });
        let relief = relief(&map);
        // 400 tiles, less the 6² hill and the 8² ring round it that sees it.
        assert_eq!(relief.systems, 1);
        assert_eq!(relief.specks, 0);
        let moving = 8 * 8 - 4 * 4; // the hill's own edge plus the plain that abuts it
        let expected = (400 - moving) as f32 * 100.0 / 400.0;
        assert!(
            (relief.flat_share - expected).abs() < 0.01,
            "flat share {:.2}, expected {expected:.2}",
            relief.flat_share
        );
        assert!((relief.bands[0] - 91.0).abs() < 0.01);
        assert!((relief.bands[2] - 9.0).abs() < 0.01);
    }

    #[test]
    fn relief_hears_a_wall_and_not_an_open_field() {
        // Same map twice over, once dead flat and once striped every other tile.
        // The sight-line count is what separates them, and it is the number the
        // §2.1 composition shares cannot see: both maps here are 100% buildable.
        let flat = grid(64, 64, |_, _| land(0));
        let striped = grid(64, 64, |x, _| land(if x % 2 == 0 { 0 } else { 4 }));
        assert_eq!(relief(&flat).crossings, 0.0);
        assert!(
            relief(&striped).crossings > 12.0,
            "a map that steps every other tile should light this up: {:.2}",
            relief(&striped).crossings
        );
        assert!(relief(&flat).flat_share > 99.9);
        assert_eq!(relief(&striped).flat_share, 0.0);
    }

    #[test]
    fn rock_is_land_but_not_buildable() {
        let map = grid(4, 4, |x, _| {
            if x < 2 {
                land(0)
            } else {
                land(ROCK_HEIGHT_MIN)
            }
        });
        let composition = composition(&map);
        assert_eq!(composition.buildable, 50.0);
        assert_eq!(composition.rock, 50.0);
        assert_eq!(largest_buildable_region(&map), 8);
    }
}
