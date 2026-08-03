//! Atmosphere — time of day, lit windows, and ambient motion.
//!
//! The thesis from [`docs/design/01-art-direction.md`](../../../docs/design/01-art-direction.md)
//! §6.3 is that **calm is not still**: a world that holds perfectly still while
//! the camera holds still reads as broken, not as quiet. This module is the
//! cheap half of the fix — one tint pass, one window layer, and a handful of
//! two-to-four frame loops.
//!
//! Four rules shape everything here:
//!
//! 1. **The cycle is the sim's, not the frame's.** The day advances on
//!    `Time<Virtual>` gated by [`SimClock`], so it honours pause and speed
//!    (brief §3.4: twelve minutes at 1×).
//! 2. **Night stays legible.** The tint is floored at
//!    [`time_of_day::MIN_LEGIBILITY`] in code, not in a comment.
//! 3. **Noise is world-anchored.** Every phase, offset and occupancy roll
//!    hashes on integer tile coordinates (brief §2.4) — never on screen
//!    position, never on time. Screen-anchored noise boils under scroll and
//!    that is the one thing the pixel contract will not forgive.
//! 4. **Art bakes when data changes.** Window and chimney placement rebuilds
//!    only when a tile's *quantized* density level moves (brief §2.5); the
//!    per-frame systems do a comparison and usually nothing else.
//!
//! Every sprite this module spawns sits at whole world coordinates with an
//! even texel size, so a centred sprite lands on texel boundaries at every
//! zoom. Nothing rotates.
//!
//! Trees are deliberately absent: everything swaying at once is noise, not
//! calm (brief §6.3).

mod bake;
mod smoke;
mod time_of_day;
mod water;
mod windows;

use bevy::prelude::*;
use rail_sim::SimClock;

use bake::{track_density_levels, DensityLevels};
use smoke::{bake_chimney_smoke, step_chimney_smoke, SmokeLayer};
use time_of_day::{advance_time_of_day, spawn_day_tint, sync_day_tint};
use water::{
    bake_water_decals, rebuild_water_decals, step_coast_foam, step_water_shimmer, WaterDecals,
};
use windows::{step_window_light, sync_lit_windows, WindowLayer};

// The public read model. `DayPhase` / `DAY_CYCLE_SECS` are for the systems that
// will read the clock next (status strip, lamps, weather) and are exported now
// so nobody re-derives the cycle from their own timer.
#[allow(unused_imports)]
pub use time_of_day::{DayPhase, TimeOfDay, DAY_CYCLE_SECS};

/// Draw order for the layers this module owns.
///
/// The rest of presentation places terrain at `0.0`, buildings at `0.5`, track
/// at `1.0`, peeps / stations at `2.0`, trains at `3.0`, build feedback at
/// `3.5` and overlays at `4.5`. Water decals slot just above terrain, smoke
/// above the peeps band, and the time-of-day tint caps the world (brief §6.1
/// band order) while staying far below the camera at `1000.0`.
///
/// Lit windows are the exception and have **no constant here**: buildings are
/// Y-sorted across a band, so each light is drawn at its own lot's `z` plus a
/// small lift. A fixed layer would put lights behind the houses south of them.
pub(crate) const WATER_DECAL_Z: f32 = 0.15;
pub(crate) const COAST_FOAM_Z: f32 = 0.2;
pub(crate) const CHIMNEY_SMOKE_Z: f32 = 2.75;
pub(crate) const DAY_TINT_Z: f32 = 64.0;

/// Wall-clock seconds driving every ambient loop.
///
/// Deliberately **not** scaled by sim speed: a 1.2 s shimmer is an authored
/// wall-clock value (brief §1, "slow"), and water running at 3× reads as
/// agitation rather than calm. It does freeze with the sim, so a paused world
/// reads as held rather than as alive-but-ignoring-you.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct AmbientClock {
    pub secs: f32,
}

/// Wrap point for [`AmbientClock`]. The 1.2 s, 2.4 s and 3.0 s loops all divide
/// it exactly, so the wrap never lands mid-frame and no loop jumps.
const AMBIENT_WRAP: f32 = 1440.0;

/// Time of day, lit windows and ambient motion.
///
/// Reads [`SimClock`], [`rail_sim::TownDensity`] and [`rail_map::MapGrid`];
/// writes only its own entities and resources.
pub struct AtmospherePlugin;

impl Plugin for AtmospherePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TimeOfDay>()
            .init_resource::<AmbientClock>()
            .init_resource::<DensityLevels>()
            .init_resource::<WindowLayer>()
            .init_resource::<SmokeLayer>()
            .init_resource::<WaterDecals>()
            .add_systems(Startup, (spawn_day_tint, bake_water_decals))
            .add_systems(
                Update,
                (
                    advance_time_of_day,
                    advance_ambient_clock,
                    // The sea belongs to the world it was baked from. A new map
                    // or a load replaces `MapGrid`, and the old world's glints
                    // and foam would otherwise stay painted over the new one —
                    // on dry land, and off the edge of a smaller map.
                    rebuild_water_decals,
                    sync_day_tint.after(advance_time_of_day),
                    step_window_light.after(advance_time_of_day),
                    // Bakes run after the fade has been applied for this frame,
                    // so a house that goes up mid-dusk lights at exactly the
                    // step its neighbours are already on.
                    (track_density_levels, sync_lit_windows, bake_chimney_smoke)
                        .chain()
                        .after(step_window_light),
                    step_water_shimmer.after(advance_ambient_clock),
                    step_coast_foam.after(advance_ambient_clock),
                    step_chimney_smoke.after(advance_ambient_clock),
                ),
            );
    }
}

/// Advance ambient motion with unscaled real time while the sim runs.
fn advance_ambient_clock(
    clock: Res<SimClock>,
    time: Res<Time<Real>>,
    mut ambient: ResMut<AmbientClock>,
) {
    if !clock.is_running() {
        return;
    }
    ambient.secs = (ambient.secs + time.delta_secs()) % AMBIENT_WRAP;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atmosphere::smoke::{ChimneySmoke, CHIMNEY_SMOKE_PERIOD};
    use crate::atmosphere::water::{CoastFoam, WaterShimmer, COAST_FOAM_PERIOD, WATER_SHIMMER_PERIOD};
    use crate::atmosphere::windows::LitWindow;
    use crate::town::BuildingWindows;
    use rail_map::generate_map;
    use rail_sim::{IndustryRegistry, StationRegistry, TileCoord, TownDensity, TrackNetwork};

    /// A headless world with a small map, a dense district and a thin one.
    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        // Carve a lake rather than trusting the generator to supply one. Maps
        // are deliberately land-dominated now — many have no open water at all
        // — so a generated test map is not a reliable source of water tiles.
        let mut map = generate_map(24, 24, 42);
        for y in 16..22 {
            for x in 2..8 {
                if let Some(tile) = map.get_mut(TileCoord { x, y }) {
                    tile.water = true;
                    tile.kind = rail_map::TerrainKind::Water;
                    tile.height = -3;
                }
            }
        }
        app.insert_resource(map);
        app.insert_resource(SimClock::default());

        let mut density = TownDensity::default();
        for y in 4..10 {
            for x in 4..10 {
                density.set(TileCoord { x, y }, 0.9);
            }
        }
        for x in 12..16 {
            density.set(TileCoord { x, y: 12 }, 0.12);
        }
        app.insert_resource(density);

        // Lit windows are drawn from the town slice's baked window masks, so
        // the buildings have to actually exist for this layer to have anything
        // to light. Running both here is deliberate: it exercises the
        // density → buildings → windows chain rather than a stand-in for it.
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_asset::<Image>();
        app.init_asset::<bevy::image::TextureAtlasLayout>();

        // District character needs the anchors it classifies against. Without a
        // station every block would read as rural — capped at one lot and tier
        // zero — and a rural hamlet is not what "a district lights up" means.
        let mut stations = StationRegistry::new();
        stations.insert("Eastgate", TileCoord { x: 7, y: 7 }, rail_sim::GROUND_LAYER);
        app.insert_resource(stations);
        app.init_resource::<IndustryRegistry>();
        app.init_resource::<TrackNetwork>();
        // The town slice stamps its Town Talk lines with the sim tick.
        app.init_resource::<rail_sim::StationService>();
        app.init_resource::<rail_sim::ComplaintFeed>();

        app.add_plugins(crate::town::TownBuildingsPlugin);
        app.add_plugins(AtmospherePlugin);
        app
    }

    fn count<C: Component>(app: &mut App) -> usize {
        app.world_mut().query::<&C>().iter(app.world()).count()
    }

    /// Stand `n` finished buildings in the world.
    ///
    /// The town slice takes ten seconds of stake-then-scaffold before a lot has
    /// windows to light, and that machine is tested where it lives. What this
    /// layer owns is "one light per lot that reports a lit frame", so the lots
    /// are placed directly rather than waited for.
    fn stand_lots(app: &mut App, n: i32) -> Vec<Entity> {
        (0..n)
            .map(|i| {
                app.world_mut()
                    .spawn((
                        BuildingWindows {
                            lit_frame: Some(0),
                            flip_x: false,
                        },
                        Transform::from_xyz((i * 16) as f32, 128.0, 1.5),
                    ))
                    .id()
            })
            .collect()
    }

    #[test]
    fn ambient_wrap_is_a_whole_number_of_every_loop() {
        for period in [
            WATER_SHIMMER_PERIOD,
            COAST_FOAM_PERIOD,
            CHIMNEY_SMOKE_PERIOD,
        ] {
            let loops = AMBIENT_WRAP / period;
            assert!(
                (loops - loops.round()).abs() < 1e-4,
                "ambient clock must wrap on a loop boundary (period {period})"
            );
        }
    }

    #[test]
    fn atmosphere_layers_sit_in_band_order() {
        // Terrain 0.0 < water decals < buildings. Lit windows are relative to
        // their own lot, so they are asserted in `windows.rs` instead.
        assert!(WATER_DECAL_Z > 0.0 && WATER_DECAL_Z < 0.5);
        assert!(COAST_FOAM_Z > WATER_DECAL_Z && COAST_FOAM_Z < 0.5);
        // Smoke reads over the town; the tint caps the world but stays under
        // the camera plane at 1000.
        assert!(CHIMNEY_SMOKE_Z > 2.0);
        assert!(DAY_TINT_Z > 4.5 && DAY_TINT_Z < 1000.0);
    }

    #[test]
    fn the_world_is_never_still() {
        let mut app = test_app();
        stand_lots(&mut app, 24);
        app.update();
        app.update();

        // Brief §9: over ten seconds of panning, at least three things are
        // moving that the player did not cause.
        assert!(count::<LitWindow>(&mut app) > 20, "a dense district lights up");
        assert!(count::<ChimneySmoke>(&mut app) > 0, "occupied buildings smoke");
        assert!(count::<WaterShimmer>(&mut app) > 0, "open water shimmers");
        assert!(count::<CoastFoam>(&mut app) > 0, "the coast laps");
    }

    /// Wiring check for the water rebake: the plugin has to actually register
    /// it, not just define it. This is the reported bug end to end — a New Map
    /// left the previous world's sea shimmering over dry ground.
    #[test]
    fn a_new_world_does_not_inherit_the_old_one_s_water() {
        let mut app = test_app();
        app.update();
        assert!(count::<WaterShimmer>(&mut app) + count::<CoastFoam>(&mut app) > 0);

        // The same size, and not a drop of water on it.
        app.world_mut()
            .insert_resource(rail_map::MapGrid::empty(24, 24, 7));
        app.update();
        app.update();

        assert_eq!(
            count::<WaterShimmer>(&mut app) + count::<CoastFoam>(&mut app),
            0,
            "water is still being drawn on a map that has none"
        );
    }

    #[test]
    fn a_settled_town_does_not_rebake() {
        let mut app = test_app();
        app.update();
        let before = count::<LitWindow>(&mut app);
        app.update();
        assert_eq!(before, count::<LitWindow>(&mut app));
    }

    #[test]
    fn an_abandoned_town_goes_dark() {
        let mut app = test_app();
        let lots = stand_lots(&mut app, 8);
        // Two frames: the building atlas is baked on the first, and nothing can
        // be lit until it exists.
        app.update();
        app.update();
        assert!(count::<LitWindow>(&mut app) > 0);

        // The town empties: the lots are cleared and the density goes with them.
        for lot in lots {
            app.world_mut().entity_mut(lot).despawn();
        }
        let mut density = app.world_mut().resource_mut::<TownDensity>();
        for y in 0..24 {
            for x in 0..24 {
                density.set(TileCoord { x, y }, 0.0);
            }
        }
        app.update();

        assert_eq!(count::<LitWindow>(&mut app), 0, "abandoned tiles go dark");
        assert_eq!(count::<ChimneySmoke>(&mut app), 0, "abandoned chimneys go out");
    }

    #[test]
    fn a_lot_that_stops_reporting_windows_loses_its_light() {
        // Windows going dark is the first signal of decline, and the town slice
        // expresses it by dropping `lit_frame`. Honour it without waiting for
        // the lot to be demolished.
        let mut app = test_app();
        let lots = stand_lots(&mut app, 4);
        // Two frames — see `an_abandoned_town_goes_dark`.
        app.update();
        app.update();
        assert_eq!(count::<LitWindow>(&mut app), 4);

        for lot in &lots {
            app.world_mut()
                .entity_mut(*lot)
                .get_mut::<BuildingWindows>()
                .expect("lot has windows")
                .lit_frame = None;
        }
        app.update();
        assert_eq!(count::<LitWindow>(&mut app), 0, "dimmed lots go dark");
    }

    #[test]
    fn pause_holds_both_clocks() {
        let mut app = test_app();
        app.update();
        app.world_mut().resource_mut::<SimClock>().paused = true;
        app.world_mut().resource_mut::<TimeOfDay>().fraction = 0.4;
        app.world_mut().resource_mut::<AmbientClock>().secs = 5.0;
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(app.world().resource::<TimeOfDay>().fraction, 0.4);
        assert_eq!(app.world().resource::<AmbientClock>().secs, 5.0);
    }
}
