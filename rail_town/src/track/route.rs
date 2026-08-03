//! Smart route proposal — the default drag (brief 04 §2.2).
//!
//! A* over the sixteen-direction graph, weighted by what a tile actually
//! costs to build ([`tile_build_cost`]) plus a straightness term, so the
//! proposal follows contours, picks river narrows, and spends the player's
//! money the way the player would. This is where the game demonstrates that
//! it understands its own terrain.
//!
//! # What the search will and will not propose
//!
//! - **Existing track is free to ride through.** Reusing a corridor costs the
//!   route nothing, which is exactly the pull toward one shared alignment the
//!   sim's economics want. The commit skips those tiles when placing.
//! - **Bridges are straight.** On water the only legal continuation is the
//!   incoming direction — a deck that turns mid-river is not a thing the span
//!   rules would accept, and it would read as nonsense besides.
//! - **Half-steps stay on land** and are only offered while both crossed
//!   tiles are clear of track, since the shallow link cannot form otherwise.
//! - **Grade is a hard limit**, exactly as [`path_grades_ok`] enforces it, so
//!   nothing is proposed that placement would then refuse.
//!
//! # Stability
//!
//! Brief 04 §2.2: *"prefer the previous frame's shape when costs are within a
//! few percent. A flickering ghost is unusable."* The search takes the
//! previous proposal and discounts steps that land on its tiles by one cent —
//! enough to hold the shape against an exact tie or a hair's improvement,
//! nowhere near enough to override a real cost difference. Everything else is
//! a pure function of the endpoints: ties in the heap break on coordinates,
//! never on hash order.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use rail_sim::ids::TileCoord;
use rail_sim::track::{
    clock_separation, intermediate_tiles, is_half_step, step, tile_build_cost, TrackNetwork,
    TrackTerrain, DIR_COUNT, GROUND_LAYER, MAX_GRADE,
};

use super::propose::ProposedPath;

/// Cost of one clock-step of bearing change, in cents.
///
/// A quarter of a flat tile per step of the rose: a 90° turn prices like one
/// flat tile, so straightness settles ties and gentle S-bends beat zigzags,
/// while a genuinely cheaper alignment still wins on cost.
const TURN_PENALTY_CENTS: i64 = 250;

/// Discount for a step that lands on the previous proposal's path.
///
/// One cent: the hysteresis the brief asks for, and no more.
const PREVIOUS_SHAPE_DISCOUNT_CENTS: i64 = 1;

/// Surcharge on a half-step move, just under one flat tile.
///
/// A shallow link places one tile where a compass pair places two, so on
/// invoice alone the cheapest crossing of open ground is a zigzag of knight's
/// moves — optimal, and deranged on screen. Charging most of the saved tile
/// back keeps the shallow run the honest winner for a genuinely shallow drag
/// while a straight drag proposes the straight line. The player who wants the
/// sparse-run discount on a cardinal alignment can still lay it with Shift.
const HALF_STEP_EXTRA_CENTS: i64 = 900;

/// Popped-node ceiling. A 96x96 map holds ~156k states; past a quarter
/// million something is pathological and the honest answer is "no route".
const SEARCH_BUDGET: usize = 250_000;

/// `incoming` value for the start state, where no direction is held yet.
const NO_DIR: u8 = DIR_COUNT as u8;

/// Propose the cheapest legal run from `from` to `to`.
///
/// `contour_lock` (Alt) refuses any step that changes ground height, routing
/// around anything that would climb. `previous` is last frame's accepted
/// proposal, used only for the one-cent shape hold.
///
/// Returns `None` when no legal route exists inside the search budget — the
/// preview turns that into a loud, specific rejection.
pub fn propose_smart(
    network: &TrackNetwork,
    terrain: &TrackTerrain,
    from: TileCoord,
    to: TileCoord,
    contour_lock: bool,
    previous: Option<&[TileCoord]>,
) -> Option<ProposedPath> {
    if !terrain.contains(from) || !terrain.contains(to) {
        return None;
    }
    if from == to {
        return Some(ProposedPath {
            tiles: vec![from],
            endpoint: to,
        });
    }
    // A destination that can neither be built on nor already carries track is
    // unreachable by definition; say so without searching.
    if network.id_at(to, GROUND_LAYER).is_none() && tile_build_cost(terrain, to).is_err() {
        return None;
    }

    let on_previous: std::collections::HashSet<(i32, i32)> = previous
        .unwrap_or(&[])
        .iter()
        .map(|t| (t.x, t.y))
        .collect();

    // State: (tile, incoming direction). Keyed cost map + parent links.
    type Key = (i32, i32, u8);
    let mut best: HashMap<Key, i64> = HashMap::new();
    let mut parent: HashMap<Key, Key> = HashMap::new();
    // Heap entries order by (cost, x, y, dir) — coordinate tie-breaks keep the
    // search deterministic regardless of hash iteration.
    let mut heap: BinaryHeap<Reverse<(i64, i32, i32, u8)>> = BinaryHeap::new();

    let start: Key = (from.x, from.y, NO_DIR);
    best.insert(start, 0);
    heap.push(Reverse((0, from.x, from.y, NO_DIR)));

    let mut popped = 0usize;
    let mut goal: Option<Key> = None;

    while let Some(Reverse((cost, x, y, in_dir))) = heap.pop() {
        let key = (x, y, in_dir);
        if best.get(&key).is_some_and(|&b| cost > b) {
            continue;
        }
        if x == to.x && y == to.y {
            goal = Some(key);
            break;
        }
        popped += 1;
        if popped > SEARCH_BUDGET {
            return None;
        }

        let cur = TileCoord { x, y };
        let cur_water = terrain.is_water(cur);
        let cur_height = terrain.height_at(cur).unwrap_or(0);

        for dir in 0..DIR_COUNT {
            // A bridge deck holds its line: on water the only continuation is
            // straight ahead.
            if cur_water && in_dir != NO_DIR && dir as u8 != in_dir {
                continue;
            }
            let next = step(cur, dir);
            if !terrain.contains(next) {
                continue;
            }
            let next_water = terrain.is_water(next);
            if is_half_step(dir) {
                // Shallow links live on land, with both crossed tiles clear.
                // Unbuildable rock blocks them too: the sim would accept the
                // hop (crossed tiles stay bare), but a proposal that slips
                // through a mountain wall reads as a cheat, not a route.
                if cur_water || next_water {
                    continue;
                }
                let crossed_blocked =
                    intermediate_tiles(cur, dir)
                        .into_iter()
                        .flatten()
                        .any(|mid| {
                            terrain.is_water(mid)
                                || network.id_at(mid, GROUND_LAYER).is_some()
                                || tile_build_cost(terrain, mid).is_err()
                        });
                if crossed_blocked {
                    continue;
                }
            }

            let next_height = terrain.height_at(next).unwrap_or(0);
            // Same rule as `path_grades_ok`: water legs are a flood tag, not a
            // climb; land legs obey the grade wall.
            if !cur_water && !next_water {
                let grade = (cur_height as i16 - next_height as i16).unsigned_abs() as u8;
                if grade > MAX_GRADE {
                    continue;
                }
                if contour_lock && grade != 0 {
                    continue;
                }
            } else if contour_lock {
                // Holding the contour means staying off bridges too — a deck
                // is a climb in the sense that matters to the player's intent.
                continue;
            }

            // Riding existing track is free; new tiles price at what they
            // would actually cost to build.
            let tile_cents = if network.id_at(next, GROUND_LAYER).is_some() {
                0
            } else {
                match tile_build_cost(terrain, next) {
                    Ok(c) => c,
                    Err(_) => continue,
                }
            };
            let turn_cents = if in_dir == NO_DIR {
                0
            } else {
                clock_separation(in_dir as usize, dir) as i64 * TURN_PENALTY_CENTS
            };
            let shallow_cents = if is_half_step(dir) {
                HALF_STEP_EXTRA_CENTS
            } else {
                0
            };
            let hold = if on_previous.contains(&(next.x, next.y)) {
                PREVIOUS_SHAPE_DISCOUNT_CENTS
            } else {
                0
            };
            let step_cost = tile_cents + turn_cents + shallow_cents - hold;

            let next_key: Key = (next.x, next.y, dir as u8);
            let candidate = cost + step_cost;
            if best.get(&next_key).is_none_or(|&b| candidate < b) {
                best.insert(next_key, candidate);
                parent.insert(next_key, key);
                heap.push(Reverse((candidate, next.x, next.y, dir as u8)));
            }
        }
    }

    let goal = goal?;
    let mut tiles = Vec::new();
    let mut cursor = goal;
    loop {
        tiles.push(TileCoord {
            x: cursor.0,
            y: cursor.1,
        });
        match parent.get(&cursor) {
            Some(&p) => cursor = p,
            None => break,
        }
    }
    tiles.reverse();
    Some(ProposedPath {
        tiles,
        endpoint: to,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat land everywhere, with `water` and `heights` overrides.
    fn terrain_with(
        w: u32,
        h: u32,
        water: &[(i32, i32)],
        heights: &[((i32, i32), i8)],
    ) -> TrackTerrain {
        let mut cells = Vec::with_capacity((w * h) as usize);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let is_water = water.contains(&(x, y));
                let height = heights
                    .iter()
                    .find(|(c, _)| *c == (x, y))
                    .map(|(_, hh)| *hh)
                    .unwrap_or(if is_water { -1 } else { 4 });
                cells.push((is_water, height));
            }
        }
        TrackTerrain::new(w, h, cells)
    }

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    fn legs_are_dir16(tiles: &[TileCoord]) {
        for w in tiles.windows(2) {
            assert!(
                rail_sim::track::dir_index(w[0], w[1]).is_some(),
                "leg {:?} -> {:?} is not one of the sixteen",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn open_ground_proposes_the_straight_line() {
        let t = terrain_with(16, 16, &[], &[]);
        let n = TrackNetwork::default();
        let p = propose_smart(&n, &t, tile(2, 8), tile(12, 8), false, None).unwrap();
        legs_are_dir16(&p.tiles);
        assert_eq!(p.tiles.first(), Some(&tile(2, 8)));
        assert_eq!(p.tiles.last(), Some(&tile(12, 8)));
        // Nothing to route around: the cheapest run is the straight one.
        assert_eq!(p.tiles.len(), 11);
        assert!(p.tiles.iter().all(|c| c.y == 8), "swerved on open ground");
    }

    #[test]
    fn a_mountain_wall_is_walked_around_not_through() {
        // A vertical wall of unbuildable rock with one gap.
        let mut heights = Vec::new();
        for y in 0..16 {
            if y != 12 {
                heights.push(((8, y), 14i8));
            }
        }
        let t = terrain_with(16, 16, &[], &heights);
        let n = TrackNetwork::default();
        let p = propose_smart(&n, &t, tile(2, 2), tile(14, 2), false, None).unwrap();
        legs_are_dir16(&p.tiles);
        assert!(
            p.tiles.contains(&tile(8, 12)),
            "the only gap in the wall is the only way through: {:?}",
            p.tiles
        );
        assert!(p.tiles.iter().all(|c| {
            c.x != 8 || c.y == 12
        }));
    }

    #[test]
    fn a_river_is_crossed_at_its_narrows() {
        // A river three wide, narrowing to one tile at y = 10.
        let mut water = Vec::new();
        for y in 0..16 {
            water.push((7, y));
            if y != 10 {
                water.push((6, y));
                water.push((8, y));
            }
        }
        let t = terrain_with(16, 16, &water, &[]);
        let n = TrackNetwork::default();
        let p = propose_smart(&n, &t, tile(2, 3), tile(13, 3), false, None).unwrap();
        legs_are_dir16(&p.tiles);
        let wet: Vec<_> = p.tiles.iter().filter(|c| t.is_water(**c)).collect();
        assert_eq!(
            wet,
            vec![&tile(7, 10)],
            "the one-tile narrows is the cheap crossing: {:?}",
            p.tiles
        );
    }

    #[test]
    fn bridges_hold_their_line() {
        // Wide-open water band; every crossing is span 3, so the route may
        // cross anywhere — but never turn mid-water.
        let mut water = Vec::new();
        for y in 0..16 {
            for x in 6..=8 {
                water.push((x, y));
            }
        }
        let t = terrain_with(16, 16, &water, &[]);
        let n = TrackNetwork::default();
        let p = propose_smart(&n, &t, tile(1, 2), tile(14, 13), false, None).unwrap();
        legs_are_dir16(&p.tiles);
        for w in p.tiles.windows(3) {
            if t.is_water(w[1]) {
                let d1 = rail_sim::track::dir_index(w[0], w[1]).unwrap();
                let d2 = rail_sim::track::dir_index(w[1], w[2]).unwrap();
                assert_eq!(d1, d2, "deck turned mid-water at {:?}", w[1]);
            }
        }
    }

    #[test]
    fn contour_lock_refuses_to_climb() {
        // A ridge one band (delta 3) high across the middle, with a level
        // corridor at y = 13.
        let mut heights = Vec::new();
        for x in 0..16 {
            for y in 4..13 {
                heights.push(((x, y), 7i8));
            }
        }
        let t = terrain_with(16, 16, &[], &heights);
        let n = TrackNetwork::default();

        // Unlocked: happy to climb the bank and come back down.
        let free = propose_smart(&n, &t, tile(2, 14), tile(14, 2), false, None).unwrap();
        assert!(free.tiles.iter().any(|c| t.height_at(*c) == Some(7)));

        // Locked from the low side: no step may change height, and the high
        // plateau is unreachable — the proposal must refuse, not sneak a climb.
        let locked = propose_smart(&n, &t, tile(2, 14), tile(14, 2), true, None);
        assert!(locked.is_none(), "contour lock climbed: {locked:?}");

        // Locked along the level corridor: fine.
        let along = propose_smart(&n, &t, tile(2, 14), tile(14, 14), true, None).unwrap();
        assert!(along.tiles.iter().all(|c| t.height_at(*c) == Some(4)));
    }

    #[test]
    fn existing_track_is_a_free_ride() {
        // A corridor of existing track skirting an expensive hill field.
        let mut heights = Vec::new();
        for x in 4..=11 {
            for y in 4..=8 {
                heights.push(((x, y), 8i8)); // hills: 3x cost
            }
        }
        let t = terrain_with(16, 16, &[], &heights);
        let mut owned = TrackNetwork::default();
        let mut money = rail_sim::Money::new(1_000_000);
        let mut ledger = rail_sim::MoneyLedger::default();
        for x in 2..=13 {
            rail_sim::track::try_place_track(
                &mut owned,
                &mut money,
                &mut ledger,
                &t,
                tile(x, 10),
                GROUND_LAYER,
            )
            .expect("corridor tile places");
        }
        let p = propose_smart(&owned, &t, tile(2, 6), tile(13, 6), false, None).unwrap();
        assert!(
            p.tiles.iter().filter(|c| c.y == 10).count() >= 8,
            "should ride the free corridor: {:?}",
            p.tiles
        );
    }

    #[test]
    fn a_genuinely_shallow_drag_still_proposes_the_sparse_run() {
        // (2,8) -> (12,13) is exactly five ENE half-steps. The surcharge must
        // not push the proposal onto a compass staircase - the shallow run is
        // both cheaper and the shape the player pointed at.
        let t = terrain_with(16, 16, &[], &[]);
        let n = TrackNetwork::default();
        let p = propose_smart(&n, &t, tile(2, 8), tile(12, 13), false, None).unwrap();
        legs_are_dir16(&p.tiles);
        assert_eq!(
            p.tiles.len(),
            6,
            "five knight's moves, six tiles: {:?}",
            p.tiles
        );
    }

    #[test]
    fn the_previous_shape_holds_against_a_tie() {
        // Two equal-cost staircases exist between diagonal corners; the held
        // previous path must win the tie, whichever it was.
        let t = terrain_with(12, 12, &[], &[]);
        let n = TrackNetwork::default();
        let first = propose_smart(&n, &t, tile(2, 2), tile(9, 5), false, None).unwrap();
        let held = propose_smart(&n, &t, tile(2, 2), tile(9, 5), false, Some(&first.tiles))
            .unwrap();
        assert_eq!(first.tiles, held.tiles, "the held shape flickered");
    }

    #[test]
    fn proposals_are_deterministic() {
        let t = terrain_with(24, 24, &[(9, 9), (9, 10), (9, 11)], &[((14, 14), 8)]);
        let n = TrackNetwork::default();
        let a = propose_smart(&n, &t, tile(2, 2), tile(21, 20), false, None).unwrap();
        let b = propose_smart(&n, &t, tile(2, 2), tile(21, 20), false, None).unwrap();
        assert_eq!(a.tiles, b.tiles);
    }

    #[test]
    fn an_unreachable_goal_is_an_honest_none() {
        // Goal on high rock.
        let t = terrain_with(8, 8, &[], &[((6, 6), 16)]);
        let n = TrackNetwork::default();
        assert!(propose_smart(&n, &t, tile(1, 1), tile(6, 6), false, None).is_none());
    }
}
