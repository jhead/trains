//! Train entities, buy/place commands, path follow, and occupancy.

mod apply;
mod movement;
mod path;
mod profile;
mod train;

pub use apply::{apply_train_commands, track_for_station, TrainEdit};
pub use movement::{advance_trains, blocker_for, ticks_for_piece, TileOccupancy};
pub use path::{find_path, find_path_for_kind};
pub use profile::{TrainProfile, TRANSIT_PROFILE, TRANSPORT_PROFILE};
pub use train::{
    buy_cost, Train, TrainCargo, TrainLocation, TrainOnLine, TrainYard, TRANSPORT_COST_CENTS,
    TRANSIT_COST_CENTS,
};
