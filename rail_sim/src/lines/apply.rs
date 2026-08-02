//! Apply CreateLine / AssignTrainToLine from [`PendingWorldCommand`].

use bevy_ecs::prelude::*;

use crate::apply::PendingWorldCommand;
use crate::commands::CommandKind;
use crate::stations::StationRegistry;
use crate::trains::TrainOnLine;

use super::registry::{suggest_line_name, LineRegistry};

/// Drain line-related pending commands.
pub fn apply_line_commands(
    mut pending: MessageReader<PendingWorldCommand>,
    mut lines: ResMut<LineRegistry>,
    stations: Res<StationRegistry>,
    mut trains: Query<(Entity, &crate::trains::Train, Option<&mut TrainOnLine>)>,
    mut commands: Commands,
) {
    for msg in pending.read() {
        match &msg.command.kind {
            CommandKind::CreateLine(c) => {
                if c.stops.len() < 2 {
                    continue;
                }
                // Validate stations exist.
                if c.stops.iter().any(|s| stations.get(*s).is_none()) {
                    continue;
                }
                let name = c
                    .name
                    .clone()
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| suggest_line_name(&stations, &c.stops));
                let _ = lines.create(name, c.stops.clone());
            }
            CommandKind::AssignTrainToLine(a) => {
                if !lines.assign_train(a.line, a.train) {
                    continue;
                }
                let mut found = false;
                for (entity, train, on_line) in trains.iter_mut() {
                    if train.id != a.train {
                        continue;
                    }
                    found = true;
                    if let Some(mut on) = on_line {
                        on.line = a.line;
                        on.next_stop = 0;
                        on.forward = true;
                    } else {
                        commands.entity(entity).insert(TrainOnLine {
                            line: a.line,
                            next_stop: 0,
                            forward: true,
                        });
                    }
                    break;
                }
                if !found {
                    // Train not spawned yet — assignment lives on the registry;
                    // component is attached when / if the train exists.
                    let _ = found;
                }
            }
            CommandKind::UnassignTrain(u) => {
                lines.unassign_train(u.train);
                for (entity, train, on_line) in trains.iter_mut() {
                    if train.id == u.train && on_line.is_some() {
                        commands.entity(entity).remove::<TrainOnLine>();
                    }
                }
            }
            _ => {}
        }
    }
}
