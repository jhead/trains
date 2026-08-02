//! Demand jobs, delivery payouts, and per-train operating costs.

mod jobs;
mod opex;
mod payout;

pub use jobs::{assign_jobs, spawn_demand_jobs, JobBoard, JobKind};
pub use opex::{apply_train_opex, TRAIN_OPEX_CENTS};
pub use payout::{
    resolve_deliveries, GOODS_DELIVERY_CENTS, PASSENGER_FARE_CENTS,
};
