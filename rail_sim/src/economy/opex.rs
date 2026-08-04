//! Running costs: per-train operating expense and track / station maintenance.
//!
//! # Two minutes, and only one of them is the player's
//!
//! This crate has two clocks and they differ by a factor of **640**:
//!
//! | Unit | Ticks | In real time |
//! | --- | --- | --- |
//! | One `FixedUpdate` tick | 1 | 1/64 s |
//! | One **sim**-minute | [`TICKS_PER_SIM_MINUTE`] = 6 | 0.094 s |
//! | One **real** minute | [`TICKS_PER_REAL_MINUTE`] = 3,840 | 60 s |
//!
//! A tick advances the world by [`SIM_SECONDS_PER_TICK`] (10) sim-seconds, so
//! the town lives 640× faster than the wall clock: a sim *day* goes by in 2¼
//! real minutes.
//!
//! **Every running cost in this module is per _real_ minute**, because a rate
//! is something the player watches change while they sit there, and the status
//! strip and the ledger both report `$/min` in the minutes they are living in.
//! The constants say `REAL_MIN` in their names for one reason: an earlier
//! version wrote `TICKS_PER_MINUTE = 64 * 60` and called it a sim-minute, which
//! is the same 3,840 ticks but a different claim about what the numbers mean.
//! Read as sim-minutes, every authored rate was being collected at 1/640 of its
//! stated value — upkeep came to about 3% of gross income, one train paid for
//! five thousand tiles of dead track, and design 08 §3's overextension trap
//! could not occur at all. The unit is load-bearing; keep it in the name.
//!
//! # Rates are charged smoothly
//!
//! Charging happens every Advance tick, spread evenly via a millicent
//! accumulator ([`MaintenanceAccrual`]) so a rate far below one cent per tick
//! still accrues honestly instead of rounding to nothing. Debiting a whole
//! per-minute rate on every tick would multiply every running cost by the tick
//! rate, which empties a new save in about two minutes — that failure is on
//! record in `docs/BURNDOWN.md`, and the accumulator is what keeps the authored
//! numbers meaning what they say.
//!
//! Speed multipliers scale virtual time, so `FixedUpdate` runs proportionally
//! more often at 2× / 3× and upkeep accrues faster on the wall clock — which is
//! correct, because more of the world's time is passing.
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

use crate::commands::TrainKind;
use crate::money::Money;
use crate::peeps::SIM_SECONDS_PER_TICK;
use crate::stations::StationRegistry;
use crate::track::{piece_maintenance_weight, TrackNetwork};
use crate::trains::{Train, TrainLocation, TrainProfile};

use super::ledger::{MoneyCategory, MoneyLedger};

/// Advance ticks in one **sim**-minute — six, at ten sim-seconds a tick.
///
/// This is the world's own clock: sim-days, peep routines and goal deadlines
/// are counted in it. Money is not. See the module docs.
pub const TICKS_PER_SIM_MINUTE: i64 = 60 / SIM_SECONDS_PER_TICK as i64;

/// Advance ticks in one **real** minute, at the default 64 Hz fixed timestep.
///
/// 640× [`TICKS_PER_SIM_MINUTE`]. Every constant in this module is a rate per
/// one of these.
pub const TICKS_PER_REAL_MINUTE: i64 = 64 * 60;

/// Cents per **real minute** per unit of track maintenance weight.
///
/// [`piece_maintenance_weight`] returns the weight of a piece (ground `1`,
/// bridge `4`); this turns that weight into money — `$10` a minute for a tile
/// of plain track, `$40` for a tile of bridge.
///
/// Sized against what track *earns*, not what it cost to lay. The opening
/// beat — twenty-odd tiles round a corner, two stops ten apart, one transit —
/// grosses about `$1,070` a minute and spends `$210` of it holding its own
/// ground. A straight sixteen-tile line between stops fifteen apart does much
/// better, `$2,520` against the same `$160`, which is the super-linear fare
/// rewarding a better-shaped railway rather than a cheaper one.
///
/// The same rate makes two hundred tiles carrying nothing cost `$2,000` a
/// minute — against a healthy four-stop network's `$829` of margin, more than
/// twice over. That is design 08 §3.1 working: *"track that isn't carrying
/// enough starts costing more than it earns."*
///
/// Do not tune this by eye. `rail_sim/tests/economy_arc.rs` measures both
/// networks against a running sim and will say which way it moved.
pub const MAINT_CENTS_PER_WEIGHT_PER_REAL_MIN: i64 = 1_000;

/// Legacy / average opex constant (alerts). Prefer
/// [`TrainProfile::opex_cents_per_real_min`].
pub const TRAIN_OPEX_CENTS: i64 = 10;

/// Fractional upkeep carried between ticks, in **cent-ticks**.
///
/// Rates are per real minute and far below a cent per tick, so the remainder
/// has to live somewhere or every charge rounds to zero.
#[derive(Debug, Clone, Copy, Default, Resource)]
pub struct MaintenanceAccrual {
    track_cent_ticks: i64,
    train_cent_ticks: i64,
}

/// Take whole cents out of an accumulator, keeping the remainder.
///
/// The carry holds the *numerator* rather than a per-tick quotient. Dividing
/// first would truncate on every tick and quietly under-bill.
fn accrue(carry: &mut i64, per_real_min_cents: i64) -> i64 {
    *carry += per_real_min_cents;
    let due = *carry / TICKS_PER_REAL_MINUTE;
    *carry -= due * TICKS_PER_REAL_MINUTE;
    due
}

/// What a set of trains costs to run, in cents per real minute.
pub fn train_opex_total_cents_per_real_min(kinds: &[TrainKind]) -> i64 {
    kinds
        .iter()
        .map(|k| TrainProfile::for_kind(*k).opex_cents_per_real_min)
        .sum()
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
    let per_real_min: i64 = q
        .iter()
        .map(|(train, _)| TrainProfile::for_kind(train.kind).opex_cents_per_real_min)
        .sum();

    let due = accrue(&mut accrual.train_cent_ticks, per_real_min);
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

/// Track maintenance for the whole network, in **cents per real minute**.
pub fn track_maintenance_total(network: &TrackNetwork) -> i64 {
    network
        .iter()
        .map(|p| piece_maintenance_weight(p.is_bridge()) * MAINT_CENTS_PER_WEIGHT_PER_REAL_MIN)
        .sum()
}

/// Station upkeep the player is actually billed for, in cents per real minute.
///
/// A stop with no railhead under it is not a stop the railway is maintaining —
/// it is a town on the map. The distinction is not pedantic, because two kinds
/// of station arrive without the player building anything:
/// [`seed_stations_and_industries`](crate::stations::seed_stations_and_industries)
/// plants the opening anchors, and
/// [`spawn_new_demand`](crate::demand::spawn_new_demand) plants a new settlement
/// every few minutes for the rest of the session — *unconnected by definition*,
/// since being unconnected is what makes it an opportunity.
///
/// Billing those was a slow, invisible tax on doing nothing: a fresh world
/// opened at `$90`/min of station upkeep for three anchors the player had not
/// reached, and every marker the world put down added `$30`/min more, forever,
/// whether or not it was ever served. Measured over the first fifteen minutes of
/// the opening beat that is `$440`/min rising to `$500`/min with no change to
/// the railway. Design 08 §3.3's liability is *"an interchange nobody uses"* —
/// something the player chose and paid for — not a village the world invented.
///
/// Player-built stops are unaffected: [`try_place_station`] refuses a site with
/// no track under it, so anything the player paid for is always billed.
///
/// [`try_place_station`]: crate::stations::try_place_station
pub fn station_maintenance_billed(network: &TrackNetwork, stations: &StationRegistry) -> i64 {
    stations
        .iter()
        .filter(|s| crate::trains::track_for_station(network, s.tile, s.layer).is_some())
        .map(|s| s.tier.maint_cents_per_real_min())
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
    let per_real_min =
        track_maintenance_total(&network) + station_maintenance_billed(&network, &stations);
    if per_real_min <= 0 {
        return;
    }
    let due = accrue(&mut accrual.track_cent_ticks, per_real_min);
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
    use crate::track::{try_place_track, TrackTerrain, GROUND_LAYER, TRACK_MAINT_WEIGHT};

    #[test]
    fn the_two_minutes_differ_by_the_ratio_the_docs_claim() {
        // The whole module hangs off this. A future edit that "simplifies" one
        // of these into the other reintroduces the 1/640 collection bug.
        assert_eq!(TICKS_PER_SIM_MINUTE, 6);
        assert_eq!(TICKS_PER_REAL_MINUTE, 3_840);
        assert_eq!(TICKS_PER_REAL_MINUTE / TICKS_PER_SIM_MINUTE, 640);
    }

    #[test]
    fn maintenance_ticks_debit_per_tile() {
        let terrain = TrackTerrain::new(4, 4, (0..16).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut money = Money::new(10_000_000);
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
            2 * TRACK_MAINT_WEIGHT * MAINT_CENTS_PER_WEIGHT_PER_REAL_MIN
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
        let per_real_min = 6_000; // $60/min
        let mut carry = 0i64;
        let charged: i64 = (0..TICKS_PER_REAL_MINUTE)
            .map(|_| accrue(&mut carry, per_real_min))
            .sum();
        assert_eq!(
            charged, per_real_min,
            "a real minute of ticks bills one real minute"
        );
    }

    /// The mirror-image bug, and the one this change fixes: dividing by a real
    /// minute while *calling* it a sim-minute collects 1/640 of the rate.
    #[test]
    fn a_sim_minute_of_ticks_bills_a_640th_of_the_rate() {
        let per_real_min = 64_000; // $640/min
        let mut carry = 0i64;
        let charged: i64 = (0..TICKS_PER_SIM_MINUTE)
            .map(|_| accrue(&mut carry, per_real_min))
            .sum();
        assert_eq!(
            charged,
            per_real_min / 640,
            "six ticks is a sim-minute, which is a 640th of the rate — which is \
             exactly why the constants are not named after it"
        );
    }

    #[test]
    fn sub_cent_rates_still_accrue_instead_of_rounding_away() {
        // A rate far below a cent per tick.
        let mut carry = 0i64;
        let charged: i64 = (0..TICKS_PER_REAL_MINUTE)
            .map(|_| accrue(&mut carry, 20))
            .sum();
        assert_eq!(charged, 20);
    }

    #[test]
    fn an_early_network_has_runway_on_the_opening_balance() {
        // 16 tiles, 2 stations, 1 transit train: the first line. It must be
        // survivable for long enough to earn, from a standing start, with no
        // income at all — design 08 §1, money paces expansion rather than
        // ending the game.
        let track = 16 * TRACK_MAINT_WEIGHT * MAINT_CENTS_PER_WEIGHT_PER_REAL_MIN;
        let stations = 2 * crate::stations::StationTier::Station.maint_cents_per_real_min();
        let trains = crate::trains::TRANSIT_PROFILE.opex_cents_per_real_min;
        let per_real_min = track + stations + trains;
        let runway_minutes = crate::money::STARTING_CASH_CENTS / per_real_min;
        assert!(
            (8..=60).contains(&runway_minutes),
            "an early network costs {per_real_min}c/min, which is {runway_minutes} \
             minutes of runway on the opening balance — too short to learn in, or \
             too long to feel"
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
