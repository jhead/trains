//! Apply BuyTrain / PlaceTrain / SellTrain from [`PendingWorldCommand`].
//!
//! # The game says what it did
//!
//! Buying and placing a train are the two moments a playtester reported being
//! blind to — *"I put a train on it, but it never did anything… oh wait, it
//! just started moving"*. Both now speak in Town Talk, the feed the game
//! already uses for everything else it wants to tell the player, in the same
//! whole-sentence shape [`crate::demand::spawn_new_demand`] and the border use.
//! A bought train is an [`TalkKind::Opportunity`] because it is something the
//! player now has to do; a placed one is [`TalkKind::Praise`] because it is the
//! railway starting to work.

use bevy_ecs::prelude::*;

use crate::apply::PendingWorldCommand;
use crate::commands::{CommandKind, TrainKind};
use crate::economy::{requeue_cargo, JobBoard, MoneyCategory, MoneyLedger};
use crate::ids::{StationId, TileCoord, TrainId};
use crate::lines::LineRegistry;
use crate::money::Money;
use crate::peeps::{ComplaintEntry, ComplaintFeed, TalkKind};
use crate::stations::{IndustryRegistry, StationRegistry, StationService};
use crate::track::{step, TrackNetwork, DIR8, GROUND_LAYER};

use super::train::{buy_cost, Train, TrainCargo, TrainLocation, TrainOnLine, TrainYard};

/// Presentation hook when a train is bought, enters the map, or is sold.
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
    Sold {
        id: TrainId,
        kind: TrainKind,
        /// What came back — the full purchase price.
        refund_cents: i64,
    },
}

/// Push one whole sentence into Town Talk.
///
/// A line with no `station_name` carries its own sentence (see
/// [`ComplaintEntry::display_line`]), which is the shape every non-peep entry in
/// the feed uses. `station_id` is deliberately left empty: it marks a line as
/// *the town speaking about that stop* for
/// [`ComplaintFeed::town_spoke_recently`], and a train entering service must not
/// silence the district's own news. The tile is enough for click-to-locate.
fn say(feed: &mut ComplaintFeed, kind: TalkKind, tick: u64, tile: Option<TileCoord>, line: String) {
    feed.push(ComplaintEntry {
        kind,
        peep_name: line,
        station_name: String::new(),
        wait_minutes: 0,
        sim_tick: tick,
        peep_id: None,
        station_id: None,
        tile,
        count: 1,
    });
}

/// Drain train-related pending commands.
#[allow(clippy::too_many_arguments)]
pub fn apply_train_commands(
    mut pending: MessageReader<PendingWorldCommand>,
    mut money: ResMut<Money>,
    mut ledger: ResMut<MoneyLedger>,
    mut yard: ResMut<TrainYard>,
    mut lines: ResMut<LineRegistry>,
    mut talk: ResMut<ComplaintFeed>,
    mut board: ResMut<JobBoard>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    service: Res<StationService>,
    network: Res<TrackNetwork>,
    trains: Query<(Entity, &Train, &TrainCargo)>,
    mut commands: Commands,
    mut edits: MessageWriter<TrainEdit>,
) {
    let tick = service.tick;
    for msg in pending.read() {
        match &msg.command.kind {
            CommandKind::BuyTrain(b) => {
                let cost = buy_cost(b.kind);
                if ledger
                    .try_debit(&mut money, MoneyCategory::RollingStock, cost)
                    .is_err()
                {
                    continue;
                }
                let id = yard.buy(b.kind);
                // The yard is invisible on the map, so this line is the only
                // thing telling the player their money became a train and what
                // to do with it next.
                say(
                    &mut talk,
                    TalkKind::Opportunity,
                    tick,
                    None,
                    format!("Train {} delivered - click a station to place it", id.0),
                );
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
                let mut entity = commands.spawn((
                    Train {
                        id: p.train,
                        kind,
                    },
                    TrainLocation::at_track(track),
                    TrainCargo::Empty,
                ));
                if let Some(line) = lines.line_for_train(p.train) {
                    entity.insert(TrainOnLine {
                        line: line.id,
                        next_stop: 0,
                        forward: true,
                    });
                }
                say(
                    &mut talk,
                    TalkKind::Praise,
                    tick,
                    Some(station.tile),
                    format!("Train {} entering service at {}", p.train.0, station.name),
                );
                edits.write(TrainEdit::Placed {
                    id: p.train,
                    kind,
                    station: p.at_station,
                    tile: station.tile,
                });
            }
            // Rolling stock is reversible like track: the full purchase price
            // comes back, whether the train is running or still in the yard.
            //
            // **Not wired into [`CommandHistory`].** A faithful inverse would
            // have to mint the same [`TrainId`] again, and `BuyTrain` always
            // allocates a fresh one — so undo would silently hand back a
            // different train, and any line assignment or route naming the old
            // id would point at nothing. A full refund is its own undo: the
            // money is back and the player can buy again.
            CommandKind::SellTrain(s) => {
                let placed = trains.iter().find(|(_, t, _)| t.id == s.train);
                let kind = match placed {
                    Some((entity, train, cargo)) => {
                        // The run it was carrying is not the player's to lose:
                        // `assign_jobs` took it off the board, so put it back.
                        requeue_cargo(&mut board, &stations, &industries, cargo);
                        commands.entity(entity).despawn();
                        train.kind
                    }
                    // Unplaced stock sells straight out of the yard.
                    None => match yard.take(s.train) {
                        Some(kind) => kind,
                        None => continue,
                    },
                };
                lines.unassign_train(s.train);
                let refund = buy_cost(kind);
                ledger.credit(&mut money, MoneyCategory::RollingStock, refund);
                say(
                    &mut talk,
                    TalkKind::Opportunity,
                    tick,
                    None,
                    // No figure here: the confirm dialog named the price before
                    // the player agreed to it, and the sim has no money
                    // formatter — the one that draws `$3,000` is presentation.
                    format!("Train {} sold - full price back", s.train.0),
                );
                edits.write(TrainEdit::Sold {
                    id: s.train,
                    kind,
                    refund_cents: refund,
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::{App, FixedUpdate};

    use crate::commands::{BuyTrain, PlaceTrain, SellTrain};
    use crate::command_buffer::CommandBuffer;
    use crate::economy::{passenger_fare_cents, JobKind};
    use crate::stations::GoodKind;
    use crate::track::{try_place_track, TrackTerrain};
    use crate::trains::train::{TRANSIT_COST_CENTS, TRANSPORT_COST_CENTS};
    use crate::{SimClock, SimPlugin};

    /// A paused world with one east-west line and two stops on it.
    ///
    /// Paused so nothing in `Advance` runs: these tests are about the command
    /// pass, and a moving train would keep changing the answer.
    fn world() -> (App, StationId, StationId) {
        let mut app = App::new();
        app.add_plugins(SimPlugin);
        app.world_mut().resource_mut::<SimClock>().paused = true;

        let terrain = TrackTerrain::new(32, 32, (0..32 * 32).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut money = Money::new(10_000_000);
        let mut ledger = MoneyLedger::default();
        for x in 2..=12 {
            try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                TileCoord { x, y: 8 },
                GROUND_LAYER,
            )
            .expect("track");
        }
        let mut stations = StationRegistry::new();
        let east = stations.insert("Eastgate", TileCoord { x: 3, y: 8 }, GROUND_LAYER);
        let west = stations.insert("Westbrook", TileCoord { x: 11, y: 8 }, GROUND_LAYER);

        let w = app.world_mut();
        w.insert_resource(network);
        w.insert_resource(stations);
        w.insert_resource(crate::WorldAnchorsSeeded(true));
        (app, east, west)
    }

    fn push(app: &mut App, kind: CommandKind) {
        app.world_mut().resource_mut::<CommandBuffer>().push(kind);
        app.world_mut().run_schedule(FixedUpdate);
    }

    fn talk_lines(app: &App) -> Vec<String> {
        app.world()
            .resource::<ComplaintFeed>()
            .iter()
            .map(|e| e.display_line())
            .collect()
    }

    fn cash(app: &App) -> i64 {
        app.world().resource::<Money>().cents()
    }

    #[test]
    fn buying_a_train_says_so_and_says_what_to_do_next() {
        // The soft-lock this fixes: the yard is invisible, so a player who has
        // paid for a train has no way of knowing they own one.
        let (mut app, _, _) = world();
        push(
            &mut app,
            CommandKind::BuyTrain(BuyTrain {
                kind: TrainKind::Transit,
            }),
        );

        let lines = talk_lines(&app);
        assert!(
            lines
                .iter()
                .any(|l| l == "Train 1 delivered - click a station to place it"),
            "Town Talk should announce the purchase: {lines:?}"
        );
        assert!(lines.iter().all(|l| l.is_ascii()), "{lines:?}");
    }

    #[test]
    fn placing_a_train_announces_the_moment_it_enters_service() {
        // "It never did anything... oh wait, it just started moving." That
        // moment now has a sentence attached to it.
        let (mut app, east, _) = world();
        push(
            &mut app,
            CommandKind::BuyTrain(BuyTrain {
                kind: TrainKind::Transit,
            }),
        );
        push(
            &mut app,
            CommandKind::PlaceTrain(PlaceTrain {
                train: TrainId(1),
                at_station: east,
            }),
        );

        let lines = talk_lines(&app);
        assert_eq!(
            lines.first().map(String::as_str),
            Some("Train 1 entering service at Eastgate"),
            "the newest line is the train entering service: {lines:?}"
        );
        assert_eq!(
            app.world_mut().query::<&Train>().iter(app.world()).count(),
            1
        );
    }

    #[test]
    fn selling_a_placed_train_refunds_exactly_what_it_cost_and_takes_it_off_the_map() {
        let (mut app, east, _) = world();
        let before = cash(&app);
        push(
            &mut app,
            CommandKind::BuyTrain(BuyTrain {
                kind: TrainKind::Transit,
            }),
        );
        assert_eq!(cash(&app), before - TRANSIT_COST_CENTS);
        push(
            &mut app,
            CommandKind::PlaceTrain(PlaceTrain {
                train: TrainId(1),
                at_station: east,
            }),
        );

        push(
            &mut app,
            CommandKind::SellTrain(SellTrain {
                train: TrainId(1),
            }),
        );

        assert_eq!(
            cash(&app),
            before,
            "a sold train returns its full price, exactly like a demolished tile"
        );
        assert_eq!(
            app.world_mut().query::<&Train>().iter(app.world()).count(),
            0,
            "the train is off the map"
        );
        assert!(app
            .world()
            .resource::<TrainYard>()
            .unplaced()
            .is_empty());
        let lines = talk_lines(&app);
        assert!(
            lines.iter().any(|l| l == "Train 1 sold - full price back"),
            "{lines:?}"
        );
    }

    #[test]
    fn selling_an_unplaced_train_frees_the_yard_slot() {
        // The other half of the soft-lock: stock that only exists in the yard
        // is still stock, and the player must be able to get their money back
        // without first finding somewhere to put it.
        let (mut app, _, _) = world();
        let before = cash(&app);
        push(
            &mut app,
            CommandKind::BuyTrain(BuyTrain {
                kind: TrainKind::Transport,
            }),
        );
        assert_eq!(app.world().resource::<TrainYard>().unplaced().len(), 1);

        push(
            &mut app,
            CommandKind::SellTrain(SellTrain {
                train: TrainId(1),
            }),
        );

        assert_eq!(cash(&app), before);
        assert!(
            app.world()
                .resource::<TrainYard>()
                .unplaced()
                .is_empty(),
            "the yard slot is freed, not left holding a train nobody owns"
        );
        assert_eq!(
            app.world().resource::<TrainYard>().peek_kind(TrainKind::Transport),
            None
        );
    }

    #[test]
    fn a_sold_train_puts_the_run_it_was_carrying_back_on_the_board() {
        // `assign_jobs` takes a job *off* the board when a train picks it up,
        // so the run exists only as cargo. Selling mid-route must not delete
        // demand the town still has.
        let (mut app, east, west) = world();
        let track = {
            let network = app.world().resource::<TrackNetwork>();
            network
                .id_at(TileCoord { x: 6, y: 8 }, GROUND_LAYER)
                .expect("railhead")
        };
        app.world_mut().spawn((
            Train {
                id: TrainId(9),
                kind: TrainKind::Transit,
            },
            TrainLocation::at_track(track),
            TrainCargo::Passengers {
                from: east,
                to: west,
            },
        ));

        push(
            &mut app,
            CommandKind::SellTrain(SellTrain {
                train: TrainId(9),
            }),
        );

        let board = app.world().resource::<JobBoard>();
        assert_eq!(board.jobs.len(), 1, "the run went back on the board");
        assert_eq!(
            board.jobs[0].kind,
            JobKind::Passenger {
                from: east,
                to: west
            }
        );
        assert_eq!(
            board.jobs[0].reward_cents,
            passenger_fare_cents(8),
            "and it is priced by the same distance rule as any other posting \
             — Eastgate to Westbrook is eight tiles"
        );
        assert_eq!(
            app.world_mut().query::<&Train>().iter(app.world()).count(),
            0
        );
    }

    #[test]
    fn an_empty_train_sells_without_inventing_work() {
        let (mut app, _, _) = world();
        let track = {
            let network = app.world().resource::<TrackNetwork>();
            network
                .id_at(TileCoord { x: 6, y: 8 }, GROUND_LAYER)
                .expect("railhead")
        };
        app.world_mut().spawn((
            Train {
                id: TrainId(4),
                kind: TrainKind::Transport,
            },
            TrainLocation::at_track(track),
            TrainCargo::Empty,
        ));
        let before = cash(&app);

        push(
            &mut app,
            CommandKind::SellTrain(SellTrain {
                train: TrainId(4),
            }),
        );

        assert_eq!(cash(&app), before + TRANSPORT_COST_CENTS);
        assert!(app.world().resource::<JobBoard>().jobs.is_empty());
    }

    #[test]
    fn selling_a_train_drops_its_line_assignment() {
        let (mut app, east, west) = world();
        push(
            &mut app,
            CommandKind::BuyTrain(BuyTrain {
                kind: TrainKind::Transit,
            }),
        );
        push(
            &mut app,
            CommandKind::PlaceTrain(PlaceTrain {
                train: TrainId(1),
                at_station: east,
            }),
        );
        let line = {
            let mut lines = app.world_mut().resource_mut::<LineRegistry>();
            let id = lines
                .create("Eastgate - Westbrook".into(), vec![east, west])
                .expect("line");
            assert!(lines.assign_train(id, TrainId(1)));
            id
        };

        push(
            &mut app,
            CommandKind::SellTrain(SellTrain {
                train: TrainId(1),
            }),
        );

        let lines = app.world().resource::<LineRegistry>();
        assert!(
            lines.line_for_train(TrainId(1)).is_none(),
            "a sold train must not still be counted as running a line"
        );
        assert!(lines.get(line).expect("line").trains.is_empty());
    }

    #[test]
    fn selling_a_train_that_does_not_exist_changes_nothing() {
        let (mut app, _, _) = world();
        let before = cash(&app);
        push(
            &mut app,
            CommandKind::SellTrain(SellTrain {
                train: TrainId(77),
            }),
        );
        assert_eq!(cash(&app), before);
        assert!(app.world().resource::<ComplaintFeed>().is_empty());
    }

    #[test]
    fn a_sale_leaves_no_undo_entry_because_the_money_is_the_undo() {
        // `BuyTrain` mints a *new* id, so a recorded inverse would hand back a
        // different train. The refund is the reversal; the history stack stays
        // out of it. Pinned so nobody wires it in without reading why.
        let (mut app, east, _) = world();
        push(
            &mut app,
            CommandKind::BuyTrain(BuyTrain {
                kind: TrainKind::Transit,
            }),
        );
        push(
            &mut app,
            CommandKind::PlaceTrain(PlaceTrain {
                train: TrainId(1),
                at_station: east,
            }),
        );
        let undo_before = app.world().resource::<crate::CommandHistory>().undo_len();

        push(
            &mut app,
            CommandKind::SellTrain(SellTrain {
                train: TrainId(1),
            }),
        );

        assert_eq!(
            app.world().resource::<crate::CommandHistory>().undo_len(),
            undo_before,
            "selling is money-reversible and is its own undo"
        );
    }

    #[test]
    fn a_goods_run_comes_back_as_a_goods_job() {
        let (mut app, _, _) = world();
        let (from, to) = {
            let mut industries = app.world_mut().resource_mut::<IndustryRegistry>();
            let from = industries.insert_tier(
                "Cedar Yard",
                TileCoord { x: 4, y: 7 },
                crate::stations::IndustryTier::Yard,
                Some(GoodKind::Lumber),
                None,
            );
            let to = industries.insert_tier(
                "Mill End",
                TileCoord { x: 10, y: 7 },
                crate::stations::IndustryTier::Works,
                None,
                Some(GoodKind::Lumber),
            );
            (from, to)
        };
        let track = {
            let network = app.world().resource::<TrackNetwork>();
            network
                .id_at(TileCoord { x: 6, y: 8 }, GROUND_LAYER)
                .expect("railhead")
        };
        app.world_mut().spawn((
            Train {
                id: TrainId(3),
                kind: TrainKind::Transport,
            },
            TrainLocation::at_track(track),
            TrainCargo::Goods {
                kind: GoodKind::Lumber,
                from,
                to,
            },
        ));

        push(
            &mut app,
            CommandKind::SellTrain(SellTrain {
                train: TrainId(3),
            }),
        );

        let board = app.world().resource::<JobBoard>();
        assert_eq!(
            board.jobs.first().map(|j| j.kind.clone()),
            Some(JobKind::Goods {
                kind: GoodKind::Lumber,
                from,
                to
            })
        );
    }
}
