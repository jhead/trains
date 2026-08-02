//! Player intent as serializable command structs.
//!
//! Every action the player takes becomes a command buffered and applied on
//! the fixed-tick boundary. Later the same types can be networked or replayed.

use serde::{Deserialize, Serialize};

use crate::border::{AssignTrainToBorder, CloseBorder, OpenBorder, SetBorderTrade};
use crate::ids::{LineId, StationId, TileCoord, TrackId, TrainId};
use crate::stations::{DemolishStation, PlaceStation, UpgradeStation};

/// Stable command envelope for buffering / replay / future networking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimCommand {
    /// Monotonic id assigned when the command is accepted (0 until assigned).
    pub sequence: u64,
    pub kind: CommandKind,
}

impl CommandKind {
    pub fn pause(paused: bool) -> Self {
        Self::Pause(Pause { paused })
    }

    pub fn set_speed(multiplier: u8) -> Self {
        Self::SetSpeed(SetSpeed { multiplier })
    }

    pub fn toggle_pause_from(currently_paused: bool) -> Self {
        Self::pause(!currently_paused)
    }
}

/// Discriminated player / system intents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandKind {
    PlaceTrack(PlaceTrack),
    Demolish(Demolish),
    AutoFillTrack(AutoFillTrack),
    BuyTrain(BuyTrain),
    PlaceTrain(PlaceTrain),
    CreateLine(CreateLine),
    AssignTrainToLine(AssignTrainToLine),
    UnassignTrain(UnassignTrain),
    PlaceStation(PlaceStation),
    DemolishStation(DemolishStation),
    UpgradeStation(UpgradeStation),
    OpenBorder(OpenBorder),
    CloseBorder(CloseBorder),
    SetBorderTrade(SetBorderTrade),
    AssignTrainToBorder(AssignTrainToBorder),
    SetSpeed(SetSpeed),
    Pause(Pause),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceTrack {
    pub tile: TileCoord,
    /// Reserved for tunnels / elevated; ground-only in MVP.
    pub layer: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Demolish {
    pub track: TrackId,
}

/// Straight auto-fill between two anchors (inclusive).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoFillTrack {
    pub from: TileCoord,
    pub to: TileCoord,
    pub layer: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuyTrain {
    pub kind: TrainKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceTrain {
    pub train: TrainId,
    pub at_station: StationId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrainKind {
    Transit,
    Transport,
}

/// Confirm a player-drawn line (ordered station stops).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateLine {
    /// Optional override; empty / missing → endpoint suggestion.
    pub name: Option<String>,
    pub stops: Vec<StationId>,
}

/// Assign a placed train to a line (prefers line work over free-roam).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignTrainToLine {
    pub train: TrainId,
    pub line: LineId,
}

/// Clear a train's line assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnassignTrain {
    pub train: TrainId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetSpeed {
    /// 0 = paused, 1 = 1x, 3 = 3x, etc.
    pub multiplier: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pause {
    pub paused: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_track_roundtrips_shape() {
        let cmd = SimCommand {
            sequence: 1,
            kind: CommandKind::PlaceTrack(PlaceTrack {
                tile: TileCoord { x: 2, y: -1 },
                layer: 0,
            }),
        };
        match cmd.kind {
            CommandKind::PlaceTrack(p) => {
                assert_eq!(p.tile.x, 2);
                assert_eq!(p.layer, 0);
            }
            _ => panic!("expected PlaceTrack"),
        }
    }
}
