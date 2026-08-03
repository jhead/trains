//! Apply track world-commands after [`crate::apply::apply_commands`].

use bevy_ecs::prelude::*;

use crate::apply::PendingWorldCommand;
use crate::commands::{CommandKind, Demolish, PlaceTrack};
use crate::economy::MoneyLedger;
use crate::history::{CommandHistory, HistoryMode};
use crate::ids::TileCoord;
use crate::money::Money;

use super::network::TrackNetwork;
use super::place::{try_autofill_track, try_demolish, try_place_path, try_place_track};
use super::rules::PlacementError;
use super::terrain::TrackTerrain;

/// Fired when track is added or removed so presentation can bake sprites.
///
/// [`TrackEdit::Failed`] lets UI / audio react to soft-rejected builds.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum TrackEdit {
    Placed {
        id: crate::ids::TrackId,
        tile: crate::ids::TileCoord,
        layer: u8,
        is_bridge: bool,
    },
    Removed {
        id: crate::ids::TrackId,
        tile: crate::ids::TileCoord,
        layer: u8,
    },
    /// Place / autofill / demolish rejected (funds, terrain, occupied, …).
    Failed {
        error: PlacementError,
        tile: Option<TileCoord>,
    },
}

/// Drain [`PendingWorldCommand`] track kinds into the [`TrackNetwork`].
///
/// Register in [`SimSet::ApplyCommands`] **after** [`crate::apply_commands`].
pub fn apply_track_commands(
    mut pending: MessageReader<PendingWorldCommand>,
    mut network: ResMut<TrackNetwork>,
    mut money: ResMut<Money>,
    mut ledger: ResMut<MoneyLedger>,
    mut history: ResMut<CommandHistory>,
    terrain: Option<Res<TrackTerrain>>,
    mut edits: MessageWriter<TrackEdit>,
) {
    let Some(terrain) = terrain else {
        if history.mode() != HistoryMode::Record {
            history.finish_replay();
        }
        return;
    };

    let replaying = matches!(
        history.mode(),
        HistoryMode::Undoing | HistoryMode::Redoing
    );

    for msg in pending.read() {
        match &msg.command.kind {
            CommandKind::PlaceTrack(p) => {
                match try_place_track(
                    &mut network,
                    &mut money,
                    &mut ledger,
                    &terrain,
                    p.tile,
                    p.layer,
                ) {
                    Ok(placed) => {
                        edits.write(TrackEdit::Placed {
                            id: placed.id,
                            tile: placed.piece.tile,
                            layer: placed.piece.layer,
                            is_bridge: placed.piece.is_bridge(),
                        });
                        let inverse = CommandKind::Demolish(Demolish {
                            track: placed.id,
                        });
                        if replaying {
                            history.push_batch_inverse(inverse);
                        } else {
                            history.record_player_action(vec![inverse]);
                        }
                    }
                    Err(error) => {
                        edits.write(TrackEdit::Failed {
                            error,
                            tile: Some(p.tile),
                        });
                    }
                }
            }
            CommandKind::Demolish(d) => {
                match try_demolish(&mut network, &mut money, &mut ledger, d.track) {
                    Ok(piece) => {
                        edits.write(TrackEdit::Removed {
                            id: piece.id,
                            tile: piece.tile,
                            layer: piece.layer,
                        });
                        let inverse = CommandKind::PlaceTrack(PlaceTrack {
                            tile: piece.tile,
                            layer: piece.layer,
                        });
                        if replaying {
                            history.push_batch_inverse(inverse);
                        } else {
                            history.record_player_action(vec![inverse]);
                        }
                    }
                    Err(error) => {
                        edits.write(TrackEdit::Failed {
                            error,
                            tile: None,
                        });
                    }
                }
            }
            CommandKind::AutoFillPath(p) => {
                match try_place_path(
                    &mut network,
                    &mut money,
                    &mut ledger,
                    &terrain,
                    &p.tiles,
                    p.layer,
                ) {
                    Ok(placed) => {
                        if placed.is_empty() {
                            edits.write(TrackEdit::Failed {
                                error: PlacementError::AlreadyOccupied,
                                tile: p.tiles.last().copied(),
                            });
                        } else {
                            let mut inverse = Vec::with_capacity(placed.len());
                            for pl in &placed {
                                edits.write(TrackEdit::Placed {
                                    id: pl.id,
                                    tile: pl.piece.tile,
                                    layer: pl.piece.layer,
                                    is_bridge: pl.piece.is_bridge(),
                                });
                                inverse.push(CommandKind::Demolish(Demolish { track: pl.id }));
                            }
                            if replaying {
                                for inv in inverse {
                                    history.push_batch_inverse(inv);
                                }
                            } else {
                                history.record_player_action(inverse);
                            }
                        }
                    }
                    Err(error) => {
                        edits.write(TrackEdit::Failed {
                            error,
                            tile: p.tiles.last().copied(),
                        });
                    }
                }
            }
            CommandKind::AutoFillTrack(a) => {
                match try_autofill_track(
                    &mut network,
                    &mut money,
                    &mut ledger,
                    &terrain,
                    a.from,
                    a.to,
                    a.layer,
                ) {
                    Ok(placed) => {
                        if placed.is_empty() {
                            edits.write(TrackEdit::Failed {
                                error: PlacementError::AlreadyOccupied,
                                tile: Some(a.to),
                            });
                        } else {
                            let mut inverse = Vec::with_capacity(placed.len());
                            for p in &placed {
                                edits.write(TrackEdit::Placed {
                                    id: p.id,
                                    tile: p.piece.tile,
                                    layer: p.piece.layer,
                                    is_bridge: p.piece.is_bridge(),
                                });
                                inverse.push(CommandKind::Demolish(Demolish { track: p.id }));
                            }
                            if replaying {
                                for inv in inverse {
                                    history.push_batch_inverse(inv);
                                }
                            } else {
                                history.record_player_action(inverse);
                            }
                        }
                    }
                    Err(error) => {
                        edits.write(TrackEdit::Failed {
                            error,
                            tile: Some(a.to),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    if replaying {
        history.finish_replay();
    }
}
