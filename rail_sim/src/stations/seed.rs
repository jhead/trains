//! Auto-seed named stations + industries on land after map gen.
//!
//! Call once at startup with a land predicate (typically from [`crate::track::TrackTerrain`]
//! or `rail_map::MapGrid`). Picks spaced land tiles on the **largest reachable
//! landmass** — land plus the banks a bridge can reach — so the player can
//! always rail-connect the MVP anchors.

use crate::ids::TileCoord;
use crate::track::{GROUND_LAYER, MAX_BRIDGE_SPAN};

use super::industry::{GoodKind, IndustryRegistry};
use super::registry::StationRegistry;
use super::service::StationService;

const STATION_NAMES: &[&str] = &["Eastgate", "Westbrook", "Millhaven", "Ridgeline"];

/// Sites the map generator suggests for anchors, best first.
///
/// `rail_sim` cannot see `rail_map`, so the app inserts this alongside its
/// [`TrackTerrain`](crate::track::TrackTerrain). Design 02 §4.1: the opening
/// beat is level design, and only the generator knows where it put the
/// question it wants the player to answer first.
#[derive(Debug, Clone, Default, bevy_ecs::prelude::Resource)]
pub struct AnchorSites(pub Vec<TileCoord>);

/// Seed 3 stations and 2 industries onto walkable land tiles.
///
/// Industries: Pine Sawmill (produces lumber) and Harbor Mill (consumes lumber).
///
/// Knows land from everything else and no more, so it cannot tell a bridgeable
/// river from a wall of rock: the landmass flood stays land-only. Callers that
/// hold terrain should pass a water predicate to
/// [`seed_stations_and_industries_at`] instead.
pub fn seed_stations_and_industries(
    stations: &mut StationRegistry,
    industries: &mut IndustryRegistry,
    service: &mut StationService,
    width: u32,
    height: u32,
    is_land: impl Fn(TileCoord) -> bool,
) {
    seed_stations_and_industries_at(
        stations,
        industries,
        service,
        width,
        height,
        is_land,
        |_| false,
        &[],
    )
}

/// Seed anchors, preferring sites the generator picked.
///
/// `preferred` leads the sampler; anything left is filled by the existing
/// farthest-point pass, so a map with no hints behaves exactly as before.
///
/// `is_water` decides which gaps a component may reach across — see
/// [`largest_landmass`]. Anchors are still only ever placed on `is_land` tiles.
#[allow(clippy::too_many_arguments)]
pub fn seed_stations_and_industries_at(
    stations: &mut StationRegistry,
    industries: &mut IndustryRegistry,
    service: &mut StationService,
    width: u32,
    height: u32,
    is_land: impl Fn(TileCoord) -> bool,
    is_water: impl Fn(TileCoord) -> bool,
    preferred: &[TileCoord],
) {
    if !stations.is_empty() || !industries.is_empty() {
        return;
    }

    let mut land: Vec<TileCoord> = Vec::new();
    for y in 2..(height.saturating_sub(2) as i32) {
        for x in 2..(width.saturating_sub(2) as i32) {
            let c = TileCoord { x, y };
            if is_land(c) {
                land.push(c);
            }
        }
    }
    if land.is_empty() {
        return;
    }

    // Prefer the largest reachable landmass so autofill/pathing can link them.
    let land = largest_landmass(&land, &is_water);
    if land.is_empty() {
        return;
    }

    let targets = pick_spaced(&land, 5, preferred);
    let mut ti = 0usize;

    for name in STATION_NAMES.iter().take(3) {
        if ti >= targets.len() {
            break;
        }
        let tile = targets[ti];
        ti += 1;
        let id = stations.insert(*name, tile, GROUND_LAYER);
        service.ensure(id);
    }

    // Sawmill + mill: next two spaced tiles (or fall back to land list).
    let saw_tile = targets.get(ti).copied().or_else(|| land.first().copied());
    ti += 1;
    let mill_tile = targets
        .get(ti)
        .copied()
        .or_else(|| land.get(land.len() / 2).copied());

    if let Some(tile) = saw_tile {
        if stations.at(tile, GROUND_LAYER).is_none() {
            industries.insert(
                "Pine Sawmill",
                tile,
                Some(GoodKind::Lumber),
                None,
            );
        }
    }
    if let Some(tile) = mill_tile {
        if stations.at(tile, GROUND_LAYER).is_none() && industries.at(tile).is_none() {
            industries.insert(
                "Harbor Mill",
                tile,
                None,
                Some(GoodKind::Lumber),
            );
        }
    }
}

/// Keep only tiles in the largest component of `land`, where two banks a bridge
/// could join are one component.
///
/// A land-only flood strands the far side of any watercourse that reaches the
/// map edge, so nothing is ever seeded across it and design 02 §3.4's crossing
/// decision has nothing on the other side of it worth crossing to. A component
/// therefore steps orthogonally onto land, or hops a straight run of at most
/// [`MAX_BRIDGE_SPAN`] water tiles to the land beyond — the same run
/// [`path_bridge_spans_ok`](crate::track::path_bridge_spans_ok) lets a bridge
/// cover. Anything else, rock or off-map, stops the walk.
///
/// Only land is returned: water joins components, it never hosts an anchor.
fn largest_landmass(land: &[TileCoord], is_water: impl Fn(TileCoord) -> bool) -> Vec<TileCoord> {
    use std::collections::{HashSet, VecDeque};

    if land.is_empty() {
        return Vec::new();
    }
    let set: HashSet<(i32, i32)> = land.iter().map(|c| (c.x, c.y)).collect();
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    let mut best: Vec<TileCoord> = Vec::new();

    // Row-major starts and FIFO expansion, so equal-sized components always tie
    // toward the same one — seeding must not depend on set iteration order.
    for &start in land {
        let key = (start.x, start.y);
        if !seen.insert(key) {
            continue;
        }
        let mut component = Vec::new();
        let mut q = VecDeque::new();
        q.push_back(start);
        while let Some(cur) = q.pop_front() {
            component.push(cur);
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                for step in 1..=(MAX_BRIDGE_SPAN as i32 + 1) {
                    let n = (cur.x + dx * step, cur.y + dy * step);
                    if set.contains(&n) {
                        if seen.insert(n) {
                            q.push_back(TileCoord { x: n.0, y: n.1 });
                        }
                        break;
                    }
                    if !is_water(TileCoord { x: n.0, y: n.1 }) {
                        break;
                    }
                }
            }
        }
        if component.len() > best.len() {
            best = component;
        }
    }
    best
}

/// Greedy farthest-point sampling for roughly even coverage.
fn pick_spaced(land: &[TileCoord], count: usize, preferred: &[TileCoord]) -> Vec<TileCoord> {
    if land.is_empty() || count == 0 {
        return Vec::new();
    }
    let mut chosen = Vec::with_capacity(count);

    // Generator-suggested sites first. The first two are the opening beat
    // (design 02 §4.1) - a home town near the centre and a destination eight to
    // twelve tiles away. Farthest-point sampling alone drives every anchor to a
    // map corner, which makes the player's first act a haul between opposite
    // edges before anything has paid out.
    let on_land: std::collections::HashSet<(i32, i32)> =
        land.iter().map(|c| (c.x, c.y)).collect();
    for &site in preferred {
        if chosen.len() >= count {
            break;
        }
        if on_land.contains(&(site.x, site.y)) && !chosen.contains(&site) {
            chosen.push(site);
        }
    }

    if chosen.is_empty() {
        // Start near map "interest": prefer a tile around the centroid of land.
        let (sx, sy) = land.iter().fold((0i64, 0i64), |a, c| {
            (a.0 + c.x as i64, a.1 + c.y as i64)
        });
        let n = land.len() as i64;
        let cx = (sx / n) as i32;
        let cy = (sy / n) as i32;
        let mut best = land[0];
        let mut best_d = i32::MAX;
        for &c in land {
            let d = (c.x - cx).abs() + (c.y - cy).abs();
            if d < best_d {
                best_d = d;
                best = c;
            }
        }
        chosen.push(best);
    }

    while chosen.len() < count {
        let mut far = land[0];
        let mut far_d = -1i32;
        for &c in land {
            let min_d = chosen
                .iter()
                .map(|p| (p.x - c.x).abs() + (p.y - c.y).abs())
                .min()
                .unwrap_or(0);
            if min_d > far_d {
                far_d = min_d;
                far = c;
            }
        }
        if far_d <= 0 {
            break;
        }
        chosen.push(far);
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_named_stations_and_two_industries() {
        let mut stations = StationRegistry::new();
        let mut industries = IndustryRegistry::new();
        let mut service = StationService::default();
        seed_stations_and_industries(
            &mut stations,
            &mut industries,
            &mut service,
            32,
            32,
            |_| true,
        );
        assert_eq!(stations.len(), 3);
        assert_eq!(industries.len(), 2);
        assert!(stations.iter().any(|s| s.name == "Eastgate"));
        assert!(industries.producer_of(GoodKind::Lumber).is_some());
        assert!(industries.consumer_of(GoodKind::Lumber).is_some());
    }

    #[test]
    fn seeds_only_on_largest_landmass() {
        // Two islands: a small 2-tile and a large 3×3 — anchors must stay on the 3×3.
        let mut stations = StationRegistry::new();
        let mut industries = IndustryRegistry::new();
        let mut service = StationService::default();
        let is_land = |c: TileCoord| {
            // Large mass around (10,10)
            (c.x >= 8 && c.x <= 14 && c.y >= 8 && c.y <= 14)
                // Tiny island
                || (c.x == 2 && (c.y == 2 || c.y == 3))
        };
        seed_stations_and_industries(
            &mut stations,
            &mut industries,
            &mut service,
            32,
            32,
            is_land,
        );
        for s in stations.iter() {
            assert!(
                s.tile.x >= 8 && s.tile.x <= 14 && s.tile.y >= 8 && s.tile.y <= 14,
                "station {} on tiny island {:?}",
                s.name,
                s.tile
            );
        }
        for i in industries.iter() {
            assert!(
                i.tile.x >= 8 && i.tile.x <= 14 && i.tile.y >= 8 && i.tile.y <= 14,
                "industry {} on tiny island {:?}",
                i.name,
                i.tile
            );
        }
    }

    /// West bank of the test river; the east bank starts at `WEST_BANK + width`.
    const WEST_BANK: i32 = 10;

    /// A 32×32 map cut top to bottom by a river `width` tiles wide.
    fn river_map(width: i32) -> (impl Fn(TileCoord) -> bool, impl Fn(TileCoord) -> bool) {
        let wet = move |c: TileCoord| c.x >= WEST_BANK && c.x < WEST_BANK + width;
        (move |c: TileCoord| !wet(c), wet)
    }

    /// Every seeded anchor tile, stations and industries together.
    fn anchor_tiles(stations: &StationRegistry, industries: &IndustryRegistry) -> Vec<TileCoord> {
        stations
            .iter()
            .map(|s| s.tile)
            .chain(industries.iter().map(|i| i.tile))
            .collect()
    }

    #[test]
    fn a_bridgeable_river_is_seeded_on_both_banks() {
        // Design 02 §3.4: crossing is a decision, which needs something on the
        // far side to cross *to*. This river reaches both map edges, so a
        // land-only flood would keep one bank and drop the other entirely.
        let width = MAX_BRIDGE_SPAN as i32;
        let mut stations = StationRegistry::new();
        let mut industries = IndustryRegistry::new();
        let mut service = StationService::default();
        let (is_land, is_water) = river_map(width);
        seed_stations_and_industries_at(
            &mut stations,
            &mut industries,
            &mut service,
            32,
            32,
            &is_land,
            &is_water,
            &[],
        );

        let tiles = anchor_tiles(&stations, &industries);
        assert_eq!(tiles.len(), 5);
        assert!(
            tiles.iter().all(|t| is_land(*t)),
            "an anchor landed in the river: {tiles:?}"
        );
        assert!(
            tiles.iter().any(|t| t.x < WEST_BANK),
            "nothing seeded on the west bank: {tiles:?}"
        );
        assert!(
            tiles.iter().any(|t| t.x >= WEST_BANK + width),
            "nothing seeded on the east bank: {tiles:?}"
        );
    }

    #[test]
    fn water_wider_than_a_bridge_still_splits_the_map() {
        // One tile past the span limit there is no crossing, so the far bank is
        // unreachable by rail and seeding it would strand an anchor.
        let width = MAX_BRIDGE_SPAN as i32 + 1;
        let mut stations = StationRegistry::new();
        let mut industries = IndustryRegistry::new();
        let mut service = StationService::default();
        let (is_land, is_water) = river_map(width);
        seed_stations_and_industries_at(
            &mut stations,
            &mut industries,
            &mut service,
            32,
            32,
            &is_land,
            &is_water,
            &[],
        );

        // East bank is 16 columns to the west bank's 8, so it wins the component.
        let tiles = anchor_tiles(&stations, &industries);
        assert_eq!(tiles.len(), 5);
        assert!(
            tiles.iter().all(|t| t.x >= WEST_BANK + width),
            "anchors stranded across unbridgeable water: {tiles:?}"
        );
    }

    #[test]
    fn a_bridge_never_crosses_rock() {
        // The gap is exactly one tile, but it is not water: a wall of rock is a
        // wall, and the smaller side stays unseeded.
        let mut stations = StationRegistry::new();
        let mut industries = IndustryRegistry::new();
        let mut service = StationService::default();
        let is_land = |c: TileCoord| c.x != WEST_BANK;
        seed_stations_and_industries_at(
            &mut stations,
            &mut industries,
            &mut service,
            32,
            32,
            is_land,
            |_| false, // nothing is water; the column is rock
            &[],
        );

        let tiles = anchor_tiles(&stations, &industries);
        assert!(
            tiles.iter().all(|t| t.x > WEST_BANK),
            "rock joined two landmasses: {tiles:?}"
        );
    }
}
