//! Periodic spawn of new settlements / industries outside served coverage.

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::ids::{StationId, TileCoord};
use crate::peeps::{ComplaintEntry, ComplaintFeed, TalkKind, SIM_SECONDS_PER_TICK};
use crate::stations::{
    GoodKind, IndustryId, IndustryRegistry, IndustryTier, StationRegistry, StationService,
};
use crate::track::{TrackNetwork, TrackTerrain, GROUND_LAYER};
use crate::trains::track_for_station;

use super::sites::pick_demand_site;

// # Reading these numbers
//
// A sim-minute is **not** a real minute, and the gap is enormous. One tick is
// [`SIM_SECONDS_PER_TICK`] (10) sim-seconds, and `FixedUpdate` runs at 64 Hz,
// so the world lives about **640x faster than the clock on the wall**:
//
//     real_seconds = sim_minutes * 60 / SIM_SECONDS_PER_TICK / 64
//                  = sim_minutes * 0.09375
//
// So a value that reads like a comfortable few minutes is under half a second
// of play. Brief 08 §4.2 asks for "roughly one meaningful new opportunity
// every few minutes early on"; at four sim-minutes that would be one every
// 0.375 s, and the whole session's budget would be spent inside the first three
// seconds. Convert to real time before changing any of these.

/// Felt rhythm: first new demand after this many sim-minutes (~3 real minutes).
pub const DEMAND_FIRST_DELAY_SIM_MINUTES: u32 = 1_920;
/// Gap after the first opportunity, in sim-minutes (~4 real minutes).
///
/// The *starting* gap. It widens as the map fills — see
/// [`DEMAND_INTERVAL_GROWTH_SIM_MINUTES`].
pub const DEMAND_INTERVAL_SIM_MINUTES: u32 = 2_560;
/// Each opportunity widens the next gap by this percentage of the opening one,
/// so the rhythm stretches as the network matures.
///
/// Brief 08 §4.2: *"roughly one meaningful new opportunity every few minutes
/// early on, stretching as the network matures."* Early on the player has
/// nothing to do and wants the world to speak up; an hour in they have a
/// railway to run and a fresh marker every four minutes is nagging. At 17% of
/// a four-minute opening gap that is +40 s a time: four minutes at the start,
/// ten by the tenth.
///
/// A percentage of the configured gap rather than a fixed number of minutes, so
/// a world (or a test) that sets a different cadence gets a proportional curve
/// rather than this constant swamping it.
pub const DEMAND_INTERVAL_GROWTH_PERCENT: u32 = 17;
/// The gap never stretches past this percentage of the opening one (~10 real
/// minutes at the default cadence).
///
/// A ceiling rather than an end: brief 08 §4 is explicit that a player who has
/// connected everything must never be finished, so the world keeps asking —
/// just less often.
pub const DEMAND_INTERVAL_MAX_PERCENT: u32 = 250;

/// How many **unconnected** opportunities may stand open at once.
///
/// This replaced a lifetime cap of eight, which was the whole session's supply:
/// at one every four real minutes the world fell permanently silent at minute
/// thirty-one, and brief 08 §4 calls that the missing rung — *"a player who has
/// connected everything available has finished, and there is no hour-long
/// arc."*
///
/// A cap is still wanted, but on the *board* rather than the session. Three
/// unanswered markers is a menu; a dozen is wallpaper, and a player who ignores
/// the world should not drown in reminders that they are ignoring it. Connect
/// one and the next is free to appear.
pub const DEMAND_MAX_PENDING: usize = 3;

/// Minimum Manhattan spacing from existing anchors, for the first opportunity.
pub const DEMAND_MIN_ANCHOR_SPACING: i32 = 8;
/// Extra minimum spacing per opportunity already revealed.
///
/// Brief 08 §4.3, the pull outward: *"new demand should appear at increasing
/// distance as the network grows… the terrain that was impossible at minute
/// five is the interesting problem at minute fifty."* Without this the twelfth
/// opportunity can land as close as the first, and the difficulty curve the
/// world is supposed to provide simply never arrives.
pub const DEMAND_SPACING_GROWTH: i32 = 2;
/// Ceiling on the hard spacing floor, so a filling map can still find a site.
///
/// Roughly half the width of the default map. Past that, a board with anchors
/// spread across it has nowhere left that satisfies the floor,
/// [`pick_demand_site`] returns `None`, and the world falls silent — which is
/// the failure this whole change exists to remove. Distance beyond this point
/// is expressed as a *preference* in the site score instead, which degrades
/// gracefully when the map runs out of far away.
pub const DEMAND_MIN_SPACING_MAX: i32 = 32;

/// Reject candidate tiles with service influence above this (0..=1).
pub const DEMAND_SERVICE_INFLUENCE_MAX: f32 = 0.05;

const SETTLEMENT_NAMES: &[&str] = &[
    "Ridgeline",
    "Northford",
    "Southmere",
    "Ashvale",
    "Clearwater",
    "Stonebridge",
    "Hillcrest",
    "Lowmarsh",
];

/// Name, lot size, produces, consumes. The lot is how much ground the site
/// stands on, and therefore how far a goods platform may sit from its centre
/// (04 §6) — quarries and wharves sprawl, yards do not.
const INDUSTRY_SPECS: &[(&str, IndustryTier, Option<GoodKind>, Option<GoodKind>)] = &[
    ("Quarry Ridge", IndustryTier::Complex, Some(GoodKind::Ore), None),
    ("Harbor Foundry", IndustryTier::Works, None, Some(GoodKind::Ore)),
    ("Cedar Yard", IndustryTier::Yard, Some(GoodKind::Lumber), None),
    ("Builders' Wharf", IndustryTier::Complex, None, Some(GoodKind::Lumber)),
    ("High Quarry", IndustryTier::Works, Some(GoodKind::Ore), None),
    ("Mill End", IndustryTier::Yard, None, Some(GoodKind::Ore)),
];

fn minutes_to_ticks(minutes: u32) -> u32 {
    let secs = minutes.saturating_mul(60);
    secs.saturating_add(SIM_SECONDS_PER_TICK.saturating_sub(1)) / SIM_SECONDS_PER_TICK
}

/// What kind of new demand was revealed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DemandOpportunityKind {
    Settlement(StationId),
    Industry(IndustryId),
}

/// An open opportunity until the player connects it (or dismisses the alert).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DemandOpportunity {
    pub kind: DemandOpportunityKind,
    pub name: String,
    pub tile: TileCoord,
}

/// Tunable session state for new-demand spawning.
///
/// Field order and types are load-bearing: this resource is serialised whole
/// into [`WorldSnapshot`](crate::save::WorldSnapshot), and `bincode` reads by
/// position rather than by name.
#[derive(Debug, Clone, PartialEq, Resource, Serialize, Deserialize)]
pub struct DemandSpawner {
    /// Ticks remaining until the next spawn attempt.
    pub ticks_until_next: u32,
    /// How many new anchors this session has revealed. **Uncapped** — it paces
    /// the rhythm and the reach, it does not end them.
    pub spawned_count: u32,
    /// How many unconnected opportunities may stand open at once.
    pub max_pending: u32,
    /// The **opening** gap between opportunities, in ticks. The gap actually
    /// used widens with [`Self::spawned_count`] — see [`Self::interval_after`].
    pub interval_ticks: u32,
    /// Base minimum spacing from existing anchors. The *effective* floor grows
    /// with [`Self::spawned_count`] — see [`Self::effective_min_spacing`].
    pub min_spacing: i32,
    /// Max service influence allowed at a candidate tile.
    pub max_influence: f32,
    /// Open opportunities (for alerts / presentation markers).
    pub open: Vec<DemandOpportunity>,
    /// Settlement name index into [`SETTLEMENT_NAMES`] (wraps).
    pub next_settlement: usize,
    /// Industry spec index into [`INDUSTRY_SPECS`] (wraps).
    pub next_industry: usize,
    /// Alternate settlement / industry.
    pub next_is_settlement: bool,
}

impl Default for DemandSpawner {
    fn default() -> Self {
        Self {
            ticks_until_next: minutes_to_ticks(DEMAND_FIRST_DELAY_SIM_MINUTES),
            spawned_count: 0,
            max_pending: DEMAND_MAX_PENDING as u32,
            interval_ticks: minutes_to_ticks(DEMAND_INTERVAL_SIM_MINUTES),
            min_spacing: DEMAND_MIN_ANCHOR_SPACING,
            max_influence: DEMAND_SERVICE_INFLUENCE_MAX,
            open: Vec::new(),
            next_settlement: 0,
            next_industry: 0,
            next_is_settlement: true,
        }
    }
}

impl DemandSpawner {
    /// Test helper: fire on the next Advance tick.
    pub fn ready_now(&mut self) {
        self.ticks_until_next = 0;
    }

    /// Gap to the opportunity after the `spawned_count`-th, in ticks.
    ///
    /// Starts at [`Self::interval_ticks`] and widens by
    /// [`DEMAND_INTERVAL_GROWTH_PERCENT`] of it each time, up to
    /// [`DEMAND_INTERVAL_MAX_PERCENT`] — so the world speaks up often while the
    /// player has nothing to do and eases off once they have a railway to run.
    pub fn interval_after(&self, spawned_count: u32) -> u32 {
        let percent = 100u64
            .saturating_add(u64::from(DEMAND_INTERVAL_GROWTH_PERCENT) * u64::from(spawned_count))
            .min(u64::from(DEMAND_INTERVAL_MAX_PERCENT));
        let ticks = u64::from(self.interval_ticks) * percent / 100;
        ticks.min(u64::from(u32::MAX)).max(1) as u32
    }

    /// Hard spacing floor for the next opportunity: the base, plus
    /// [`DEMAND_SPACING_GROWTH`] per opportunity already revealed.
    ///
    /// Brief 08 §4.3 — the pull outward. The tenth new town has to be somewhere
    /// the first one could not have been, or the map stops getting harder.
    pub fn effective_min_spacing(&self) -> i32 {
        self.min_spacing
            .saturating_add(DEMAND_SPACING_GROWTH.saturating_mul(self.spawned_count as i32))
            .min(DEMAND_MIN_SPACING_MAX.max(self.min_spacing))
    }

    /// Distance the site scorer should *prefer*, beyond the hard floor.
    pub fn preferred_distance(&self) -> i32 {
        self.effective_min_spacing()
            .saturating_add(DEMAND_SPACING_GROWTH.saturating_mul(self.spawned_count as i32))
    }

    /// `true` when the board is as full of unanswered markers as it may get.
    pub fn board_is_full(&self) -> bool {
        self.open.len() >= self.max_pending as usize
    }

    pub fn is_open_station(&self, id: StationId) -> bool {
        self.open
            .iter()
            .any(|o| matches!(o.kind, DemandOpportunityKind::Settlement(s) if s == id))
    }

    pub fn is_open_industry(&self, id: IndustryId) -> bool {
        self.open
            .iter()
            .any(|o| matches!(o.kind, DemandOpportunityKind::Industry(i) if i == id))
    }
}

/// Spawn a new settlement or industry outside served coverage on cadence.
pub fn spawn_new_demand(
    mut spawner: ResMut<DemandSpawner>,
    terrain: Option<Res<TrackTerrain>>,
    mut stations: ResMut<StationRegistry>,
    mut industries: ResMut<IndustryRegistry>,
    mut service: ResMut<StationService>,
    mut talk: ResMut<ComplaintFeed>,
    network: Res<TrackNetwork>,
) {
    // Drop opportunities that the player has connected.
    spawner.open.retain(|opp| match opp.kind {
        DemandOpportunityKind::Settlement(id) => stations
            .get(id)
            .map(|s| track_for_station(&network, s.tile, s.layer).is_none())
            .unwrap_or(false),
        DemandOpportunityKind::Industry(id) => industries
            .get(id)
            .map(|i| track_for_station(&network, i.tile, GROUND_LAYER).is_none())
            .unwrap_or(false),
    });

    let Some(terrain) = terrain else {
        return;
    };
    // Wait until the world has its opening anchors.
    if stations.is_empty() {
        return;
    }

    if spawner.ticks_until_next > 0 {
        spawner.ticks_until_next -= 1;
        return;
    }

    // The clock has run out, but the board is already carrying as many
    // unanswered markers as it should. Hold at zero rather than resetting: the
    // world is ready, and the moment the player connects one of the standing
    // opportunities the next appears. What is capped is the noise, not the
    // supply — brief 08 §4 wants no session where the world stops asking.
    if spawner.board_is_full() {
        return;
    }

    let prefer_far = spawner.preferred_distance();
    let site = |min_spacing: i32| {
        pick_demand_site(
            &terrain,
            &stations,
            &industries,
            &service,
            min_spacing,
            prefer_far,
            spawner.max_influence,
        )
    };

    // Ask at the grown distance first, and fall back to the opening one rather
    // than saying nothing. The growing floor is a *pull*, not a gate: on a map
    // that has filled up there may be nowhere left thirty tiles from
    // everything, and brief 08 §4 would rather have a nearer opportunity than a
    // silence. `prefer_far` is unchanged either way, so the scorer still picks
    // the furthest of what is left.
    let Some(tile) = site(spawner.effective_min_spacing()).or_else(|| site(spawner.min_spacing))
    else {
        // Nowhere at all is free right now — every candidate is served, water,
        // or occupied. Retry on the next beat rather than spinning every tick,
        // and do not count the miss as an opportunity.
        spawner.ticks_until_next = spawner.interval_after(spawner.spawned_count);
        return;
    };

    let tick = service.tick;
    let spawn_settlement = spawner.next_is_settlement;

    if spawn_settlement {
        let name = SETTLEMENT_NAMES[spawner.next_settlement % SETTLEMENT_NAMES.len()];
        spawner.next_settlement = spawner.next_settlement.saturating_add(1);
        let id = stations.insert(name, tile, GROUND_LAYER);
        service.ensure(id);
        let message = format!("New settlement: {name} - not yet served");
        talk.push(ComplaintEntry {
            kind: TalkKind::Opportunity,
            peep_name: message.clone(),
            station_name: String::new(),
            wait_minutes: 0,
            sim_tick: tick,
            peep_id: None,
            station_id: Some(id),
            tile: Some(tile),
            count: 1,
        });
        spawner.open.push(DemandOpportunity {
            kind: DemandOpportunityKind::Settlement(id),
            name: name.to_string(),
            tile,
        });
    } else {
        let (name, tier, produces, consumes) =
            INDUSTRY_SPECS[spawner.next_industry % INDUSTRY_SPECS.len()];
        spawner.next_industry = spawner.next_industry.saturating_add(1);
        let id = industries.insert_tier(name, tile, tier, produces, consumes);
        let label = if produces.is_some() {
            format!("New industry: {name} - not yet served")
        } else {
            format!("New mill: {name} - not yet served")
        };
        talk.push(ComplaintEntry {
            kind: TalkKind::Opportunity,
            peep_name: label,
            station_name: String::new(),
            wait_minutes: 0,
            sim_tick: tick,
            peep_id: None,
            station_id: None,
            tile: Some(tile),
            count: 1,
        });
        spawner.open.push(DemandOpportunity {
            kind: DemandOpportunityKind::Industry(id),
            name: name.to_string(),
            tile,
        });
    }

    spawner.next_is_settlement = !spawn_settlement;
    spawner.spawned_count = spawner.spawned_count.saturating_add(1);
    spawner.ticks_until_next = spawner.interval_after(spawner.spawned_count);
}

#[cfg(test)]
mod tests {
    /// Ticks per real second at the fixed timestep. Only used to state the
    /// pacing claim in the units a player actually experiences.
    const TICKS_PER_REAL_SECOND: f32 = 64.0;

    fn real_minutes(sim_minutes: u32) -> f32 {
        minutes_to_ticks(sim_minutes) as f32 / TICKS_PER_REAL_SECOND / 60.0
    }

    #[test]
    fn opportunities_arrive_minutes_apart_in_real_time_not_seconds() {
        // A new game must not open with a fistful of markers already on the
        // board (brief 08 §4.2).
        let first = real_minutes(DEMAND_FIRST_DELAY_SIM_MINUTES);
        let gap = real_minutes(DEMAND_INTERVAL_SIM_MINUTES);
        assert!(
            (2.0..6.0).contains(&first),
            "first opportunity at {first:.1} real minutes"
        );
        assert!(
            (2.0..8.0).contains(&gap),
            "opportunities {gap:.1} real minutes apart"
        );
    }

    /// **The arc, as a timeline.** Brief 08 §4.2: a felt rhythm early,
    /// stretching as the network matures, and never a silence.
    ///
    /// The failure this replaces: a lifetime cap of eight meant the world
    /// produced its last opportunity at real minute 31 and then said nothing
    /// for the rest of the session.
    #[test]
    fn the_world_keeps_asking_for_two_hours_and_slows_down_as_it_goes() {
        let spawner = DemandSpawner::default();
        let mut at = real_minutes(DEMAND_FIRST_DELAY_SIM_MINUTES);
        let mut arrivals = vec![at];
        for count in 1..40u32 {
            at += spawner.interval_after(count) as f32 / TICKS_PER_REAL_SECOND / 60.0;
            arrivals.push(at);
        }

        let by_minute = |m: f32| arrivals.iter().filter(|a| **a <= m).count();
        assert!(
            (2.0..=4.0).contains(&arrivals[0]),
            "the first opportunity lands at {:.1} min",
            arrivals[0]
        );
        assert!(
            by_minute(30.0) >= 5,
            "only {} opportunities in the first half hour — too quiet to teach",
            by_minute(30.0)
        );
        assert!(
            by_minute(60.0) >= 8,
            "only {} opportunities in the first hour",
            by_minute(60.0)
        );
        assert!(
            by_minute(120.0) >= 13,
            "a two-hour session should see at least 13 opportunities, saw {}",
            by_minute(120.0)
        );

        // Stretching, not stalling.
        let early_gap = arrivals[1] - arrivals[0];
        let late_gap = arrivals[12] - arrivals[11];
        assert!(
            late_gap > early_gap * 1.8,
            "the rhythm should ease off: {early_gap:.1} min early, \
             {late_gap:.1} min late"
        );
        assert!(
            late_gap <= 11.0,
            "{late_gap:.1} minutes between opportunities is a silence, not a rhythm"
        );
    }

    /// Brief 08 §4.3, the pull outward — later opportunities must be reachable
    /// only by a railway that has grown.
    #[test]
    fn later_opportunities_are_required_to_land_further_out() {
        let mut spawner = DemandSpawner::default();
        let first = spawner.effective_min_spacing();
        assert_eq!(first, DEMAND_MIN_ANCHOR_SPACING);

        spawner.spawned_count = 11;
        let twelfth = spawner.effective_min_spacing();
        assert!(
            twelfth >= first * 3,
            "the twelfth opportunity may still land {twelfth} tiles out where \
             the first had to clear {first} — that is not a difficulty curve"
        );

        // And it stops growing before it can starve the picker of sites.
        spawner.spawned_count = 500;
        assert_eq!(spawner.effective_min_spacing(), DEMAND_MIN_SPACING_MAX);
        assert!(spawner.preferred_distance() >= spawner.effective_min_spacing());
    }

    use super::*;
    use bevy_app::{App, FixedUpdate};

    use crate::economy::JobBoard;
    use crate::stations::seed_stations_and_industries;
    use crate::SimPlugin;

    fn land_terrain(w: u32, h: u32) -> TrackTerrain {
        TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 3i8)))
    }

    #[test]
    fn default_cadence_is_felt_minutes() {
        let s = DemandSpawner::default();
        assert_eq!(s.ticks_until_next, minutes_to_ticks(DEMAND_FIRST_DELAY_SIM_MINUTES));
        assert_eq!(s.interval_ticks, minutes_to_ticks(DEMAND_INTERVAL_SIM_MINUTES));
        assert_eq!(s.max_pending as usize, DEMAND_MAX_PENDING);
        // Pinned in ticks so a careless edit to either constant has to look at
        // the real-time cost. See the module header: 6 ticks per sim-minute.
        assert_eq!(s.ticks_until_next, 11_520);
        assert_eq!(s.interval_ticks, 15_360);
    }

    #[test]
    fn new_demand_appears_outside_served_without_track_expansion() {
        let mut app = App::new();
        app.add_plugins(SimPlugin);
        // Replace default cadence with aggressive test timing.
        {
            let mut spawner = app.world_mut().resource_mut::<DemandSpawner>();
            spawner.ticks_until_next = 0;
            spawner.interval_ticks = 1;
            spawner.max_pending = 4;
            spawner.min_spacing = 6;
        }
        let terrain = land_terrain(48, 48);
        app.world_mut().insert_resource(terrain.clone());

        // Seed opening anchors (same path as production).
        {
            let mut stations = StationRegistry::new();
            let mut industries = IndustryRegistry::new();
            let mut service = StationService::default();
            seed_stations_and_industries(
                &mut stations,
                &mut industries,
                &mut service,
                48,
                48,
                |_| true,
            );
            let world = app.world_mut();
            *world.resource_mut::<StationRegistry>() = stations;
            *world.resource_mut::<IndustryRegistry>() = industries;
            *world.resource_mut::<StationService>() = service;
        }
        // Mark world-seed done so Update hook does not double-seed.
        app.world_mut()
            .insert_resource(crate::WorldAnchorsSeeded(true));

        let initial_stations = app.world().resource::<StationRegistry>().len();
        let initial_industries = app.world().resource::<IndustryRegistry>().len();
        assert!(initial_stations >= 2);

        // Give opening stations some service so coverage is non-zero near them.
        {
            let stations: Vec<StationId> = app
                .world()
                .resource::<StationRegistry>()
                .iter()
                .map(|s| s.id)
                .collect();
            let mut service = app.world_mut().resource_mut::<StationService>();
            for id in stations {
                let s = service.ensure(id);
                s.score = 80;
                s.deliveries = 3;
            }
        }

        // Many Advance ticks, never place track.
        for _ in 0..12 {
            app.world_mut().run_schedule(FixedUpdate);
        }

        let stations = app.world().resource::<StationRegistry>();
        let industries = app.world().resource::<IndustryRegistry>();
        let service = app.world().resource::<StationService>();
        let spawner = app.world().resource::<DemandSpawner>();
        let talk = app.world().resource::<ComplaintFeed>();

        assert!(
            spawner.spawned_count >= 2,
            "expected new demand, got {}",
            spawner.spawned_count
        );
        assert!(stations.len() + industries.len() > initial_stations + initial_industries);
        assert!(
            talk.iter().any(|e| e.kind == TalkKind::Opportunity),
            "Town Talk should announce new demand"
        );

        // Every open opportunity must sit outside meaningful service coverage.
        for opp in &spawner.open {
            let influence = super::super::sites::service_influence_at(opp.tile, stations, service);
            assert!(
                influence <= DEMAND_SERVICE_INFLUENCE_MAX + 0.01,
                "opportunity {} influence {influence} too high at {:?}",
                opp.name,
                opp.tile
            );
        }

        // The board cap is on what stands unanswered, not on the session.
        assert!(
            spawner.open.len() <= spawner.max_pending as usize,
            "{} markers standing against a cap of {}",
            spawner.open.len(),
            spawner.max_pending
        );
    }

    /// **The world never runs out of things to ask for**, however full the map
    /// gets. Brief 08 §4: a player who has connected everything has finished,
    /// and there is no hour-long arc.
    #[test]
    fn a_crowded_map_still_produces_opportunities_rather_than_going_quiet() {
        let mut app = App::new();
        app.add_plugins(SimPlugin);
        app.world_mut().insert_resource(land_terrain(64, 64));
        {
            let mut stations = StationRegistry::new();
            let mut industries = IndustryRegistry::new();
            let mut service = StationService::default();
            seed_stations_and_industries(
                &mut stations,
                &mut industries,
                &mut service,
                64,
                64,
                |_| true,
            );
            let world = app.world_mut();
            *world.resource_mut::<StationRegistry>() = stations;
            *world.resource_mut::<IndustryRegistry>() = industries;
            *world.resource_mut::<StationService>() = service;
        }
        app.world_mut()
            .insert_resource(crate::WorldAnchorsSeeded(true));
        {
            let mut spawner = app.world_mut().resource_mut::<DemandSpawner>();
            spawner.ticks_until_next = 0;
            spawner.interval_ticks = 1;
            // Far past where a 64² map can honour the hard floor.
            spawner.spawned_count = 40;
        }

        // Answer every opportunity the moment it appears, forty times over.
        for _ in 0..40 {
            let mut spawned = false;
            for _ in 0..50 {
                app.world_mut().run_schedule(FixedUpdate);
                let mut spawner = app.world_mut().resource_mut::<DemandSpawner>();
                if !spawner.open.is_empty() {
                    spawner.open.clear();
                    spawned = true;
                    break;
                }
            }
            assert!(
                spawned,
                "the world went quiet after {} opportunities — a grown spacing \
                 floor must pull outward, not gate",
                app.world().resource::<DemandSpawner>().spawned_count
            );
        }
        assert!(app.world().resource::<DemandSpawner>().spawned_count >= 80);
    }

    /// The pending cap holds the noise down, and connecting one frees the slot
    /// — the world is never done, but it is never wallpaper either.
    #[test]
    fn the_board_fills_to_its_cap_and_then_waits_for_the_player() {
        let mut app = App::new();
        app.add_plugins(SimPlugin);
        app.world_mut().insert_resource(land_terrain(64, 64));
        {
            let mut stations = StationRegistry::new();
            let mut industries = IndustryRegistry::new();
            let mut service = StationService::default();
            seed_stations_and_industries(
                &mut stations,
                &mut industries,
                &mut service,
                64,
                64,
                |_| true,
            );
            let world = app.world_mut();
            *world.resource_mut::<StationRegistry>() = stations;
            *world.resource_mut::<IndustryRegistry>() = industries;
            *world.resource_mut::<StationService>() = service;
        }
        app.world_mut()
            .insert_resource(crate::WorldAnchorsSeeded(true));
        {
            let mut spawner = app.world_mut().resource_mut::<DemandSpawner>();
            spawner.ticks_until_next = 0;
            spawner.interval_ticks = 1;
            spawner.min_spacing = 5;
        }

        for _ in 0..400 {
            app.world_mut().run_schedule(FixedUpdate);
        }
        let spawner = app.world().resource::<DemandSpawner>();
        assert_eq!(
            spawner.open.len(),
            DEMAND_MAX_PENDING,
            "the board should fill to its cap"
        );
        let spawned = spawner.spawned_count;
        assert!(spawned >= DEMAND_MAX_PENDING as u32);

        // Ignoring the world does not bury the player in markers…
        for _ in 0..400 {
            app.world_mut().run_schedule(FixedUpdate);
        }
        assert_eq!(
            app.world().resource::<DemandSpawner>().spawned_count,
            spawned,
            "a full board must stop producing, not keep piling up"
        );

        // …and answering one immediately makes room for the next, because the
        // clock kept running while the board was full.
        {
            let mut spawner = app.world_mut().resource_mut::<DemandSpawner>();
            spawner.open.remove(0);
        }
        app.world_mut().run_schedule(FixedUpdate);
        assert_eq!(
            app.world().resource::<DemandSpawner>().spawned_count,
            spawned + 1,
            "connecting one opportunity should let the next arrive"
        );
    }

    /// A new settlement becomes work the moment the player reaches it — and not
    /// one tick before.
    ///
    /// Both halves matter, and the second one is why the opening beat was
    /// unplayable. An opportunity is *unconnected by definition*, so posting
    /// jobs to it puts demand on the board that no train can ever clear; the
    /// board is a fixed-size queue with no expiry, and within a couple of real
    /// minutes every slot held a run between villages with no track while the
    /// one line the player had built could not get a fare posted. See
    /// `rail_sim/tests/economy_cold_start.rs` for the measurement.
    #[test]
    fn a_new_settlement_becomes_work_once_the_player_connects_it() {
        let mut app = App::new();
        app.add_plugins(SimPlugin);
        let terrain = land_terrain(32, 32);
        app.world_mut().insert_resource(terrain.clone());
        {
            let mut stations = StationRegistry::new();
            let mut industries = IndustryRegistry::new();
            let mut service = StationService::default();
            seed_stations_and_industries(
                &mut stations,
                &mut industries,
                &mut service,
                32,
                32,
                |_| true,
            );
            let world = app.world_mut();
            *world.resource_mut::<StationRegistry>() = stations;
            *world.resource_mut::<IndustryRegistry>() = industries;
            *world.resource_mut::<StationService>() = service;
        }
        // Force a settlement spawn.
        {
            let mut spawner = app.world_mut().resource_mut::<DemandSpawner>();
            spawner.ready_now();
            spawner.next_is_settlement = true;
            spawner.interval_ticks = 1000;
        }
        app.world_mut().run_schedule(FixedUpdate);
        let station_count = app.world().resource::<StationRegistry>().len();
        assert!(station_count >= 4, "new settlement should exist");
        let max_seed_id = 3u64;

        let newcomer = {
            let stations = app.world().resource::<StationRegistry>();
            stations
                .iter()
                .filter(|s| s.id.0 > max_seed_id)
                .min_by_key(|s| s.id.0)
                .map(|s| (s.id, s.tile))
                .expect("the settlement that just appeared")
        };

        // Force frequent job waves so every ordered pair comes round.
        let spin = |app: &mut App, waves: u32| {
            for _ in 0..waves {
                {
                    let mut board = app.world_mut().resource_mut::<JobBoard>();
                    board.spawn_cooldown = 10_000; // trip spawn_demand_jobs gate
                }
                app.world_mut().run_schedule(FixedUpdate);
            }
        };
        let mentions_newcomer = |app: &App| {
            app.world()
                .resource::<JobBoard>()
                .jobs
                .iter()
                .any(|j| match &j.kind {
                    crate::economy::JobKind::Passenger { from, to } => {
                        *from == newcomer.0 || *to == newcomer.0
                    }
                    _ => false,
                })
        };

        spin(&mut app, 60);
        assert!(
            !mentions_newcomer(&app),
            "an unconnected settlement must not hold a board slot no train can \
             clear; board={:?}",
            app.world().resource::<JobBoard>().jobs
        );

        // The player runs a line from the nearest seeded anchor to the newcomer.
        let anchor = {
            let stations = app.world().resource::<StationRegistry>();
            stations
                .iter()
                .filter(|s| s.id.0 <= max_seed_id)
                .min_by_key(|s| {
                    (
                        (s.tile.x - newcomer.1.x).abs() + (s.tile.y - newcomer.1.y).abs(),
                        s.id.0,
                    )
                })
                .map(|s| s.tile)
                .expect("a seeded anchor")
        };
        app.world_mut()
            .resource_scope(|world, mut network: Mut<crate::track::TrackNetwork>| {
                world.resource_scope(|world, mut money: Mut<crate::money::Money>| {
                    world.resource_scope(|_w, mut ledger: Mut<crate::economy::MoneyLedger>| {
                        *money = crate::money::Money::new(100_000_000);
                        let mut cur = anchor;
                        while cur != newcomer.1 {
                            let _ = crate::track::try_place_track(
                                &mut network,
                                &mut money,
                                &mut ledger,
                                &terrain,
                                cur,
                                GROUND_LAYER,
                            );
                            cur.x += (newcomer.1.x - cur.x).signum();
                            cur.y += (newcomer.1.y - cur.y).signum();
                        }
                        let _ = crate::track::try_place_track(
                            &mut network,
                            &mut money,
                            &mut ledger,
                            &terrain,
                            newcomer.1,
                            GROUND_LAYER,
                        );
                    });
                });
            });

        spin(&mut app, 60);
        assert!(
            mentions_newcomer(&app),
            "a connected settlement must produce fares; board={:?}",
            app.world().resource::<JobBoard>().jobs
        );
    }
}
