//! Input → sim commands, and presentation sync from [`SimClock`].
//!
//! Keyboard (MVP):
//! - Space — toggle pause
//! - 1 / 2 / 3 — set speed multiplier (1x / 2x / 3x); unpauses

use bevy::prelude::*;
use rail_sim::{CommandBuffer, CommandKind, SimClock};

pub struct SimBridgePlugin;

impl Plugin for SimBridgePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (time_controls_input, sync_virtual_time_from_clock));
    }
}

/// Push pause / speed commands from the keyboard into the sim command buffer.
fn time_controls_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut buffer: ResMut<CommandBuffer>,
    clock: Res<SimClock>,
) {
    if keys.just_pressed(KeyCode::Space) {
        buffer.push(CommandKind::toggle_pause_from(clock.paused));
    }
    if keys.just_pressed(KeyCode::Digit1) {
        buffer.push(CommandKind::set_speed(1));
    }
    if keys.just_pressed(KeyCode::Digit2) {
        buffer.push(CommandKind::set_speed(2));
    }
    if keys.just_pressed(KeyCode::Digit3) {
        buffer.push(CommandKind::set_speed(3));
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
