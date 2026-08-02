//! Evaluating goals on the fixed tick, and saying so in Town Talk.
//!
//! Two systems, both cheap and both no-ops in sandbox:
//!
//! - [`generate_goals_once`] waits for the world's anchors and derives the set.
//! - [`evaluate_goals`] recomputes every open goal's progress each Advance tick,
//!   completes what is met and lapses what is past its deadline.
//!
//! # Where progress comes from
//!
//! Every number is read off state the sandbox already keeps — there is no goal
//! bookkeeping anywhere else in the crate, and no system writes anything for
//! goals' benefit:
//!
//! | Goal | Read from |
//! | --- | --- |
//! | Connect | [`TrackNetwork`] + [`find_path`] between the two stops |
//! | Population | [`HouseholdRegistry::population`] |
//! | Deliveries | [`MoneyLedger`] fare and delivery totals ÷ their unit price |
//! | Serve | [`StationService`] score, banked one tick at a time |
//! | Grow | [`TownDensity`] summed over the stop's catchment |
//!
//! # Announcements
//!
//! Goal events go into the existing Town Talk feed (design 08 §8 — a lens on
//! the sandbox does not get its own notification system). A completion is an
//! [`TalkKind::Opportunity`] line, a lapse and a closing deadline are
//! [`TalkKind::Warning`]. Both carry a whole sentence with an empty station
//! name, which is the feed's existing shape for a line that speaks for itself.

use bevy_ecs::prelude::*;

use crate::economy::{MoneyCategory, MoneyLedger, GOODS_DELIVERY_CENTS, PASSENGER_FARE_CENTS};
use crate::ids::{StationId, TileCoord};
use crate::peeps::{ComplaintEntry, ComplaintFeed, HouseholdRegistry, TalkKind};
use crate::stations::{IndustryRegistry, StationRegistry, StationService};
use crate::town::TownDensity;
use crate::track::TrackNetwork;
use crate::trains::{find_path, track_for_station};

use super::board::GoalBoard;
use super::generate::generate_goal_set;
use super::goal::{Goal, GoalKind, GoalStatus};

/// Derive this world's goal set once its anchors exist.
///
/// Runs in `Update` rather than the fixed tick because it is waiting on
/// `seed_world_anchors_once`, which lives there too. Ordering between them is
/// deliberately not declared: this retries every frame until stations appear,
/// which also covers a world restored from a save mid-generation.
pub fn generate_goals_once(
    mut board: ResMut<GoalBoard>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    mut talk: ResMut<ComplaintFeed>,
    service: Res<StationService>,
) {
    if !board.needs_generation() || stations.is_empty() {
        return;
    }
    let goals = generate_goal_set(board.seed, &stations, &industries);
    let count = goals.len();
    let first = goals.first().map(|g| g.title.clone());
    board.install(goals);

    if count == 0 {
        return;
    }
    // The set introduces itself the way everything else in this world does.
    announce(
        &mut talk,
        TalkKind::Opportunity,
        format!("{count} goals for this map - deadlines only, nothing ends the game"),
        None,
        None,
        service.tick,
    );
    if let Some(title) = first {
        announce(
            &mut talk,
            TalkKind::Opportunity,
            format!("First up: {title}"),
            None,
            None,
            service.tick,
        );
    }
}

/// Recompute every open goal, then resolve what is met or out of time.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_goals(
    mut board: ResMut<GoalBoard>,
    stations: Res<StationRegistry>,
    service: Res<StationService>,
    density: Res<TownDensity>,
    ledger: Res<MoneyLedger>,
    network: Res<TrackNetwork>,
    households: Res<HouseholdRegistry>,
    mut talk: ResMut<ComplaintFeed>,
) {
    if !board.is_active() || board.is_empty() {
        return;
    }
    let now = service.tick;
    let population = households.population() as u64;
    let runs = paid_runs(&ledger);

    // Collected rather than pushed inline: the feed and the board are both
    // borrowed from the same resource set, and a goal's announcement wants the
    // resolved goal, not a half-updated one.
    let mut announcements: Vec<(TalkKind, String, Option<StationId>, Option<TileCoord>)> =
        Vec::new();

    for goal in board.iter_mut() {
        if !goal.is_active() {
            continue;
        }
        let current = measure(
            goal,
            &stations,
            &service,
            &density,
            &network,
            population,
            runs,
        );
        goal.current = current;

        if goal.current >= goal.target {
            goal.status = GoalStatus::Complete;
            goal.resolved_tick = now;
            announcements.push((
                TalkKind::Opportunity,
                format!("Goal met - {}", goal.title),
                goal.kind.station(),
                tile_of(&stations, goal.kind.station()),
            ));
            continue;
        }

        if now >= goal.deadline_tick {
            goal.status = GoalStatus::Failed;
            goal.resolved_tick = now;
            // Deliberately not a defeat: the railway that was built stays built.
            announcements.push((
                TalkKind::Warning,
                format!("Deadline passed - {}. The railway stays.", goal.title),
                goal.kind.station(),
                tile_of(&stations, goal.kind.station()),
            ));
            continue;
        }

        if !goal.warned && goal.deadline_is_close(now) {
            goal.warned = true;
            announcements.push((
                TalkKind::Warning,
                format!("A day left - {}", goal.title),
                goal.kind.station(),
                tile_of(&stations, goal.kind.station()),
            ));
        }
    }

    for (kind, message, station, tile) in announcements {
        announce(&mut talk, kind, message, station, tile, now);
    }
}

/// Progress for one goal, read from live sim state.
fn measure(
    goal: &Goal,
    stations: &StationRegistry,
    service: &StationService,
    density: &TownDensity,
    network: &TrackNetwork,
    population: u64,
    runs: u64,
) -> u64 {
    match goal.kind {
        GoalKind::Connect { from, to } => u64::from(is_linked(stations, network, from, to)),
        GoalKind::Population => population,
        GoalKind::Deliveries => runs,
        GoalKind::Serve { station, min_score } => {
            // Banked, not consecutive: a bad afternoon slows the goal rather
            // than wiping the week (design 08 §1 — nothing here punishes).
            if service.score(station).score >= min_score {
                goal.current.saturating_add(1)
            } else {
                goal.current
            }
        }
        GoalKind::Grow { station } => built_tenths(stations, density, station),
    }
}

/// `true` when a train could run from one stop to the other today.
fn is_linked(
    stations: &StationRegistry,
    network: &TrackNetwork,
    from: StationId,
    to: StationId,
) -> bool {
    let Some(a) = stations.get(from) else {
        return false;
    };
    let Some(b) = stations.get(to) else {
        return false;
    };
    let (Some(a_track), Some(b_track)) = (
        track_for_station(network, a.tile, a.layer),
        track_for_station(network, b.tile, b.layer),
    ) else {
        return false;
    };
    a_track == b_track || find_path(network, a_track, b_track).is_some()
}

/// Built density inside a stop's catchment, in tenths.
///
/// Walked as a fixed `dy`/`dx` sweep rather than by iterating [`TownDensity`],
/// whose backing map has no stable order — summing floats in a `HashMap`'s
/// order would make the number differ between runs of the same world.
fn built_tenths(stations: &StationRegistry, density: &TownDensity, station: StationId) -> u64 {
    let Some(stop) = stations.get(station) else {
        return 0;
    };
    let radius = stop.tier.catchment();
    let mut total = 0.0_f32;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            total += density.get(TileCoord {
                x: stop.tile.x + dx,
                y: stop.tile.y + dy,
            });
        }
    }
    (total.max(0.0) * 10.0) as u64
}

/// Paid runs completed this session — fares plus goods deliveries.
///
/// Read back out of the ledger rather than counted separately, because the
/// ledger is already the authority on what the railway has earned (and is
/// already in the save).
fn paid_runs(ledger: &MoneyLedger) -> u64 {
    let fares = ledger.total(MoneyCategory::Fares).max(0) / PASSENGER_FARE_CENTS.max(1);
    let goods = ledger.total(MoneyCategory::Deliveries).max(0) / GOODS_DELIVERY_CENTS.max(1);
    (fares + goods) as u64
}

fn tile_of(stations: &StationRegistry, station: Option<StationId>) -> Option<TileCoord> {
    stations.get(station?).map(|s| s.tile)
}

/// Push one whole-sentence line into Town Talk.
fn announce(
    talk: &mut ComplaintFeed,
    kind: TalkKind,
    message: String,
    station_id: Option<StationId>,
    tile: Option<TileCoord>,
    tick: u64,
) {
    talk.push(ComplaintEntry {
        kind,
        // The feed renders `peep_name` alone when the station name is empty —
        // the same shape new-demand announcements already use.
        peep_name: message,
        station_name: String::new(),
        wait_minutes: 0,
        sim_tick: tick,
        peep_id: None,
        station_id,
        tile,
        count: 1,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::{GoalId, GoalMode};
    use crate::money::Money;
    use crate::peeps::TICKS_PER_DAY;
    use crate::track::{try_place_track, TrackTerrain, GROUND_LAYER};
    use bevy_app::{App, Update};

    /// Run the `Update` schedule directly, as the other sim tests do — a bare
    /// `App` has no `Time`, so the full `Main` schedule is more than is needed.
    fn step(app: &mut App) {
        app.world_mut().run_schedule(Update);
    }

    fn app_with(goals: Vec<Goal>, stations: StationRegistry) -> App {
        let mut app = App::new();
        let mut board = GoalBoard::default();
        board.start(GoalMode::Goals, 42);
        board.install(goals);
        app.insert_resource(board)
            .insert_resource(stations)
            .init_resource::<StationService>()
            .init_resource::<TownDensity>()
            .init_resource::<MoneyLedger>()
            .init_resource::<TrackNetwork>()
            .init_resource::<HouseholdRegistry>()
            .init_resource::<ComplaintFeed>()
            .add_systems(Update, evaluate_goals);
        app
    }

    fn goal(kind: GoalKind, target: u64, deadline: u64) -> Goal {
        Goal::new(GoalId(0), kind, "test goal", target, deadline)
    }

    fn board_of(app: &App) -> &GoalBoard {
        app.world().resource::<GoalBoard>()
    }

    #[test]
    fn a_sandbox_world_never_evaluates_anything() {
        let mut app = app_with(
            vec![goal(GoalKind::Deliveries, 1, TICKS_PER_DAY)],
            StationRegistry::new(),
        );
        app.world_mut().resource_mut::<GoalBoard>().mode = GoalMode::Sandbox;
        app.world_mut()
            .resource_mut::<MoneyLedger>()
            .record(MoneyCategory::Fares, PASSENGER_FARE_CENTS * 4);
        step(&mut app);
        assert!(
            board_of(&app).iter().all(|g| g.is_active() && g.current == 0),
            "goals mode is off; nothing moves"
        );
    }

    #[test]
    fn deliveries_progress_comes_out_of_the_ledger() {
        let mut app = app_with(
            vec![goal(GoalKind::Deliveries, 3, TICKS_PER_DAY)],
            StationRegistry::new(),
        );
        {
            let mut ledger = app.world_mut().resource_mut::<MoneyLedger>();
            ledger.record(MoneyCategory::Fares, PASSENGER_FARE_CENTS * 2);
            ledger.record(MoneyCategory::Deliveries, GOODS_DELIVERY_CENTS);
        }
        step(&mut app);
        let goal = board_of(&app).iter().next().unwrap();
        assert_eq!(goal.current, 3, "two fares and one delivery are three runs");
        assert!(goal.is_complete());
        assert!(
            app.world()
                .resource::<ComplaintFeed>()
                .iter()
                .any(|e| e.display_line().starts_with("Goal met")),
            "a met goal says so in Town Talk"
        );
    }

    #[test]
    fn a_passed_deadline_lapses_the_goal_and_never_ends_the_game() {
        let mut app = app_with(
            vec![goal(GoalKind::Deliveries, 100, 1)],
            StationRegistry::new(),
        );
        app.world_mut().resource_mut::<StationService>().tick = 5;
        step(&mut app);

        let goal = board_of(&app).iter().next().unwrap();
        assert!(goal.is_failed());
        assert_eq!(goal.resolved_tick, 5);
        let line = app
            .world()
            .resource::<ComplaintFeed>()
            .latest_line()
            .unwrap();
        assert!(line.contains("Deadline passed"));
        assert!(line.contains("railway stays"), "failure is not a defeat");
    }

    #[test]
    fn a_closing_deadline_is_announced_once_and_only_once() {
        let mut app = app_with(
            vec![goal(GoalKind::Deliveries, 100, TICKS_PER_DAY)],
            StationRegistry::new(),
        );
        app.world_mut().resource_mut::<StationService>().tick = 1;
        for _ in 0..5 {
            step(&mut app);
        }
        let warnings = app
            .world()
            .resource::<ComplaintFeed>()
            .iter()
            .filter(|e| e.display_line().starts_with("A day left"))
            .count();
        assert_eq!(warnings, 1);
    }

    #[test]
    fn service_time_banks_only_while_the_stop_is_actually_served() {
        let mut stations = StationRegistry::new();
        let id = stations.insert("Eastgate", TileCoord { x: 4, y: 4 }, GROUND_LAYER);
        let mut app = app_with(
            vec![goal(
                GoalKind::Serve {
                    station: id,
                    min_score: 50,
                },
                3,
                TICKS_PER_DAY,
            )],
            stations,
        );

        step(&mut app);
        assert_eq!(board_of(&app).iter().next().unwrap().current, 0);

        app.world_mut().resource_mut::<StationService>().ensure(id).score = 60;
        step(&mut app);
        step(&mut app);
        assert_eq!(board_of(&app).iter().next().unwrap().current, 2);

        // A bad stretch stalls the goal; it never rewinds it.
        app.world_mut().resource_mut::<StationService>().ensure(id).score = 10;
        step(&mut app);
        assert_eq!(board_of(&app).iter().next().unwrap().current, 2);
    }

    #[test]
    fn a_connect_goal_completes_when_rail_actually_joins_the_two_stops() {
        let mut stations = StationRegistry::new();
        let a = stations.insert("A", TileCoord { x: 1, y: 2 }, GROUND_LAYER);
        let b = stations.insert("B", TileCoord { x: 4, y: 2 }, GROUND_LAYER);
        let mut app = app_with(
            vec![goal(GoalKind::Connect { from: a, to: b }, 1, TICKS_PER_DAY)],
            stations,
        );

        step(&mut app);
        assert!(
            board_of(&app).iter().next().unwrap().is_active(),
            "no track, no link"
        );

        let terrain = TrackTerrain::new(8, 8, (0..64).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut money = Money::new(500_000);
        let mut ledger = MoneyLedger::default();
        for x in 1..=4 {
            try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                TileCoord { x, y: 2 },
                GROUND_LAYER,
            )
            .unwrap();
        }
        app.world_mut().insert_resource(network);
        step(&mut app);
        assert!(board_of(&app).iter().next().unwrap().is_complete());
    }

    #[test]
    fn density_progress_is_summed_in_a_fixed_order() {
        let mut stations = StationRegistry::new();
        let id = stations.insert("Eastgate", TileCoord { x: 10, y: 10 }, GROUND_LAYER);
        let mut app = app_with(
            vec![goal(GoalKind::Grow { station: id }, 20, TICKS_PER_DAY)],
            stations,
        );
        {
            let mut density = app.world_mut().resource_mut::<TownDensity>();
            for dx in -2..=2 {
                density.set(TileCoord { x: 10 + dx, y: 10 }, 0.5);
            }
        }
        step(&mut app);
        let first = board_of(&app).iter().next().unwrap().current;
        assert_eq!(first, 25, "five half-built tiles are 2.5 built");

        // Same world, same number, every time.
        for _ in 0..8 {
            app.world_mut().resource_mut::<GoalBoard>().iter_mut().for_each(|g| {
                g.status = GoalStatus::Active;
                g.current = 0;
            });
            step(&mut app);
            assert_eq!(board_of(&app).iter().next().unwrap().current, first);
        }
    }

    #[test]
    fn a_goal_about_a_demolished_stop_stalls_instead_of_panicking() {
        let missing = StationId(99);
        let mut app = app_with(
            vec![
                goal(
                    GoalKind::Serve {
                        station: missing,
                        min_score: 10,
                    },
                    5,
                    TICKS_PER_DAY,
                ),
                goal(GoalKind::Grow { station: missing }, 5, TICKS_PER_DAY),
            ],
            StationRegistry::new(),
        );
        step(&mut app);
        assert!(board_of(&app).iter().all(|g| g.is_active()));
    }
}
