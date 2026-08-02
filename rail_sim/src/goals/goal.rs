//! One objective: what it asks for, how far along it is, and when it is due.
//!
//! Every [`GoalKind`] is a **read** of state the sandbox already produces —
//! the station registry, service scores, town density, the ledger, the track
//! graph, the household roll. Nothing here simulates anything, and a new goal
//! kind that needed its own simulation would be the wrong goal (design 08 §8:
//! *"no separate systems, no special rules"*).
//!
//! Targets live on [`Goal::target`] rather than inside the kind, so progress is
//! one shape — `current` out of `target` — for every objective. The UI needs no
//! per-kind arithmetic, and a new kind costs one match arm in
//! [`super::progress`] and one label.

use serde::{Deserialize, Serialize};

use crate::ids::StationId;
use crate::peeps::{SIM_SECONDS_PER_TICK, TICKS_PER_DAY};

/// Stable id within one goal set. Not global — a new map starts at zero.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct GoalId(pub u32);

/// The condition a goal tests.
///
/// Parameters that are *not* the target (which station, which score floor) live
/// here; the number to reach is always [`Goal::target`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalKind {
    /// A rail path exists between two stops. Target is always `1`.
    Connect { from: StationId, to: StationId },
    /// Peeps living in the town.
    Population,
    /// Paid runs completed — fares plus goods deliveries.
    Deliveries,
    /// Ticks banked with a stop at or above `min_score`.
    ///
    /// Cumulative, not consecutive: a bad afternoon slows the goal down rather
    /// than wiping the week. Failure in this game is a deadline, never a reset.
    Serve { station: StationId, min_score: u8 },
    /// Built density inside a stop's catchment, in tenths.
    Grow { station: StationId },
}

impl GoalKind {
    /// Short word for what `current` / `target` are counted in.
    pub fn unit(self) -> &'static str {
        match self {
            Self::Connect { .. } => "",
            Self::Population => "residents",
            Self::Deliveries => "runs",
            Self::Serve { .. } => "min",
            Self::Grow { .. } => "built",
        }
    }

    /// The station this goal is about, when it is about one.
    pub fn station(self) -> Option<StationId> {
        match self {
            Self::Connect { from, .. } => Some(from),
            Self::Serve { station, .. } | Self::Grow { station } => Some(station),
            Self::Population | Self::Deliveries => None,
        }
    }
}

/// Where a goal stands. **`Failed` is never a game-over** — design 08 §1: the
/// economy never ends the game, and neither does a lens on it. A missed
/// deadline stops the goal, not the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GoalStatus {
    #[default]
    Active,
    Complete,
    Failed,
}

impl GoalStatus {
    /// One word for the row, so colour never carries the state alone
    /// (design 03 §4).
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "open",
            Self::Complete => "met",
            Self::Failed => "missed",
        }
    }
}

/// One objective, its progress, and its deadline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Goal {
    pub id: GoalId,
    pub kind: GoalKind,
    /// Player-facing sentence, resolved against real anchor names when the set
    /// was generated. Stored rather than looked up so a demolished stop leaves
    /// the goal readable instead of blank.
    pub title: String,
    pub target: u64,
    pub current: u64,
    /// Sim tick the goal is due by. Deadlines are the *only* thing goals mode
    /// adds to the sandbox (design 08 §8).
    pub deadline_tick: u64,
    pub status: GoalStatus,
    /// Tick the goal completed or lapsed on; `0` while it is still open.
    pub resolved_tick: u64,
    /// `true` once the one-time "deadline is close" line has gone to Town Talk.
    pub warned: bool,
}

impl Goal {
    pub fn new(
        id: GoalId,
        kind: GoalKind,
        title: impl Into<String>,
        target: u64,
        deadline_tick: u64,
    ) -> Self {
        Self {
            id,
            kind,
            title: title.into(),
            target: target.max(1),
            current: 0,
            deadline_tick,
            status: GoalStatus::Active,
            resolved_tick: 0,
            warned: false,
        }
    }

    pub fn is_active(&self) -> bool {
        self.status == GoalStatus::Active
    }

    pub fn is_complete(&self) -> bool {
        self.status == GoalStatus::Complete
    }

    pub fn is_failed(&self) -> bool {
        self.status == GoalStatus::Failed
    }

    /// Progress as a whole percentage, clamped to `0..=100`.
    pub fn percent(&self) -> u32 {
        if self.target == 0 {
            return 100;
        }
        let filled = self.current.min(self.target).saturating_mul(100) / self.target;
        filled.min(100) as u32
    }

    /// `current` / `target` with its unit — design 03 §8.4: a bare meter is
    /// unreadable at these sizes, so every bar carries its numeral.
    pub fn progress_label(&self) -> String {
        match self.kind {
            GoalKind::Connect { .. } => {
                if self.current > 0 { "linked" } else { "no route" }.into()
            }
            GoalKind::Serve { .. } => format!(
                "{} / {} {}",
                ticks_to_minutes(self.current),
                ticks_to_minutes(self.target),
                self.kind.unit()
            ),
            GoalKind::Grow { .. } => format!(
                "{} / {} {}",
                tenths(self.current),
                tenths(self.target),
                self.kind.unit()
            ),
            _ => format!("{} / {} {}", self.current, self.target, self.kind.unit()),
        }
    }

    /// Sim day the goal is due on, counting from day 0 like the rest of the sim.
    pub fn deadline_day(&self) -> u64 {
        self.deadline_tick / TICKS_PER_DAY
    }

    /// "by day 4" — the deadline stated plainly, with no clock arithmetic asked
    /// of the player.
    pub fn deadline_label(&self) -> String {
        format!("by day {}", self.deadline_day())
    }

    /// How much time is left, at `now_tick`. Resolved goals report the outcome
    /// instead, because a met goal's deadline stops being interesting.
    pub fn time_label(&self, now_tick: u64) -> String {
        match self.status {
            GoalStatus::Complete => "met".into(),
            GoalStatus::Failed => "missed".into(),
            GoalStatus::Active => {
                if now_tick >= self.deadline_tick {
                    return "overdue".into();
                }
                let left = self.deadline_tick - now_tick;
                let days = left / TICKS_PER_DAY;
                if days >= 1 {
                    return format!("{days}d left");
                }
                let minutes = ticks_to_minutes(left);
                if minutes >= 60 {
                    format!("{}h left", minutes / 60)
                } else {
                    format!("{}m left", minutes.max(1))
                }
            }
        }
    }

    /// `true` once the deadline is inside the last sim-day, so the world can say
    /// so once. Design 08 §5.2: pressure is announced with time to react.
    pub fn deadline_is_close(&self, now_tick: u64) -> bool {
        self.is_active()
            && now_tick < self.deadline_tick
            && self.deadline_tick - now_tick <= TICKS_PER_DAY
    }
}

/// Whole sim-minutes for a tick count.
fn ticks_to_minutes(ticks: u64) -> u64 {
    ticks.saturating_mul(u64::from(SIM_SECONDS_PER_TICK)) / 60
}

/// Tenths rendered as one decimal place, without touching floating point.
fn tenths(value: u64) -> String {
    format!("{}.{}", value / 10, value % 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: StationId = StationId(1);
    const B: StationId = StationId(2);

    fn goal(kind: GoalKind, target: u64) -> Goal {
        Goal::new(GoalId(1), kind, "test", target, TICKS_PER_DAY * 2)
    }

    #[test]
    fn progress_is_a_percentage_that_never_overflows_the_bar() {
        let mut g = goal(GoalKind::Deliveries, 40);
        assert_eq!(g.percent(), 0);
        g.current = 10;
        assert_eq!(g.percent(), 25);
        g.current = 400;
        assert_eq!(g.percent(), 100, "over-delivering does not exceed 100%");
    }

    #[test]
    fn every_kind_renders_a_numeral_beside_its_bar() {
        // Design 03 §8.4 — a bare meter is unreadable at these sizes.
        for kind in [
            GoalKind::Connect { from: A, to: B },
            GoalKind::Population,
            GoalKind::Deliveries,
            GoalKind::Serve {
                station: A,
                min_score: 50,
            },
            GoalKind::Grow { station: A },
        ] {
            let g = goal(kind, 100);
            assert!(!g.progress_label().is_empty(), "{kind:?} has no readout");
        }
    }

    #[test]
    fn a_connect_goal_reads_as_a_state_not_a_count() {
        let mut g = goal(GoalKind::Connect { from: A, to: B }, 1);
        assert_eq!(g.progress_label(), "no route");
        g.current = 1;
        assert_eq!(g.progress_label(), "linked");
    }

    #[test]
    fn held_time_reads_in_minutes_not_ticks() {
        let mut g = goal(
            GoalKind::Serve {
                station: A,
                min_score: 50,
            },
            TICKS_PER_DAY,
        );
        g.current = 6 * 30; // 30 sim-minutes at 10s a tick
        assert_eq!(g.progress_label(), "30 / 1440 min");
    }

    #[test]
    fn density_reads_with_one_decimal_place() {
        let mut g = goal(GoalKind::Grow { station: A }, 190);
        g.current = 47;
        assert_eq!(g.progress_label(), "4.7 / 19.0 built");
    }

    #[test]
    fn time_left_counts_down_then_says_overdue() {
        let g = goal(GoalKind::Deliveries, 10);
        assert_eq!(g.time_label(0), "2d left");
        // Inside the last day it drops to hours, then to minutes.
        assert_eq!(g.time_label(TICKS_PER_DAY + 1), "23h left");
        assert_eq!(g.time_label(TICKS_PER_DAY * 2 - 6), "1m left");
        assert_eq!(g.time_label(TICKS_PER_DAY * 2), "overdue");
        assert_eq!(g.time_label(TICKS_PER_DAY * 9), "overdue");
    }

    #[test]
    fn a_resolved_goal_reports_its_outcome_rather_than_a_countdown() {
        let mut g = goal(GoalKind::Deliveries, 10);
        g.status = GoalStatus::Complete;
        assert_eq!(g.time_label(0), "met");
        g.status = GoalStatus::Failed;
        assert_eq!(g.time_label(0), "missed");
    }

    #[test]
    fn the_world_gets_one_days_warning_before_a_deadline() {
        let g = goal(GoalKind::Deliveries, 10);
        assert!(!g.deadline_is_close(0), "two days out is not close");
        assert!(g.deadline_is_close(TICKS_PER_DAY + 1));
        assert!(
            !g.deadline_is_close(TICKS_PER_DAY * 2),
            "past the deadline is overdue, not close"
        );
    }

    #[test]
    fn status_words_carry_the_state_without_colour() {
        // Design 03 §4: colour never carries meaning alone.
        assert_eq!(GoalStatus::Active.label(), "open");
        assert_eq!(GoalStatus::Complete.label(), "met");
        assert_eq!(GoalStatus::Failed.label(), "missed");
    }

    #[test]
    fn deadlines_are_stated_in_days() {
        let g = goal(GoalKind::Deliveries, 10);
        assert_eq!(g.deadline_day(), 2);
        assert_eq!(g.deadline_label(), "by day 2");
    }
}
