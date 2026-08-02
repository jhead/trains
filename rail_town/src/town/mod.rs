//! Placeholder building and peep sprites driven by sim town / peep state.

mod buildings;
mod peep_sprites;

use bevy::prelude::*;

use buildings::sync_building_sprites;
use peep_sprites::sync_peep_sprites;

pub struct TownPresentationPlugin;

impl Plugin for TownPresentationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (sync_building_sprites, sync_peep_sprites));
    }
}
