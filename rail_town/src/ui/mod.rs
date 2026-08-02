//! Pixel UI kit, status strip, toolbar, undo input, and track SFX.
//!
//! **Bitmap font follow-up:** see [`kit`] — integer Bevy font sizes until a
//! true bitmap pixel font ships.

mod complaints;
mod kit;
#[cfg(feature = "sfx")]
mod sfx;
mod status_strip;
mod toolbar;
mod undo;

use bevy::prelude::*;

use complaints::{setup_complaint_feed_ui, update_complaint_feed_ui};
use kit::pointer_blocks_world;
use status_strip::{setup_status_strip, speed_button_clicks, update_status_strip};
use toolbar::{setup_toolbar, toolbar_button_clicks, update_toolbar_visuals};
use undo::undo_redo_input;

/// True while the pointer is over a UI button (world clicks should ignore).
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct UiBlocksWorld(pub bool);

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiBlocksWorld>()
            .add_systems(
                Startup,
                (setup_status_strip, setup_toolbar, setup_complaint_feed_ui),
            )
            .add_systems(
                Update,
                (
                    sync_ui_blocks_world,
                    update_status_strip,
                    speed_button_clicks,
                    update_toolbar_visuals,
                    toolbar_button_clicks,
                    update_complaint_feed_ui,
                    undo_redo_input,
                ),
            );

        #[cfg(feature = "sfx")]
        {
            app.add_systems(Startup, sfx::setup_track_sfx)
                .add_systems(Update, sfx::play_track_sfx);
        }
    }
}

fn sync_ui_blocks_world(
    interactions: Query<&Interaction, With<Button>>,
    mut blocks: ResMut<UiBlocksWorld>,
) {
    blocks.0 = pointer_blocks_world(&interactions);
}
