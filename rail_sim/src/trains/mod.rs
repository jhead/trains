//! Train entities, buy/place commands, path follow, and occupancy.

mod apply;
mod movement;
mod path;
mod train;

pub use apply::{apply_train_commands, track_for_station, TrainEdit};
pub use movement::{advance_trains, ticks_for_piece, TileOccupancy};
pub use path::find_path;
pub use train::{
    buy_cost, Train, TrainCargo, TrainLocation, TrainYard, TRANSPORT_COST_CENTS,
    TRANSIT_COST_CENTS,
};
