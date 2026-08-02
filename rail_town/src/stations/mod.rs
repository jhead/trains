//! Station / industry placeholder sprites.
//!
//! Stations and industries are **auto-seeded** by `rail_sim` once terrain exists
//! (see `rail_sim::seed_stations_and_industries`). This module only draws them.

mod visuals;

use bevy::prelude::*;

use visuals::sync_station_industry_sprites;

pub struct StationsPlugin;

impl Plugin for StationsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, sync_station_industry_sprites);
    }
}
