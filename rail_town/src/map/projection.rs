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

/// Install the boot world's heights before anything draws it.
///
/// `MapPlugin::build` installs the heights of the grid *it* generated, and the
/// shell then replaces that grid during `PreStartup`. Nothing in `Startup` reads
/// a lifted position today, but "the height field belongs to the map on screen"
/// is the invariant, and the cheapest place to keep it true is here.
pub fn install_map_heights(map: Res<MapGrid>) {
    rail_map::set_iso_heights(&map);
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
        .insert_resource(Settings::default())
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
