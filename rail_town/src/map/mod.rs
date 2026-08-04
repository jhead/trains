//! Map presentation: spawn terrain sprites and drive the orthographic camera.
//!
//! Terrain data lives in [`rail_map::MapGrid`]. This module only draws and
//! navigates — no track placement or sim logic.
//!
//! # Two projections, one world
//!
//! The world can be drawn from directly above (the shipping view) or in 2:1
//! dimetric, and the player can swap between them in session against the same
//! world and the same save. The projection itself lives in `rail_map::coords`,
//! so everything that ever asked where a tile is gets reprojected for free.
//! What this module carries is the difference between the two *renderers*:
//!
//! - [`terrain::chunk`] composites 16 × 16 tiles into one sprite each and
//!   autotiles the seams. [`terrain::iso`] draws one diamond per tile plus up
//!   to two cliff faces from a baked atlas. Exactly one of them is registered
//!   at a time; both atlases are baked at startup so a flip does not pay for a
//!   bake.
//! - [`iso_sort`] y-sorts the world every frame in isometric ([`iso_depth`]
//!   holds the maths), and does not run at all in top-down, where the layer
//!   bands in brief 01 §6.1 are the whole of depth.
//! - The **Map View** runs in both. Brief 02 §6 is explicit that the plate is
//!   "a second, purpose-built rendering" rather than a zoomed-out camera, so it
//!   was never a picture of the world and never needed to match its projection.
//!   [`schematic`] lays the plate out in tile order at a fixed scale, and
//!   [`map_view`]'s click-to-fly resolves a plate point to a tile and hands
//!   *that* to the camera.
//!
//! [`projection`] owns the flip and everything that has to follow it.

mod camera;
pub mod iso_depth;
pub mod iso_sort;
mod map_view;
pub mod projection;
mod schematic;
mod terrain;

use bevy::prelude::*;
use rail_map::{generate_map, DEFAULT_MAP_HEIGHT, DEFAULT_MAP_SEED, DEFAULT_MAP_WIDTH};

use camera::{apply_camera_focus, camera_pan, camera_zoom, setup_map_camera};
use iso_sort::iso_depth_sort;
use map_view::{
    block_zoom_in_map_view, exit_map_view_before_focus, map_view_click_fly, setup_map_view_banner,
    toggle_map_view,
};
use projection::{
    apply_projection_setting, drawing_iso, drawing_top_down, install_boot_projection,
    install_map_heights, toggle_projection_hotkey,
};
use schematic::{
    mark_schematic_dirty, rebake_schematic, setup_schematic, sync_schematic_trains,
    sync_schematic_visibility, SchematicState,
};
use terrain::chunk::{rebuild_dirty_terrain, setup_terrain_atlas, TerrainDirty};
use terrain::iso::{rebuild_iso_terrain, setup_iso_atlas, IsoTerrainState};

pub use camera::{ortho_scale_for_zoom, CameraFocusRequest, MapCamera};
pub use map_view::MapViewState;
pub use projection::ViewProjection;
pub use schematic::SCHEMATIC_OVERLAY_Z;
// The tile-shaped footprint every ghost / ring / overlay cell tints: a square
// from above, a diamond in isometric, so nothing draws a square onto a diamond
// grid or a diamond onto a square one.
pub use terrain::iso::TileMark;
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
        // The lift the isometric projection applies has to be right before
        // anything asks where a tile is — including this plugin's own camera
        // framing. Harmless from above, which never reads it.
        rail_map::set_iso_heights(&grid);
        app.insert_resource(grid)
            .init_resource::<crate::input::KeyBindings>()
            .init_resource::<CameraFocusRequest>()
            .init_resource::<MapViewState>()
            .init_resource::<SchematicState>()
            .init_resource::<TerrainDirty>()
            .init_resource::<IsoTerrainState>()
            .init_resource::<ViewProjection>()
            // Before `Startup`, so the first frame is drawn in the projection
            // the player left the game in. `PreStartup` is also where the shell
            // installs its world; this reads `Settings`, which the shell has
            // already loaded in its `build`.
            .add_systems(PreStartup, install_boot_projection)
            .add_systems(
                Startup,
                (
                    install_map_heights,
                    setup_map_camera,
                    // Both banks, whichever view opens: a flip is then a
                    // re-spawn and never a bake.
                    setup_terrain_atlas,
                    setup_iso_atlas,
                    setup_map_view_banner,
                    setup_schematic,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    // The flip runs before the renderers look at their state,
                    // so the frame it happens on is the frame the new one
                    // builds.
                    toggle_projection_hotkey.in_set(crate::input::PlayerVerbSet),
                    apply_projection_setting.after(toggle_projection_hotkey),
                    rebuild_dirty_terrain
                        .after(apply_projection_setting)
                        .run_if(drawing_top_down),
                    rebuild_iso_terrain
                        .after(apply_projection_setting)
                        .run_if(drawing_iso),
                    map_view_click_fly.in_set(crate::input::PlayerVerbSet),
                    exit_map_view_before_focus.after(map_view_click_fly),
                    apply_camera_focus.after(exit_map_view_before_focus),
                    camera_pan.in_set(crate::input::PlayerVerbSet),
                    camera_zoom.in_set(crate::input::PlayerVerbSet),
                    toggle_map_view
                        .after(camera_zoom)
                        .in_set(crate::input::PlayerVerbSet),
                    block_zoom_in_map_view
                        .after(toggle_map_view)
                        .after(camera_zoom),
                ),
            )
            // The Map View's own render. Painting follows the toggle and the
            // camera, so the plate is right on the frame the view opens.
            .add_systems(
                Update,
                (
                    mark_schematic_dirty,
                    rebake_schematic,
                    sync_schematic_visibility,
                    sync_schematic_trains,
                )
                    .chain()
                    .after(toggle_map_view)
                    .after(camera_pan),
            )
            // Depth sorting is the last word on where a sprite draws, so it runs
            // after every system that could have moved one and before the
            // transforms are propagated to the renderer. Isometric only: from
            // above, the layer a spawner writes *is* the depth.
            .add_systems(
                PostUpdate,
                iso_depth_sort
                    .before(TransformSystems::Propagate)
                    .run_if(drawing_iso),
            );
    }
}

#[cfg(test)]
pub(crate) mod tests {
    /// Serialises every test in this crate that installs a projection.
    ///
    /// The live projection is a process-global (see `rail_map::coords` for why),
    /// and Rust runs a crate's tests on one thread pool, so two tests that each
    /// set it would otherwise read each other's. Take this before calling
    /// `rail_map::set_projection`, and put the old value back.
    pub static PROJECTION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Hold a projection for the body of one test and restore it afterwards.
    pub struct ProjectionGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        restore: rail_map::Projection,
    }

    impl ProjectionGuard {
        pub fn new(projection: rail_map::Projection) -> Self {
            let lock = PROJECTION_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let restore = rail_map::set_projection(projection);
            Self {
                _lock: lock,
                restore,
            }
        }
    }

    impl Drop for ProjectionGuard {
        fn drop(&mut self) {
            rail_map::set_projection(self.restore);
        }
    }
}
