//! Spawn placeholder coloured sprites for each map tile.
//!
//! Tiles meet edge-to-edge at full [`TILE_SIZE`] (pixel contract §2.3). A grid
//! overlay may be added later behind a toggle — never baked into terrain size.

use bevy::prelude::*;
use rail_map::{tile_to_world, MapGrid, TerrainKind, TILE_SIZE};
use rail_sim::ids::TileCoord;

use crate::palette::{
    GRASS_D, GRASS_M, HILL_D, HILL_L, HILL_M, ROCK_D, ROCK_L, ROCK_M, SAND_D, SAND_M, SNOW,
    WATER_D, WATER_F, WATER_L, WATER_M,
};

/// Marker on each terrain sprite; `coord` is for picking / tools in later slices.
#[derive(Component)]
#[allow(dead_code)] // Read by track tools / picking in later slices.
pub struct MapTileSprite {
    pub coord: TileCoord,
}

pub fn spawn_map_tiles(mut commands: Commands, map: Res<MapGrid>) {
    let size = Vec2::splat(TILE_SIZE);
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

/// Map height / kind onto the binding terrain ramps (brief 01 §3).
///
/// Flat ground stays in the dark/mid steps; light steps and snow mark elevation.
pub fn terrain_color(kind: TerrainKind, height: i8) -> Color {
    match kind {
        TerrainKind::Water => match height {
            ..=-8 => WATER_D,
            -7..=-4 => WATER_M,
            -3..=-2 => WATER_L,
            _ => WATER_F,
        },
        TerrainKind::Beach => {
            if height <= 0 {
                SAND_D
            } else {
                SAND_M
            }
        }
        // Flat plains: bottom two-thirds of the grass ramp only (no GRASS_L).
        TerrainKind::Plains => {
            if height <= 3 {
                GRASS_D
            } else {
                GRASS_M
            }
        }
        TerrainKind::Hills => match height {
            ..=7 => HILL_D,
            8..=9 => HILL_M,
            _ => HILL_L,
        },
        TerrainKind::Mountain => match height {
            ..=12 => ROCK_D,
            13..=14 => ROCK_M,
            15 => ROCK_L,
            _ => SNOW,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::{GRASS_D, GRASS_L, WATER_D, WATER_F};

    #[test]
    fn water_depth_uses_ramp_not_flat_blue() {
        assert_eq!(terrain_color(TerrainKind::Water, -10), WATER_D);
        assert_eq!(terrain_color(TerrainKind::Water, -1), WATER_F);
        assert_ne!(
            terrain_color(TerrainKind::Water, -10),
            terrain_color(TerrainKind::Water, -1)
        );
    }

    #[test]
    fn plains_never_use_grass_light() {
        for h in 2..=5 {
            let c = terrain_color(TerrainKind::Plains, h);
            assert_ne!(c, GRASS_L);
            assert!(c == GRASS_D || c == GRASS_M);
        }
    }

    #[test]
    fn mountain_and_hills_step_with_height() {
        assert_ne!(
            terrain_color(TerrainKind::Hills, 6),
            terrain_color(TerrainKind::Hills, 10)
        );
        assert_ne!(
            terrain_color(TerrainKind::Mountain, 11),
            terrain_color(TerrainKind::Mountain, 16)
        );
    }
}
