//! The session arc, measured rather than asserted from memory.
//!
//! Brief 08 is a set of claims about *rates*: a well-run early network outruns
//! its costs, track that carries nothing starts costing more than it earns, and
//! pruning it visibly restores the rate. None of those can be checked against a
//! constant — they are properties of a running network over a stretch of real
//! time — so this file scripts two representative networks, runs them, and
//! measures what the ledger actually collected.
//!
//! It exists because the numbers were wrong for a long time in both directions:
//! once 64× too high (a save emptied in two minutes, `docs/BURNDOWN.md`), then
//! 640× too low (upkeep at 3% of gross, one train paying for five thousand
//! tiles of dead track). Both passed every unit test in the crate, because a
//! unit test on a constant cannot see a unit error in the constant.
//!
//! # Units
//!
//! Everything here is per **real minute**, the clock the player is sitting at.
//! `FixedUpdate` runs at 64 Hz, so a real minute is
//! [`TICKS_PER_REAL_MINUTE`] = 3,840 ticks. A *sim*-minute is six ticks. See
//! `rail_sim::economy::opex` for the table.

use bevy_app::{App, FixedUpdate};
use rail_sim::economy::opex::{train_opex_total_cents_per_real_min, TICKS_PER_REAL_MINUTE};
use rail_sim::ids::{StationId, TileCoord, TrackId, TrainId};
use rail_sim::{
    passenger_fare_cents, station_maintenance_billed, station_maintenance_total,
    track_maintenance_total, DemandSpawner,
    GoodKind, IndustryRegistry, Money, MoneyCategory, MoneyLedger, SimPlugin, StationRegistry,
    StationService, TrackNetwork, TrackTerrain, Train, TrainCargo, TrainKind, TrainLocation,
    WorldAnchorsSeeded, GROUND_LAYER, MAINT_CENTS_PER_WEIGHT_PER_REAL_MIN, TRACK_MAINT_WEIGHT,
};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Ticks in one real minute, as a loop count.
const REAL_MINUTE: u32 = TICKS_PER_REAL_MINUTE as u32;

/// Real minutes a rate is averaged over before it is compared to another rate.
///
/// A window is a **sample**, and how good a sample it is depends on how many
/// paid runs fall inside it — the network's jobs are a mix of long and short
/// hauls, so a window holding few of them reports a rate with real spread in
/// it, and comparing two such rates amplifies the spread again because upkeep
/// is a fixed subtraction.
///
/// This was `2`, which held about ninety runs when a transit crossed a tile in
/// three ticks. Brief 17 §4 halved train speed, and a two-minute window then
/// held forty-five: the same claim, measured half as well. Measured at six, the
/// working and pruned windows of the test below read `$2,401` and `$2,402` a
/// minute — flat, the way they were before. **It is the sample size being held
/// constant here, not the answer.**
const RATE_WINDOW_MINUTES: u32 = 6;

/// Flat, dry, buildable ground — the network is the variable, not the terrain.
fn flat_terrain(w: u32, h: u32) -> TrackTerrain {
    TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 2i8)))
}

/// A headless world with the full sim schedule and nothing changing under it.
fn world(w: u32, h: u32) -> App {
    let mut app = App::new();
    app.add_plugins(SimPlugin);
    app.insert_resource(flat_terrain(w, h));
    // The generator's own anchors would move the numbers; this file builds its
    // networks by hand.
    app.insert_resource(WorldAnchorsSeeded(true));
    // Likewise new demand: a settlement appearing mid-measurement would change
    // the network being measured.
    app.world_mut()
        .resource_mut::<DemandSpawner>()
        .ticks_until_next = u32::MAX;
    // Construction is not what is being measured; keep the treasury out of the
    // way so nothing soft-fails for lack of funds.
    app.insert_resource(Money::new(1_000_000_000));
    app
}

/// Lay a straight run of track and return the ids, in order.
fn lay(app: &mut App, from: TileCoord, to: TileCoord) -> Vec<TrackId> {
    let tiles = rail_sim::straight_line(from, to).expect("axis-aligned run");
    let terrain = app.world().resource::<TrackTerrain>().clone();
    let mut ids = Vec::new();
    app.world_mut()
        .resource_scope(|world, mut network: bevy_ecs::prelude::Mut<TrackNetwork>| {
            world.resource_scope(|world, mut money: bevy_ecs::prelude::Mut<Money>| {
                world.resource_scope(|_w, mut ledger: bevy_ecs::prelude::Mut<MoneyLedger>| {
                    for tile in &tiles {
                        match rail_sim::track::try_place_track(
                            &mut network,
                            &mut money,
                            &mut ledger,
                            &terrain,
                            *tile,
                            GROUND_LAYER,
                        ) {
                            Ok(placed) => ids.push(placed.id),
                            // Already laid (a crossing tile); reuse it.
                            Err(_) => {
                                if let Some(id) = network.id_at(*tile, GROUND_LAYER) {
                                    ids.push(id);
                                }
                            }
                        }
                    }
                });
            });
        });
    ids
}

/// Register a stop on a tile that already carries track.
fn station(app: &mut App, name: &str, tile: TileCoord) -> StationId {
    let id = app
        .world_mut()
        .resource_mut::<StationRegistry>()
        .insert(name, tile, GROUND_LAYER);
    app.world_mut().resource_mut::<StationService>().ensure(id);
    id
}

fn industry(
    app: &mut App,
    name: &str,
    tile: TileCoord,
    produces: Option<GoodKind>,
    consumes: Option<GoodKind>,
) {
    app.world_mut()
        .resource_mut::<IndustryRegistry>()
        .insert(name, tile, produces, consumes);
}

/// Put a train on the rails at `track`.
fn train(app: &mut App, id: u64, kind: TrainKind, track: TrackId) {
    app.world_mut().spawn((
        Train {
            id: TrainId(id),
            kind,
        },
        TrainLocation::at_track(track),
        TrainCargo::Empty,
    ));
}

fn run(app: &mut App, ticks: u32) {
    for _ in 0..ticks {
        app.world_mut().run_schedule(FixedUpdate);
    }
}

/// What a stretch of running the sim earned and spent, per real minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rates {
    /// Fares + deliveries.
    gross: i64,
    /// Track / station maintenance and train opex.
    upkeep: i64,
    /// Deliveries completed.
    runs: u64,
}

impl Rates {
    fn net(self) -> i64 {
        self.gross - self.upkeep
    }

    /// Upkeep as a percentage of gross; `999` when nothing was earned.
    fn upkeep_percent(self) -> i64 {
        if self.gross <= 0 {
            return 999;
        }
        self.upkeep * 100 / self.gross
    }

    fn describe(self, label: &str) -> String {
        format!(
            "{label}: gross ${}/min over {} runs, upkeep ${}/min ({}%), net ${}/min",
            self.gross / 100,
            self.runs,
            self.upkeep / 100,
            self.upkeep_percent(),
            self.net() / 100
        )
    }
}

/// Run for `minutes` real minutes and report the operating rates.
///
/// **Operating** deliberately excludes construction and rolling stock: those
/// are capital, and brief 08 §3.1's claim is about what a standing network
/// costs to *hold*. Folding a one-off build into the rate would make a network
/// look ruinous the minute it was laid and healthy forever after — which is
/// exactly the reading that hid the real bug.
fn measure(app: &mut App, minutes: u32) -> Rates {
    let before = totals(app);
    run(app, REAL_MINUTE * minutes);
    let after = totals(app);
    let per_min = |a: i64, b: i64| (b - a) / i64::from(minutes);
    Rates {
        gross: per_min(before.0, after.0),
        upkeep: per_min(before.1, after.1),
        runs: after.2 - before.2,
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

fn network_len(app: &App) -> usize {
    app.world().resource::<TrackNetwork>().len()
}

// ---------------------------------------------------------------------------
// The two networks
// ---------------------------------------------------------------------------

/// Two stops fifteen tiles apart, sixteen tiles of track, one transit train.
/// The first ten minutes of a session (brief 08 §7).
fn early_network() -> App {
    let mut app = world(48, 48);
    let ids = lay(&mut app, TileCoord { x: 5, y: 10 }, TileCoord { x: 20, y: 10 });
    station(&mut app, "Ashford", TileCoord { x: 5, y: 10 });
    station(&mut app, "Brackwell", TileCoord { x: 20, y: 10 });
    train(&mut app, 1, TrainKind::Transit, ids[0]);
    app
}

/// Four stops, a goods route, and 94 tiles of track — the hour mark.
///
/// The main line and the branch are both **double track**, which is not
/// gold-plating: three free-roaming trains on single track meet head-on and
/// deadlock, and a network measured while gridlocked measures nothing. Slack is
/// what design 07 §4.3 expects a player to have bought by this point, and it is
/// paid for here in the only currency that matters to this file — 94 tiles of
/// upkeep for what is 47 tiles of route.
fn mid_network() -> App {
    let mut app = world(64, 64);
    let across = lay(&mut app, TileCoord { x: 8, y: 20 }, TileCoord { x: 40, y: 20 });
    lay(&mut app, TileCoord { x: 8, y: 21 }, TileCoord { x: 40, y: 21 });
    let down = lay(&mut app, TileCoord { x: 24, y: 6 }, TileCoord { x: 24, y: 19 });
    lay(&mut app, TileCoord { x: 25, y: 6 }, TileCoord { x: 25, y: 19 });

    station(&mut app, "Westbrook", TileCoord { x: 8, y: 20 });
    station(&mut app, "Eastgate", TileCoord { x: 40, y: 20 });
    station(&mut app, "Northfield", TileCoord { x: 24, y: 6 });
    station(&mut app, "Midvale", TileCoord { x: 24, y: 20 });
    industry(
        &mut app,
        "Quarry Ridge",
        TileCoord { x: 24, y: 8 },
        Some(GoodKind::Ore),
        None,
    );
    industry(
        &mut app,
        "Harbor Foundry",
        TileCoord { x: 38, y: 20 },
        None,
        Some(GoodKind::Ore),
    );

    train(&mut app, 1, TrainKind::Transit, across[0]);
    train(&mut app, 2, TrainKind::Transit, *across.last().unwrap());
    train(&mut app, 3, TrainKind::Transport, down[0]);
    app
}

/// Track laid where nobody goes — the overextension the brief is built around.
///
/// A comb of sidings well clear of the working network, so nothing routes over
/// it and nothing is served by it. Two hundred tiles is about what an
/// enthusiastic ten minutes of building produces.
fn add_dead_track(app: &mut App, tiles: i32) {
    let mut laid = 0;
    let mut y = 44;
    while laid < tiles && y < 62 {
        let span = (tiles - laid).min(40);
        lay(
            app,
            TileCoord { x: 5, y },
            TileCoord { x: 5 + span - 1, y },
        );
        laid += span;
        y += 2;
    }
}

/// Rip up every tile at or below `from_y` — the pruning pass.
fn demolish_dead_track(app: &mut App, from_y: i32) {
    let doomed: Vec<TrackId> = app
        .world()
        .resource::<TrackNetwork>()
        .iter()
        .filter(|p| p.tile.y >= from_y)
        .map(|p| p.id)
        .collect();
    app.world_mut()
        .resource_scope(|world, mut network: bevy_ecs::prelude::Mut<TrackNetwork>| {
            world.resource_scope(|world, mut money: bevy_ecs::prelude::Mut<Money>| {
                world.resource_scope(|_w, mut ledger: bevy_ecs::prelude::Mut<MoneyLedger>| {
                    for id in &doomed {
                        let _ =
                            rail_sim::track::try_demolish(&mut network, &mut money, &mut ledger, *id);
                    }
                });
            });
        });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The opening network pays for itself, with room to expand.
///
/// Brief 08 §7: by minute ten the player has three or four stations and a first
/// profit. Upkeep has to be a *felt* share of that — big enough that holding
/// ground is a decision, small enough that the first line is never a mistake.
///
/// # The share moved, and why
///
/// The band below used to be `20..=45`, and this line measured 27%. It now
/// measures 14% against **identical constants**, because the line got twice as
/// much work done rather than because anything got cheaper:
///
/// | | before | after |
/// | --- | --- | --- |
/// | runs in three minutes | 128 | 245 |
/// | gross | $1,318/min | $2,523/min |
/// | upkeep | $360/min | $360/min |
///
/// `spawn_demand_jobs` picked its station pair by indexing the raw sim tick,
/// which advances by exactly the spawn interval between waves; with two
/// stations, every other wave picked the same station for both ends and posted
/// nothing, so this train stood idle for half of every cycle. The two-station
/// networks in this file all roughly doubled when that was fixed; the
/// four-station ones did not move at all (`mid` went $2,428 → $2,409), and
/// neither did the 60-tile haul, which is travel-bound rather than demand-bound.
/// See `rail_sim/tests/economy_cold_start.rs`.
///
/// So the *floor* here was calibrated against a line idling half the time and
/// has been lowered to match what the same standing cost is a share of now. It
/// is a sanity bound, not a tuning target: the mechanism it was standing in for
/// — track that carries nothing costing more than it earns — is measured
/// directly by [`dead_track_sinks_the_network_and_pruning_brings_it_back`], and
/// the share is *supposed* to vary with shape.
///
/// A later change — the flat boarding component in
/// `rail_sim::economy::payout::PASSENGER_FARE_BOARDING_TILES` — moved every one
/// of these again, downward and hardest at the short end, which is what a
/// boarding term is for. Currently measured across this file: **30%** for a
/// four-tile shuttle (was 44%), **33%** for the opening beat, **13%** here,
/// **62%** for the mid network, **16%** for a sixty-tile haul. That spread is
/// the design — reaching further keeps more of what it earns — and the floor
/// still has three points under the tightest reading.
#[test]
fn an_early_network_clears_its_upkeep_with_room_to_expand() {
    let mut app = early_network();
    run(&mut app, REAL_MINUTE); // warm up: first jobs, first runs
    let rates = measure(&mut app, 3);
    eprintln!("{}", rates.describe("early: 2 stops, 16 tiles, 1 transit"));

    assert!(
        rates.net() > 0,
        "the opening line must pay for itself — {}",
        rates.describe("early")
    );
    assert!(
        (10..=45).contains(&rates.upkeep_percent()),
        "upkeep should stay a visible share of gross: a rounding error means \
         overextension cannot happen, a stranglehold means expansion cannot — {}",
        rates.describe("early")
    );
    assert!(
        rates.net() > rates.upkeep,
        "surplus should exceed running costs, so expansion is fundable — {}",
        rates.describe("early")
    );
}

/// The mid-session network is profitable at a larger scale, with margin to
/// commit to expensive terrain (brief 08 §2 — "a well-run network outruns its
/// costs comfortably, and that surplus is meant to be spent expanding").
#[test]
fn a_mid_session_network_is_profitable_at_a_larger_scale() {
    let mut app = mid_network();
    run(&mut app, REAL_MINUTE);
    let rates = measure(&mut app, 3);
    eprintln!("{}", rates.describe("mid: 4 stops, 94 tiles, 3 trains"));

    assert!(
        rates.net() > 0,
        "a working four-stop network must be in profit — {}",
        rates.describe("mid")
    );
    assert!(
        rates.net() * 5 > rates.gross,
        "a fifth of gross should survive as margin, or there is nothing to \
         expand with — {}",
        rates.describe("mid")
    );
    assert!(
        rates.gross > 200_000,
        "a network this size should gross well over $2,000/min — {}",
        rates.describe("mid")
    );
}

/// **The overextension trap, and the way out of it.**
///
/// Brief 08 §3.1: *"track that isn't carrying enough starts costing more than
/// it earns"*. §9.2: pruning it visibly restores the rate. Both halves are one
/// test because either alone is satisfiable by a broken economy — a game where
/// all track is ruinous passes the first, and a game where none is passes the
/// second.
#[test]
fn dead_track_sinks_the_network_and_pruning_brings_it_back() {
    let mut app = mid_network();
    // Settle before sampling. The claim is about *steady-state* rates, and the
    // job board takes a few real minutes to reach one: it posts work on its own
    // cadence and the trains take it off at theirs, so the queue depth — and
    // with it the mix of long and short hauls a train is choosing between —
    // is still moving for the first few minutes of a fresh network. A single
    // minute of warmup was enough when trains ran at twice this speed and
    // emptied the board as fast as it filled; at brief 17 §4's timetable it
    // leaves the first window measuring the transient and the last one
    // measuring the steady state, and then compares them.
    run(&mut app, REAL_MINUTE * 6);
    let working = measure(&mut app, RATE_WINDOW_MINUTES);
    eprintln!("{}", working.describe("mid, working"));
    assert!(working.net() > 0, "baseline must be profitable");

    let before_tiles = network_len(&app);
    add_dead_track(&mut app, 200);
    let overextended_tiles = network_len(&app);
    assert!(
        overextended_tiles >= before_tiles + 190,
        "expected ~200 more tiles, got {}",
        overextended_tiles - before_tiles
    );

    let overextended = measure(&mut app, RATE_WINDOW_MINUTES);
    eprintln!("{}", overextended.describe("mid + 200 dead tiles"));
    assert!(
        overextended.net() < 0,
        "200 tiles carrying nothing must cost more than the network earns — {}",
        overextended.describe("overextended")
    );
    assert!(
        overextended.gross > working.gross / 2,
        "the dead branch must sink the network by costing money, not by \
         stopping the trains — {}",
        overextended.describe("overextended")
    );

    demolish_dead_track(&mut app, 44);
    assert!(
        network_len(&app) <= before_tiles,
        "pruning should leave the working network only"
    );
    let pruned = measure(&mut app, RATE_WINDOW_MINUTES);
    eprintln!("{}", pruned.describe("mid, pruned"));
    assert!(
        pruned.net() > 0,
        "pruning must restore the rate — {}",
        pruned.describe("pruned")
    );
    assert!(
        pruned.net() * 4 > working.net() * 3,
        "pruning should recover most of the original margin — {} vs {}",
        pruned.describe("pruned"),
        working.describe("working")
    );
}

/// The rate readout the player actually watches moves within a couple of
/// minutes of the network changing, in both directions (brief 08 §3.2, §9.2).
#[test]
fn the_on_screen_rate_follows_the_network_within_a_couple_of_minutes() {
    let mut app = mid_network();
    run(&mut app, REAL_MINUTE * 4);
    let healthy = app
        .world()
        .resource::<MoneyLedger>()
        .net_rate_cents_per_min();
    assert!(healthy > 0, "a working network reads positive: {healthy}");

    add_dead_track(&mut app, 200);
    // Long enough for the construction spend to age out of the rate window, so
    // what is left is the standing cost of the branch and nothing else.
    run(&mut app, REAL_MINUTE * 4);
    let sunk = app
        .world()
        .resource::<MoneyLedger>()
        .net_rate_cents_per_min();
    assert!(
        sunk < 0,
        "the strip must show the decline, not just the ledger: {sunk}"
    );

    demolish_dead_track(&mut app, 44);
    run(&mut app, REAL_MINUTE * 4);
    let recovered = app
        .world()
        .resource::<MoneyLedger>()
        .net_rate_cents_per_min();
    assert!(
        recovered > 0,
        "pruning must visibly restore the rate: {recovered}"
    );
}

/// Paving the map is ruinous, and the numbers say why (brief 08 §9.1).
#[test]
fn paving_the_whole_map_costs_more_than_any_railway_earns() {
    let mut app = mid_network();
    run(&mut app, REAL_MINUTE);
    let working = measure(&mut app, RATE_WINDOW_MINUTES);

    // A 64x64 map is 4,096 tiles. The claim is about the *rate*, so charge the
    // standing upkeep of that directly rather than laying four thousand tiles.
    let paved = 4_096 * TRACK_MAINT_WEIGHT * MAINT_CENTS_PER_WEIGHT_PER_REAL_MIN;
    assert!(
        paved > working.gross * 4,
        "paving the map costs ${}/min against a working network's ${}/min gross \
         — that is not a trap, it is a rounding error",
        paved / 100,
        working.gross / 100
    );
}

/// Trains never stop for money (brief 08 §3.2 — running out is annoying and
/// recoverable, never terminal). `docs/BURNDOWN.md` playtest row 1: parking
/// them made bankruptcy permanent, because trains are the only income.
#[test]
fn a_broke_railway_keeps_running_and_earns_its_way_out() {
    let mut app = early_network();
    app.insert_resource(Money::new(0));
    run(&mut app, REAL_MINUTE);

    let parked = app
        .world_mut()
        .query::<&TrainLocation>()
        .iter(app.world())
        .filter(|loc| loc.parked)
        .count();
    assert_eq!(
        parked, 0,
        "a train parked for money can never earn its way out"
    );
    assert!(
        app.world().resource::<Money>().cents() >= 0,
        "the balance floors at zero; debt would be a second way to lose"
    );
    assert!(
        app.world()
            .resource::<MoneyLedger>()
            .total(MoneyCategory::Fares)
            > 0,
        "a broke railway still earns"
    );
    assert!(
        app.world().resource::<Money>().cents() > 0,
        "and recovers: after a minute the balance is off the floor"
    );
}

/// The upkeep the constants promise is the upkeep the ledger collects.
///
/// This is the unit bug that made the whole economy inert: the accrual divisor
/// and the authored rate have to mean the same minute. Off by 640 in either
/// direction, this test is the one that notices.
#[test]
fn the_upkeep_the_constants_promise_is_the_upkeep_that_is_collected() {
    let mut app = mid_network();
    let promised = {
        let network = app.world().resource::<TrackNetwork>();
        let stations = app.world().resource::<StationRegistry>();
        // `_billed` rather than `_total`: what the ledger collects is the
        // upkeep of stops rail has reached, and every stop in this network has.
        // The two agree here, and must — a world where they disagree is one
        // where the player is being charged for towns they never connected.
        assert_eq!(
            station_maintenance_billed(network, stations),
            station_maintenance_total(stations),
            "every stop in the mid network is on the railway"
        );
        track_maintenance_total(network) + station_maintenance_billed(network, stations)
    } + train_opex_total_cents_per_real_min(&[
        TrainKind::Transit,
        TrainKind::Transit,
        TrainKind::Transport,
    ]);

    let before = totals(&app);
    run(&mut app, REAL_MINUTE * 2);
    let after = totals(&app);
    let collected = (after.1 - before.1) / 2;

    let drift = (collected - promised).abs() * 100 / promised.max(1);
    assert!(
        drift <= 2,
        "authored upkeep is ${}/min but ${}/min was collected ({drift}% off) — \
         the accrual divisor and the authored rate disagree about what a minute is",
        promised / 100,
        collected / 100
    );
}

/// Long hauls pay disproportionately (brief 08 §2), measured end to end rather
/// than on the fare function alone.
#[test]
fn reaching_further_earns_more_than_running_more_short_hops() {
    // Same track, same train, same minutes — only the distance differs.
    let short = {
        let mut app = world(64, 64);
        let ids = lay(&mut app, TileCoord { x: 5, y: 10 }, TileCoord { x: 9, y: 10 });
        station(&mut app, "Ashford", TileCoord { x: 5, y: 10 });
        station(&mut app, "Brackwell", TileCoord { x: 9, y: 10 });
        train(&mut app, 1, TrainKind::Transit, ids[0]);
        run(&mut app, REAL_MINUTE);
        measure(&mut app, RATE_WINDOW_MINUTES)
    };
    let long = {
        let mut app = world(64, 64);
        let ids = lay(&mut app, TileCoord { x: 2, y: 10 }, TileCoord { x: 62, y: 10 });
        station(&mut app, "Ashford", TileCoord { x: 2, y: 10 });
        station(&mut app, "Brackwell", TileCoord { x: 62, y: 10 });
        train(&mut app, 1, TrainKind::Transit, ids[0]);
        run(&mut app, REAL_MINUTE);
        measure(&mut app, RATE_WINDOW_MINUTES)
    };
    eprintln!("{}", short.describe("4-tile shuttle"));
    eprintln!("{}", long.describe("60-tile haul"));

    assert!(
        long.runs < short.runs,
        "the long line must complete fewer runs, or this proves nothing"
    );
    assert!(
        long.gross > short.gross,
        "a 60-tile line grossing ${}/min against a 4-tile shuttle's ${}/min \
         means the shortest line still wins and nothing pulls outward",
        long.gross / 100,
        short.gross / 100
    );
}

/// The fare curve itself, stated as the ratio design 08 §2 asks for.
///
/// 34.1x before the fare gained a flat boarding component, 23.7x after — a
/// deliberate narrowing, since a flat term is a bigger share of a small fare
/// than a large one. The bound is unmoved: it guards the distance from a
/// *linear* 15x, and that distance is still comfortable. See
/// `rail_sim::economy::payout::PASSENGER_FARE_BOARDING_TILES` for the table and
/// the reason.
#[test]
fn a_long_haul_fare_is_far_more_than_linear() {
    let short = passenger_fare_cents(4);
    let long = passenger_fare_cents(60);
    let ratio = long as f64 / short as f64;
    assert!(
        ratio > 22.5,
        "a 60-tile haul pays {ratio:.1}x a 4-tile hop; a linear fare pays 15x, \
         and anything near that leaves short lines strictly dominant"
    );
}
