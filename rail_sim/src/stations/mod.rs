//! Named stations and industries (demand anchors).
//!
//! **Stations are built, not given** ([04 — Building & Tools] §6): a station is
//! a kind of track, placed with the track tools on a piece of line as a
//! platform. [`place`] holds the rules, [`tier`] holds the four grades, and
//! [`apply`] drains the command buffer exactly as `track::apply` does.
//!
//! Industries and the opening anchors are still **auto-seeded** at map start
//! (see [`seed::seed_stations_and_industries`]) — they are the world the player
//! reaches toward, not the platforms the player owns.
//!
//! # Wiring
//! [`apply_station_commands`] must run after `apply_commands` and **before**
//! `track::apply_track_commands`, which owns `CommandHistory::finish_replay`.
//! [`StationCommand::from_kind`] / [`StationCommand::into_kind`] are the only
//! two functions that name the station variants of `CommandKind`.

mod apply;
mod industry;
mod place;
mod registry;
mod seed;
mod service;
mod tier;

pub use apply::{
    apply_station_command, apply_station_commands, line_using, push_station_command,
    StationCommand, StationEdit,
};
pub use industry::{GoodKind, Industry, IndustryId, IndustryRegistry};
pub use place::{
    best_platform_run, platform_runs, suggest_station_name, try_demolish_station,
    try_place_station, try_upgrade_station, validate_station_site, DemolishStation, PlaceStation,
    PlacedStation, PlatformRun, RetieredStation, StationPlacementError, UpgradeStation,
};
pub use registry::{Station, StationRegistry};
pub use seed::{seed_stations_and_industries, seed_stations_and_industries_at, AnchorSites};
pub use service::{StationService, StationServiceScore};
pub use tier::{
    catchment_influence, max_catchment, station_maintenance_total, StationTier, StationTierSpec,
    HALT_COST_CENTS, HALT_SPEC, INTERCHANGE_COST_CENTS, INTERCHANGE_SPEC, MIN_STATION_SPACING,
    STATION_COST_CENTS, STATION_SPEC, TERMINUS_COST_CENTS, TERMINUS_SPEC,
};
