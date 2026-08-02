//! Station↔station (and track↔track) pathfinding on [`TrackNetwork`].

use std::collections::{HashMap, VecDeque};

use crate::ids::TrackId;
use crate::track::TrackNetwork;

/// BFS shortest path over [`TrackNetwork::neighbor_ids`].
///
/// Returns a path including both `from` and `to`. Empty / single-node when
/// `from == to`. `None` when disconnected or either id is missing.
pub fn find_path(network: &TrackNetwork, from: TrackId, to: TrackId) -> Option<Vec<TrackId>> {
    if network.piece(from).is_none() || network.piece(to).is_none() {
        return None;
    }
    if from == to {
        return Some(vec![from]);
    }

    let mut prev: HashMap<TrackId, TrackId> = HashMap::new();
    let mut queue = VecDeque::new();
    queue.push_back(from);
    prev.insert(from, from);

    while let Some(cur) = queue.pop_front() {
        for next in network.neighbor_ids(cur) {
            if prev.contains_key(&next) {
                continue;
            }
            prev.insert(next, cur);
            if next == to {
                return Some(reconstruct(&prev, from, to));
            }
            queue.push_back(next);
        }
    }
    None
}

fn reconstruct(prev: &HashMap<TrackId, TrackId>, from: TrackId, to: TrackId) -> Vec<TrackId> {
    let mut path = Vec::new();
    let mut cur = to;
    loop {
        path.push(cur);
        if cur == from {
            break;
        }
        cur = prev[&cur];
    }
    path.reverse();
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::TileCoord;
    use crate::money::Money;
    use crate::track::{try_place_track, TrackTerrain, GROUND_LAYER};

    fn land(w: u32, h: u32) -> TrackTerrain {
        TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 0i8)))
    }

    #[test]
    fn bfs_finds_shortest_path_along_line() {
        let terrain = land(8, 8);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(500_000);
        let mut ids = Vec::new();
        for x in 1..=5 {
            let p = try_place_track(
                &mut network,
                &mut money,
                &terrain,
                TileCoord { x, y: 2 },
                GROUND_LAYER,
            )
            .unwrap();
            ids.push(p.id);
        }

        let path = find_path(&network, ids[0], ids[4]).expect("connected");
        assert_eq!(path, ids);

        // Branch: add a detour that shouldn't be preferred.
        let _ = try_place_track(
            &mut network,
            &mut money,
            &terrain,
            TileCoord { x: 2, y: 3 },
            GROUND_LAYER,
        );
        let path2 = find_path(&network, ids[0], ids[4]).expect("still connected");
        assert_eq!(path2.len(), 5);
    }

    #[test]
    fn bfs_returns_none_when_disconnected() {
        let terrain = land(8, 8);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(500_000);
        let a = try_place_track(
            &mut network,
            &mut money,
            &terrain,
            TileCoord { x: 1, y: 1 },
            GROUND_LAYER,
        )
        .unwrap();
        let b = try_place_track(
            &mut network,
            &mut money,
            &terrain,
            TileCoord { x: 5, y: 5 },
            GROUND_LAYER,
        )
        .unwrap();
        assert!(find_path(&network, a.id, b.id).is_none());
    }
}
