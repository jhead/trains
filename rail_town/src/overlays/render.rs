//! Spawn / update translucent overlay sprites over map tiles.

use bevy::prelude::*;
use rail_map::{tile_to_world, MapGrid, TILE_SIZE};
use rail_sim::ids::TileCoord;
use rail_sim::{StationRegistry, StationService, TileOccupancy, TownDensity, TrackNetwork};

use super::score::{color_for_strength, strength_for};
use super::{ActiveOverlay, OverlayKind};

#[derive(Component)]
pub struct OverlayTileSprite {
    #[allow(dead_code)] // retained for picking / debug later
    pub coord: TileCoord,
}

pub fn sync_overlay_sprites(
    mut commands: Commands,
    overlay: Res<ActiveOverlay>,
    map: Res<MapGrid>,
    stations: Res<StationRegistry>,
    service: Res<StationService>,
    network: Res<TrackNetwork>,
    occupancy: Res<TileOccupancy>,
    density: Res<TownDensity>,
    existing: Query<(Entity, &OverlayTileSprite)>,
) {
    let _perf = crate::overlays::perf::scope("sync_overlay_sprites");
    // Rebuild each frame while an overlay is active — map is small (64²) and
    // sets are ring / track scoped. Clears cleanly when toggled off.
    for (entity, _) in &existing {
        commands.entity(entity).despawn();
    }

    if overlay.0 == OverlayKind::None {
        return;
    }

    let mut tiles: Vec<TileCoord> = Vec::new();
    match overlay.0 {
        OverlayKind::Congestion => {
            for piece in network.iter() {
                tiles.push(piece.tile);
            }
        }
        OverlayKind::Density => {
            for (tile, d) in density.iter() {
                if d > 0.02 {
                    tiles.push(tile);
                }
            }
            for station in stations.iter() {
                push_ring(&mut tiles, station.tile, &map);
            }
        }
        OverlayKind::Service => {
            for station in stations.iter() {
                push_ring(&mut tiles, station.tile, &map);
            }
        }
        OverlayKind::None => {}
    }
    tiles.sort_by_key(|t| (t.y, t.x));
    tiles.dedup();

    for tile in tiles {
        let strength = strength_for(
            overlay.0,
            tile,
            &stations,
            &service,
            &network,
            &occupancy,
            &density,
        );
        let Some(color) = color_for_strength(overlay.0, strength) else {
            continue;
        };
        let (wx, wy) = tile_to_world(tile);
        commands.spawn((
            Sprite::from_color(color, Vec2::splat(TILE_SIZE)),
            Transform::from_xyz(wx, wy, 4.5),
            OverlayTileSprite { coord: tile },
        ));
    }
}

fn push_ring(tiles: &mut Vec<TileCoord>, center: TileCoord, map: &MapGrid) {
    for dy in -rail_sim::GROWTH_RADIUS..=rail_sim::GROWTH_RADIUS {
        for dx in -rail_sim::GROWTH_RADIUS..=rail_sim::GROWTH_RADIUS {
            let t = TileCoord {
                x: center.x + dx,
                y: center.y + dy,
            };
            if map.contains(t) {
                tiles.push(t);
            }
        }
    }
}
