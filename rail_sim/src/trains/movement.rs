//! Advance trains along paths; one train per tile (wait if occupied).
//!
//! Congestion: when blocked, [`TileOccupancy::blocked_by`] records the blocker
//! for the inspector. Opposite-direction passing on single track is deferred
//! (needs passing loops / double track — see Phase C gaps).

use std::collections::HashMap;

use bevy_ecs::prelude::*;

use crate::ids::{TrackId, TrainId};
use crate::track::TrackNetwork;

use super::profile::TrainProfile;
use super::train::{Train, TrainLocation};

/// Occupancy map rebuilt each movement pass (train id on each track tile).
#[derive(Debug, Clone, Default, Resource)]
pub struct TileOccupancy {
    pub by_track: HashMap<TrackId, TrainId>,
    /// Train waiting on next tile → id of the train occupying that tile.
    pub blocked_by: HashMap<TrainId, TrainId>,
}

/// Move trains one step when progress allows and the next tile is free.
pub fn advance_trains(
    network: Res<TrackNetwork>,
    mut occupancy: ResMut<TileOccupancy>,
    mut q: Query<(&Train, &mut TrainLocation)>,
) {
    // Rebuild occupancy from current positions.
    occupancy.by_track.clear();
    occupancy.blocked_by.clear();
    for (train, loc) in q.iter() {
        occupancy.by_track.insert(loc.track, train.id);
    }

    for (train, mut loc) in q.iter_mut() {
        if loc.parked {
            continue;
        }
        // Dwell at stop: count down, don't move.
        if loc.dwell_remaining > 0 {
            loc.dwell_remaining = loc.dwell_remaining.saturating_sub(1);
            continue;
        }
        if loc.at_destination() {
            continue;
        }
        let Some(piece) = network.piece(loc.track) else {
            continue;
        };
        let profile = TrainProfile::for_kind(train.kind);
        let needed = profile.ticks_for_piece(piece.max_grade, piece.curve);
        loc.progress = loc.progress.saturating_add(1);
        if loc.progress < needed {
            continue;
        }

        let next_index = loc.path_index + 1;
        let Some(&next_track) = loc.path.get(next_index) else {
            continue;
        };

        // Freight (etc.) refuse tiles steeper than profile max.
        if let Some(next_piece) = network.piece(next_track) {
            if !profile.tolerates_grade(next_piece.max_grade) {
                loc.progress = needed.saturating_sub(1);
                continue;
            }
        }

        // Occupied by another train → wait and record blocker.
        if let Some(&other) = occupancy.by_track.get(&next_track) {
            if other != train.id {
                loc.progress = needed.saturating_sub(1);
                occupancy.blocked_by.insert(train.id, other);
                continue;
            }
        }

        loc.progress = 0;
        occupancy.by_track.remove(&loc.track);
        occupancy.by_track.insert(next_track, train.id);
        loc.track = next_track;
        loc.path_index = next_index;
    }
}

/// Grade / curve slow trains using the kind's [`TrainProfile`].
pub fn ticks_for_piece(kind: crate::commands::TrainKind, max_grade: u8, curve: u8) -> u16 {
    TrainProfile::for_kind(kind).ticks_for_piece(max_grade, curve)
}

/// Blocker id for a waiting train, if any.
pub fn blocker_for(occupancy: &TileOccupancy, train: TrainId) -> Option<TrainId> {
    occupancy.blocked_by.get(&train).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::TrainKind;

    #[test]
    fn profile_tick_helper_matches() {
        assert_eq!(ticks_for_piece(TrainKind::Transit, 0, 0), 3);
        assert_eq!(ticks_for_piece(TrainKind::Transport, 0, 0), 5);
        assert!(ticks_for_piece(TrainKind::Transport, 1, 0) > ticks_for_piece(TrainKind::Transit, 1, 0));
    }
}
