//! Demand jobs, delivery payouts, operating costs, ledger, and alerts.

mod alerts;
mod jobs;
mod ledger;
mod opex;
mod payout;

pub use alerts::{
    refresh_alerts, Alert, AlertBoard, AlertFocus, AlertKind, AlertKey, ALERT_CASH_LOW_MINUTES,
    ALERT_SERVICE_LOW_SCORE, ALERT_WAITING_OVERWHELMED,
};
pub use jobs::{assign_jobs, spawn_demand_jobs, JobBoard, JobKind};
pub use ledger::{
    tick_money_ledger, MoneyCategory, MoneyLedger, LEDGER_HISTORY_LEN, LEDGER_SAMPLE_SIM_SECS,
};
pub use opex::{apply_train_opex, TRAIN_OPEX_CENTS};
pub use payout::{
    resolve_deliveries, GOODS_DELIVERY_CENTS, PASSENGER_FARE_CENTS,
};
