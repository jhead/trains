//! Auto-seed named stations + industries on land after map gen.
//!
//! Call once at startup with a land predicate (typically from [`crate::track::TrackTerrain`]
//! or `rail_map::MapGrid`). Picks spaced land tiles so the player has clear
//! destinations to connect with track.

use crate::ids::TileCoord;
use crate::track::GROUND_LAYER;

use super::industry::{GoodKind, IndustryRegistry};
use super::registry::StationRegistry;
use super::service::StationService;

const STATION_NAMES: &[&str] = &["Eastgate", "Westbrook", "Millhaven", "Ridgeline"];

/// Seed 3 stations and 2 industries onto walkable land tiles.
///
/// Industries: Pine Sawmill (produces lumber) and Harbor Mill (consumes lumber).
pub fn seed_stations_and_industries(
    stations: &mut StationRegistry,
    industries: &mut IndustryRegistry,
    service: &mut StationService,
    width: u32,
    height: u32,
    is_land: impl Fn(TileCoord) -> bool,
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

    let targets = pick_spaced(&land, 5);
    let mut ti = 0usize;

    for (i, name) in STATION_NAMES.iter().take(3).enumerate() {
        if ti >= targets.len() {
            break;
        }
        let tile = targets[ti];
        ti += 1;
        let id = stations.insert(*name, tile, GROUND_LAYER);
        service.ensure(id);
        let _ = i;
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

/// Greedy farthest-point sampling for roughly even coverage.
fn pick_spaced(land: &[TileCoord], count: usize) -> Vec<TileCoord> {
    if land.is_empty() || count == 0 {
        return Vec::new();
    }
    let mut chosen = Vec::with_capacity(count);
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
}
