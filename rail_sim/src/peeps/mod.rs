//! Peeps — named residents who actually make journeys.
//!
//! Brief 06 §4 in one paragraph: peeps have full names, households, routines
//! and memories; they walk from their front door to a station, wait, board,
//! ride, alight, walk on, spend time and come home; their mood is caused by
//! that experience and expressed on the sprite, in Town Talk and in what they
//! decide to do next. A bounded set runs in full detail biased toward the
//! camera ([`budget`]); everyone else folds into district flow ([`flow`]).
//!
//! # Module map
//!
//! | Module | Owns |
//! | --- | --- |
//! | [`names`] | The combinatorial name pool and portrait seeds |
//! | [`household`] | Families that share a building and move together |
//! | [`routine`] | Home, destination, and a habitual travel time |
//! | [`memory`] | Recent journeys and the patience they buy |
//! | [`journey`] | The journey state machine and peep positions |
//! | [`budget`] | Bounded full simulation biased to the camera |
//! | [`flow`] | District-level flow for the abstracted majority |
//! | [`resident`] | Identity, spawning, wait / mood, moving away |
//! | [`walk`] | Terrain-aware walking routes over walkable ground |
//! | [`complaints`] | The public Town Talk feed |
//!
//! # Reading peeps from presentation
//!
//! Everything the Inspector needs is a plain component read:
//! `(&Peep, &Routine, &Journey, &PeepPosition, &JourneyMemory, &WaitingAtStation)`.
//! Nothing in this module reads a camera — presentation publishes [`PeepFocus`].

mod budget;
mod complaints;
mod flow;
mod household;
mod journey;
mod memory;
mod names;
mod resident;
mod routine;
mod walk;

pub use budget::{
    rebalance_peep_detail, select_detailed, PeepBudget, PeepDetail, PeepFocus,
    DEFAULT_FOCUS_RADIUS, DETAIL_REBALANCE_TICKS, MAX_DETAILED_PEEPS,
};
pub use complaints::{
    ComplaintEntry, ComplaintFeed, TalkKind, TownTalkEntry, TownTalkFeed, COMPLAINT_DEDUPE_TICKS,
    COMPLAINT_WAIT_SECS, gave_up_minutes, GAVE_UP_WAIT_FLAG, MAX_COMPLAINTS, MAX_TOWN_TALK,
};
pub use flow::{
    abstract_ride_ticks, advance_abstract_flow, begin_flow_window, district_is_served,
    DistrictFlow, DistrictFlowState, ABSTRACT_ARRIVAL_WINDOW_TICKS, ABSTRACT_SERVICE_MIN,
    FLOW_DECAY_TICKS, MAX_PENDING_TRIPS,
};
pub use household::{
    Household, HouseholdId, HouseholdRegistry, HOUSEHOLD_MAX, HOUSEHOLD_MIN,
};
pub use journey::{
    advance_journeys, boardable_train, station_track, tick_clock_label, Facing, Journey,
    JourneyLeg, JourneyStage, PeepPosition, ARRIVE_EPSILON, BOARD_TICKS, STEP_FRAME_TICKS,
    WALK_TICKS_PER_TILE, WALK_TILES_PER_TICK,
};
pub use memory::{
    outcome_for, JourneyMemory, JourneyOutcome, JourneyRecord, BAD_JOURNEYS_TO_LEAVE,
    BASE_PATIENCE_SECS, GOOD_JOURNEY_SECS, MAX_PATIENCE_SECS, MEMORY_DEPTH, MIN_PATIENCE_SECS,
};
pub use names::{
    family_name, family_plural, full_name, given_name, portrait_variant, BodyType, FAMILY_NAMES,
    GIVEN_NAMES, NAME_POOL_SIZE, PORTRAIT_VARIANTS,
};
pub use resident::{
    advance_peep_waits, district_capacity, mood_from_experience, peeps_move_away,
    spawn_peep_households, Mood, Peep, WaitingAtStation, HOME_MAX_RADIUS, HOME_MIN_RADIUS,
    HOUSEHOLDS_PER_STATION, MAX_PEEPS_PER_STATION, MAX_TOWN_POPULATION, MOVE_IN_INTERVAL_TICKS,
    PEEPS_PER_DENSITY, PEEPS_PER_STATION, REPOPULATE_SCORE, SIM_SECONDS_PER_TICK,
};
pub use routine::{
    clock_label, minute_in_window, PeepRole, Routine, DAY_MINUTES, DEPART_WINDOW_MINUTES,
};
pub use walk::{
    ensure_walk_routes, find_walk_route, find_walk_route_within, walk_step, WalkRoute, WalkRouter,
    WalkStep, WalkWorld, NO_ROUTE_TALK_COOLDOWN_TICKS, WALK_CLIMB_COST, WALK_MAX_HEIGHT,
    WALK_MAX_STEP_GRADE, WALK_ROUTES_PER_TICK, WALK_SEARCH_LIMIT, WALK_STEP_COST,
};

/// Back-compat alias for the pre-journey spawn system name.
pub use resident::spawn_peep_households as spawn_peeps_for_stations;

/// Stable id for a named resident (shared by feed entries and sprites).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct PeepId(pub u64);

/// FixedUpdate ticks in one peep day, at [`SIM_SECONDS_PER_TICK`] per tick.
pub const TICKS_PER_DAY: u64 = (DAY_MINUTES as u64 * 60) / SIM_SECONDS_PER_TICK as u64;

/// Minute-of-day (`0..`[`DAY_MINUTES`]) for a sim tick.
pub fn minute_of_day(tick: u64) -> u32 {
    let secs = (tick % TICKS_PER_DAY).saturating_mul(SIM_SECONDS_PER_TICK as u64);
    (secs / 60) as u32
}

/// Which sim day a tick falls on.
pub fn day_index(tick: u64) -> u64 {
    tick / TICKS_PER_DAY
}

use bevy_app::{App, FixedUpdate, Plugin};
use bevy_ecs::schedule::IntoScheduleConfigs;

use crate::{sim_is_running, SimSet};

/// Registers peep resources and the Advance chain.
///
/// Order matters and is explicit: the district window opens, households seed,
/// every peep is guaranteed a walk-route cache, level of detail is chosen,
/// waits and moods update, then the full-detail journeys and the abstracted
/// flow advance, and finally anyone who has had enough packs up.
pub struct PeepsPlugin;

impl Plugin for PeepsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ComplaintFeed>()
            .init_resource::<PeepSpawnState>()
            .init_resource::<HouseholdRegistry>()
            .init_resource::<DistrictFlow>()
            .init_resource::<PeepBudget>()
            .init_resource::<PeepFocus>()
            .init_resource::<WalkRouter>()
            .add_systems(
                FixedUpdate,
                (
                    begin_flow_window,
                    spawn_peep_households,
                    // Peeps restored from a save arrive without a route cache;
                    // this gives them one before anybody tries to walk.
                    ensure_walk_routes,
                    rebalance_peep_detail,
                    advance_peep_waits,
                    advance_journeys,
                    advance_abstract_flow,
                    peeps_move_away,
                )
                    .chain()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_is_a_whole_number_of_ticks() {
        assert_eq!(TICKS_PER_DAY, 8640);
        assert_eq!(minute_of_day(0), 0);
        assert_eq!(minute_of_day(TICKS_PER_DAY), 0);
        assert_eq!(day_index(TICKS_PER_DAY), 1);
        assert_eq!(day_index(TICKS_PER_DAY - 1), 0);
    }

    #[test]
    fn minute_of_day_advances_six_ticks_per_minute() {
        assert_eq!(minute_of_day(6), 1);
        assert_eq!(minute_of_day(6 * 60), 60);
        assert!(minute_of_day(TICKS_PER_DAY - 1) < DAY_MINUTES);
    }
}
