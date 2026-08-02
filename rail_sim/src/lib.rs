//! Rail Town simulation library.
//!
//! ECS-friendly sim types and systems. No rendering or windowing deps.
//! Player intent flows through [`commands`]; systems apply them on the fixed tick.

pub mod commands;
pub mod ids;

use bevy_app::{App, Plugin};

/// Registers sim schedules / systems once they exist.
///
/// Slice 0: empty shell. Later slices add systems to Bevy's `FixedUpdate`
/// (via the app in `rail_town`) or extend this plugin.
pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, _app: &mut App) {
        // Fixed-tick sim systems land here (or are added from rail_town
        // into FixedUpdate). Keep all economy / movement / growth off Update.
    }
}
