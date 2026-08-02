//! Map presentation: spawn terrain sprites and drive the orthographic camera.
//!
//! Terrain data lives in [`rail_map::MapGrid`]. This module only draws and
//! navigates — no track placement or sim logic.

mod camera;
mod spawn;

use bevy::prelude::*;
use rail_map::{generate_map, DEFAULT_MAP_HEIGHT, DEFAULT_MAP_SEED, DEFAULT_MAP_WIDTH};

use camera::{camera_pan, camera_zoom, setup_map_camera};
use spawn::spawn_map_tiles;

/// Inserts a generated [`MapGrid`] and registers spawn / camera systems.
pub struct MapPlugin {
    pub width: u32,
    pub height: u32,
    pub seed: u64,
}

impl Default for MapPlugin {
    fn default() -> Self {
        Self {
            width: DEFAULT_MAP_WIDTH,
            height: DEFAULT_MAP_HEIGHT,
            seed: DEFAULT_MAP_SEED,
        }
    }
}

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        let grid = generate_map(self.width, self.height, self.seed);
        app.insert_resource(grid)
            .add_systems(Startup, (setup_map_camera, spawn_map_tiles).chain())
            .add_systems(Update, (camera_pan, camera_zoom));
    }
}
