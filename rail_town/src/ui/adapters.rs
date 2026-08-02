//! Windows whose bodies are owned by other modules.
//!
//! Three panels are windows now but are drawn elsewhere: Goals (`shell`),
//! Neighbours (`border`) and the Inspector (`inspect`). Goals and Neighbours
//! carry [`UiWindow`](super::window::UiWindow) on their own roots, so the window
//! manager owns their position, stacking and title bar directly.
//!
//! The Inspector is the exception, and deliberately so. It is not opened by a
//! button — it opens because the player selected something in the world — and
//! `inspect` is being worked on in parallel, so this module keeps the window
//! manager's idea of "the Inspector is open" in step with [`Selection`] from the
//! outside. That is enough to put the Inspector in the `Esc` stack today: `Esc`
//! over an inspected station clears the selection instead of pausing the game.
//!
//! When `inspect::panel` adopts `UiWindow` it also becomes draggable and gets a
//! close box, and nothing here has to change — the sync below is idempotent.

use bevy::prelude::*;
use rail_sim::GoalBoard;

use crate::inspect::Selection;
use crate::ui::window::{WindowId, WindowManager};

/// Whether the Goals window has been offered once.
#[derive(Resource, Debug, Default)]
pub struct GoalsIntroduced(pub bool);

/// Remembers whether the Inspector slot was open last frame.
///
/// Without it the two directions of the sync cannot be told apart: "the player
/// closed the window" and "the player has not selected anything yet" look
/// identical from a single frame's state.
#[derive(Resource, Debug, Default)]
pub struct InspectorLink {
    was_open: bool,
}

/// Keep the Inspector's window slot in step with the world selection.
///
/// One system rather than two, so the two directions can never race:
/// - selecting something opens the slot, which puts it in the `Esc` stack;
/// - the slot closing from the outside (`Esc`, or a close box once `inspect`
///   has one) clears the selection, so the panel goes away rather than
///   reopening on the next frame.
pub fn sync_inspector_window(
    mut selection: ResMut<Selection>,
    mut manager: ResMut<WindowManager>,
    mut link: ResMut<InspectorLink>,
) {
    let selected = selection.0.is_some();
    let open = manager.is_open(WindowId::Inspector);

    if link.was_open && !open && selected {
        selection.clear();
        link.was_open = false;
        return;
    }
    if selected && !open {
        manager.open(WindowId::Inspector);
    } else if !selected && open {
        manager.close(WindowId::Inspector);
    }
    // Guarded so this does not mark the resource changed on every idle frame.
    let now_open = manager.is_open(WindowId::Inspector);
    if link.was_open != now_open {
        link.was_open = now_open;
    }
}

/// Open the Goals window once, the first time a map actually has goals.
///
/// A goals map that never shows its goals is a puzzle with the instructions in
/// a drawer. After that first open it belongs to the player like every other
/// window — including staying closed if that is where they left it.
pub fn introduce_goals_window(
    board: Option<Res<GoalBoard>>,
    mut introduced: ResMut<GoalsIntroduced>,
    mut manager: ResMut<WindowManager>,
) {
    if introduced.0 {
        return;
    }
    let Some(board) = board else {
        return;
    };
    if !board.is_active() || board.is_empty() {
        return;
    }
    introduced.0 = true;
    manager.open(WindowId::Goals);
}
