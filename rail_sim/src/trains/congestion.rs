//! Passing loops, double track, and rerouting round a blockage.
//!
//! `docs/design/07-trains-and-lines.md` §4.3 wants congestion to be *solvable*:
//! the player lays a siding beside a busy single line, or a second running line
//! along a corridor, and the trains use it. §4.4 wants slack rewarded — when the
//! main line is blocked and the player paid for another way round, trains take
//! it and the player watches the money they spent save them.
//!
//! Both shapes are already expressible on the tile graph (a passing loop is a
//! short parallel run of track, double track is a long one), so this module lays
//! nothing new: it is the movement policy that makes them pay off. When the
//! network offers no slack at all the policy declines and the train simply waits,
//! which is exactly the pre-existing behaviour — no new deadlock path.

use std::collections::{HashMap, HashSet};

use crate::commands::TrainKind;
use crate::ids::{TrackId, TrainId};
use crate::track::TrackNetwork;

use super::movement::TileOccupancy;
use super::path::find_path_avoiding;
use super::profile::TrainProfile;
use super::train::{Train, TrainLocation};

/// Ticks held at a stop line before a train looks for a way round.
pub const REROUTE_AFTER_TICKS: u16 = 4;
/// Extra tiles a near reroute may cost — a passing loop or the parallel tile of
/// a double-track corridor is only ever a handful of tiles longer.
pub const REROUTE_NEAR_EXTRA: usize = 8;
/// Ticks held before *any* longer alternative is worth taking. This is the
/// "second way round" the player paid for finally earning its keep.
pub const REROUTE_LONG_AFTER_TICKS: u16 = 30;
/// Ticks held nose-to-nose before one train steps aside into a passing loop.
pub const YIELD_AFTER_TICKS: u16 = 12;
/// Ticks before a train that stepped aside may do so again (stops shuffling).
pub const YIELD_COOLDOWN_TICKS: u16 = 60;
/// How far along the other train's route we refuse to park while letting it by.
const YIELD_LOOKAHEAD: usize = 8;

/// What each train wanted at the start of the movement pass.
///
/// Snapshotted before anything moves so head-on detection is order-independent.
#[derive(Debug, Clone)]
pub struct TrainIntent {
    pub at: TrackId,
    /// Tile the train is trying to enter, if it still has route left.
    pub next: Option<TrackId>,
    /// First few tiles of the remaining route.
    pub ahead: Vec<TrackId>,
}

impl TrainIntent {
    pub fn of(loc: &TrainLocation) -> Self {
        Self {
            at: loc.track,
            next: loc.path.get(loc.path_index + 1).copied(),
            ahead: loc
                .path
                .iter()
                .skip(loc.path_index)
                .take(YIELD_LOOKAHEAD)
                .copied()
                .collect(),
        }
    }
}

/// The move a held train can make instead of waiting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Way {
    /// Another route to the same destination — double track, or a way round.
    Reroute(Vec<TrackId>),
    /// Step into a passing loop, let the other train through, then carry on.
    Yield(Vec<TrackId>),
}

/// Decide what a held train does about the tile it cannot enter.
///
/// Returns the route *ahead* of the train (starting on its current tile), or
/// `None` when the network offers nothing better than waiting.
pub fn way_round(
    network: &TrackNetwork,
    occupancy: &TileOccupancy,
    intent: &HashMap<TrainId, TrainIntent>,
    train: &Train,
    loc: &TrainLocation,
    blocker: Option<TrainId>,
    held_ticks: u16,
) -> Option<Way> {
    if held_ticks < REROUTE_AFTER_TICKS {
        return None;
    }
    let dest = loc.destination()?;
    if dest == loc.track {
        return None;
    }

    let busy = tiles_taken_by_others(occupancy, train.id);
    let wanted = loc.path.get(loc.path_index + 1).copied();

    // 1. A genuine alternative: the parallel tile of a double-track corridor,
    //    the far side of a passing loop, or the long way round a blockage.
    if let Some(route) = find_path_avoiding(network, loc.track, dest, train.kind, &busy) {
        let remaining = loc.path.len().saturating_sub(loc.path_index);
        let near = route.len() <= remaining.saturating_add(REROUTE_NEAR_EXTRA);
        let worth_it = near || held_ticks >= REROUTE_LONG_AFTER_TICKS;
        if worth_it && route.get(1).copied() != wanted && route.len() > 1 {
            return Some(Way::Reroute(route));
        }
    }

    // 2. Single track with nowhere to route: nose-to-nose, one of the pair
    //    steps into a passing loop so the other can pass.
    let blocker = blocker?;
    if held_ticks < YIELD_AFTER_TICKS || occupancy.yield_cooldown(train.id) > 0 {
        return None;
    }
    let other = intent.get(&blocker)?;
    if other.next != Some(loc.track) {
        // Not a standoff — the tile ahead will clear on its own.
        return None;
    }
    // Deterministic tie-break: the higher-numbered train gives way. If it has
    // no loop to give way into, the other one tries after a longer hold.
    if blocker.0 > train.id.0 && held_ticks < YIELD_AFTER_TICKS.saturating_mul(3) {
        return None;
    }

    let mut avoid = busy;
    avoid.extend(other.ahead.iter().copied());
    avoid.extend(loc.path.iter().skip(loc.path_index).copied());
    let siding = passing_tile(network, loc.track, train.kind, &avoid)?;
    Some(Way::Yield(yield_route(loc, siding)))
}

/// Tiles held by trains other than `me`.
pub fn tiles_taken_by_others(occupancy: &TileOccupancy, me: TrainId) -> HashSet<TrackId> {
    occupancy
        .by_track
        .iter()
        .filter(|(_, &id)| id != me)
        .map(|(&track, _)| track)
        .collect()
}

/// A free tile beside `at` the train can stand on while another train passes.
///
/// This is the passing loop: any track adjacent to the current tile that is not
/// occupied, not on either train's route, and not too steep for the profile.
/// Lowest id wins so the choice is deterministic.
pub fn passing_tile(
    network: &TrackNetwork,
    at: TrackId,
    kind: TrainKind,
    avoid: &HashSet<TrackId>,
) -> Option<TrackId> {
    let profile = TrainProfile::for_kind(kind);
    network
        .neighbor_ids(at)
        .into_iter()
        .filter(|id| !avoid.contains(id))
        .filter(|id| {
            network
                .piece(*id)
                .is_some_and(|p| profile.tolerates_grade(p.max_grade))
        })
        .min_by_key(|id| id.0)
}

/// Route ahead that ducks into `siding`, comes back, and resumes the journey.
///
/// Keeping the original tail means the train never forgets where it was going,
/// so a yield costs time and nothing else.
pub fn yield_route(loc: &TrainLocation, siding: TrackId) -> Vec<TrackId> {
    let mut route = vec![loc.track, siding, loc.track];
    route.extend(loc.path.iter().skip(loc.path_index + 1).copied());
    route
}

/// Walk `blocked_by` to the train at the head of a queue.
///
/// §4.2: following the chain to its head should take seconds, so the inspector
/// can offer the cause directly rather than one hop at a time.
pub fn blocked_chain_head(occupancy: &TileOccupancy, train: TrainId) -> Option<TrainId> {
    let mut seen = HashSet::new();
    seen.insert(train);
    let mut cur = *occupancy.blocked_by.get(&train)?;
    loop {
        if !seen.insert(cur) {
            // Cycle (mutual standoff) — cur is as far as the chain goes.
            return Some(cur);
        }
        match occupancy.blocked_by.get(&cur) {
            Some(&next) => cur = next,
            None => return Some(cur),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economy::MoneyLedger;
    use crate::ids::TileCoord;
    use crate::money::Money;
    use crate::track::{try_place_track, TrackTerrain, GROUND_LAYER};

    fn land(w: u32, h: u32) -> TrackTerrain {
        TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 0i8)))
    }

    fn lay(network: &mut TrackNetwork, terrain: &TrackTerrain, tiles: &[(i32, i32)]) -> Vec<TrackId> {
        let mut money = Money::new(50_000_000);
        let mut ledger = MoneyLedger::default();
        tiles
            .iter()
            .map(|&(x, y)| {
                try_place_track(
                    network,
                    &mut money,
                    &mut ledger,
                    terrain,
                    TileCoord { x, y },
                    GROUND_LAYER,
                )
                .expect("place")
                .id
            })
            .collect()
    }

    #[test]
    fn passing_tile_finds_a_free_neighbour_off_route() {
        let terrain = land(8, 8);
        let mut network = TrackNetwork::new();
        // Main line y=2 with a one-tile loop at (3, 3).
        let main = lay(&mut network, &terrain, &[(1, 2), (2, 2), (3, 2), (4, 2)]);
        let loop_ids = lay(&mut network, &terrain, &[(3, 3)]);

        let mut avoid: HashSet<TrackId> = main.iter().copied().collect();
        avoid.remove(&main[2]);
        let found = passing_tile(&network, main[2], TrainKind::Transit, &avoid);
        assert_eq!(found, Some(loop_ids[0]), "should step into the loop tile");
    }

    #[test]
    fn passing_tile_declines_when_there_is_no_slack() {
        let terrain = land(8, 8);
        let mut network = TrackNetwork::new();
        let main = lay(&mut network, &terrain, &[(1, 2), (2, 2), (3, 2)]);
        let avoid: HashSet<TrackId> = main.iter().copied().collect();
        assert_eq!(passing_tile(&network, main[1], TrainKind::Transit, &avoid), None);
    }

    #[test]
    fn yield_route_ducks_aside_and_keeps_the_destination() {
        let mut loc = TrainLocation::at_track(TrackId(1));
        loc.path = vec![TrackId(1), TrackId(2), TrackId(3)];
        loc.path_index = 0;
        let route = yield_route(&loc, TrackId(9));
        assert_eq!(
            route,
            vec![TrackId(1), TrackId(9), TrackId(1), TrackId(2), TrackId(3)]
        );
        assert_eq!(route.last(), loc.path.last(), "destination survives a yield");
    }

    #[test]
    fn chain_head_walks_a_queue_to_its_cause() {
        let mut occupancy = TileOccupancy::default();
        // 4 waits on 3 waits on 2 waits on 1; 1 is the cause.
        occupancy.blocked_by.insert(TrainId(4), TrainId(3));
        occupancy.blocked_by.insert(TrainId(3), TrainId(2));
        occupancy.blocked_by.insert(TrainId(2), TrainId(1));
        assert_eq!(blocked_chain_head(&occupancy, TrainId(4)), Some(TrainId(1)));
        assert_eq!(blocked_chain_head(&occupancy, TrainId(1)), None);
    }

    #[test]
    fn chain_head_terminates_on_a_standoff() {
        let mut occupancy = TileOccupancy::default();
        occupancy.blocked_by.insert(TrainId(1), TrainId(2));
        occupancy.blocked_by.insert(TrainId(2), TrainId(1));
        assert!(blocked_chain_head(&occupancy, TrainId(1)).is_some());
    }
}
