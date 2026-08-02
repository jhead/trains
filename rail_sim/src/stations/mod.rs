//! Named stations and industries (demand anchors).
//!
//! **MVP placement:** stations and industries are **auto-seeded** at map start
//! on land tiles spaced across the map (see [`seed::seed_stations_and_industries`]).
//! There is no click-to-place station tool in this slice — connect them with track.

mod industry;
mod registry;
mod seed;
mod service;

pub use industry::{GoodKind, Industry, IndustryId, IndustryRegistry};
pub use registry::{Station, StationRegistry};
pub use seed::seed_stations_and_industries;
pub use service::{StationService, StationServiceScore};
