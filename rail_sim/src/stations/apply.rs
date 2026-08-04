//! Apply station world-commands after [`crate::apply::apply_commands`].
//!
//! # Ordering
//! Register **after** [`crate::apply_commands`] and **before**
//! [`crate::track::apply_track_commands`]. The track handler owns
//! [`CommandHistory::finish_replay`]; running after it would make a station
//! inverse replayed during an undo look like a fresh player action and wipe the
//! redo stack.
//!
//! # Demolishing a stop a line calls at
//! 04 §4 makes demolition a first-class verb that *names its consequence*
//! rather than refusing, so this pass drops the call from every line that has
//! one and records the slots it took, in [`LineId`] order. The inverse is a
//! [`PlaceStation`] carrying those slots: the rebuilt stop gets a fresh
//! [`StationId`], so undo splices *that* id back into the same positions.
//!
//! # Wiring seam
//! [`StationCommand::from_kind`] and [`StationCommand::into_kind`] are the only
//! two places that name the station variants of [`CommandKind`].

use bevy_ecs::prelude::*;

use crate::apply::PendingWorldCommand;
use crate::command_buffer::CommandBuffer;
use crate::commands::CommandKind;
use crate::economy::MoneyLedger;
use crate::history::{CommandHistory, HistoryMode};
use crate::ids::{LineId, StationId, TileCoord};
use crate::lines::{LineRegistry, LineStopSlot};
use crate::money::Money;
use crate::peeps::{ComplaintEntry, ComplaintFeed, TalkKind};
use crate::track::TrackNetwork;

use super::industry::IndustryRegistry;
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
        /// Lines that lost a call, in [`LineId`] order.
        dropped_from: Vec<LineId>,
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
    /// One of the two functions in the crate that name them; everything else
    /// works in [`StationCommand`].
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
    /// The other half of [`Self::from_kind`], used to buffer an intent and to
    /// record an inverse for undo.
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
#[allow(clippy::too_many_arguments)]
pub fn apply_station_commands(
    mut pending: MessageReader<PendingWorldCommand>,
    mut stations: ResMut<StationRegistry>,
    industries: Res<IndustryRegistry>,
    mut service: ResMut<StationService>,
    mut money: ResMut<Money>,
    mut ledger: ResMut<MoneyLedger>,
    mut history: ResMut<CommandHistory>,
    network: Res<TrackNetwork>,
    mut lines: ResMut<LineRegistry>,
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
            &industries,
            &mut service,
            &mut money,
            &mut ledger,
            &mut history,
            &network,
            &mut lines,
            &command,
            &mut |edit| queued.push(edit),
        );
    }
    for edit in queued {
        edits.write(edit);
    }
}

/// Say in Town Talk what the player just did to the railway's stops.
///
/// Mirrors [`crate::lines::apply`]: everything the player does to a line says so
/// in the feed, because an action the world does not acknowledge is an action
/// the player cannot tell landed. A new platform is the strongest case of all —
/// it is the moment 04 §6 exists for, and the sentence names the one thing the
/// player has to do next, which is put a line through it.
///
/// Reads [`StationEdit`] rather than living inside the apply pass so the pure
/// [`apply_station_command`] keeps its signature and its tests.
pub fn announce_station_edits(
    mut edits: MessageReader<StationEdit>,
    service: Res<StationService>,
    stations: Res<StationRegistry>,
    mut talk: ResMut<ComplaintFeed>,
) {
    let tick = service.tick;
    for edit in edits.read() {
        let (tile, sentence) = match edit {
            StationEdit::Placed { id, tile, tier, .. } => {
                let name = station_name(&stations, *id, *tier);
                (
                    Some(*tile),
                    format!("{name} opened - no line calls there yet"),
                )
            }
            StationEdit::Retiered { id, tile, to, .. } => {
                let name = station_name(&stations, *id, *to);
                (
                    Some(*tile),
                    format!(
                        "{name} rebuilt as {} - reach {} tiles",
                        to.label(),
                        to.catchment()
                    ),
                )
            }
            // A stop the player lifted. The tier is the fact worth keeping:
            // "Ashford Halt closed" reads as a place, not as an id.
            StationEdit::Removed { tile, tier, .. } => (
                Some(*tile),
                format!("{} closed - platforms lifted", tier.label()),
            ),
            StationEdit::Failed { .. } => continue,
        };
        talk.push(ComplaintEntry {
            kind: TalkKind::Opportunity,
            // Whole-sentence town line: the sentence goes in `peep_name` and
            // `station_name` stays empty, which is how `display_line` knows not
            // to quote it as somebody speaking.
            peep_name: sentence,
            station_name: String::new(),
            wait_minutes: 0,
            sim_tick: tick,
            peep_id: None,
            station_id: None,
            tile,
            count: 1,
        });
    }
}

/// A stop's name, falling back to its tier for one that has just been lifted.
fn station_name(stations: &StationRegistry, id: StationId, tier: StationTier) -> String {
    stations
        .get(id)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| tier.label().to_string())
}

/// Apply one station intent, recording its inverse for undo.
///
/// Pure over its inputs so it can be driven from tests without a schedule —
/// the ECS system above is only a drain plus a message pump.
#[allow(clippy::too_many_arguments)]
pub fn apply_station_command(
    stations: &mut StationRegistry,
    industries: &IndustryRegistry,
    service: &mut StationService,
    money: &mut Money,
    ledger: &mut MoneyLedger,
    history: &mut CommandHistory,
    network: &TrackNetwork,
    lines: &mut LineRegistry,
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
            industries,
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
                // Undoing a demolish: put the call back where it was. The stop
                // is a new id, so the slots are filled with that.
                for slot in &p.restore_stops {
                    lines.restore_stop(slot.line, slot.index, placed.id);
                }
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
            match try_demolish_station(stations, service, money, ledger, d.station) {
                Ok(removed) => {
                    let (dropped_from, restore_stops) = drop_calls(lines, removed.id);
                    edit(StationEdit::Removed {
                        id: removed.id,
                        tile: removed.tile,
                        layer: removed.layer,
                        tier: removed.tier,
                        dropped_from,
                    });
                    // Restore the same tier, name and calls so undo is a true inverse.
                    record(
                        history,
                        replaying,
                        StationCommand::Place(PlaceStation {
                            tile: removed.tile,
                            layer: removed.layer,
                            tier: removed.tier,
                            name: Some(removed.name),
                            restore_stops,
                        }),
                    );
                }
                Err(error) => edit(StationEdit::Failed { error, tile }),
            }
        }
        StationCommand::Upgrade(u) => {
            match try_upgrade_station(
                stations,
                industries,
                service,
                money,
                ledger,
                network,
                u.station,
                u.to,
            ) {
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
///
/// [`LineId`] order, so the answer is the same on every machine and every run.
pub fn line_using(lines: &LineRegistry, station: StationId) -> Option<LineId> {
    lines.lines_calling_at(station).first().copied()
}

/// Take `station` out of every line that calls there.
///
/// Returns the lines that lost a call and the slots to put it back in, both in
/// [`LineId`] order — the first for the [`StationEdit`], the second for undo.
fn drop_calls(lines: &mut LineRegistry, station: StationId) -> (Vec<LineId>, Vec<LineStopSlot>) {
    let calling = lines.lines_calling_at(station);
    let mut slots: Vec<LineStopSlot> = Vec::new();
    for line in &calling {
        let Some(removed) = lines.remove_stop(*line, station) else {
            continue;
        };
        slots.extend(
            removed
                .indices
                .into_iter()
                .map(|index| LineStopSlot { line: *line, index }),
        );
    }
    (calling, slots)
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
        industries: IndustryRegistry,
        service: StationService,
        money: Money,
        ledger: MoneyLedger,
        history: CommandHistory,
        network: TrackNetwork,
        lines: LineRegistry,
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
            industries: IndustryRegistry::new(),
            service: StationService::default(),
            money: Money::new(1_000_000),
            ledger: MoneyLedger::default(),
            history: CommandHistory::new(),
            network,
            lines: LineRegistry::new(),
        }
    }

    fn apply(w: &mut World, command: StationCommand) -> Vec<StationEdit> {
        let mut edits = Vec::new();
        apply_station_command(
            &mut w.stations,
            &w.industries,
            &mut w.service,
            &mut w.money,
            &mut w.ledger,
            &mut w.history,
            &w.network,
            &mut w.lines,
            &command,
            &mut |edit| edits.push(edit),
        );
        edits
    }

    /// Replay `w`'s newest undo entry, as the command buffer would.
    fn undo(w: &mut World) -> Vec<StationEdit> {
        let inverse = w.history.begin_undo().expect("an undo entry");
        let mut edits = Vec::new();
        for kind in inverse {
            let command = StationCommand::from_kind(&kind).expect("a station inverse");
            edits.extend(apply(w, command));
        }
        w.history.finish_replay();
        edits
    }

    fn place_at(x: i32, tier: StationTier) -> StationCommand {
        StationCommand::Place(PlaceStation::new(
            TileCoord { x, y: 8 },
            GROUND_LAYER,
            tier,
            None,
        ))
    }

    /// A platform the player paid for has to be visible as news, not only as a
    /// sprite: Town Talk is where this game acknowledges what the player did,
    /// and the sentence points at the next move (put a line through it).
    #[test]
    fn a_new_platform_opens_in_town_talk() {
        let mut app = bevy_app::App::new();
        app.init_resource::<StationRegistry>()
            .init_resource::<StationService>()
            .init_resource::<ComplaintFeed>()
            .add_message::<StationEdit>()
            .add_systems(bevy_app::Update, announce_station_edits);

        let id = {
            let mut stations = app.world_mut().resource_mut::<StationRegistry>();
            stations.insert_tier(
                "Ashford Halt",
                TileCoord { x: 6, y: 8 },
                GROUND_LAYER,
                StationTier::Halt,
                StationTier::Halt.build_cents(),
            )
        };

        app.world_mut().write_message(StationEdit::Placed {
            id,
            tile: TileCoord { x: 6, y: 8 },
            layer: GROUND_LAYER,
            tier: StationTier::Halt,
        });
        app.update();

        let line = app
            .world()
            .resource::<ComplaintFeed>()
            .latest_line()
            .expect("the town says something");
        assert_eq!(line, "Ashford Halt opened - no line calls there yet");
        assert!(line.is_ascii(), "{line} would draw as tofu");

        // An upgrade is news too, and it names what the money bought.
        app.world_mut().write_message(StationEdit::Retiered {
            id,
            tile: TileCoord { x: 6, y: 8 },
            from: StationTier::Halt,
            to: StationTier::Interchange,
        });
        app.update();
        let line = app
            .world()
            .resource::<ComplaintFeed>()
            .latest_line()
            .expect("a second line");
        assert_eq!(
            line,
            format!(
                "Ashford Halt rebuilt as Interchange - reach {} tiles",
                StationTier::Interchange.catchment()
            )
        );

        // A refusal is the tool's to voice at the cursor, not the town's.
        let before = app.world().resource::<ComplaintFeed>().len();
        app.world_mut().write_message(StationEdit::Failed {
            error: StationPlacementError::NoTrack,
            tile: Some(TileCoord { x: 1, y: 1 }),
        });
        app.update();
        assert_eq!(app.world().resource::<ComplaintFeed>().len(), before);
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
            StationCommand::Place(PlaceStation::new(
                TileCoord { x: 20, y: 20 },
                GROUND_LAYER,
                StationTier::Halt,
                None,
            )),
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
                dropped_from: vec![],
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

    /// Three stops in a row, all called at by one line.
    fn line_of_three(w: &mut World) -> (LineId, Vec<StationId>) {
        let mut ids = Vec::new();
        for x in [5, 9, 13] {
            apply(w, place_at(x, StationTier::Halt));
            ids.push(
                w.stations
                    .at(TileCoord { x, y: 8 }, GROUND_LAYER)
                    .expect("station")
                    .id,
            );
        }
        let line = w
            .lines
            .create("Riverside Loop".into(), ids.clone())
            .expect("line");
        (line, ids)
    }

    /// 04 §4: the stop goes and the line loses the call — no refusal.
    #[test]
    fn demolishing_a_stop_a_line_calls_at_drops_the_call() {
        let mut w = world(12);
        let (line, ids) = line_of_three(&mut w);

        let edits = apply(
            &mut w,
            StationCommand::Demolish(DemolishStation { station: ids[1] }),
        );

        assert!(w.stations.get(ids[1]).is_none(), "the stop is gone");
        assert_eq!(
            edits,
            vec![StationEdit::Removed {
                id: ids[1],
                tile: TileCoord { x: 9, y: 8 },
                layer: GROUND_LAYER,
                tier: StationTier::Halt,
                dropped_from: vec![line],
            }],
            "the edit names the lines that lost a call"
        );
        assert_eq!(w.lines.get(line).expect("line").stops, vec![ids[0], ids[2]]);
        assert!(!w.lines.get(line).expect("line").is_dormant());
    }

    #[test]
    fn every_line_calling_there_loses_the_stop() {
        let mut w = world(12);
        let (first, ids) = line_of_three(&mut w);
        let second = w
            .lines
            .create("Quarry Run".into(), vec![ids[2], ids[1]])
            .expect("second line");

        let edits = apply(
            &mut w,
            StationCommand::Demolish(DemolishStation { station: ids[1] }),
        );

        match &edits[0] {
            StationEdit::Removed { dropped_from, .. } => {
                assert_eq!(dropped_from, &vec![first, second], "in LineId order");
            }
            other => panic!("expected a lift, got {other:?}"),
        }
        assert_eq!(w.lines.get(first).expect("line").stops, vec![ids[0], ids[2]]);
        assert_eq!(w.lines.get(second).expect("line").stops, vec![ids[2]]);
        assert!(
            w.lines.get(second).expect("line").is_dormant(),
            "a line left with one call is dormant, not deleted"
        );
        assert_eq!(w.lines.len(), 2, "neither line is thrown away");
    }

    /// Undo puts the stop *and* the call back — at the same index.
    #[test]
    fn undoing_a_demolish_restores_the_stop_in_its_old_place_on_the_line() {
        let mut w = world(12);
        let (line, ids) = line_of_three(&mut w);
        let before = w.money.cents();

        apply(
            &mut w,
            StationCommand::Demolish(DemolishStation { station: ids[1] }),
        );
        assert_eq!(w.lines.get(line).expect("line").stops.len(), 2);

        undo(&mut w);

        let stops = &w.lines.get(line).expect("line").stops;
        assert_eq!(stops.len(), 3, "the call came back");
        let restored = w
            .stations
            .at(TileCoord { x: 9, y: 8 }, GROUND_LAYER)
            .expect("the stop was rebuilt");
        assert_eq!(
            stops[1], restored.id,
            "the rebuilt stop takes the slot the old one held"
        );
        assert_eq!(restored.tier, StationTier::Halt);
        assert_eq!(w.money.cents(), before, "the refund is handed back");
        assert!(w.history.can_redo(), "the undo is itself redoable");
    }

    #[test]
    fn undo_restores_a_stop_called_at_twice_and_wakes_the_line() {
        let mut w = world(12);
        let (_, ids) = line_of_three(&mut w);
        // An out-and-back that calls at the middle stop on the way home.
        let hub = w
            .lines
            .create("Hub Shuttle".into(), vec![ids[1], ids[2], ids[1]])
            .expect("line");

        apply(
            &mut w,
            StationCommand::Demolish(DemolishStation { station: ids[1] }),
        );
        assert!(w.lines.get(hub).expect("line").is_dormant());

        undo(&mut w);

        let restored = w
            .stations
            .at(TileCoord { x: 9, y: 8 }, GROUND_LAYER)
            .expect("rebuilt")
            .id;
        assert_eq!(
            w.lines.get(hub).expect("line").stops,
            vec![restored, ids[2], restored],
            "both calls come back in their original positions"
        );
        assert!(!w.lines.get(hub).expect("line").is_dormant());
    }

    /// A goods platform is refused where there is nothing to load, and the
    /// refusal travels the same loud-failure path as every other rule.
    #[test]
    fn a_goods_platform_off_an_industry_fails_loudly() {
        let mut w = world(12);
        let edits = apply(&mut w, place_at(6, StationTier::GoodsPlatform));

        assert!(w.stations.is_empty());
        assert_eq!(
            edits,
            vec![StationEdit::Failed {
                error: StationPlacementError::NoIndustryHere,
                tile: Some(TileCoord { x: 6, y: 8 }),
            }]
        );

        // Put a sawmill beside the line and the same command commits.
        w.industries
            .insert("Pine Sawmill", TileCoord { x: 6, y: 6 }, None, None);
        let edits = apply(&mut w, place_at(6, StationTier::GoodsPlatform));
        assert_eq!(w.stations.len(), 1);
        assert!(matches!(edits[0], StationEdit::Placed { .. }));
    }

    #[test]
    fn line_using_finds_the_first_line_calling_at_a_stop() {
        let mut lines = LineRegistry::new();
        let (a, b) = (StationId(1), StationId(2));
        let id = lines
            .create("Ashford - Brackwell".into(), vec![a, b])
            .expect("line");
        assert_eq!(line_using(&lines, a), Some(id));
        assert_eq!(line_using(&lines, StationId(9)), None);
    }
}
