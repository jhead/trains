//! Swapping the world's projection while the game is running.
//!
//! `rail_map::coords` holds the live [`Projection`] and every conversion that
//! reads it. This module is the presentation half: it decides *when* the flag
//! moves, and it does the work that has to follow a move.
//!
//! # What a flip is
//!
//! Mostly, nothing — and that is the point. Almost every sprite in the world
//! re-derives its position from `tile_to_world` every frame already, because
//! that is how the reconcile passes were written: stations, industries,
//! buildings, trains, peeps, smoke, water decals and alerts all move on the
//! next frame with no prompting. What is left is the handful of things that
//! *cache* a position or a drawing, and the flip does exactly four things for
//! them:
//!
//! - **Swap the terrain renderer.** The two are different pipelines drawing the
//!   same data (`terrain::chunk` composites 16 × 16 chunk textures;
//!   `terrain::iso` spawns a diamond and up to two cliff faces per tile), so
//!   the outgoing one's sprites are despawned and the incoming one rebuilds
//!   from nothing. Both atlases are baked at startup, so no bake happens here.
//! - **Drop the track sprites.** Track art is baked per projection, so the
//!   pieces need re-deriving. `track::visuals` treats "there are pieces and no
//!   art" as the same wholesale rebuild a load asks for, and its bank is keyed
//!   on the projection, so the flip back re-uses cells instead of re-painting.
//! - **Re-aim the camera.** The tile under the middle of the screen, and the
//!   fraction of a tile the camera stood off its centre, are resolved *before*
//!   the flag moves and put back after.
//! - **Drop the depth sort's memory.** `iso_sort` stores each sprite's layer z
//!   in an `IsoLayer` and rewrites the real z from the tile under it. Leaving
//!   either behind on the way out of isometric would strand every world sprite
//!   in the sort band, above the day tint and under nothing.
//!
//! It also raises [`PendingWorld::mark_rebuilding`](crate::shell::PendingWorld),
//! the shell's "everything drawn is stale, derive it again" flag. Nothing reads
//! it today — the shell documents it as the seam for anything that later needs
//! telling — but a flip leaves the world in precisely the condition a load
//! does, so it says so through the same channel rather than growing a second
//! one that could drift.
//!
//! # What a flip is not
//!
//! It is not sim state. Nothing in `rail_sim` reads the projection, no command
//! is pushed, no entity carrying sim data is touched, and `SCHEMA_VERSION` is
//! untouched. `a_flip_is_invisible_to_the_simulation` runs a world forward
//! across a flip and hashes it against the same world run without one.
//!
//! # Where the mode is kept
//!
//! In `Settings.display.isometric`, which is a `Display` row like any other and
//! persists through the flat key-value settings file. That file has no schema
//! and reads absent keys as the default, so an old profile loads as top-down
//! and a new one written by this build loads on an old build with the extra key
//! ignored. It is not in a save: a save records the world, and how the player
//! is looking at the world is not part of it.
//!
//! [`Settings`](crate::shell::Settings) is therefore the single source of
//! truth. The key binding cycles that setting rather than the projection, so
//! the Controls tab, the Display tab and the hotkey cannot disagree.

use bevy::prelude::*;
// `Projection` is a Bevy component too (the camera's), so the projection this
// module is about is always spelled out.
use rail_map::{MapGrid, Projection as MapProjection};

use super::camera::{
    default_zoom_index_for, ortho_scale_for_zoom, zoom_factor_at, CameraZoomIndex, MapCamera,
};
use super::iso_sort::IsoLayer;
use super::terrain::chunk::{despawn_flat_terrain, TerrainChunk, TerrainDirty};
use super::terrain::iso::{despawn_iso_terrain, IsoTerrain, IsoTerrainState};
use crate::input::{ControlAction, KeyBindings};
use crate::shell::{PendingWorld, Settings};
use crate::track::TrackSprite;

/// The projection the presentation is currently built for.
///
/// Mirrors `rail_map`'s global so systems can take a run condition and change
/// detection on it. The global is the one the coordinate helpers read; this is
/// the one the schedule reads.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewProjection(pub MapProjection);

impl ViewProjection {
    #[inline]
    pub fn is_iso(self) -> bool {
        self.0 == MapProjection::Iso
    }
}

/// Where a sprite stands on the **ground plane**, so it can be put back.
///
/// # The bug this exists to make unrepeatable
///
/// Most presentation reconciles: stations, industries, trains, ghosts and
/// overlays all re-derive their `Transform` from a tile every frame, so a
/// projection flip reaches them for free and so does spawning under one. The
/// rest spawn once, write a `Transform`, and never think about it again — town
/// buildings, rural props, water shimmer, chimney smoke, construction dust.
/// Those need two separate things to be true, and the shipped code had neither:
/// the position has to be *projected* when it is computed, and it has to be
/// *recomputed* when the projection changes underneath it.
///
/// Both were missed the same way. `lot_base` returns ground texels
/// (`tile.x * TILE_TEXELS + jitter`) and the spawner passed them straight into
/// a `Transform`; `pose_for` wrote `(pos.x + 0.5) * TILE_SIZE`. Those are the
/// top-down projection written out by hand, so both were correct from above and
/// both put their sprites up-and-right of the diamond in isometric — houses out
/// over the river, some off the map entirely.
///
/// So: carry the ground position, and let [`anchor_world_sprites`] own the
/// `Transform`. A spawner that attaches one of these cannot get the projection
/// wrong, because it never writes the projected value; and a flip repositions
/// everything wearing one without knowing what any of it is.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct GroundAnchor(pub Vec2);

impl GroundAnchor {
    #[inline]
    pub fn new(gx: f32, gy: f32) -> Self {
        Self(Vec2::new(gx, gy))
    }

    /// The world position this anchor resolves to right now.
    #[inline]
    pub fn world(self) -> Vec2 {
        let (x, y) = rail_map::ground_to_world(self.0.x, self.0.y);
        Vec2::new(x, y)
    }

    /// A transform at this anchor, on layer `z`. What a spawner writes, so the
    /// sprite is in the right place on the frame it appears rather than on the
    /// one after.
    #[inline]
    pub fn transform(self, z: f32) -> Transform {
        let world = self.world();
        Transform::from_xyz(world.x, world.y, z)
    }
}

/// Keep every [`GroundAnchor`] over the ground it is anchored to.
///
/// Writes `x` and `y` only: `z` belongs to whoever spawned the sprite (its
/// layer) and, in isometric, to `iso_sort` (its depth). Writes only when the
/// value actually moves, so a still frame costs a comparison per anchored
/// sprite and no change-detection traffic at all.
///
/// This runs every frame rather than only on a flip. A flip is the case that
/// motivated it, but "the sprite is where its anchor says" is the invariant,
/// and an invariant that is only restored at one moment is one a later change
/// can quietly break between moments.
pub fn anchor_world_sprites(mut anchored: Query<(&GroundAnchor, &mut Transform)>) {
    let _perf = crate::overlays::perf::scope("anchor_world_sprites");
    for (anchor, mut transform) in &mut anchored {
        let world = anchor.world();
        if transform.translation.x != world.x || transform.translation.y != world.y {
            transform.translation.x = world.x;
            transform.translation.y = world.y;
        }
    }
}

/// Run condition: the world is being drawn in 2:1 dimetric.
pub fn drawing_iso(view: Res<ViewProjection>) -> bool {
    view.is_iso()
}

/// Run condition: the world is being drawn from directly above.
pub fn drawing_top_down(view: Res<ViewProjection>) -> bool {
    !view.is_iso()
}

/// The projection a settings flag names.
#[inline]
pub fn projection_for(isometric: bool) -> MapProjection {
    if isometric {
        MapProjection::Iso
    } else {
        MapProjection::TopDown
    }
}

/// Put the settings flag where the boot world will see it.
///
/// Runs before `Startup`, alongside the shell's own world install, so the very
/// first frame is drawn in the projection the player left the game in rather
/// than flipping one frame after the window opens.
pub fn install_boot_projection(mut commands: Commands, settings: Res<Settings>) {
    let wanted = projection_for(settings.display.isometric);
    rail_map::set_projection(wanted);
    commands.insert_resource(ViewProjection(wanted));
}

/// Keep the projection's height field on the map that is actually installed.
///
/// # This is a load-bearing invariant, and it was violated
///
/// The lift `tile_to_world` applies in isometric comes from a process-global
/// height field, so **whoever installs a `MapGrid` owes it an installed height
/// field, before anything asks where a tile is**. Three places install one: this
/// plugin at boot, the shell's New Map, and the shell's *load*.
///
/// The load did not, and it was the one that mattered. A load replaces the map
/// mid-`Update` (`shell::save::regenerate_map_from_save`) and inserts a restored
/// `TrackNetwork` in the same breath. `track::visuals` treats a freshly inserted
/// network as a wholesale rebuild and spawns a sprite per piece through
/// `tile_to_world` — reading, on that frame, the *previous* world's heights. The
/// terrain caught up on the next frame and the track never did, because a track
/// sprite is placed once and then left alone. Every piece of the loaded railway
/// stood at the wrong elevation, on a map that otherwise looked correct.
///
/// So the field follows the resource rather than being a side effect of the
/// terrain build. Running in `PreUpdate` puts it ahead of all of `Update` by the
/// schedule instead of by anyone remembering an `.after()`; the load reinstalls
/// it inline as well, because a mid-`Update` swap cannot wait for the next
/// frame's `PreUpdate` and the rebuild it triggers happens immediately.
pub fn follow_map_heights(map: Res<MapGrid>) {
    if map.is_changed() {
        rail_map::set_iso_heights(&map);
    }
}

/// The bound key cycles the *setting*, never the projection directly, so the
/// Display row and the hotkey can never disagree about which view is on.
pub fn toggle_projection_hotkey(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    mut settings: ResMut<Settings>,
) {
    if bindings.just_pressed(&keys, ControlAction::ToggleProjection) {
        settings.display.isometric = !settings.display.isometric;
    }
}

/// Everything a flip has to reach. Split out of the system so the parameter
/// list stays inside Bevy's tuple limits and reads as one job.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Renderers<'w, 's> {
    pub chunks: Query<'w, 's, (Entity, &'static TerrainChunk, &'static Sprite)>,
    pub iso: Query<'w, 's, Entity, With<IsoTerrain>>,
    pub track: Query<'w, 's, Entity, With<TrackSprite>>,
    /// Disjoint from the camera query below, which also writes `Transform`.
    /// The camera has no `Sprite` and is never adopted, so excluding it costs
    /// nothing and is what makes the two queries legal in one system.
    pub sorted:
        Query<'w, 's, (Entity, &'static mut Transform, &'static IsoLayer), Without<MapCamera>>,
}

/// Follow [`Settings`] into the live projection.
///
/// Idempotent by construction: it only does anything on a frame where the
/// setting and [`ViewProjection`] disagree, and it makes them agree.
#[allow(clippy::too_many_arguments)]
pub fn apply_projection_setting(
    mut commands: Commands,
    settings: Res<Settings>,
    map: Res<MapGrid>,
    mut view: ResMut<ViewProjection>,
    mut images: ResMut<Assets<Image>>,
    mut flat_state: ResMut<TerrainDirty>,
    mut iso_state: ResMut<IsoTerrainState>,
    mut pending: ResMut<PendingWorld>,
    mut renderers: Renderers,
    mut camera: Query<
        (
            &mut Transform,
            &mut bevy::camera::Projection,
            &mut CameraZoomIndex,
        ),
        With<MapCamera>,
    >,
) {
    let wanted = projection_for(settings.display.isometric);
    if view.0 == wanted {
        return;
    }
    let previous = view.0;
    let started = bevy::platform::time::Instant::now();

    // ── 1. What is the player looking at, in the projection they are in? ──
    //
    // Read before the flag moves, because `world_to_tile` answers for whichever
    // projection is live. A camera outside the map still gives a coordinate,
    // and putting that coordinate back is exactly as right as keeping it.
    // The offset is carried as a *ground-plane* displacement, not a tile. Both
    // projections are linear away from the lift, so a difference between two
    // positions crosses exactly — and keeping it is what stops the camera
    // snapping to the nearest tile centre and drifting up to half a tile every
    // flip, which would make two flips visibly not the identity.
    let focus = camera.single().ok().map(|(transform, _, _)| {
        let centre = transform.translation.truncate();
        let tile = rail_map::world_to_tile(centre.x, centre.y);
        let (ax, ay) = rail_map::tile_to_world(tile);
        let offset = rail_map::unproject_offset(centre.x - ax, centre.y - ay);
        (tile, offset)
    });

    // ── 2. Move the flag ──────────────────────────────────────────────────
    rail_map::set_projection(wanted);
    // The lift the iso branch reads belongs to the map on screen. Installing it
    // here rather than only in the terrain build means the first frame after a
    // flip already has it, including for anything that reads a tile position
    // before the terrain system runs.
    rail_map::set_iso_heights(&map);
    view.0 = wanted;

    // ── 3. Swap the terrain renderer ──────────────────────────────────────
    match wanted {
        MapProjection::Iso => despawn_flat_terrain(
            &mut commands,
            &mut images,
            &mut flat_state,
            &renderers.chunks,
        ),
        MapProjection::TopDown => {
            despawn_iso_terrain(&mut commands, &mut iso_state, &renderers.iso);
            // The sort band is isometric's alone. Every sprite it adopted has to
            // go back to the layer z its own spawner wrote, and forget it was
            // ever adopted, or the next flip would read a sorted z as a layer.
            for (entity, mut transform, layer) in &mut renderers.sorted {
                transform.translation.z = layer.0;
                commands.entity(entity).remove::<IsoLayer>();
            }
        }
    }

    // ── 4. Track art is baked per projection, so drop the sprites ─────────
    //
    // The bank behind them is keyed on the projection as well, so the second
    // flip re-uses cells rather than re-painting them. `apply_track_sprites`
    // rebuilds everything it finds missing on the rebuild frame.
    for entity in renderers.track.iter() {
        commands.entity(entity).despawn();
    }

    // ── 5. Say so through the shell's own rebuild seam ────────────────────
    //
    // Nothing reads this today; it is the channel a load already uses to
    // announce the same condition, and a flip has no business inventing a
    // second one for a hook to miss later.
    pending.mark_rebuilding();

    // ── 6. Put the camera back over the tile it was over ──────────────────
    if let Ok((mut transform, mut projection, mut zoom)) = camera.single_mut() {
        if let Some((tile, (gx, gy))) = focus {
            let (wx, wy) = rail_map::tile_to_world(tile);
            let (dx, dy) = rail_map::project_offset(gx, gy);
            transform.translation.x = (wx + dx).round();
            transform.translation.y = (wy + dy).round();
        }
        // A tile is twice as wide in isometric, so the two views do not want the
        // same opening rung. Move the zoom only if the player has not chosen
        // one: a deliberate 3× survives the flip, a default does not become a
        // different default.
        if zoom.0 == default_zoom_index_for(previous) {
            zoom.0 = default_zoom_index_for(wanted);
            if let bevy::camera::Projection::Orthographic(ortho) = projection.as_mut() {
                ortho.scale = ortho_scale_for_zoom(zoom_factor_at(zoom.0));
            }
        }
    }

    info!(
        "projection: {} -> {} in {:?}",
        previous.label(),
        wanted.label(),
        started.elapsed()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;
    use bevy::input::InputPlugin;
    use bevy::state::app::StatesPlugin;
    use rail_sim::ids::TileCoord;
    use rail_sim::track::try_place_track;
    use std::collections::BTreeMap;

    /// A headless app running the real map plugin over a real generated world.
    ///
    /// Everything the flip touches is registered: both terrain renderers, the
    /// depth sort, the camera, the Map View, the schematic. What is not here is
    /// a GPU — nothing below reads a texture, only which entities exist and
    /// where they are.
    fn flip_app(seed: u64) -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            InputPlugin,
            AssetPlugin::default(),
        ))
        .init_asset::<Image>()
        .init_resource::<crate::ui::UiBlocksWorld>()
        .init_resource::<crate::track::TrackToolState>()
        .init_resource::<rail_sim::TrackNetwork>()
        .init_resource::<rail_sim::StationRegistry>()
        .init_resource::<rail_sim::IndustryRegistry>()
        .init_resource::<rail_sim::LineRegistry>()
        .init_resource::<rail_sim::DemandSpawner>()
        .init_resource::<PendingWorld>()
        .insert_resource({
            // These tests assert *transitions*, so they pin their own starting
            // view instead of inheriting the shipping default — which is a
            // product decision (isometric, since the owner went all-in) and
            // has already flipped once without any transition changing.
            let mut settings = Settings::default();
            settings.display.isometric = false;
            settings
        })
        .add_message::<rail_sim::TrackEdit>()
        .add_message::<rail_sim::StationEdit>()
        .add_plugins(super::super::MapPlugin {
            width: 32,
            height: 32,
            seed,
        });
        app
    }

    /// Everything on screen that the flip is allowed to move, and nothing else:
    /// which entities exist, what they are, and where they stand.
    ///
    /// Sorted and counted rather than compared entity by entity, because a
    /// rebuild legitimately re-spawns with fresh ids — what must not change is
    /// the picture.
    fn presentation(app: &mut App) -> BTreeMap<&'static str, usize> {
        let mut census = BTreeMap::new();
        census.insert(
            "iso terrain",
            app.world_mut()
                .query_filtered::<Entity, With<IsoTerrain>>()
                .iter(app.world())
                .count(),
        );
        census.insert(
            "flat chunks",
            app.world_mut()
                .query_filtered::<Entity, With<TerrainChunk>>()
                .iter(app.world())
                .count(),
        );
        census.insert(
            "sort adoptions",
            app.world_mut()
                .query_filtered::<Entity, With<IsoLayer>>()
                .iter(app.world())
                .count(),
        );
        census.insert(
            "sprites",
            app.world_mut()
                .query_filtered::<Entity, With<Sprite>>()
                .iter(app.world())
                .count(),
        );
        census.insert(
            "cameras",
            app.world_mut()
                .query_filtered::<Entity, With<MapCamera>>()
                .iter(app.world())
                .count(),
        );
        census
    }

    fn camera_state(app: &mut App) -> (i32, i32, usize) {
        let (transform, zoom) = app
            .world_mut()
            .query_filtered::<(&Transform, &CameraZoomIndex), With<MapCamera>>()
            .single(app.world())
            .map(|(t, z)| (*t, *z))
            .expect("one map camera");
        (
            transform.translation.x as i32,
            transform.translation.y as i32,
            zoom.0,
        )
    }

    fn set_iso(app: &mut App, iso: bool) {
        app.world_mut().resource_mut::<Settings>().display.isometric = iso;
        // One frame to flip, one for the incoming renderer to settle.
        app.update();
        app.update();
    }

    #[test]
    fn the_setting_names_a_projection() {
        assert_eq!(projection_for(false), MapProjection::TopDown);
        assert_eq!(projection_for(true), MapProjection::Iso);
    }

    #[test]
    fn the_resource_mirrors_the_flag() {
        assert!(!ViewProjection(MapProjection::TopDown).is_iso());
        assert!(ViewProjection(MapProjection::Iso).is_iso());
        assert_eq!(ViewProjection::default().0, MapProjection::TopDown);
    }

    /// The camera re-aim is a round trip through whichever projection is live:
    /// resolve the tile in the old one, place it in the new one.
    #[test]
    fn the_focused_tile_survives_the_flip() {
        let _lock = crate::map::tests::PROJECTION_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let restore = rail_map::projection();

        let tile = TileCoord { x: 21, y: 13 };
        for (from, to) in [
            (MapProjection::TopDown, MapProjection::Iso),
            (MapProjection::Iso, MapProjection::TopDown),
        ] {
            rail_map::set_projection(from);
            rail_map::clear_iso_heights();
            let (cx, cy) = rail_map::tile_to_world(tile);
            let focused = rail_map::world_to_tile(cx, cy);
            assert_eq!(focused, tile, "the camera did not resolve its own centre");

            // Stand the camera a third of a tile off centre, as panning leaves it.
            let offset = rail_map::project_offset(9.0, -5.0);
            let (camx, camy) = (cx + offset.0, cy + offset.1);
            let ground = rail_map::unproject_offset(camx - cx, camy - cy);

            rail_map::set_projection(to);
            let (nx, ny) = rail_map::tile_to_world(focused);
            assert_eq!(
                rail_map::world_to_tile(nx.round(), ny.round()),
                tile,
                "the camera landed somewhere else after the flip"
            );
            // ... and the fraction of a tile it was standing off centre came
            // with it, rather than snapping to the middle.
            let back = rail_map::project_offset(ground.0, ground.1);
            let (rx, ry) = rail_map::unproject_offset(back.0, back.1);
            assert!(
                (rx - 9.0).abs() < 1e-3 && (ry + 5.0).abs() < 1e-3,
                "the sub-tile offset did not survive: {rx}, {ry}"
            );
        }

        rail_map::set_projection(restore);
    }

    /// Two flips are the identity. Not "looks about the same" — the same
    /// census of presentation entities, and the camera back on the same tile at
    /// the same rung.
    ///
    /// The failure this guards against is the cheap one: a flip that spawns the
    /// incoming renderer without despawning the outgoing one leaves both on
    /// screen, and a second flip leaves three sets. Counting sprites catches
    /// that whether the leak is terrain, track art or the sort's bookkeeping.
    #[test]
    fn two_flips_leave_the_presentation_exactly_as_it_was() {
        let _guard = crate::map::tests::ProjectionGuard::new(MapProjection::TopDown);
        let mut app = flip_app(4242);
        app.update();
        app.update();

        let before = presentation(&mut app);
        let camera_before = camera_state(&mut app);
        assert!(before["flat chunks"] > 0, "top-down drew no terrain");
        assert_eq!(before["iso terrain"], 0);
        assert_eq!(before["sort adoptions"], 0, "the sorter must not run here");

        set_iso(&mut app, true);
        let middle = presentation(&mut app);
        assert!(middle["iso terrain"] > 0, "isometric drew no terrain");
        assert_eq!(middle["flat chunks"], 0, "the flat renderer was left behind");

        set_iso(&mut app, false);
        assert_eq!(
            presentation(&mut app),
            before,
            "a flip and a flip back did not land where they started"
        );
        assert_eq!(camera_state(&mut app), camera_before);
        assert_eq!(
            rail_map::projection(),
            MapProjection::TopDown,
            "the global flag is out of step with the resource"
        );

        // And again, from the other side: two flips out of isometric are also
        // the identity, which is the case where the sorter has adopted sprites
        // and has to give them back.
        set_iso(&mut app, true);
        let iso_before = presentation(&mut app);
        let iso_camera = camera_state(&mut app);
        set_iso(&mut app, false);
        set_iso(&mut app, true);
        assert_eq!(presentation(&mut app), iso_before);
        assert_eq!(camera_state(&mut app), iso_camera);
    }

    /// Nothing that draws survives into the other view.
    ///
    /// Terrain is the obvious one, but the sort's `IsoLayer` is the subtle one:
    /// it remembers a sprite's layer z, and a sprite still carrying one on the
    /// way back into isometric would have its *sorted* z read as its layer and
    /// walk off into the band a little further every flip.
    #[test]
    fn the_sort_gives_back_every_sprite_it_adopted() {
        let _guard = crate::map::tests::ProjectionGuard::new(MapProjection::TopDown);
        let mut app = flip_app(99);
        app.update();

        // A world sprite of the kind a gameplay system spawns, at a known layer.
        let (wx, wy) = rail_map::tile_to_world(TileCoord { x: 6, y: 6 });
        let sprite = app
            .world_mut()
            .spawn((Sprite::default(), Transform::from_xyz(wx, wy, 3.0)))
            .id();
        app.update();

        set_iso(&mut app, true);
        let sorted = app.world().entity(sprite).get::<Transform>().unwrap();
        assert!(
            sorted.translation.z > super::super::iso_depth::BAND_FLOOR,
            "isometric did not sort the sprite at all"
        );
        assert!(app.world().entity(sprite).get::<IsoLayer>().is_some());

        set_iso(&mut app, false);
        let restored = app.world().entity(sprite).get::<Transform>().unwrap();
        assert_eq!(
            restored.translation.z, 3.0,
            "the sprite kept a sorted z after the sorter stopped running"
        );
        assert!(
            app.world().entity(sprite).get::<IsoLayer>().is_none(),
            "the sorter kept its memory of a sprite it no longer owns"
        );

        // A second round trip must land on the same z, not drift.
        set_iso(&mut app, true);
        set_iso(&mut app, false);
        assert_eq!(
            app.world()
                .entity(sprite)
                .get::<Transform>()
                .unwrap()
                .translation
                .z,
            3.0
        );
    }

    /// Picking round-trips in both projections, over real generated terrain —
    /// with the one exception isometric genuinely has, stated as a rule rather
    /// than tolerated as slop.
    ///
    /// From above, every tile centre answers with its own tile and that is the
    /// whole story. In isometric a cliff standing one row nearer the camera
    /// really does hide the ground behind it, so a hidden tile's centre answers
    /// with the tile the player can actually see there. What must hold in both
    /// is the property the cursor depends on: **whatever tile comes back, its
    /// own centre comes back to it** — point at a tile and the ghost that draws
    /// is under the pointer.
    ///
    /// `rail_map` proves the maths on synthetic terrain; this proves it against
    /// the map the game installs, with the height field the terrain build put
    /// there.
    #[test]
    fn picking_round_trips_over_a_real_world_in_both_projections() {
        let _guard = crate::map::tests::ProjectionGuard::new(MapProjection::TopDown);
        let mut app = flip_app(31_337);
        app.update();
        app.update();

        for iso in [false, true, false] {
            set_iso(&mut app, iso);
            let map = app.world().resource::<MapGrid>().clone();
            let (mut checked, mut occluded) = (0, 0);
            for y in 0..map.height as i32 {
                for x in 0..map.width as i32 {
                    let tile = TileCoord { x, y };
                    let (wx, wy) = rail_map::tile_to_world(tile);
                    let picked = rail_map::world_to_tile(wx, wy);
                    checked += 1;

                    // The answer is always self-consistent.
                    let (px, py) = rail_map::tile_to_world(picked);
                    assert_eq!(
                        rail_map::world_to_tile(px, py),
                        picked,
                        "picking is not stable on its own answer at {tile:?}"
                    );

                    if picked == tile {
                        continue;
                    }
                    occluded += 1;
                    assert!(iso, "top-down picking must be exact, {tile:?} was not");
                    assert!(
                        picked.x + picked.y < tile.x + tile.y,
                        "{picked:?} is not nearer the camera than {tile:?}"
                    );
                    assert!(
                        rail_map::tile_height(picked) > rail_map::tile_height(tile),
                        "{picked:?} is not standing above {tile:?}, so it cannot hide it"
                    );
                }
            }
            assert_eq!(checked, 32 * 32);
            if iso {
                // A real map, so some ground is genuinely behind a cliff — but
                // if this were most of the map the view would be unusable, and
                // that is worth a number rather than a shrug.
                assert!(
                    occluded * 20 < checked,
                    "{occluded} of {checked} tile centres are hidden behind cliffs"
                );
            } else {
                assert_eq!(occluded, 0);
            }
        }
    }

    /// The projection is invisible to the simulation.
    ///
    /// Determinism is the repo's load-bearing property, so this is not "the sim
    /// looks unaffected" — it is the same world, ticked the same number of
    /// times, hashed, with flips happening throughout one run and never in the
    /// other. Any command the flip pushed, any sim entity it despawned, any
    /// resource it touched would show up here as a different number.
    #[test]
    fn a_flip_is_invisible_to_the_simulation() {
        fn run(flipping: bool) -> u64 {
            let _guard = crate::map::tests::ProjectionGuard::new(MapProjection::TopDown);
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                // The sim ticks in `FixedUpdate`, which reads the clock — so
                // without this each `update()` would run a wall-clock-dependent
                // number of ticks and the two runs would differ for a reason
                // that has nothing to do with the projection.
                .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                    std::time::Duration::from_secs_f64(1.0 / 60.0),
                ))
                .insert_resource(rail_map::generate_map(32, 32, 5150))
                .add_plugins(rail_sim::SimPlugin);
            let map = app.world().resource::<MapGrid>().clone();
            app.insert_resource(track_terrain_of(&map));
            app.insert_resource(rail_sim::AnchorSites(map.anchor_hints()));
            app.update();

            for tick in 0..120 {
                if flipping && tick % 17 == 0 {
                    // Everything the flip does to the *world*: move the flag and
                    // re-install the lift. The presentation half cannot reach
                    // the sim by construction — it despawns sprites and reads
                    // coordinates — so what is left to prove is that these two
                    // are inert, and they are the two that a sim system could
                    // conceivably observe.
                    rail_map::set_projection(rail_map::projection().flipped());
                    rail_map::set_iso_heights(&map);
                }
                app.update();
            }
            sim_fingerprint(app.world())
        }

        let quiet = run(false);
        let flipped = run(true);
        assert_eq!(
            quiet, flipped,
            "flipping the projection changed the simulation"
        );
        assert_ne!(quiet, 0, "the fingerprint read nothing at all");
    }

    /// The terrain the sim builds on, from the map the presentation draws.
    fn track_terrain_of(map: &MapGrid) -> rail_sim::TrackTerrain {
        let mut cells = Vec::with_capacity((map.width * map.height) as usize);
        for y in 0..map.height {
            for x in 0..map.width {
                let tile = map.tile(TileCoord {
                    x: x as i32,
                    y: y as i32,
                });
                cells.push((tile.water, tile.height));
            }
        }
        rail_sim::TrackTerrain::new(map.width, map.height, cells)
    }

    // ── Everything that stands on the ground ──────────────────────────────

    /// Every anchored sprite, by the ground it is anchored to.
    fn anchored(app: &mut App) -> BTreeMap<(i32, i32), (f32, f32)> {
        app.world_mut()
            .query::<(&GroundAnchor, &Transform)>()
            .iter(app.world())
            .map(|(anchor, tf)| {
                (
                    (anchor.0.x as i32, anchor.0.y as i32),
                    (tf.translation.x, tf.translation.y),
                )
            })
            .collect()
    }

    /// Assert every anchored sprite is standing on its own ground right now.
    fn assert_all_anchored(app: &mut App, whose: &str) -> usize {
        let mut checked = 0;
        for (ground, drawn) in anchored(app) {
            let (wx, wy) =
                rail_map::ground_to_world(ground.0 as f32, ground.1 as f32);
            assert_eq!(
                drawn,
                (wx, wy),
                "{whose}: the sprite anchored at {ground:?} is drawn at {drawn:?}, \
                 but that ground is at ({wx}, {wy}) in {}",
                rail_map::projection().label()
            );
            checked += 1;
        }
        checked
    }

    /// Houses, farmsteads and rural props are placed once and then left alone,
    /// which is what made them the bug: `lot_base` lays a block out in ground
    /// texels and the spawner wrote those straight into a `Transform`. Correct
    /// from above, and up-and-right of the diamond in isometric — out over the
    /// river, some off the map.
    ///
    /// The real spawners are used here, not a stand-in: `seed_rural` plants the
    /// countryside at boot and this checks what it actually produced.
    #[test]
    fn everything_standing_on_the_ground_moves_with_the_ground() {
        let _guard = crate::map::tests::ProjectionGuard::new(MapProjection::TopDown);
        let mut app = game_app(7_707);
        // Pin the starting view: the shipping default is isometric now, and a
        // transition test that inherits it would measure a no-op flip.
        set_iso(&mut app, false);
        settle(&mut app);

        let flat = anchored(&mut app);
        assert!(
            flat.len() > 20,
            "the countryside planted almost nothing to check: {}",
            flat.len()
        );
        assert_all_anchored(&mut app, "top-down");

        // Into isometric: everything has to move, and land on its own ground.
        set_iso(&mut app, true);
        let iso = anchored(&mut app);
        assert_eq!(iso.len(), flat.len(), "the flip lost or duplicated sprites");
        assert_all_anchored(&mut app, "isometric");
        let moved = iso.iter().filter(|(g, p)| flat.get(*g) != Some(*p)).count();
        assert!(
            moved * 2 > iso.len(),
            "only {moved} of {} anchored sprites moved; the projection is not \
             reaching them",
            iso.len()
        );

        // ... and back is exactly where they started.
        set_iso(&mut app, false);
        assert_eq!(anchored(&mut app), flat, "a round trip moved the town");
    }

    /// Nothing in the world is standing anywhere the world does not reach.
    ///
    /// The two tests above ask whether the things wearing a [`GroundAnchor`] are
    /// in the right place, which is only half a question: a spawner that never
    /// attached one is invisible to them. This asks the other half, of every
    /// world sprite there is, by the one property a misprojected sprite cannot
    /// fake — a position that resolves back to a tile on the map.
    ///
    /// It is exactly the sweep that would have caught the shipped bug. A house
    /// at top-down `(1400, 1400)` drawn into an isometric world unprojects to
    /// ground `(2100, 700)`, which is tile `(65, 21)` on a 48-tile map: off the
    /// east edge by seventeen tiles, which is what "some of them cleared the
    /// map" looked like.
    ///
    /// Deliberately a *class* test with no list of types in it. Anything that
    /// grows a new world sprite is covered the day it is written.
    #[test]
    fn no_world_sprite_stands_off_the_map() {
        let _guard = crate::map::tests::ProjectionGuard::new(MapProjection::Iso);
        let mut app = game_app(5_150);
        app.world_mut()
            .resource_mut::<Settings>()
            .display
            .isometric = true;
        settle(&mut app);

        let map = app.world().resource::<MapGrid>().clone();
        // Generous: a sprite may legitimately hang a little past the edge (a
        // roof, a cliff face, the plinth). Seventeen tiles out is not that.
        let margin = 4;
        let mut checked = 0;
        let mut strays = Vec::new();
        // `IsoLayer` is exactly "a root world sprite the depth sorter adopted",
        // which is the population this is about: the day tint and the Map View
        // plate live above the band and are never adopted, and terrain sorts
        // itself and is on the map by construction.
        let mut query = app
            .world_mut()
            .query_filtered::<&Transform, (With<Sprite>, With<IsoLayer>, Without<ChildOf>)>();
        for transform in query.iter(app.world()) {
            let at = transform.translation;
            checked += 1;
            let tile = rail_map::world_to_tile(at.x, at.y);
            let inside = tile.x >= -margin
                && tile.y >= -margin
                && tile.x < map.width as i32 + margin
                && tile.y < map.height as i32 + margin;
            if !inside {
                strays.push((at.x, at.y, tile));
            }
        }
        assert!(checked > 100, "the sweep saw almost nothing: {checked}");
        assert!(
            strays.is_empty(),
            "{} of {checked} world sprites are drawn off a {}x{} map: {:?}",
            strays.len(),
            map.width,
            map.height,
            &strays[..strays.len().min(5)]
        );
    }

    /// The other half: a thing that appears *while* isometric is on has to be
    /// right when it appears, not one flip later. A spawner that writes ground
    /// texels into a transform passes the flip test and fails this one.
    #[test]
    fn something_that_spawns_in_isometric_is_placed_in_isometric() {
        let _guard = crate::map::tests::ProjectionGuard::new(MapProjection::Iso);
        let mut app = game_app(31_415);
        app.world_mut()
            .resource_mut::<Settings>()
            .display
            .isometric = true;
        settle(&mut app);

        let born_here = anchored(&mut app);
        assert!(born_here.len() > 20, "nothing was planted to check");
        assert_all_anchored(&mut app, "spawned in isometric");

        // The same world booted from above and then flipped has to agree, or
        // "spawned under this projection" and "moved into it" are two different
        // answers and one of them is wrong.
        drop(app);
        rail_map::set_projection(MapProjection::TopDown);
        let mut flipped = game_app(31_415);
        settle(&mut flipped);
        set_iso(&mut flipped, true);
        assert_eq!(
            anchored(&mut flipped),
            born_here,
            "a town spawned in isometric and a town flipped into it disagree"
        );
    }

    /// The ground itself can move — a load brings a different world, and the
    /// elevation under a tile changes with it. An anchored sprite follows,
    /// whatever caused the change and whatever order the systems ran in.
    ///
    /// This is what makes the load path safe rather than lucky.
    /// `shell::save::regenerate_map_from_save` installs the new heights inline
    /// so the first frame is already right, but the ordering between the load
    /// and the systems that read a tile position is not constrained, and this
    /// is the net under that.
    #[test]
    fn an_anchored_sprite_follows_a_change_of_world() {
        let _guard = crate::map::tests::ProjectionGuard::new(MapProjection::Iso);
        let mut app = flip_app(2_024);
        app.world_mut()
            .resource_mut::<Settings>()
            .display
            .isometric = true;
        app.update();
        app.update();

        let tile = TileCoord { x: 9, y: 9 };
        let (gx, gy) = rail_map::tile_to_ground(tile);
        let anchor = GroundAnchor::new(gx, gy);
        let entity = app
            .world_mut()
            .spawn((Sprite::default(), anchor, anchor.transform(2.0)))
            .id();
        app.update();
        let before = app.world().entity(entity).get::<Transform>().unwrap().translation;

        // A different world, raised sharply under that very tile.
        let mut swapped = app.world().resource::<MapGrid>().clone();
        for t in swapped.tiles_mut() {
            t.height = 0;
            t.water = false;
        }
        swapped.get_mut(tile).unwrap().height = 15;
        app.insert_resource(swapped);
        app.update();
        app.update();

        let after = app.world().entity(entity).get::<Transform>().unwrap().translation;
        assert_ne!(
            before.y, after.y,
            "the ground under the sprite rose 15 bands and the sprite did not"
        );
        let (wx, wy) = rail_map::ground_to_world(gx, gy);
        assert_eq!((after.x, after.y), (wx, wy));
        assert_eq!(
            after.y - rail_map::project(gx, gy).1,
            15.0 * rail_map::ISO_LIFT,
            "the sprite is not standing on the new summit"
        );
    }

    // ── Saving, loading, and the world the sprites were built for ─────────

    /// The whole game, headless: sim, shell (so saves and loads run through the
    /// real path), map and track.
    fn game_app(seed: u64) -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            InputPlugin,
            AssetPlugin::default(),
        ))
        .init_asset::<Image>()
        .init_asset::<bevy::image::TextureAtlasLayout>()
        .init_resource::<UiScale>()
        .init_resource::<crate::ui::UiBlocksWorld>()
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ))
        .add_plugins(rail_sim::SimPlugin)
        .add_plugins(crate::shell::ShellPlugin {
            boot_seed: crate::shell::BootSeed::Fixed(seed),
            suppress_world_input: false,
        })
        .add_plugins(super::super::MapPlugin {
            width: 48,
            height: 48,
            seed,
        })
        .add_plugins(crate::track::TrackPlugin)
        // The town is where the anchored sprites come from: `seed_rural` plants
        // the countryside, and the lot phase machine grows houses on it.
        .add_plugins(crate::town::TownPresentationPlugin);
        app.insert_resource(Settings::default());
        app
    }

    /// Run until the town has planted itself.
    ///
    /// `sync_building_sprites` spends its first frame baking the atlas and then
    /// waits three more before the one-shot rural seed fires, so a test that
    /// updates twice sees an empty countryside and proves nothing.
    fn settle(app: &mut App) {
        for _ in 0..8 {
            app.update();
        }
    }

    /// Lay a railway with something of everything on it: a straight run, two
    /// turns, a junction leg, and — where the map offers one — a water crossing
    /// wider than the cheap tier.
    fn lay_a_railway(app: &mut App) -> Vec<TileCoord> {
        let map = app.world().resource::<MapGrid>().clone();
        let terrain = track_terrain_of(&map);
        let mut network = app.world().resource::<rail_sim::TrackNetwork>().clone();
        let mut money = rail_sim::Money::new(50_000_000);
        let mut ledger = rail_sim::MoneyLedger::default();
        let mut laid = Vec::new();

        // Sweep the map for a run that includes a bridge above the cheap span,
        // so the piece kinds under test are the ones the ladder actually has.
        let mut best: Option<Vec<TileCoord>> = None;
        for y in 2..(map.height as i32 - 2) {
            let row: Vec<TileCoord> = (2..(map.width as i32 - 2))
                .map(|x| TileCoord { x, y })
                .collect();
            let spans = row
                .iter()
                .filter(|t| terrain.is_water(**t))
                .count();
            if spans > rail_sim::CHEAP_BRIDGE_SPAN as usize {
                best = Some(row);
                break;
            }
        }
        let run = best.unwrap_or_else(|| {
            (2..20).map(|x| TileCoord { x, y: 8 }).collect()
        });

        for tile in run {
            if try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                tile,
                rail_sim::GROUND_LAYER,
            )
            .is_ok()
            {
                laid.push(tile);
            }
        }
        // A junction and two turns hanging off whatever went down.
        if let Some(mid) = laid.get(laid.len() / 2).copied() {
            for (dx, dy) in [(0, 1), (1, 1), (1, 2), (0, -1), (-1, -2)] {
                let tile = TileCoord {
                    x: mid.x + dx,
                    y: mid.y + dy,
                };
                if try_place_track(
                    &mut network,
                    &mut money,
                    &mut ledger,
                    &terrain,
                    tile,
                    rail_sim::GROUND_LAYER,
                )
                .is_ok()
                {
                    laid.push(tile);
                }
            }
        }
        assert!(laid.len() > 8, "the test laid almost no track: {}", laid.len());

        // A station platform on the line, so a stop is in the save too.
        let mut stations = app.world().resource::<rail_sim::StationRegistry>().clone();
        stations.insert("Testfield", laid[0], rail_sim::GROUND_LAYER);
        app.insert_resource(stations);
        app.insert_resource(network);
        app.insert_resource(money);
        app.update();
        laid
    }

    /// Every track sprite, by the tile its piece stands on.
    fn track_positions(app: &mut App) -> BTreeMap<(i32, i32), (f32, f32)> {
        let network = app.world().resource::<rail_sim::TrackNetwork>().clone();
        let mut by_id = BTreeMap::new();
        for piece in network.iter() {
            by_id.insert(piece.id.0, piece.tile);
        }
        app.world_mut()
            .query::<(&TrackSprite, &Transform)>()
            .iter(app.world())
            .filter_map(|(sprite, tf)| {
                by_id.get(&sprite.id.0).map(|tile| {
                    (
                        (tile.x, tile.y),
                        (tf.translation.x, tf.translation.y),
                    )
                })
            })
            .collect()
    }

    /// Save in one session, load in a fresh one, and check the railway is drawn
    /// on the world it was saved on.
    ///
    /// # The bug
    ///
    /// A load replaces the `MapGrid` mid-`Update` and inserts the restored
    /// `TrackNetwork` in the same breath. `track::visuals` reads a freshly
    /// inserted network as a wholesale rebuild and spawns one sprite per piece
    /// through `tile_to_world` — which, in isometric, adds the elevation lift
    /// from a process-global height field. Nothing reinstalled that field for
    /// the loaded map, so every piece was placed at the *previous* world's
    /// elevation. The terrain caught up on the next frame; the track never did,
    /// because a track sprite is positioned once and then left alone.
    ///
    /// The two worlds here are deliberately different maps, so a stale height
    /// field cannot accidentally agree with a fresh one.
    fn save_then_load_in(save_view: MapProjection, load_view: MapProjection) {
        let _guard = crate::map::tests::ProjectionGuard::new(save_view);
        // The save root is a process global; hold it still for the round trip.
        let _root = crate::shell::lock_save_root("iso_load");
        let slot =
            rail_sim::save::SaveSlot::named(&format!("iso load {:?} {:?}", save_view, load_view))
                .expect("valid slot name");
        let _ = rail_sim::save::delete_slot(&slot);

        // ── Session one: build a railway and save it ──────────────────────
        let mut app = game_app(4_242);
        app.world_mut()
            .resource_mut::<Settings>()
            .display
            .isometric = save_view == MapProjection::Iso;
        app.update();
        app.update();
        let laid = lay_a_railway(&mut app);
        let saved_map = app.world().resource::<MapGrid>().clone();
        rail_sim::save::save_to_slot(app.world(), &slot).expect("save");
        let saved_network: Vec<_> = app
            .world()
            .resource::<rail_sim::TrackNetwork>()
            .iter()
            .map(|p| (p.id.0, p.tile, p.links.0))
            .collect();
        drop(app);

        // ── Session two: a different world, then load ─────────────────────
        rail_map::set_projection(load_view);
        let mut app = game_app(90_210);
        app.world_mut()
            .resource_mut::<Settings>()
            .display
            .isometric = load_view == MapProjection::Iso;
        app.update();
        app.update();
        let other = app.world().resource::<MapGrid>().clone();
        assert_ne!(
            other.tiles().to_vec(),
            saved_map.tiles().to_vec(),
            "the two sessions have to be different worlds or the test proves nothing"
        );
        // ... and different *under the railway*, which is what a stale height
        // field would be read from.
        let differs = laid
            .iter()
            .filter(|t| {
                other.get(**t).map(|c| rail_map::surface_height_of(c))
                    != saved_map.get(**t).map(|c| rail_map::surface_height_of(c))
            })
            .count();
        assert!(
            differs > 0,
            "the two worlds stand at the same height under every rail; a stale \
             lift would be invisible and this test would pass for free"
        );

        app.world_mut()
            .resource_mut::<crate::shell::ShellSaveRequest>()
            .load = Some(slot.clone());
        app.update();
        app.update();

        // A load that quietly failed would leave the previous world in place
        // and every assertion below would be measuring the wrong thing.
        let status = app
            .world()
            .resource::<crate::shell::SaveStatus>()
            .message
            .clone()
            .unwrap_or_default();
        assert!(
            status.starts_with("Loaded"),
            "the load did not happen: {status:?}"
        );

        // (a) The sim got its network back.
        let loaded: Vec<_> = app
            .world()
            .resource::<rail_sim::TrackNetwork>()
            .iter()
            .map(|p| (p.id.0, p.tile, p.links.0))
            .collect();
        let sorted = |mut v: Vec<(u64, TileCoord, u16)>| {
            v.sort_by_key(|(id, _, _)| *id);
            v
        };
        assert_eq!(
            sorted(loaded.clone()),
            sorted(saved_network),
            "the save round-trip lost track pieces"
        );

        // (b) The height field belongs to the world that was loaded.
        let installed = app.world().resource::<MapGrid>().clone();
        assert_eq!(
            installed.tiles().to_vec(),
            saved_map.tiles().to_vec(),
            "the load did not bring back the saved world"
        );
        for tile in &laid {
            assert_eq!(
                rail_map::tile_height(*tile),
                installed.get(*tile).map(rail_map::surface_height_of).unwrap_or(0),
                "the projection's lift at {tile:?} belongs to some other map"
            );
        }

        // (c) Every piece is drawn where the loaded world says its tile is.
        let drawn = track_positions(&mut app);
        assert_eq!(
            drawn.len(),
            loaded.len(),
            "the loaded railway is missing sprites"
        );
        for (&(x, y), &(sx, sy)) in &drawn {
            let tile = TileCoord { x, y };
            let (wx, wy) = rail_map::tile_to_world(tile);
            assert_eq!(
                (sx, sy),
                (wx, wy),
                "the rail at {tile:?} is drawn at ({sx}, {sy}) but its tile is \
                 at ({wx}, {wy}) in {}",
                rail_map::projection().label()
            );
        }

        let _ = rail_sim::save::delete_slot(&slot);
    }

    #[test]
    fn a_loaded_railway_is_drawn_on_the_world_it_was_saved_on_top_down() {
        save_then_load_in(MapProjection::TopDown, MapProjection::TopDown);
    }

    #[test]
    fn a_loaded_railway_is_drawn_on_the_world_it_was_saved_on_iso() {
        save_then_load_in(MapProjection::Iso, MapProjection::Iso);
    }

    /// The view is not part of the world, so a save made from above has to load
    /// into isometric and back again with the railway on the ground either way.
    #[test]
    fn a_save_made_in_one_view_loads_into_the_other() {
        save_then_load_in(MapProjection::TopDown, MapProjection::Iso);
        save_then_load_in(MapProjection::Iso, MapProjection::TopDown);
    }

    /// FNV-1a over the sim state a flip could plausibly disturb.
    fn sim_fingerprint(world: &World) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |value: u64| {
            for byte in value.to_le_bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };

        eat(world.resource::<rail_sim::Money>().cents() as u64);
        let clock = world.resource::<rail_sim::SimClock>();
        eat(clock.paused as u64);
        eat(clock.speed_multiplier as u64);
        let network = world.resource::<rail_sim::TrackNetwork>();
        eat(network.len() as u64);
        let mut pieces: Vec<_> = network.iter().collect();
        pieces.sort_by_key(|p| p.id.0);
        for piece in pieces {
            eat(piece.id.0);
            eat(piece.tile.x as u32 as u64);
            eat(piece.tile.y as u32 as u64);
            eat(piece.links.0 as u64);
        }
        let stations = world.resource::<rail_sim::StationRegistry>();
        let mut stops: Vec<_> = stations.iter().collect();
        stops.sort_by_key(|s| s.id.0);
        for station in stops {
            eat(station.id.0);
            eat(station.tile.x as u32 as u64);
            eat(station.tile.y as u32 as u64);
            eat(station.tier as u64);
        }
        let industries = world.resource::<rail_sim::IndustryRegistry>();
        let mut works: Vec<_> = industries.iter().collect();
        works.sort_by_key(|i| i.id.0);
        for industry in works {
            eat(industry.id.0);
            eat(industry.tile.x as u32 as u64);
            eat(industry.tile.y as u32 as u64);
        }
        eat(world.resource::<rail_sim::TownDensity>().is_empty() as u64);
        hash
    }
}
