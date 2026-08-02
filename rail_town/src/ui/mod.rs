//! Pixel UI kit, status strip, toolbar, undo input, Town Talk, ledger, alerts, SFX.
//!
//! **Bitmap font follow-up:** see [`kit`] — integer Bevy font sizes until a
//! true bitmap pixel font ships.

mod alerts;
pub(crate) mod kit;
mod ledger;
#[cfg(feature = "sfx")]
mod sfx;
mod status_strip;
mod toolbar;
mod town_talk;
mod undo;

use bevy::prelude::*;

use alerts::{
    alert_dismiss_all_clicks, alert_row_clicks, setup_alerts_ui, update_alert_row_hover,
    update_alerts_ui,
};
use kit::pointer_blocks_world;
use ledger::{
    ledger_toggle_input, setup_ledger_ui, update_ledger_panel, update_ledger_toggle_visual,
};
use status_strip::{setup_status_strip, speed_button_clicks, update_status_strip};
use toolbar::{setup_toolbar, toolbar_button_clicks, update_toolbar_visuals};
use town_talk::{refresh_town_talk_rows, setup_town_talk_ui, town_talk_clicks};
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
                (
                    setup_status_strip,
                    setup_toolbar,
                    setup_town_talk_ui,
                    setup_ledger_ui,
                    setup_alerts_ui,
                ),
            )
            .add_systems(
                Update,
                (
                    sync_ui_blocks_world,
                    update_status_strip,
                    speed_button_clicks,
                    update_toolbar_visuals,
                    toolbar_button_clicks,
                    refresh_town_talk_rows,
                    town_talk_clicks,
                    ledger_toggle_input,
                    update_ledger_panel,
                    update_ledger_toggle_visual,
                    update_alerts_ui,
                    alert_row_clicks,
                    alert_dismiss_all_clicks,
                    update_alert_row_hover,
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
    interactions: Query<&Interaction, Or<(With<Button>, With<kit::WorldClickBlocker>)>>,
    mut blocks: ResMut<UiBlocksWorld>,
) {
    blocks.0 = pointer_blocks_world(&interactions);
}
