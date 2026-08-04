//! Drain [`CommandBuffer`] on FixedUpdate and apply / forward commands.

use bevy_ecs::prelude::*;

use crate::clock::SimClock;
use crate::command_buffer::CommandBuffer;
use crate::commands::{CommandKind, SimCommand};

/// Emitted for world-mutating commands after Pause / SetSpeed are handled.
///
/// Track / train / economy systems should read these (after [`crate::SimSet::ApplyCommands`])
/// and implement PlaceTrack, Demolish, AutoFillTrack, BuyTrain, PlaceTrain.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct PendingWorldCommand {
    pub command: SimCommand,
}

/// Drain the command buffer and apply core handlers.
///
/// - [`CommandKind::Pause`] / [`CommandKind::SetSpeed`] update [`SimClock`] fully.
/// - Other kinds are forwarded as [`PendingWorldCommand`] for domain systems
///   (track apply + train buy/place).
pub fn apply_commands(
    mut buffer: ResMut<CommandBuffer>,
    mut clock: ResMut<SimClock>,
    mut pending: MessageWriter<PendingWorldCommand>,
) {
    for command in buffer.drain() {
        match &command.kind {
            CommandKind::Pause(pause) => {
                clock.apply_pause(*pause);
            }
            CommandKind::SetSpeed(speed) => {
                clock.apply_set_speed(*speed);
            }
            CommandKind::PlaceTrack(_)
            | CommandKind::Demolish(_)
            | CommandKind::AutoFillTrack(_)
            | CommandKind::AutoFillPath(_)
            | CommandKind::BuyTrain(_)
            | CommandKind::PlaceTrain(_)
            | CommandKind::SellTrain(_)
            | CommandKind::CreateLine(_)
            | CommandKind::AssignTrainToLine(_)
            | CommandKind::UnassignTrain(_)
            | CommandKind::PlaceStation(_)
            | CommandKind::DemolishStation(_)
            | CommandKind::UpgradeStation(_)
            | CommandKind::OpenBorder(_)
            | CommandKind::CloseBorder(_)
            | CommandKind::SetBorderTrade(_)
            | CommandKind::AssignTrainToBorder(_) => {
                pending.write(PendingWorldCommand {
                    command: command.clone(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::App;
    use bevy_app::FixedUpdate;
    use bevy_ecs::message::Messages;

    use crate::commands::{Pause, PlaceTrack, SetSpeed};
    use crate::ids::TileCoord;
    use crate::SimPlugin;

    fn drain_pending(app: &mut App) -> Vec<PendingWorldCommand> {
        let mut messages = app
            .world_mut()
            .resource_mut::<Messages<PendingWorldCommand>>();
        messages.drain().collect()
    }

    #[test]
    fn apply_commands_updates_clock_and_forwards_track() {
        let mut app = App::new();
        app.add_plugins(SimPlugin);

        {
            let mut buf = app.world_mut().resource_mut::<CommandBuffer>();
            buf.push(CommandKind::Pause(Pause { paused: true }));
            buf.push(CommandKind::SetSpeed(SetSpeed { multiplier: 3 }));
            buf.push(CommandKind::PlaceTrack(PlaceTrack {
                tile: TileCoord { x: 1, y: 2 },
                layer: 0,
            }));
        }

        app.world_mut().run_schedule(FixedUpdate);

        let clock = app.world().resource::<SimClock>();
        // SetSpeed after Pause unpauses and sets 3x.
        assert!(!clock.paused);
        assert_eq!(clock.speed_multiplier, 3);

        let pending = drain_pending(&mut app);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].command.sequence, 3);
        assert!(matches!(
            pending[0].command.kind,
            CommandKind::PlaceTrack(_)
        ));
    }

    #[test]
    fn track_apply_places_when_terrain_present() {
        use crate::track::{TrackEdit, TrackNetwork, TrackTerrain, TRACK_COST_CENTS};
        use bevy_ecs::message::Messages;
        use crate::commands::Pause;
        use crate::money::Money;

        let mut app = App::new();
        app.add_plugins(SimPlugin);
        app.insert_resource(TrackTerrain::new(8, 8, (0..64).map(|_| (false, 0i8))));
        // Pause so Advance (opex / maintenance) does not run during place.
        app.world_mut()
            .resource_mut::<crate::SimClock>()
            .apply_pause(Pause { paused: true });

        {
            let mut buf = app.world_mut().resource_mut::<CommandBuffer>();
            buf.push(CommandKind::PlaceTrack(PlaceTrack {
                tile: TileCoord { x: 3, y: 3 },
                layer: 0,
            }));
        }

        app.world_mut().run_schedule(FixedUpdate);

        assert_eq!(app.world().resource::<TrackNetwork>().len(), 1);
        assert_eq!(
            app.world().resource::<Money>().cents(),
            crate::money::STARTING_CASH_CENTS - TRACK_COST_CENTS
        );

        let edits: Vec<TrackEdit> = app
            .world_mut()
            .resource_mut::<Messages<TrackEdit>>()
            .drain()
            .collect();
        assert_eq!(edits.len(), 1);
        assert!(matches!(edits[0], TrackEdit::Placed { .. }));

        let history = app.world().resource::<crate::CommandHistory>();
        assert_eq!(history.undo_len(), 1);
    }

    #[test]
    fn undo_place_restores_empty_network_and_money() {
        use crate::history::CommandHistory;
        use crate::track::{TrackNetwork, TrackTerrain, TRACK_COST_CENTS};
        use crate::commands::Pause;
        use crate::money::Money;

        let mut app = App::new();
        app.add_plugins(SimPlugin);
        app.insert_resource(TrackTerrain::new(8, 8, (0..64).map(|_| (false, 0i8))));
        app.world_mut()
            .resource_mut::<crate::SimClock>()
            .apply_pause(Pause { paused: true });

        {
            let mut buf = app.world_mut().resource_mut::<CommandBuffer>();
            buf.push(CommandKind::PlaceTrack(PlaceTrack {
                tile: TileCoord { x: 3, y: 3 },
                layer: 0,
            }));
        }
        app.world_mut().run_schedule(FixedUpdate);
        assert_eq!(app.world().resource::<TrackNetwork>().len(), 1);

        let inverse = {
            let mut history = app.world_mut().resource_mut::<CommandHistory>();
            history.begin_undo().expect("one undo entry")
        };
        {
            let mut buf = app.world_mut().resource_mut::<CommandBuffer>();
            for kind in inverse {
                buf.push(kind);
            }
        }
        app.world_mut().run_schedule(FixedUpdate);

        assert!(app.world().resource::<TrackNetwork>().is_empty());
        assert_eq!(
            app.world().resource::<Money>().cents(),
            crate::money::STARTING_CASH_CENTS
        );
        assert!(app.world().resource::<CommandHistory>().can_redo());
        let _ = TRACK_COST_CENTS;
    }
}
