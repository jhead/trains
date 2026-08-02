//! Per-station service scores for the town / peeps slice.
//!
//! Town agent can read [`StationService`] each tick to drive growth rings and
//! wait-time complaints. This crate only updates the numbers; it does not spawn peeps.
//!
//! Scores are **tier-aware**: each entry caches the stop's
//! [`StationTier`], so a bigger platform banks more reputation per arrival and
//! shrugs off a queue that would swamp a halt. Placement keeps the cache honest
//! via [`StationService::set_tier`]; writers elsewhere need no changes.

use std::collections::HashMap;

use bevy_ecs::prelude::Resource;

use crate::ids::StationId;

use super::tier::StationTier;

/// Crowding percentage that costs a full point of score.
const CROWD_PER_PENALTY: u32 = 20;

/// Hardest crowding penalty a single sample may apply.
const MAX_CROWD_PENALTY: u8 = 5;

/// Snapshot of how well a station is being served.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StationServiceScore {
    /// Passenger / goods arrivals completed at this station.
    pub deliveries: u32,
    /// Sim tick of the most recent arrival (`0` = never).
    pub last_arrival_tick: u64,
    /// Pending passenger jobs originating here (wait-time proxy).
    pub waiting_passengers: u32,
    /// Named peeps actually standing on the platform.
    ///
    /// Tracked separately from [`Self::waiting_passengers`] because the two have
    /// different writers on different cadences — the job board rewrites its
    /// count every tick, and blending at read time keeps either writer from
    /// clobbering the other or double-charging the score.
    pub peep_waiting: u32,
    /// Rough service quality `0..=100` (town may map this to growth).
    pub score: u8,
    /// Platform grade of the stop being scored.
    pub tier: StationTier,
}

impl StationServiceScore {
    /// Everyone on the platform: pending jobs plus named peeps.
    pub fn total_waiting(&self) -> u32 {
        self.waiting_passengers.saturating_add(self.peep_waiting)
    }

    /// Waiting peeps as a percentage of the tier's platform capacity.
    pub fn crowding_percent(&self) -> u32 {
        let capacity = self.tier.capacity().max(1);
        self.total_waiting().saturating_mul(100) / capacity
    }

    /// True when the queue has outgrown the platform.
    pub fn is_overcrowded(&self) -> bool {
        self.total_waiting() > self.tier.capacity()
    }
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

    /// Cached platform grade for `id` (default tier when unknown).
    pub fn tier(&self, id: StationId) -> StationTier {
        self.score(id).tier
    }

    /// Point the score entry at a stop's tier — call on place and on upgrade.
    pub fn set_tier(&mut self, id: StationId, tier: StationTier) {
        self.ensure(id).tier = tier;
    }

    /// Drop a demolished stop so it stops counting toward growth / alerts.
    pub fn forget(&mut self, id: StationId) -> Option<StationServiceScore> {
        self.scores.remove(&id)
    }

    pub fn record_arrival(&mut self, id: StationId) {
        let tick = self.tick;
        let s = self.ensure(id);
        s.deliveries = s.deliveries.saturating_add(1);
        s.last_arrival_tick = tick;
        // Bigger platforms turn the same arrival into more reputation.
        let gain = s.tier.arrival_gain() as u16;
        s.score = (s.score as u16 + gain).min(100) as u8;
    }

    /// Set the job-board queue and charge the tick's crowding penalty.
    ///
    /// The penalty reads [`StationServiceScore::crowding_percent`], which blends
    /// in [`Self::set_peep_waiting`]'s count — so run this *after* the peep
    /// count each tick and the queue is charged exactly once, from both sources.
    pub fn set_waiting(&mut self, id: StationId, waiting: u32) {
        let s = self.ensure(id);
        s.waiting_passengers = waiting;
        // A queue only hurts relative to the platform it is standing on.
        let penalty = (s.crowding_percent() / CROWD_PER_PENALTY).min(MAX_CROWD_PENALTY as u32);
        s.score = s.score.saturating_sub(penalty as u8);
    }

    /// Record named peeps standing on the platform.
    ///
    /// Deliberately a pure setter: [`Self::set_waiting`] owns the once-per-tick
    /// penalty for the blended total, so charging here too would double-count.
    pub fn set_peep_waiting(&mut self, id: StationId, waiting: u32) {
        self.ensure(id).peep_waiting = waiting;
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

#[cfg(test)]
mod tests {
    use super::*;

    const A: StationId = StationId(1);

    #[test]
    fn bigger_tiers_bank_more_score_per_arrival() {
        let mut service = StationService::default();
        service.set_tier(A, StationTier::Halt);
        service.record_arrival(A);
        let halt = service.score(A).score;

        let mut service = StationService::default();
        service.set_tier(A, StationTier::Interchange);
        service.record_arrival(A);
        let interchange = service.score(A).score;

        assert!(
            interchange > halt,
            "interchange {interchange} should out-score halt {halt} per arrival"
        );
        assert_eq!(halt, StationTier::Halt.arrival_gain());
    }

    #[test]
    fn default_tier_keeps_the_pre_tier_arrival_gain() {
        let mut service = StationService::default();
        service.record_arrival(A);
        assert_eq!(service.score(A).score, 8);
        assert_eq!(service.score(A).tier, StationTier::Station);
    }

    #[test]
    fn the_same_queue_swamps_a_halt_and_not_an_interchange() {
        let waiting = 8;

        let mut service = StationService::default();
        service.set_tier(A, StationTier::Halt);
        service.ensure(A).score = 60;
        service.set_waiting(A, waiting);
        let halt = service.score(A);

        let mut service = StationService::default();
        service.set_tier(A, StationTier::Interchange);
        service.ensure(A).score = 60;
        service.set_waiting(A, waiting);
        let interchange = service.score(A);

        assert!(halt.is_overcrowded(), "8 waiting overflows a 6-capacity halt");
        assert!(!interchange.is_overcrowded());
        assert!(
            halt.score < interchange.score,
            "halt {} should be punished harder than interchange {}",
            halt.score,
            interchange.score
        );
    }

    #[test]
    fn forget_drops_a_demolished_stop() {
        let mut service = StationService::default();
        service.set_tier(A, StationTier::Halt);
        assert_eq!(service.tier(A), StationTier::Halt);
        assert!(service.forget(A).is_some());
        assert!(service.scores.get(&A).is_none());
        // Unknown stops read as the default tier rather than panicking.
        assert_eq!(service.tier(A), StationTier::Station);
    }
}
