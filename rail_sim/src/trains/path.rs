//! Station↔station (and track↔track) pathfinding on [`TrackNetwork`].

use std::collections::{HashMap, HashSet, VecDeque};

use crate::commands::TrainKind;
use crate::ids::TrackId;
use crate::track::TrackNetwork;
use super::profile::TrainProfile;

/// BFS shortest path over [`TrackNetwork::neighbor_ids`].
///
/// Returns a path including both `from` and `to`. Empty / single-node when
/// `from == to`. `None` when disconnected or either id is missing.
pub fn find_path(network: &TrackNetwork, from: TrackId, to: TrackId) -> Option<Vec<TrackId>> {
    find_path_for(network, from, to, None, &|_| false)
}

/// Pathfinding that refuses tiles steeper than the train's grade tolerance.
pub fn find_path_for_kind(
    network: &TrackNetwork,
    from: TrackId,
    to: TrackId,
    kind: TrainKind,
) -> Option<Vec<TrackId>> {
    find_path_for(network, from, to, Some(TrainProfile::for_kind(kind)), &|_| false)
}

/// Pathfinding that also treats `avoid` as impassable — the way round a block.
///
/// `from` and `to` are always allowed, so a train standing nose-to-tail in a
/// queue can still route out of a busy tile and into a station that is taken.
/// This is what turns a passing loop or a second running line into a usable
/// alternative rather than decoration (see `docs/design/07-trains-and-lines.md`
/// §4.3 and §4.4).
pub fn find_path_avoiding(
    network: &TrackNetwork,
    from: TrackId,
    to: TrackId,
    kind: TrainKind,
    avoid: &HashSet<TrackId>,
) -> Option<Vec<TrackId>> {
    find_path_for(
        network,
        from,
        to,
        Some(TrainProfile::for_kind(kind)),
        &|id| id != from && id != to && avoid.contains(&id),
    )
}

fn find_path_for(
    network: &TrackNetwork,
    from: TrackId,
    to: TrackId,
    profile: Option<TrainProfile>,
    blocked: &dyn Fn(TrackId) -> bool,
) -> Option<Vec<TrackId>> {
    if network.piece(from).is_none() || network.piece(to).is_none() {
        return None;
    }
    if let Some(p) = profile {
        let from_g = network.piece(from).map(|x| x.max_grade).unwrap_or(0);
        let to_g = network.piece(to).map(|x| x.max_grade).unwrap_or(0);
        if !p.tolerates_grade(from_g) || !p.tolerates_grade(to_g) {
            return None;
        }
    }
    if from == to {
        return Some(vec![from]);
    }

    let mut prev: HashMap<TrackId, TrackId> = HashMap::new();
    let mut queue = VecDeque::new();
    queue.push_back(from);
    prev.insert(from, from);

    while let Some(cur) = queue.pop_front() {
        // Sorted so the chosen route is stable regardless of hash order.
        let mut neighbors = network.neighbor_ids(cur);
        neighbors.sort_unstable_by_key(|id| id.0);
        for next in neighbors {
            if prev.contains_key(&next) {
                continue;
            }
            if blocked(next) {
                continue;
            }
            if let Some(p) = profile {
                let g = network.piece(next).map(|x| x.max_grade).unwrap_or(0);
                if !p.tolerates_grade(g) {
                    continue;
                }
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
    use crate::economy::MoneyLedger;
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
        let mut ledger = MoneyLedger::default();
        let mut ids = Vec::new();
        for x in 1..=5 {
            let p = try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
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
            &mut ledger,
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
        let mut ledger = MoneyLedger::default();
        let a = try_place_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            TileCoord { x: 1, y: 1 },
            GROUND_LAYER,
        )
        .unwrap();
        let b = try_place_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            TileCoord { x: 5, y: 5 },
            GROUND_LAYER,
        )
        .unwrap();
        assert!(find_path(&network, a.id, b.id).is_none());
    }

    #[test]
    fn freight_path_refuses_steep_tiles() {
        let terrain = land(8, 8);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(500_000);
        let mut ledger = MoneyLedger::default();
        let mut ids = Vec::new();
        for x in 1..=4 {
            let p = try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                TileCoord { x, y: 2 },
                GROUND_LAYER,
            )
            .unwrap();
            ids.push(p.id);
        }
        // Force a mid tile to grade 3 (above transport max_grade 1).
        if let Some(piece) = network.piece_mut(ids[1]) {
            piece.max_grade = 3;
        }
        assert!(find_path_for_kind(&network, ids[0], ids[3], TrainKind::Transit).is_some());
        assert!(find_path_for_kind(&network, ids[0], ids[3], TrainKind::Transport).is_none());
    }
}
