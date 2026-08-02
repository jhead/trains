//! Journey memory — what a peep remembers, and how it shapes their patience.
//!
//! Brief 06 §4.2: *"a peep remembers recent journeys, and their patience is
//! shaped by their history. Someone who has had four good commutes tolerates a
//! bad one; someone who has had four bad ones leaves."*
//!
//! The memory is a short newest-first ring so it stays cheap and serialisable —
//! a saved town keeps everyone's history, which is what makes reloading feel
//! continuous rather than like a fresh start.

use std::collections::VecDeque;

use bevy_ecs::prelude::Component;
use serde::{Deserialize, Serialize};

use crate::ids::StationId;

use super::complaints::COMPLAINT_WAIT_SECS;

/// How many journeys a peep keeps in mind.
pub const MEMORY_DEPTH: usize = 6;

/// Patience with no history at all — the same threshold a complaint uses.
pub const BASE_PATIENCE_SECS: u32 = COMPLAINT_WAIT_SECS;

/// Patience floor — even a thoroughly fed-up peep waits this long.
pub const MIN_PATIENCE_SECS: u32 = 3 * 60;

/// Patience ceiling — goodwill is not infinite.
pub const MAX_PATIENCE_SECS: u32 = 24 * 60;

/// Sim-seconds a leg may take before it counts as *slow* rather than *good*.
pub const GOOD_JOURNEY_SECS: u32 = 6 * 60;

/// Consecutive bad journeys after which a peep wants out (brief: four).
pub const BAD_JOURNEYS_TO_LEAVE: u32 = 4;

/// How a finished leg felt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JourneyOutcome {
    /// Short wait, got there by train.
    Good,
    /// Got there, but the wait or the ride dragged.
    Slow,
    /// Gave up on the platform and walked.
    GaveUp,
}

impl JourneyOutcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Slow => "slow",
            Self::GaveUp => "walked",
        }
    }

    pub fn is_bad(self) -> bool {
        !matches!(self, Self::Good)
    }
}

/// One remembered leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct JourneyRecord {
    pub from: StationId,
    pub to: StationId,
    /// Time spent on the platform (sim-seconds).
    pub wait_secs: u32,
    /// Door-to-door time for the leg (sim-seconds).
    pub total_secs: u32,
    pub outcome: JourneyOutcome,
    /// Sim tick the leg finished — feeds *"third time this week"*.
    pub ended_tick: u64,
}

impl JourneyRecord {
    /// Inspector history line — `"Eastgate → Millhaven · 7 min · good"`.
    pub fn summary(&self, from_name: &str, to_name: &str) -> String {
        format!(
            "{from_name} -> {to_name} - {} min - {}",
            (self.total_secs / 60).max(1),
            self.outcome.label()
        )
    }
}

/// A peep's recent journeys, and the patience that history buys them.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JourneyMemory {
    /// Newest first, capped at [`MEMORY_DEPTH`].
    pub recent: VecDeque<JourneyRecord>,
    pub lifetime_journeys: u32,
    pub lifetime_gave_up: u32,
    /// Bad legs since the last good one — the "four bad ones and they leave" counter.
    pub bad_streak: u32,
    /// Good legs since the last bad one.
    pub good_streak: u32,
}

impl JourneyMemory {
    pub fn record(&mut self, entry: JourneyRecord) {
        if entry.outcome.is_bad() {
            self.bad_streak = self.bad_streak.saturating_add(1);
            self.good_streak = 0;
            if entry.outcome == JourneyOutcome::GaveUp {
                self.lifetime_gave_up = self.lifetime_gave_up.saturating_add(1);
            }
        } else {
            self.good_streak = self.good_streak.saturating_add(1);
            // A good commute forgives one bad one — this is the "four good
            // commutes buys tolerance for one bad" rule, spent one at a time.
            self.bad_streak = self.bad_streak.saturating_sub(1);
        }
        self.lifetime_journeys = self.lifetime_journeys.saturating_add(1);
        self.recent.push_front(entry);
        while self.recent.len() > MEMORY_DEPTH {
            self.recent.pop_back();
        }
    }

    pub fn last(&self) -> Option<&JourneyRecord> {
        self.recent.front()
    }

    pub fn recent_bad(&self) -> u32 {
        self.recent.iter().filter(|r| r.outcome.is_bad()).count() as u32
    }

    pub fn recent_good(&self) -> u32 {
        self.recent.iter().filter(|r| !r.outcome.is_bad()).count() as u32
    }

    /// Goodwill balance — positive when recent journeys have gone well.
    pub fn goodwill(&self) -> i32 {
        self.recent_good() as i32 - self.recent_bad() as i32
    }

    /// How long this peep will stand on a platform before giving up.
    ///
    /// Four good commutes stretches patience by roughly half again; a run of
    /// bad ones collapses it toward [`MIN_PATIENCE_SECS`].
    pub fn patience_secs(&self) -> u32 {
        let base = BASE_PATIENCE_SECS as i64;
        let step = base / 8;
        let adjusted = base + step * self.goodwill() as i64;
        adjusted.clamp(MIN_PATIENCE_SECS as i64, MAX_PATIENCE_SECS as i64) as u32
    }

    /// Sustained frustration — the household should start packing (§4.3).
    pub fn wants_to_leave(&self) -> bool {
        self.bad_streak >= BAD_JOURNEYS_TO_LEAVE
    }

    /// Plain-language history line for the Peep card.
    pub fn tolerance_line(&self) -> String {
        if self.recent.is_empty() {
            return "No journeys yet.".into();
        }
        if self.wants_to_leave() {
            return format!(
                "{} bad journeys in a row - looking to move.",
                self.bad_streak
            );
        }
        if self.bad_streak > 0 {
            format!(
                "{} good of the last {} - still giving it a chance.",
                self.recent_good(),
                self.recent.len()
            )
        } else {
            format!("{} good journeys in a row.", self.good_streak)
        }
    }
}

/// Classify a finished leg from its wait and total time.
pub fn outcome_for(
    wait_secs: u32,
    total_secs: u32,
    gave_up: bool,
    patience_secs: u32,
) -> JourneyOutcome {
    if gave_up {
        return JourneyOutcome::GaveUp;
    }
    if wait_secs >= patience_secs / 2 || total_secs > GOOD_JOURNEY_SECS {
        JourneyOutcome::Slow
    } else {
        JourneyOutcome::Good
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(outcome: JourneyOutcome) -> JourneyRecord {
        JourneyRecord {
            from: StationId(1),
            to: StationId(2),
            wait_secs: 60,
            total_secs: 240,
            outcome,
            ended_tick: 0,
        }
    }

    #[test]
    fn four_good_commutes_buy_tolerance_for_one_bad() {
        let mut mem = JourneyMemory::default();
        let plain = mem.patience_secs();
        for _ in 0..4 {
            mem.record(record(JourneyOutcome::Good));
        }
        let patient = mem.patience_secs();
        assert!(
            patient > plain,
            "four good commutes should stretch patience ({patient} > {plain})"
        );

        mem.record(record(JourneyOutcome::Slow));
        assert!(!mem.wants_to_leave(), "one bad leg must not tip them out");
        assert!(mem.patience_secs() > MIN_PATIENCE_SECS);
    }

    #[test]
    fn four_bad_commutes_and_they_want_out() {
        let mut mem = JourneyMemory::default();
        for _ in 0..BAD_JOURNEYS_TO_LEAVE {
            mem.record(record(JourneyOutcome::GaveUp));
        }
        assert!(mem.wants_to_leave());
        assert_eq!(mem.lifetime_gave_up, BAD_JOURNEYS_TO_LEAVE);
        assert!(mem.patience_secs() < BASE_PATIENCE_SECS);
    }

    #[test]
    fn a_good_commute_pays_down_the_bad_streak() {
        let mut mem = JourneyMemory::default();
        for _ in 0..3 {
            mem.record(record(JourneyOutcome::Slow));
        }
        assert_eq!(mem.bad_streak, 3);
        mem.record(record(JourneyOutcome::Good));
        assert_eq!(mem.bad_streak, 2);
        assert!(!mem.wants_to_leave());
    }

    #[test]
    fn memory_is_capped_and_newest_first() {
        let mut mem = JourneyMemory::default();
        for i in 0..(MEMORY_DEPTH + 4) {
            let mut r = record(JourneyOutcome::Good);
            r.ended_tick = i as u64;
            mem.record(r);
        }
        assert_eq!(mem.recent.len(), MEMORY_DEPTH);
        assert_eq!(mem.last().unwrap().ended_tick, (MEMORY_DEPTH + 3) as u64);
        assert_eq!(mem.lifetime_journeys, (MEMORY_DEPTH + 4) as u32);
    }

    #[test]
    fn outcome_classifies_wait_and_giving_up() {
        assert_eq!(
            outcome_for(30, 120, false, BASE_PATIENCE_SECS),
            JourneyOutcome::Good
        );
        assert_eq!(
            outcome_for(9 * 60, 600, false, BASE_PATIENCE_SECS),
            JourneyOutcome::Slow
        );
        assert_eq!(
            outcome_for(9 * 60, 600, true, BASE_PATIENCE_SECS),
            JourneyOutcome::GaveUp
        );
    }
}
