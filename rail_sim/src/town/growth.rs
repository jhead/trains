//! Building density rings around stations, driven by service scores.

use std::collections::HashMap;

use bevy_ecs::prelude::*;

use crate::ids::TileCoord;
use crate::stations::{StationRegistry, StationService};

/// Chebyshev radius (tiles) of the growth ring around each station.
pub const GROWTH_RADIUS: i32 = 5;

/// Maximum stored density per tile (`1.0` = fully built-up).
pub const MAX_DENSITY: f32 = 1.0;

/// How quickly density approaches its service-driven target (per tick).
const APPROACH_RATE: f32 = 0.04;

/// Sparse building density keyed by tile.
///
/// Values are in `0.0..=`[`MAX_DENSITY`]. Tiles with density near zero may be
/// omitted; readers should treat missing tiles as `0.0`.
#[derive(Debug, Clone, Default, Resource)]
pub struct TownDensity {
    cells: HashMap<(i32, i32), f32>,
}

impl TownDensity {
    pub fn get(&self, tile: TileCoord) -> f32 {
        self.cells
            .get(&(tile.x, tile.y))
            .copied()
            .unwrap_or(0.0)
    }

    pub fn set(&mut self, tile: TileCoord, density: f32) {
        let d = density.clamp(0.0, MAX_DENSITY);
        if d < 0.001 {
            self.cells.remove(&(tile.x, tile.y));
        } else {
            self.cells.insert((tile.x, tile.y), d);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (TileCoord, f32)> + '_ {
        self.cells.iter().map(|(&(x, y), &d)| (TileCoord { x, y }, d))
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
}

/// Target density at `tile` from the strongest nearby station influence.
///
/// `influence = (score / 100) * (1 - dist / (radius+1))` using Chebyshev distance.
pub fn density_target_at(
    tile: TileCoord,
    stations: &StationRegistry,
    service: &StationService,
) -> f32 {
    let mut best = 0.0_f32;
    for station in stations.iter() {
        let dx = (station.tile.x - tile.x).abs();
        let dy = (station.tile.y - tile.y).abs();
        let dist = dx.max(dy);
        if dist > GROWTH_RADIUS {
            continue;
        }
        let score = service.score(station.id).score as f32 / 100.0;
        let falloff = 1.0 - (dist as f32) / ((GROWTH_RADIUS + 1) as f32);
        let influence = (score * falloff).clamp(0.0, MAX_DENSITY);
        if influence > best {
            best = influence;
        }
    }
    best
}

/// Move every cell in station rings toward its service-driven target.
pub fn advance_town_growth(
    mut density: ResMut<TownDensity>,
    stations: Res<StationRegistry>,
    service: Res<StationService>,
) {
    if stations.is_empty() {
        return;
    }

    // Collect tiles that need an update (union of rings) so we also shrink
    // cells that fall out of good service without iterating the whole map.
    let mut tiles: Vec<TileCoord> = Vec::new();
    for station in stations.iter() {
        for dy in -GROWTH_RADIUS..=GROWTH_RADIUS {
            for dx in -GROWTH_RADIUS..=GROWTH_RADIUS {
                tiles.push(TileCoord {
                    x: station.tile.x + dx,
                    y: station.tile.y + dy,
                });
            }
        }
    }
    tiles.sort_by_key(|t| (t.y, t.x));
    tiles.dedup();

    for tile in tiles {
        let target = density_target_at(tile, &stations, &service);
        let current = density.get(tile);
        let next = current + (target - current) * APPROACH_RATE;
        density.set(tile, next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::StationId;
    use crate::stations::StationServiceScore;
    use crate::track::GROUND_LAYER;

    fn registry_with(tile: TileCoord, name: &str) -> (StationRegistry, StationId) {
        let mut reg = StationRegistry::new();
        let id = reg.insert(name, tile, GROUND_LAYER);
        (reg, id)
    }

    #[test]
    fn service_up_raises_density_target() {
        let tile = TileCoord { x: 10, y: 10 };
        let (stations, id) = registry_with(tile, "Eastgate");
        let mut service = StationService::default();

        service.scores.insert(
            id,
            StationServiceScore {
                score: 20,
                ..Default::default()
            },
        );
        let low = density_target_at(tile, &stations, &service);

        service.scores.insert(
            id,
            StationServiceScore {
                score: 90,
                ..Default::default()
            },
        );
        let high = density_target_at(tile, &stations, &service);

        assert!(
            high > low,
            "higher service score must raise density target ({high} > {low})"
        );
        assert!(high > 0.8);
    }

    #[test]
    fn growth_tick_moves_density_toward_higher_service() {
        let tile = TileCoord { x: 5, y: 5 };
        let (stations, id) = registry_with(tile, "Eastgate");
        let mut service = StationService::default();
        service.scores.insert(
            id,
            StationServiceScore {
                score: 100,
                ..Default::default()
            },
        );

        let mut density = TownDensity::default();
        assert_eq!(density.get(tile), 0.0);

        // Simulate several growth ticks without Bevy scheduling.
        for _ in 0..40 {
            let target = density_target_at(tile, &stations, &service);
            let current = density.get(tile);
            density.set(tile, current + (target - current) * APPROACH_RATE);
        }

        assert!(
            density.get(tile) > 0.5,
            "sustained high service should thicken buildings (got {})",
            density.get(tile)
        );
    }

    #[test]
    fn service_drop_lowers_target_so_density_can_shrink() {
        let tile = TileCoord { x: 3, y: 3 };
        let (stations, id) = registry_with(tile, "Westbrook");
        let mut service = StationService::default();
        service.scores.insert(
            id,
            StationServiceScore {
                score: 100,
                ..Default::default()
            },
        );
        let high = density_target_at(tile, &stations, &service);

        service.scores.insert(
            id,
            StationServiceScore {
                score: 10,
                ..Default::default()
            },
        );
        let low = density_target_at(tile, &stations, &service);

        assert!(low < high);
        // A cell sitting at `high` would shrink toward `low` each tick.
        let mut density = high;
        for _ in 0..30 {
            density += (low - density) * APPROACH_RATE;
        }
        assert!(density < high * 0.7);
    }
}
