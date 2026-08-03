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
//! So a payout is `base × (len + len²/divisor)`, super-linear in the distance
//! carried. At the numbers below a sixty-tile haul pays about **34×** a
//! four-tile hop where a linear fare would pay 15×, and that surplus is what
//! makes a tunnel, a long bridge or an expensive alignment worth costing out.
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
use crate::trains::{track_for_station, Train, TrainCargo, TrainLocation};

use super::ledger::{MoneyCategory, MoneyLedger};

/// Passenger fare per tile of distance carried, before the super-linear term.
///
/// Sized so the fifteen-tile first line still pays about `$31` a run, which is
/// where the flat `$30` fare it replaces left the opening minutes.
pub const PASSENGER_FARE_CENTS_PER_TILE: i64 = 150;

/// Divisor on the squared term of a passenger fare. Lower = steeper.
///
/// At `40`, distance stops being linear around fourteen tiles: a haul twice as
/// long pays rather more than twice as much, and one four times as long pays
/// about six times as much.
pub const PASSENGER_FARE_DISTANCE_DIVISOR: i64 = 40;

/// Goods payout per tile of distance carried, before the super-linear term.
///
/// Roughly three times a fare, per design 08 §2's split: passengers are small
/// and frequent, freight is large and lumpy.
pub const GOODS_DELIVERY_CENTS_PER_TILE: i64 = 400;

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

/// Distance term in tenths of a tile-unit: `len + len²/divisor`, super-linear.
///
/// Tenths rather than whole units so the quadratic still bites below the
/// divisor, where integer division would otherwise floor it to nothing and make
/// short hops exactly linear.
const fn distance_units_tenths(tiles: i64, divisor: i64) -> i64 {
    let len = if tiles < 1 { 1 } else { tiles };
    10 * len + (10 * len * len) / divisor
}

/// Passenger fare for a journey of `tiles`, in cents. Never zero.
pub const fn passenger_fare_cents(tiles: i64) -> i64 {
    PASSENGER_FARE_CENTS_PER_TILE
        * distance_units_tenths(tiles, PASSENGER_FARE_DISTANCE_DIVISOR)
        / 10
}

/// Goods payout for a delivery of `tiles`, in cents. Never zero.
pub const fn goods_delivery_cents(tiles: i64) -> i64 {
    GOODS_DELIVERY_CENTS_PER_TILE * distance_units_tenths(tiles, GOODS_DELIVERY_DISTANCE_DIVISOR)
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
pub fn resolve_deliveries(
    mut money: ResMut<Money>,
    mut ledger: ResMut<MoneyLedger>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    network: Res<TrackNetwork>,
    mut service: ResMut<StationService>,
    mut q: Query<(&Train, &mut TrainLocation, &mut TrainCargo)>,
) {
    for (train, mut loc, mut cargo) in q.iter_mut() {
        if loc.parked || loc.dwell_remaining > 0 || !loc.at_destination() || cargo.is_empty() {
            continue;
        }

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
                ledger.credit_paid_run(
                    &mut money,
                    MoneyCategory::Fares,
                    passenger_fare_cents(tiles),
                );
                service.record_arrival(to);
                *cargo = TrainCargo::Empty;
                loc.begin_dwell_at(train.kind, station.tier);
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
                ledger.credit_paid_run(
                    &mut money,
                    MoneyCategory::Deliveries,
                    goods_delivery_cents(tiles),
                );
                *cargo = TrainCargo::Empty;
                // Loading at a proper goods platform takes its 140%; a bare
                // railhead against the works falls back to the train's own
                // dwell.
                match super::jobs::goods_platform_for(&stations, ind) {
                    Some(platform) => loc.begin_dwell_at(train.kind, platform.tier),
                    None => loc.begin_dwell(train.kind),
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

    #[test]
    fn the_opening_line_still_pays_about_what_it_used_to() {
        // A first line is fifteen-odd tiles. The flat fare it replaces was $30;
        // moving that materially would re-pace the whole opening.
        let fare = passenger_fare_cents(15);
        assert!(
            (2_800..=3_400).contains(&fare),
            "a first-line run pays {fare}c"
        );
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
        assert_eq!(halt, 3, "150% of the transit profile's 2 ticks");
        assert_eq!(interchange, 1, "60% of 2, floored, never zero");
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
