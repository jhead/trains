//! Local town growth driven by station service scores.
//!
//! # Service-score contract
//!
//! Growth reads [`crate::stations::StationService`] (and station tiles from
//! [`crate::stations::StationRegistry`]) each Advance tick:
//!
//! - `StationServiceScore::score` is `0..=100` — quality of service.
//! - Higher score → higher target building density in a ring around the station.
//! - Lower / decaying score → density stagnates then shrinks toward the new target.
//!
//! Trains / economy **write** scores via [`StationService::record_arrival`],
//! [`StationService::set_waiting`], and [`StationService::tick_decay`]. Town
//! treats the resource as read-only input.

mod growth;

pub use growth::{
    advance_town_growth, density_target_at, growth_due, town_falloff, TownDensity,
    GROWTH_APPROACH_RATE, GROWTH_INTERVAL_TICKS, GROWTH_PASSES_PER_DAY, GROWTH_RADIUS, MAX_DENSITY,
};

use bevy_app::{App, FixedUpdate, Plugin};
use bevy_ecs::schedule::IntoScheduleConfigs;

use crate::{sim_is_running, SimSet};

/// Registers town density and the growth Advance system.
pub struct TownPlugin;

impl Plugin for TownPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TownDensity>().add_systems(
            FixedUpdate,
            advance_town_growth
                .in_set(SimSet::Advance)
                .run_if(sim_is_running),
        );
    }
}
