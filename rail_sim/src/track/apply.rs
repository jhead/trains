//! Apply track world-commands after [`crate::apply::apply_commands`].

use bevy_ecs::prelude::*;

use crate::apply::PendingWorldCommand;
use crate::commands::CommandKind;
use crate::money::Money;

use super::network::TrackNetwork;
use super::place::{try_autofill_track, try_demolish, try_place_track};
use super::terrain::TrackTerrain;

/// Fired when track is added or removed so presentation can bake sprites.
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
}

/// Drain [`PendingWorldCommand`] track kinds into the [`TrackNetwork`].
///
/// Register in [`crate::SimSet::ApplyCommands`] **after** [`crate::apply_commands`].
pub fn apply_track_commands(
    mut pending: MessageReader<PendingWorldCommand>,
    mut network: ResMut<TrackNetwork>,
    mut money: ResMut<Money>,
    terrain: Option<Res<TrackTerrain>>,
    mut edits: MessageWriter<TrackEdit>,
) {
    let Some(terrain) = terrain else {
        return;
    };

    for msg in pending.read() {
        match &msg.command.kind {
            CommandKind::PlaceTrack(p) => {
                match try_place_track(&mut network, &mut money, &terrain, p.tile, p.layer) {
                    Ok(placed) => {
                        edits.write(TrackEdit::Placed {
                            id: placed.id,
                            tile: placed.piece.tile,
                            layer: placed.piece.layer,
                            is_bridge: placed.piece.is_bridge(),
                        });
                    }
                    Err(_) => {
                        // Soft-fail: illegal / broke — ignore (HUD later).
                    }
                }
            }
            CommandKind::Demolish(d) => match try_demolish(&mut network, &mut money, d.track) {
                Ok(piece) => {
                    edits.write(TrackEdit::Removed {
                        id: piece.id,
                        tile: piece.tile,
                        layer: piece.layer,
                    });
                }
                Err(_) => {}
            },
            CommandKind::AutoFillTrack(a) => {
                match try_autofill_track(
                    &mut network,
                    &mut money,
                    &terrain,
                    a.from,
                    a.to,
                    a.layer,
                ) {
                    Ok(placed) => {
                        for p in placed {
                            edits.write(TrackEdit::Placed {
                                id: p.id,
                                tile: p.piece.tile,
                                layer: p.piece.layer,
                                is_bridge: p.piece.is_bridge(),
                            });
                        }
                    }
                    Err(_) => {}
                }
            }
            _ => {}
        }
    }
}
