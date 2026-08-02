//! Advance trains along paths; one train per tile (wait if occupied).

use std::collections::HashMap;

use bevy_ecs::prelude::*;

use crate::ids::{TrackId, TrainId};
use crate::track::TrackNetwork;

use super::train::{Train, TrainLocation};

/// Base ticks to cross a flat straight tile.
const BASE_TICKS: u16 = 4;

/// Occupancy map rebuilt each movement pass (train id on each track tile).
#[derive(Debug, Clone, Default, Resource)]
pub struct TileOccupancy {
    pub by_track: HashMap<TrackId, TrainId>,
}

/// Move trains one step when progress allows and the next tile is free.
pub fn advance_trains(
    network: Res<TrackNetwork>,
    mut occupancy: ResMut<TileOccupancy>,
    mut q: Query<(&Train, &mut TrainLocation)>,
) {
    // Rebuild occupancy from current positions.
    occupancy.by_track.clear();
    for (train, loc) in q.iter() {
        occupancy.by_track.insert(loc.track, train.id);
    }

    // Collect move intents so we don't double-book mid-iteration.
    let mut moves: Vec<(TrainId, TrackId)> = Vec::new();

    for (train, mut loc) in q.iter_mut() {
        if loc.parked || loc.at_destination() {
            continue;
        }
        let Some(piece) = network.piece(loc.track) else {
            continue;
        };
        let needed = ticks_for_piece(piece.max_grade, piece.curve);
        loc.progress = loc.progress.saturating_add(1);
        if loc.progress < needed {
            continue;
        }

        let next_index = loc.path_index + 1;
        let Some(&next_track) = loc.path.get(next_index) else {
            continue;
        };

        // Occupied by another train → wait (keep progress at threshold).
        if let Some(other) = occupancy.by_track.get(&next_track) {
            if *other != train.id {
                loc.progress = needed.saturating_sub(1);
                continue;
            }
        }

        moves.push((train.id, next_track));
        loc.progress = 0;
        // Free current tile in occupancy so a follower can enter next pass.
        occupancy.by_track.remove(&loc.track);
        occupancy.by_track.insert(next_track, train.id);
        loc.track = next_track;
        loc.path_index = next_index;
    }

    let _ = moves;
}

/// Grade / curve slow trains: each unit of grade and each 32 curve adds a tick.
pub fn ticks_for_piece(max_grade: u8, curve: u8) -> u16 {
    let grade = max_grade as u16;
    let turn = (curve as u16) / 32;
    BASE_TICKS.saturating_add(grade).saturating_add(turn).max(1)
}
