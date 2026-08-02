//! Apply station world-commands after [`crate::apply::apply_commands`].
//!
//! # Ordering
//! Register **after** [`crate::apply_commands`] and **before**
//! [`crate::track::apply_track_commands`]. The track handler owns
//! [`CommandHistory::finish_replay`]; running after it would make a station
//! inverse replayed during an undo look like a fresh player action and wipe the
//! redo stack.
//!
//! # Wiring seam
//! [`StationCommand::from_kind`] and [`StationCommand::into_kind`] are the only
//! two places that name the station variants of [`CommandKind`]. They are inert
//! until `commands.rs` grows `PlaceStation` / `DemolishStation` /
//! `UpgradeStation`; filling them in switches the whole path on. Everything
//! else — validation, money, history, presentation — is already live and tested.

use bevy_ecs::prelude::*;

use crate::apply::PendingWorldCommand;
use crate::command_buffer::CommandBuffer;
use crate::commands::CommandKind;
use crate::economy::MoneyLedger;
use crate::history::{CommandHistory, HistoryMode};
use crate::ids::{LineId, StationId, TileCoord};
use crate::lines::LineRegistry;
use crate::money::Money;
use crate::track::TrackNetwork;

use super::place::{
    try_demolish_station, try_place_station, try_upgrade_station, DemolishStation, PlaceStation,
    StationPlacementError, UpgradeStation,
};
use super::registry::StationRegistry;
use super::service::StationService;
use super::tier::StationTier;

/// Fired when a station is built, lifted or retiered so presentation can react.
///
/// [`StationEdit::Failed`] lets UI / audio give the loud rejection the brief asks for.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum StationEdit {
    Placed {
        id: StationId,
        tile: TileCoord,
        layer: u8,
        tier: StationTier,
    },
    Removed {
        id: StationId,
        tile: TileCoord,
        layer: u8,
        tier: StationTier,
    },
    Retiered {
        id: StationId,
        tile: TileCoord,
        from: StationTier,
        to: StationTier,
    },
    /// Place / demolish / upgrade rejected (no track, spacing, funds, …).
    Failed {
        error: StationPlacementError,
        tile: Option<TileCoord>,
    },
}

/// The station slice of [`CommandKind`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StationCommand {
    Place(PlaceStation),
    Demolish(DemolishStation),
    Upgrade(UpgradeStation),
}

impl StationCommand {
    /// Tile the intent is aimed at, for the reason chip / reject flash.
    pub fn tile(&self, stations: &StationRegistry) -> Option<TileCoord> {
        match self {
            Self::Place(p) => Some(p.tile),
            Self::Demolish(d) => stations.get(d.station).map(|s| s.tile),
            Self::Upgrade(u) => stations.get(u.station).map(|s| s.tile),
        }
    }

    /// **WIRING SEAM** — recognise the station variants of [`CommandKind`].
    ///
    /// Replace the body with:
    /// ```ignore
    /// match kind {
    ///     CommandKind::PlaceStation(p) => Some(Self::Place(p.clone())),
    ///     CommandKind::DemolishStation(d) => Some(Self::Demolish(*d)),
    ///     CommandKind::UpgradeStation(u) => Some(Self::Upgrade(*u)),
    ///     _ => None,
    /// }
    /// ```
    pub fn from_kind(kind: &CommandKind) -> Option<Self> {
        match kind {
            CommandKind::PlaceStation(p) => Some(Self::Place(p.clone())),
            CommandKind::DemolishStation(d) => Some(Self::Demolish(*d)),
            CommandKind::UpgradeStation(u) => Some(Self::Upgrade(*u)),
            _ => None,
        }
    }

    /// **WIRING SEAM** — wrap the intent back into a [`CommandKind`].
    ///
    /// Replace the body with:
    /// ```ignore
    /// Some(match self {
    ///     Self::Place(p) => CommandKind::PlaceStation(p),
    ///     Self::Demolish(d) => CommandKind::DemolishStation(d),
    ///     Self::Upgrade(u) => CommandKind::UpgradeStation(u),
    /// })
    /// ```
    pub fn into_kind(self) -> Option<CommandKind> {
        Some(match self {
            Self::Place(p) => CommandKind::PlaceStation(p),
            Self::Demolish(d) => CommandKind::DemolishStation(d),
            Self::Upgrade(u) => CommandKind::UpgradeStation(u),
        })
    }
}

/// Buffer a station intent the same way the track tools buffer a [`PlaceTrack`].
///
/// Presentation calls this instead of building a [`CommandKind`] itself, so the
/// wiring seam stays in one crate.
///
/// [`PlaceTrack`]: crate::commands::PlaceTrack
pub fn push_station_command(buffer: &mut CommandBuffer, command: StationCommand) {
    if let Some(kind) = command.into_kind() {
        buffer.push(kind);
    }
}

/// Drain [`PendingWorldCommand`] station kinds into the [`StationRegistry`].
pub fn apply_station_commands(
    mut pending: MessageReader<PendingWorldCommand>,
    mut stations: ResMut<StationRegistry>,
    mut service: ResMut<StationService>,
    mut money: ResMut<Money>,
    mut ledger: ResMut<MoneyLedger>,
    mut history: ResMut<CommandHistory>,
    network: Res<TrackNetwork>,
    lines: Res<LineRegistry>,
    mut edits: MessageWriter<StationEdit>,
) {
    let commands: Vec<StationCommand> = pending
        .read()
        .filter_map(|msg| StationCommand::from_kind(&msg.command.kind))
        .collect();
    if commands.is_empty() {
        return;
    }

    let mut queued: Vec<StationEdit> = Vec::with_capacity(commands.len());
    for command in commands {
        apply_station_command(
            &mut stations,
            &mut service,
            &mut money,
            &mut ledger,
            &mut history,
            &network,
            |id| line_using(&lines, id),
            &command,
            &mut |edit| queued.push(edit),
        );
    }
    for edit in queued {
        edits.write(edit);
    }
}

/// Apply one station intent, recording its inverse for undo.
///
/// Pure over its inputs so it can be driven from tests without a schedule —
/// the ECS system above is only a drain plus a message pump.
#[allow(clippy::too_many_arguments)]
pub fn apply_station_command(
    stations: &mut StationRegistry,
    service: &mut StationService,
    money: &mut Money,
    ledger: &mut MoneyLedger,
    history: &mut CommandHistory,
    network: &TrackNetwork,
    line_using: impl Fn(StationId) -> Option<LineId>,
    command: &StationCommand,
    edit: &mut impl FnMut(StationEdit),
) {
    let replaying = matches!(
        history.mode(),
        HistoryMode::Undoing | HistoryMode::Redoing
    );
    let tile = command.tile(stations);

    match command {
        StationCommand::Place(p) => match try_place_station(
            stations,
            service,
            money,
            ledger,
            network,
            p.tile,
            p.layer,
            p.tier,
            p.name.clone(),
        ) {
            Ok(placed) => {
                edit(StationEdit::Placed {
                    id: placed.id,
                    tile: placed.station.tile,
                    layer: placed.station.layer,
                    tier: placed.station.tier,
                });
                record(
                    history,
                    replaying,
                    StationCommand::Demolish(DemolishStation { station: placed.id }),
                );
            }
            Err(error) => edit(StationEdit::Failed { error, tile }),
        },
        StationCommand::Demolish(d) => {
            match try_demolish_station(stations, service, money, ledger, d.station, line_using) {
                Ok(removed) => {
                    edit(StationEdit::Removed {
                        id: removed.id,
                        tile: removed.tile,
                        layer: removed.layer,
                        tier: removed.tier,
                    });
                    // Restore the same tier and name so undo is a true inverse.
                    record(
                        history,
                        replaying,
                        StationCommand::Place(PlaceStation {
                            tile: removed.tile,
                            layer: removed.layer,
                            tier: removed.tier,
                            name: Some(removed.name),
                        }),
                    );
                }
                Err(error) => edit(StationEdit::Failed { error, tile }),
            }
        }
        StationCommand::Upgrade(u) => {
            match try_upgrade_station(stations, service, money, ledger, network, u.station, u.to) {
                Ok(retier) => {
                    edit(StationEdit::Retiered {
                        id: retier.id,
                        tile: tile.unwrap_or(TileCoord { x: 0, y: 0 }),
                        from: retier.from,
                        to: retier.to,
                    });
                    record(
                        history,
                        replaying,
                        StationCommand::Upgrade(UpgradeStation {
                            station: retier.id,
                            to: retier.from,
                        }),
                    );
                }
                Err(error) => edit(StationEdit::Failed { error, tile }),
            }
        }
    }
}

/// First line that still calls at `station`, if any.
pub fn line_using(lines: &LineRegistry, station: StationId) -> Option<LineId> {
    lines
        .iter()
        .find(|line| line.contains_station(station))
        .map(|line| line.id)
}

/// Push an inverse onto the replay batch or record it as a player action.
///
/// [`CommandHistory::finish_replay`] stays the track handler's job — see the
/// module docs for why this system must run before it.
fn record(history: &mut CommandHistory, replaying: bool, inverse: StationCommand) {
    let Some(kind) = inverse.into_kind() else {
        // Wiring seam still open — nothing to undo yet.
        return;
    };
    if replaying {
        history.push_batch_inverse(kind);
    } else {
        history.record_player_action(vec![kind]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economy::MoneyCategory;
    use crate::ids::TileCoord;
    use crate::track::{try_place_track, TrackTerrain, GROUND_LAYER};

    struct World {
        stations: StationRegistry,
        service: StationService,
        money: Money,
        ledger: MoneyLedger,
        history: CommandHistory,
        network: TrackNetwork,
    }

    /// Flat land with a straight east-west run of `len` tiles from `(4, 8)`.
    fn world(len: i32) -> World {
        let terrain = TrackTerrain::new(32, 32, (0..32 * 32).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut money = Money::new(10_000_000);
        let mut ledger = MoneyLedger::default();
        for i in 0..len {
            try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                TileCoord { x: 4 + i, y: 8 },
                GROUND_LAYER,
            )
            .expect("track");
        }
        World {
            stations: StationRegistry::new(),
            service: StationService::default(),
            money: Money::new(1_000_000),
            ledger: MoneyLedger::default(),
            history: CommandHistory::new(),
            network,
        }
    }

    fn apply(w: &mut World, command: StationCommand) -> Vec<StationEdit> {
        let mut edits = Vec::new();
        apply_station_command(
            &mut w.stations,
            &mut w.service,
            &mut w.money,
            &mut w.ledger,
            &mut w.history,
            &w.network,
            |_| None,
            &command,
            &mut |edit| edits.push(edit),
        );
        edits
    }

    fn place_at(x: i32, tier: StationTier) -> StationCommand {
        StationCommand::Place(PlaceStation {
            tile: TileCoord { x, y: 8 },
            layer: GROUND_LAYER,
            tier,
            name: None,
        })
    }

    #[test]
    fn place_builds_charges_and_announces() {
        let mut w = world(6);
        let before = w.money.cents();

        let edits = apply(&mut w, place_at(6, StationTier::Station));

        assert_eq!(w.stations.len(), 1);
        let id = w.stations.iter().next().expect("station").id;
        assert_eq!(
            w.money.cents(),
            before - StationTier::Station.build_cents()
        );
        assert_eq!(
            w.ledger.total(MoneyCategory::Construction),
            -StationTier::Station.build_cents(),
            "station spend lands in the Construction bucket"
        );
        assert_eq!(
            edits,
            vec![StationEdit::Placed {
                id,
                tile: TileCoord { x: 6, y: 8 },
                layer: GROUND_LAYER,
                tier: StationTier::Station,
            }]
        );
        assert_eq!(w.service.tier(id), StationTier::Station);
    }

    #[test]
    fn a_rejected_place_reports_its_reason_and_costs_nothing() {
        let mut w = world(6);
        let before = w.money.cents();

        let edits = apply(
            &mut w,
            StationCommand::Place(PlaceStation {
                tile: TileCoord { x: 20, y: 20 },
                layer: GROUND_LAYER,
                tier: StationTier::Halt,
                name: None,
            }),
        );

        assert!(w.stations.is_empty());
        assert_eq!(w.money.cents(), before);
        assert_eq!(
            edits,
            vec![StationEdit::Failed {
                error: StationPlacementError::NoTrack,
                tile: Some(TileCoord { x: 20, y: 20 }),
            }]
        );
    }

    #[test]
    fn demolish_announces_the_lift_and_refunds() {
        let mut w = world(6);
        let before = w.money.cents();
        apply(&mut w, place_at(6, StationTier::Interchange));
        let id = w.stations.iter().next().expect("station").id;

        let edits = apply(
            &mut w,
            StationCommand::Demolish(DemolishStation { station: id }),
        );

        assert!(w.stations.is_empty());
        assert_eq!(w.money.cents(), before);
        assert_eq!(
            edits,
            vec![StationEdit::Removed {
                id,
                tile: TileCoord { x: 6, y: 8 },
                layer: GROUND_LAYER,
                tier: StationTier::Interchange,
            }]
        );
    }

    #[test]
    fn upgrade_announces_both_tiers_and_keeps_one_stop() {
        let mut w = world(8);
        apply(&mut w, place_at(6, StationTier::Halt));
        let id = w.stations.iter().next().expect("station").id;

        let edits = apply(
            &mut w,
            StationCommand::Upgrade(UpgradeStation {
                station: id,
                to: StationTier::Interchange,
            }),
        );

        assert_eq!(w.stations.len(), 1, "upgrading must not create a second stop");
        assert_eq!(
            edits,
            vec![StationEdit::Retiered {
                id,
                tile: TileCoord { x: 6, y: 8 },
                from: StationTier::Halt,
                to: StationTier::Interchange,
            }]
        );
        assert_eq!(w.service.tier(id), StationTier::Interchange);
    }

    #[test]
    fn a_stop_on_a_line_is_refused_with_the_line_named() {
        let mut w = world(6);
        apply(&mut w, place_at(6, StationTier::Halt));
        let id = w.stations.iter().next().expect("station").id;

        let mut edits = Vec::new();
        apply_station_command(
            &mut w.stations,
            &mut w.service,
            &mut w.money,
            &mut w.ledger,
            &mut w.history,
            &w.network,
            |_| Some(LineId(4)),
            &StationCommand::Demolish(DemolishStation { station: id }),
            &mut |edit| edits.push(edit),
        );

        assert_eq!(w.stations.len(), 1);
        assert_eq!(
            edits,
            vec![StationEdit::Failed {
                error: StationPlacementError::OnLine { line: LineId(4) },
                tile: Some(TileCoord { x: 6, y: 8 }),
            }]
        );
    }

    #[test]
    fn line_using_finds_the_first_line_calling_at_a_stop() {
        let mut lines = LineRegistry::new();
        let (a, b) = (StationId(1), StationId(2));
        let id = lines
            .create("Ashford — Brackwell".into(), vec![a, b])
            .expect("line");
        assert_eq!(line_using(&lines, a), Some(id));
        assert_eq!(line_using(&lines, StationId(9)), None);
    }
}
