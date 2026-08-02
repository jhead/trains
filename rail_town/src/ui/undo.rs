//! Construction undo / redo input (Ctrl/Cmd+Z, Ctrl/Cmd+Shift+Z or Y).

use bevy::prelude::*;
use rail_sim::{CommandBuffer, CommandHistory};

pub fn undo_redo_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut history: ResMut<CommandHistory>,
    mut buffer: ResMut<CommandBuffer>,
) {
    let ctrl = keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight)
        || keys.pressed(KeyCode::SuperLeft)
        || keys.pressed(KeyCode::SuperRight);
    if !ctrl {
        return;
    }
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);

    if keys.just_pressed(KeyCode::KeyZ) && !shift {
        if let Some(cmds) = history.begin_undo() {
            for kind in cmds {
                buffer.push(kind);
            }
        }
        return;
    }

    // Redo: Ctrl+Shift+Z or Ctrl+Y (and Cmd equivalents via Super).
    let redo = (keys.just_pressed(KeyCode::KeyZ) && shift) || keys.just_pressed(KeyCode::KeyY);
    if redo {
        if let Some(cmds) = history.begin_redo() {
            for kind in cmds {
                buffer.push(kind);
            }
        }
    }
}
