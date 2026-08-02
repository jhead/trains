//! Train buy / place tools and placeholder sprites.

mod tools;
mod visuals;

use bevy::prelude::*;

use tools::{train_tool_input, TrainToolState};
use visuals::sync_train_sprites;

pub struct TrainsPlugin;

impl Plugin for TrainsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TrainToolState>()
            .add_systems(Update, (train_tool_input, sync_train_sprites));
    }
}
