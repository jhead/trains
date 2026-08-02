//! New demand outside the served network — the ten-minute rung.
//!
//! On a felt cadence, spawns settlements and industries preferentially where
//! [`crate::stations::StationService`] influence is low/zero, capped so the
//! map does not explode. See `docs/design/08-economy-and-pressure.md` §4.

mod spawn;
mod sites;

pub use spawn::{
    spawn_new_demand, DemandOpportunity, DemandOpportunityKind, DemandSpawner,
    DEMAND_FIRST_DELAY_SIM_MINUTES, DEMAND_INTERVAL_SIM_MINUTES, DEMAND_MAX_NEW_PER_SESSION,
    DEMAND_MIN_ANCHOR_SPACING, DEMAND_SERVICE_INFLUENCE_MAX,
};
pub use sites::service_influence_at;
