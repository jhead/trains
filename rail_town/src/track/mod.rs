//! Track build / demolish tools, ghosts, and placement feedback.

mod feedback;
mod ghost;
mod preview;
mod propose;
mod sync;
mod tools;
mod visuals;

use bevy::prelude::*;

use feedback::{setup_build_feedback, sync_flash_sprites, update_build_feedback_ui};
use ghost::sync_track_ghosts;
use sync::sync_track_terrain_from_map;
use tools::track_tool_input;
use visuals::apply_track_sprites;

use crate::inspect::SelectionInputSet;

pub use tools::{BuildTool, TrackToolState};
pub use visuals::TrackSprite;

pub struct TrackPlugin;

impl Plugin for TrackPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TrackToolState>()
            .add_systems(Startup, (sync_track_terrain_from_map, setup_build_feedback))
            .add_systems(
                Update,
                (
                    track_tool_input.after(SelectionInputSet),
                    sync_track_ghosts.after(track_tool_input),
                    update_build_feedback_ui.after(track_tool_input),
                    sync_flash_sprites.after(update_build_feedback_ui),
                    apply_track_sprites,
                ),
            );
    }
}
