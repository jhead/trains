//! Advance trains along paths; one train per tile, with a way round.
//!
//! Congestion (`docs/design/07-trains-and-lines.md` §4): a held train records
//! its blocker for the inspector, then asks [`super::congestion`] whether the
//! player built any slack it can use — the parallel tile of a double-track
//! corridor, the far side of a passing loop, or a longer route round the
//! blockage. Only when the network offers nothing does it wait, which is the
//! behaviour this module has always had.
//!
//! Trains are served in [`TrainId`] order so the sim stays deterministic and a
//! queue keeps a stable pecking order between ticks.

use std::collections::{HashMap, HashSet};

use bevy_ecs::prelude::*;

use crate::commands::TrainKind;
use crate::ids::{TrackId, TrainId};
use crate::track::TrackNetwork;

use super::congestion::{way_round, TrainIntent, Way, YIELD_COOLDOWN_TICKS};
use super::profile::TrainProfile;
use super::train::{cars_of, Train, TrainConsist, TrainLocation, TrainYard};

/// Movement ticks a crossing is remembered for (railhead polish / usage).
pub const POLISH_MEMORY_TICKS: u64 = 512;
/// Prune the crossing memory this often (tiles, not trains — cheap).
const POLISH_PRUNE_EVERY: u64 = 256;

/// Occupancy map rebuilt each movement pass (train id on each track tile).
///
/// Also carries the congestion read-outs the presentation and inspector need:
/// who is blocking whom, how long each train has been held, and which tiles
/// were crossed recently (the railhead polish in
/// `docs/design/01-art-direction.md` §5.3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Resource, serde::Serialize, serde::Deserialize)]
pub struct TileOccupancy {
    pub by_track: HashMap<TrackId, TrainId>,
    /// Train waiting on next tile → id of the train occupying that tile.
    pub blocked_by: HashMap<TrainId, TrainId>,
    /// Consecutive ticks a train has been held at a stop line (absent = moving).
    pub held: HashMap<TrainId, u16>,
    /// Ticks until a train that stepped aside may step aside again.
    pub yield_cooldown: HashMap<TrainId, u16>,
    /// Monotonic movement tick; pairs with [`Self::last_crossed`].
    pub tick: u64,
    /// Movement tick a train last entered each tile.
    pub last_crossed: HashMap<TrackId, u64>,
    /// Trains that took an alternative route this tick.
    pub rerouted: Vec<TrainId>,
    /// Trains that stepped into a passing loop this tick.
    pub yielded: Vec<TrainId>,
}

impl TileOccupancy {
    /// Ticks a train has been held at a stop line (0 when running).
    pub fn held_ticks(&self, train: TrainId) -> u16 {
        self.held.get(&train).copied().unwrap_or(0)
    }

    pub fn is_blocked(&self, train: TrainId) -> bool {
        self.blocked_by.contains_key(&train)
    }

    pub fn yield_cooldown(&self, train: TrainId) -> u16 {
        self.yield_cooldown.get(&train).copied().unwrap_or(0)
    }

    /// Movement ticks since a train last crossed `track`, if it is remembered.
    pub fn ticks_since_crossed(&self, track: TrackId) -> Option<u64> {
        let last = *self.last_crossed.get(&track)?;
        Some(self.tick.saturating_sub(last))
    }

    fn begin_pass(&mut self) {
        self.tick = self.tick.saturating_add(1);
        self.by_track.clear();
        self.blocked_by.clear();
        self.rerouted.clear();
        self.yielded.clear();
    }
}

/// Trains standing on a tile the network no longer has, in [`TrainId`] order.
///
/// **A train must never exist without a valid position on track**, and one that
/// does is not a cosmetic problem. Every system that works on trains skips it:
/// it does not move, cannot be routed, and — there being no sell command — the
/// player cannot get rid of it. It is a permanent zombie standing on open
/// ground, which is exactly what the orphaned-train report describes.
///
/// Nothing *spawns* a train that way: [`super::apply::apply_train_commands`]
/// refuses to place without a railhead. Two things can leave one that way —
/// track lifted out from under a standing train, and a train returning from a
/// border crossing onto the railhead it left from, which the player may have
/// demolished while it was away.
///
/// [`advance_trains`] recalls them. The stock is the player's and they paid for
/// it, so it goes back to the yard to be placed again rather than deleted.
fn stranded_trains(
    network: &TrackNetwork,
    q: &Query<(Entity, &Train, &mut TrainLocation, Option<&TrainConsist>)>,
) -> Vec<(TrainId, Entity, TrainKind)> {
    let mut stranded: Vec<(TrainId, Entity, TrainKind)> = q
        .iter()
        .filter(|(_, _, loc, _)| network.piece(loc.track).is_none())
        .map(|(entity, train, _, _)| (train.id, entity, train.kind))
        .collect();
    stranded.sort_unstable_by_key(|(id, _, _)| id.0);
    stranded
}

/// Move trains one step when progress allows and the next tile is free.
pub fn advance_trains(
    network: Res<TrackNetwork>,
    mut occupancy: ResMut<TileOccupancy>,
    mut yard: ResMut<TrainYard>,
    mut commands: Commands,
    mut q: Query<(Entity, &Train, &mut TrainLocation, Option<&TrainConsist>)>,
) {
    // Before anything is moved, make sure everything *can* be moved. Recalled
    // entities are despawned through `Commands`, so they are still visible to
    // the queries below this tick — hence the explicit skip list.
    //
    // The yard is only touched when there is something to put in it: taking a
    // `ResMut` and dereferencing it every tick would mark the resource changed
    // for every reader, and the panels that watch it would rebuild forever.
    let stranded = stranded_trains(&network, &q);
    let recalled: Vec<TrainId> = stranded
        .iter()
        .map(|&(id, entity, kind)| {
            commands.entity(entity).despawn();
            yard.return_train(id, kind);
            id
        })
        .collect();

    occupancy.begin_pass();

    // Snapshot before anything moves: live occupancy, service order, and what
    // each train wants next (so a standoff reads the same whoever moves first).
    let mut order: Vec<(TrainId, Entity)> = Vec::new();
    let mut intent: HashMap<TrainId, TrainIntent> = HashMap::new();
    for (entity, train, loc, _) in q.iter() {
        if recalled.contains(&train.id) {
            continue;
        }
        occupancy.by_track.insert(loc.track, train.id);
        order.push((train.id, entity));
        intent.insert(train.id, TrainIntent::of(loc));
    }
    order.sort_unstable_by_key(|(id, _)| id.0);

    let live: HashSet<TrainId> = intent.keys().copied().collect();
    occupancy.held.retain(|id, _| live.contains(id));
    occupancy.yield_cooldown.retain(|id, _| live.contains(id));
    for cooldown in occupancy.yield_cooldown.values_mut() {
        *cooldown = cooldown.saturating_sub(1);
    }

    for (_, entity) in order {
        let Ok((_, train, mut loc, consist)) = q.get_mut(entity) else {
            continue;
        };
        if loc.parked {
            occupancy.held.remove(&train.id);
            continue;
        }
        // Dwell at stop: count down, don't move.
        if loc.dwell_remaining > 0 {
            loc.dwell_remaining = loc.dwell_remaining.saturating_sub(1);
            occupancy.held.remove(&train.id);
            continue;
        }
        if loc.at_destination() {
            occupancy.held.remove(&train.id);
            continue;
        }
        let Some(piece) = network.piece(loc.track) else {
            continue;
        };
        // The consist is part of the train's pace, not a separate charge: a
        // longer train crosses every tile slower, and the same slowed profile
        // is what its dwell and its sprite interpolation read.
        let profile = TrainProfile::for_kind(train.kind).for_consist(cars_of(consist));
        // Charge for the leg actually being travelled, not for "a tile". A
        // half-step link spans sqrt(5) tiles; billing it as one would make
        // shallow runs 2.24x faster than the geometry allows.
        let leg_sq = loc
            .path
            .get(loc.path_index + 1)
            .and_then(|next| network.piece(*next))
            .map(|next| {
                let dx = (next.tile.x - piece.tile.x) as i64;
                let dy = (next.tile.y - piece.tile.y) as i64;
                (dx * dx + dy * dy) as u32
            })
            .unwrap_or(1);
        let needed = profile.ticks_for_leg(piece.max_grade, piece.curve, leg_sq);
        loc.progress = loc.progress.saturating_add(1);
        if loc.progress < needed {
            continue;
        }

        let held = occupancy.held_ticks(train.id);
        let tick = occupancy.tick;
        let mut moved = false;
        let mut blocker: Option<TrainId> = None;

        // Two attempts: the route as planned, then the way round if we found one.
        for attempt in 0..2u8 {
            let Some(&next) = loc.path.get(loc.path_index + 1) else {
                break;
            };
            // Freight (etc.) refuse tiles steeper than profile max.
            let climbable = network
                .piece(next)
                .is_some_and(|p| profile.tolerates_grade(p.max_grade));
            // Occupied by another train → hold and record the blocker.
            blocker = occupancy
                .by_track
                .get(&next)
                .copied()
                .filter(|&other| other != train.id);

            if blocker.is_none() && climbable {
                occupancy.by_track.remove(&loc.track);
                occupancy.by_track.insert(next, train.id);
                occupancy.last_crossed.insert(next, tick);
                loc.track = next;
                loc.path_index += 1;
                loc.progress = 0;
                moved = true;
                break;
            }
            if attempt > 0 {
                break;
            }
            match way_round(
                &network,
                &occupancy,
                &intent,
                train,
                &loc,
                blocker,
                held.saturating_add(1),
            ) {
                Some(Way::Reroute(route)) => {
                    loc.set_route_ahead(route);
                    occupancy.rerouted.push(train.id);
                }
                Some(Way::Yield(route)) => {
                    loc.set_route_ahead(route);
                    occupancy.yielded.push(train.id);
                    occupancy
                        .yield_cooldown
                        .insert(train.id, YIELD_COOLDOWN_TICKS);
                }
                None => break,
            }
        }

        if moved {
            occupancy.held.remove(&train.id);
        } else {
            loc.progress = needed.saturating_sub(1);
            occupancy.held.insert(train.id, held.saturating_add(1));
            if let Some(other) = blocker {
                occupancy.blocked_by.insert(train.id, other);
            }
        }
    }

    if occupancy.tick % POLISH_PRUNE_EVERY == 0 {
        let tick = occupancy.tick;
        occupancy
            .last_crossed
            .retain(|_, last| tick.saturating_sub(*last) < POLISH_MEMORY_TICKS);
    }
}

/// Grade / curve slow trains using the kind's [`TrainProfile`].
pub fn ticks_for_piece(kind: crate::commands::TrainKind, max_grade: u8, curve: u8) -> u16 {
    ticks_for_consist_piece(kind, 1, max_grade, curve)
}

/// The same, for a train of `cars` cars.
///
/// The presentation interpolates a train's position against this number, so a
/// consist that the sim moves slower and the sprite moves at single-car pace
/// would arrive at the next tile early and snap back. One function, both
/// callers.
pub fn ticks_for_consist_piece(
    kind: crate::commands::TrainKind,
    cars: u8,
    max_grade: u8,
    curve: u8,
) -> u16 {
    TrainProfile::for_kind(kind)
        .for_consist(cars)
        .ticks_for_piece(max_grade, curve)
}

/// Blocker id for a waiting train, if any.
pub fn blocker_for(occupancy: &TileOccupancy, train: TrainId) -> Option<TrainId> {
    occupancy.blocked_by.get(&train).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::TrainKind;
    use crate::economy::MoneyLedger;
    use crate::ids::TileCoord;
    use crate::money::Money;
    use crate::track::{try_place_track, TrackTerrain, GROUND_LAYER};
    use crate::trains::congestion::blocked_chain_head;
    use crate::trains::find_path_for_kind;
    use bevy_app::{App, Update};

    fn land(w: u32, h: u32) -> TrackTerrain {
        TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 0i8)))
    }

    /// Lay track and return ids in the order the tiles were given.
    fn lay(
        network: &mut TrackNetwork,
        terrain: &TrackTerrain,
        tiles: &[(i32, i32)],
    ) -> Vec<TrackId> {
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

    struct Sim {
        app: App,
        /// Ticks on which some train stepped into a passing loop.
        yields: u32,
        /// Ticks on which some train took an alternative route.
        reroutes: u32,
        /// Every tile any train stood on.
        visited: HashSet<TrackId>,
    }

    impl Sim {
        fn new(network: TrackNetwork) -> Self {
            let mut app = App::new();
            app.init_resource::<TileOccupancy>()
                .init_resource::<TrainYard>()
                .insert_resource(network)
                .add_systems(Update, advance_trains);
            Self {
                app,
                yields: 0,
                reroutes: 0,
                visited: HashSet::new(),
            }
        }

        fn spawn(&mut self, id: u64, kind: TrainKind, path: Vec<TrackId>) -> TrainId {
            let id = TrainId(id);
            let mut loc = TrainLocation::at_track(path[0]);
            loc.set_path(path);
            self.app.world_mut().spawn((Train { id, kind }, loc));
            id
        }

        /// Park a train where it stands so it blocks the line without moving.
        fn park(&mut self, id: TrainId) {
            let mut q = self.app.world_mut().query::<(&Train, &mut TrainLocation)>();
            let world = self.app.world_mut();
            for (train, mut loc) in q.iter_mut(world) {
                if train.id == id {
                    loc.parked = true;
                }
            }
        }

        fn run(&mut self, ticks: u32) {
            for _ in 0..ticks {
                self.app.world_mut().run_schedule(Update);
                let occ = self.app.world().resource::<TileOccupancy>();
                self.yields += !occ.yielded.is_empty() as u32;
                self.reroutes += !occ.rerouted.is_empty() as u32;
                self.visited.extend(occ.by_track.keys().copied());
            }
        }

        fn at(&mut self, id: TrainId) -> TrackId {
            let mut q = self.app.world_mut().query::<(&Train, &TrainLocation)>();
            q.iter(self.app.world())
                .find(|(t, _)| t.id == id)
                .map(|(_, loc)| loc.track)
                .expect("train exists")
        }

        fn arrived(&mut self, id: TrainId) -> bool {
            let mut q = self.app.world_mut().query::<(&Train, &TrainLocation)>();
            q.iter(self.app.world())
                .find(|(t, _)| t.id == id)
                .map(|(_, loc)| loc.at_destination())
                .unwrap_or(false)
        }

        fn occupancy(&self) -> &TileOccupancy {
            self.app.world().resource::<TileOccupancy>()
        }
    }

    /// The orphaned-train bug: a train whose tile the network has forgotten.
    ///
    /// It never moves, cannot be routed, and there is no sell command — so
    /// without a recall it stands on open ground forever. Both ways a train can
    /// get there are covered: the track lifted out from under it, and a border
    /// crossing landing on a `TrackId` that has since been demolished.
    #[test]
    fn a_train_whose_track_vanished_goes_back_to_the_yard() {
        let terrain = land(12, 6);
        let mut network = TrackNetwork::new();
        let main = lay(&mut network, &terrain, &[(1, 2), (2, 2), (3, 2)]);
        let ghost = TrackId(9_999);

        let mut sim = Sim::new(network);
        let running = sim.spawn(1, TrainKind::Transit, main.clone());
        // Landed from a border crossing onto a railhead that no longer exists.
        let stranded = sim.spawn(2, TrainKind::Transport, vec![ghost]);
        sim.run(1);

        let live: Vec<TrainId> = {
            let mut q = sim.app.world_mut().query::<&Train>();
            q.iter(sim.app.world()).map(|t| t.id).collect()
        };
        assert_eq!(
            live,
            vec![running],
            "a train with no tile under it must not survive the tick"
        );
        assert_eq!(
            sim.app.world().resource::<TrainYard>().unplaced(),
            &[(stranded, TrainKind::Transport)],
            "the player paid for it: recall it to the yard, do not delete it"
        );
        // The line it was never on is unaffected.
        sim.run(40);
        assert_eq!(sim.at(running), *main.last().unwrap());
    }

    #[test]
    fn a_healthy_railway_never_touches_the_yard() {
        // The recall must not cost a `TrainYard` change every tick: anything
        // that watches the yard would then rebuild forever. (A live FPS
        // regression was exactly this shape — see the note in `advance_trains`.)
        let terrain = land(12, 6);
        let mut network = TrackNetwork::new();
        let main = lay(&mut network, &terrain, &[(1, 2), (2, 2), (3, 2)]);

        let mut app = App::new();
        app.init_resource::<TileOccupancy>()
            .init_resource::<TrainYard>()
            .insert_resource(network)
            .add_systems(Update, advance_trains);
        let mut loc = TrainLocation::at_track(main[0]);
        loc.set_path(main);
        app.world_mut().spawn((
            Train {
                id: TrainId(1),
                kind: TrainKind::Transit,
            },
            loc,
        ));

        app.update();
        let baseline = app.world().resource_ref::<TrainYard>().last_changed();
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(
            app.world().resource_ref::<TrainYard>().last_changed(),
            baseline,
            "an untouched yard must not report itself changed"
        );
    }

    #[test]
    fn profile_tick_helper_matches() {
        assert_eq!(ticks_for_piece(TrainKind::Transit, 0, 0), 6);
        assert_eq!(ticks_for_piece(TrainKind::Transport, 0, 0), 10);
        assert!(
            ticks_for_piece(TrainKind::Transport, 1, 0) > ticks_for_piece(TrainKind::Transit, 1, 0)
        );
    }

    /// **The speed claim, measured on a running line** rather than read off the
    /// profile: brief 17 §4 says ten tiles is ten sim-minutes and about one real
    /// second of watching at 1x.
    #[test]
    fn ten_tiles_takes_ten_sim_minutes_of_running() {
        let terrain = land(16, 6);
        let mut network = TrackNetwork::new();
        let tiles: Vec<(i32, i32)> = (1..=11).map(|x| (x, 3)).collect();
        let main = lay(&mut network, &terrain, &tiles);

        let mut sim = Sim::new(network);
        let a = sim.spawn(1, TrainKind::Transit, main.clone());

        let mut ticks = 0u32;
        while !sim.arrived(a) && ticks < 10_000 {
            sim.run(1);
            ticks += 1;
        }

        // Eleven tiles of track is ten steps between them.
        assert_eq!(ticks, 60, "ten tiles should take sixty ticks of running");
        let sim_minutes = ticks * crate::peeps::SIM_SECONDS_PER_TICK / 60;
        assert_eq!(sim_minutes, 10, "ten tiles, ten sim-minutes");
        assert!(
            (0.9..=1.0).contains(&(f64::from(ticks) / 64.0)),
            "and about a real second at 1x, got {}",
            f64::from(ticks) / 64.0
        );
    }

    #[test]
    fn single_train_runs_the_line() {
        let terrain = land(12, 6);
        let mut network = TrackNetwork::new();
        let main = lay(
            &mut network,
            &terrain,
            &[(1, 2), (2, 2), (3, 2), (4, 2), (5, 2)],
        );
        let mut sim = Sim::new(network);
        let a = sim.spawn(1, TrainKind::Transit, main.clone());
        sim.run(60);
        assert_eq!(sim.at(a), *main.last().unwrap());
    }

    /// A siding beside a single line lets a head-on pair through. This is the
    /// regression the burn-down called "passing loops deferred".
    #[test]
    fn passing_loop_resolves_a_head_on_standoff() {
        let terrain = land(16, 8);
        let mut network = TrackNetwork::new();
        // Single line y=3 from x=1..=8, with a one-tile loop above x=4.
        let main = lay(
            &mut network,
            &terrain,
            &[
                (1, 3),
                (2, 3),
                (3, 3),
                (4, 3),
                (5, 3),
                (6, 3),
                (7, 3),
                (8, 3),
            ],
        );
        let loop_tile = lay(&mut network, &terrain, &[(4, 4)])[0];

        let mut sim = Sim::new(network);
        let east: Vec<TrackId> = main.clone();
        let west: Vec<TrackId> = main.iter().rev().copied().collect();
        let a = sim.spawn(1, TrainKind::Transit, east);
        let b = sim.spawn(2, TrainKind::Transit, west);

        sim.run(600);

        assert!(
            sim.visited.contains(&loop_tile),
            "the loop the player paid for must actually be run over"
        );
        assert!(sim.arrived(a), "eastbound should reach the far end");
        assert!(sim.arrived(b), "westbound should reach the far end");
        assert_eq!(sim.at(a), main[main.len() - 1]);
        assert_eq!(sim.at(b), main[0]);
    }

    /// A dead-end siding is not a bypass, so there is no route round: one train
    /// has to physically step aside and wait. This is the yield path.
    #[test]
    fn dead_end_siding_lets_one_train_stand_aside() {
        let terrain = land(12, 12);
        let mut network = TrackNetwork::new();
        // Diagonal single line, plus a stub touching only the (4,4) tile.
        let main = lay(
            &mut network,
            &terrain,
            &[(1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (6, 6)],
        );
        let stub = lay(&mut network, &terrain, &[(5, 3)])[0];
        assert_eq!(
            network.neighbor_ids(stub),
            vec![main[3]],
            "stub must be a true dead end, not a bypass"
        );

        let mut sim = Sim::new(network);
        let a = sim.spawn(1, TrainKind::Transit, main.clone());
        let b = sim.spawn(2, TrainKind::Transit, main.iter().rev().copied().collect());

        sim.run(900);

        assert!(sim.yields > 0, "one of the pair must stand in the siding");
        assert!(sim.visited.contains(&stub), "the siding gets used");
        assert!(sim.arrived(a), "eastbound clears the standoff");
        assert!(sim.arrived(b), "westbound clears the standoff");
        assert_eq!(sim.at(a), main[main.len() - 1]);
        assert_eq!(sim.at(b), main[0]);
    }

    /// The same standoff without a loop: nobody passes. This is what the loop
    /// is buying, and it keeps the fallback honest (wait, never teleport).
    #[test]
    fn head_on_without_a_loop_still_just_waits() {
        let terrain = land(16, 8);
        let mut network = TrackNetwork::new();
        let main = lay(
            &mut network,
            &terrain,
            &[(1, 3), (2, 3), (3, 3), (4, 3), (5, 3), (6, 3), (7, 3), (8, 3)],
        );

        let mut sim = Sim::new(network);
        let a = sim.spawn(1, TrainKind::Transit, main.clone());
        let b = sim.spawn(2, TrainKind::Transit, main.iter().rev().copied().collect());

        sim.run(600);

        assert_eq!(sim.yields, 0, "no loop to step into");
        assert!(!sim.arrived(a) && !sim.arrived(b), "single track holds both");
        assert!(sim.occupancy().is_blocked(a) || sim.occupancy().is_blocked(b));
    }

    /// Two running lines on the same corridor: opposite directions stop fighting.
    #[test]
    fn double_track_corridor_carries_both_directions() {
        let terrain = land(16, 8);
        let mut network = TrackNetwork::new();
        let up: Vec<(i32, i32)> = (1..=8).map(|x| (x, 3)).collect();
        let down: Vec<(i32, i32)> = (1..=8).map(|x| (x, 4)).collect();
        let up_ids = lay(&mut network, &terrain, &up);
        let down_ids = lay(&mut network, &terrain, &down);

        let mut sim = Sim::new(network);
        // Both trains are *routed down the same line*; the sim must discover the
        // parallel one rather than deadlock.
        let a = sim.spawn(1, TrainKind::Transit, up_ids.clone());
        let b = sim.spawn(2, TrainKind::Transit, {
            let mut p = vec![*down_ids.last().unwrap()];
            p.extend(up_ids.iter().rev().skip(1).copied());
            p
        });

        sim.run(600);

        assert!(
            sim.reroutes > 0,
            "the second running line should be discovered"
        );
        assert!(sim.arrived(a), "eastbound should arrive on a double corridor");
        assert!(sim.arrived(b), "westbound should arrive on a double corridor");
    }

    /// §4.4 — the player paid for a second way round; it must save them.
    #[test]
    fn reroutes_around_a_stalled_train_when_slack_exists() {
        let terrain = land(16, 10);
        let mut network = TrackNetwork::new();
        // Main line y=4 x=1..=7, plus a longer loop over the top through y=2.
        let main = lay(
            &mut network,
            &terrain,
            &[(1, 4), (2, 4), (3, 4), (4, 4), (5, 4), (6, 4), (7, 4)],
        );
        let _detour = lay(
            &mut network,
            &terrain,
            &[
                (2, 3),
                (2, 2),
                (3, 2),
                (4, 2),
                (5, 2),
                (6, 2),
                (6, 3),
            ],
        );

        let mut sim = Sim::new(network);
        // A parked train sits on the middle of the main line and never moves.
        let blocker = sim.spawn(9, TrainKind::Transit, vec![main[4]]);
        sim.park(blocker);
        let runner = sim.spawn(1, TrainKind::Transit, main.clone());

        sim.run(900);

        assert!(sim.reroutes > 0, "slack must be spent, not admired");
        assert!(
            sim.arrived(runner),
            "the second way round should get the train home"
        );
        assert_eq!(sim.at(runner), main[main.len() - 1]);
        // It really went round rather than through the parked train.
        assert_eq!(sim.at(blocker), main[4]);
    }

    /// No slack anywhere: hold and name the blocker, exactly as before.
    #[test]
    fn no_slack_still_only_waits_and_records_the_blocker() {
        let terrain = land(12, 6);
        let mut network = TrackNetwork::new();
        let main = lay(&mut network, &terrain, &[(1, 2), (2, 2), (3, 2), (4, 2)]);

        let mut sim = Sim::new(network);
        let stuck = sim.spawn(9, TrainKind::Transit, vec![main[3]]);
        sim.park(stuck);
        let follower = sim.spawn(1, TrainKind::Transit, main.clone());

        sim.run(200);

        assert_eq!(sim.at(follower), main[2], "should queue behind the blockage");
        assert_eq!(blocker_for(sim.occupancy(), follower), Some(stuck));
        assert!(sim.occupancy().held_ticks(follower) > 0);
    }

    /// A queue reads as a queue: every waiting train names the one ahead, and
    /// the chain walks back to the cause.
    #[test]
    fn a_queue_chains_back_to_its_cause() {
        let terrain = land(14, 6);
        let mut network = TrackNetwork::new();
        let main = lay(
            &mut network,
            &terrain,
            &[(1, 2), (2, 2), (3, 2), (4, 2), (5, 2), (6, 2)],
        );

        let mut sim = Sim::new(network);
        let head = sim.spawn(9, TrainKind::Transit, vec![main[5]]);
        sim.park(head);
        let first = sim.spawn(1, TrainKind::Transit, main.clone());
        let second = sim.spawn(2, TrainKind::Transit, main.clone());

        sim.run(200);

        assert_eq!(blocked_chain_head(sim.occupancy(), second), Some(head));
        assert_eq!(blocker_for(sim.occupancy(), first), Some(head));
    }

    /// Railhead polish (`01-art-direction.md` §5.3) needs to know what was used.
    #[test]
    fn crossings_are_remembered_for_railhead_polish() {
        let terrain = land(12, 6);
        let mut network = TrackNetwork::new();
        let main = lay(&mut network, &terrain, &[(1, 2), (2, 2), (3, 2), (4, 2)]);
        let branch = lay(&mut network, &terrain, &[(1, 0)]);

        let mut sim = Sim::new(network);
        let _ = sim.spawn(1, TrainKind::Transit, main.clone());
        sim.run(30);

        let occ = sim.occupancy();
        assert!(
            occ.ticks_since_crossed(main[3]).is_some(),
            "the run should mark the main line"
        );
        assert!(
            occ.ticks_since_crossed(branch[0]).is_none(),
            "a branch nobody runs stays dull"
        );
    }

    #[test]
    fn service_order_is_deterministic_across_runs() {
        let terrain = land(16, 8);
        let build = || {
            let mut network = TrackNetwork::new();
            let ids = lay(
                &mut network,
                &terrain,
                &[(1, 3), (2, 3), (3, 3), (4, 3), (5, 3), (6, 3)],
            );
            let loops = lay(&mut network, &terrain, &[(3, 4), (4, 4)]);
            (network, ids, loops)
        };
        let mut seen = Vec::new();
        for _ in 0..3 {
            let (network, ids, _) = build();
            let mut sim = Sim::new(network);
            let a = sim.spawn(1, TrainKind::Transit, ids.clone());
            let b = sim.spawn(2, TrainKind::Transit, ids.iter().rev().copied().collect());
            sim.run(120);
            seen.push((sim.at(a), sim.at(b)));
        }
        assert!(
            seen.windows(2).all(|w| w[0] == w[1]),
            "same inputs must give the same positions: {seen:?}"
        );
    }

    #[test]
    fn find_path_avoiding_prefers_the_parallel_line() {
        let terrain = land(16, 8);
        let mut network = TrackNetwork::new();
        let up = lay(
            &mut network,
            &terrain,
            &[(1, 3), (2, 3), (3, 3), (4, 3), (5, 3)],
        );
        let down = lay(
            &mut network,
            &terrain,
            &[(1, 4), (2, 4), (3, 4), (4, 4), (5, 4)],
        );
        let mut busy = HashSet::new();
        busy.insert(up[2]);
        let route = crate::trains::find_path_avoiding(
            &network,
            up[0],
            *up.last().unwrap(),
            TrainKind::Transit,
            &busy,
        )
        .expect("parallel line is a way round");
        assert!(!route.contains(&up[2]), "must not run through the blockage");
        assert!(
            route.iter().any(|id| down.contains(id)),
            "should use the second running line"
        );
        // Sanity: the direct route exists when nothing is in the way.
        let direct =
            find_path_for_kind(&network, up[0], *up.last().unwrap(), TrainKind::Transit).unwrap();
        assert!(direct.len() <= route.len());
    }
}
