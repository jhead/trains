//! Per-station service scores for the town / peeps slice.
//!
//! Town agent can read [`StationService`] each tick to drive growth rings and
//! wait-time complaints. This crate only updates the numbers; it does not spawn peeps.

use std::collections::HashMap;

use bevy_ecs::prelude::Resource;

use crate::ids::StationId;

/// Snapshot of how well a station is being served.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StationServiceScore {
    /// Passenger / goods arrivals completed at this station.
    pub deliveries: u32,
    /// Sim tick of the most recent arrival (`0` = never).
    pub last_arrival_tick: u64,
    /// Pending passenger jobs originating here (wait-time proxy).
    pub waiting_passengers: u32,
    /// Rough service quality `0..=100` (town may map this to growth).
    pub score: u8,
}

/// Authoritative service readout keyed by station.
///
/// Updated by economy systems on delivery / job spawn. Town systems should
/// treat this as read-only input.
#[derive(Debug, Clone, Default, Resource)]
pub struct StationService {
    pub scores: HashMap<StationId, StationServiceScore>,
    pub tick: u64,
}

impl StationService {
    pub fn score(&self, id: StationId) -> StationServiceScore {
        self.scores.get(&id).copied().unwrap_or_default()
    }

    pub fn ensure(&mut self, id: StationId) -> &mut StationServiceScore {
        self.scores.entry(id).or_default()
    }

    pub fn record_arrival(&mut self, id: StationId) {
        let tick = self.tick;
        let s = self.ensure(id);
        s.deliveries = s.deliveries.saturating_add(1);
        s.last_arrival_tick = tick;
        s.score = (s.score as u16 + 8).min(100) as u8;
    }

    pub fn set_waiting(&mut self, id: StationId, waiting: u32) {
        let s = self.ensure(id);
        s.waiting_passengers = waiting;
        // Waiting passengers pull the score down gently.
        let penalty = waiting.min(20) as u8;
        s.score = s.score.saturating_sub(penalty / 4);
    }

    pub fn tick_decay(&mut self) {
        self.tick = self.tick.saturating_add(1);
        for s in self.scores.values_mut() {
            // Idle stations slowly lose score so neglected areas stagnate.
            if self.tick.saturating_sub(s.last_arrival_tick) > 120 {
                s.score = s.score.saturating_sub(1);
            }
        }
    }
}
