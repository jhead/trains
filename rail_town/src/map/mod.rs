//! Map presentation: spawn terrain sprites and drive the orthographic camera.
//!
//! Terrain data lives in [`rail_map::MapGrid`]. This module only draws and
//! navigates — no track placement or sim logic.
//!
//! # Isometric evaluation prototype
//!
//! This branch draws the world in 2:1 dimetric projection instead of top-down.
//! The projection itself lives in `rail_map::coords`, so everything that ever
//! asked where a tile is gets reprojected for free; what changes here is:
//!
//! - [`terrain::iso`] draws diamonds and cliff faces instead of
//!   [`terrain::chunk`]'s axis-aligned chunk textures. The compositor still
//!   compiles and still has its own tests — it is simply not registered.
//! - [`iso_sort`] y-sorts the world every frame ([`iso_depth`] holds the maths).
//! - **Map View is gated off.** Its plate is a top-down schematic of a world
//!   that is no longer top-down, and its click-to-fly inverts a projection that
//!   no longer applies. Fixing it is a second design problem and tells the owner
//!   nothing about whether the projection is worth having.

mod camera;
pub mod iso_depth;
pub mod iso_sort;
// Gated off below, kept whole so switching back is a plugin edit, not a revert.
#[allow(dead_code)]
mod map_view;
#[allow(dead_code)]
mod schematic;
mod terrain;

use bevy::prelude::*;
use rail_map::{generate_map, DEFAULT_MAP_HEIGHT, DEFAULT_MAP_SEED, DEFAULT_MAP_WIDTH};

use camera::{apply_camera_focus, camera_pan, camera_zoom, setup_map_camera};
use iso_sort::iso_depth_sort;
use terrain::iso::{rebuild_iso_terrain, setup_iso_terrain, IsoTerrainState};

pub use camera::{ortho_scale_for_zoom, CameraFocusRequest, MapCamera};
// The tile-shaped diamond every ghost / ring / overlay cell tints, so nothing
// draws a square footprint onto a diamond grid.
pub use terrain::iso::IsoDiamond;
// Still exported, and still a resource everything can read — it is simply never
// set true on this branch, because the Map View is gated off (see above).
pub use map_view::MapViewState;
pub use schematic::SCHEMATIC_OVERLAY_Z;
// The one kind+height -> colour contract, for every schematic read of the
// world — the Map View plate and the New Map preview must not disagree with
// the ground they predict.
pub use terrain::material::terrain_color;

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
        // The lift the projection applies has to be right before anything asks
        // where a tile is — including this plugin's own camera framing.
        rail_map::set_iso_heights(&grid);
        app.insert_resource(grid)
            .init_resource::<crate::input::KeyBindings>()
            .init_resource::<CameraFocusRequest>()
            .init_resource::<MapViewState>()
            .init_resource::<IsoTerrainState>()
            .add_systems(Startup, (setup_map_camera, setup_iso_terrain).chain())
            .add_systems(
                Update,
                (
                    rebuild_iso_terrain,
                    apply_camera_focus,
                    camera_pan.in_set(crate::input::PlayerVerbSet),
                    camera_zoom.in_set(crate::input::PlayerVerbSet),
                ),
            )
            // Depth sorting is the last word on where a sprite draws, so it runs
            // after every system that could have moved one and before the
            // transforms are propagated to the renderer.
            .add_systems(
                PostUpdate,
                iso_depth_sort.before(TransformSystems::Propagate),
            );

        // ── Gated off for the iso prototype ────────────────────────────────
        //
        // Map View (`map_view`, `schematic`) paints a top-down plate of the
        // world and flies the camera by inverting a top-down projection. Both
        // are wrong here and neither is part of what the owner is evaluating,
        // so the systems are not registered. `MapViewState` stays as a resource
        // that is never true, which is enough for the HUD, the mixer and the
        // inspect slice to keep compiling and behaving.
        //
        // Also unregistered: `terrain::chunk`'s compositor (`setup_terrain`,
        // `rebuild_dirty_terrain`, `TerrainDirty`) — replaced by `terrain::iso`.
    }
}
