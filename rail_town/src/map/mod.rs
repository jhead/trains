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
pub mod paths;
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
    anchor_world_sprites, apply_projection_setting, drawing_iso, drawing_top_down,
    follow_map_heights, install_boot_projection, toggle_projection_hotkey,
};
use schematic::{
    mark_schematic_dirty, rebake_schematic, setup_schematic, sync_schematic_trains,
    sync_schematic_visibility, SchematicState,
};
use terrain::chunk::{
    mark_worn_chunks_dirty, rebuild_dirty_terrain, setup_terrain_atlas, TerrainDirty,
};
use terrain::iso::{rebuild_iso_terrain, setup_iso_atlas, sync_iso_paths, IsoTerrainState};

pub use camera::{ortho_scale_for_zoom, CameraFocusRequest, MapCamera};
pub use map_view::MapViewState;
pub use projection::{GroundAnchor, ViewProjection};
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
        // This plugin, and every system it registers below, writes the
        // process-global projection state. Under test that state is owned
        // rather than shared, and the owning is done here rather than by each
        // test remembering to: a test cannot get these systems without this
        // line, so there is nothing left to forget. See `tests::ProjectionGuard`.
        #[cfg(test)]
        tests::own_globals_for(app);

        let grid = generate_map(self.width, self.height, self.seed);
        // The lift the isometric projection applies has to be right before
        // anything asks where a tile is — including this plugin's own camera
        // framing. Harmless from above, which never reads it.
        crate::map::projection::set_iso_heights(&grid);
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
            // Ahead of all of `Update` by the schedule rather than by anyone
            // remembering an `.after()`: the lift has to be this world's before
            // a single system asks where a tile is. See `follow_map_heights`.
            .add_systems(PreUpdate, follow_map_heights)
            .add_systems(
                Startup,
                (
                    follow_map_heights,
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
                    // Desire paths, in each renderer's own idiom: from above a
                    // worn tile is a chunk to re-composite, in isometric it is
                    // a sprite of its own. Both read *level transitions* and
                    // never wear itself, so both are idle on the frames — very
                    // nearly all of them — where no tile changed step.
                    mark_worn_chunks_dirty
                        .after(apply_projection_setting)
                        .before(rebuild_dirty_terrain)
                        .run_if(drawing_top_down),
                    rebuild_dirty_terrain
                        .after(apply_projection_setting)
                        .run_if(drawing_top_down),
                    rebuild_iso_terrain
                        .after(apply_projection_setting)
                        .run_if(drawing_iso),
                    // Everything that spawns once and then holds still. Late
                    // in `Update`, so a spawner that ran this frame is already
                    // covered, and before `PostUpdate`'s depth sort.
                    anchor_world_sprites.after(apply_projection_setting),
                    // After the terrain rebuild, which owns despawning
                    // everything that module draws — finding no path sprites
                    // is how this learns it has to draw them all again.
                    sync_iso_paths
                        .after(rebuild_iso_terrain)
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
    use bevy::app::{First, Last, PostStartup, PostUpdate, PreStartup, PreUpdate, Startup, Update};
    use bevy::ecs::schedule::{ExecutorKind, ScheduleLabel, Schedules};
    use bevy::prelude::*;

    /// Own the projection globals for the body of one test, and put the
    /// projection back afterwards.
    ///
    /// The live projection and the iso height field are process-globals (see
    /// `rail_map::coords` for why they have to be), and Rust runs a crate's
    /// tests as threads in one process. Two tests that each write one would
    /// otherwise be writing into each other — and the one that *fails* would be
    /// the innocent one, several files away, one run in eight.
    ///
    /// So this is not a convention any more. Every write this crate makes goes
    /// through `map::projection`'s three wrappers, and in a test build each of
    /// them asserts the writing thread holds one of these — forgetting is a
    /// panic naming the fix, not a wrong number in a neighbour.
    /// [`own_globals_for`] is how an app that writes them from its own systems
    /// holds one for as long as it exists.
    ///
    /// Re-entrant per thread: taking a second one nests, so a test can pin the
    /// projection it wants and then build an app that takes its own.
    ///
    /// # Lock order
    ///
    /// This is the **innermost** of the crate's test-wide locks. An app holds
    /// it for as long as the app lives, and apps are built last, so a test that
    /// also wants `shell::lock_save_root` must take that one *first*. Two tests
    /// taking the two in opposite orders deadlock the whole run — which is a
    /// hang, not a failure, and reads as CI being slow.
    pub struct ProjectionGuard {
        _owned: rail_map::testing::WorldGuard,
        restore: rail_map::Projection,
    }

    impl ProjectionGuard {
        /// Own the globals and put `projection` live until this is dropped.
        pub fn new(projection: rail_map::Projection) -> Self {
            let owned = rail_map::testing::WorldGuard::acquire();
            let restore = crate::map::projection::set_projection(projection);
            Self {
                _owned: owned,
                restore,
            }
        }

        /// Own the globals without saying anything about what they should be —
        /// for a holder that exists to keep other threads out rather than to
        /// choose a view. Whatever the projection was comes back on drop.
        pub fn hold() -> Self {
            let owned = rail_map::testing::WorldGuard::acquire();
            Self {
                _owned: owned,
                restore: rail_map::projection(),
            }
        }
    }

    impl Drop for ProjectionGuard {
        fn drop(&mut self) {
            // Still the owner here: the field, and with it the ownership, is
            // released after this body runs.
            crate::map::projection::set_projection(self.restore);
        }
    }

    /// Give `app` the projection globals for as long as it lives.
    ///
    /// [`super::MapPlugin`] calls this on itself, so *any* test that builds a
    /// map app is covered without knowing this exists — which is the point,
    /// since the systems that write the globals are the plugin's, not the
    /// test's, and they run frames after the line the author wrote. A test that
    /// registers those systems into a bare app by hand should call this too.
    ///
    /// Two halves, and both are load-bearing:
    ///
    /// - The guard goes in as a non-send resource, so it is dropped exactly
    ///   when the app is. Re-entrancy means a test that already took one nests
    ///   rather than deadlocking, and two apps on two threads serialise instead
    ///   of interleaving.
    /// - The schedules are pinned to one thread, because Bevy's multi-threaded
    ///   executor hands a `Send` system to a shared, process-wide task pool —
    ///   so `apply_projection_setting` would otherwise write from a thread that
    ///   is nobody's in particular, and ownership could not be checked at all.
    pub(crate) fn own_globals_for(app: &mut App) {
        // Idempotent: a test helper that already did this and then added
        // `MapPlugin` would otherwise *replace* the resource, dropping the
        // outer guard while the nested one it just took is still in the app —
        // handing the globals away with the app still running on them.
        let already_owned = app.world().get_non_send_resource::<ProjectionGuard>();
        if already_owned.is_none() {
            app.insert_non_send_resource(ProjectionGuard::hold());
        }
        run_on_the_calling_thread(app);
    }

    /// Run every one of `app`'s schedules on whichever thread calls `update`.
    fn run_on_the_calling_thread(app: &mut App) {
        // Everything the plugins built before this one, which is where the
        // state-transition and fixed-timestep schedules come from.
        if let Some(mut schedules) = app.world_mut().get_resource_mut::<Schedules>() {
            for (_, schedule) in schedules.iter_mut() {
                schedule.set_executor_kind(ExecutorKind::SingleThreaded);
            }
        }
        // ... and the main loop by name, created here if it does not exist yet
        // so that the plugins built *after* this one inherit the setting when
        // they add their systems.
        for label in [
            First.intern(),
            PreStartup.intern(),
            Startup.intern(),
            PostStartup.intern(),
            PreUpdate.intern(),
            Update.intern(),
            PostUpdate.intern(),
            Last.intern(),
        ] {
            app.edit_schedule(label, |schedule| {
                schedule.set_executor_kind(ExecutorKind::SingleThreaded);
            });
        }
    }

    /// The enforcement itself, because a check that works is invisible.
    ///
    /// Every write in this crate is checked because it goes through
    /// `map::projection`'s wrappers — a call site that reaches past them to
    /// `rail_map` directly is not, and neither is a build where the `cfg(test)`
    /// line inside a wrapper has been deleted as "dead code". Nothing else in
    /// the suite would fail if that happened: every test above would still pass
    /// and the race would quietly be back. This is what notices.
    ///
    /// Deterministic in every interleaving: an unowned write is refused whether
    /// the globals are free or held by some other test's thread, and it is
    /// refused *before* it touches anything, so this cannot disturb one.
    #[test]
    #[should_panic(expected = "does not own the projection globals")]
    fn writing_the_globals_without_owning_them_is_refused() {
        crate::map::projection::set_projection(rail_map::Projection::Iso);
    }
}
