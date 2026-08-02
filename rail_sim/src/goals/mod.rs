//! Goals mode — the sandbox with objectives and deadlines on it.
//!
//! Design brief: [`docs/design/08-economy-and-pressure.md`](../../../docs/design/08-economy-and-pressure.md) §8.
//! The brief is emphatic, and this module is built to obey it:
//!
//! > The same systems with objectives and deadlines. Objectives are drawn from
//! > what the sandbox already produces … Deadlines make the pacing bite, and
//! > that is the only difference. No separate systems, no special rules.
//!
//! So there is no goal simulation here. [`progress::evaluate_goals`] *reads*
//! [`StationRegistry`](crate::stations::StationRegistry),
//! [`StationService`](crate::stations::StationService),
//! [`TownDensity`](crate::town::TownDensity),
//! [`MoneyLedger`](crate::economy::MoneyLedger),
//! [`TrackNetwork`](crate::track::TrackNetwork) and the household roll, and
//! writes nothing back except the goals' own progress. Every improvement to the
//! sandbox improves goals mode for free, which is the point.
//!
//! # Shape
//!
//! | Module | Owns |
//! | --- | --- |
//! | `goal` | What one objective asks for and how it reads |
//! | `board` | The world's set, and the sandbox / goals switch |
//! | `generate` | Deriving a set from the map seed and its anchors |
//! | `progress` | Evaluation on the fixed tick, and Town Talk |
//!
//! # Failure
//!
//! A missed deadline sets [`GoalStatus::Failed`] and says so once in Town Talk.
//! That is the whole consequence. There is no game-over, no score, and nothing
//! is taken away — design 08 §5.3 is unambiguous that stagnation is the only
//! failure the game has, and a lens on the sandbox does not get to invent
//! another one.
//!
//! # Turning it on
//!
//! Sandbox is the default and every system here returns immediately in it. The
//! shell calls [`GoalBoard::start`] when it installs a world, with the mode the
//! player chose on the New Map screen and that world's seed.

mod board;
mod generate;
mod goal;
mod progress;

pub use board::{GoalBoard, GoalMode};
pub use generate::{generate_goal_set, GOALS_PER_SET};
pub use goal::{Goal, GoalId, GoalKind, GoalStatus};
pub use progress::{evaluate_goals, generate_goals_once};

use bevy_app::{App, FixedUpdate, Plugin, Update};
use bevy_ecs::schedule::IntoScheduleConfigs;

use crate::{sim_is_running, SimSet};

/// Registers the goal board, set generation, and per-tick evaluation.
///
/// Evaluation is ordered after [`tick_money_ledger`](crate::economy::tick_money_ledger)
/// so a goal reads the tick's finished numbers: that system runs after the
/// service tick has advanced and after the tick's income has landed, which are
/// the two things every goal's progress is measured against.
pub struct GoalsPlugin;

impl Plugin for GoalsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GoalBoard>()
            .add_systems(Update, generate_goals_once)
            .add_systems(
                FixedUpdate,
                evaluate_goals
                    .after(crate::economy::tick_money_ledger)
                    .in_set(SimSet::Advance)
                    .run_if(sim_is_running),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::TileCoord;
    use crate::peeps::ComplaintFeed;
    use crate::stations::{IndustryRegistry, StationRegistry, StationService};
    use crate::track::GROUND_LAYER;
    use crate::SimClock;
    use bevy_app::App;

    /// The plugin on its own, with only the resources its systems read. Kept
    /// independent of `SimPlugin` so these tests describe *this* plugin rather
    /// than where the app happens to register it.
    fn goals_app() -> App {
        let mut app = App::new();
        app.add_plugins(GoalsPlugin)
            .init_resource::<SimClock>()
            .init_resource::<StationRegistry>()
            .init_resource::<IndustryRegistry>()
            .init_resource::<StationService>()
            .init_resource::<ComplaintFeed>();
        app
    }

    fn step(app: &mut App) {
        app.world_mut().run_schedule(Update);
    }

    #[test]
    fn the_plugin_leaves_a_sandbox_world_completely_alone() {
        let mut app = goals_app();
        step(&mut app);
        let board = app.world().resource::<GoalBoard>();
        assert_eq!(board.mode, GoalMode::Sandbox);
        assert!(board.is_empty());
        assert!(!board.needs_generation());
    }

    #[test]
    fn a_goals_world_generates_its_set_as_soon_as_anchors_exist() {
        let mut app = goals_app();
        app.world_mut()
            .resource_mut::<GoalBoard>()
            .start(GoalMode::Goals, 84_213);

        // No anchors yet — generation waits rather than inventing any.
        step(&mut app);
        assert!(app.world().resource::<GoalBoard>().is_empty());
        assert!(app.world().resource::<GoalBoard>().needs_generation());

        let mut stations = StationRegistry::new();
        stations.insert("Eastgate", TileCoord { x: 8, y: 8 }, GROUND_LAYER);
        stations.insert("Westbrook", TileCoord { x: 18, y: 8 }, GROUND_LAYER);
        app.world_mut().insert_resource(stations);
        step(&mut app);

        let board = app.world().resource::<GoalBoard>();
        assert!(
            !board.is_empty(),
            "the set lands once there is a world to describe"
        );
        assert!(!board.needs_generation());
        assert!(
            board.iter().all(|g| g.deadline_tick > 0),
            "deadlines are the point"
        );
        assert!(
            app.world()
                .resource::<ComplaintFeed>()
                .iter()
                .any(|e| e.display_line().starts_with("First up:")),
            "the set introduces itself in Town Talk, not in a popup"
        );
    }
}
