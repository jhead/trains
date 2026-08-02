//! Money / clock / tool HUD and complaint feed.

mod complaints;
mod hud;

use bevy::prelude::*;

use complaints::{setup_complaint_feed_ui, update_complaint_feed_ui};
use hud::{setup_hud, update_hud};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (setup_hud, setup_complaint_feed_ui))
            .add_systems(Update, (update_hud, update_complaint_feed_ui));
    }
}
