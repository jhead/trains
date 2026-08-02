//! Periodic spawn of new settlements / industries outside served coverage.

use bevy_ecs::prelude::*;

use crate::ids::{StationId, TileCoord};
use crate::peeps::{ComplaintEntry, ComplaintFeed, TalkKind, SIM_SECONDS_PER_TICK};
use crate::stations::{
    GoodKind, IndustryId, IndustryRegistry, StationRegistry, StationService,
};
use crate::track::{TrackNetwork, TrackTerrain, GROUND_LAYER};
use crate::trains::track_for_station;

use super::sites::pick_demand_site;

/// Felt rhythm: first new demand after this many sim-minutes.
pub const DEMAND_FIRST_DELAY_SIM_MINUTES: u32 = 8;
/// Subsequent opportunities every this many sim-minutes.
pub const DEMAND_INTERVAL_SIM_MINUTES: u32 = 4;
/// Cap on anchors revealed after the initial seed (stations + industries).
pub const DEMAND_MAX_NEW_PER_SESSION: u32 = 8;
/// Minimum Manhattan spacing from existing anchors.
pub const DEMAND_MIN_ANCHOR_SPACING: i32 = 8;
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

const INDUSTRY_SPECS: &[(&str, Option<GoodKind>, Option<GoodKind>)] = &[
    ("Quarry Ridge", Some(GoodKind::Ore), None),
    ("Harbor Foundry", None, Some(GoodKind::Ore)),
    ("Cedar Yard", Some(GoodKind::Lumber), None),
    ("Builders' Wharf", None, Some(GoodKind::Lumber)),
    ("High Quarry", Some(GoodKind::Ore), None),
    ("Mill End", None, Some(GoodKind::Ore)),
];

fn minutes_to_ticks(minutes: u32) -> u32 {
    let secs = minutes.saturating_mul(60);
    secs.saturating_add(SIM_SECONDS_PER_TICK.saturating_sub(1)) / SIM_SECONDS_PER_TICK
}

/// What kind of new demand was revealed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemandOpportunityKind {
    Settlement(StationId),
    Industry(IndustryId),
}

/// An open opportunity until the player connects it (or dismisses the alert).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemandOpportunity {
    pub kind: DemandOpportunityKind,
    pub name: String,
    pub tile: TileCoord,
}

/// Tunable session state for new-demand spawning.
#[derive(Debug, Clone, Resource)]
pub struct DemandSpawner {
    /// Ticks remaining until the next spawn attempt.
    pub ticks_until_next: u32,
    /// How many new anchors this session has revealed.
    pub spawned_count: u32,
    /// Session cap (stations + industries combined).
    pub max_new: u32,
    /// Interval between spawns after the first (ticks).
    pub interval_ticks: u32,
    /// Minimum spacing from existing anchors.
    pub min_spacing: i32,
    /// Max service influence allowed at a candidate tile.
    pub max_influence: f32,
    /// Open opportunities (for alerts / presentation markers).
    pub open: Vec<DemandOpportunity>,
    /// Settlement name index into [`SETTLEMENT_NAMES`].
    pub next_settlement: usize,
    /// Industry spec index into [`INDUSTRY_SPECS`].
    pub next_industry: usize,
    /// Alternate settlement / industry.
    pub next_is_settlement: bool,
}

impl Default for DemandSpawner {
    fn default() -> Self {
        Self {
            ticks_until_next: minutes_to_ticks(DEMAND_FIRST_DELAY_SIM_MINUTES),
            spawned_count: 0,
            max_new: DEMAND_MAX_NEW_PER_SESSION,
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

    if spawner.spawned_count >= spawner.max_new {
        return;
    }
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

    // Prefer farther sites as more opportunities land (pull outward).
    let prefer_far = DEMAND_MIN_ANCHOR_SPACING
        + (spawner.spawned_count as i32).saturating_mul(4);

    let Some(tile) = pick_demand_site(
        &terrain,
        &stations,
        &industries,
        &service,
        spawner.min_spacing,
        prefer_far,
        spawner.max_influence,
    ) else {
        // Retry next interval rather than spinning every tick.
        spawner.ticks_until_next = spawner.interval_ticks.max(1);
        return;
    };

    let tick = service.tick;
    let spawn_settlement = spawner.next_is_settlement;

    if spawn_settlement {
        let name = SETTLEMENT_NAMES[spawner.next_settlement % SETTLEMENT_NAMES.len()];
        spawner.next_settlement = spawner.next_settlement.saturating_add(1);
        let id = stations.insert(name, tile, GROUND_LAYER);
        service.ensure(id);
        let message = format!("New settlement: {name} — not yet served");
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
        let (name, produces, consumes) =
            INDUSTRY_SPECS[spawner.next_industry % INDUSTRY_SPECS.len()];
        spawner.next_industry = spawner.next_industry.saturating_add(1);
        let id = industries.insert(name, tile, produces, consumes);
        let label = if produces.is_some() {
            format!("New industry: {name} — not yet served")
        } else {
            format!("New mill: {name} — not yet served")
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
    spawner.ticks_until_next = spawner.interval_ticks.max(1);
}

#[cfg(test)]
mod tests {
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
        assert_eq!(s.max_new, DEMAND_MAX_NEW_PER_SESSION);
        // 8 sim-min ≈ 48 ticks at 10s/tick; 4 min ≈ 24 ticks.
        assert_eq!(s.ticks_until_next, 48);
        assert_eq!(s.interval_ticks, 24);
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
            spawner.max_new = 4;
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

        // Cap must hold.
        assert!(spawner.spawned_count <= spawner.max_new);
    }

    #[test]
    fn jobs_board_picks_up_new_stations() {
        let mut app = App::new();
        app.add_plugins(SimPlugin);
        app.world_mut().insert_resource(land_terrain(32, 32));
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

        // Force frequent job waves so the A→B cycle covers the new station.
        for _ in 0..40 {
            {
                let mut board = app.world_mut().resource_mut::<JobBoard>();
                board.spawn_cooldown = 10_000; // trip spawn_demand_jobs gate
            }
            app.world_mut().run_schedule(FixedUpdate);
        }
        let board = app.world().resource::<JobBoard>();
        let has_new_endpoint = board.jobs.iter().any(|j| match &j.kind {
            crate::economy::JobKind::Passenger { from, to } => {
                from.0 > max_seed_id || to.0 > max_seed_id
            }
            _ => false,
        });
        assert!(
            has_new_endpoint,
            "passenger jobs should eventually include new stations; board={:?}",
            board.jobs
        );
    }
}
