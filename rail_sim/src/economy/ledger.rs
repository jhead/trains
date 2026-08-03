//! Categorised money flow for the ledger panel and $/min rate.
//!
//! Soft-fail economics stay on [`Money`](crate::money::Money); this resource only
//! records what already succeeded (or would-be credits).

use std::collections::VecDeque;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::money::{InsufficientFunds, Money};

/// How many recent net samples the sparkline holds — one per real minute, so
/// the trend covers **the last 24 real minutes** of a session.
///
/// It used to cover 1.1 real seconds. A sample was 30 *sim*-seconds, which is
/// three ticks, so twenty-four of them were seventy-two ticks — a trend line
/// that answered "what happened in the last second?" while design 08 §6 asks
/// for "a history graph long enough to show a trend across a session".
pub const LEDGER_HISTORY_LEN: usize = 24;

/// Sim seconds per history sample: **one real minute**.
///
/// A tick advances the world [`SIM_SECONDS_PER_TICK`](crate::peeps::SIM_SECONDS_PER_TICK)
/// = 10 sim-seconds and `FixedUpdate` runs at 64 Hz, so a real minute is 3,840
/// ticks and 38,400 sim-seconds. Stated in sim-seconds because that is the unit
/// [`MoneyLedger::on_sim_secs`] is fed in; stated as a real minute in the doc
/// because that is the minute the player is sitting in and the one the `$/min`
/// readout claims to be reporting.
pub const LEDGER_SAMPLE_SIM_SECS: u32 =
    (crate::economy::opex::TICKS_PER_REAL_MINUTE as u32) * crate::peeps::SIM_SECONDS_PER_TICK;

/// Completed samples averaged into the headline `$/min` rate.
///
/// Three real minutes: long enough that one train purchase does not read as a
/// collapse, short enough that design 08 §9.2 holds — *"pruning an unprofitable
/// branch visibly restores the rate, within a minute"* — because the worst
/// minute leaves the average as soon as a better one arrives.
pub const LEDGER_RATE_SAMPLES: usize = 3;

/// Income / expense buckets shown in the ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MoneyCategory {
    /// Passenger fares.
    Fares,
    /// Goods delivery payouts.
    Deliveries,
    /// Track (and future station) construction; demolish refunds credit here.
    Construction,
    /// Rolling stock purchase.
    RollingStock,
    /// Per-train operating cost.
    TrainOpex,
    /// Per-tile track maintenance (bridges cost more).
    TrackMaintenance,
}

impl MoneyCategory {
    pub const ALL: [MoneyCategory; 6] = [
        Self::Fares,
        Self::Deliveries,
        Self::Construction,
        Self::RollingStock,
        Self::TrainOpex,
        Self::TrackMaintenance,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Fares => "Fares",
            Self::Deliveries => "Deliveries",
            Self::Construction => "Construction",
            Self::RollingStock => "Rolling stock",
            Self::TrainOpex => "Train opex",
            Self::TrackMaintenance => "Track maint.",
        }
    }

    pub fn is_income(self) -> bool {
        matches!(self, Self::Fares | Self::Deliveries)
    }

    fn index(self) -> usize {
        match self {
            Self::Fares => 0,
            Self::Deliveries => 1,
            Self::Construction => 2,
            Self::RollingStock => 3,
            Self::TrainOpex => 4,
            Self::TrackMaintenance => 5,
        }
    }
}

const LEDGER_CATS: usize = 6;

/// Session + recent-window category accounting.
#[derive(Debug, Clone, PartialEq, Resource, Serialize, Deserialize)]
pub struct MoneyLedger {
    /// Signed cents for the whole session (credits positive, debits negative).
    totals: [i64; LEDGER_CATS],
    /// Signed cents in the open sample window.
    window: [i64; LEDGER_CATS],
    /// Last completed window (for “recent” panel rows).
    last_window: [i64; LEDGER_CATS],
    /// Net cents per completed sample (oldest → newest).
    history: VecDeque<i64>,
    window_sim_secs: u32,
    /// Cached net ¢ per **real** minute from completed samples.
    net_rate_cents_per_min: i64,
    /// Paid runs completed this session — fares plus goods deliveries.
    ///
    /// Counted rather than derived. It used to be recovered by dividing the
    /// fare total by the fare price, which worked only while every run paid the
    /// same; now that a payout scales with the distance carried, one long haul
    /// would have read as a dozen runs and any delivery quota would have fallen
    /// over on its own arithmetic.
    paid_runs: u64,
}

impl Default for MoneyLedger {
    fn default() -> Self {
        Self {
            totals: [0; LEDGER_CATS],
            window: [0; LEDGER_CATS],
            last_window: [0; LEDGER_CATS],
            history: VecDeque::with_capacity(LEDGER_HISTORY_LEN),
            window_sim_secs: 0,
            net_rate_cents_per_min: 0,
            paid_runs: 0,
        }
    }
}

impl MoneyLedger {
    pub fn record(&mut self, category: MoneyCategory, signed_cents: i64) {
        if signed_cents == 0 {
            return;
        }
        let i = category.index();
        self.totals[i] = self.totals[i].saturating_add(signed_cents);
        self.window[i] = self.window[i].saturating_add(signed_cents);
    }

    /// Credit balance and ledger together.
    pub fn credit(&mut self, money: &mut Money, category: MoneyCategory, amount: i64) {
        if amount <= 0 {
            return;
        }
        money.credit(amount);
        self.record(category, amount);
    }

    /// Credit a completed delivery — one paid run, whatever it was worth.
    pub fn credit_paid_run(
        &mut self,
        money: &mut Money,
        category: MoneyCategory,
        amount: i64,
    ) {
        self.credit(money, category, amount);
        self.paid_runs = self.paid_runs.saturating_add(1);
    }

    /// Paid runs completed this session — fares plus goods deliveries.
    pub fn paid_runs(&self) -> u64 {
        self.paid_runs
    }

    /// Debit balance and ledger together (soft-fail on funds).
    pub fn try_debit(
        &mut self,
        money: &mut Money,
        category: MoneyCategory,
        amount: i64,
    ) -> Result<(), InsufficientFunds> {
        money.try_debit(amount)?;
        if amount > 0 {
            self.record(category, -amount);
        }
        Ok(())
    }

    /// Session total for a category (signed).
    pub fn total(&self, category: MoneyCategory) -> i64 {
        self.totals[category.index()]
    }

    /// Last completed sample window for a category (signed), else current open window.
    pub fn recent(&self, category: MoneyCategory) -> i64 {
        let last = self.last_window[category.index()];
        if self.history.is_empty() {
            self.window[category.index()]
        } else {
            last
        }
    }

    /// Sum of session income categories.
    pub fn session_income(&self) -> i64 {
        MoneyCategory::ALL
            .iter()
            .filter(|c| c.is_income())
            .map(|c| self.total(*c).max(0))
            .sum()
    }

    /// Sum of session expense magnitudes (positive number).
    pub fn session_expense(&self) -> i64 {
        MoneyCategory::ALL
            .iter()
            .filter(|c| !c.is_income())
            .map(|c| (-self.total(*c)).max(0))
            .sum()
    }

    pub fn net_rate_cents_per_min(&self) -> i64 {
        self.net_rate_cents_per_min
    }

    /// Net cents per completed sample, oldest first (for sparkline).
    pub fn history_nets(&self) -> impl Iterator<Item = i64> + '_ {
        self.history.iter().copied()
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Advance the sample clock by `sim_secs` and close a window when due.
    pub fn on_sim_secs(&mut self, sim_secs: u32) {
        if sim_secs == 0 {
            return;
        }
        self.window_sim_secs = self.window_sim_secs.saturating_add(sim_secs);
        while self.window_sim_secs >= LEDGER_SAMPLE_SIM_SECS {
            self.close_window();
            self.window_sim_secs -= LEDGER_SAMPLE_SIM_SECS;
        }
    }

    fn close_window(&mut self) {
        let net: i64 = self.window.iter().sum();
        self.last_window = self.window;
        self.window = [0; LEDGER_CATS];
        self.history.push_back(net);
        while self.history.len() > LEDGER_HISTORY_LEN {
            self.history.pop_front();
        }
        self.recompute_rate();
    }

    /// Average net over the last [`LEDGER_RATE_SAMPLES`] completed minutes.
    ///
    /// One sample *is* one real minute, so the average of the recent ones is
    /// already cents per real minute — no scaling, and nothing to get the units
    /// wrong in. Averaging only the recent few is what keeps the readout
    /// responsive: a session-long mean would take twenty-four minutes to admit
    /// that a branch had been pruned.
    fn recompute_rate(&mut self) {
        let recent: Vec<i64> = self
            .history
            .iter()
            .rev()
            .take(LEDGER_RATE_SAMPLES)
            .copied()
            .collect();
        if recent.is_empty() {
            self.net_rate_cents_per_min = 0;
            return;
        }
        let total: i64 = recent.iter().sum();
        self.net_rate_cents_per_min = total / recent.len() as i64;
    }
}

/// Advance ledger sample windows from sim ticks.
pub fn tick_money_ledger(mut ledger: bevy_ecs::prelude::ResMut<MoneyLedger>) {
    ledger.on_sim_secs(crate::peeps::SIM_SECONDS_PER_TICK);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_debit_and_credit_account() {
        let mut money = Money::new(10_000);
        let mut ledger = MoneyLedger::default();

        ledger
            .try_debit(&mut money, MoneyCategory::Construction, 1_500)
            .unwrap();
        assert_eq!(money.cents(), 8_500);
        assert_eq!(ledger.total(MoneyCategory::Construction), -1_500);

        ledger.credit(&mut money, MoneyCategory::Fares, 500);
        assert_eq!(money.cents(), 9_000);
        assert_eq!(ledger.total(MoneyCategory::Fares), 500);

        ledger.credit(&mut money, MoneyCategory::Construction, 1_500); // refund
        assert_eq!(ledger.total(MoneyCategory::Construction), 0);
        assert_eq!(ledger.session_income(), 500);
        assert_eq!(ledger.session_expense(), 0);
    }

    #[test]
    fn failed_debit_does_not_record() {
        let mut money = Money::new(100);
        let mut ledger = MoneyLedger::default();
        assert!(ledger
            .try_debit(&mut money, MoneyCategory::TrainOpex, 200)
            .is_err());
        assert_eq!(ledger.total(MoneyCategory::TrainOpex), 0);
        assert_eq!(money.cents(), 100);
    }

    #[test]
    fn sample_windows_drive_net_rate() {
        let mut ledger = MoneyLedger::default();
        ledger.record(MoneyCategory::Fares, 3_000); // +$30
        ledger.record(MoneyCategory::TrainOpex, -600); // -$6
        ledger.on_sim_secs(LEDGER_SAMPLE_SIM_SECS);
        // One sample is one real minute, so its net *is* the ¢/min rate.
        assert_eq!(ledger.net_rate_cents_per_min(), 2_400);
        assert_eq!(ledger.history_len(), 1);
        assert_eq!(ledger.recent(MoneyCategory::Fares), 3_000);
    }

    /// Design 08 §6 wants a trend "long enough to show a trend across a
    /// session", and the strip's `$/min` has to mean the minute the player is
    /// living in.
    #[test]
    fn one_sample_is_one_real_minute_and_the_trend_spans_the_session() {
        let ticks_per_sample =
            LEDGER_SAMPLE_SIM_SECS / crate::peeps::SIM_SECONDS_PER_TICK;
        assert_eq!(ticks_per_sample, 3_840, "a sample is a real minute of ticks");
        assert_eq!(
            LEDGER_HISTORY_LEN as u32 * ticks_per_sample / 3_840,
            24,
            "the sparkline covers 24 real minutes"
        );
    }

    /// A bad minute must not haunt the readout, or pruning never looks like it
    /// worked (design 08 §9.2).
    #[test]
    fn the_rate_recovers_within_a_few_minutes_of_the_network_recovering() {
        let mut ledger = MoneyLedger::default();
        for _ in 0..8 {
            ledger.record(MoneyCategory::TrackMaintenance, -10_000);
            ledger.on_sim_secs(LEDGER_SAMPLE_SIM_SECS);
        }
        assert!(ledger.net_rate_cents_per_min() < 0, "eight bad minutes");

        for _ in 0..LEDGER_RATE_SAMPLES {
            ledger.record(MoneyCategory::Fares, 10_000);
            ledger.on_sim_secs(LEDGER_SAMPLE_SIM_SECS);
        }
        assert_eq!(
            ledger.net_rate_cents_per_min(),
            10_000,
            "three good minutes must fully clear eight bad ones"
        );
        // And the trend still remembers the bad stretch.
        assert!(ledger.history_nets().any(|n| n < 0));
    }

    #[test]
    fn paid_runs_counts_deliveries_not_dollars() {
        let mut money = Money::new(0);
        let mut ledger = MoneyLedger::default();
        ledger.credit_paid_run(&mut money, MoneyCategory::Fares, 660);
        ledger.credit_paid_run(&mut money, MoneyCategory::Fares, 22_500);
        ledger.credit_paid_run(&mut money, MoneyCategory::Deliveries, 96_000);
        assert_eq!(
            ledger.paid_runs(),
            3,
            "a long haul is one run, however much it paid"
        );
        assert_eq!(money.cents(), 660 + 22_500 + 96_000);
        // Plain credits are not runs.
        ledger.credit(&mut money, MoneyCategory::Construction, 500);
        assert_eq!(ledger.paid_runs(), 3);
    }

    #[test]
    fn income_and_expense_split() {
        let mut ledger = MoneyLedger::default();
        ledger.record(MoneyCategory::Fares, 100);
        ledger.record(MoneyCategory::Deliveries, 200);
        ledger.record(MoneyCategory::RollingStock, -50_000);
        ledger.record(MoneyCategory::TrainOpex, -10);
        assert_eq!(ledger.session_income(), 300);
        assert_eq!(ledger.session_expense(), 50_010);
    }
}
