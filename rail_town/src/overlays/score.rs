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

/// Congestion 0..=1 — **sustained use**, not a train-position light.
///
/// Brief 07 §4.1 asks for "track under sustained heavy use", and the previous
/// read — 1.0 wherever a train stood this frame, 0.55 beside it — made a
/// saturated corridor and an empty one identical between trains, with the
/// tint chasing the trains around. The signal now:
///
/// - A **standing train** is 1.0. A queue is the acute symptom and it should
///   read at full strength.
/// - A tile **crossed recently** fades from 0.85 over the same
///   [`memory window`](rail_sim::trains::POLISH_MEMORY_TICKS) the railhead polish
///   uses. A busy corridor is re-crossed before it can fade, so it holds its
///   tint; a one-off movement clears in seconds. Recency under a rolling
///   window *is* duty cycle, which is what "sustained" means here.
/// - Any other track keeps a **0.06 outline** so the overlay still shows the
///   network it is scoring.
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
    if let Some(since) = occupancy.ticks_since_crossed(id) {
        let window = rail_sim::trains::POLISH_MEMORY_TICKS as f32;
        let freshness = 1.0 - (since as f32 / window).min(1.0);
        if freshness > 0.0 {
            return 0.06 + 0.79 * freshness;
        }
    }
    0.06
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

    /// Brief 07 §4.1: the tint reads sustained use — a corridor crossed
    /// moments ago holds most of its strength, an old crossing has faded to
    /// the outline, and the tile never flips to zero while it carries track.
    #[test]
    fn congestion_fades_with_the_crossing_memory() {
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
        occupancy.last_crossed.insert(placed.id, 0);

        occupancy.tick = 32;
        let fresh = congestion_strength(tile, &network, &occupancy);
        occupancy.tick = rail_sim::trains::POLISH_MEMORY_TICKS - 1;
        let stale = congestion_strength(tile, &network, &occupancy);

        assert!(fresh > 0.7, "a fresh crossing reads hot: {fresh}");
        assert!(stale < 0.1, "an old crossing has faded: {stale}");
        assert!(stale >= 0.06, "track never drops below the outline: {stale}");
        assert!(fresh > stale);
    }

    #[test]
    fn overlay_color_uses_alpha() {
        let c = overlay_color(OverlayKind::Service, 0.2);
        assert!(c.to_srgba().alpha > 0.1);
    }
}
