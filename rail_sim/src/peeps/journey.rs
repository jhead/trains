//! The journey state machine — peeps actually make journeys.
//!
//! Brief 06 §4.1:
//!
//! ```text
//!   home  →  walk to station  →  wait on platform  →  board
//!         →  ride  →  alight  →  walk to destination  →  spend time  →  return
//! ```
//!
//! *"Every stage is visible."* — so every stage is a public variant of
//! [`JourneyStage`], every peep carries a real [`PeepPosition`] in fractional
//! tile space, and the Inspector can read the whole thing without asking the
//! renderer anything. Positions stay fractional in the sim and are rounded to
//! whole texels at draw time, which is what the pixel contract (art 01 §2.1)
//! requires.

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::commands::TrainKind;
use crate::ids::{StationId, TileCoord, TrackId, TrainId};
use crate::stations::{StationRegistry, StationService};
use crate::track::{TrackNetwork, TrackTerrain};
use crate::trains::{track_for_station, Train, TrainLocation};

use super::budget::PeepDetail;
use super::complaints::{gave_up_minutes, ComplaintEntry, ComplaintFeed, TalkKind};
use super::flow::DistrictFlow;
use super::memory::{outcome_for, JourneyMemory, JourneyRecord};
use super::names::hash64;
use super::resident::{Peep, WaitingAtStation, SIM_SECONDS_PER_TICK};
use super::routine::{clock_label, Routine, DAY_MINUTES};
use super::walk::{walk_step, WalkRoute, WalkRouter, WalkStep, WalkWorld};
use super::{day_index, minute_of_day};

/// Ticks a peep takes to walk one tile. Transit crosses a tile in 3 ticks, so
/// walking is deliberately eight times slower — giving up and walking has to
/// *feel* like a worse deal, not just score like one.
pub const WALK_TICKS_PER_TILE: u32 = 24;

/// Tiles covered per Advance tick while walking.
pub const WALK_TILES_PER_TICK: f32 = 1.0 / WALK_TICKS_PER_TILE as f32;

/// Ticks between walk-cycle frames (2-frame cycle, art 01 §7).
pub const STEP_FRAME_TICKS: u32 = 8;

/// Ticks spent visibly boarding / alighting.
pub const BOARD_TICKS: u32 = 3;

/// How close (in tiles) counts as arrived.
pub const ARRIVE_EPSILON: f32 = 0.06;

/// Facing for sprite selection — four directions, chosen never rotated (art 01 §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Facing {
    North,
    South,
    East,
    West,
}

impl Default for Facing {
    fn default() -> Self {
        Self::South
    }
}

impl Facing {
    pub const ALL: [Facing; 4] = [Self::North, Self::South, Self::East, Self::West];

    /// Sprite-bank row index.
    pub fn index(self) -> usize {
        match self {
            Self::North => 0,
            Self::South => 1,
            Self::East => 2,
            Self::West => 3,
        }
    }

    /// Facing implied by a movement delta; `None` when standing still.
    pub fn from_delta(dx: f32, dy: f32) -> Option<Self> {
        if dx.abs() < f32::EPSILON && dy.abs() < f32::EPSILON {
            return None;
        }
        if dx.abs() >= dy.abs() {
            Some(if dx >= 0.0 { Self::East } else { Self::West })
        } else {
            Some(if dy >= 0.0 { Self::North } else { Self::South })
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::North => "north",
            Self::South => "south",
            Self::East => "east",
            Self::West => "west",
        }
    }
}

/// Where a peep physically is, in fractional tile space.
///
/// Whole values sit at a tile centre; the renderer converts with
/// `(v + 0.5) * TILE_SIZE` and rounds to a whole texel.
#[derive(Component, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PeepPosition {
    pub x: f32,
    pub y: f32,
    pub facing: Facing,
    /// Which of the two walk frames to draw (`0` or `1`).
    pub step: u8,
    /// True while actually moving — the walk cycle only runs then.
    pub walking: bool,
    /// Hashed sub-tile offset so housemates and a platform crowd don't stack.
    pub jitter_x: f32,
    pub jitter_y: f32,
    step_ticks: u32,
}

impl PeepPosition {
    /// Place a peep on a tile, with a stable per-peep sub-tile offset.
    pub fn at_tile(tile: TileCoord, seed: u64) -> Self {
        let h = hash64(seed);
        let jitter_x = ((h % 21) as f32 - 10.0) / 40.0; // ±0.25 tile
        let jitter_y = (((h >> 8) % 21) as f32 - 10.0) / 40.0;
        Self {
            x: tile.x as f32 + jitter_x,
            y: tile.y as f32 + jitter_y,
            facing: Facing::South,
            step: 0,
            walking: false,
            jitter_x,
            jitter_y,
            step_ticks: 0,
        }
    }

    /// Tile the peep is standing on.
    pub fn tile(&self) -> TileCoord {
        TileCoord {
            x: self.x.round() as i32,
            y: self.y.round() as i32,
        }
    }

    /// Snap to a tile without changing facing.
    pub fn snap_to(&mut self, tile: TileCoord) {
        self.x = tile.x as f32 + self.jitter_x;
        self.y = tile.y as f32 + self.jitter_y;
    }

    pub fn stand_still(&mut self) {
        self.walking = false;
    }

    /// Step toward a tile at walking pace. Returns `true` once arrived.
    pub fn walk_toward(&mut self, target: TileCoord, tiles_per_tick: f32) -> bool {
        let tx = target.x as f32 + self.jitter_x;
        let ty = target.y as f32 + self.jitter_y;
        let dx = tx - self.x;
        let dy = ty - self.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist <= ARRIVE_EPSILON.max(tiles_per_tick) {
            self.x = tx;
            self.y = ty;
            self.walking = false;
            return true;
        }
        let inv = tiles_per_tick / dist;
        self.x += dx * inv;
        self.y += dy * inv;
        if let Some(f) = Facing::from_delta(dx, dy) {
            self.facing = f;
        }
        self.walking = true;
        self.tick_walk_cycle();
        false
    }

    /// Keep the walk cycle running across a route corner.
    ///
    /// [`Self::walk_toward`] stops the cycle on the tick it reaches its target,
    /// which is right at the end of a walk and wrong in the middle of one — a
    /// peep turning a corner should not stutter to a halt for a tick.
    pub fn keep_walking(&mut self) {
        self.walking = true;
        self.tick_walk_cycle();
    }

    fn tick_walk_cycle(&mut self) {
        self.step_ticks = self.step_ticks.saturating_add(1);
        if self.step_ticks >= STEP_FRAME_TICKS {
            self.step_ticks = 0;
            self.step ^= 1;
        }
    }
}

/// Which half of the round trip a peep is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JourneyLeg {
    /// Home → destination.
    Outbound,
    /// Destination → home.
    Return,
}

impl JourneyLeg {
    pub fn label(self) -> &'static str {
        match self {
            Self::Outbound => "out",
            Self::Return => "home",
        }
    }
}

/// Every stage of the brief's journey, each one observable from outside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JourneyStage {
    /// Between trips, waiting for their habitual departure time.
    AtHome,
    /// On the lane, heading for the platform.
    WalkingToStation,
    /// Standing on the platform, wait accumulating.
    WaitingOnPlatform,
    /// Stepping onto a train that is going their way.
    Boarding,
    /// Aboard — position follows the train.
    Riding,
    /// Stepping off at the far end.
    Alighting,
    /// Walking the last stretch to where they were going.
    WalkingToDestination,
    /// Doing whatever they came to do.
    SpendingTime,
    /// Gave up on the platform and is walking the whole way (§4.3).
    WalkingInstead,
    /// Packed up and heading out of town for good (§3.2).
    LeavingTown,
}

impl JourneyStage {
    /// Plain-language stage name for the Peep card.
    pub fn label(self) -> &'static str {
        match self {
            Self::AtHome => "At home",
            Self::WalkingToStation => "Walking to the station",
            Self::WaitingOnPlatform => "Waiting on the platform",
            Self::Boarding => "Boarding",
            Self::Riding => "Riding",
            Self::Alighting => "Getting off",
            Self::WalkingToDestination => "Walking to the destination",
            Self::SpendingTime => "Spending time",
            Self::WalkingInstead => "Gave up - walking",
            Self::LeavingTown => "Leaving town",
        }
    }

    pub fn is_walking(self) -> bool {
        matches!(
            self,
            Self::WalkingToStation
                | Self::WalkingToDestination
                | Self::WalkingInstead
                | Self::LeavingTown
        )
    }

    /// True while the peep is on a platform (counts toward station pressure).
    pub fn is_waiting(self) -> bool {
        matches!(self, Self::WaitingOnPlatform)
    }

    /// True when the peep is out of the house.
    pub fn is_travelling(self) -> bool {
        !matches!(self, Self::AtHome)
    }

    /// Riding peeps are inside a carriage — the renderer hides them.
    pub fn is_visible(self) -> bool {
        !matches!(self, Self::Riding)
    }

    /// The coarse stage an abstracted peep collapses to.
    ///
    /// [`Self::LeavingTown`] survives coarsening: a household that has decided
    /// to go must still finish going even while off camera.
    pub fn coarse(self) -> Self {
        match self {
            Self::AtHome => Self::AtHome,
            Self::LeavingTown => Self::LeavingTown,
            Self::WalkingToStation
            | Self::WaitingOnPlatform
            | Self::Boarding
            | Self::Riding
            | Self::Alighting => Self::WaitingOnPlatform,
            Self::WalkingToDestination | Self::SpendingTime | Self::WalkingInstead => {
                Self::SpendingTime
            }
        }
    }
}

/// A peep's current trip. Public in full — the Inspector reads this directly.
#[derive(Component, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Journey {
    pub stage: JourneyStage,
    pub leg: JourneyLeg,
    /// Tile the peep is heading for while walking.
    pub target: TileCoord,
    /// Ticks spent in the current stage.
    pub stage_ticks: u32,
    /// Sim-seconds elapsed on this leg, door to door.
    pub leg_secs: u32,
    /// Platform wait carried into the ride, so the finished leg can be graded.
    pub leg_wait_secs: u32,
    /// Train currently carrying this peep.
    pub riding: Option<TrainId>,
    /// Day index of the last outbound departure — one commute per day.
    pub last_depart_day: Option<u64>,
    /// Station they board at on this leg.
    pub from_station: StationId,
    /// Station they alight at on this leg.
    pub to_station: StationId,
    /// Set when the peep abandoned the platform on this leg.
    pub gave_up: bool,
}

impl Journey {
    pub fn new(routine: &Routine) -> Self {
        Self {
            stage: JourneyStage::AtHome,
            leg: JourneyLeg::Outbound,
            target: routine.home,
            stage_ticks: 0,
            leg_secs: 0,
            leg_wait_secs: 0,
            riding: None,
            last_depart_day: None,
            from_station: routine.home_station,
            to_station: routine.destination_station,
            gave_up: false,
        }
    }

    pub fn set_stage(&mut self, stage: JourneyStage) {
        if self.stage != stage {
            self.stage = stage;
            self.stage_ticks = 0;
        }
    }

    /// Stage duration in sim-seconds.
    pub fn stage_secs(&self) -> u32 {
        self.stage_ticks.saturating_mul(SIM_SECONDS_PER_TICK)
    }

    /// Where to consider this peep for level-of-detail ranking.
    pub fn anchor_tile(
        &self,
        home: TileCoord,
        destination: TileCoord,
        from_station_tile: Option<TileCoord>,
    ) -> TileCoord {
        match self.stage {
            JourneyStage::AtHome | JourneyStage::LeavingTown => home,
            JourneyStage::WalkingToStation
            | JourneyStage::WaitingOnPlatform
            | JourneyStage::Boarding
            | JourneyStage::Riding
            | JourneyStage::Alighting => from_station_tile.unwrap_or(home),
            JourneyStage::WalkingToDestination
            | JourneyStage::SpendingTime
            | JourneyStage::WalkingInstead => destination,
        }
    }

    /// One-line "where they are going right now" for the Peep card (05 §3.3).
    pub fn describe(&self, from_name: &str, to_name: &str) -> String {
        match self.stage {
            JourneyStage::AtHome => "At home.".into(),
            JourneyStage::WalkingToStation => format!("Walking to {from_name}."),
            JourneyStage::WaitingOnPlatform => {
                format!("Waiting at {from_name} for the {to_name} train.")
            }
            JourneyStage::Boarding => format!("Boarding at {from_name}."),
            JourneyStage::Riding => format!("Riding to {to_name}."),
            JourneyStage::Alighting => format!("Getting off at {to_name}."),
            JourneyStage::WalkingToDestination => match self.leg {
                JourneyLeg::Outbound => format!("Walking from {to_name} to where they're going."),
                JourneyLeg::Return => "Walking the last stretch home.".into(),
            },
            JourneyStage::SpendingTime => format!("Spending the day near {to_name}."),
            JourneyStage::WalkingInstead => match self.leg {
                JourneyLeg::Outbound => format!("Gave up at {from_name} - walking instead."),
                JourneyLeg::Return => format!("Gave up at {from_name} - walking home."),
            },
            JourneyStage::LeavingTown => format!("Leaving {from_name} for good."),
        }
    }

    /// True when the sprite layer should draw this peep.
    pub fn is_visible(&self) -> bool {
        self.stage.is_visible()
    }

    /// Collapse to the coarse stage set used by abstracted peeps.
    pub fn coarsen(&mut self) {
        let coarse = self.stage.coarse();
        if coarse != self.stage {
            self.riding = None;
            self.set_stage(coarse);
        }
    }

    /// Start a fresh leg between two stations, clearing the per-leg counters.
    pub fn begin_leg(&mut self, leg: JourneyLeg, from: StationId, to: StationId) {
        self.leg = leg;
        self.from_station = from;
        self.to_station = to;
        self.leg_secs = 0;
        self.leg_wait_secs = 0;
        self.gave_up = false;
        self.riding = None;
    }
}

/// Track tile a station's trains stop on.
pub fn station_track(
    network: &TrackNetwork,
    stations: &StationRegistry,
    id: StationId,
) -> Option<TrackId> {
    let s = stations.get(id)?;
    track_for_station(network, s.tile, s.layer)
}

/// A transit train standing at `from` whose remaining path reaches `to`.
///
/// This is how a peep decides to board: *is this train going where I'm going?*
pub fn boardable_train(
    network: &TrackNetwork,
    stations: &StationRegistry,
    trains: &Query<(&Train, &TrainLocation)>,
    from: StationId,
    to: StationId,
) -> Option<TrainId> {
    let from_track = station_track(network, stations, from)?;
    let to_track = station_track(network, stations, to)?;
    for (train, loc) in trains.iter() {
        if train.kind != TrainKind::Transit || loc.parked {
            continue;
        }
        if loc.track != from_track {
            continue;
        }
        if loc.path[loc.path_index..].contains(&to_track) {
            return Some(train.id);
        }
    }
    None
}

/// Advance the journey state machine for every full-detail peep.
///
/// Walking is terrain-aware: every walked stage follows a cached [`WalkRoute`]
/// over walkable ground (see [`super::walk`]), so nobody crosses water or a
/// cliff face. Only full-detail peeps have positions, so only they ever need a
/// route; the abstracted majority never touch this system.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn advance_journeys(
    stations: Res<StationRegistry>,
    service: Res<StationService>,
    network: Res<TrackNetwork>,
    terrain: Option<Res<TrackTerrain>>,
    trains: Query<(&Train, &TrainLocation)>,
    mut flow: ResMut<DistrictFlow>,
    mut feed: ResMut<ComplaintFeed>,
    mut router: ResMut<WalkRouter>,
    mut peeps: Query<(
        &Peep,
        &Routine,
        &mut Journey,
        &mut PeepPosition,
        &mut WaitingAtStation,
        &mut JourneyMemory,
        &PeepDetail,
        Option<&mut WalkRoute>,
    )>,
) {
    let tick = service.tick;
    let minute = minute_of_day(tick);
    let today = day_index(tick);

    // The town gets a fixed number of route searches per tick; a peep who
    // misses out stands on the doorstep for a tick and asks again.
    router.begin_tick();
    let world = terrain
        .as_deref()
        .map(|t| WalkWorld::new(t, Some(network.as_ref())));

    for (peep, routine, mut journey, mut pos, mut waiting, mut memory, detail, mut route) in
        peeps.iter_mut()
    {
        if !detail.is_full() {
            continue;
        }
        journey.stage_ticks = journey.stage_ticks.saturating_add(1);
        if journey.stage.is_travelling() {
            journey.leg_secs = journey.leg_secs.saturating_add(SIM_SECONDS_PER_TICK);
        }

        match journey.stage {
            JourneyStage::AtHome => {
                pos.stand_still();
                let already_went = journey.last_depart_day == Some(today);
                if !already_went && routine.is_departure_time(minute) {
                    journey.last_depart_day = Some(today);
                    journey.begin_leg(
                        JourneyLeg::Outbound,
                        routine.home_station,
                        routine.destination_station,
                    );
                    waiting.station = routine.home_station;
                    waiting.wait_secs = 0;
                    flow.entry(routine.home_station).departures += 1;

                    if routine.home_station == routine.destination_station {
                        // Same district — this trip was always a walk.
                        journey.target = routine.destination;
                        journey.set_stage(JourneyStage::WalkingToDestination);
                    } else {
                        journey.target = stations
                            .get(routine.home_station)
                            .map(|s| s.tile)
                            .unwrap_or(routine.home);
                        journey.set_stage(JourneyStage::WalkingToStation);
                    }
                }
            }

            JourneyStage::WalkingToStation => {
                let target = journey.target;
                let step = walk_step(
                    route.as_deref_mut(),
                    &mut pos,
                    target,
                    WALK_TILES_PER_TICK,
                    world.as_ref(),
                    &mut router,
                );
                match step {
                    WalkStep::Arrived => {
                        waiting.station = journey.from_station;
                        waiting.wait_secs = 0;
                        journey.set_stage(JourneyStage::WaitingOnPlatform);
                    }
                    WalkStep::Walking | WalkStep::Waiting => {}
                    WalkStep::NoRoute => {
                        // Cut off from their own platform. They stay home rather
                        // than wading there — and the town hears about it.
                        if let Some(route) = route.as_deref_mut() {
                            route.clear();
                        }
                        pos.stand_still();
                        waiting.wait_secs = 0;
                        journey.set_stage(JourneyStage::AtHome);
                        let place = station_name(&stations, journey.from_station);
                        speak_no_route(
                            &mut feed,
                            &mut router,
                            tick,
                            peep,
                            journey.from_station,
                            &place,
                            pos.tile(),
                        );
                    }
                }
            }

            JourneyStage::WaitingOnPlatform => {
                pos.stand_still();
                {
                    let district = flow.entry(journey.from_station);
                    district.waiting += 1;
                    district.pressure_secs = district
                        .pressure_secs
                        .saturating_add(SIM_SECONDS_PER_TICK.min(waiting.wait_secs));
                }
                // `advance_peep_waits` owns wait accumulation and mood.
                let patience = memory.patience_secs();
                if let Some(train) = boardable_train(
                    &network,
                    &stations,
                    &trains,
                    journey.from_station,
                    journey.to_station,
                ) {
                    journey.riding = Some(train);
                    journey.leg_wait_secs = waiting.wait_secs;
                    journey.set_stage(JourneyStage::Boarding);
                } else if waiting.wait_secs >= patience {
                    // §4.3 — a frustrated peep gives up and walks, and it shows.
                    journey.gave_up = true;
                    journey.leg_wait_secs = waiting.wait_secs;
                    journey.target = match journey.leg {
                        JourneyLeg::Outbound => routine.destination,
                        JourneyLeg::Return => routine.home,
                    };
                    journey.set_stage(JourneyStage::WalkingInstead);
                    let station_name = stations
                        .get(journey.from_station)
                        .map(|s| s.name.clone())
                        .unwrap_or_default();
                    flow.entry(journey.from_station).gave_up += 1;
                    feed.push(ComplaintEntry {
                        kind: TalkKind::Complaint,
                        peep_name: peep.given_name().to_string(),
                        station_name,
                        wait_minutes: gave_up_minutes(waiting.wait_minutes()),
                        sim_tick: tick,
                        peep_id: Some(peep.id),
                        station_id: Some(journey.from_station),
                        tile: Some(pos.tile()),
                        count: 1,
                    });
                    waiting.wait_secs = 0;
                }
            }

            JourneyStage::Boarding => {
                pos.stand_still();
                waiting.wait_secs = 0;
                if journey.stage_ticks >= BOARD_TICKS {
                    journey.set_stage(JourneyStage::Riding);
                }
            }

            JourneyStage::Riding => {
                let to_track = station_track(&network, &stations, journey.to_station);
                let aboard = journey
                    .riding
                    .and_then(|id| trains.iter().find(|(t, _)| t.id == id));
                match (aboard, to_track) {
                    (Some((_, loc)), Some(dest_track)) => {
                        if let Some(piece) = network.piece(loc.track) {
                            pos.snap_to(piece.tile);
                        }
                        if loc.track == dest_track {
                            journey.set_stage(JourneyStage::Alighting);
                        } else if !loc.path[loc.path_index..].contains(&dest_track) {
                            // The train changed its mind — get off and walk.
                            journey.gave_up = true;
                            journey.target = match journey.leg {
                                JourneyLeg::Outbound => routine.destination,
                                JourneyLeg::Return => routine.home,
                            };
                            journey.riding = None;
                            journey.set_stage(JourneyStage::WalkingInstead);
                        }
                    }
                    _ => {
                        // Train gone. Stranded — walk the rest.
                        journey.gave_up = true;
                        journey.target = match journey.leg {
                            JourneyLeg::Outbound => routine.destination,
                            JourneyLeg::Return => routine.home,
                        };
                        journey.riding = None;
                        journey.set_stage(JourneyStage::WalkingInstead);
                    }
                }
            }

            JourneyStage::Alighting => {
                journey.riding = None;
                if let Some(st) = stations.get(journey.to_station) {
                    pos.snap_to(st.tile);
                }
                pos.stand_still();
                if journey.stage_ticks >= BOARD_TICKS {
                    journey.target = match journey.leg {
                        JourneyLeg::Outbound => routine.destination,
                        JourneyLeg::Return => routine.home,
                    };
                    journey.set_stage(JourneyStage::WalkingToDestination);
                }
            }

            JourneyStage::WalkingToDestination | JourneyStage::WalkingInstead => {
                let target = journey.target;
                let step = walk_step(
                    route.as_deref_mut(),
                    &mut pos,
                    target,
                    WALK_TILES_PER_TICK,
                    world.as_ref(),
                    &mut router,
                );
                match step {
                    WalkStep::Arrived => {
                        finish_leg(
                            &mut journey,
                            &mut memory,
                            &mut waiting,
                            &mut flow,
                            routine,
                            tick,
                        );
                    }
                    WalkStep::Walking | WalkStep::Waiting => {}
                    WalkStep::NoRoute => {
                        // They are out in the world with no way through. They
                        // stop where they stand and the leg is graded a failure
                        // — nobody is teleported and nobody fords a river.
                        if let Some(route) = route.as_deref_mut() {
                            route.clear();
                        }
                        pos.stand_still();
                        journey.gave_up = true;
                        let place = station_name(&stations, journey.to_station);
                        speak_no_route(
                            &mut feed,
                            &mut router,
                            tick,
                            peep,
                            journey.to_station,
                            &place,
                            pos.tile(),
                        );
                        finish_leg(
                            &mut journey,
                            &mut memory,
                            &mut waiting,
                            &mut flow,
                            routine,
                            tick,
                        );
                    }
                }
            }

            JourneyStage::SpendingTime => {
                pos.stand_still();
                let stay_ticks = routine.stay_minutes.saturating_mul(60) / SIM_SECONDS_PER_TICK;
                if journey.stage_ticks >= stay_ticks.max(1) {
                    journey.begin_leg(
                        JourneyLeg::Return,
                        routine.destination_station,
                        routine.home_station,
                    );
                    waiting.station = routine.destination_station;
                    waiting.wait_secs = 0;
                    flow.entry(routine.destination_station).departures += 1;
                    if routine.home_station == routine.destination_station {
                        journey.target = routine.home;
                        journey.set_stage(JourneyStage::WalkingToDestination);
                    } else {
                        journey.target = stations
                            .get(routine.destination_station)
                            .map(|s| s.tile)
                            .unwrap_or(routine.destination);
                        journey.set_stage(JourneyStage::WalkingToStation);
                    }
                }
            }

            JourneyStage::LeavingTown => {
                let target = journey.target;
                // A departing household with no walkable way out waits by the
                // door; `peeps_move_away` retires them on its own timeout.
                let _ = walk_step(
                    route.as_deref_mut(),
                    &mut pos,
                    target,
                    WALK_TILES_PER_TICK,
                    world.as_ref(),
                    &mut router,
                );
                // Despawn is owned by `peeps_move_away` once they reach the edge.
            }
        }
    }
}

fn station_name(stations: &StationRegistry, id: StationId) -> String {
    stations.get(id).map(|s| s.name.clone()).unwrap_or_default()
}

/// Say, once and plainly, that somebody cannot walk where they were going.
///
/// Uses the existing [`TalkKind::Warning`] shape a household departure already
/// uses — a warning with no station carries its own whole sentence — so the
/// Town Talk voice stays *plain, specific, named* and no new kind appears in a
/// feed that other slices match exhaustively. Rate limited by [`WalkRouter`],
/// because a district cut off by a river would otherwise say it every tick.
fn speak_no_route(
    feed: &mut ComplaintFeed,
    router: &mut WalkRouter,
    tick: u64,
    peep: &Peep,
    station: StationId,
    place: &str,
    tile: TileCoord,
) {
    if !router.may_speak(tick) {
        return;
    }
    router.note_spoke(tick);
    let place = if place.is_empty() { "town" } else { place };
    feed.push(ComplaintEntry {
        kind: TalkKind::Warning,
        peep_name: format!(
            "{} cannot walk to {place} - no way across",
            peep.given_name()
        ),
        station_name: String::new(),
        wait_minutes: 0,
        sim_tick: tick,
        peep_id: Some(peep.id),
        station_id: Some(station),
        tile: Some(tile),
        count: 1,
    });
}

/// Close out a leg: grade it, remember it, and set up what comes next.
///
/// Shared with [`super::flow`] so an abstracted peep grades their journeys by
/// exactly the same rule as a fully simulated one — a peep who walks on screen
/// mid-commute must not get a different verdict for it.
pub(super) fn finish_leg(
    journey: &mut Journey,
    memory: &mut JourneyMemory,
    waiting: &mut WaitingAtStation,
    flow: &mut DistrictFlow,
    routine: &Routine,
    tick: u64,
) {
    let patience = memory.patience_secs();
    let outcome = outcome_for(
        journey.leg_wait_secs,
        journey.leg_secs,
        journey.gave_up,
        patience,
    );
    memory.record(JourneyRecord {
        from: journey.from_station,
        to: journey.to_station,
        wait_secs: journey.leg_wait_secs,
        total_secs: journey.leg_secs,
        outcome,
        ended_tick: tick,
    });
    flow.entry(journey.to_station).completed += 1;
    waiting.wait_secs = 0;

    match journey.leg {
        JourneyLeg::Outbound => {
            journey.set_stage(JourneyStage::SpendingTime);
        }
        JourneyLeg::Return => {
            waiting.station = routine.home_station;
            journey.begin_leg(
                JourneyLeg::Outbound,
                routine.home_station,
                routine.destination_station,
            );
            journey.set_stage(JourneyStage::AtHome);
        }
    }
}

/// `"07:24"` for a sim tick — handy for Town Talk and the Peep card.
pub fn tick_clock_label(tick: u64) -> String {
    clock_label(minute_of_day(tick) % DAY_MINUTES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::TrainKind;
    use crate::economy::MoneyLedger;
    use crate::money::Money;
    use crate::peeps::budget::PeepDetail;
    use crate::peeps::memory::JourneyOutcome;
    use crate::peeps::names::BodyType;
    use crate::peeps::routine::PeepRole;
    use crate::peeps::{advance_peep_waits, HouseholdId, PeepId};
    use crate::stations::StationServiceScore;
    use crate::track::{try_place_track, TrackTerrain, GROUND_LAYER};
    use bevy_app::{App, Update};

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    fn routine() -> Routine {
        Routine::from_seed(1, tile(2, 2), StationId(1), tile(20, 20), StationId(2))
    }

    /// Flat, dry ground the whole way across.
    fn dry_land(w: u32, h: u32) -> TrackTerrain {
        TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 0i8)))
    }

    /// A north-south river at `x = 8`, with an optional dry ford at `gap`.
    fn river_town(gap: Option<i32>) -> TrackTerrain {
        let (w, h) = (20u32, 8u32);
        TrackTerrain::new(
            w,
            h,
            (0..w * h).map(|i| {
                let x = (i % w) as i32;
                let y = (i / w) as i32;
                (x == 8 && gap != Some(y), 0i8)
            }),
        )
    }

    /// A two-station line with one transit train standing at the near platform.
    struct Town {
        app: App,
        east: StationId,
        mill: StationId,
    }

    impl Town {
        /// `stations_linked` lays the rails; without them nobody can board.
        fn new(stations_linked: bool) -> Self {
            Self::with_terrain(stations_linked, dry_land(20, 8))
        }

        fn with_terrain(stations_linked: bool, terrain: TrackTerrain) -> Self {
            let mut app = App::new();
            let mut network = TrackNetwork::new();
            let mut path = Vec::new();
            if stations_linked {
                let mut money = Money::new(50_000_000);
                let mut ledger = MoneyLedger::default();
                for x in 2..=14 {
                    let piece = try_place_track(
                        &mut network,
                        &mut money,
                        &mut ledger,
                        &terrain,
                        tile(x, 2),
                        GROUND_LAYER,
                    )
                    .expect("place track");
                    path.push(piece.id);
                }
            }

            let mut stations = StationRegistry::new();
            let east = stations.insert("Eastgate", tile(2, 2), GROUND_LAYER);
            let mill = stations.insert("Millhaven", tile(14, 2), GROUND_LAYER);

            let mut service = StationService::default();
            service.scores.insert(
                east,
                StationServiceScore {
                    score: 50,
                    ..Default::default()
                },
            );
            service.scores.insert(
                mill,
                StationServiceScore {
                    score: 50,
                    ..Default::default()
                },
            );

            app.insert_resource(network)
                .insert_resource(stations)
                .insert_resource(service)
                // Walking is terrain-aware, so the harness carries real terrain.
                .insert_resource(terrain)
                .init_resource::<crate::trains::TileOccupancy>()
                .init_resource::<DistrictFlow>()
                .init_resource::<ComplaintFeed>()
                .init_resource::<WalkRouter>();

            if stations_linked {
                let mut loc = TrainLocation::at_track(path[0]);
                loc.set_path(path);
                app.world_mut().spawn((
                    Train {
                        id: TrainId(1),
                        kind: TrainKind::Transit,
                    },
                    loc,
                ));
            }

            app.add_systems(
                Update,
                (
                    advance_peep_waits,
                    advance_journeys,
                    crate::trains::advance_trains,
                )
                    .chain(),
            );

            Self { app, east, mill }
        }

        /// One resident who lives on the platform tile, so the walk is short and
        /// the test can watch the whole loop inside a few hundred ticks.
        fn add_resident(&mut self, home: TileCoord, destination: TileCoord) {
            let routine = Routine {
                role: PeepRole::Commuter,
                home,
                home_station: self.east,
                destination,
                destination_station: self.mill,
                depart_minute: 0,
                stay_minutes: 30,
            };
            self.app.world_mut().spawn((
                Peep {
                    id: PeepId(1),
                    name: "Mara Aldertone".into(),
                    home,
                    mood: super::super::Mood::Content,
                    household: HouseholdId(1),
                    body: BodyType::Slight,
                    portrait: 0,
                    moved_in_tick: 0,
                },
                routine,
                Journey::new(&routine),
                PeepPosition::at_tile(home, 1),
                JourneyMemory::default(),
                WaitingAtStation::at(self.east),
                PeepDetail::Full,
                WalkRoute::default(),
            ));
        }

        /// Where the peep is standing right now.
        fn position(&mut self) -> PeepPosition {
            let mut q = self.app.world_mut().query::<&PeepPosition>();
            *q.iter(self.app.world()).next().unwrap()
        }

        /// Run, recording the tile the peep stood on each tick.
        fn run_tracking(&mut self, ticks: u32) -> Vec<TileCoord> {
            let mut seen = Vec::new();
            for _ in 0..ticks {
                self.app.world_mut().run_schedule(Update);
                let tile = self.position().tile();
                if seen.last() != Some(&tile) {
                    seen.push(tile);
                }
            }
            seen
        }

        /// Run and collect every stage the peep passed through.
        fn run(&mut self, ticks: u32) -> Vec<JourneyStage> {
            let mut seen = Vec::new();
            for _ in 0..ticks {
                self.app.world_mut().run_schedule(Update);
                let stage = self.stage();
                if seen.last() != Some(&stage) {
                    seen.push(stage);
                }
            }
            seen
        }

        fn stage(&mut self) -> JourneyStage {
            let mut q = self.app.world_mut().query::<&Journey>();
            q.iter(self.app.world()).next().unwrap().stage
        }

        fn memory(&mut self) -> JourneyMemory {
            let mut q = self.app.world_mut().query::<&JourneyMemory>();
            q.iter(self.app.world()).next().unwrap().clone()
        }

        fn talk(&self) -> Vec<String> {
            self.app
                .world()
                .resource::<ComplaintFeed>()
                .iter()
                .map(|e| e.display_line())
                .collect()
        }
    }

    #[test]
    fn a_peep_makes_the_whole_journey_the_brief_describes() {
        let mut town = Town::new(true);
        town.add_resident(tile(2, 2), tile(14, 4));
        let seen = town.run(400);

        for stage in [
            JourneyStage::WalkingToStation,
            JourneyStage::WaitingOnPlatform,
            JourneyStage::Boarding,
            JourneyStage::Riding,
            JourneyStage::Alighting,
            JourneyStage::WalkingToDestination,
            JourneyStage::SpendingTime,
        ] {
            assert!(
                seen.contains(&stage),
                "journey never reached {stage:?}; saw {seen:?}"
            );
        }
        // Home → station → platform → board → ride → alight → walk → stay, in order.
        let order: Vec<usize> = [
            JourneyStage::WalkingToStation,
            JourneyStage::WaitingOnPlatform,
            JourneyStage::Boarding,
            JourneyStage::Riding,
            JourneyStage::Alighting,
            JourneyStage::WalkingToDestination,
            JourneyStage::SpendingTime,
        ]
        .iter()
        .map(|s| seen.iter().position(|x| x == s).unwrap())
        .collect();
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "stages ran out of order: {seen:?}"
        );

        let memory = town.memory();
        assert_eq!(memory.lifetime_journeys, 1, "the leg should be remembered");
        assert_eq!(memory.last().unwrap().to, town.mill);
    }

    #[test]
    fn riding_hides_the_peep_and_carries_them_along_the_line() {
        let mut town = Town::new(true);
        town.add_resident(tile(2, 2), tile(14, 4));
        let mut ride_positions = Vec::new();
        for _ in 0..200 {
            town.app.world_mut().run_schedule(Update);
            let mut q = town.app.world_mut().query::<(&Journey, &PeepPosition)>();
            if let Some((journey, pos)) = q.iter(town.app.world()).next() {
                if journey.stage == JourneyStage::Riding {
                    assert!(!journey.is_visible(), "a peep on a train is not drawn");
                    ride_positions.push(pos.tile());
                }
            }
        }
        assert!(!ride_positions.is_empty(), "the peep never rode");
        assert!(
            ride_positions.first() != ride_positions.last(),
            "riding peep never moved with the train"
        );
    }

    #[test]
    fn with_no_railway_the_peep_gives_up_and_walks_and_town_talk_says_so() {
        let mut town = Town::new(false);
        town.add_resident(tile(2, 2), tile(14, 4));
        let seen = town.run(300);

        assert!(
            seen.contains(&JourneyStage::WaitingOnPlatform),
            "expected them to try the platform first: {seen:?}"
        );
        assert!(
            seen.contains(&JourneyStage::WalkingInstead),
            "a frustrated peep must give up and walk: {seen:?}"
        );
        let talk = town.talk();
        assert!(
            talk.iter()
                .any(|l| l.contains("then walked") || l.contains("are waiting at")),
            "Town Talk never reported the walk-off: {talk:?}"
        );
    }

    /// The playtest bug, pinned: *"they just walk through water and any other
    /// terrain."* Given a ford, the peep must use it.
    #[test]
    fn a_walking_peep_goes_round_the_water_and_never_through_it() {
        let terrain = river_town(Some(6));
        // The straight line from home to destination crosses the river…
        assert!(terrain.is_water(tile(8, 2)), "the test river is not wet");

        let mut town = Town::with_terrain(false, terrain);
        town.add_resident(tile(2, 2), tile(16, 2));
        // No railway, so they give up on the platform and walk the whole way.
        let tiles = town.run_tracking(1_400);

        let terrain = river_town(Some(6));
        for tile in &tiles {
            assert!(
                !terrain.is_water(*tile),
                "the peep walked on water at {tile:?}"
            );
        }
        assert!(
            tiles.iter().any(|t| t.x > 8),
            "the peep never reached the far bank: {tiles:?}"
        );
        assert!(
            tiles.iter().any(|t| *t == tile(8, 6)),
            "the peep did not use the ford: {tiles:?}"
        );
        assert!(
            tiles.iter().any(|t| *t == tile(16, 2)),
            "the peep never arrived by the long way round: {tiles:?}"
        );
    }

    /// No ford, no bridge, no route: they must not swim, and they must not
    /// teleport either.
    #[test]
    fn a_peep_cut_off_from_their_station_stays_home_and_town_talk_says_so() {
        // Home on the far bank from their own platform.
        let mut town = Town::with_terrain(false, river_town(None));
        town.add_resident(tile(12, 2), tile(16, 2));
        let seen = town.run(200);

        assert_eq!(
            town.stage(),
            JourneyStage::AtHome,
            "a peep with no walkable route must stay home: {seen:?}"
        );
        assert!(
            !seen.contains(&JourneyStage::WaitingOnPlatform),
            "they cannot have reached a platform they cannot walk to: {seen:?}"
        );
        assert_eq!(
            town.position().tile(),
            tile(12, 2),
            "a cut-off peep must not drift or teleport"
        );
        let talk = town.talk();
        assert!(
            talk.iter()
                .any(|l| l.contains("cannot walk to Eastgate") && l.contains("no way across")),
            "Town Talk never said they were cut off: {talk:?}"
        );
    }

    /// Cut off from where they were *going* rather than from their station: they
    /// stop on their own bank and the leg is graded a failure.
    #[test]
    fn a_peep_cut_off_from_their_destination_stops_rather_than_fording_it() {
        let mut town = Town::with_terrain(false, river_town(None));
        town.add_resident(tile(2, 2), tile(16, 2));
        let tiles = town.run_tracking(400);

        let terrain = river_town(None);
        for tile in &tiles {
            assert!(!terrain.is_water(*tile), "walked on water at {tile:?}");
            assert!(tile.x < 8, "somehow crossed the river at {tile:?}");
        }
        let talk = town.talk();
        assert!(
            talk.iter().any(|l| l.contains("cannot walk to Millhaven")),
            "Town Talk never said the far bank was unreachable: {talk:?}"
        );
        let memory = town.memory();
        assert!(
            memory.lifetime_gave_up >= 1,
            "a trip nobody could make should grade as a failure"
        );
    }

    #[test]
    fn giving_up_is_remembered_as_a_bad_journey() {
        let mut town = Town::new(false);
        town.add_resident(tile(2, 2), tile(4, 2));
        town.run(600);
        let memory = town.memory();
        assert!(
            memory.lifetime_gave_up >= 1,
            "the walk-off was not recorded"
        );
        assert!(
            memory.patience_secs() < BASE_PATIENCE_SECS_FOR_TEST,
            "a bad journey should shorten patience"
        );
    }

    const BASE_PATIENCE_SECS_FOR_TEST: u32 = super::super::memory::BASE_PATIENCE_SECS;

    #[test]
    fn every_brief_stage_exists_and_names_itself() {
        let stages = [
            JourneyStage::AtHome,
            JourneyStage::WalkingToStation,
            JourneyStage::WaitingOnPlatform,
            JourneyStage::Boarding,
            JourneyStage::Riding,
            JourneyStage::Alighting,
            JourneyStage::WalkingToDestination,
            JourneyStage::SpendingTime,
        ];
        for s in stages {
            assert!(!s.label().is_empty(), "{s:?} has no label");
        }
        assert!(JourneyStage::WalkingInstead.is_walking());
        assert!(!JourneyStage::Riding.is_visible());
        assert!(JourneyStage::WaitingOnPlatform.is_waiting());
        assert!(!JourneyStage::AtHome.is_travelling());
    }

    #[test]
    fn walking_reaches_its_target_and_faces_the_right_way() {
        let mut pos = PeepPosition::at_tile(tile(0, 0), 1);
        let mut ticks = 0;
        while !pos.walk_toward(tile(5, 0), WALK_TILES_PER_TICK) {
            ticks += 1;
            assert!(ticks < 10_000, "peep never arrived");
        }
        assert_eq!(pos.tile(), tile(5, 0));
        assert_eq!(pos.facing, Facing::East);
        // Eight times slower than a transit train's 3 ticks per tile.
        assert!(ticks >= 5 * (WALK_TICKS_PER_TILE as i32 - 2));
    }

    #[test]
    fn walk_cycle_alternates_two_frames() {
        let mut pos = PeepPosition::at_tile(tile(0, 0), 7);
        let start = pos.step;
        for _ in 0..STEP_FRAME_TICKS {
            pos.walk_toward(tile(40, 0), WALK_TILES_PER_TICK);
        }
        assert_ne!(pos.step, start, "walk frame never advanced");
        assert!(pos.step <= 1);
    }

    #[test]
    fn standing_still_stops_the_walk_cycle() {
        let mut pos = PeepPosition::at_tile(tile(0, 0), 7);
        pos.walk_toward(tile(9, 0), WALK_TILES_PER_TICK);
        assert!(pos.walking);
        pos.stand_still();
        assert!(!pos.walking);
    }

    #[test]
    fn facing_picks_the_dominant_axis() {
        assert_eq!(Facing::from_delta(3.0, 1.0), Some(Facing::East));
        assert_eq!(Facing::from_delta(-3.0, 1.0), Some(Facing::West));
        assert_eq!(Facing::from_delta(0.5, 4.0), Some(Facing::North));
        assert_eq!(Facing::from_delta(0.5, -4.0), Some(Facing::South));
        assert_eq!(Facing::from_delta(0.0, 0.0), None);
    }

    #[test]
    fn positions_jitter_so_a_crowd_does_not_stack() {
        let a = PeepPosition::at_tile(tile(4, 4), 1);
        let b = PeepPosition::at_tile(tile(4, 4), 2);
        assert!(
            (a.x - b.x).abs() > f32::EPSILON || (a.y - b.y).abs() > f32::EPSILON,
            "two peeps on one tile drew at exactly the same point"
        );
        assert_eq!(a.tile(), tile(4, 4));
        assert_eq!(b.tile(), tile(4, 4));
    }

    #[test]
    fn journey_starts_at_home_and_describes_itself() {
        let r = routine();
        let j = Journey::new(&r);
        assert_eq!(j.stage, JourneyStage::AtHome);
        assert_eq!(j.leg, JourneyLeg::Outbound);
        assert_eq!(j.describe("Eastgate", "Millhaven"), "At home.");
    }

    #[test]
    fn describe_names_both_ends_while_waiting() {
        let r = routine();
        let mut j = Journey::new(&r);
        j.set_stage(JourneyStage::WaitingOnPlatform);
        assert_eq!(
            j.describe("Eastgate", "Millhaven"),
            "Waiting at Eastgate for the Millhaven train."
        );
        j.set_stage(JourneyStage::WalkingInstead);
        assert_eq!(
            j.describe("Eastgate", "Millhaven"),
            "Gave up at Eastgate - walking instead."
        );
    }

    #[test]
    fn coarsening_collapses_to_three_stages() {
        let r = routine();
        for stage in [
            JourneyStage::WalkingToStation,
            JourneyStage::Boarding,
            JourneyStage::Riding,
            JourneyStage::Alighting,
        ] {
            let mut j = Journey::new(&r);
            j.set_stage(stage);
            j.riding = Some(TrainId(1));
            j.coarsen();
            assert_eq!(j.stage, JourneyStage::WaitingOnPlatform);
            assert!(j.riding.is_none());
        }
        let mut j = Journey::new(&r);
        j.set_stage(JourneyStage::WalkingInstead);
        j.coarsen();
        assert_eq!(j.stage, JourneyStage::SpendingTime);

        // A household that has decided to leave still leaves off camera.
        let mut j = Journey::new(&r);
        j.set_stage(JourneyStage::LeavingTown);
        j.coarsen();
        assert_eq!(j.stage, JourneyStage::LeavingTown);
    }

    #[test]
    fn finishing_a_giving_up_leg_records_a_bad_journey() {
        let r = routine();
        let mut j = Journey::new(&r);
        j.begin_leg(JourneyLeg::Outbound, StationId(1), StationId(2));
        j.gave_up = true;
        j.leg_wait_secs = 20 * 60;
        j.leg_secs = 40 * 60;
        let mut memory = JourneyMemory::default();
        let mut waiting = WaitingAtStation::at(StationId(1));
        let mut flow = DistrictFlow::default();
        finish_leg(&mut j, &mut memory, &mut waiting, &mut flow, &r, 99);

        assert_eq!(j.stage, JourneyStage::SpendingTime);
        let rec = memory.last().unwrap();
        assert_eq!(rec.outcome, JourneyOutcome::GaveUp);
        assert_eq!(rec.ended_tick, 99);
        assert_eq!(flow.get(StationId(2)).completed, 1);
    }

    #[test]
    fn return_leg_finishes_back_at_home() {
        let r = routine();
        let mut j = Journey::new(&r);
        j.begin_leg(JourneyLeg::Return, StationId(2), StationId(1));
        j.leg_secs = 120;
        let mut memory = JourneyMemory::default();
        let mut waiting = WaitingAtStation::at(StationId(2));
        let mut flow = DistrictFlow::default();
        finish_leg(&mut j, &mut memory, &mut waiting, &mut flow, &r, 5);
        assert_eq!(j.stage, JourneyStage::AtHome);
        assert_eq!(j.leg, JourneyLeg::Outbound);
        assert_eq!(waiting.station, r.home_station);
        assert_eq!(memory.last().unwrap().outcome, JourneyOutcome::Good);
    }
}
