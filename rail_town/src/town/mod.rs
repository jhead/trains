//! Placeholder building and peep sprites driven by sim town / peep state.

mod buildings;
mod peep_sprites;

use bevy::prelude::*;

use buildings::sync_building_sprites;
use peep_sprites::{sync_peep_focus, sync_peep_sprites};

// Read by the atmosphere slice's lit-window layer: it draws each building's
// baked window mask over the lot that produced it, so the two line up by
// construction rather than by a second guess at where the windows are.
pub use buildings::building_art::BuildingAtlas;
pub use buildings::BuildingWindows;
#[cfg(test)]
pub use buildings::TownBuildingsPlugin;
pub use peep_sprites::PeepSprite;

pub struct TownPresentationPlugin;

impl Plugin for TownPresentationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                sync_building_sprites,
                // Publish the camera's region of interest before drawing, so the
                // sim's bounded peep simulation follows what the player is watching.
                sync_peep_focus,
                sync_peep_sprites.after(sync_peep_focus),
            ),
        );
    }
}
