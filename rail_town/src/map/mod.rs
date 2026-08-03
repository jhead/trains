//! Map presentation: spawn terrain sprites and drive the orthographic camera.
//!
//! Terrain data lives in [`rail_map::MapGrid`]. This module only draws and
//! navigates — no track placement or sim logic.

mod camera;
mod map_view;
mod terrain;

use bevy::prelude::*;
use rail_map::{generate_map, DEFAULT_MAP_HEIGHT, DEFAULT_MAP_SEED, DEFAULT_MAP_WIDTH};

use camera::{apply_camera_focus, camera_pan, camera_zoom, setup_map_camera};
use map_view::{
    block_zoom_in_map_view, exit_map_view_before_focus, map_view_click_fly, setup_map_view_banner,
    toggle_map_view,
};
use terrain::{rebuild_dirty_terrain, setup_terrain, TerrainDirty};

pub use camera::{CameraFocusRequest, MapCamera};
pub use map_view::MapViewState;

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
            .init_resource::<crate::input::KeyBindings>()
            .init_resource::<CameraFocusRequest>()
            .init_resource::<MapViewState>()
            .init_resource::<TerrainDirty>()
            .add_systems(
                Startup,
                (setup_map_camera, setup_terrain, setup_map_view_banner).chain(),
            )
            .add_systems(
                Update,
                (
                    rebuild_dirty_terrain,
                    map_view_click_fly,
                    exit_map_view_before_focus.after(map_view_click_fly),
                    apply_camera_focus.after(exit_map_view_before_focus),
                    camera_pan,
                    camera_zoom,
                    toggle_map_view.after(camera_zoom),
                    block_zoom_in_map_view
                        .after(toggle_map_view)
                        .after(camera_zoom),
                ),
            );
    }
}
