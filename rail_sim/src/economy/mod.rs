//! Demand jobs, delivery payouts, operating costs, ledger, and alerts.

mod alerts;
mod jobs;
mod ledger;
/// Running costs, and the two minutes this crate keeps.
pub mod opex;
mod payout;

pub use alerts::{
    refresh_alerts, Alert, AlertBoard, AlertFocus, AlertKind, AlertKey, GridlockWatch,
    ALERT_CASH_LOW_MINUTES, ALERT_SERVICE_LOW_SCORE, ALERT_WAITING_OVERWHELMED,
};
pub use jobs::{
    assign_jobs, drain_peep_demand, spawn_demand_jobs, sync_peep_platform_pressure, Job, JobBoard,
    JobKind,
};
pub use ledger::{
    tick_money_ledger, MoneyCategory, MoneyLedger, LEDGER_HISTORY_LEN, LEDGER_RATE_SAMPLES,
    LEDGER_SAMPLE_SIM_SECS,
};
pub use opex::{
    apply_track_maintenance, apply_train_opex, track_maintenance_total,
    train_opex_total_cents_per_real_min, MaintenanceAccrual,
    MAINT_CENTS_PER_WEIGHT_PER_REAL_MIN, TICKS_PER_REAL_MINUTE, TICKS_PER_SIM_MINUTE,
    TRAIN_OPEX_CENTS,
};
pub use payout::{
    goods_delivery_cents, haul_tiles, passenger_fare_cents, resolve_deliveries,
    GOODS_DELIVERY_CENTS, GOODS_DELIVERY_CENTS_PER_TILE, GOODS_DELIVERY_DISTANCE_DIVISOR,
    PASSENGER_FARE_CENTS, PASSENGER_FARE_CENTS_PER_TILE, PASSENGER_FARE_DISTANCE_DIVISOR,
};
