//! New demand outside the served network — the ten-minute rung.
//!
//! On a felt cadence, spawns settlements and industries preferentially where
//! [`crate::stations::StationService`] influence is low or zero, further out
//! each time, and **never stops**. See `docs/design/08-economy-and-pressure.md`
//! §4: *"a player who has connected everything available has finished, and
//! there is no hour-long arc."* What is bounded is how many unanswered markers
//! stand on the board at once, not how many the session may produce.

mod spawn;
mod sites;

pub use spawn::{
    spawn_new_demand, DemandOpportunity, DemandOpportunityKind, DemandSpawner,
    DEMAND_FIRST_DELAY_SIM_MINUTES, DEMAND_INTERVAL_GROWTH_PERCENT,
    DEMAND_INTERVAL_MAX_PERCENT, DEMAND_INTERVAL_SIM_MINUTES, DEMAND_MAX_PENDING,
    DEMAND_MIN_ANCHOR_SPACING, DEMAND_MIN_SPACING_MAX, DEMAND_SERVICE_INFLUENCE_MAX,
    DEMAND_SPACING_GROWTH,
};
pub use sites::{min_anchor_distance, service_influence_at};
