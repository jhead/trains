//! District-level flow — the abstracted majority.
//!
//! Brief 06 §4.1: *"The abstracted majority still produce demand, service
//! pressure and complaints; they simply don't have sprites."*
//!
//! Abstracted peeps keep their name, household, routine and memory — only the
//! walking is dropped. They collapse to three coarse stages (home, platform,
//! away), and their pressure lands in [`DistrictFlow`], which the Inspector and
//! overlays can read as a per-station readout.

use std::collections::HashMap;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::ids::{StationId, TileCoord};
use crate::stations::{StationRegistry, StationService};

use super::budget::PeepDetail;
use super::complaints::{gave_up_minutes, ComplaintEntry, ComplaintFeed, TalkKind};
use super::household::HouseholdRegistry;
use super::journey::{finish_leg, Journey, JourneyLeg, JourneyStage};
use super::memory::JourneyMemory;
use super::resident::{Peep, WaitingAtStation, SIM_SECONDS_PER_TICK};
use super::routine::Routine;
use super::{day_index, minute_of_day};

/// Ticks between decays of the rolling flow counters.
pub const FLOW_DECAY_TICKS: u32 = 600;

/// Minimum service score at which an abstracted district counts as served.
pub const ABSTRACT_SERVICE_MIN: u8 = 20;

/// How recently a station must have seen an arrival to count as served (ticks).
pub const ABSTRACT_ARRIVAL_WINDOW_TICKS: u64 = 240;

/// Ticks per tile used to estimate an abstracted ride (transit pace).
pub const ABSTRACT_RIDE_TICKS_PER_TILE: u32 = 3;

/// Cap on queued trip requests, so an undrained queue cannot grow without bound.
///
/// The queue is the peeps slice's **demand surface**: every departure — full
/// detail or abstracted — lands here as a wanted `from → to` trip. Draining it
/// into passenger jobs is one line in `economy`, which peeps does not own; see
/// [`DistrictFlow::take_pending`].
pub const MAX_PENDING_TRIPS: usize = 32;

/// Per-station flow readout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistrictFlowState {
    /// Residents whose home station is this one.
    pub residents: u32,
    /// Peeps standing on this platform right now (full + abstract).
    pub waiting: u32,
    /// Abstracted residents attached to this district.
    pub abstracted: u32,
    /// Trips started here recently.
    pub departures: u32,
    /// Legs finished here recently.
    pub completed: u32,
    /// People who gave up here recently.
    pub gave_up: u32,
    /// Accumulated unserved waiting, in sim-seconds — the service pressure.
    pub pressure_secs: u32,
}

impl DistrictFlowState {
    /// Rough `0..=100` pressure reading for overlays.
    pub fn pressure_score(&self) -> u8 {
        let from_wait = (self.pressure_secs / 60).min(60) as u32;
        let from_walkers = self.gave_up.saturating_mul(8);
        (from_wait + from_walkers).min(100) as u8
    }

    /// Plain-language district line — *"Eastgate: 14 waiting, 3 walked."*
    pub fn describe(&self, station_name: &str) -> String {
        if self.waiting == 0 && self.gave_up == 0 {
            return format!("{station_name}: {} residents, quiet.", self.residents);
        }
        format!(
            "{station_name}: {} waiting, {} gave up and walked.",
            self.waiting, self.gave_up
        )
    }
}

/// Aggregated demand and pressure per district (station catchment).
#[derive(Debug, Clone, Default, Resource)]
pub struct DistrictFlow {
    districts: HashMap<StationId, DistrictFlowState>,
    /// Trips peeps want to make, drained into the job board.
    pending: Vec<(StationId, StationId)>,
    decay_ticks: u32,
}

impl DistrictFlow {
    pub fn get(&self, id: StationId) -> DistrictFlowState {
        self.districts.get(&id).copied().unwrap_or_default()
    }

    pub fn entry(&mut self, id: StationId) -> &mut DistrictFlowState {
        self.districts.entry(id).or_default()
    }

    pub fn iter(&self) -> impl Iterator<Item = (StationId, &DistrictFlowState)> {
        self.districts.iter().map(|(id, s)| (*id, s))
    }

    pub fn pressure(&self, id: StationId) -> u8 {
        self.get(id).pressure_score()
    }

    /// District under the most pressure — the next thing worth fixing.
    pub fn worst(&self) -> Option<(StationId, u8)> {
        self.districts
            .iter()
            .map(|(id, s)| (*id, s.pressure_score()))
            .max_by_key(|(id, p)| (*p, id.0))
            .filter(|(_, p)| *p > 0)
    }

    /// Total residents across every district.
    pub fn population(&self) -> u32 {
        self.districts.values().map(|s| s.residents).sum()
    }

    /// A peep wants to travel `from` → `to`. Deduplicated, oldest evicted at cap
    /// so an undrained queue still reads as *current* demand rather than history.
    pub fn request_trip(&mut self, from: StationId, to: StationId) {
        if from == to || self.pending.contains(&(from, to)) {
            return;
        }
        if self.pending.len() >= MAX_PENDING_TRIPS {
            self.pending.remove(0);
        }
        self.pending.push((from, to));
    }

    pub fn pending_trips(&self) -> &[(StationId, StationId)] {
        &self.pending
    }

    pub fn take_pending(&mut self) -> Vec<(StationId, StationId)> {
        std::mem::take(&mut self.pending)
    }

    /// Reset the per-tick live counts; rolling counters decay slowly.
    pub fn begin_tick(&mut self) {
        for s in self.districts.values_mut() {
            s.waiting = 0;
            s.abstracted = 0;
            s.residents = 0;
        }
        self.decay_ticks = self.decay_ticks.saturating_add(1);
        if self.decay_ticks >= FLOW_DECAY_TICKS {
            self.decay_ticks = 0;
            for s in self.districts.values_mut() {
                s.departures /= 2;
                s.completed /= 2;
                s.gave_up /= 2;
                s.pressure_secs /= 2;
            }
        }
    }
}

/// Recount residents per district and clear the per-tick live counts.
pub fn begin_flow_window(households: Res<HouseholdRegistry>, mut flow: ResMut<DistrictFlow>) {
    flow.begin_tick();
    for household in households.iter() {
        flow.entry(household.home_station).residents += household.members.len() as u32;
    }
}

/// True when a district currently has usable rail service.
pub fn district_is_served(service: &StationService, station: StationId, tick: u64) -> bool {
    let score = service.score(station);
    score.score >= ABSTRACT_SERVICE_MIN
        && score.last_arrival_tick > 0
        && tick.saturating_sub(score.last_arrival_tick) <= ABSTRACT_ARRIVAL_WINDOW_TICKS
}

/// Estimated ticks for an abstracted ride between two stations.
pub fn abstract_ride_ticks(from: TileCoord, to: TileCoord) -> u32 {
    let dist = (from.x - to.x)
        .abs()
        .max((from.y - to.y).abs())
        .unsigned_abs();
    dist.saturating_mul(ABSTRACT_RIDE_TICKS_PER_TILE).max(1)
}

/// Advance abstracted peeps through the coarse stage set.
///
/// Walking, boarding, riding and alighting all fold into one platform stage:
/// off camera the only thing that matters is *did service carry them, and how
/// long did it take*. The verdict still runs through [`finish_leg`], so an
/// abstracted commute is graded by the same rule as a fully simulated one.
pub fn advance_abstract_flow(
    stations: Res<StationRegistry>,
    service: Res<StationService>,
    mut flow: ResMut<DistrictFlow>,
    mut feed: ResMut<ComplaintFeed>,
    mut peeps: Query<(
        &Peep,
        &Routine,
        &mut Journey,
        &mut WaitingAtStation,
        &mut JourneyMemory,
        &PeepDetail,
    )>,
) {
    let tick = service.tick;
    let minute = minute_of_day(tick);
    let today = day_index(tick);

    for (peep, routine, mut journey, mut waiting, mut memory, detail) in peeps.iter_mut() {
        if detail.is_full() {
            continue;
        }
        journey.coarsen();
        journey.stage_ticks = journey.stage_ticks.saturating_add(1);
        if journey.stage.is_travelling() {
            journey.leg_secs = journey.leg_secs.saturating_add(SIM_SECONDS_PER_TICK);
        }
        flow.entry(routine.home_station).abstracted += 1;

        match journey.stage {
            JourneyStage::AtHome => {
                let already_went = journey.last_depart_day == Some(today);
                if !already_went && routine.is_departure_time(minute) {
                    journey.last_depart_day = Some(today);
                    journey.begin_leg(
                        JourneyLeg::Outbound,
                        routine.home_station,
                        routine.destination_station,
                    );
                    waiting.station = routine.home_station;
                    waiting.wait_secs = 0;
                    journey.set_stage(JourneyStage::WaitingOnPlatform);
                    flow.entry(routine.home_station).departures += 1;
                    flow.request_trip(routine.home_station, routine.destination_station);
                }
            }

            JourneyStage::WaitingOnPlatform => {
                let station = journey.from_station;
                {
                    let district = flow.entry(station);
                    district.waiting += 1;
                    district.pressure_secs = district
                        .pressure_secs
                        .saturating_add(SIM_SECONDS_PER_TICK.min(waiting.wait_secs));
                }

                let same_district = journey.from_station == journey.to_station;
                let ride_ticks =
                    ride_ticks_between(&stations, journey.from_station, journey.to_station);
                let served = same_district || district_is_served(&service, station, tick);
                let patience = memory.patience_secs();

                if served && journey.stage_ticks >= ride_ticks {
                    journey.leg_wait_secs = waiting.wait_secs;
                    finish_leg(
                        &mut journey,
                        &mut memory,
                        &mut waiting,
                        &mut flow,
                        routine,
                        tick,
                    );
                } else if waiting.wait_secs >= patience {
                    journey.gave_up = true;
                    journey.leg_wait_secs = waiting.wait_secs;
                    let station_name = stations
                        .get(station)
                        .map(|s| s.name.clone())
                        .unwrap_or_default();
                    flow.entry(station).gave_up += 1;
                    feed.push(ComplaintEntry {
                        kind: TalkKind::Complaint,
                        peep_name: peep.given_name().to_string(),
                        station_name,
                        wait_minutes: gave_up_minutes(waiting.wait_minutes()),
                        sim_tick: tick,
                        peep_id: Some(peep.id),
                        station_id: Some(station),
                        tile: stations.get(station).map(|s| s.tile),
                        count: 1,
                    });
                    finish_leg(
                        &mut journey,
                        &mut memory,
                        &mut waiting,
                        &mut flow,
                        routine,
                        tick,
                    );
                }
            }

            JourneyStage::SpendingTime => {
                let stay_ticks = routine.stay_minutes.saturating_mul(60) / SIM_SECONDS_PER_TICK;
                if journey.stage_ticks >= stay_ticks.max(1) {
                    journey.begin_leg(
                        JourneyLeg::Return,
                        routine.destination_station,
                        routine.home_station,
                    );
                    waiting.station = routine.destination_station;
                    waiting.wait_secs = 0;
                    journey.set_stage(JourneyStage::WaitingOnPlatform);
                    flow.entry(routine.destination_station).departures += 1;
                    flow.request_trip(routine.destination_station, routine.home_station);
                }
            }

            // Coarsening guarantees the fine stages never reach here.
            _ => journey.coarsen(),
        }
    }
}

fn ride_ticks_between(stations: &StationRegistry, from: StationId, to: StationId) -> u32 {
    match (stations.get(from), stations.get(to)) {
        (Some(a), Some(b)) => abstract_ride_ticks(a.tile, b.tile),
        _ => ABSTRACT_RIDE_TICKS_PER_TILE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stations::StationServiceScore;

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    #[test]
    fn flow_tracks_pressure_and_names_the_worst_district() {
        let mut flow = DistrictFlow::default();
        flow.entry(StationId(1)).pressure_secs = 30 * 60;
        flow.entry(StationId(1)).gave_up = 3;
        flow.entry(StationId(2)).pressure_secs = 60;
        let (worst, score) = flow.worst().unwrap();
        assert_eq!(worst, StationId(1));
        assert!(score > flow.pressure(StationId(2)));
    }

    #[test]
    fn begin_tick_clears_live_counts_but_keeps_history() {
        let mut flow = DistrictFlow::default();
        flow.entry(StationId(1)).waiting = 9;
        flow.entry(StationId(1)).gave_up = 4;
        flow.begin_tick();
        assert_eq!(flow.get(StationId(1)).waiting, 0);
        assert_eq!(flow.get(StationId(1)).gave_up, 4);
    }

    #[test]
    fn pending_trips_dedupe_and_cap() {
        let mut flow = DistrictFlow::default();
        for _ in 0..4 {
            flow.request_trip(StationId(1), StationId(2));
        }
        flow.request_trip(StationId(2), StationId(2)); // same station: not a trip
        assert_eq!(flow.pending_trips().len(), 1);
        for i in 0..100u64 {
            flow.request_trip(StationId(i + 10), StationId(i + 200));
        }
        assert_eq!(flow.pending_trips().len(), MAX_PENDING_TRIPS);
        // Oldest evicted — the queue reads as current demand, not history.
        assert!(!flow.pending_trips().contains(&(StationId(1), StationId(2))));
    }

    #[test]
    fn demand_surface_drains_once() {
        let mut flow = DistrictFlow::default();
        flow.request_trip(StationId(1), StationId(2));
        flow.request_trip(StationId(2), StationId(3));
        let taken = flow.take_pending();
        assert_eq!(taken.len(), 2);
        assert!(flow.pending_trips().is_empty());
    }

    #[test]
    fn service_window_decides_whether_a_district_is_served() {
        let mut service = StationService::default();
        service.tick = 500;
        service.scores.insert(
            StationId(1),
            StationServiceScore {
                score: 60,
                last_arrival_tick: 400,
                ..Default::default()
            },
        );
        assert!(district_is_served(&service, StationId(1), 500));
        // Stale arrival — nobody is coming.
        assert!(!district_is_served(&service, StationId(1), 5_000));
        // Never served at all.
        assert!(!district_is_served(&service, StationId(2), 500));
    }

    #[test]
    fn ride_estimate_scales_with_distance() {
        let near = abstract_ride_ticks(tile(0, 0), tile(2, 0));
        let far = abstract_ride_ticks(tile(0, 0), tile(20, 0));
        assert!(far > near);
        assert!(near >= 1);
    }
}
