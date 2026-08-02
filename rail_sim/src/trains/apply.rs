//! Apply BuyTrain / PlaceTrain from [`PendingWorldCommand`].

use bevy_ecs::prelude::*;

use crate::apply::PendingWorldCommand;
use crate::commands::{CommandKind, TrainKind};
use crate::ids::{StationId, TileCoord, TrainId};
use crate::money::Money;
use crate::stations::StationRegistry;
use crate::track::{step, TrackNetwork, DIR8, GROUND_LAYER};

use super::train::{buy_cost, Train, TrainCargo, TrainLocation, TrainYard};

/// Presentation hook when a train is bought or enters the map.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum TrainEdit {
    Bought {
        id: TrainId,
        kind: TrainKind,
    },
    Placed {
        id: TrainId,
        kind: TrainKind,
        station: StationId,
        tile: TileCoord,
    },
}

/// Drain train-related pending commands.
pub fn apply_train_commands(
    mut pending: MessageReader<PendingWorldCommand>,
    mut money: ResMut<Money>,
    mut yard: ResMut<TrainYard>,
    stations: Res<StationRegistry>,
    network: Res<TrackNetwork>,
    mut commands: Commands,
    mut edits: MessageWriter<TrainEdit>,
) {
    for msg in pending.read() {
        match &msg.command.kind {
            CommandKind::BuyTrain(b) => {
                let cost = buy_cost(b.kind);
                if money.try_debit(cost).is_err() {
                    continue;
                }
                let id = yard.buy(b.kind);
                edits.write(TrainEdit::Bought { id, kind: b.kind });
            }
            CommandKind::PlaceTrain(p) => {
                let Some(kind) = yard.take(p.train) else {
                    continue;
                };
                let Some(station) = stations.get(p.at_station) else {
                    // Refund into yard if station vanished.
                    yard.return_train(p.train, kind);
                    continue;
                };
                let Some(track) = track_for_station(&network, station.tile, station.layer) else {
                    yard.return_train(p.train, kind);
                    continue;
                };
                commands.spawn((
                    Train {
                        id: p.train,
                        kind,
                    },
                    TrainLocation::at_track(track),
                    TrainCargo::Empty,
                ));
                edits.write(TrainEdit::Placed {
                    id: p.train,
                    kind,
                    station: p.at_station,
                    tile: station.tile,
                });
            }
            _ => {}
        }
    }
}

/// Track under the station tile, or an orthogonally/diagonally adjacent tile.
pub fn track_for_station(
    network: &TrackNetwork,
    tile: TileCoord,
    layer: u8,
) -> Option<crate::ids::TrackId> {
    if let Some(id) = network.id_at(tile, layer) {
        return Some(id);
    }
    for (i, _) in DIR8.iter().enumerate() {
        let n = step(tile, i);
        if let Some(id) = network.id_at(n, layer) {
            return Some(id);
        }
    }
    // Also try ground layer explicitly if caller passed something else.
    if layer != GROUND_LAYER {
        return track_for_station(network, tile, GROUND_LAYER);
    }
    None
}
