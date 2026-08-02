//! Rail Town simulation library.
//!
//! ECS-friendly sim types and systems. No rendering or windowing deps.
//! Player intent flows through [`commands`]; [`apply::apply_commands`] drains
//! the [`command_buffer::CommandBuffer`] on the fixed tick.
//! Track placement runs in [`SimSet::ApplyCommands`] after that drain.
//! Trains / demand / opex run in [`SimSet::Advance`] when the sim is running.

pub mod apply;
pub mod border;
pub mod clock;
pub mod command_buffer;
pub mod commands;
pub mod demand;
pub mod economy;
pub mod event_director;
pub mod goals;
pub mod history;
pub mod ids;
pub mod lines;
pub mod money;
pub mod peeps;
pub mod save;
pub mod stations;
pub mod town;
pub mod track;
pub mod trains;

pub use apply::{apply_commands, PendingWorldCommand};
pub use border::{
    advance_border_trade, apply_border_commands, echo_manifest, push_border_command,
    railhead_on_edge, try_close_border, try_open_border, AssignTrainToBorder, BorderCommand,
    BorderEdge, BorderEdit, BorderError, BorderLink, BorderManifest, BorderPlugin, BorderRegistry,
    BorderRun, CloseBorder, LinkId, OpenBorder, PresenceSource, SetBorderTrade, StandingOffer,
    StandingRequest, TransitTrain, BORDER_ARRIVAL_CENTS, BORDER_CROSSING_TICKS,
    BORDER_PORTAL_COST_CENTS, MANIFEST_SCHEMA_VERSION,
};
pub use clock::{sim_is_running, SimClock, SimSpeed};
pub use command_buffer::CommandBuffer;
pub use commands::{
    AssignTrainToLine, AutoFillTrack, BuyTrain, CommandKind, CreateLine, Demolish, Pause,
    PlaceTrack, PlaceTrain, SetSpeed, SimCommand, TrainKind, UnassignTrain,
};
pub use history::{CommandHistory, HistoryEntry, HistoryMode, HISTORY_DEPTH};
pub use demand::{
    service_influence_at, spawn_new_demand, DemandOpportunity, DemandOpportunityKind,
    DemandSpawner, DEMAND_FIRST_DELAY_SIM_MINUTES, DEMAND_INTERVAL_SIM_MINUTES,
    DEMAND_MAX_NEW_PER_SESSION, DEMAND_MIN_ANCHOR_SPACING, DEMAND_SERVICE_INFLUENCE_MAX,
};
pub use economy::{
    apply_track_maintenance, apply_train_opex, assign_jobs, drain_peep_demand, refresh_alerts,
    resolve_deliveries, spawn_demand_jobs, sync_peep_platform_pressure, tick_money_ledger,
    track_maintenance_total, Alert, AlertBoard, AlertFocus, AlertKind, AlertKey, Job, JobBoard,
    JobKind, MaintenanceAccrual, MoneyCategory, MoneyLedger, ALERT_CASH_LOW_MINUTES,
    ALERT_SERVICE_LOW_SCORE, ALERT_WAITING_OVERWHELMED, GOODS_DELIVERY_CENTS, LEDGER_HISTORY_LEN,
    LEDGER_SAMPLE_SIM_SECS, PASSENGER_FARE_CENTS, TRAIN_OPEX_CENTS,
};
pub use event_director::EventDirector;
pub use goals::{
    evaluate_goals, generate_goal_set, generate_goals_once, Goal, GoalBoard, GoalId, GoalKind,
    GoalMode, GoalStatus, GoalsPlugin, GOALS_PER_SET,
};
pub use ids::{EntityId, LineId, StationId, TileCoord, TrackId, TrainId};
pub use lines::{
    apply_line_commands, line_colour_rgba, line_path, suggest_line_name, Line, LineColour,
    LineDirection, LineRegistry, LINE_PALETTE,
};
pub use money::{InsufficientFunds, Money, STARTING_CASH_CENTS};
pub use peeps::{
    advance_abstract_flow, advance_journeys, advance_peep_waits, begin_flow_window, day_index,
    district_capacity, gave_up_minutes, minute_of_day, mood_from_experience, peeps_move_away,
    rebalance_peep_detail, spawn_peep_households, BodyType, ComplaintEntry, ComplaintFeed,
    DistrictFlow, DistrictFlowState, Facing, Household, HouseholdId, HouseholdRegistry, Journey,
    JourneyLeg, JourneyMemory, JourneyOutcome, JourneyRecord, JourneyStage, Mood, Peep, PeepBudget,
    PeepDetail, PeepFocus, PeepId, PeepPosition, PeepRole, PeepsPlugin, Routine, TalkKind,
    TownTalkEntry, TownTalkFeed, WaitingAtStation, BAD_JOURNEYS_TO_LEAVE, COMPLAINT_DEDUPE_TICKS,
    COMPLAINT_WAIT_SECS, DAY_MINUTES, GAVE_UP_WAIT_FLAG, MAX_COMPLAINTS, MAX_DETAILED_PEEPS,
    MAX_TOWN_TALK, MEMORY_DEPTH, PEEPS_PER_STATION, SIM_SECONDS_PER_TICK, TICKS_PER_DAY,
};
pub use save::{
    autosave, delete_slot, list_slots, load_from_slot, queue_autosave, queue_save, save_to_slot,
    save_to_slot_async, SaveError, SaveJobs, SaveMeta, SavePlugin, SaveResult, SaveSlot, SlotInfo,
    WorldSnapshot, AUTOSAVE_SLOTS, SCHEMA_VERSION,
};
pub use stations::{
    apply_station_commands, catchment_influence, push_station_command,
    seed_stations_and_industries, station_maintenance_total, DemolishStation, GoodKind, Industry,
    IndustryId, IndustryRegistry, PlaceStation, Station, StationCommand, StationEdit,
    StationPlacementError, StationRegistry, StationService, StationServiceScore, StationTier,
    StationTierSpec, UpgradeStation, HALT_COST_CENTS, INTERCHANGE_COST_CENTS, MIN_STATION_SPACING,
    STATION_COST_CENTS, TERMINUS_COST_CENTS,
};
pub use town::{TownDensity, TownPlugin, GROWTH_RADIUS, MAX_DENSITY};
pub use track::{
    apply_track_commands, bridge_cost_for_span, local_slope, path_bridge_spans_ok, path_grades_ok,
    piece_maintenance_cents, straight_line, tile_build_cost, tile_cost, validate_tile_empty,
    PlacementError, TrackEdit, TrackNetwork, TrackPiece, TrackTerrain, BRIDGE_COST_CENTS,
    BRIDGE_MAINT_CENTS, GROUND_LAYER, MAX_BRIDGE_SPAN, MAX_CURVE, MAX_GRADE, MOUNTAIN_HEIGHT_MIN,
    TRACK_COST_CENTS, TRACK_MAINT_CENTS,
};
pub use trains::{
    advance_trains, apply_train_commands, blocked_chain_head, blocker_for, buy_cost, find_path,
    find_path_avoiding, find_path_for_kind,
    ticks_for_piece, track_for_station, TileOccupancy, Train, TrainCargo, TrainEdit, TrainLocation,
    TrainOnLine, TrainProfile, TrainYard, TRANSIT_COST_CENTS, TRANSIT_PROFILE, TRANSPORT_COST_CENTS,
    TRANSPORT_PROFILE,
};

use bevy_app::{App, FixedUpdate, Plugin, Update};
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::{IntoScheduleConfigs, SystemSet};

/// Ordered sim system sets on [`FixedUpdate`].
///
/// Track / train / economy handlers should run in [`SimSet::ApplyCommands`]
/// *after* [`apply_commands`] (read [`PendingWorldCommand`]), or in
/// [`SimSet::Advance`] with `.run_if(sim_is_running)`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimSet {
    /// Drain command buffer; apply pause/speed; emit world-command messages.
    ApplyCommands,
    /// Tick the living sim (trains, demand, growth). Gate with [`sim_is_running`].
    Advance,
}

#[derive(Resource, Default)]
pub struct WorldAnchorsSeeded(pub bool);

/// Registers sim resources and FixedUpdate command drain + track/train apply + advance.
pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CommandBuffer>()
            .init_resource::<CommandHistory>()
            .init_resource::<SimClock>()
            .init_resource::<EventDirector>()
            .init_resource::<TrackNetwork>()
            .init_resource::<StationRegistry>()
            .init_resource::<IndustryRegistry>()
            .init_resource::<StationService>()
            .init_resource::<TrainYard>()
            .init_resource::<TileOccupancy>()
            .init_resource::<JobBoard>()
            .init_resource::<MoneyLedger>()
            .init_resource::<MaintenanceAccrual>()
            .init_resource::<AlertBoard>()
            .init_resource::<DemandSpawner>()
            .init_resource::<LineRegistry>()
            .init_resource::<WorldAnchorsSeeded>()
            .insert_resource(Money::sandbox_starting())
            .add_message::<PendingWorldCommand>()
            .add_message::<TrackEdit>()
            .add_message::<TrainEdit>()
            .add_message::<StationEdit>()
            .configure_sets(
                FixedUpdate,
                (SimSet::ApplyCommands, SimSet::Advance).chain(),
            )
            .add_systems(
                FixedUpdate,
                (
                    apply_commands,
                    // Must precede the track handler: it owns `CommandHistory::finish_replay`,
                    // so a station inverse replayed after it reads as a fresh player action
                    // and wipes the redo stack.
                    apply_station_commands
                        .after(apply_commands)
                        .before(apply_track_commands),
                    apply_track_commands.after(apply_commands),
                    apply_train_commands.after(apply_commands),
                    apply_line_commands.after(apply_commands),
                )
                    .in_set(SimSet::ApplyCommands),
            )
            .add_systems(
                FixedUpdate,
                (
                    spawn_new_demand,
                    // Peep routines decide when people travel; these two carry
                    // that into the railway — platform pressure into the score,
                    // departures onto the job board. Both must precede
                    // `spawn_demand_jobs`, which charges the tick's crowding
                    // penalty from the blended queue.
                    sync_peep_platform_pressure.after(spawn_new_demand),
                    drain_peep_demand.after(sync_peep_platform_pressure),
                    spawn_demand_jobs.after(drain_peep_demand),
                    assign_jobs.after(spawn_demand_jobs),
                    advance_trains.after(assign_jobs),
                    resolve_deliveries.after(advance_trains),
                    apply_train_opex.after(resolve_deliveries),
                    apply_track_maintenance.after(apply_train_opex),
                    tick_station_service.after(apply_track_maintenance),
                    tick_money_ledger.after(tick_station_service),
                    refresh_alerts.after(tick_money_ledger),
                )
                    .in_set(SimSet::Advance)
                    .run_if(sim_is_running),
            )
            .add_systems(Update, seed_world_anchors_once)
            .add_plugins((
                TownPlugin,
                PeepsPlugin,
                goals::GoalsPlugin,
                save::SavePlugin,
                border::BorderPlugin,
            ));
    }
}

fn tick_station_service(mut service: ResMut<StationService>) {
    service.tick_decay();
}

/// Auto-seed stations + industries once [`TrackTerrain`] is available.
fn seed_world_anchors_once(
    mut seeded: ResMut<WorldAnchorsSeeded>,
    terrain: Option<Res<TrackTerrain>>,
    mut stations: ResMut<StationRegistry>,
    mut industries: ResMut<IndustryRegistry>,
    mut service: ResMut<StationService>,
) {
    if seeded.0 {
        return;
    }
    let Some(terrain) = terrain else {
        return;
    };
    seed_stations_and_industries(
        &mut stations,
        &mut industries,
        &mut service,
        terrain.width(),
        terrain.height(),
        |c| {
            terrain.contains(c)
                && !terrain.is_water(c)
                && terrain.height_at(c).unwrap_or(0) < crate::track::MOUNTAIN_HEIGHT_MIN
                && crate::track::local_slope(&terrain, c) <= crate::track::MAX_GRADE + 1
        },
    );
    seeded.0 = true;
}
