//! Terrain-aware walking — peeps follow walkable ground, and only ground.
//!
//! Playtest, verbatim: *"Walking is really silly. They don't seem to path-find
//! in the world, they just walk through water and any other terrain."* A person
//! who strolls across a lake is not a person, and brief 06 §4.2's whole
//! "residents are individually visible and knowable" hook dies with them.
//!
//! So a walk is a **route over walkable tiles**, not a straight line between two
//! endpoints:
//!
//! - **Water is impassable** unless a bridge deck carries the walker over it.
//! - **Track is ordinary ground.** Crossing the railway on foot is fine; that is
//!   what a level crossing is.
//! - **Cliffs and the mountain band are impassable**, and steep ground is
//!   expensive, so a route prefers the easy way round a hill to the hard way
//!   over it.
//!
//! The route is computed once per walk and cached on [`WalkRoute`], because a
//! peep walks the same route for many ticks ([`WALK_TICKS_PER_TILE`] each).
//! [`WalkRouter`] caps how many routes the whole town may compute per tick, so
//! a rush-hour departure burst costs a few searches per tick rather than sixty.
//!
//! [`WalkRoute`] is a **cache, not state**: it is recomputed from terrain the
//! moment it is missing or stale, which is why it is deliberately not part of
//! the save snapshot.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap};

use bevy_ecs::prelude::*;

use crate::ids::TileCoord;
use crate::track::{TrackNetwork, TrackTerrain, GROUND_LAYER, MOUNTAIN_HEIGHT_MIN};

use super::journey::{Journey, PeepPosition};

/// Cost of one flat step, in route units. Ten so a climb can cost a fraction.
pub const WALK_STEP_COST: u32 = 10;

/// Extra cost per unit of height climbed or dropped between two tiles.
///
/// Nearly a whole extra step per height unit: a peep will happily walk three
/// tiles round a hillock rather than take one step up it, which is what makes a
/// lane read as a lane instead of as a ruler line.
pub const WALK_CLIMB_COST: u32 = 8;

/// Steepest single step a person will take, in height units.
///
/// Matches the railway's own [`crate::track::MAX_GRADE`] by intent rather than
/// by import: anywhere a line could be laid, somebody can walk. Anything
/// steeper is a cliff face and is refused outright.
pub const WALK_MAX_STEP_GRADE: u8 = 4;

/// Land at this height or above is impassable on foot — the cliff / peak band.
pub const WALK_MAX_HEIGHT: i8 = MOUNTAIN_HEIGHT_MIN;

/// Tiles expanded before a search gives up and reports no route.
///
/// Bounds the worst case: a cut-off peep costs one capped search per departure,
/// not an unbounded flood over the whole map.
pub const WALK_SEARCH_LIMIT: usize = 8_192;

/// Routes the whole town may compute in one tick (see [`WalkRouter`]).
pub const WALK_ROUTES_PER_TICK: usize = 4;

/// Ticks between "somebody cannot walk there" Town Talk lines.
///
/// Matches the complaint dedupe window, so a cut-off district says it once and
/// does not fill the feed.
pub const NO_ROUTE_TALK_COOLDOWN_TICKS: u64 = 120;

/// Terrain (and the railway on top of it) as a walker sees it.
///
/// Borrowed for the length of one system run. `network` is optional because a
/// bridge is the only thing track adds to walkability, and a headless caller
/// without a network still routes correctly over land.
#[derive(Clone, Copy)]
pub struct WalkWorld<'a> {
    terrain: &'a TrackTerrain,
    network: Option<&'a TrackNetwork>,
}

impl<'a> WalkWorld<'a> {
    pub fn new(terrain: &'a TrackTerrain, network: Option<&'a TrackNetwork>) -> Self {
        Self { terrain, network }
    }

    pub fn terrain(&self) -> &TrackTerrain {
        self.terrain
    }

    /// True when a person can stand on this tile.
    pub fn is_walkable(&self, tile: TileCoord) -> bool {
        if !self.terrain.contains(tile) {
            return false;
        }
        if self.terrain.is_water(tile) {
            // Only a deck gets you over water.
            return self.has_bridge(tile);
        }
        self.terrain.height_at(tile).unwrap_or(0) < WALK_MAX_HEIGHT
    }

    /// True when the tile carries a bridge deck.
    pub fn has_bridge(&self, tile: TileCoord) -> bool {
        self.network
            .and_then(|n| n.at(tile, GROUND_LAYER))
            .is_some_and(|piece| piece.is_bridge())
    }

    /// Cost of stepping `from` → `to`, or `None` when a person cannot.
    ///
    /// Water height is a flood tag rather than a climb (the same rule track
    /// placement uses), so a bridge deck is always a flat step.
    pub fn step_cost(&self, from: TileCoord, to: TileCoord) -> Option<u32> {
        if !self.is_walkable(to) {
            return None;
        }
        let wet = self.terrain.is_water(from) || self.terrain.is_water(to);
        if wet {
            return Some(WALK_STEP_COST);
        }
        let a = self.terrain.height_at(from).unwrap_or(0);
        let b = self.terrain.height_at(to).unwrap_or(0);
        let climb = (a as i16 - b as i16).unsigned_abs() as u32;
        if climb > WALK_MAX_STEP_GRADE as u32 {
            return None;
        }
        Some(WALK_STEP_COST + climb * WALK_CLIMB_COST)
    }
}

/// Four-neighbourhood, in a fixed order so a route is stable run to run.
///
/// Deliberately not eight: a diagonal between two tiles that are both beside
/// the same river would cut the corner across the water.
fn neighbors(tile: TileCoord) -> [TileCoord; 4] {
    [
        TileCoord {
            x: tile.x,
            y: tile.y + 1,
        },
        TileCoord {
            x: tile.x + 1,
            y: tile.y,
        },
        TileCoord {
            x: tile.x,
            y: tile.y - 1,
        },
        TileCoord {
            x: tile.x - 1,
            y: tile.y,
        },
    ]
}

fn heuristic(from: TileCoord, to: TileCoord) -> u32 {
    let d = (from.x - to.x).unsigned_abs() + (from.y - to.y).unsigned_abs();
    d.saturating_mul(WALK_STEP_COST)
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Node {
    est: u32,
    cost: u32,
    tile: TileCoord,
}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        // Cheapest estimate first. Ties go to the node *furthest along*, which
        // is what stops an open field — where every route costs the same —
        // from being expanded whole before the goal is reached. Remaining ties
        // break on a fixed tile order, so two runs return the same lane.
        self.est
            .cmp(&other.est)
            .then_with(|| other.cost.cmp(&self.cost))
            .then_with(|| self.tile.y.cmp(&other.tile.y))
            .then_with(|| self.tile.x.cmp(&other.tile.x))
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Cheapest walkable route from `from` to `to`, inclusive of both.
///
/// A* over the four-neighbourhood, same shape as the train pathfinder in
/// [`crate::trains::find_path`] — a `prev` map, a deterministic neighbour order
/// and a reconstruct — with a cost function instead of a plain queue, which is
/// what buys "prefer easy ground over steep".
///
/// The tile the peep is standing on is always enterable (they are on it); the
/// destination must be walkable, so nobody is ever routed to a tile in a lake.
pub fn find_walk_route(
    world: &WalkWorld<'_>,
    from: TileCoord,
    to: TileCoord,
) -> Option<Vec<TileCoord>> {
    find_walk_route_within(world, from, to, WALK_SEARCH_LIMIT)
}

/// [`find_walk_route`] with an explicit expansion cap (tests, tools).
pub fn find_walk_route_within(
    world: &WalkWorld<'_>,
    from: TileCoord,
    to: TileCoord,
    limit: usize,
) -> Option<Vec<TileCoord>> {
    if !world.terrain.contains(from) || !world.terrain.contains(to) {
        return None;
    }
    if from == to {
        return Some(vec![from]);
    }
    if !world.is_walkable(to) {
        return None;
    }

    let mut best: HashMap<TileCoord, u32> = HashMap::new();
    let mut prev: HashMap<TileCoord, TileCoord> = HashMap::new();
    let mut open: BinaryHeap<Reverse<Node>> = BinaryHeap::new();
    best.insert(from, 0);
    open.push(Reverse(Node {
        est: heuristic(from, to),
        cost: 0,
        tile: from,
    }));

    let mut expanded = 0usize;
    while let Some(Reverse(node)) = open.pop() {
        if node.tile == to {
            return Some(reconstruct(&prev, from, to));
        }
        if best.get(&node.tile).is_some_and(|&c| c < node.cost) {
            continue; // stale heap entry
        }
        expanded += 1;
        if expanded > limit {
            return None;
        }
        for next in neighbors(node.tile) {
            let Some(step) = world.step_cost(node.tile, next) else {
                continue;
            };
            let cost = node.cost.saturating_add(step);
            if best.get(&next).is_some_and(|&c| c <= cost) {
                continue;
            }
            best.insert(next, cost);
            prev.insert(next, node.tile);
            open.push(Reverse(Node {
                est: cost.saturating_add(heuristic(next, to)),
                cost,
                tile: next,
            }));
        }
    }
    None
}

fn reconstruct(
    prev: &HashMap<TileCoord, TileCoord>,
    from: TileCoord,
    to: TileCoord,
) -> Vec<TileCoord> {
    let mut path = vec![to];
    let mut cur = to;
    while cur != from {
        let Some(&back) = prev.get(&cur) else {
            break;
        };
        path.push(back);
        cur = back;
    }
    path.reverse();
    path
}

/// How many routes the town has left to compute this tick, and when it last
/// said out loud that somebody was cut off.
///
/// Route computation is amortised rather than budgeted per peep: whoever asks
/// first this tick gets the search, everyone else stands still for a tick and
/// asks again. At [`WALK_TICKS_PER_TILE`](super::journey::WALK_TICKS_PER_TILE)
/// ticks per tile, a tick of hesitation on the doorstep is invisible.
#[derive(Resource, Debug, Clone)]
pub struct WalkRouter {
    /// Routes the whole town may compute per tick.
    pub routes_per_tick: usize,
    /// Expansion cap for one search.
    pub search_limit: usize,
    /// Remaining searches this tick.
    remaining: usize,
    /// Tick of the last "cannot walk there" Town Talk line.
    last_no_route_tick: Option<u64>,
    /// Routes found since start (diagnostics).
    pub routes_found: u64,
    /// Searches that came back with nothing (diagnostics).
    pub routes_failed: u64,
}

impl Default for WalkRouter {
    fn default() -> Self {
        Self {
            routes_per_tick: WALK_ROUTES_PER_TICK,
            search_limit: WALK_SEARCH_LIMIT,
            remaining: WALK_ROUTES_PER_TICK,
            last_no_route_tick: None,
            routes_found: 0,
            routes_failed: 0,
        }
    }
}

impl WalkRouter {
    /// Refill the per-tick search allowance.
    pub fn begin_tick(&mut self) {
        self.remaining = self.routes_per_tick;
    }

    /// Searches still allowed this tick.
    pub fn remaining(&self) -> usize {
        self.remaining
    }

    fn take(&mut self) -> bool {
        if self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        true
    }

    /// True when a no-route line may be spoken at `tick`.
    pub fn may_speak(&self, tick: u64) -> bool {
        match self.last_no_route_tick {
            None => true,
            Some(last) => tick.saturating_sub(last) >= NO_ROUTE_TALK_COOLDOWN_TICKS,
        }
    }

    pub fn note_spoke(&mut self, tick: u64) {
        self.last_no_route_tick = Some(tick);
    }
}

/// What one tick of walking did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkStep {
    /// Standing on the target tile.
    Arrived,
    /// Moved along the route.
    Walking,
    /// No route yet this tick — waiting for the router, standing still.
    Waiting,
    /// There is genuinely no walkable way there.
    NoRoute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum RouteState {
    /// Nothing computed yet for the current goal.
    #[default]
    Idle,
    /// Following `tiles`.
    Following,
    /// Search failed — the caller decides what the peep does about it.
    NoRoute,
}

/// The cached walk a peep is following.
///
/// Recomputed whenever the goal changes, whenever the peep is put down
/// somewhere else (level-of-detail promotion), and whenever it is missing —
/// which is why it never needs saving.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct WalkRoute {
    tiles: Vec<TileCoord>,
    /// Index of the waypoint being walked toward.
    next: usize,
    goal: Option<TileCoord>,
    state: RouteState,
}

impl WalkRoute {
    /// Tile the route ends on, if one is planned.
    pub fn goal(&self) -> Option<TileCoord> {
        self.goal
    }

    /// The whole planned route, start tile first.
    pub fn tiles(&self) -> &[TileCoord] {
        &self.tiles
    }

    /// Waypoint currently being walked toward.
    pub fn next_tile(&self) -> Option<TileCoord> {
        self.tiles.get(self.next).copied()
    }

    /// Waypoints still to walk, including the one under way.
    pub fn remaining(&self) -> usize {
        self.tiles.len().saturating_sub(self.next)
    }

    /// True when the last search found nothing.
    pub fn is_blocked(&self) -> bool {
        self.state == RouteState::NoRoute
    }

    /// True while a computed route is being followed.
    pub fn is_following(&self) -> bool {
        self.state == RouteState::Following
    }

    /// Forget everything — the next call re-plans from scratch.
    pub fn clear(&mut self) {
        self.tiles.clear();
        self.next = 0;
        self.goal = None;
        self.state = RouteState::Idle;
    }

    fn reset_for(&mut self, goal: TileCoord) {
        self.tiles.clear();
        self.next = 0;
        self.goal = Some(goal);
        self.state = RouteState::Idle;
    }

    /// True when the peep is no longer anywhere near the route they were on.
    fn stale_at(&self, tile: TileCoord) -> bool {
        let Some(next) = self.next_tile() else {
            return true;
        };
        (next.x - tile.x).abs() + (next.y - tile.y).abs() > 2
    }

    /// Walk one tick toward `target`, following walkable ground.
    ///
    /// Plans on first use and after any invalidation; otherwise this is a
    /// single `walk_toward` against the next waypoint, which is what keeps a
    /// peep cheap for the many ticks a walk takes.
    pub fn advance(
        &mut self,
        pos: &mut PeepPosition,
        target: TileCoord,
        speed: f32,
        world: &WalkWorld<'_>,
        router: &mut WalkRouter,
    ) -> WalkStep {
        if self.goal != Some(target) {
            self.reset_for(target);
        }
        if self.state == RouteState::Following && self.stale_at(pos.tile()) {
            // Put down somewhere else since the route was planned.
            self.reset_for(target);
        }

        match self.state {
            RouteState::NoRoute => return WalkStep::NoRoute,
            RouteState::Idle => {
                let here = pos.tile();
                if here == target {
                    pos.stand_still();
                    return WalkStep::Arrived;
                }
                if !router.take() {
                    pos.stand_still();
                    return WalkStep::Waiting;
                }
                match find_walk_route_within(world, here, target, router.search_limit) {
                    Some(tiles) => {
                        router.routes_found = router.routes_found.saturating_add(1);
                        self.tiles = tiles;
                        // The first entry is the tile they are standing on.
                        self.next = 1.min(self.tiles.len().saturating_sub(1));
                        self.state = RouteState::Following;
                    }
                    None => {
                        router.routes_failed = router.routes_failed.saturating_add(1);
                        self.state = RouteState::NoRoute;
                        pos.stand_still();
                        return WalkStep::NoRoute;
                    }
                }
            }
            RouteState::Following => {}
        }

        let Some(waypoint) = self.next_tile() else {
            pos.stand_still();
            return WalkStep::Arrived;
        };
        if pos.walk_toward(waypoint, speed) {
            self.next += 1;
            if self.next >= self.tiles.len() {
                return WalkStep::Arrived;
            }
            // Mid-route corner: keep the walk cycle running rather than
            // stuttering to a stop on every tile boundary.
            pos.keep_walking();
        }
        WalkStep::Walking
    }
}

/// One tick of walking for a peep who may or may not have a route or a terrain.
///
/// - Route **and** terrain: the real thing.
/// - No terrain snapshot (headless tests, dedicated server): straight line, the
///   pre-terrain behaviour, because there is nothing to route around.
/// - Terrain but no route component yet (a peep restored from a save, one tick
///   before [`ensure_walk_routes`] catches up): stand still. Never a straight
///   line — that is the bug this module exists to fix.
pub fn walk_step(
    route: Option<&mut WalkRoute>,
    pos: &mut PeepPosition,
    target: TileCoord,
    speed: f32,
    world: Option<&WalkWorld<'_>>,
    router: &mut WalkRouter,
) -> WalkStep {
    match (route, world) {
        (Some(route), Some(world)) => route.advance(pos, target, speed, world, router),
        (_, None) => {
            if pos.walk_toward(target, speed) {
                WalkStep::Arrived
            } else {
                WalkStep::Walking
            }
        }
        (None, Some(_)) => {
            pos.stand_still();
            WalkStep::Waiting
        }
    }
}

/// Give every peep a route cache.
///
/// Freshly spawned peeps get one in their bundle; this catches the ones a save
/// restore put back into the world, so the route stays out of the save file.
pub fn ensure_walk_routes(
    mut commands: Commands,
    peeps: Query<Entity, (With<Journey>, Without<WalkRoute>)>,
) {
    for entity in peeps.iter() {
        commands.entity(entity).insert(WalkRoute::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economy::MoneyLedger;
    use crate::money::Money;
    use crate::peeps::{Facing, WALK_TILES_PER_TICK};
    use crate::track::try_place_track;

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    fn land(w: u32, h: u32) -> TrackTerrain {
        TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 0i8)))
    }

    /// A north-south river at `x = river_x`, with an optional dry gap at `gap_y`.
    fn river(w: u32, h: u32, river_x: i32, gap: Option<i32>) -> TrackTerrain {
        let cells = (0..w * h).map(|i| {
            let x = (i % w) as i32;
            let y = (i / w) as i32;
            let wet = x == river_x && gap != Some(y);
            (wet, 0i8)
        });
        TrackTerrain::new(w, h, cells)
    }

    fn heights(w: u32, h: u32, f: impl Fn(i32, i32) -> i8) -> TrackTerrain {
        let cells = (0..w * h).map(|i| {
            let x = (i % w) as i32;
            let y = (i / w) as i32;
            (false, f(x, y))
        });
        TrackTerrain::new(w, h, cells)
    }

    fn route_is_contiguous(route: &[TileCoord]) -> bool {
        route
            .windows(2)
            .all(|w| (w[0].x - w[1].x).abs() + (w[0].y - w[1].y).abs() == 1)
    }

    #[test]
    fn a_route_goes_round_water_rather_than_over_it() {
        let terrain = river(12, 8, 5, Some(6));
        let world = WalkWorld::new(&terrain, None);
        let route = find_walk_route(&world, tile(1, 1), tile(9, 1)).expect("a way round exists");

        assert!(route_is_contiguous(&route), "route jumps: {route:?}");
        assert_eq!(route.first(), Some(&tile(1, 1)));
        assert_eq!(route.last(), Some(&tile(9, 1)));
        for step in &route {
            assert!(
                !terrain.is_water(*step),
                "peep would walk on water at {step:?}"
            );
        }
        // It has to go up to the ford at y = 6 and back, so it is longer than
        // the straight line it used to take.
        assert!(route.len() > 9, "route suspiciously short: {route:?}");
    }

    #[test]
    fn no_route_when_the_water_cuts_the_map_in_two() {
        let terrain = river(12, 8, 5, None);
        let world = WalkWorld::new(&terrain, None);
        assert!(find_walk_route(&world, tile(1, 1), tile(9, 1)).is_none());
        // …but each bank is still fine on its own.
        assert!(find_walk_route(&world, tile(1, 1), tile(3, 6)).is_some());
    }

    #[test]
    fn a_bridge_is_walkable_ground() {
        let terrain = river(12, 8, 5, None);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(5_000_000);
        let mut ledger = MoneyLedger::default();
        for x in 3..=7 {
            try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                tile(x, 4),
                GROUND_LAYER,
            )
            .expect("lay the crossing");
        }

        let bridged = WalkWorld::new(&terrain, Some(&network));
        assert!(bridged.has_bridge(tile(5, 4)), "expected a deck over the river");
        let route = find_walk_route(&bridged, tile(1, 1), tile(9, 1)).expect("over the bridge");
        assert!(route.contains(&tile(5, 4)), "route ignored the bridge: {route:?}");
        assert!(route_is_contiguous(&route));

        // Without the network the same water is still water.
        let dry = WalkWorld::new(&terrain, None);
        assert!(find_walk_route(&dry, tile(1, 1), tile(9, 1)).is_none());
    }

    #[test]
    fn track_on_land_is_ordinary_ground_to_walk_on() {
        let terrain = land(10, 6);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(5_000_000);
        let mut ledger = MoneyLedger::default();
        for y in 0..6 {
            try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                tile(4, y),
                GROUND_LAYER,
            )
            .expect("lay the line");
        }
        let world = WalkWorld::new(&terrain, Some(&network));
        let route = find_walk_route(&world, tile(1, 2), tile(8, 2)).expect("cross the railway");
        assert_eq!(route.len(), 8, "a level crossing costs nothing: {route:?}");
        assert!(route.contains(&tile(4, 2)));
    }

    #[test]
    fn steep_ground_is_avoided_when_flat_ground_is_available() {
        // A ridge four high across the middle, with a flat notch at y = 0.
        let terrain = heights(9, 5, |x, y| if x == 4 && y > 0 { 4 } else { 0 });
        let world = WalkWorld::new(&terrain, None);
        let route = find_walk_route(&world, tile(1, 1), tile(7, 1)).expect("a way exists");
        assert!(
            route.iter().all(|t| terrain.height_at(*t) == Some(0)),
            "route climbed the ridge instead of walking round it: {route:?}"
        );
        assert!(route_is_contiguous(&route));
    }

    #[test]
    fn a_gentle_rise_is_walked_over_not_round() {
        let terrain = heights(9, 5, |x, _| if x == 4 { 1 } else { 0 });
        let world = WalkWorld::new(&terrain, None);
        let route = find_walk_route(&world, tile(1, 3), tile(7, 3)).expect("a way exists");
        assert_eq!(route.len(), 7, "a one-step rise is not worth a detour");
    }

    #[test]
    fn cliffs_and_peaks_are_impassable() {
        let terrain = heights(9, 5, |x, _| if x == 4 { WALK_MAX_HEIGHT } else { 0 });
        let world = WalkWorld::new(&terrain, None);
        assert!(!world.is_walkable(tile(4, 2)));
        assert!(find_walk_route(&world, tile(1, 2), tile(7, 2)).is_none());

        // A sheer step is refused even below the peak band.
        let step = heights(6, 3, |x, _| if x >= 3 { WALK_MAX_STEP_GRADE as i8 + 1 } else { 0 });
        let world = WalkWorld::new(&step, None);
        assert!(world.is_walkable(tile(3, 1)), "the top is standable");
        assert!(
            world.step_cost(tile(2, 1), tile(3, 1)).is_none(),
            "nobody scales a cliff face"
        );
        assert!(find_walk_route(&world, tile(0, 1), tile(5, 1)).is_none());
    }

    #[test]
    fn a_destination_in_the_water_has_no_route() {
        let terrain = river(12, 8, 5, Some(6));
        let world = WalkWorld::new(&terrain, None);
        assert!(find_walk_route(&world, tile(1, 1), tile(5, 1)).is_none());
        // Standing still is always possible, even somewhere silly.
        assert_eq!(
            find_walk_route(&world, tile(5, 1), tile(5, 1)),
            Some(vec![tile(5, 1)])
        );
    }

    #[test]
    fn the_search_is_capped_rather_than_flooding_the_map() {
        let terrain = river(40, 40, 20, None);
        let world = WalkWorld::new(&terrain, None);
        // A generous cap still fails on a genuinely cut map…
        assert!(find_walk_route(&world, tile(1, 1), tile(30, 30)).is_none());
        // …and a tight cap fails fast rather than expanding the whole bank.
        assert!(find_walk_route_within(&world, tile(1, 1), tile(30, 30), 16).is_none());
    }

    /// Cost guard: an open-ground walk must not expand the field it crosses.
    #[test]
    fn an_open_ground_route_is_found_without_flooding_the_field() {
        let terrain = land(64, 64);
        let world = WalkWorld::new(&terrain, None);
        let route = find_walk_route_within(&world, tile(2, 32), tile(42, 32), 128)
            .expect("a straight walk across open ground is cheap to find");
        assert_eq!(route.len(), 41);
    }

    #[test]
    fn routes_are_deterministic() {
        let terrain = river(16, 12, 7, Some(9));
        let world = WalkWorld::new(&terrain, None);
        let a = find_walk_route(&world, tile(2, 2), tile(12, 3));
        let b = find_walk_route(&world, tile(2, 2), tile(12, 3));
        assert_eq!(a, b);
    }

    #[test]
    fn walking_follows_the_route_tile_by_tile_and_faces_the_way_it_goes() {
        let terrain = river(12, 8, 5, Some(6));
        let world = WalkWorld::new(&terrain, None);
        let mut router = WalkRouter::default();
        router.begin_tick();
        let mut route = WalkRoute::default();
        let mut pos = PeepPosition::at_tile(tile(1, 1), 7);

        let mut facings = Vec::new();
        let mut arrived = false;
        for _ in 0..6_000 {
            router.begin_tick();
            let step = route.advance(
                &mut pos,
                tile(9, 1),
                WALK_TILES_PER_TICK,
                &world,
                &mut router,
            );
            assert_ne!(step, WalkStep::NoRoute);
            assert!(
                !terrain.is_water(pos.tile()),
                "the peep stepped into the water at {:?}",
                pos.tile()
            );
            if facings.last() != Some(&pos.facing) {
                facings.push(pos.facing);
            }
            if step == WalkStep::Arrived {
                arrived = true;
                break;
            }
        }
        assert!(arrived, "the peep never got there");
        assert_eq!(pos.tile(), tile(9, 1));
        assert!(
            facings.contains(&Facing::North) && facings.contains(&Facing::East),
            "facing must follow the route round the water: {facings:?}"
        );
        assert_eq!(route.remaining(), 0);
    }

    #[test]
    fn a_cut_off_peep_reports_no_route_instead_of_swimming() {
        let terrain = river(12, 8, 5, None);
        let world = WalkWorld::new(&terrain, None);
        let mut router = WalkRouter::default();
        router.begin_tick();
        let mut route = WalkRoute::default();
        let mut pos = PeepPosition::at_tile(tile(1, 1), 3);
        let before = (pos.x, pos.y);

        let step = route.advance(
            &mut pos,
            tile(9, 1),
            WALK_TILES_PER_TICK,
            &world,
            &mut router,
        );
        assert_eq!(step, WalkStep::NoRoute);
        assert!(route.is_blocked());
        assert_eq!((pos.x, pos.y), before, "a blocked peep must not drift");
        assert!(!pos.walking);

        // The failure is remembered, so it costs one search and not one a tick.
        let spent = router.routes_failed;
        route.advance(
            &mut pos,
            tile(9, 1),
            WALK_TILES_PER_TICK,
            &world,
            &mut router,
        );
        assert_eq!(router.routes_failed, spent, "re-searched a known dead end");

        // Clearing it is how the caller says "try again next time you set off".
        route.clear();
        assert!(!route.is_blocked());
    }

    #[test]
    fn the_router_amortises_route_computation_across_ticks() {
        let terrain = land(24, 24);
        let world = WalkWorld::new(&terrain, None);
        let mut router = WalkRouter::default();
        router.begin_tick();

        let mut routes: Vec<WalkRoute> = (0..WALK_ROUTES_PER_TICK + 3)
            .map(|_| WalkRoute::default())
            .collect();
        let mut waiting = 0;
        for (i, route) in routes.iter_mut().enumerate() {
            let mut pos = PeepPosition::at_tile(tile(1, i as i32), i as u64);
            let step = route.advance(
                &mut pos,
                tile(20, i as i32),
                WALK_TILES_PER_TICK,
                &world,
                &mut router,
            );
            if step == WalkStep::Waiting {
                waiting += 1;
                assert!(!pos.walking, "a peep with no route yet stands still");
            }
        }
        assert_eq!(waiting, 3, "the per-tick route budget was not enforced");
        assert_eq!(router.remaining(), 0);

        // Next tick the rest get theirs.
        router.begin_tick();
        assert_eq!(router.remaining(), WALK_ROUTES_PER_TICK);
    }

    #[test]
    fn a_new_goal_replans_and_a_teleport_replans() {
        let terrain = land(24, 24);
        let world = WalkWorld::new(&terrain, None);
        let mut router = WalkRouter::default();
        let mut route = WalkRoute::default();
        let mut pos = PeepPosition::at_tile(tile(2, 2), 1);

        router.begin_tick();
        route.advance(&mut pos, tile(8, 2), WALK_TILES_PER_TICK, &world, &mut router);
        assert_eq!(route.goal(), Some(tile(8, 2)));
        let first = route.tiles().to_vec();

        router.begin_tick();
        route.advance(&mut pos, tile(2, 9), WALK_TILES_PER_TICK, &world, &mut router);
        assert_eq!(route.goal(), Some(tile(2, 9)));
        assert_ne!(route.tiles(), first.as_slice());

        // Put down somewhere else with the same goal: the stale route is dropped.
        pos.snap_to(tile(14, 14));
        router.begin_tick();
        route.advance(&mut pos, tile(2, 9), WALK_TILES_PER_TICK, &world, &mut router);
        assert_eq!(route.tiles().first(), Some(&tile(14, 14)));
    }

    #[test]
    fn no_route_talk_is_rate_limited() {
        let mut router = WalkRouter::default();
        assert!(router.may_speak(0));
        router.note_spoke(10);
        assert!(!router.may_speak(20));
        assert!(router.may_speak(10 + NO_ROUTE_TALK_COOLDOWN_TICKS));
    }

    #[test]
    fn without_terrain_walking_is_the_old_straight_line() {
        let mut router = WalkRouter::default();
        let mut pos = PeepPosition::at_tile(tile(0, 0), 1);
        let mut ticks = 0;
        loop {
            let step = walk_step(
                None,
                &mut pos,
                tile(3, 0),
                WALK_TILES_PER_TICK,
                None,
                &mut router,
            );
            ticks += 1;
            assert!(ticks < 10_000);
            if step == WalkStep::Arrived {
                break;
            }
        }
        assert_eq!(pos.tile(), tile(3, 0));
    }
}
