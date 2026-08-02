//! Spawn placeholder colored sprites for each map tile.

use bevy::prelude::*;
use rail_map::{tile_to_world, MapGrid, TerrainKind, TILE_SIZE};
use rail_sim::ids::TileCoord;

/// Marker on each terrain sprite; `coord` is for picking / tools in later slices.
#[derive(Component)]
#[allow(dead_code)] // Read by track tools / picking in later slices.
pub struct MapTileSprite {
    pub coord: TileCoord,
}

pub fn spawn_map_tiles(mut commands: Commands, map: Res<MapGrid>) {
    let size = Vec2::splat(TILE_SIZE - 1.0); // 1px gap so the grid reads clearly
    for y in 0..map.height as i32 {
        for x in 0..map.width as i32 {
            let coord = TileCoord { x, y };
            let tile = map.tile(coord);
            let (wx, wy) = tile_to_world(coord);
            commands.spawn((
                Sprite::from_color(terrain_color(tile.kind, tile.height), size),
                Transform::from_xyz(wx, wy, 0.0),
                MapTileSprite { coord },
            ));
        }
    }
}

fn terrain_color(kind: TerrainKind, height: i8) -> Color {
    match kind {
        TerrainKind::Water => {
            let t = ((-height as f32) / 12.0).clamp(0.0, 1.0);
            Color::srgb(0.12 + t * 0.05, 0.28 + t * 0.1, 0.55 + t * 0.2)
        }
        TerrainKind::Beach => Color::srgb(0.82, 0.75, 0.52),
        TerrainKind::Plains => {
            let t = (height as f32 / 5.0).clamp(0.0, 1.0);
            Color::srgb(0.28 + t * 0.1, 0.55 + t * 0.15, 0.25)
        }
        TerrainKind::Hills => {
            let t = ((height as f32 - 5.0) / 5.0).clamp(0.0, 1.0);
            Color::srgb(0.35 + t * 0.1, 0.48 - t * 0.05, 0.28)
        }
        TerrainKind::Mountain => {
            let t = ((height as f32 - 10.0) / 6.0).clamp(0.0, 1.0);
            Color::srgb(0.45 + t * 0.25, 0.45 + t * 0.25, 0.48 + t * 0.25)
        }
    }
}
