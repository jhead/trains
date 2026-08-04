//! What a second car is worth, measured against what a second train is worth.
//!
//! Design 07 §3 makes a claim with a number in it: **a car is the later-game
//! lever, and a second train is the early one.** That is not a statement about
//! a constant — it is a statement about two railways running side by side, one
//! of them one carriage longer and the other one engine larger, on the same
//! demand. So this file runs them and measures.
//!
//! # The two demands
//!
//! * **A thin line** is the opening beat's shape: the station-pair walk posts
//!   one working per ordered pair and nothing stacks, because nobody lives
//!   there yet. A second carriage has nothing to put in it.
//! * **A busy line** is a grown district: peep departures for the same pair
//!   arrive faster than one carriage can lift them, so the board carries a
//!   queue ([`MAX_PENDING_PER_PAIR`](rail_sim::economy::MAX_PENDING_PER_PAIR)).
//!   This file feeds the board directly rather than growing a town for twenty
//!   minutes, because the thing being measured is the *train*, not the town.
//!
//! # Units
//!
//! Per **real minute**, the clock every cost in the game is charged in
//! (`rail_sim::economy::opex`). Capital is quoted as the minutes of extra net
//! income needed to pay it back, which is the only comparison a player makes.

use bevy_app::{App, FixedUpdate};
use rail_sim::economy::opex::TICKS_PER_REAL_MINUTE;
use rail_sim::economy::{Job, JobBoard, JobKind, MAX_PENDING_PER_PAIR};
use rail_sim::ids::{StationId, TileCoord, TrackId, TrainId};
use rail_sim::{
    passenger_fare_cents, DemandSpawner, Money, MoneyCategory, MoneyLedger, SimPlugin,
    StationRegistry, StationService, TrackNetwork, TrackTerrain, Train, TrainCargo, TrainConsist,
    TrainKind, TrainLocation, WorldAnchorsSeeded, GROUND_LAYER, TRANSIT_CAR_COST_CENTS,
    TRANSIT_COST_CENTS,
};

const REAL_MINUTE: u32 = TICKS_PER_REAL_MINUTE as u32;

/// The opening beat's separation (design 02 §4.1): ten tiles between stops.
const HOME: TileCoord = TileCoord { x: 4, y: 8 };
const AWAY: TileCoord = TileCoord { x: 14, y: 8 };

/// How many real minutes each variant is measured over.
///
/// Long enough that a round trip's granularity does not decide the answer: a
/// transit does the twenty-tile round trip in about two real seconds, so four
/// minutes holds well over a hundred of them.
const WINDOW_MINUTES: u32 = 4;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A straight ten-tile line with two stops and `trains` transits on it.
///
/// `cars` is the length of **the first train**; any others are single cars, so
/// "two trains" and "one two-car train" differ by exactly the thing under test.
///
/// The corridor is **double track**, in every variant including the one-train
/// ones. A second train on a single line meets the first nose to nose and both
/// stop — measured, that railway earns nothing at all — so a single-track
/// harness would not be comparing a second train against a second carriage, it
/// would be comparing a deadlock against a carriage. The player who buys a
/// second train has to lay the loop as well, and the honest way to hold that
/// constant is to give every variant the same railway.
fn line(trains: u8, cars: u8) -> App {
    let mut app = App::new();
    app.add_plugins(SimPlugin);
    app.insert_resource(TrackTerrain::new(
        24,
        16,
        (0..24 * 16).map(|_| (false, 2i8)),
    ));
    // Nothing may change under the measurement: no seeded anchors, no new
    // settlements, and a treasury deep enough that nothing soft-fails.
    app.insert_resource(WorldAnchorsSeeded(true));
    app.world_mut()
        .resource_mut::<DemandSpawner>()
        .ticks_until_next = u32::MAX;
    app.insert_resource(Money::new(1_000_000_000));

    let ids = lay(&mut app, HOME, AWAY);
    // The passing road, one tile south, joined at both ends by the diagonals
    // the sixteen-direction graph gives for free.
    lay(
        &mut app,
        TileCoord {
            x: HOME.x,
            y: HOME.y + 1,
        },
        TileCoord {
            x: AWAY.x,
            y: AWAY.y + 1,
        },
    );
    station(&mut app, "Eastgate", HOME);
    station(&mut app, "Westbrook", AWAY);

    for index in 0..trains {
        let at = ids[index as usize];
        let length = if index == 0 { cars } else { 1 };
        app.world_mut().spawn((
            Train {
                id: TrainId(u64::from(index) + 1),
                kind: TrainKind::Transit,
            },
            TrainLocation::at_track(at),
            TrainCargo::Empty,
            TrainConsist::of(length),
        ));
    }
    app
}

fn lay(app: &mut App, from: TileCoord, to: TileCoord) -> Vec<TrackId> {
    let tiles = rail_sim::straight_line(from, to).expect("axis-aligned run");
    let terrain = app.world().resource::<TrackTerrain>().clone();
    let mut ids = Vec::new();
    app.world_mut()
        .resource_scope(|world, mut network: bevy_ecs::prelude::Mut<TrackNetwork>| {
            world.resource_scope(|world, mut money: bevy_ecs::prelude::Mut<Money>| {
                world.resource_scope(|_w, mut ledger: bevy_ecs::prelude::Mut<MoneyLedger>| {
                    for tile in &tiles {
                        if let Ok(placed) = rail_sim::track::try_place_track(
                            &mut network,
                            &mut money,
                            &mut ledger,
                            &terrain,
                            *tile,
                            GROUND_LAYER,
                        ) {
                            ids.push(placed.id);
                        }
                    }
                });
            });
        });
    ids
}

fn station(app: &mut App, name: &str, tile: TileCoord) -> StationId {
    let id = app
        .world_mut()
        .resource_mut::<StationRegistry>()
        .insert(name, tile, GROUND_LAYER);
    app.world_mut().resource_mut::<StationService>().ensure(id);
    id
}

/// Keep both directions of the pair queued `depth` deep, as a busy district
/// would. `0` leaves the sim's own demand alone.
fn top_up(app: &mut App, depth: usize) {
    if depth == 0 {
        return;
    }
    let reward = passenger_fare_cents(10);
    let mut board = app.world_mut().resource_mut::<JobBoard>();
    for (from, to) in [(StationId(1), StationId(2)), (StationId(2), StationId(1))] {
        let kind = JobKind::Passenger { from, to };
        let queued = board.jobs.iter().filter(|j| j.kind == kind).count();
        for _ in queued..depth {
            board.jobs.push(Job {
                kind: kind.clone(),
                reward_cents: reward,
            });
        }
    }
}

/// Fares collected per real minute, and how many journeys they were.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rate {
    gross_per_min: i64,
    runs: u64,
}

fn measure(trains: u8, cars: u8, depth: usize) -> Rate {
    let mut app = line(trains, cars);
    // A minute of settling first: the first boarding of a session is not the
    // steady state, and the empty repositioning leg is paid for by nobody.
    for _ in 0..REAL_MINUTE {
        top_up(&mut app, depth);
        app.world_mut().run_schedule(FixedUpdate);
    }

    let before = banked(&app);
    for _ in 0..(REAL_MINUTE * WINDOW_MINUTES) {
        top_up(&mut app, depth);
        app.world_mut().run_schedule(FixedUpdate);
    }
    let after = banked(&app);

    Rate {
        gross_per_min: (after.0 - before.0) / i64::from(WINDOW_MINUTES),
        runs: (after.1 - before.1) / u64::from(WINDOW_MINUTES),
    }
}

fn banked(app: &App) -> (i64, u64) {
    let ledger = app.world().resource::<MoneyLedger>();
    (ledger.total(MoneyCategory::Fares), ledger.paid_runs())
}

/// Minutes of extra income needed to pay a piece of capital back.
fn payback_minutes(capital_cents: i64, extra_per_min: i64) -> f64 {
    if extra_per_min <= 0 {
        return f64::INFINITY;
    }
    capital_cents as f64 / extra_per_min as f64
}

fn table(label: &str, rows: &[(&str, i64, Rate, i64)]) {
    eprintln!("\n=== {label} ===");
    eprintln!(
        "{:<22} {:>10} {:>7} {:>10} {:>12}",
        "railway", "gross/min", "runs", "capital", "payback"
    );
    for (name, capital, rate, base) in rows {
        let extra = rate.gross_per_min - base;
        let payback = payback_minutes(*capital, extra);
        eprintln!(
            "{:<22} {:>10} {:>7} {:>10} {:>12}",
            name,
            format!("${}", rate.gross_per_min / 100),
            rate.runs,
            format!("${}", capital / 100),
            if payback.is_finite() {
                format!("{payback:.1} min")
            } else {
                "never".into()
            }
        );
    }
}

// ---------------------------------------------------------------------------
// The claims
// ---------------------------------------------------------------------------

/// **A car must not be the opening move.**
///
/// On the line the opening beat actually plays — one working per pair, because
/// there is no town yet — a second carriage carries nothing and costs the train
/// a seventh of its speed. A second *train* serves the other direction and
/// earns. The player who buys the car first is worse off than the one who
/// bought nothing, and that is what makes the car a later-game lever rather
/// than a strictly-better upgrade.
#[test]
fn on_a_thin_line_a_second_train_earns_and_a_second_car_does_not() {
    let one_car = measure(1, 1, 0);
    let two_cars = measure(1, 2, 0);
    let two_trains = measure(2, 1, 0);

    table(
        "a thin line - the opening beat's demand",
        &[
            ("one train, one car", 0, one_car, one_car.gross_per_min),
            (
                "one train, two cars",
                TRANSIT_CAR_COST_CENTS,
                two_cars,
                one_car.gross_per_min,
            ),
            (
                "two trains",
                TRANSIT_COST_CENTS,
                two_trains,
                one_car.gross_per_min,
            ),
        ],
    );

    assert!(
        one_car.gross_per_min > 0,
        "the baseline line has to earn something at all"
    );
    assert!(
        two_cars.gross_per_min <= one_car.gross_per_min,
        "a carriage nobody is waiting for must not pay: {} against {}",
        two_cars.gross_per_min,
        one_car.gross_per_min
    );
    assert!(
        two_trains.gross_per_min > one_car.gross_per_min,
        "a second train serves the other direction and must earn: {} against {}",
        two_trains.gross_per_min,
        one_car.gross_per_min
    );
    assert!(
        two_trains.gross_per_min > two_cars.gross_per_min,
        "and it must beat the car it costs twice as much as"
    );
}

/// **…and on a busy line it must be worth buying.**
///
/// Once a pair's queue is deeper than one carriage, the car lifts what the
/// engine leaves behind. It is not a doubling — a two-car transit is a seventh
/// slower and makes fewer round trips — which is exactly the trade the price is
/// set against.
#[test]
fn on_a_busy_line_every_car_lifts_the_queue_it_can_reach() {
    let one_car = measure(1, 1, MAX_PENDING_PER_PAIR);
    let two_cars = measure(1, 2, MAX_PENDING_PER_PAIR);
    let three_cars = measure(1, 3, MAX_PENDING_PER_PAIR);
    let two_trains = measure(2, 1, MAX_PENDING_PER_PAIR);

    table(
        "a busy line - a queue three deep",
        &[
            ("one train, one car", 0, one_car, one_car.gross_per_min),
            (
                "one train, two cars",
                TRANSIT_CAR_COST_CENTS,
                two_cars,
                one_car.gross_per_min,
            ),
            (
                "one train, three cars",
                TRANSIT_CAR_COST_CENTS * 2,
                three_cars,
                one_car.gross_per_min,
            ),
            (
                "two trains",
                TRANSIT_COST_CENTS,
                two_trains,
                one_car.gross_per_min,
            ),
        ],
    );

    assert!(
        two_cars.gross_per_min > one_car.gross_per_min * 3 / 2,
        "a second carriage on a three-deep queue should be worth at least half \
         again: {} against {}",
        two_cars.gross_per_min,
        one_car.gross_per_min
    );
    assert!(
        three_cars.gross_per_min > two_cars.gross_per_min,
        "and a third carriage more still: {} against {}",
        three_cars.gross_per_min,
        two_cars.gross_per_min
    );
    // Journeys per trip, not trips per minute: the longer train makes **fewer**
    // round trips than the short one, and earns more anyway because each of
    // them clears more of the queue. That is the shape of the whole lever, and
    // it is the reason a car is not simply a cheaper train.
    assert!(
        three_cars.runs > two_cars.runs && two_cars.runs > one_car.runs,
        "a longer train should complete more journeys: {} / {} / {}",
        one_car.runs,
        two_cars.runs,
        three_cars.runs
    );

    // **The point of the price.** Per dollar of capital, the car beats the
    // train on a queue the train would only half-fill — and the tradeoff is a
    // real one, because the train keeps running when the first is held.
    let car_extra = two_cars.gross_per_min - one_car.gross_per_min;
    let train_extra = two_trains.gross_per_min - one_car.gross_per_min;
    let car_payback = payback_minutes(TRANSIT_CAR_COST_CENTS, car_extra);
    let train_payback = payback_minutes(TRANSIT_COST_CENTS, train_extra);
    eprintln!(
        "\nbusy line payback: first car {car_payback:.1} min, second train \
         {train_payback:.1} min"
    );
    assert!(
        car_payback < train_payback,
        "on a queue this deep the car should be the cheaper way to lift it: \
         {car_payback:.1} min against {train_payback:.1} min"
    );
    assert!(
        car_payback < 10.0,
        "a car that takes {car_payback:.1} minutes to pay back is not a lever, \
         it is an ornament"
    );
}

/// The whole design in one measurement: **which lever wins flips with the
/// demand**, and neither dominates.
#[test]
fn which_lever_wins_depends_on_the_queue_and_not_on_the_price() {
    let thin_car = measure(1, 2, 0).gross_per_min - measure(1, 1, 0).gross_per_min;
    let busy_one = measure(1, 1, MAX_PENDING_PER_PAIR).gross_per_min;
    let busy_car = measure(1, 2, MAX_PENDING_PER_PAIR).gross_per_min - busy_one;

    eprintln!(
        "\na car is worth ${}/min on a thin line and ${}/min on a busy one",
        thin_car / 100,
        busy_car / 100
    );
    assert!(
        thin_car <= 0,
        "a car on a thin line is dead weight, not a small win"
    );
    assert!(
        busy_car > 0,
        "a car on a busy line has to be worth its price"
    );
}
