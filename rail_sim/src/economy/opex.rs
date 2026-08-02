//! Per-train operating cost and track maintenance each Advance tick (soft-fail).

use bevy_ecs::prelude::*;

use crate::money::Money;
use crate::stations::{station_maintenance_total, StationRegistry};
use crate::track::{piece_maintenance_cents, TrackNetwork};
use crate::trains::{Train, TrainLocation, TrainProfile};

use super::ledger::{MoneyCategory, MoneyLedger};

/// Legacy / average opex constant (alerts). Prefer [`TrainProfile::opex_cents`].
pub const TRAIN_OPEX_CENTS: i64 = 10;

/// Debit opex for each unparked train using its kind profile.
/// On insufficient funds, park the train (do not delete).
pub fn apply_train_opex(
    mut money: ResMut<Money>,
    mut ledger: ResMut<MoneyLedger>,
    mut q: Query<(&Train, &mut TrainLocation)>,
) {
    for (train, mut loc) in q.iter_mut() {
        let opex = TrainProfile::for_kind(train.kind).opex_cents;
        if loc.parked {
            if money.can_afford(opex) {
                if ledger
                    .try_debit(&mut money, MoneyCategory::TrainOpex, opex)
                    .is_ok()
                {
                    loc.parked = false;
                }
            }
            continue;
        }
        if ledger
            .try_debit(&mut money, MoneyCategory::TrainOpex, opex)
            .is_err()
        {
            loc.parked = true;
        }
    }
}

/// Sum per-tile track maintenance for the current network.
pub fn track_maintenance_total(network: &TrackNetwork) -> i64 {
    network
        .iter()
        .map(|p| piece_maintenance_cents(p.is_bridge()))
        .sum()
}

/// Debit track maintenance each Advance tick.
///
/// On insufficient funds: drain remaining cash into maintenance, park all
/// trains, leave track in place (soft-fail — never delete).
pub fn apply_track_maintenance(
    mut money: ResMut<Money>,
    mut ledger: ResMut<MoneyLedger>,
    network: Res<TrackNetwork>,
    stations: Res<StationRegistry>,
    mut trains: Query<&mut TrainLocation>,
) {
    // A station is a kind of track, so its upkeep shares the same bucket.
    let total = track_maintenance_total(&network) + station_maintenance_total(&stations);
    if total <= 0 {
        return;
    }
    if ledger
        .try_debit(&mut money, MoneyCategory::TrackMaintenance, total)
        .is_ok()
    {
        return;
    }
    // Soft-fail: take what we can, park everything, keep the network.
    let avail = money.cents();
    if avail > 0 {
        let _ = ledger.try_debit(&mut money, MoneyCategory::TrackMaintenance, avail);
    }
    for mut loc in trains.iter_mut() {
        loc.parked = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::TileCoord;
    use crate::track::{try_place_track, TrackTerrain, GROUND_LAYER, TRACK_MAINT_CENTS};

    #[test]
    fn maintenance_ticks_debit_per_tile() {
        let terrain = TrackTerrain::new(4, 4, (0..16).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut money = Money::new(100_000);
        let mut ledger = MoneyLedger::default();

        try_place_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            TileCoord { x: 1, y: 1 },
            GROUND_LAYER,
        )
        .unwrap();
        try_place_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            TileCoord { x: 2, y: 1 },
            GROUND_LAYER,
        )
        .unwrap();

        let before = money.cents();
        let due = track_maintenance_total(&network);
        assert_eq!(due, 2 * TRACK_MAINT_CENTS);

        ledger
            .try_debit(&mut money, MoneyCategory::TrackMaintenance, due)
            .unwrap();
        assert_eq!(money.cents(), before - due);
        assert_eq!(ledger.total(MoneyCategory::TrackMaintenance), -due);
    }

    #[test]
    fn maintenance_soft_fail_parks_without_deleting_track() {
        let terrain = TrackTerrain::new(4, 4, (0..16).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut money = Money::new(50_000);
        let mut ledger = MoneyLedger::default();
        try_place_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            TileCoord { x: 0, y: 0 },
            GROUND_LAYER,
        )
        .unwrap();

        // Bare ECS-ish soft-fail path: can't afford maintenance → drain + keep track.
        money = Money::new(0);
        let total = track_maintenance_total(&network);
        assert!(total > 0);
        assert!(ledger
            .try_debit(&mut money, MoneyCategory::TrackMaintenance, total)
            .is_err());
        assert_eq!(network.len(), 1);
    }
}
