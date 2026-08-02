//! Pure overlay strength / colour helpers (unit-tested without Bevy rendering).

use bevy::prelude::{Alpha, Color};
use rail_sim::ids::TileCoord;
use rail_sim::{
    StationRegistry, StationService, TileOccupancy, TownDensity, TrackNetwork, GROWTH_RADIUS,
    GROUND_LAYER, MAX_DENSITY,
};

use crate::palette::{HI, OK, WARN};

use super::OverlayKind;

/// Map a 0..=1 strength onto diagnostic palette colours (never world art hues).
pub fn overlay_color(kind: OverlayKind, strength: f32) -> Color {
    let t = strength.clamp(0.0, 1.0);
    let base = match kind {
        OverlayKind::None => return Color::NONE,
        OverlayKind::Service => {
            if t >= 0.65 {
                OK
            } else if t >= 0.35 {
                HI
            } else {
                WARN
            }
        }
        OverlayKind::Congestion => {
            if t >= 0.9 {
                WARN
            } else if t >= 0.4 {
                HI
            } else {
                OK
            }
        }
        OverlayKind::Density => {
            if t >= 0.55 {
                HI
            } else {
                OK
            }
        }
    };
    let alpha = match kind {
        OverlayKind::Density => 0.15 + t * 0.45,
        OverlayKind::Congestion => 0.25 + t * 0.4,
        OverlayKind::Service => 0.2 + (1.0 - t) * 0.35,
        OverlayKind::None => 0.0,
    };
    base.with_alpha(alpha.clamp(0.12, 0.65))
}

/// Service influence at a tile from nearest station score (0..=1).
pub fn service_strength(
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
        let v = (score * falloff).clamp(0.0, 1.0);
        if v > best {
            best = v;
        }
    }
    best
}

/// Congestion 0..=1: occupied tile = 1; neighbour of occupied = 0.55.
pub fn congestion_strength(
    tile: TileCoord,
    network: &TrackNetwork,
    occupancy: &TileOccupancy,
) -> f32 {
    let Some(id) = network.id_at(tile, GROUND_LAYER) else {
        return 0.0;
    };
    if occupancy.by_track.contains_key(&id) {
        return 1.0;
    }
    let mut busy_n = 0u32;
    for dy in -1..=1 {
        for dx in -1..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let n = TileCoord {
                x: tile.x + dx,
                y: tile.y + dy,
            };
            if let Some(nid) = network.id_at(n, GROUND_LAYER) {
                if occupancy.by_track.contains_key(&nid) {
                    busy_n += 1;
                }
            }
        }
    }
    if busy_n > 0 {
        0.55
    } else {
        0.12
    }
}

pub fn density_strength(tile: TileCoord, density: &TownDensity) -> f32 {
    (density.get(tile) / MAX_DENSITY).clamp(0.0, 1.0)
}

pub fn strength_for(
    kind: OverlayKind,
    tile: TileCoord,
    stations: &StationRegistry,
    service: &StationService,
    network: &TrackNetwork,
    occupancy: &TileOccupancy,
    density: &TownDensity,
) -> f32 {
    match kind {
        OverlayKind::None => 0.0,
        OverlayKind::Service => service_strength(tile, stations, service),
        OverlayKind::Congestion => congestion_strength(tile, network, occupancy),
        OverlayKind::Density => density_strength(tile, density),
    }
}

pub fn color_for_strength(kind: OverlayKind, strength: f32) -> Option<Color> {
    if kind == OverlayKind::None || strength <= 0.02 {
        return None;
    }
    Some(overlay_color(kind, strength))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::ids::TrainId;
    use rail_sim::money::Money;
    use rail_sim::track::{try_place_track, TrackTerrain};
    use rail_sim::{MoneyLedger, StationServiceScore};

    #[test]
    fn service_falls_off_with_distance() {
        let mut stations = StationRegistry::default();
        let id = stations.insert("Eastgate", TileCoord { x: 10, y: 10 }, GROUND_LAYER);
        let mut service = StationService::default();
        service.scores.insert(
            id,
            StationServiceScore {
                score: 100,
                ..Default::default()
            },
        );
        let at = service_strength(TileCoord { x: 10, y: 10 }, &stations, &service);
        let far = service_strength(TileCoord { x: 15, y: 10 }, &stations, &service);
        assert!(at > far);
        assert!(at > 0.9);
    }

    #[test]
    fn congestion_marks_occupied_track() {
        let terrain = TrackTerrain::new(8, 8, (0..64).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut money = Money::new(50_000);
        let mut ledger = MoneyLedger::default();
        let tile = TileCoord { x: 3, y: 3 };
        let placed = try_place_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            tile,
            GROUND_LAYER,
        )
        .expect("place");
        let mut occupancy = TileOccupancy::default();
        occupancy.by_track.insert(placed.id, TrainId(1));
        assert_eq!(congestion_strength(tile, &network, &occupancy), 1.0);
        assert_eq!(
            congestion_strength(TileCoord { x: 4, y: 3 }, &network, &occupancy),
            0.0
        );
    }

    #[test]
    fn overlay_color_uses_alpha() {
        let c = overlay_color(OverlayKind::Service, 0.2);
        assert!(c.to_srgba().alpha > 0.1);
    }
}
