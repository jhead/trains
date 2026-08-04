//! The first three minutes, measured — the opening beat as it is actually played.
//!
//! `economy_arc.rs` measures a *steady state*: it hands the world two stations
//! and nothing else, silences the demand spawner, and asks what a running
//! network earns per minute. Every number in it was green while the owner's
//! first session read **$28.80 of fares against $410/min of running costs**, so
//! whatever it measures is not the opening. What a player meets on minute one is
//! a different world:
//!
//! * **three** seeded anchors, not two, only two of which their first line
//!   reaches — and, before this file existed, all three billing maintenance;
//! * an L-shaped run round terrain, not a straight ruler line;
//! * a job board fed by `spawn_demand_jobs`' station-pair walk and by peep
//!   routines, neither of which the arc test lets run;
//! * `spawn_new_demand` planting a fresh unconnected settlement every few
//!   minutes for the whole session.
//!
//! This file scripts exactly that and measures it minute by minute.
//!
//! # Which minute
//!
//! **Real** minutes, the clock the player is sitting at — 3,840 ticks each. The
//! sim's own minute is six ticks (0.094 s), so "paying out by the third minute"
//! read as sim-minutes would be eighteen ticks, which is less time than one
//! transit needs to cross six tiles. Design 02 §4.1 is a claim about the
//! player's first three minutes, and the ledger the claim failed against reports
//! `$/min` in those same minutes. See `rail_sim::economy::opex` for the table.

use bevy_app::{App, FixedUpdate};
use rail_sim::economy::opex::TICKS_PER_REAL_MINUTE;
use rail_sim::ids::{StationId, TileCoord};
use rail_sim::{
    passenger_fare_cents, station_maintenance_billed, track_maintenance_total, AutoFillTrack,
    BuyTrain, CommandBuffer, CommandKind, IndustryRegistry, JobBoard, JobKind, Money, MoneyCategory,
    MoneyLedger, PlaceStation, PlaceTrain, SimPlugin, StationRegistry, StationService, StationTier,
    TrackNetwork, TrackTerrain, TrainKind, TrainYard, WorldAnchorsSeeded, GROUND_LAYER,
    STARTING_CASH_CENTS, TRANSIT_PROFILE,
};

/// Ticks in one real minute, as a loop count.
const REAL_MINUTE: u32 = TICKS_PER_REAL_MINUTE as u32;

// ---------------------------------------------------------------------------
// The opening beat, built the way it is played
// ---------------------------------------------------------------------------

/// Home town, near the middle of the map — the generator's first anchor hint.
const HOME: TileCoord = TileCoord { x: 20, y: 20 };
/// The destination, ten tiles away on the Chebyshev measure a fare is paid on.
///
/// Design 02 §4.1 puts the opening pair "eight to twelve tiles" apart, and the
/// owner's session was ten: ten east, ten south, an L of twenty tiles.
const AWAY: TileCoord = TileCoord { x: 30, y: 30 };
/// The corner of the L. Going round it rather than straight is the point — no
/// real terrain lets you draw the ten-tile diagonal.
const CORNER: TileCoord = TileCoord { x: 30, y: 20 };

/// Flat, dry, buildable ground.
///
/// Terrain is not the variable here. The owner's line cost more per tile than
/// this one does — $165 against $100, slope and water premiums — but that
/// changes what a line costs to *build*, and this file is about what it earns.
/// `rail_town/tests/cold_start_pays_out.rs` runs the same measurement on real
/// generated terrain across the standard seeds.
fn flat_terrain(w: u32, h: u32) -> TrackTerrain {
    TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 2i8)))
}

/// The same ground with a one-tile stream through `wet`.
fn terrain_with_stream(w: u32, h: u32, wet: TileCoord) -> TrackTerrain {
    TrackTerrain::new(
        w,
        h,
        (0..w * h).map(|i| {
            let (x, y) = ((i % w) as i32, (i / w) as i32);
            ((x, y) == (wet.x, wet.y), 2i8)
        }),
    )
}

/// A world at tick zero: real terrain, real seeded anchors, real starting cash.
///
/// Anchors come from [`rail_sim::seed_stations_and_industries_at`] with the
/// opening pair as the generator's hints, which is precisely the call
/// `SimPlugin`'s `seed_world_anchors_once` makes against a generated map. That
/// seeding plants **three** stations and two industries; the player's first line
/// reaches two of them.
fn cold_start_world() -> App {
    let mut app = App::new();
    app.add_plugins(SimPlugin);
    let terrain = flat_terrain(64, 64);

    {
        let world = app.world_mut();
        let mut stations = std::mem::take(&mut *world.resource_mut::<StationRegistry>());
        let mut industries = std::mem::take(&mut *world.resource_mut::<IndustryRegistry>());
        let mut service = std::mem::take(&mut *world.resource_mut::<StationService>());
        rail_sim::seed_stations_and_industries_at(
            &mut stations,
            &mut industries,
            &mut service,
            terrain.width(),
            terrain.height(),
            |c| terrain.contains(c) && !terrain.is_water(c),
            |c| terrain.contains(c) && terrain.is_water(c),
            &[HOME, AWAY],
        );
        *world.resource_mut::<StationRegistry>() = stations;
        *world.resource_mut::<IndustryRegistry>() = industries;
        *world.resource_mut::<StationService>() = service;
    }
    app.insert_resource(WorldAnchorsSeeded(true));
    app.insert_resource(terrain);
    app.insert_resource(Money::new(STARTING_CASH_CENTS));
    app
}

fn push(app: &mut App, kind: CommandKind) {
    app.world_mut().resource_mut::<CommandBuffer>().push(kind);
}

fn run(app: &mut App, ticks: u32) {
    for _ in 0..ticks {
        app.world_mut().run_schedule(FixedUpdate);
    }
}

/// Lay the L and put one transit on it, through the command buffer the UI uses.
///
/// Three ticks all told, which is the point: design 02 §4.1 gives the player two
/// minutes to get this far and the measurement below starts from tick zero.
fn build_the_opening_line(app: &mut App) {
    for (from, to) in [(HOME, CORNER), (CORNER, AWAY)] {
        push(
            app,
            CommandKind::AutoFillTrack(AutoFillTrack {
                from,
                to,
                layer: GROUND_LAYER,
            }),
        );
    }
    push(
        app,
        CommandKind::BuyTrain(BuyTrain {
            kind: TrainKind::Transit,
        }),
    );
    run(app, 2);

    let bought = app
        .world()
        .resource::<TrainYard>()
        .unplaced()
        .first()
        .map(|(id, _)| *id)
        .expect("a transit in the yard");
    let home = station_at(app, HOME);
    push(
        app,
        CommandKind::PlaceTrain(PlaceTrain {
            train: bought,
            at_station: home,
        }),
    );
    run(app, 1);
}

fn station_at(app: &App, tile: TileCoord) -> StationId {
    app.world()
        .resource::<StationRegistry>()
        .at(tile, GROUND_LAYER)
        .map(|s| s.id)
        .unwrap_or_else(|| panic!("no station seeded at {tile:?}"))
}

// ---------------------------------------------------------------------------
// Measurement
// ---------------------------------------------------------------------------

/// One real minute of the opening, as the status strip would have shown it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Minute {
    minute: u32,
    /// Fares + deliveries collected this minute.
    income: i64,
    /// Train opex + track/station maintenance charged this minute.
    upkeep: i64,
    /// Deliveries completed this minute.
    runs: u64,
    /// Balance at the end of the minute.
    cash: i64,
    /// Session net so far, capital included.
    net_including_capex: i64,
}

impl Minute {
    fn operating_net(self) -> i64 {
        self.income - self.upkeep
    }
}

/// `(income, upkeep, paid runs)` session totals; the first two positive cents.
fn totals(app: &App) -> (i64, i64, u64) {
    let ledger = app.world().resource::<MoneyLedger>();
    let income = ledger.total(MoneyCategory::Fares) + ledger.total(MoneyCategory::Deliveries);
    let upkeep =
        -(ledger.total(MoneyCategory::TrainOpex) + ledger.total(MoneyCategory::TrackMaintenance));
    (income, upkeep, ledger.paid_runs())
}

/// Run `minutes` real minutes, sampling the ledger at every minute boundary.
fn measure_minutes(app: &mut App, minutes: u32) -> Vec<Minute> {
    let mut out = Vec::new();
    let mut previous = totals(app);
    for minute in 1..=minutes {
        run(app, REAL_MINUTE);
        let now = totals(app);
        let ledger = app.world().resource::<MoneyLedger>();
        out.push(Minute {
            minute,
            income: now.0 - previous.0,
            upkeep: now.1 - previous.1,
            runs: now.2 - previous.2,
            cash: app.world().resource::<Money>().cents(),
            net_including_capex: ledger.session_income() - ledger.session_expense(),
        });
        previous = now;
    }
    out
}

/// Ticks until the first fare is collected, giving up after `limit`.
fn ticks_to_first_delivery(app: &mut App, limit: u32) -> Option<u32> {
    for tick in 1..=limit {
        run(app, 1);
        if app.world().resource::<MoneyLedger>().paid_runs() > 0 {
            return Some(tick);
        }
    }
    None
}

/// What this line costs to hold per real minute, from the constants rather than
/// the ledger — track, the stops rail actually reaches, and one transit's crew.
fn line_upkeep_per_real_min(app: &App) -> i64 {
    let network = app.world().resource::<TrackNetwork>();
    let stations = app.world().resource::<StationRegistry>();
    track_maintenance_total(network)
        + station_maintenance_billed(network, stations)
        + TRANSIT_PROFILE.opex_cents_per_real_min
}

fn print_table(label: &str, rows: &[Minute]) {
    eprintln!("\n=== {label} ===");
    eprintln!(
        "{:>4}  {:>12}  {:>12}  {:>10}  {:>6}  {:>13}  {:>15}",
        "min", "income/min", "upkeep/min", "net/min", "runs", "cash", "session net"
    );
    for r in rows {
        eprintln!(
            "{:>4}  {:>12}  {:>12}  {:>10}  {:>6}  {:>13}  {:>15}",
            r.minute,
            format!("${}", r.income / 100),
            format!("${}", r.upkeep / 100),
            format!("${}", r.operating_net() / 100),
            r.runs,
            format!("${}", r.cash / 100),
            format!("${}", r.net_including_capex / 100),
        );
    }
}

// ---------------------------------------------------------------------------
// The acceptance bar
// ---------------------------------------------------------------------------

/// **The opening beat pays out by minute three and clears its capital by ten.**
///
/// Design 02 §4.1 is binding: the first line is *"affordable within the first
/// minute, connectable within the second, and paying out by the third."*
///
/// # Where the ten comes from
///
/// It is derived, not chosen. The line above costs
/// `21 tiles x $100 = $2,100` to build and `$3,000` for the transit — `$5,100`
/// of capital against `$10,000` of opening cash. At the surplus this measures
/// (`~$658/min`) that is `5,100 / 658 = 7.8` minutes, so the session net crosses
/// zero in minute eight. Ten is that with headroom for terrain: the owner's real
/// line cost `$3,300` to build rather than `$2,100`, which is `$6,300` of
/// capital and minute ten on the nose.
///
/// # What it read before
///
/// The same script, same constants, before `spawn_demand_jobs` was fixed:
///
/// ```text
///  min    income/min    upkeep/min     net/min    runs           cash
///    1           $37          $470       $-432       2          $3147
///    2            $0          $470       $-470       0          $2677
///    ...
///    8            $0          $327       $-327       0             $0     <- broke
///   15            $0            $0          $0       0             $0
/// ```
///
/// Two fares, ever, and bankrupt by minute eight — which is the session the
/// owner played. Three things were wrong and all three are named in the code
/// that fixes them: the station-pair walk in `economy::jobs` never walked, the
/// job board silted up with runs no train could make, and every settlement the
/// world invented billed station upkeep the moment it appeared.
#[test]
fn the_opening_line_pays_out_by_minute_three_and_clears_its_capital_by_ten() {
    let mut app = cold_start_world();
    build_the_opening_line(&mut app);

    let tiles = app.world().resource::<TrackNetwork>().len();
    let separation = rail_sim::haul_tiles(HOME, AWAY);
    let upkeep_rate = line_upkeep_per_real_min(&app);
    eprintln!(
        "opening line: {tiles} tiles of track, endpoints {separation} tiles apart, \
         ${} a run, ${}/min to hold",
        passenger_fare_cents(separation) / 100,
        upkeep_rate / 100
    );

    // Time to first delivery, measured before the minute table so the table
    // starts from a standing train rather than a warmed-up one.
    let first = ticks_to_first_delivery(&mut app, REAL_MINUTE * 3);
    let first = first.expect("a fare inside three real minutes, per brief 02 §4.1");
    eprintln!(
        "first delivery at tick {first} ({:.2} real minutes)",
        f64::from(first) / f64::from(REAL_MINUTE)
    );
    assert!(
        first < REAL_MINUTE,
        "the first payout has to land inside the first minute of running, not \
         just eventually — took {first} ticks"
    );

    let rows = measure_minutes(&mut app, 12);
    print_table("cold start: L of 20 tiles, sep 10, 1 transit", &rows);

    // Brief 02 §4.1 — paying out by the third minute. Every minute of it, in
    // fact: a line that only clears its costs on average is one the player
    // watches bleed.
    for row in rows.iter().take(3) {
        assert!(
            row.operating_net() > 0,
            "minute {} of the opening beat must earn more than it costs to run \
             — ${}/min income against ${}/min upkeep",
            row.minute,
            row.income / 100,
            row.upkeep / 100,
        );
    }

    // The surplus has to be worth something, not a rounding error above break
    // even: brief 08 §1 is that money *paces* expansion, so there has to be
    // something to expand with.
    let third = rows[2];
    assert!(
        third.income > third.upkeep * 2,
        "by minute three the line should be earning multiples of what it costs, \
         or the second line is unaffordable — ${}/min against ${}/min",
        third.income / 100,
        third.upkeep / 100,
    );

    let paid_back = rows
        .iter()
        .find(|r| r.net_including_capex > 0)
        .map(|r| r.minute);
    eprintln!("session net turns positive in minute {paid_back:?}");
    assert!(
        matches!(paid_back, Some(m) if m <= 10),
        "capital has to be cleared inside the first ten minutes — see the \
         derivation above; got {paid_back:?}"
    );

    // Nothing may drift toward the floor: the owner's session hit $0 at minute
    // eight and stopped collecting upkeep, which is how a dead railway looks
    // solvent.
    assert!(
        rows.iter().all(|r| r.cash > STARTING_CASH_CENTS / 4),
        "the balance must never approach the floor on a working opening line"
    );
}

/// Two identical worlds produce an identical ledger, minute for minute.
///
/// The pair walk is indexed by a counter derived from the sim tick and the
/// station list is sorted before it is walked; both are there so that the same
/// seed sends the same people between the same towns. A fare scales with
/// distance now, so an unsorted walk would not merely reorder events — it would
/// pay different money for them.
#[test]
fn the_opening_beat_is_deterministic() {
    let measure = || {
        let mut app = cold_start_world();
        build_the_opening_line(&mut app);
        measure_minutes(&mut app, 4)
    };
    let a = measure();
    let b = measure();
    assert_eq!(a, b, "two identical sessions must produce one ledger");
}

// ---------------------------------------------------------------------------
// The three defects, each pinned where it can be seen
// ---------------------------------------------------------------------------

/// The board carries work the railway can make, and not the rest of the world's
/// wishes.
///
/// This is the one that killed the session. `spawn_new_demand` plants a new
/// settlement every few minutes and each is *unconnected by definition*; the
/// board is a fixed-size queue with no expiry, so every pair between two
/// unreachable villages held a slot forever. Measured before the fix: nine jobs
/// standing at minute fifteen, **none** of them between the two stations the
/// player had actually joined, and the train stopped for good.
#[test]
fn the_board_never_silts_up_with_runs_no_train_can_make() {
    let mut app = cold_start_world();
    build_the_opening_line(&mut app);
    let home = station_at(&app, HOME);
    let away = station_at(&app, AWAY);

    // Long enough for the demand spawner to plant several opportunities.
    run(&mut app, REAL_MINUTE * 15);

    let board = app.world().resource::<JobBoard>();
    eprintln!("board at minute 15: {} jobs", board.jobs.len());
    for job in &board.jobs {
        eprintln!("  {:?} @ ${}", job.kind, job.reward_cents / 100);
    }
    assert!(
        app.world().resource::<StationRegistry>().len() > 3,
        "the world should have spoken up by now, or this proves nothing"
    );
    for job in &board.jobs {
        match job.kind {
            JobKind::Passenger { from, to } => assert!(
                (from == home || from == away) && (to == home || to == away),
                "a job between stops no train can reach is holding a slot: {:?}",
                job.kind
            ),
            JobKind::Goods { .. } => panic!(
                "no industry on this map has a railhead: {:?}",
                job.kind
            ),
        }
    }
}

/// Stops the world invented, and the player never reached, are not billed.
///
/// A fresh world opens with three seeded anchors and grows a new settlement
/// every few minutes for the rest of the session, none of which the player
/// built or paid for. Billing them charged `$30`/min each from the moment they
/// appeared: the opening beat's upkeep read `$440`/min at minute one and
/// `$500`/min by minute fifteen with no change to the railway at all.
#[test]
fn upkeep_is_flat_while_the_railway_is() {
    let mut app = cold_start_world();
    build_the_opening_line(&mut app);
    run(&mut app, REAL_MINUTE);

    let rows = measure_minutes(&mut app, 15);
    let stations_before = app.world().resource::<StationRegistry>().len();
    assert!(
        stations_before > 3,
        "the demand spawner should have planted something by minute sixteen"
    );

    let first = rows[0].upkeep;
    for row in &rows {
        assert_eq!(
            row.upkeep / 100,
            first / 100,
            "upkeep moved from ${}/min to ${}/min in minute {} without the \
             player laying a tile — the world was billing them for its own \
             opportunities",
            first / 100,
            row.upkeep / 100,
            row.minute,
        );
    }
    eprintln!(
        "upkeep held at ${}/min across fifteen minutes and {} new anchors",
        first / 100,
        stations_before - 3
    );
}

// ---------------------------------------------------------------------------
// The second shape: a compact three-stop local line
// ---------------------------------------------------------------------------
//
// The opening pair is the *first* thing a player builds. This is the second,
// and the owner reported it as "barely break even" on a build that already had
// the demand fixes above: three stops inside a ten-by-ten footprint, an L round
// a corner, one tile of water bridged.
//
// It sits in the worst region of every curve at once, which is why it is worth
// pinning:
//
//   * fares are super-linear in **endpoint** separation, so short hops are paid
//     worst of all — and a compact line is nothing but short hops;
//   * the fare pays endpoint distance while the train runs *track* distance, so
//     an L pays a nine-tile fare for eighteen tiles of running;
//   * maintenance and opex are flat standing charges that do not care how short
//     the line is;
//   * the bridged tile carries `BRIDGE_MAINT_WEIGHT` = 4, so one tile of water
//     costs what four tiles of ground do.

/// The three stops, an L inside a ten-by-ten box.
///
/// Sited well clear of the seeded anchors and their industries — `MIN_STATION_
/// SPACING` refuses a platform within three tiles of another stop, and a local
/// line accidentally sharing a catchment with the opening pair would not be
/// measuring itself.
const LOCAL_A: TileCoord = TileCoord { x: 6, y: 46 };
const LOCAL_B: TileCoord = TileCoord { x: 15, y: 46 };
const LOCAL_C: TileCoord = TileCoord { x: 15, y: 55 };
/// The stream, four tiles along the first leg.
const LOCAL_FORD: TileCoord = TileCoord { x: 10, y: 46 };

/// Build the owner's compact three-stop line through the real command path.
///
/// Tiers are what a player would actually pick: the one town gets a `Station`,
/// the two lineside halts get `Halt`s. All three are **player-built and paid
/// for**, which is the pessimistic reading — a real session would have had one
/// or two of them free as seeded anchors, and the payback arithmetic for that
/// variant is in the test below.
fn build_the_local_line(app: &mut App, bridged: bool) {
    for (from, to) in [(LOCAL_A, LOCAL_B), (LOCAL_B, LOCAL_C)] {
        push(
            app,
            CommandKind::AutoFillTrack(AutoFillTrack {
                from,
                to,
                layer: GROUND_LAYER,
            }),
        );
    }
    run(app, 2);
    assert!(
        !bridged
            || app
                .world()
                .resource::<TrackNetwork>()
                .iter()
                .any(|p| p.is_bridge()),
        "the stream should have been bridged"
    );

    for (tile, tier) in [
        (LOCAL_A, StationTier::Station),
        (LOCAL_B, StationTier::Halt),
        (LOCAL_C, StationTier::Halt),
    ] {
        push(
            app,
            CommandKind::PlaceStation(PlaceStation::new(tile, GROUND_LAYER, tier, None)),
        );
    }
    push(
        app,
        CommandKind::BuyTrain(BuyTrain {
            kind: TrainKind::Transit,
        }),
    );
    run(app, 2);

    let bought = app
        .world()
        .resource::<TrainYard>()
        .unplaced()
        .first()
        .map(|(id, _)| *id)
        .expect("a transit in the yard");
    let a = station_at(app, LOCAL_A);
    push(
        app,
        CommandKind::PlaceTrain(PlaceTrain {
            train: bought,
            at_station: a,
        }),
    );
    run(app, 1);
}

/// A world with no seeded anchors anywhere near the local line.
///
/// The generator's anchors are planted where they always are, so the demand
/// spawner and the maintenance pass see a realistic world; none of them is on
/// the player's line, so what this measures is that line and nothing else.
fn local_line_world(bridged: bool) -> App {
    let mut app = cold_start_world();
    if bridged {
        app.insert_resource(terrain_with_stream(64, 64, LOCAL_FORD));
    }
    build_the_local_line(&mut app, bridged);
    app
}

/// What the ledger says a compact local line is worth, with and without water.
fn measure_local_line(bridged: bool) -> (Vec<Minute>, i64, i64) {
    let mut app = local_line_world(bridged);
    for tile in [LOCAL_A, LOCAL_B, LOCAL_C] {
        assert!(
            app.world()
                .resource::<StationRegistry>()
                .at(tile, GROUND_LAYER)
                .is_some(),
            "the stop at {tile:?} was refused — the line is not three stops"
        );
    }
    let capex = {
        let ledger = app.world().resource::<MoneyLedger>();
        -(ledger.total(MoneyCategory::Construction) + ledger.total(MoneyCategory::RollingStock))
    };
    let upkeep_rate = line_upkeep_per_real_min(&app);
    let rows = measure_minutes(&mut app, 15);
    (rows, capex, upkeep_rate)
}

/// **A compact three-stop local line is a living, not a treadmill.**
///
/// The owner's words, on a build that already had the demand fixes: *"even a
/// basic 3-stop line within ~10x10 with one tile water bridge is barely break
/// even."*
///
/// # The target, derived
///
/// A line like this should be **clearly profitable but modest**: it is a local
/// branch, not a trunk. The bar is that steady operating income is at least
/// **1.5x** its running costs, and that the capital comes back inside the
/// opening pair's worst measured case — sixteen minutes, from
/// `rail_town/tests/cold_start_pays_out.rs`. Below 1.5x the player is watching a
/// number hover, which is the "treadmill" reading; far above it and a compact
/// line would dominate the long haul the fare curve exists to reward.
///
/// # The arithmetic it has to beat
///
/// Nineteen tiles of track, one of them bridged, so the maintenance weight is
/// `18 x 1 + 1 x 4 = 22`. Plus one `Station` and two `Halt`s, plus one transit:
///
/// ```text
/// track   22 weight x $10/min  = $220/min
/// stops   $30 + $10 + $10      =  $50/min
/// train                        = $140/min
///                                -------
///                                $410/min
/// ```
///
/// Capital is `$1,800` of ground, `$800` for the bridged tile, `$2,000` of
/// platforms and `$3,000` of stock — `$7,600` of the opening `$10,000`. A
/// session that inherited two of the three stops as seeded anchors pays
/// `$1,600` less than that.
#[test]
fn a_compact_three_stop_local_line_is_a_living_not_a_treadmill() {
    let (bridged, capex, upkeep_rate) = measure_local_line(true);
    let (dry, dry_capex, dry_upkeep) = measure_local_line(false);

    print_table("compact local: 3 stops in 10x10, one bridge", &bridged);
    print_table("the same line with no water to cross", &dry);
    eprintln!(
        "\nbridged: ${} of capital, ${}/min to hold\n\
         dry:     ${} of capital, ${}/min to hold\n\
         the one bridged tile costs ${}/min and ${} to build",
        capex / 100,
        upkeep_rate / 100,
        dry_capex / 100,
        dry_upkeep / 100,
        (upkeep_rate - dry_upkeep) / 100,
        (capex - dry_capex) / 100,
    );

    let steady = bridged[2];
    eprintln!(
        "steady: ${}/min income against ${}/min upkeep = {:.2}x",
        steady.income / 100,
        steady.upkeep / 100,
        steady.income as f64 / steady.upkeep.max(1) as f64,
    );

    // Clearly profitable, not knife-edge: 1.5x its running costs, held.
    for row in bridged.iter().skip(2) {
        assert!(
            row.income * 2 >= row.upkeep * 3,
            "minute {}: a compact local line earned ${}/min against ${}/min of \
             running costs ({:.2}x) — that is the treadmill the owner reported, \
             not a living",
            row.minute,
            row.income / 100,
            row.upkeep / 100,
            row.income as f64 / row.upkeep.max(1) as f64,
        );
    }

    // …and modest: a local branch must not out-earn the reach the fare curve
    // exists to reward. Design 08 §2 — long hauls pay disproportionately.
    assert!(
        steady.income < passenger_fare_cents(60) * 20,
        "a three-stop local line grossing ${}/min would make reaching outward \
         pointless",
        steady.income / 100
    );

    let paid_back = bridged
        .iter()
        .find(|r| r.net_including_capex > 0)
        .map(|r| r.minute);
    eprintln!("capital cleared in minute {paid_back:?} (${} of it)", capex / 100);
    assert!(
        matches!(paid_back, Some(m) if m <= 16),
        "a compact local line must clear its capital inside the opening pair's \
         worst case of sixteen minutes; got {paid_back:?}"
    );
}
