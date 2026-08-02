//! Credit money when a loaded train reaches its destination.

use bevy_ecs::prelude::*;

use crate::money::Money;
use crate::stations::{IndustryRegistry, StationRegistry, StationService};
use crate::track::{TrackNetwork, GROUND_LAYER};
use crate::trains::{track_for_station, Train, TrainCargo, TrainLocation};

use super::ledger::{MoneyCategory, MoneyLedger};

/// Passenger fare on delivery: $5.00.
pub const PASSENGER_FARE_CENTS: i64 = 500;
/// Goods delivery payout: $20.00.
pub const GOODS_DELIVERY_CENTS: i64 = 2_000;

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
            TrainCargo::Passengers { to, .. } => {
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
                ledger.credit(&mut money, MoneyCategory::Fares, PASSENGER_FARE_CENTS);
                service.record_arrival(to);
                *cargo = TrainCargo::Empty;
                loc.begin_dwell(train.kind);
            }
            TrainCargo::Goods { to, .. } => {
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
                ledger.credit(&mut money, MoneyCategory::Deliveries, GOODS_DELIVERY_CENTS);
                *cargo = TrainCargo::Empty;
                loc.begin_dwell(train.kind);
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

    #[test]
    fn passenger_delivery_credits_money() {
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
            PASSENGER_FARE_CENTS
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
    }
}
