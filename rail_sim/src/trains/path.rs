//! Station↔station (and track↔track) pathfinding on [`TrackNetwork`].

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use crate::commands::TrainKind;
use crate::ids::TrackId;
use crate::track::TrackNetwork;
use super::profile::TrainProfile;

/// Shortest path over [`TrackNetwork::neighbor_ids`], by distance.
///
/// Returns a path including both `from` and `to`. Empty / single-node when
/// `from == to`. `None` when disconnected or either id is missing.
pub fn find_path(network: &TrackNetwork, from: TrackId, to: TrackId) -> Option<Vec<TrackId>> {
    find_path_for(network, from, to, None, &|_| false)
}

/// Pathfinding that refuses tiles steeper than the train's grade tolerance
/// **and weighs legs by that train's own running time** — grade drag, curve
/// drag, leg length — so the route it picks is the *fastest* one for the
/// train, not the fewest-hops one. A longer flat alignment genuinely beats a
/// short steep curvy one, which is the third leg of "shortest, cheapest and
/// fastest are three different routes" (design 02 §3).
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

/// Dijkstra over the piece graph.
///
/// With a profile, an edge costs that train's [`TrainProfile::ticks_for_leg`]
/// into the next piece — its own time, grade and curve drag included. Without
/// one, an edge costs the leg's length in integer eighths (√1/√2/√5 → 8/11/18),
/// which is plain "shortest". Ties break on ascending [`TrackId`], so the
/// chosen route is stable regardless of hash order.
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
    let mut best: HashMap<TrackId, u32> = HashMap::new();
    let mut heap: BinaryHeap<Reverse<(u32, u64)>> = BinaryHeap::new();
    best.insert(from, 0);
    heap.push(Reverse((0, from.0)));

    while let Some(Reverse((cost, cur_raw))) = heap.pop() {
        let cur = TrackId(cur_raw);
        if best.get(&cur).is_some_and(|&b| cost > b) {
            continue;
        }
        if cur == to {
            return Some(reconstruct(&prev, from, to));
        }
        let cur_tile = network.piece(cur).map(|p| p.tile)?;

        let mut neighbors = network.neighbor_ids(cur);
        neighbors.sort_unstable_by_key(|id| id.0);
        for next in neighbors {
            if blocked(next) {
                continue;
            }
            let Some(piece) = network.piece(next) else {
                continue;
            };
            if let Some(p) = profile {
                if !p.tolerates_grade(piece.max_grade) {
                    continue;
                }
            }
            let dx = piece.tile.x - cur_tile.x;
            let dy = piece.tile.y - cur_tile.y;
            let length_sq = (dx * dx + dy * dy) as u32;
            let leg = match profile {
                Some(p) => p.ticks_for_leg(piece.max_grade, piece.curve, length_sq) as u32,
                None => match length_sq {
                    1 => 8,
                    2 => 11,
                    _ => 18,
                },
            };
            let candidate = cost.saturating_add(leg);
            if best.get(&next).is_none_or(|&b| candidate < b) {
                best.insert(next, candidate);
                prev.insert(next, cur);
                heap.push(Reverse((candidate, next.0)));
            }
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

    /// The third leg of "shortest, cheapest, fastest are three routes": a
    /// train's own router weighs grade drag, so the short steep way loses to
    /// the long flat one *for the train*, while plain distance still picks the
    /// short way.
    #[test]
    fn a_train_prefers_the_longer_flat_route_over_the_short_climb() {
        let terrain = land(12, 12);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(500_000);
        let mut ledger = MoneyLedger::default();
        let mut place = |x: i32, y: i32| {
            try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                TileCoord { x, y },
                GROUND_LAYER,
            )
            .unwrap()
            .id
        };

        // Two parallel rows from (1,2) to (8,2), the same length: the direct
        // one is a hard climb, the one below it is flat. Hop count ties; only
        // running time separates them.
        let a = place(1, 2);
        let steep: Vec<_> = (2..=7).map(|x| place(x, 2)).collect();
        let b = place(8, 2);
        let _flat: Vec<_> = (2..=7).map(|x| place(x, 3)).collect();
        for id in &steep {
            network.piece_mut(*id).unwrap().max_grade = 4;
        }

        let transit = find_path_for_kind(&network, a, b, TrainKind::Transit).expect("routed");
        assert!(
            !transit.iter().any(|id| steep.contains(id)),
            "transit should dodge the climb on time alone: {transit:?}"
        );

        let plain = find_path(&network, a, b).expect("routed");
        assert_eq!(plain.len(), 8, "plain distance is indifferent to the climb");
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
