//! Running costs: per-train operating expense and track / station maintenance.
//!
//! # Rates are per minute, charged smoothly
//!
//! Every constant here is **cents per sim-minute**, because that is the unit
//! the player reads — the status strip shows a net `$/min` rate and the ledger
//! samples in the same unit. Charging happens every Advance tick, spread evenly
//! via a millicent accumulator ([`MaintenanceAccrual`]) so a rate far below one
//! cent per tick still accrues honestly instead of rounding to nothing.
//!
//! This matters more than it looks. Debiting a whole per-minute rate on every
//! tick multiplies every running cost by the tick rate, which turns a modest
//! network into thousands of dollars a minute and empties a new save in about
//! two minutes. The accumulator is what keeps the authored numbers meaning what
//! they say.
//!
//! # Running out of money is never terminal
//!
//! `DESIGN.md`: *"Money paces expansion. It never ends the game."* Trains are
//! the only source of income, so parking them when the balance hits zero makes
//! bankruptcy permanent — the player can never earn their way out. Instead the
//! balance floors at zero, unpaid upkeep is simply not collected, and **trains
//! keep running**. Only paid construction is blocked. Recovery is then
//! automatic, and the design's "prune and rebalance, not start over" is a real
//! option rather than advice the player cannot act on.

use bevy_ecs::prelude::*;

use crate::money::Money;
use crate::stations::{station_maintenance_total, StationRegistry};
use crate::track::{piece_maintenance_cents, TrackNetwork};
use crate::trains::{Train, TrainLocation, TrainProfile};

use super::ledger::{MoneyCategory, MoneyLedger};

/// Legacy / average opex constant (alerts). Prefer [`TrainProfile::opex_cents`].
pub const TRAIN_OPEX_CENTS: i64 = 10;

/// Advance ticks in one sim-minute, at the default 64 Hz fixed timestep.
///
/// Speed multipliers scale virtual time, so `FixedUpdate` runs proportionally
/// more often at 2× / 3× and upkeep accrues faster in real time — which is
/// correct, because more sim-minutes are passing.
pub const TICKS_PER_MINUTE: i64 = 64 * 60;

/// Cents per sim-minute charged per unit of track maintenance weight.
///
/// [`piece_maintenance_cents`] returns the *weight* of a piece (ground `1`,
/// bridge `4`); this turns that weight into money. Sixty tiles of plain track
/// therefore costs about `$12` a minute — enough that an unused branch is a
/// real liability, per `DESIGN.md`'s "track that isn't carrying enough starts
/// costing more than it earns", and far short of ruinous.
pub const MAINT_CENTS_PER_WEIGHT_PER_MINUTE: i64 = 20;

/// Fractional upkeep carried between ticks, in **cent-ticks**.
///
/// Rates are per minute and far below a cent per tick, so the remainder has to
/// live somewhere or every charge rounds to zero.
#[derive(Debug, Clone, Copy, Default, Resource)]
pub struct MaintenanceAccrual {
    track_cent_ticks: i64,
    train_cent_ticks: i64,
}

/// Take whole cents out of an accumulator, keeping the remainder.
///
/// The carry holds the *numerator* rather than a per-tick quotient. Dividing
/// first would truncate on every tick and quietly under-bill — at the rates
/// here, by about 4%.
fn accrue(carry: &mut i64, per_minute_cents: i64) -> i64 {
    *carry += per_minute_cents;
    let due = *carry / TICKS_PER_MINUTE;
    *carry -= due * TICKS_PER_MINUTE;
    due
}

/// Debit each train's operating cost.
///
/// Trains are **never** parked for lack of money — see the module docs. A train
/// that has been parked by something else unparks itself here once the balance
/// recovers.
pub fn apply_train_opex(
    mut money: ResMut<Money>,
    mut ledger: ResMut<MoneyLedger>,
    mut accrual: ResMut<MaintenanceAccrual>,
    mut q: Query<(&Train, &mut TrainLocation)>,
) {
    let per_minute: i64 = q
        .iter()
        .map(|(train, _)| TrainProfile::for_kind(train.kind).opex_cents)
        .sum();

    let due = accrue(&mut accrual.train_cent_ticks, per_minute);
    if due > 0 {
        // Collect what the balance can cover; the shortfall is simply not
        // collected. Debt would be a second way to lose, and there isn't one.
        let payable = due.min(money.cents().max(0));
        if payable > 0 {
            let _ = ledger.try_debit(&mut money, MoneyCategory::TrainOpex, payable);
        }
    }

    for (_, mut loc) in q.iter_mut() {
        if loc.parked {
            loc.parked = false;
        }
    }
}

/// Track maintenance for the whole network, in **cents per sim-minute**.
pub fn track_maintenance_total(network: &TrackNetwork) -> i64 {
    network
        .iter()
        .map(|p| piece_maintenance_cents(p.is_bridge()) * MAINT_CENTS_PER_WEIGHT_PER_MINUTE)
        .sum()
}

/// Debit track and station maintenance, spread evenly across the minute.
///
/// Never parks trains and never drains the balance below zero: unpaid upkeep is
/// left uncollected so income can recover the player. See the module docs.
pub fn apply_track_maintenance(
    mut money: ResMut<Money>,
    mut ledger: ResMut<MoneyLedger>,
    mut accrual: ResMut<MaintenanceAccrual>,
    network: Res<TrackNetwork>,
    stations: Res<StationRegistry>,
) {
    // A station is a kind of track, so its upkeep shares the same bucket.
    let per_minute = track_maintenance_total(&network) + station_maintenance_total(&stations);
    if per_minute <= 0 {
        return;
    }
    let due = accrue(&mut accrual.track_cent_ticks, per_minute);
    if due <= 0 {
        return;
    }
    let payable = due.min(money.cents().max(0));
    if payable > 0 {
        let _ = ledger.try_debit(&mut money, MoneyCategory::TrackMaintenance, payable);
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
        assert_eq!(
            due,
            2 * TRACK_MAINT_CENTS * MAINT_CENTS_PER_WEIGHT_PER_MINUTE
        );

        ledger
            .try_debit(&mut money, MoneyCategory::TrackMaintenance, due)
            .unwrap();
        assert_eq!(money.cents(), before - due);
        assert_eq!(ledger.total(MoneyCategory::TrackMaintenance), -due);
    }

    /// The bug this whole module is shaped around: charging a per-minute rate
    /// on every tick multiplies upkeep by the tick rate and empties a new save
    /// in about two minutes.
    #[test]
    fn a_minutes_upkeep_costs_a_minutes_worth_not_a_tick_times_a_minute() {
        let per_minute = 6_000; // $60/min
        let mut carry = 0i64;
        let charged: i64 = (0..TICKS_PER_MINUTE)
            .map(|_| accrue(&mut carry, per_minute))
            .sum();
        assert_eq!(charged, per_minute, "a minute of ticks bills one minute");
    }

    #[test]
    fn sub_cent_rates_still_accrue_instead_of_rounding_away() {
        // One tile at 20c/min is far below a cent per tick.
        let mut carry = 0i64;
        let charged: i64 = (0..TICKS_PER_MINUTE).map(|_| accrue(&mut carry, 20)).sum();
        assert_eq!(charged, 20);
    }

    #[test]
    fn a_modest_early_network_is_affordable_on_starting_cash() {
        // 60 tiles, 3 stations, 2 trains should be a fraction of the opening
        // balance per minute — the design wants money to pace expansion, not to
        // end the game before the first payout.
        let track = 60 * TRACK_MAINT_CENTS * MAINT_CENTS_PER_WEIGHT_PER_MINUTE;
        let stations = 3 * 300;
        let trains = 2 * 1_000;
        let per_minute = track + stations + trains;
        assert!(
            per_minute < crate::money::STARTING_CASH_CENTS / 20,
            "an early network costs {per_minute}c/min against a \
             {}c opening balance — that is under 20 minutes of runway",
            crate::money::STARTING_CASH_CENTS
        );
    }

    #[test]
    fn maintenance_never_takes_the_balance_below_zero() {
        let mut money = Money::new(150);
        let mut ledger = MoneyLedger::default();
        // More is due than the player has.
        let due = 900i64;
        let payable = due.min(money.cents().max(0));
        let _ = ledger.try_debit(&mut money, MoneyCategory::TrackMaintenance, payable);
        assert_eq!(money.cents(), 0, "upkeep floors at zero, it never goes debt");
        assert!(
            money.cents() >= 0,
            "a negative balance would be a second way to lose"
        );
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
