//! Track build / demolish tools and placeholder sprites.

mod sync;
mod tools;
mod visuals;

use bevy::prelude::*;

use sync::sync_track_terrain_from_map;
use tools::track_tool_input;
use visuals::apply_track_sprites;

pub use tools::{BuildTool, TrackToolState};

pub struct TrackPlugin;

impl Plugin for TrackPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TrackToolState>()
            .add_systems(Startup, sync_track_terrain_from_map)
            .add_systems(Update, (track_tool_input, apply_track_sprites));
    }
}
