//! Drain [`CommandBuffer`] on FixedUpdate and apply / forward commands.

use std::sync::Once;

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
/// - Other kinds are forwarded as [`PendingWorldCommand`] for domain systems.
/// - Stub arms also log once so missing handlers are obvious during integration.
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
            CommandKind::PlaceTrack(_) => {
                log_stub_once("PlaceTrack");
                pending.write(PendingWorldCommand {
                    command: command.clone(),
                });
            }
            CommandKind::Demolish(_) => {
                log_stub_once("Demolish");
                pending.write(PendingWorldCommand {
                    command: command.clone(),
                });
            }
            CommandKind::AutoFillTrack(_) => {
                log_stub_once("AutoFillTrack");
                pending.write(PendingWorldCommand {
                    command: command.clone(),
                });
            }
            CommandKind::BuyTrain(_) => {
                log_stub_once("BuyTrain");
                pending.write(PendingWorldCommand {
                    command: command.clone(),
                });
            }
            CommandKind::PlaceTrain(_) => {
                log_stub_once("PlaceTrain");
                pending.write(PendingWorldCommand {
                    command: command.clone(),
                });
            }
        }
    }
}

fn log_stub_once(name: &'static str) {
    match name {
        "PlaceTrack" => {
            static ONCE: Once = Once::new();
            ONCE.call_once(|| {
                eprintln!(
                    "rail_sim: PlaceTrack apply stub — track agent should handle PendingWorldCommand"
                );
            });
        }
        "Demolish" => {
            static ONCE: Once = Once::new();
            ONCE.call_once(|| {
                eprintln!(
                    "rail_sim: Demolish apply stub — track agent should handle PendingWorldCommand"
                );
            });
        }
        "AutoFillTrack" => {
            static ONCE: Once = Once::new();
            ONCE.call_once(|| {
                eprintln!(
                    "rail_sim: AutoFillTrack apply stub — track agent should handle PendingWorldCommand"
                );
            });
        }
        "BuyTrain" => {
            static ONCE: Once = Once::new();
            ONCE.call_once(|| {
                eprintln!(
                    "rail_sim: BuyTrain apply stub — economy/train agent should handle PendingWorldCommand"
                );
            });
        }
        "PlaceTrain" => {
            static ONCE: Once = Once::new();
            ONCE.call_once(|| {
                eprintln!(
                    "rail_sim: PlaceTrain apply stub — train agent should handle PendingWorldCommand"
                );
            });
        }
        _ => {}
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
}
