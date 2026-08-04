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

/// Ticks with no arrival before a stop's reputation starts to slide.
///
/// A tick is [`SIM_SECONDS_PER_TICK`](crate::peeps::SIM_SECONDS_PER_TICK) = 10
/// sim-seconds, so this is twenty sim-minutes: long enough that a stop between
/// two calls is not "neglected", short enough that a branch nobody runs is.
pub const SCORE_IDLE_GRACE_TICKS: u64 = 120;

/// Ticks between the single points a neglected stop loses.
///
/// # Why this is not one point per tick
///
/// It was, and that quietly broke the game's third pillar. An arrival banks
/// [`StationTierSpec::arrival_gain`](super::tier::StationTierSpec) — 8 for the
/// workhorse tier — and a point a tick spends that in eight ticks, so a stop
/// held a score only while trains arrived less than ~128 ticks apart. Nothing
/// in a real railway does: a lap of a three-stop line is several hundred ticks,
/// and only a *delivery* banks anything. Measured on a generated map with a
/// train running, every station on the network sat at score `0` for the whole
/// session — and `town::density_target_at` multiplies by `score / 100`, so
/// **no town anywhere ever grew**, however well it was served.
///
/// One point per minute of sim time makes the decay what its own comment always
/// said it was — *slow*. A stop with a full hundred fades to nothing after
/// about sixteen sim-hours with no train at all, which is neglect anybody would
/// call neglect, while a stop served every few minutes keeps what it earns.
pub const SCORE_DECAY_EVERY_TICKS: u64 = 60;

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

    /// A scheduled train called here and went on its way.
    ///
    /// Brief 06 §5 makes service reliability — *"do trains actually come?"* —
    /// the multiplier on growth, and a call is what "trains come" means. Only
    /// deliveries used to count, which measured throughput instead: a stop with
    /// a train every three sim-minutes all day read as **unserved** unless
    /// somebody happened to be riding to it, because a line train dead-heads to
    /// a job's origin and only banks score where it sets its passengers down.
    ///
    /// Worth half an arrival, rounded up, so a stop trains merely pass through
    /// keeps its lights on while a stop people actually travel to still earns
    /// faster. `deliveries` is untouched — that is a count of paid runs, and the
    /// Inspector and the goals board both read it as one.
    pub fn record_call(&mut self, id: StationId) {
        let tick = self.tick;
        let s = self.ensure(id);
        s.last_arrival_tick = tick;
        let gain = (u16::from(s.tier.arrival_gain()).div_ceil(2)).max(1);
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
        // Idle stations slowly lose score so neglected areas stagnate. One
        // shared beat rather than a per-station timer keeps this deterministic
        // and free of state a save would have to carry.
        if !self.tick.is_multiple_of(SCORE_DECAY_EVERY_TICKS) {
            return;
        }
        for s in self.scores.values_mut() {
            if self.tick.saturating_sub(s.last_arrival_tick) > SCORE_IDLE_GRACE_TICKS {
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

    /// The arithmetic the town pillar stands on: what one arrival banks has to
    /// outlast the gap until the next one, or `density_target_at` reads `0`
    /// forever and no town ever grows.
    #[test]
    fn a_stop_keeps_what_it_earns_between_trains() {
        let mut service = StationService::default();
        service.record_arrival(A);
        let banked = service.score(A).score;
        assert_eq!(banked, StationTier::Station.arrival_gain());

        // A realistic gap between calls on a short line.
        for _ in 0..300 {
            service.tick_decay();
        }
        assert!(
            service.score(A).score > 0,
            "a stop served every 300 ticks must not read as neglected"
        );

        // Neglect it for a good part of a sim day and it is forgotten.
        let mut service = StationService::default();
        service.ensure(A).score = 100;
        for _ in 0..(100 * SCORE_DECAY_EVERY_TICKS + SCORE_IDLE_GRACE_TICKS) {
            service.tick_decay();
        }
        assert_eq!(service.score(A).score, 0, "and neglect still costs the lot");
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
