//! Building density rings around stations, driven by service scores.
//!
//! # A town grows over days, not seconds
//!
//! Binding standard: [`docs/design/17-time-and-pacing.md`](../../../docs/design/17-time-and-pacing.md).
//!
//! This pass used to run **every tick** at 4% of the remaining gap, which put a
//! block at half its target in seventeen ticks — a quarter of a real second —
//! and full inside one. The player laid a line and the town was finished before
//! they let go of the mouse, which is the owner's report: *"house growth happens
//! too quickly, within a few in-game minutes. It should be more gradual, e.g.
//! over a few days."*
//!
//! Growth is therefore denominated in **sim days** and nothing else. It
//! advances [`GROWTH_PASSES_PER_DAY`] times a day, by [`GROWTH_APPROACH_RATE`]
//! of the remaining gap each time, which is an exponential approach with a time
//! constant of `1/0.302` ≈ 3.3 sim days. Against the lot thresholds it uses
//! (`rail_town::town::lots::LOT_UP` = 0.14 / 0.32 / 0.56 / 0.80) a fully served
//! block therefore takes up its lots on this schedule:
//!
//! | Lot | Sim days | Real minutes at 1x |
//! | --- | --- | --- |
//! | first — a stake, then a cottage | 0.5 | 1.1 |
//! | second | 1.3 | 2.9 |
//! | third | 2.7 | 6.1 |
//! | fourth | 5.3 | 12.0 |
//!
//! The first cottage inside the first sim day is deliberate and is not a
//! loophole in "over a few days": brief 06 §1 wants growth to be *visibly
//! caused*, and a consequence the player cannot connect to their decision has
//! not been caused as far as they are concerned. The **district** is the thing
//! that takes days; the first hint that it has started is prompt.
//!
//! Decline runs on the same rate in the same units, so a district that loses its
//! service sheds half its buildings over about two and a half sim days — brief
//! 06 §3.2's *"legible and gradual"* in the units that brief's promise implies.

use std::collections::HashMap;

use bevy_ecs::prelude::*;

use crate::ids::TileCoord;
use crate::peeps::TICKS_PER_DAY;
use crate::stations::{catchment_influence, StationRegistry, StationService};
use crate::track::TrackTerrain;

/// Chebyshev radius (tiles) of the growth ring around each station.
pub const GROWTH_RADIUS: i32 = 5;

/// Maximum stored density per tile (`1.0` = fully built-up).
pub const MAX_DENSITY: f32 = 1.0;

/// Growth passes in one sim day — once a sim-hour.
///
/// The cadence is the *resolution* of growth, not its speed:
/// [`GROWTH_APPROACH_RATE`] is derived from it so that changing one without the
/// other is a visible mistake rather than a silent re-pacing. An hour is fine
/// enough that a block crossing a lot threshold does so at a moment nobody can
/// predict, and coarse enough that the pass costs a 360th of what it used to.
pub const GROWTH_PASSES_PER_DAY: u64 = 24;

/// Ticks between growth passes.
pub const GROWTH_INTERVAL_TICKS: u64 = TICKS_PER_DAY / GROWTH_PASSES_PER_DAY;

/// How much of the remaining gap to its target a cell closes each pass.
///
/// Derived, not chosen: the target is a **half-life of about 2.3 sim days**, so
/// `1 - (1 - r)^24 = 1 - e^-0.302` over one day. That puts the first cottage
/// half a day after service starts and the fourth lot on day five, which is the
/// table in the module docs.
pub const GROWTH_APPROACH_RATE: f32 = 0.0125;

/// True on ticks the growth pass is due.
///
/// Public because the cadence is part of the pacing contract: tests that want
/// "a sim day of growth" should step the tick and ask, rather than counting
/// system runs, so they keep meaning the same thing if the cadence moves.
pub fn growth_due(tick: u64) -> bool {
    tick.is_multiple_of(GROWTH_INTERVAL_TICKS)
}

/// Steepness of the fall from a town's core to its edge.
///
/// The ramp used to be linear, which left a thin skirt of density all the way
/// out to the catchment boundary: every town read as the same soft blob, and
/// the map filled with evenly scattered single buildings instead of a few
/// distinct places. A power curve holds the core at full density and collapses
/// the outskirts, which is what makes a town read as a town (brief 06 §7 —
/// *town scale, not city scale*).
const EDGE_FALLOFF_POW: f32 = 2.6;

/// Below this, a tile is open country rather than outskirts.
///
/// This is the **hard edge**: past it a tile grows nothing, however good the
/// service is. Empty land between towns is a feature, not a gap.
const EDGE_CUTOFF: f32 = 0.16;

/// Share of full density supported at `dist` tiles from a station of `radius`.
///
/// Steep, and it stops: for the four station tiers (catchment 3 / 5 / 6 / 8)
/// this puts the last built ring at 2 / 3 / 3 / 4 tiles out. A better station
/// still makes a bigger town — it just makes a town, not a haze.
pub fn town_falloff(dist: i32, radius: i32) -> f32 {
    if radius <= 0 || dist < 0 || dist > radius {
        return 0.0;
    }
    let t = 1.0 - dist as f32 / (radius + 1) as f32;
    let shaped = t.powf(EDGE_FALLOFF_POW);
    if shaped < EDGE_CUTOFF {
        0.0
    } else {
        shaped
    }
}

/// Sparse building density keyed by tile.
///
/// Values are in `0.0..=`[`MAX_DENSITY`]. Tiles with density near zero may be
/// omitted; readers should treat missing tiles as `0.0`.
///
/// # The map is the edge of the world
///
/// A station may legally stand two tiles from the border, and its growth ring
/// reaches further than that — so the ring is the one thing in the sim that
/// routinely asks about tiles which do not exist. Density therefore carries the
/// map extent it belongs to ([`TownDensity::set_bounds`]) and refuses writes
/// outside it. Presentation draws whatever is in here, so "nothing is stored off
/// the map" is the cheapest possible guarantee that nothing is *drawn* off the
/// map. Bounds are unset until the world's terrain is known, which keeps
/// hand-built test fixtures and save restores working unchanged.
#[derive(Debug, Clone, Default, Resource)]
pub struct TownDensity {
    cells: HashMap<(i32, i32), f32>,
    /// Map extent the cells are confined to, once the world is known.
    bounds: Option<(u32, u32)>,
}

impl TownDensity {
    pub fn get(&self, tile: TileCoord) -> f32 {
        self.cells
            .get(&(tile.x, tile.y))
            .copied()
            .unwrap_or(0.0)
    }

    /// Confine this field to a `width` x `height` map, dropping anything outside.
    ///
    /// Idempotent and O(1) once the bounds have settled, so it is safe to call
    /// every tick: the sweep only runs when the world underneath actually
    /// changes size (a new map, a loaded save).
    pub fn set_bounds(&mut self, width: u32, height: u32) {
        if self.bounds == Some((width, height)) {
            return;
        }
        self.bounds = Some((width, height));
        self.cells
            .retain(|&(x, y), _| within(TileCoord { x, y }, width, height));
    }

    /// Map extent this field is confined to, if the world is known yet.
    pub fn bounds(&self) -> Option<(u32, u32)> {
        self.bounds
    }

    /// Whether `tile` is somewhere density may legally exist.
    pub fn in_bounds(&self, tile: TileCoord) -> bool {
        match self.bounds {
            Some((w, h)) => within(tile, w, h),
            None => true,
        }
    }

    pub fn set(&mut self, tile: TileCoord, density: f32) {
        if !self.in_bounds(tile) {
            return;
        }
        let d = density.clamp(0.0, MAX_DENSITY);
        if d < 0.001 {
            self.cells.remove(&(tile.x, tile.y));
        } else {
            self.cells.insert((tile.x, tile.y), d);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (TileCoord, f32)> + '_ {
        self.cells.iter().map(|(&(x, y), &d)| (TileCoord { x, y }, d))
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
}

#[inline]
fn within(tile: TileCoord, width: u32, height: u32) -> bool {
    tile.x >= 0 && tile.y >= 0 && (tile.x as u32) < width && (tile.y as u32) < height
}

/// Target density at `tile` from the strongest nearby station influence.
///
/// `influence = (score / 100) * town_falloff(dist, radius)` using Chebyshev
/// distance. Service quality sets how *dense* a town gets; the falloff sets how
/// *far* it spreads, and it has a hard edge.
pub fn density_target_at(
    tile: TileCoord,
    stations: &StationRegistry,
    service: &StationService,
) -> f32 {
    let mut best = 0.0_f32;
    for station in stations.iter() {
        // Catchment comes from the station's tier, so an Interchange reaches
        // further than a Halt. Asking at full quality keeps that one rule in
        // one place: this call answers *does the station reach here at all*.
        if catchment_influence(station, 100, tile) <= 0.0 {
            continue;
        }
        let dist = (station.tile.x - tile.x)
            .abs()
            .max((station.tile.y - tile.y).abs());
        let quality = f32::from(service.score(station.id).score) / 100.0;
        let influence = (quality * town_falloff(dist, station.tier.catchment())).min(MAX_DENSITY);
        if influence > best {
            best = influence;
        }
    }
    best
}

/// Move every cell in station rings toward its service-driven target.
///
/// The ring is clamped to the map before anything is written. A station may
/// stand two tiles from the border and reach four, so without this a coastal
/// town grows houses out over the edge of the world — and every layer that
/// draws from density draws them there.
pub fn advance_town_growth(
    mut density: ResMut<TownDensity>,
    stations: Res<StationRegistry>,
    service: Res<StationService>,
    terrain: Option<Res<TrackTerrain>>,
) {
    // The terrain is the map's extent as the sim knows it. Telling density about
    // it here — rather than at world install — means a swapped world is honoured
    // by whoever ticks next, with no extra wiring anywhere else.
    if let Some(terrain) = terrain.as_deref() {
        density.set_bounds(terrain.width(), terrain.height());
    }

    // The bounds sweep above is a world-identity check and has to run on every
    // tick — a swapped world must be honoured by whoever ticks next. Growth
    // itself is on the day clock (module docs), so it is gated after it.
    if !growth_due(service.tick) {
        return;
    }

    if stations.is_empty() {
        return;
    }

    // Collect tiles that need an update (union of rings) so we also shrink
    // cells that fall out of good service without iterating the whole map.
    let mut tiles: Vec<TileCoord> = Vec::new();
    for station in stations.iter() {
        let radius = station.tier.catchment();
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let tile = TileCoord {
                    x: station.tile.x + dx,
                    y: station.tile.y + dy,
                };
                if !density.in_bounds(tile) {
                    continue;
                }
                tiles.push(tile);
            }
        }
    }
    tiles.sort_by_key(|t| (t.y, t.x));
    tiles.dedup();

    let ground = terrain.as_deref();
    for tile in tiles {
        // A house needs ground to stand on: water and the impassable band are
        // not habitable, however good the service is (playtest: "houses should
        // not spawn on water tiles"). Forcing the *target* to zero rather than
        // skipping the tile means density a stale save already put there
        // recedes through the same approach rate as everything else — the
        // misplaced houses move out instead of squatting forever. Fixture
        // worlds with no terrain resource keep the old behaviour: no terrain,
        // no opinion.
        let habitable = ground.map_or(true, |t| {
            !t.is_water(tile)
                && t.height_at(tile).unwrap_or(0) < crate::track::MOUNTAIN_HEIGHT_MIN
        });
        let target = if habitable {
            density_target_at(tile, &stations, &service)
        } else {
            0.0
        };
        let current = density.get(tile);
        let next = current + (target - current) * GROWTH_APPROACH_RATE;
        density.set(tile, next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::StationId;
    use crate::stations::StationServiceScore;
    use crate::track::GROUND_LAYER;
    use bevy_app::App;

    /// Density at which presentation takes up each successive lot of a block.
    ///
    /// The authority is `rail_town::town::lots::LOT_UP`; `rail_sim` cannot
    /// depend on `rail_town`, so it is restated here and the two halves are
    /// pinned together in that module's
    /// `a_block_fills_over_days_and_the_first_house_lands_promptly`.
    /// These are what make the pacing table in the module docs *visible*.
    const LOT_UP: [f32; 4] = [0.14, 0.32, 0.56, 0.80];

    fn registry_with(tile: TileCoord, name: &str) -> (StationRegistry, StationId) {
        let mut reg = StationRegistry::new();
        let id = reg.insert(name, tile, GROUND_LAYER);
        (reg, id)
    }

    /// A world of one perfectly served station on flat ground.
    fn served_world(tile: TileCoord, map: u32) -> App {
        let mut app = App::new();
        let (stations, id) = registry_with(tile, "Eastgate");
        let mut service = StationService::default();
        service.scores.insert(
            id,
            StationServiceScore {
                score: 100,
                ..Default::default()
            },
        );
        app.insert_resource(stations)
            .insert_resource(service)
            .insert_resource(flat_terrain(map, map))
            .init_resource::<TownDensity>()
            .add_systems(bevy_app::Update, advance_town_growth);
        app
    }

    /// Advance `days` **sim days** of growth.
    ///
    /// Growth tests are written in sim days rather than in iteration counts on
    /// purpose: the cadence is a pacing decision that may move, and a test that
    /// counts system runs quietly stops asking the question it was written to
    /// ask when it does.
    ///
    /// The clock is stepped one [`GROWTH_INTERVAL_TICKS`] at a time rather than
    /// one tick at a time — the pass does nothing on the ticks in between, and
    /// stepping through them costs a hundredfold for no extra coverage. That
    /// the idle ticks really are idle is
    /// [`growth_only_advances_on_its_own_cadence`]'s job.
    fn run_days(app: &mut App, days: f32) {
        let passes = (days * GROWTH_PASSES_PER_DAY as f32).round() as u64;
        for _ in 0..passes {
            app.update();
            app.world_mut().resource_mut::<StationService>().tick += GROWTH_INTERVAL_TICKS;
        }
    }

    fn density_at(app: &App, tile: TileCoord) -> f32 {
        app.world().resource::<TownDensity>().get(tile)
    }

    #[test]
    fn service_up_raises_density_target() {
        let tile = TileCoord { x: 10, y: 10 };
        let (stations, id) = registry_with(tile, "Eastgate");
        let mut service = StationService::default();

        service.scores.insert(
            id,
            StationServiceScore {
                score: 20,
                ..Default::default()
            },
        );
        let low = density_target_at(tile, &stations, &service);

        service.scores.insert(
            id,
            StationServiceScore {
                score: 90,
                ..Default::default()
            },
        );
        let high = density_target_at(tile, &stations, &service);

        assert!(
            high > low,
            "higher service score must raise density target ({high} > {low})"
        );
        assert!(high > 0.8);
    }

    #[test]
    fn growth_moves_density_toward_higher_service() {
        let tile = TileCoord { x: 5, y: 5 };
        let mut app = served_world(tile, 16);
        assert_eq!(density_at(&app, tile), 0.0);

        // Two and a half sim days — the half-life in the module docs.
        run_days(&mut app, 2.5);

        assert!(
            density_at(&app, tile) > 0.5,
            "sustained high service should thicken buildings (got {})",
            density_at(&app, tile)
        );
    }

    /// **The pacing claim.** Brief 17: a served block takes up its four lots
    /// over about five sim days, and the first of them inside the first day.
    ///
    /// Stated against `rail_town`'s lot thresholds, because those are what turn
    /// a density value into a building the player can see. A model that reaches
    /// the right number on the wrong day is the bug this test exists to catch.
    #[test]
    fn a_served_block_fills_its_lots_over_days_not_seconds() {
        let tile = TileCoord { x: 8, y: 8 };
        let expected_by: [(f32, usize); 4] = [(1.0, 0), (2.0, 1), (3.5, 2), (6.0, 3)];

        for (day, lot) in expected_by {
            let mut app = served_world(tile, 24);
            run_days(&mut app, day);
            let d = density_at(&app, tile);
            assert!(
                d >= LOT_UP[lot],
                "lot {} should be taken up by sim day {day}, density was {d} \
                 against a threshold of {}",
                lot + 1,
                LOT_UP[lot],
            );
        }

        // …and not before. The whole complaint was a town that finished while
        // the player was still holding the mouse.
        let mut app = served_world(tile, 24);
        run_days(&mut app, 0.25);
        assert!(
            density_at(&app, tile) < LOT_UP[0],
            "a quarter of a sim day in, the block is still open ground; got {}",
            density_at(&app, tile)
        );

        let mut app = served_world(tile, 24);
        run_days(&mut app, 2.0);
        assert!(
            density_at(&app, tile) < LOT_UP[2],
            "the third lot must not arrive on day two — that is the whole \
             district built in under five real minutes; got {}",
            density_at(&app, tile)
        );
    }

    /// The gate is the day clock, not the frame rate.
    #[test]
    fn growth_only_advances_on_its_own_cadence() {
        let tile = TileCoord { x: 6, y: 6 };
        let mut app = served_world(tile, 16);

        // Tick zero is due, so one pass lands; the rest of the interval is idle.
        app.update();
        let after_first = density_at(&app, tile);
        assert!(after_first > 0.0, "the first pass should land");

        for _ in 0..(GROWTH_INTERVAL_TICKS - 1) {
            app.world_mut().resource_mut::<StationService>().tick += 1;
            app.update();
        }
        assert_eq!(
            density_at(&app, tile),
            after_first,
            "growth ran between passes — the cadence is not holding"
        );

        app.world_mut().resource_mut::<StationService>().tick += 1;
        app.update();
        assert!(
            density_at(&app, tile) > after_first,
            "the next due tick should advance growth"
        );
    }

    #[test]
    fn service_drop_lowers_target_so_density_can_shrink() {
        let tile = TileCoord { x: 3, y: 3 };
        let (stations, id) = registry_with(tile, "Westbrook");
        let mut service = StationService::default();
        service.scores.insert(
            id,
            StationServiceScore {
                score: 100,
                ..Default::default()
            },
        );
        let high = density_target_at(tile, &stations, &service);

        service.scores.insert(
            id,
            StationServiceScore {
                score: 10,
                ..Default::default()
            },
        );
        let low = density_target_at(tile, &stations, &service);

        assert!(low < high);
        // A cell sitting at `high` sheds toward `low` on the same day clock it
        // grew on: roughly half the gap over two and a half sim days.
        let passes = (2.5 * GROWTH_PASSES_PER_DAY as f32) as u32;
        let mut density = high;
        for _ in 0..passes {
            density += (low - density) * GROWTH_APPROACH_RATE;
        }
        assert!(
            density < high * 0.7,
            "two and a half sim days of neglect should visibly thin the \
             district: {density} against {high}"
        );

        // …and it is *gradual*: a single sim day must not empty a street.
        let mut after_a_day = high;
        for _ in 0..GROWTH_PASSES_PER_DAY {
            after_a_day += (low - after_a_day) * GROWTH_APPROACH_RATE;
        }
        assert!(
            after_a_day > high * 0.7,
            "brief 06 §3.2 wants decline legible and gradual, not a demolition: \
             {after_a_day} against {high} after one sim day"
        );
    }

    #[test]
    fn falloff_is_steep_and_then_stops() {
        // A core at full density, a couple of rings of visible outskirts, and
        // then nothing — not a gradient reaching the catchment boundary.
        assert_eq!(town_falloff(0, 5), 1.0);
        let mut last = 1.0;
        for d in 1..=5 {
            let f = town_falloff(d, 5);
            assert!(f < last || f == 0.0, "falloff must decrease at {d}: {f} !< {last}");
            last = f;
        }
        assert_eq!(town_falloff(4, 5), 0.0, "the edge has to be hard");
        assert_eq!(town_falloff(5, 5), 0.0);
        assert_eq!(town_falloff(6, 5), 0.0, "nothing grows outside the catchment");

        // Halfway out, the outskirts are well under half the core's density.
        assert!(town_falloff(2, 5) < 0.5);
    }

    #[test]
    fn a_bigger_station_still_makes_a_bigger_town() {
        let built = |radius: i32| (0..=radius).filter(|d| town_falloff(*d, radius) > 0.0).count();
        assert!(built(3) < built(5));
        assert!(built(5) <= built(8));
        assert!(built(8) < 8, "even an interchange must leave open country");
    }

    /// Terrain of `w` x `h` plains at sea level.
    fn flat_terrain(w: u32, h: u32) -> TrackTerrain {
        TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 0i8)))
    }

    #[test]
    fn a_town_never_grows_off_the_edge_of_the_map() {
        // The reported bug: a station near the border grew a ring of homes past
        // the map edge, because the ring walked `tile +/- radius` with nothing
        // to stop it. Two tiles in is a legal seed position, and the catchment
        // reaches further than that.
        let mut app = served_world(TileCoord { x: 2, y: 2 }, 16);
        run_days(&mut app, 4.0);

        let density = app.world().resource::<TownDensity>();
        assert!(!density.is_empty(), "the town has to actually grow");
        for (tile, d) in density.iter() {
            assert!(
                tile.x >= 0 && tile.y >= 0 && tile.x < 16 && tile.y < 16,
                "density escaped the map at ({}, {}) = {d}",
                tile.x,
                tile.y
            );
        }
    }

    #[test]
    fn houses_never_stand_on_water_or_the_impassable_band() {
        // The reported bug: a well-served catchment grew homes straight across
        // a lake, because the growth pass never asked the ground's opinion. A
        // lake tile and a cliff tile sit inside the catchment here; both must
        // stay empty while the land around them fills in.
        let mut app = App::new();
        let (stations, id) = registry_with(TileCoord { x: 8, y: 8 }, "Lakeside");
        let mut service = StationService::default();
        service.scores.insert(
            id,
            StationServiceScore {
                score: 100,
                ..Default::default()
            },
        );
        let water = TileCoord { x: 9, y: 8 };
        let cliff = TileCoord { x: 8, y: 9 };
        let terrain = TrackTerrain::new(
            16,
            16,
            (0..16i32).flat_map(|y| {
                (0..16i32).map(move |x| {
                    if (TileCoord { x, y }) == water {
                        (true, -2i8)
                    } else if (TileCoord { x, y }) == cliff {
                        (false, crate::track::MOUNTAIN_HEIGHT_MIN)
                    } else {
                        (false, 0i8)
                    }
                })
            }),
        );
        // Stale-save shape: density already sits on the water before the first
        // tick, as a save written before this rule would have it.
        let mut density = TownDensity::default();
        density.set_bounds(16, 16);
        density.set(water, 0.8);

        app.insert_resource(stations)
            .insert_resource(service)
            .insert_resource(terrain)
            .insert_resource(density)
            .add_systems(bevy_app::Update, advance_town_growth);

        // Receding, not teleporting, is the designed shape — so this is stated
        // in sim days. Sixteen of them takes the seeded 0.8 under 0.01 at the
        // model's rate, and the land beside the station is built by then.
        run_days(&mut app, 16.0);

        let density = app.world().resource::<TownDensity>();
        assert!(
            density.get(TileCoord { x: 7, y: 8 }) > 0.1,
            "the land beside the station has to actually grow"
        );
        assert!(
            density.get(water) < 0.01,
            "the lake kept its houses: {}",
            density.get(water)
        );
        assert!(
            density.get(cliff) < 0.01,
            "the cliff face kept its houses: {}",
            density.get(cliff)
        );
    }

    #[test]
    fn a_new_map_drops_the_old_map_s_density() {
        // A smaller world must not inherit the cells of a larger one: those
        // tiles are off the new map, and everything that draws density would
        // draw them there.
        let mut density = TownDensity::default();
        density.set_bounds(32, 32);
        density.set(TileCoord { x: 30, y: 30 }, 0.9);
        density.set(TileCoord { x: 4, y: 4 }, 0.9);
        assert_eq!(density.len(), 2);

        density.set_bounds(16, 16);
        assert_eq!(density.get(TileCoord { x: 30, y: 30 }), 0.0);
        assert_eq!(density.get(TileCoord { x: 4, y: 4 }), 0.9);
        assert_eq!(density.len(), 1);

        // And it stays refused, rather than being swept once and forgotten.
        density.set(TileCoord { x: 30, y: 30 }, 0.9);
        density.set(TileCoord { x: -1, y: 4 }, 0.9);
        assert_eq!(density.len(), 1, "out-of-bounds writes must not land");
    }

    #[test]
    fn density_is_unbounded_until_the_world_is_known() {
        // Hand-built fixtures and save restores write before any terrain exists.
        let mut density = TownDensity::default();
        assert_eq!(density.bounds(), None);
        density.set(TileCoord { x: 900, y: 900 }, 0.5);
        assert_eq!(density.get(TileCoord { x: 900, y: 900 }), 0.5);
    }

    #[test]
    fn a_town_is_a_core_not_a_carpet() {
        let tile = TileCoord { x: 20, y: 20 };
        let (stations, id) = registry_with(tile, "Eastgate");
        let mut service = StationService::default();
        service.scores.insert(
            id,
            StationServiceScore {
                score: 100,
                ..Default::default()
            },
        );

        let radius = 8;
        let mut built = 0;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let t = TileCoord {
                    x: tile.x + dx,
                    y: tile.y + dy,
                };
                if density_target_at(t, &stations, &service) > 0.0 {
                    built += 1;
                }
            }
        }
        // A default station's catchment is 11×11 = 121 tiles. The town it grows
        // must sit well inside that, with open country the rest of the way out.
        assert_eq!(built, 49, "a town should be a 7x7 core, got {built} tiles");
    }
}
