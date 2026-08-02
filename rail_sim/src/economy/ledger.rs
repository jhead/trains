//! Categorised money flow for the ledger panel and $/min rate.
//!
//! Soft-fail economics stay on [`Money`](crate::money::Money); this resource only
//! records what already succeeded (or would-be credits).

use std::collections::VecDeque;

use bevy_ecs::prelude::Resource;

use crate::money::{InsufficientFunds, Money};

/// How many recent net samples feed the sparkline / rate.
pub const LEDGER_HISTORY_LEN: usize = 24;
/// Sim seconds per history sample (~half a minute at [`crate::SIM_SECONDS_PER_TICK`]).
pub const LEDGER_SAMPLE_SIM_SECS: u32 = 30;

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
#[derive(Debug, Clone, Resource)]
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
    /// Cached net ¢/min from completed samples.
    net_rate_cents_per_min: i64,
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

    fn recompute_rate(&mut self) {
        if self.history.is_empty() {
            self.net_rate_cents_per_min = 0;
            return;
        }
        let total_net: i64 = self.history.iter().sum();
        let total_secs = (self.history.len() as i64) * i64::from(LEDGER_SAMPLE_SIM_SECS);
        if total_secs <= 0 {
            self.net_rate_cents_per_min = 0;
            return;
        }
        self.net_rate_cents_per_min = (total_net.saturating_mul(60)) / total_secs;
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
        // Net +2400¢ over 30s → +4800¢/min
        assert_eq!(ledger.net_rate_cents_per_min(), 4_800);
        assert_eq!(ledger.history_len(), 1);
        assert_eq!(ledger.recent(MoneyCategory::Fares), 3_000);
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
