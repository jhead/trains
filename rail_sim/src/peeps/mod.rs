//! Named residents, wait-time tracking, and the public Town Talk feed.
//!
//! Peeps wait at stations from [`crate::stations::StationRegistry`]. Wait
//! accumulates faster when [`crate::stations::StationService`] score is low.
//! Crossing the complaint threshold pushes a line into [`ComplaintFeed`].

mod complaints;
mod resident;

pub use complaints::{
    ComplaintEntry, ComplaintFeed, TalkKind, TownTalkEntry, TownTalkFeed, COMPLAINT_DEDUPE_TICKS,
    COMPLAINT_WAIT_SECS, MAX_COMPLAINTS, MAX_TOWN_TALK,
};
pub use resident::{
    advance_peep_waits, spawn_peeps_for_stations, Mood, Peep, WaitingAtStation, PEEPS_PER_STATION,
    SIM_SECONDS_PER_TICK,
};

/// Stable id for a named resident (shared by feed entries and sprites).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PeepId(pub u64);

use bevy_app::{App, FixedUpdate, Plugin};
use bevy_ecs::schedule::IntoScheduleConfigs;

use crate::{sim_is_running, SimSet};

/// Registers peep resources and Advance systems (spawn + wait / complain).
pub struct PeepsPlugin;

impl Plugin for PeepsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ComplaintFeed>()
            .init_resource::<PeepSpawnState>()
            .add_systems(
                FixedUpdate,
                (
                    spawn_peeps_for_stations,
                    advance_peep_waits.after(spawn_peeps_for_stations),
                )
                    .in_set(SimSet::Advance)
                    .run_if(sim_is_running),
            );
    }
}

/// Tracks how many peeps have been spawned so we can assign stable ids / names.
#[derive(Debug, Clone, Default, bevy_ecs::prelude::Resource)]
pub struct PeepSpawnState {
    pub next_id: u64,
    pub spawned_for: std::collections::HashSet<crate::ids::StationId>,
}
