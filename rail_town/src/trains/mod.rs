//! Train buy / place tools, the facing bank, and the sprites that read it.

mod bank;
mod tools;
mod visuals;

use bevy::prelude::*;

use tools::{apply_confirmed_sell, sell_selected_train_input, train_tool_input};
use visuals::{sync_train_smoke, sync_train_sprites, sync_train_stop_indicators};

use crate::inspect::SelectionInputSet;

pub use tools::{arm_train_place, TrainPlaceKind, TrainToolState};
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
                    // Before the tool input, so the frame `X` opens the sell
                    // dialog is not also the frame the tool disarms itself and
                    // the track tool starts demolishing under the modal.
                    sell_selected_train_input
                        .before(train_tool_input)
                        .in_set(crate::input::PlayerVerbSet),
                    apply_confirmed_sell.after(train_tool_input),
                    sync_train_sprites,
                    sync_train_stop_indicators.after(sync_train_sprites),
                    sync_train_smoke.after(sync_train_sprites),
                ),
            );
    }
}
