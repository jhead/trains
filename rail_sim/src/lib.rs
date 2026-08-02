//! Rail Town simulation library.
//!
//! ECS-friendly sim types and systems. No rendering or windowing deps.
//! Player intent flows through [`commands`]; [`apply::apply_commands`] drains
//! the [`command_buffer::CommandBuffer`] on the fixed tick.
//! Track placement runs in [`SimSet::ApplyCommands`] after that drain.

pub mod apply;
pub mod clock;
pub mod command_buffer;
pub mod commands;
pub mod event_director;
pub mod ids;
pub mod money;
pub mod track;

pub use apply::{apply_commands, PendingWorldCommand};
pub use clock::{sim_is_running, SimClock, SimSpeed};
pub use command_buffer::CommandBuffer;
pub use commands::{
    AutoFillTrack, BuyTrain, CommandKind, Demolish, Pause, PlaceTrack, PlaceTrain, SetSpeed,
    SimCommand, TrainKind,
};
pub use event_director::EventDirector;
pub use ids::{EntityId, StationId, TileCoord, TrackId, TrainId};
pub use money::{InsufficientFunds, Money, STARTING_CASH_CENTS};
pub use track::{
    apply_track_commands, TrackEdit, TrackNetwork, TrackPiece, TrackTerrain, BRIDGE_COST_CENTS,
    GROUND_LAYER, MAX_BRIDGE_SPAN, TRACK_COST_CENTS,
};

use bevy_app::{App, FixedUpdate, Plugin};
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

/// Registers sim resources and FixedUpdate command drain + track apply.
pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CommandBuffer>()
            .init_resource::<SimClock>()
            .init_resource::<EventDirector>()
            .init_resource::<TrackNetwork>()
            .insert_resource(Money::sandbox_starting())
            .add_message::<PendingWorldCommand>()
            .add_message::<TrackEdit>()
            .configure_sets(
                FixedUpdate,
                (SimSet::ApplyCommands, SimSet::Advance).chain(),
            )
            .add_systems(
                FixedUpdate,
                (
                    apply_commands,
                    apply_track_commands.after(apply_commands),
                )
                    .in_set(SimSet::ApplyCommands),
            );
    }
}
