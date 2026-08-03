//! Train buy / place tools, the facing bank, and the sprites that read it.

mod bank;
mod tools;
mod visuals;

use bevy::prelude::*;

use tools::train_tool_input;
use visuals::{sync_train_smoke, sync_train_sprites, sync_train_stop_indicators};

use crate::inspect::SelectionInputSet;

pub use tools::{TrainPlaceKind, TrainToolState};
pub use visuals::TrainSprite;

pub struct TrainsPlugin;

impl Plugin for TrainsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TrainToolState>()
            .init_resource::<crate::input::KeyBindings>()
            .add_systems(
                Update,
                (
                    train_tool_input
                        .after(SelectionInputSet)
                        .in_set(crate::input::PlayerVerbSet),
                    sync_train_sprites,
                    sync_train_stop_indicators.after(sync_train_sprites),
                    sync_train_smoke.after(sync_train_sprites),
                ),
            );
    }
}
