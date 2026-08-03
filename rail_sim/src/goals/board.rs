//! The world's goal set, and the switch that turns goals mode on.
//!
//! One resource, inert by default. In [`GoalMode::Sandbox`] — which is what
//! every world is unless the New Map screen says otherwise — the board holds no
//! goals and [`super::progress::evaluate_goals`] returns on its first line, so
//! goals mode costs the sandbox nothing.

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use super::goal::{Goal, GoalId, GoalStatus};

/// Session shape, chosen on the New Map screen (design 09 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GoalMode {
    #[default]
    Sandbox,
    Goals,
}

impl GoalMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sandbox => "Sandbox",
            Self::Goals => "Goals",
        }
    }
}

/// The goals for this world, plus how the world was started.
///
/// The set is generated from the map seed once the world's anchors exist — see
/// [`generate_goal_set`](super::generate_goal_set) — so it suits the terrain the
/// player is actually looking at rather than being a fixed campaign.
///
/// A board that has resolved every goal earns a new, harder one rather than
/// standing as a finished scoreboard: design 08 §5.3 names stagnation as the
/// only failure, and a goals world whose board has stopped asking for anything
/// is exactly that.
#[derive(Debug, Clone, Default, PartialEq, Resource, Serialize, Deserialize)]
pub struct GoalBoard {
    pub mode: GoalMode,
    /// Map seed the set was (or will be) generated from.
    pub seed: u64,
    goals: Vec<Goal>,
    /// `true` once generation has run for this world, successfully or not.
    generated: bool,
    /// Which set this is: `0` for the world's first, `1` for the harder one
    /// that replaced it, and so on. Feeds
    /// [`generate_goal_set`](super::generate_goal_set), so regeneration stays a
    /// pure function of the world rather than of when it happened to be asked.
    generation: u32,
}

impl GoalBoard {
    /// Begin a world. Clears any previous set, so New Map and Load both land in
    /// a known state without the caller reaching into the fields.
    pub fn start(&mut self, mode: GoalMode, seed: u64) {
        self.mode = mode;
        self.seed = seed;
        self.goals.clear();
        self.generated = false;
        self.generation = 0;
    }

    /// `true` when this world is playing to goals.
    pub fn is_active(&self) -> bool {
        self.mode == GoalMode::Goals
    }

    /// `true` while an active board is still waiting for its set.
    pub fn needs_generation(&self) -> bool {
        self.is_active() && !self.generated
    }

    /// Install a generated set. Marks the board generated even for an empty
    /// set, so a world with no usable anchors does not retry every frame.
    pub fn install(&mut self, goals: Vec<Goal>) {
        self.goals = goals;
        self.generated = true;
    }

    /// Which set this is — `0` for the world's first.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Replace a resolved board with its successor and count the generation.
    pub fn install_next_generation(&mut self, goals: Vec<Goal>) {
        self.generation = self.generation.saturating_add(1);
        self.install(goals);
    }

    /// `true` when every goal on a non-empty board has been met or missed.
    ///
    /// The trigger for the next set. An empty board is *not* resolved: a world
    /// with no usable anchors never generated one, and asking again every tick
    /// would be a spin, not a milestone.
    pub fn all_resolved(&self) -> bool {
        !self.goals.is_empty() && self.goals.iter().all(|g| !g.is_active())
    }

    pub fn iter(&self) -> impl Iterator<Item = &Goal> {
        self.goals.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Goal> {
        self.goals.iter_mut()
    }

    pub fn get(&self, id: GoalId) -> Option<&Goal> {
        self.goals.iter().find(|g| g.id == id)
    }

    pub fn len(&self) -> usize {
        self.goals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.goals.is_empty()
    }

    pub fn count(&self, status: GoalStatus) -> usize {
        self.goals.iter().filter(|g| g.status == status).count()
    }

    /// One line for the panel header: "2 of 6 met · 1 missed".
    ///
    /// Missed goals are stated, not hidden — but they are stated last, and
    /// nothing about the wording suggests the session is over, because it is
    /// not (design 08 §1).
    pub fn summary_line(&self) -> String {
        let met = self.count(GoalStatus::Complete);
        let missed = self.count(GoalStatus::Failed);
        let mut line = format!("{met} of {} met", self.len());
        if missed > 0 {
            line.push_str(&format!(" - {missed} missed"));
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::GoalKind;

    fn board_with(statuses: &[GoalStatus]) -> GoalBoard {
        let mut board = GoalBoard::default();
        board.start(GoalMode::Goals, 42);
        board.install(
            statuses
                .iter()
                .enumerate()
                .map(|(i, status)| {
                    let mut goal =
                        Goal::new(GoalId(i as u32), GoalKind::Deliveries, "test", 10, 100);
                    goal.status = *status;
                    goal
                })
                .collect(),
        );
        board
    }

    #[test]
    fn a_sandbox_board_is_inert() {
        let board = GoalBoard::default();
        assert_eq!(board.mode, GoalMode::Sandbox);
        assert!(!board.is_active());
        assert!(
            !board.needs_generation(),
            "sandbox must never ask for a goal set"
        );
        assert!(board.is_empty());
    }

    #[test]
    fn starting_a_goals_world_asks_for_a_set_exactly_once() {
        let mut board = GoalBoard::default();
        board.start(GoalMode::Goals, 84_213);
        assert!(board.needs_generation());
        board.install(Vec::new());
        assert!(
            !board.needs_generation(),
            "a world with no usable anchors must not retry forever"
        );
    }

    #[test]
    fn starting_again_clears_the_previous_worlds_goals() {
        let mut board = board_with(&[GoalStatus::Complete, GoalStatus::Active]);
        board.start(GoalMode::Goals, 7);
        assert!(board.is_empty());
        assert_eq!(board.seed, 7);
        assert!(board.needs_generation());
    }

    #[test]
    fn the_summary_states_misses_without_sounding_terminal() {
        let board = board_with(&[
            GoalStatus::Complete,
            GoalStatus::Complete,
            GoalStatus::Failed,
            GoalStatus::Active,
        ]);
        assert_eq!(board.summary_line(), "2 of 4 met - 1 missed");

        let clean = board_with(&[GoalStatus::Complete, GoalStatus::Active]);
        assert_eq!(
            clean.summary_line(),
            "1 of 2 met",
            "no misses, no mention of them"
        );
    }
}
