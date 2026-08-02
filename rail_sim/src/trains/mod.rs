//! Train entities, buy/place commands, path follow, and occupancy.

mod apply;
pub mod congestion;
mod movement;
mod path;
mod profile;
mod train;

pub use apply::{apply_train_commands, track_for_station, TrainEdit};
pub use congestion::{
    blocked_chain_head, passing_tile, tiles_taken_by_others, way_round, TrainIntent, Way,
    REROUTE_AFTER_TICKS, REROUTE_LONG_AFTER_TICKS, REROUTE_NEAR_EXTRA, YIELD_AFTER_TICKS,
    YIELD_COOLDOWN_TICKS,
};
pub use movement::{
    advance_trains, blocker_for, ticks_for_piece, TileOccupancy, POLISH_MEMORY_TICKS,
};
pub use path::{find_path, find_path_avoiding, find_path_for_kind};
pub use profile::{TrainProfile, TRANSIT_PROFILE, TRANSPORT_PROFILE};
pub use train::{
    buy_cost, Train, TrainCargo, TrainLocation, TrainOnLine, TrainYard, TRANSPORT_COST_CENTS,
    TRANSIT_COST_CENTS,
};
