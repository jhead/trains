//! Input → sim commands, and presentation sync from [`SimClock`].
//!
//! Keyboard (default bindings; every one is rebindable through
//! [`crate::input::KeyBindings`]):
//! - Space — toggle pause
//! - 1 / 2 / 3 — set speed multiplier (1x / 2x / 3x); unpauses

use bevy::prelude::*;
use rail_sim::{CommandBuffer, CommandKind, SimClock};

use crate::input::{ControlAction, KeyBindings};

pub struct SimBridgePlugin;

impl Plugin for SimBridgePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KeyBindings>()
            .add_systems(Update, (time_controls_input, sync_virtual_time_from_clock));
    }
}

/// Push pause / speed commands from the keyboard into the sim command buffer.
fn time_controls_input(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    mut buffer: ResMut<CommandBuffer>,
    clock: Res<SimClock>,
) {
    if bindings.just_pressed(&keys, ControlAction::PauseResume) {
        buffer.push(CommandKind::toggle_pause_from(clock.paused));
    }
    for (action, speed) in [
        (ControlAction::Speed1, 1),
        (ControlAction::Speed2, 2),
        (ControlAction::Speed3, 3),
    ] {
        if bindings.just_pressed(&keys, action) {
            buffer.push(CommandKind::set_speed(speed));
        }
    }
}

/// Drive Bevy virtual time from [`SimClock`] so FixedUpdate rate matches speed.
///
/// When paused we keep relative speed at 1.0 (do **not** pause Bevy time) so
/// FixedUpdate still drains the command buffer — build-while-paused stays alive.
fn sync_virtual_time_from_clock(clock: Res<SimClock>, mut time: ResMut<Time<Virtual>>) {
    if time.is_paused() {
        time.unpause();
    }
    let speed = if clock.paused {
        1.0
    } else {
        clock.relative_speed()
    };
    time.set_relative_speed(speed);
}
