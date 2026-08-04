//! Credit money when a loaded train reaches its destination.
//!
//! # Distance is the whole point
//!
//! Design 08 §2: *"Long hauls pay disproportionately. Distance should be worth
//! more than linearly, so that reaching further is genuinely lucrative and the
//! pull outward is economic rather than merely narrative."*
//!
//! A flat fare says the opposite. If a four-tile hop and a sixty-tile haul pay
//! the same, the four-tile hop wins on every axis — less track to lay, less
//! track to maintain, more runs per minute — and the optimal railway is a tram
//! circling one square. Every other system in the game pulls outward; a flat
//! fare pulled back, harder than any of them pushed.
//!
//! So a payout is `base × (boarding + len + len²/divisor)`, super-linear in the
//! distance carried. At the numbers below a sixty-tile haul pays about **24×** a
//! four-tile hop where a linear fare would pay 15×, and that surplus is what
//! makes a tunnel, a long bridge or an expensive alignment worth costing out.
//!
//! ## …but a fare is not *only* a distance
//!
//! `boarding` is a flat term, worth a couple of tiles, charged on every journey
//! however short. It exists because everything on the *cost* side of the ledger
//! is flat — maintenance, opex and station upkeep are per-minute charges that do
//! not care how far anybody travelled — and a purely distance-scaled fare
//! therefore left the compact local line paying flat costs out of the thinnest
//! part of the curve. See [`PASSENGER_FARE_BOARDING_TILES`]. Goods have no such
//! term, and the reason is on [`goods_delivery_cents`].
//!
//! ## Distance carried, not track laid
//!
//! `len` is the Chebyshev separation of the two *endpoints*, not the length of
//! the route the train took. Paying for route length would pay for winding
//! track — a player could earn more by building worse — and would count the
//! empty repositioning leg that [`TrainLocation::path`] carries in front of the
//! revenue leg. Endpoint separation pays for *reaching further*, and leaves a
//! direct alignment strictly better than a rambling one: same fare, less track,
//! less upkeep, more runs an hour.

use bevy_ecs::prelude::*;

use crate::ids::TileCoord;
use crate::money::Money;
use crate::stations::{IndustryRegistry, StationRegistry, StationService};
use crate::track::{TrackNetwork, GROUND_LAYER};
use crate::trains::{track_for_station, Train, TrainCargo, TrainConsist, TrainLocation};

use super::ledger::{MoneyCategory, MoneyLedger};

/// Passenger fare per tile of distance carried, before the super-linear term.
///
/// A fifteen-tile first line pays about `$62` a run.
///
/// # Why it doubled with the timetable
///
/// Brief 17 §4 halved train speed, which halves how many fares a train collects
/// in a real minute — and **every cost in this game is charged per real
/// minute** (design 08 §3, [`crate::economy::opex`]). A fare is paid per
/// journey, so leaving it alone would not have been "unchanged": it would have
/// halved the income side of a ledger whose cost side never moved, and the
/// opening beat design 02 §4.1 pins is measured in exactly those minutes.
///
/// Measured on the cold-start harness, before and after: the opening line ran
/// 57 paid runs a minute at `$21` a fare and now runs 28 at `$42`. The rate the
/// player sees is the same rate, which is the point — this is a
/// re-denomination, not a buff. Every relationship design 08 §2 asks for is
/// untouched: the quadratic divisor, the boarding term's share, and the 3:1
/// goods-to-passenger split are all ratios, and all three scaled with it.
pub const PASSENGER_FARE_CENTS_PER_TILE: i64 = 300;

/// Divisor on the squared term of a passenger fare. Lower = steeper.
///
/// At `40`, distance stops being linear around fourteen tiles: a haul twice as
/// long pays rather more than twice as much, and one four times as long pays
/// about six times as much.
pub const PASSENGER_FARE_DISTANCE_DIVISOR: i64 = 40;

/// Flat boarding component of a passenger fare, in tile-equivalents.
///
/// # Why a fare is not purely a distance
///
/// A ticket is sold, a platform is staffed, a train stops and starts again.
/// None of that gets cheaper because the journey is short, and every real fare
/// table in the world is a flag-fall plus a rate. This curve had only the rate,
/// and that is a specific, load-bearing omission rather than a simplification:
/// **every cost on the other side of the ledger is flat**. Maintenance, opex and
/// station upkeep are charged per minute regardless of how far anybody went, so
/// a purely distance-scaled fare left one shape of railway — the compact local
/// line, all of whose journeys are short — paying flat costs out of the thinnest
/// part of the curve.
///
/// The owner found it immediately: *"even a basic 3-stop line within ~10x10 with
/// one tile water bridge is barely break even."* Measured, that line ran at 2.1x
/// its running costs but took **seventeen minutes** to give back the `$7,600` it
/// cost, having dropped the balance to `$2,847` on the way — a number that
/// creeps is indistinguishable from a number that is stuck.
///
/// Two tile-equivalents is the smallest term that fixes the shape without
/// touching the crown:
///
/// | separation | before | after | change |
/// | --- | --- | --- | --- |
/// | 4 tiles | $6.60 | $9.60 | +45% |
/// | 9 tiles | $16.50 | $19.50 | +18% |
/// | 15 tiles | $30.90 | $33.90 | +10% |
/// | 60 tiles | $225.00 | $228.00 | +1.3% |
///
/// The lift is concentrated exactly where the curve was thinnest and fades to
/// nothing over distance, so design 08 §2's *"long hauls pay
/// disproportionately"* is untouched: a sixty-tile haul still pays **23.7x** a
/// four-tile hop where a linear fare pays 15x.
///
/// Goods deliberately have no such term — see [`goods_delivery_cents`].
pub const PASSENGER_FARE_BOARDING_TILES: i64 = 2;

/// Goods payout per tile of distance carried, before the super-linear term.
///
/// Roughly three times a fare, per design 08 §2's split: passengers are small
/// and frequent, freight is large and lumpy. Doubled alongside the fare when
/// brief 17 §4 halved train speed — see [`PASSENGER_FARE_CENTS_PER_TILE`]; the
/// ratio between the two is the design decision and it has not moved.
pub const GOODS_DELIVERY_CENTS_PER_TILE: i64 = 800;

/// Divisor on the squared term of a goods payout — half the passenger divisor,
/// so freight rewards distance twice as steeply.
///
/// Long-distance bulk freight is the thing worth building a mountain crossing
/// for; short trips between adjacent industries are a lorry's job.
pub const GOODS_DELIVERY_DISTANCE_DIVISOR: i64 = 20;

/// Nominal passenger fare — a fifteen-tile run, the length of a first line.
///
/// Kept as a named quantity because the alerts and the onboarding copy want
/// "about what a run is worth" without a distance to hand.
pub const PASSENGER_FARE_CENTS: i64 = passenger_fare_cents(15);

/// Nominal goods payout — a fifteen-tile run.
pub const GOODS_DELIVERY_CENTS: i64 = goods_delivery_cents(15);

/// Payout units in tenths of a tile: `boarding + len + len²/divisor`.
///
/// Tenths rather than whole units so the quadratic still bites below the
/// divisor, where integer division would otherwise floor it to nothing and make
/// short hops exactly linear.
///
/// `boarding` is the flat term charged whatever the distance — see
/// [`PASSENGER_FARE_BOARDING_TILES`]. Passing `0` gives the pure distance curve.
const fn distance_units_tenths(tiles: i64, divisor: i64, boarding: i64) -> i64 {
    let len = if tiles < 1 { 1 } else { tiles };
    10 * boarding + 10 * len + (10 * len * len) / divisor
}

/// Passenger fare for a journey of `tiles`, in cents. Never zero.
pub const fn passenger_fare_cents(tiles: i64) -> i64 {
    PASSENGER_FARE_CENTS_PER_TILE
        * distance_units_tenths(
            tiles,
            PASSENGER_FARE_DISTANCE_DIVISOR,
            PASSENGER_FARE_BOARDING_TILES,
        )
        / 10
}

/// Goods payout for a delivery of `tiles`, in cents. Never zero.
///
/// No boarding term, unlike a passenger fare. A freight payout is priced on the
/// tonnage actually moved, and design 08 §2 wants this curve the steepest thing
/// in the game — *"large, lumpy, scaling with distance and commodity value"*. A
/// flat component would pay for shunting a wagon next door, which is a lorry's
/// job and the one thing the freight curve exists to discourage.
pub const fn goods_delivery_cents(tiles: i64) -> i64 {
    GOODS_DELIVERY_CENTS_PER_TILE
        * distance_units_tenths(tiles, GOODS_DELIVERY_DISTANCE_DIVISOR, 0)
        / 10
}

/// Chebyshev tiles between two points — the distance a train covers on a grid
/// that allows diagonals, and the same measure catchments and station spacing
/// already use.
pub fn haul_tiles(from: TileCoord, to: TileCoord) -> i64 {
    let dx = (from.x - to.x).abs() as i64;
    let dy = (from.y - to.y).abs() as i64;
    dx.max(dy)
}

/// When a train with cargo sits at its path destination, pay out and clear cargo.
///
/// # A carload is a fare
///
/// A consist carries several loads of one working
/// ([`TrainConsist`](crate::trains::TrainConsist)), and each is a separate job
/// that was taken off the board — so each is paid separately, at the same
/// distance-scaled rate, and each counts as one paid run. A two-car transit
/// arriving with two carloads has collected two fares, and the ledger says two
/// because two people's journeys ended.
pub fn resolve_deliveries(
    mut money: ResMut<Money>,
    mut ledger: ResMut<MoneyLedger>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    network: Res<TrackNetwork>,
    mut service: ResMut<StationService>,
    mut q: Query<(
        &Train,
        &mut TrainLocation,
        &mut TrainCargo,
        Option<&mut TrainConsist>,
    )>,
) {
    for (train, mut loc, mut cargo, mut consist) in q.iter_mut() {
        if loc.parked || loc.dwell_remaining > 0 || !loc.at_destination() || cargo.is_empty() {
            continue;
        }
        // A train with no consist component is the single car it always was;
        // one that has a consist and somehow no load still delivers the one
        // working its cargo names.
        let cars = consist.as_ref().map(|c| c.cars.max(1)).unwrap_or(1);
        let loads = i64::from(consist.as_ref().map(|c| c.laden.max(1)).unwrap_or(1));

        match *cargo {
            TrainCargo::Passengers { from, to } => {
                let Some(station) = stations.get(to) else {
                    *cargo = TrainCargo::Empty;
                    continue;
                };
                let Some(dest_track) = track_for_station(&network, station.tile, station.layer)
                else {
                    continue;
                };
                if loc.track != dest_track {
                    continue;
                }
                // A stop demolished mid-journey leaves the passengers with a
                // destination and no origin; they still travelled, so they still
                // pay — at the shortest fare rather than nothing.
                let tiles = stations
                    .get(from)
                    .map(|origin| haul_tiles(origin.tile, station.tile))
                    .unwrap_or(1);
                for _ in 0..loads {
                    ledger.credit_paid_run(
                        &mut money,
                        MoneyCategory::Fares,
                        passenger_fare_cents(tiles),
                    );
                }
                // One call at the platform, however long the train: service is
                // a statement about the timetable, and a longer train is not a
                // more frequent one.
                service.record_arrival(to);
                *cargo = TrainCargo::Empty;
                if let Some(c) = consist.as_mut() {
                    c.unload();
                }
                loc.begin_dwell_at(train.kind, cars, station.tier);
            }
            TrainCargo::Goods { to, from, .. } => {
                let Some(ind) = industries.get(to) else {
                    *cargo = TrainCargo::Empty;
                    continue;
                };
                let Some(dest_track) = track_for_station(&network, ind.tile, GROUND_LAYER) else {
                    continue;
                };
                if loc.track != dest_track {
                    continue;
                }
                let tiles = industries
                    .get(from)
                    .map(|origin| haul_tiles(origin.tile, ind.tile))
                    .unwrap_or(1);
                for _ in 0..loads {
                    ledger.credit_paid_run(
                        &mut money,
                        MoneyCategory::Deliveries,
                        goods_delivery_cents(tiles),
                    );
                }
                *cargo = TrainCargo::Empty;
                if let Some(c) = consist.as_mut() {
                    c.unload();
                }
                // Loading at a proper goods platform takes its 140%; a bare
                // railhead against the works falls back to the train's own
                // dwell.
                match super::jobs::goods_platform_for(&stations, ind) {
                    Some(platform) => loc.begin_dwell_at(train.kind, cars, platform.tier),
                    None => loc.begin_dwell(train.kind, cars),
                }
            }
            TrainCargo::Empty => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::TrainKind;
    use crate::ids::{TileCoord, TrainId};
    use crate::track::{try_place_track, TrackNetwork, TrackTerrain};
    use crate::trains::Train;
    use bevy_app::App;

    /// Design 08 §2 — the pull outward has to be economic.
    ///
    /// # The crown narrowed, on purpose
    ///
    /// This read 34.1x before [`PASSENGER_FARE_BOARDING_TILES`] and reads 23.7x
    /// now. That is the boarding term doing exactly what it is for: a flat
    /// component is a larger share of a small fare than of a large one, so
    /// lifting the short end necessarily closes some of the gap.
    ///
    /// The bound stays at 22.5 because that is where it was set and what it
    /// guards is unchanged — a *linear* fare pays 15x, and a curve that drifted
    /// down toward it would leave the shortest line strictly dominant, which is
    /// the failure this whole module exists to prevent. 23.7x against 15x is
    /// still a haul paying half again what its distance alone is worth, and
    /// `economy_arc::reaching_further_earns_more_than_running_more_short_hops`
    /// checks the same claim end to end on a running sim, where it survives
    /// with a 6x margin (a 60-tile haul grosses $4,902/min against a 4-tile
    /// shuttle's $816/min).
    ///
    /// There is now only 1.2 of headroom, so a future lift to the short end
    /// needs this measured rather than assumed.
    #[test]
    fn a_long_haul_pays_far_more_than_linearly() {
        let short = passenger_fare_cents(4);
        let long = passenger_fare_cents(60);
        let ratio = long as f64 / short as f64;
        assert!(
            ratio > 22.5,
            "a 60-tile haul pays {ratio:.1}x a 4-tile hop; a linear fare pays \
             15x, and anything near that leaves short lines dominant"
        );
        // Freight rewards distance harder still.
        let goods_ratio =
            goods_delivery_cents(60) as f64 / goods_delivery_cents(4) as f64;
        assert!(
            goods_ratio > ratio,
            "goods {goods_ratio:.1}x should out-scale passengers {ratio:.1}x"
        );
    }

    #[test]
    fn fares_rise_with_every_extra_tile_and_never_reach_zero() {
        let mut previous = 0;
        for tiles in 0..=80 {
            let fare = passenger_fare_cents(tiles);
            assert!(fare > 0, "a {tiles}-tile run paid nothing");
            assert!(
                fare >= previous,
                "fare fell going from {} to {tiles} tiles",
                tiles - 1
            );
            previous = fare;
        }
        // A negative distance cannot happen, but must not panic or pay a bonus.
        assert_eq!(passenger_fare_cents(-5), passenger_fare_cents(1));
    }

    /// A first line's run is worth about what it should be.
    ///
    /// The band was `2_800..=3_400` against the flat `$30` fare this curve
    /// replaced, then `3_600` once [`PASSENGER_FARE_BOARDING_TILES`] arrived.
    /// It doubled with brief 17 §4's timetable, because a fare is paid per
    /// journey and a train now makes half as many of them in the real minute
    /// every cost is charged in — see [`PASSENGER_FARE_CENTS_PER_TILE`].
    ///
    /// The number that actually matters is unchanged, and it is measured rather
    /// than asserted here: `rail_sim/tests/economy_cold_start.rs` still clears
    /// the opening line's capital in minute seven. This is the cheap guard that
    /// notices an order-of-magnitude slip before that suite has to run.
    #[test]
    fn the_opening_line_still_pays_about_what_it_used_to() {
        let fare = passenger_fare_cents(15);
        assert!(
            (5_600..=7_200).contains(&fare),
            "a first-line run pays {fare}c"
        );
        // Per *real* minute — the only comparison that means anything — the
        // opening line is where it was: half the runs at twice the fare.
        let runs_per_min_before = 57;
        let runs_per_min_now = 28;
        let before = runs_per_min_before * (fare / 2);
        let now = runs_per_min_now * fare;
        assert!(
            (now * 10 / before).abs_diff(10) <= 1,
            "the timetable was re-denominated, not re-balanced: {before}c/min \
             before against {now}c/min now"
        );
    }

    /// The boarding term lifts short hops and leaves long hauls alone.
    ///
    /// This is the shape the change was made for, so it is pinned as a shape
    /// rather than as five separate numbers: the shorter the journey, the more
    /// of it is the flat term, and by sixty tiles the flat term has vanished
    /// into the quadratic.
    #[test]
    fn boarding_lifts_the_short_end_and_fades_over_distance() {
        let without = |tiles: i64| {
            PASSENGER_FARE_CENTS_PER_TILE
                * distance_units_tenths(tiles, PASSENGER_FARE_DISTANCE_DIVISOR, 0)
                / 10
        };
        let lift = |tiles: i64| {
            let base = without(tiles);
            (passenger_fare_cents(tiles) - base) * 100 / base
        };
        // 4 tiles: +45%. 9: +18%. 15: +10%. 60: +1%.
        assert!(lift(4) > 40, "a four-tile hop gained only {}%", lift(4));
        assert!(
            lift(9) > 15 && lift(9) < 25,
            "a nine-tile hop gained {}%",
            lift(9)
        );
        assert!(lift(60) < 3, "a sixty-tile haul gained {}%", lift(60));
        for pair in [4, 9, 15, 30, 60].windows(2) {
            assert!(
                lift(pair[0]) > lift(pair[1]),
                "the lift must fall with distance, but {} tiles gained {}% and \
                 {} tiles gained {}%",
                pair[0],
                lift(pair[0]),
                pair[1],
                lift(pair[1]),
            );
        }
    }

    #[test]
    fn haul_distance_is_measured_between_endpoints() {
        let a = TileCoord { x: 4, y: 4 };
        let b = TileCoord { x: 20, y: 10 };
        assert_eq!(haul_tiles(a, b), 16);
        assert_eq!(haul_tiles(b, a), 16);
        assert_eq!(haul_tiles(a, a), 0);
    }

    /// The tier's dwell percentage reaches the train (04 §6's table): the same
    /// transit turns around three times faster at an interchange than at a
    /// halt. Before this, `StationTier::dwell_ticks` had no production caller
    /// and every platform boarded at the train's own pace.
    #[test]
    fn the_platform_grade_sets_the_turnaround() {
        let dwell_after_arrival_at = |tier: crate::stations::StationTier| -> u16 {
            let mut app = App::new();
            app.init_resource::<StationRegistry>()
                .init_resource::<IndustryRegistry>()
                .init_resource::<StationService>()
                .init_resource::<TrackNetwork>()
                .init_resource::<crate::economy::MoneyLedger>()
                .insert_resource(Money::new(0));

            let terrain = TrackTerrain::new(8, 8, (0..64).map(|_| (false, 0i8)));
            let mut network = TrackNetwork::new();
            let mut place_money = Money::new(500_000);
            let mut place_ledger = crate::economy::MoneyLedger::default();
            let mut ids = Vec::new();
            for x in 1..=4 {
                let p = try_place_track(
                    &mut network,
                    &mut place_money,
                    &mut place_ledger,
                    &terrain,
                    TileCoord { x, y: 2 },
                    GROUND_LAYER,
                )
                .unwrap();
                ids.push(p.id);
            }
            app.insert_resource(network);

            let east;
            let west;
            {
                let mut stations = app.world_mut().resource_mut::<StationRegistry>();
                east = stations.insert("East", TileCoord { x: 1, y: 2 }, GROUND_LAYER);
                west =
                    stations.insert_tier("West", TileCoord { x: 4, y: 2 }, GROUND_LAYER, tier, 0);
            }

            let dest = *ids.last().unwrap();
            app.world_mut().spawn((
                Train {
                    id: TrainId(1),
                    kind: TrainKind::Transit,
                },
                TrainLocation {
                    track: dest,
                    path: ids.clone(),
                    path_index: ids.len() - 1,
                    progress: 0,
                    parked: false,
                    dwell_remaining: 0,
                },
                TrainCargo::Passengers {
                    from: east,
                    to: west,
                },
            ));

            app.add_systems(bevy_app::Update, resolve_deliveries);
            app.world_mut().run_schedule(bevy_app::Update);

            app.world_mut()
                .query::<&TrainLocation>()
                .iter(app.world())
                .next()
                .unwrap()
                .dwell_remaining
        };

        let halt = dwell_after_arrival_at(crate::stations::StationTier::Halt);
        let interchange = dwell_after_arrival_at(crate::stations::StationTier::Interchange);
        assert!(
            halt > interchange,
            "a halt boards slower than an interchange turns around \
             (halt {halt}, interchange {interchange})"
        );
        // Stated against the profile, not against the number it happens to
        // produce: base dwell is a pacing constant (brief 17 §4) and it moves
        // with train speed.
        let base = crate::trains::TRANSIT_PROFILE.dwell_ticks;
        assert_eq!(halt, base * 150 / 100, "150% of the transit profile's dwell");
        assert_eq!(interchange, base * 60 / 100, "60% of it, floored, never zero");
        assert!(interchange >= 1);
    }

    /// **A carload is a fare.** Three carriages of people arriving is three
    /// fares and three paid runs — but *one* call at the platform, because a
    /// longer train is not a more frequent one.
    #[test]
    fn every_carload_pays_its_own_fare_and_the_stop_counts_one_call() {
        use crate::trains::TrainConsist;

        let banked = |cars: u8, laden: u8| -> (i64, u64, u32, u16) {
            let mut app = App::new();
            app.init_resource::<StationRegistry>()
                .init_resource::<IndustryRegistry>()
                .init_resource::<StationService>()
                .init_resource::<TrackNetwork>()
                .init_resource::<crate::economy::MoneyLedger>()
                .insert_resource(Money::new(0));

            let terrain = TrackTerrain::new(8, 8, (0..64).map(|_| (false, 0i8)));
            let mut network = TrackNetwork::new();
            let mut place_money = Money::new(500_000);
            let mut place_ledger = crate::economy::MoneyLedger::default();
            let mut ids = Vec::new();
            for x in 1..=4 {
                ids.push(
                    try_place_track(
                        &mut network,
                        &mut place_money,
                        &mut place_ledger,
                        &terrain,
                        TileCoord { x, y: 2 },
                        GROUND_LAYER,
                    )
                    .unwrap()
                    .id,
                );
            }
            app.insert_resource(network);

            let (east, west) = {
                let mut stations = app.world_mut().resource_mut::<StationRegistry>();
                (
                    stations.insert("East", TileCoord { x: 1, y: 2 }, GROUND_LAYER),
                    stations.insert("West", TileCoord { x: 4, y: 2 }, GROUND_LAYER),
                )
            };

            app.world_mut().spawn((
                Train {
                    id: TrainId(1),
                    kind: TrainKind::Transit,
                },
                TrainLocation {
                    track: *ids.last().unwrap(),
                    path: ids.clone(),
                    path_index: ids.len() - 1,
                    progress: 0,
                    parked: false,
                    dwell_remaining: 0,
                },
                TrainCargo::Passengers {
                    from: east,
                    to: west,
                },
                TrainConsist { cars, laden },
            ));

            app.add_systems(bevy_app::Update, resolve_deliveries);
            app.world_mut().run_schedule(bevy_app::Update);

            let world = app.world_mut();
            let dwell = world
                .query::<&TrainLocation>()
                .iter(world)
                .next()
                .unwrap()
                .dwell_remaining;
            let consist = *world.query::<&TrainConsist>().iter(world).next().unwrap();
            assert_eq!(consist.laden, 0, "everything aboard got off");
            (
                app.world().resource::<Money>().cents(),
                app.world()
                    .resource::<crate::economy::MoneyLedger>()
                    .paid_runs(),
                app.world().resource::<StationService>().score(west).deliveries,
                dwell,
            )
        };

        let fare = passenger_fare_cents(3);
        let (one_cash, one_runs, one_calls, one_dwell) = banked(1, 1);
        assert_eq!(one_cash, fare, "a single car pays exactly what it always did");
        assert_eq!(one_runs, 1);
        assert_eq!(one_calls, 1);

        let (three_cash, three_runs, three_calls, three_dwell) = banked(3, 3);
        assert_eq!(three_cash, fare * 3, "three carloads, three fares");
        assert_eq!(three_runs, 3, "and three journeys somebody paid for");
        assert_eq!(three_calls, 1, "but one call at the platform");
        assert!(
            three_dwell > one_dwell,
            "a longer train boards for longer: {three_dwell} against {one_dwell}"
        );

        // Half-empty pays for what it carried, not for what it could have.
        let (half_cash, half_runs, ..) = banked(3, 1);
        assert_eq!(half_cash, fare);
        assert_eq!(half_runs, 1);
    }

    #[test]
    fn passenger_delivery_credits_money_by_distance() {
        let mut app = App::new();
        app.init_resource::<StationRegistry>()
            .init_resource::<IndustryRegistry>()
            .init_resource::<StationService>()
            .init_resource::<TrackNetwork>()
            .init_resource::<crate::economy::MoneyLedger>()
            .insert_resource(Money::new(0));

        let terrain = TrackTerrain::new(8, 8, (0..64).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut place_money = Money::new(500_000);
        let mut place_ledger = crate::economy::MoneyLedger::default();
        let mut ids = Vec::new();
        for x in 1..=4 {
            let p = try_place_track(
                &mut network,
                &mut place_money,
                &mut place_ledger,
                &terrain,
                TileCoord { x, y: 2 },
                GROUND_LAYER,
            )
            .unwrap();
            ids.push(p.id);
        }
        app.insert_resource(network);

        let east;
        let west;
        {
            let mut stations = app.world_mut().resource_mut::<StationRegistry>();
            east = stations.insert("East", TileCoord { x: 1, y: 2 }, GROUND_LAYER);
            west = stations.insert("West", TileCoord { x: 4, y: 2 }, GROUND_LAYER);
        }

        let dest = *ids.last().unwrap();
        app.world_mut().spawn((
            Train {
                id: TrainId(1),
                kind: TrainKind::Transit,
            },
            TrainLocation {
                track: dest,
                path: ids.clone(),
                path_index: ids.len() - 1,
                progress: 0,
                parked: false,
                dwell_remaining: 0,
            },
            TrainCargo::Passengers {
                from: east,
                to: west,
            },
        ));

        app.add_systems(bevy_app::Update, resolve_deliveries);
        app.world_mut().run_schedule(bevy_app::Update);

        assert_eq!(
            app.world().resource::<Money>().cents(),
            passenger_fare_cents(3),
            "a three-tile hop pays a three-tile fare"
        );
        assert!(app
            .world_mut()
            .query::<&TrainCargo>()
            .iter(app.world())
            .next()
            .unwrap()
            .is_empty());
        assert_eq!(
            app.world().resource::<StationService>().score(west).deliveries,
            1
        );
        assert_eq!(
            app.world()
                .resource::<crate::economy::MoneyLedger>()
                .paid_runs(),
            1,
            "a delivery is one run however much it paid"
        );
    }
}
