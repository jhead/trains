//! Construction undo / redo input (Ctrl/Cmd+Z, Ctrl/Cmd+Shift+Z or Y).
//!
//! Both are rebindable — the keys above are the defaults the Controls tab
//! lists. Shift is the one thing not in the map: `Shift+Undo` is redo by
//! convention rather than by binding, exactly as it is everywhere else.

use bevy::prelude::*;
use rail_sim::{CommandBuffer, CommandHistory};

use crate::input::{ControlAction, KeyBindings};

pub fn undo_redo_input(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    mut history: ResMut<CommandHistory>,
    mut buffer: ResMut<CommandBuffer>,
) {
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let undo_key = bindings.just_pressed(&keys, ControlAction::Undo);

    if undo_key && !shift {
        if let Some(cmds) = history.begin_undo() {
            for kind in cmds {
                buffer.push(kind);
            }
        }
        return;
    }

    // Redo: Shift held on the undo chord, or the redo binding on its own.
    if (undo_key && shift) || bindings.just_pressed(&keys, ControlAction::Redo) {
        if let Some(cmds) = history.begin_redo() {
            for kind in cmds {
                buffer.push(kind);
            }
        }
    }
}
