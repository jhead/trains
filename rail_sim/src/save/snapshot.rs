//! [`WorldSnapshot`] — the complete, serialisable state of one Rail Town world.
//!
//! # Why this shape
//!
//! `IMPLEMENTATION_PLAN.md` § Multiplayer seams point 6 says the save blob will
//! later double as a **neighbour map chunk**. So the snapshot is plain data with
//! stable ids ([`crate::ids`]) and never Bevy [`Entity`] handles — an entity id
//! means nothing to the machine on the other end of a link. Everything that is
//! genuinely derived (occupancy, alerts) is still captured so a loaded world is
//! byte-identical to the one that was saved rather than merely equivalent.
//!
//! # Capture / restore
//!
//! - [`WorldSnapshot::capture`] takes `&World` and is deliberately cheap: it
//!   clones resources and walks two small queries. Encoding and writing happen
//!   later, off the sim thread (see [`super::save_to_slot_async`]).
//! - [`WorldSnapshot::restore`] rebuilds resources and respawns train / peep
//!   entities. It returns a [`RestoreReport`] instead of failing hard, because a
//!   world that loads with one odd station is better than a world that refuses.
//!
//! # Growing the snapshot
//!
//! Sections are separate structs so a new field lands in one place. Sections
//! that embed a sim type directly (track network, lines, job board, ledger,
//! alerts, demand) pick up new fields on those types automatically. Sections
//! that mirror a type by hand — stations, peeps — must be extended by hand;
//! those are the areas under active change, see the module docs in `mod.rs`.

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::clock::SimClock;
use crate::command_buffer::CommandBuffer;
use crate::demand::DemandSpawner;
use crate::border::BorderRegistry;
use crate::goals::GoalBoard;
use crate::economy::{AlertBoard, JobBoard, MoneyLedger};
use crate::history::CommandHistory;
use crate::ids::{StationId, TileCoord};
use crate::lines::LineRegistry;
use crate::money::Money;
use crate::peeps::{
    ComplaintEntry, ComplaintFeed, DistrictFlow, DistrictFlowState, Household, HouseholdRegistry,
    Journey, JourneyMemory, Peep, PeepBudget, PeepDetail, PeepId, PeepPosition, PeepSpawnState,
    Routine, TalkKind, WaitingAtStation, SIM_SECONDS_PER_TICK,
};
use crate::stations::{
    Industry, IndustryRegistry, Station, StationRegistry, StationService, StationServiceScore,
    StationTier,
};
use crate::town::TownDensity;
use crate::track::{TrackNetwork, TrackTerrain};
use crate::trains::{TileOccupancy, Train, TrainCargo, TrainLocation, TrainOnLine, TrainYard};
use crate::WorldAnchorsSeeded;

/// Save schema version. Bump on any change to the blob shape.
///
/// A save written by a different version is refused with
/// [`SaveError::VersionMismatch`](super::SaveError::VersionMismatch); there is
/// no silent partial read.
pub const SCHEMA_VERSION: u16 = 3;

/// Terrain generator revision recorded with the map.
///
/// Bump when `rail_map::generate_map` changes so a reloaded world can tell that
/// regenerating from the seed would no longer reproduce its tiles. The terrain
/// itself is stored, so the world is still exact — only cosmetic regeneration
/// (terrain kind bands) would drift.
pub const GENERATOR_VERSION: u16 = 1;

// ---------------------------------------------------------------------------
// Map
// ---------------------------------------------------------------------------

/// How the map was made, plus the terrain it produced.
///
/// `rail_sim` cannot see `rail_map` (that crate depends on this one), so the
/// seed and dimensions arrive via the [`MapDescriptor`] resource that the app
/// inserts next to its `MapGrid`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapSnapshot {
    pub seed: u64,
    pub width: u32,
    pub height: u32,
    pub gen: MapGenOptions,
    /// Materialised terrain. `None` only before the app has inserted one.
    pub terrain: Option<TerrainChunk>,
}

/// Knobs that shaped the map, beyond seed and size.
///
/// The generator's options really steer it now — terrain style, water style,
/// resource spread — so a save that recorded only *that* a world was generated
/// could not reproduce it. Seed sharing is a design promise (02 §5), and it is
/// only kept if the knobs travel with the seed.
///
/// `rail_sim` cannot see `rail_map` (that crate depends on this one), so they
/// travel as the byte `rail_map::MapGenOptions::pack` produces and the app
/// unpacks on the other side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapGenOptions {
    pub generator_version: u16,
    /// Packed `rail_map::MapGenOptions`, or `None` on a world whose app never
    /// declared how it was made — a bare test world. A loader that finds `None`
    /// must leave the map it already has alone rather than regenerate from a
    /// setup it is guessing at.
    pub knobs: Option<u8>,
}

impl Default for MapGenOptions {
    fn default() -> Self {
        Self {
            generator_version: GENERATOR_VERSION,
            knobs: None,
        }
    }
}

/// Water + height for a rectangle of tiles, row-major (`y * width + x`).
///
/// This is the piece that becomes a neighbour map chunk: it is self-describing,
/// has no ids in it, and reconstructs a [`TrackTerrain`] on its own.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TerrainChunk {
    pub width: u32,
    pub height: u32,
    pub water: Vec<bool>,
    pub heights: Vec<i8>,
}

impl TerrainChunk {
    pub fn from_terrain(terrain: &TrackTerrain) -> Self {
        let width = terrain.width();
        let height = terrain.height();
        let len = (width as usize).saturating_mul(height as usize);
        let mut water = Vec::with_capacity(len);
        let mut heights = Vec::with_capacity(len);
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                let c = TileCoord { x, y };
                water.push(terrain.is_water(c));
                heights.push(terrain.height_at(c).unwrap_or(0));
            }
        }
        Self {
            width,
            height,
            water,
            heights,
        }
    }

    /// Rebuild a [`TrackTerrain`], or `None` if the cell counts disagree.
    pub fn to_terrain(&self) -> Option<TrackTerrain> {
        let len = (self.width as usize).checked_mul(self.height as usize)?;
        if self.water.len() != len || self.heights.len() != len {
            return None;
        }
        let cells = self
            .water
            .iter()
            .copied()
            .zip(self.heights.iter().copied())
            .collect::<Vec<_>>();
        Some(TrackTerrain::new(self.width, self.height, cells))
    }
}

/// Seed / size / options for the world's map, inserted by the app next to its
/// `MapGrid` so the sim can record which world this is.
///
/// Without it a snapshot still stores the full terrain and loads exactly; only
/// the seed shown in the UI (and regeneration) would be unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Resource, Serialize, Deserialize)]
pub struct MapDescriptor {
    pub seed: u64,
    pub width: u32,
    pub height: u32,
    pub gen: MapGenOptions,
}

impl MapDescriptor {
    pub fn new(seed: u64, width: u32, height: u32) -> Self {
        Self {
            seed,
            width,
            height,
            gen: MapGenOptions::default(),
        }
    }

    /// Record how the generator was steered — `rail_map::MapGenOptions::pack`.
    ///
    /// An app that calls this is promising that `(seed, width, height, knobs)`
    /// reproduces the map exactly, which is what lets a load rebuild the world
    /// rather than merely restore its tiles.
    pub fn with_knobs(mut self, knobs: u8) -> Self {
        self.gen.knobs = Some(knobs);
        self
    }
}

// ---------------------------------------------------------------------------
// Stations / industries / service
// ---------------------------------------------------------------------------

/// Demand anchors and how well each is being served.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StationsSnapshot {
    /// Whole [`Station`] records, ascending by id — new station fields ride along.
    pub stations: Vec<Station>,
    /// Whole [`Industry`] records, ascending by id.
    pub industries: Vec<Industry>,
    /// Service scores, ascending by station id.
    pub service: Vec<ServiceScoreSnapshot>,
    /// [`StationService::tick`] — the sim's master tick counter.
    pub service_tick: u64,
}

/// One station's service readout.
///
/// Mirrors [`StationServiceScore`], which cannot be serialised directly. Both
/// halves of the mirror destructure the source type field by field with no
/// `..` rest pattern, so a new field on it stops compiling here until someone
/// decides whether it belongs in the blob. It used to restore with
/// `..Default::default()`, and `peep_waiting` — the named residents standing on
/// the platform — was quietly lost by every save that way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceScoreSnapshot {
    pub station: StationId,
    pub deliveries: u32,
    pub last_arrival_tick: u64,
    pub waiting_passengers: u32,
    /// Named peeps on the platform, counted separately from the job-board queue.
    pub peep_waiting: u32,
    pub score: u8,
    /// Cached platform grade of the stop being scored.
    pub tier: StationTier,
}

// ---------------------------------------------------------------------------
// Trains
// ---------------------------------------------------------------------------

/// Rolling stock: unplaced yard, placed trains, and current occupancy.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TrainsSnapshot {
    pub yard: TrainYard,
    /// Placed trains, ascending by train id.
    pub placed: Vec<TrainSnapshot>,
    /// Occupancy and congestion memory (held ticks, reroutes, railhead polish).
    pub occupancy: TileOccupancy,
}

/// One placed train: who it is, where it is, what it carries, what it serves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainSnapshot {
    pub train: Train,
    pub location: TrainLocation,
    pub cargo: TrainCargo,
    pub on_line: Option<TrainOnLine>,
}

// ---------------------------------------------------------------------------
// Town
// ---------------------------------------------------------------------------

/// Building density, sparse and sorted for a stable blob.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TownSnapshot {
    pub density: Vec<(TileCoord, f32)>,
}

// ---------------------------------------------------------------------------
// Peeps
// ---------------------------------------------------------------------------

/// Residents with their names, families, journeys, moods — and the Town Talk
/// they generated.
///
/// `09-shell-and-menus.md` §6 is explicit: peep names and histories persisting
/// across a save is what makes a town feel like a continuous place rather than
/// a re-rolled state. Every half of that lives here — the name, the household
/// they share a home with, the trip they are on, what they remember of the last
/// few, and what the town said about them.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PeepsSnapshot {
    /// Residents, ascending by peep id.
    pub peeps: Vec<PeepSnapshot>,
    /// Families, ascending by household id.
    pub households: Vec<Household>,
    /// Next peep id to hand out, so reloaded peeps never collide with new ones.
    pub next_id: u64,
    /// Stations that already have residents (ascending).
    pub spawned_for: Vec<StationId>,
    /// Town Talk, newest first, exactly as the feed holds it.
    pub town_talk: Vec<TownTalkSnapshot>,
    /// Abstracted district flow, ascending by station id.
    pub districts: Vec<(StationId, DistrictFlowState)>,
    /// Trips the districts want, not yet drained into the job board.
    pub pending_trips: Vec<(StationId, StationId)>,
    /// Level-of-detail budget settings.
    pub budget: BudgetSnapshot,
}

/// One resident and every component that makes them that person.
///
/// The components are stored whole, so a new field on [`Peep`], [`Routine`],
/// [`Journey`] or [`JourneyMemory`] is carried without touching this module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeepSnapshot {
    pub peep: Peep,
    pub waiting: Option<WaitingAtStation>,
    pub routine: Option<Routine>,
    pub journey: Option<Journey>,
    pub position: Option<PeepPosition>,
    pub memory: Option<JourneyMemory>,
    pub detail: Option<PeepDetail>,
}

/// Tunables of the bounded-simulation budget.
///
/// Mirrors [`PeepBudget`] by hand, and destructures it field by field with no
/// `..` rest pattern so a new tunable is a compile error here rather than a
/// silent omission. Two of its fields are deliberately **not** persisted:
///
/// - `detailed` / `abstracted` are readouts, not settings — how many peeps were
///   at full detail is a function of where the camera is, and both are rewritten
///   by the next rebalance.
/// - `ticks` is the countdown to that rebalance. It is session state measured in
///   sim ticks, and a load starts it fresh rather than resuming mid-count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetSnapshot {
    pub max_detailed: usize,
    pub rebalance_every: u32,
}

impl Default for BudgetSnapshot {
    fn default() -> Self {
        Self::from_budget(PeepBudget::default())
    }
}

impl BudgetSnapshot {
    fn from_budget(budget: PeepBudget) -> Self {
        let PeepBudget {
            max_detailed,
            rebalance_every,
            ticks: _,
            detailed: _,
            abstracted: _,
        } = budget;
        Self {
            max_detailed,
            rebalance_every,
        }
    }

    fn to_budget(self) -> PeepBudget {
        let Self {
            max_detailed,
            rebalance_every,
        } = self;
        PeepBudget {
            max_detailed,
            rebalance_every,
            ticks: 0,
            detailed: 0,
            abstracted: 0,
        }
    }
}

/// Serialisable mirror of [`TalkKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TalkKindSnapshot {
    #[default]
    Complaint,
    Praise,
    Opportunity,
    Warning,
}

impl From<TalkKind> for TalkKindSnapshot {
    fn from(k: TalkKind) -> Self {
        match k {
            TalkKind::Complaint => Self::Complaint,
            TalkKind::Praise => Self::Praise,
            TalkKind::Opportunity => Self::Opportunity,
            TalkKind::Warning => Self::Warning,
        }
    }
}

impl From<TalkKindSnapshot> for TalkKind {
    fn from(k: TalkKindSnapshot) -> Self {
        match k {
            TalkKindSnapshot::Complaint => Self::Complaint,
            TalkKindSnapshot::Praise => Self::Praise,
            TalkKindSnapshot::Opportunity => Self::Opportunity,
            TalkKindSnapshot::Warning => Self::Warning,
        }
    }
}

/// One Town Talk line — a peep's history, in the player's words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TownTalkSnapshot {
    pub kind: TalkKindSnapshot,
    pub peep_name: String,
    pub station_name: String,
    pub wait_minutes: u32,
    pub sim_tick: u64,
    pub peep_id: Option<u64>,
    pub station_id: Option<StationId>,
    pub tile: Option<TileCoord>,
    pub count: u32,
}

impl From<&ComplaintEntry> for TownTalkSnapshot {
    fn from(e: &ComplaintEntry) -> Self {
        Self {
            kind: e.kind.into(),
            peep_name: e.peep_name.clone(),
            station_name: e.station_name.clone(),
            wait_minutes: e.wait_minutes,
            sim_tick: e.sim_tick,
            peep_id: e.peep_id.map(|p| p.0),
            station_id: e.station_id,
            tile: e.tile,
            count: e.count,
        }
    }
}

impl From<&TownTalkSnapshot> for ComplaintEntry {
    fn from(s: &TownTalkSnapshot) -> Self {
        Self {
            kind: s.kind.into(),
            peep_name: s.peep_name.clone(),
            station_name: s.station_name.clone(),
            wait_minutes: s.wait_minutes,
            sim_tick: s.sim_tick,
            peep_id: s.peep_id.map(PeepId),
            station_id: s.station_id,
            tile: s.tile,
            count: s.count,
        }
    }
}

// ---------------------------------------------------------------------------
// Economy
// ---------------------------------------------------------------------------

/// Open work, money flow, and the alert board.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EconomySnapshot {
    pub jobs: JobBoard,
    pub ledger: MoneyLedger,
    pub alerts: AlertBoard,
}

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

/// Pause + speed at the moment of saving.
///
/// Mirrors [`SimClock`] by hand, destructured field by field with no `..` rest
/// pattern in either direction: everything the clock knows is worth saving, and
/// a new field must be an explicit decision rather than a default that quietly
/// replaces what the player had.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockSnapshot {
    pub paused: bool,
    pub speed_multiplier: u8,
}

impl Default for ClockSnapshot {
    fn default() -> Self {
        Self::from_clock(SimClock::default())
    }
}

impl ClockSnapshot {
    fn from_clock(clock: SimClock) -> Self {
        let SimClock {
            paused,
            speed_multiplier,
        } = clock;
        Self {
            paused,
            speed_multiplier,
        }
    }

    fn to_clock(self) -> SimClock {
        let Self {
            paused,
            speed_multiplier,
        } = self;
        SimClock {
            paused,
            speed_multiplier,
        }
    }
}

// ---------------------------------------------------------------------------
// The snapshot
// ---------------------------------------------------------------------------

/// A complete world: map, network, anchors, lines, stock, town, people, money.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSnapshot {
    /// Schema this snapshot was built against — see [`SCHEMA_VERSION`].
    pub schema_version: u16,
    pub map: MapSnapshot,
    pub track: TrackNetwork,
    pub stations: StationsSnapshot,
    pub lines: LineRegistry,
    pub trains: TrainsSnapshot,
    pub town: TownSnapshot,
    pub peeps: PeepsSnapshot,
    pub economy: EconomySnapshot,
    pub demand: DemandSpawner,
    /// Goal set and progress. A goals world that lost this on load would
    /// silently revert to a sandbox board.
    pub goals: GoalBoard,
    /// Open and archived border links, cached neighbour manifests, and trains
    /// mid-crossing. Transit stock lives here as plain data rather than as
    /// entities, so a save/load mid-crossing cannot strand it.
    pub borders: BorderRegistry,
    pub clock: ClockSnapshot,
    /// Treasury, in cents.
    pub money_cents: i64,
    /// Whether the world's stations / industries were already auto-seeded.
    /// Restoring this stops the seeder from laying a second set on load.
    pub anchors_seeded: bool,
}

impl Default for WorldSnapshot {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            map: MapSnapshot {
                seed: 0,
                width: 0,
                height: 0,
                gen: MapGenOptions::default(),
                terrain: None,
            },
            track: TrackNetwork::default(),
            stations: StationsSnapshot::default(),
            lines: LineRegistry::default(),
            trains: TrainsSnapshot::default(),
            town: TownSnapshot::default(),
            peeps: PeepsSnapshot::default(),
            economy: EconomySnapshot::default(),
            demand: DemandSpawner::default(),
            goals: GoalBoard::default(),
            borders: BorderRegistry::default(),
            clock: ClockSnapshot::default(),
            money_cents: 0,
            anchors_seeded: false,
        }
    }
}

/// What restoring had to work around. Empty is the happy path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub warnings: Vec<String>,
}

impl RestoreReport {
    pub fn is_clean(&self) -> bool {
        self.warnings.is_empty()
    }

    fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }
}

impl WorldSnapshot {
    /// Copy the whole world out of `world`.
    ///
    /// Cheap by design — clones resources and walks the train / peep queries so
    /// the caller can hand the result to a background encoder and let the sim
    /// carry on. Missing resources fall back to their defaults, so this works on
    /// a half-built world (tests, a map that has not been generated yet).
    pub fn capture(world: &World) -> Self {
        let service_tick = world
            .get_resource::<StationService>()
            .map(|s| s.tick)
            .unwrap_or(0);

        Self {
            schema_version: SCHEMA_VERSION,
            map: capture_map(world),
            track: world.get_resource::<TrackNetwork>().cloned().unwrap_or_default(),
            stations: capture_stations(world, service_tick),
            lines: world.get_resource::<LineRegistry>().cloned().unwrap_or_default(),
            trains: capture_trains(world),
            town: capture_town(world),
            peeps: capture_peeps(world),
            economy: EconomySnapshot {
                jobs: world.get_resource::<JobBoard>().cloned().unwrap_or_default(),
                ledger: world.get_resource::<MoneyLedger>().cloned().unwrap_or_default(),
                alerts: world.get_resource::<AlertBoard>().cloned().unwrap_or_default(),
            },
            demand: world.get_resource::<DemandSpawner>().cloned().unwrap_or_default(),
            goals: world.get_resource::<GoalBoard>().cloned().unwrap_or_default(),
            borders: world.get_resource::<BorderRegistry>().cloned().unwrap_or_default(),
            clock: world
                .get_resource::<SimClock>()
                .copied()
                .map(ClockSnapshot::from_clock)
                .unwrap_or_default(),
            money_cents: world.get_resource::<Money>().map(|m| m.cents()).unwrap_or(0),
            anchors_seeded: world
                .get_resource::<WorldAnchorsSeeded>()
                .map(|s| s.0)
                .unwrap_or(false),
        }
    }

    /// Sim ticks elapsed in this world (the service tick is the master counter).
    pub fn sim_tick(&self) -> u64 {
        self.stations.service_tick
    }

    /// Rough sim-seconds of play, for the save list.
    pub fn elapsed_sim_secs(&self) -> u64 {
        self.sim_tick()
            .saturating_mul(u64::from(SIM_SECONDS_PER_TICK))
    }

    /// Write this world back into `world`, replacing what is there.
    ///
    /// Existing train and peep entities are despawned first; sim resources are
    /// overwritten wholesale. Undo history is dropped (its inverses describe a
    /// world that no longer exists) and pending commands are discarded.
    pub fn restore(&self, world: &mut World) -> RestoreReport {
        let mut report = RestoreReport::default();

        despawn_all::<Train>(world);
        despawn_all::<Peep>(world);

        restore_map(self, world);
        world.insert_resource(self.track.clone());
        restore_stations(self, world, &mut report);
        world.insert_resource(self.lines.clone());
        restore_trains(self, world);
        restore_town(self, world);
        restore_peeps(self, world);

        world.insert_resource(self.economy.jobs.clone());
        world.insert_resource(self.economy.ledger.clone());
        world.insert_resource(self.economy.alerts.clone());
        world.insert_resource(self.demand.clone());
        world.insert_resource(self.goals.clone());
        world.insert_resource(self.borders.clone());

        world.insert_resource(self.clock.to_clock());
        world.insert_resource(Money::new(self.money_cents));
        world.insert_resource(WorldAnchorsSeeded(self.anchors_seeded));

        // Session state that must not survive a load: queued intent for the old
        // world, and undo entries whose inverses point at vanished track ids.
        if let Some(mut buffer) = world.get_resource_mut::<CommandBuffer>() {
            let _ = buffer.drain();
        }
        world.insert_resource(CommandHistory::new());

        report
    }
}

// ---------------------------------------------------------------------------
// capture helpers
// ---------------------------------------------------------------------------

fn capture_map(world: &World) -> MapSnapshot {
    let terrain = world
        .get_resource::<TrackTerrain>()
        .map(TerrainChunk::from_terrain);
    let descriptor = world.get_resource::<MapDescriptor>().copied();

    let (width, height) = match (&descriptor, &terrain) {
        (Some(d), _) => (d.width, d.height),
        (None, Some(t)) => (t.width, t.height),
        (None, None) => (0, 0),
    };

    MapSnapshot {
        seed: descriptor.map(|d| d.seed).unwrap_or(0),
        width,
        height,
        gen: descriptor.map(|d| d.gen).unwrap_or_default(),
        terrain,
    }
}

fn capture_stations(world: &World, service_tick: u64) -> StationsSnapshot {
    let mut stations: Vec<Station> = world
        .get_resource::<StationRegistry>()
        .map(|r| r.iter().cloned().collect())
        .unwrap_or_default();
    stations.sort_by_key(|s| s.id.0);

    let mut industries: Vec<Industry> = world
        .get_resource::<IndustryRegistry>()
        .map(|r| r.iter().cloned().collect())
        .unwrap_or_default();
    industries.sort_by_key(|i| i.id.0);

    // `StationService::scores` is a `HashMap`, so the sort below is what makes
    // the blob deterministic rather than hash-order noise.
    let mut service: Vec<ServiceScoreSnapshot> = world
        .get_resource::<StationService>()
        .map(|s| {
            s.scores
                .iter()
                .map(|(station, score)| {
                    let StationServiceScore {
                        deliveries,
                        last_arrival_tick,
                        waiting_passengers,
                        peep_waiting,
                        score,
                        tier,
                    } = *score;
                    ServiceScoreSnapshot {
                        station: *station,
                        deliveries,
                        last_arrival_tick,
                        waiting_passengers,
                        peep_waiting,
                        score,
                        tier,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    service.sort_by_key(|s| s.station.0);

    StationsSnapshot {
        stations,
        industries,
        service,
        service_tick,
    }
}

fn capture_trains(world: &World) -> TrainsSnapshot {
    let mut placed = Vec::new();
    if let Some(mut query) = world.try_query::<(
        &Train,
        &TrainLocation,
        Option<&TrainCargo>,
        Option<&TrainOnLine>,
    )>() {
        for (train, location, cargo, on_line) in query.iter(world) {
            placed.push(TrainSnapshot {
                train: *train,
                location: location.clone(),
                cargo: cargo.cloned().unwrap_or_default(),
                on_line: on_line.copied(),
            });
        }
    }
    placed.sort_by_key(|t| t.train.id.0);

    TrainsSnapshot {
        yard: world.get_resource::<TrainYard>().cloned().unwrap_or_default(),
        placed,
        occupancy: world
            .get_resource::<TileOccupancy>()
            .cloned()
            .unwrap_or_default(),
    }
}

fn capture_town(world: &World) -> TownSnapshot {
    let mut density: Vec<(TileCoord, f32)> = world
        .get_resource::<TownDensity>()
        .map(|d| d.iter().collect())
        .unwrap_or_default();
    density.sort_by_key(|(t, _)| (t.y, t.x));
    TownSnapshot { density }
}

fn capture_peeps(world: &World) -> PeepsSnapshot {
    let mut peeps = Vec::new();
    if let Some(mut query) = world.try_query::<(
        &Peep,
        Option<&WaitingAtStation>,
        Option<&Routine>,
        Option<&Journey>,
        Option<&PeepPosition>,
        Option<&JourneyMemory>,
        Option<&PeepDetail>,
    )>() {
        for (peep, waiting, routine, journey, position, memory, detail) in query.iter(world) {
            peeps.push(PeepSnapshot {
                peep: peep.clone(),
                waiting: waiting.cloned(),
                routine: routine.copied(),
                journey: journey.cloned(),
                position: position.copied(),
                memory: memory.cloned(),
                detail: detail.copied(),
            });
        }
    }
    peeps.sort_by_key(|p| p.peep.id.0);

    let mut households: Vec<Household> = world
        .get_resource::<HouseholdRegistry>()
        .map(|r| r.iter().cloned().collect())
        .unwrap_or_default();
    households.sort_by_key(|h| h.id.0);

    let (next_id, mut spawned_for) = world
        .get_resource::<PeepSpawnState>()
        .map(|s| {
            (
                s.next_id,
                s.spawned_for.iter().copied().collect::<Vec<StationId>>(),
            )
        })
        .unwrap_or_default();
    spawned_for.sort_by_key(|s| s.0);

    let town_talk = world
        .get_resource::<ComplaintFeed>()
        .map(|f| f.iter().map(TownTalkSnapshot::from).collect())
        .unwrap_or_default();

    let (mut districts, pending_trips) = world
        .get_resource::<DistrictFlow>()
        .map(|f| {
            (
                f.iter().map(|(id, state)| (id, *state)).collect::<Vec<_>>(),
                f.pending_trips().to_vec(),
            )
        })
        .unwrap_or_default();
    districts.sort_by_key(|(id, _)| id.0);

    let budget = world
        .get_resource::<PeepBudget>()
        .copied()
        .map(BudgetSnapshot::from_budget)
        .unwrap_or_default();

    PeepsSnapshot {
        peeps,
        households,
        next_id,
        spawned_for,
        town_talk,
        districts,
        pending_trips,
        budget,
    }
}

// ---------------------------------------------------------------------------
// restore helpers
// ---------------------------------------------------------------------------

fn despawn_all<C: Component>(world: &mut World) {
    let doomed: Vec<Entity> = match world.try_query::<(Entity, &C)>() {
        Some(mut query) => query.iter(world).map(|(e, _)| e).collect(),
        None => Vec::new(),
    };
    for entity in doomed {
        world.despawn(entity);
    }
}

fn restore_map(snapshot: &WorldSnapshot, world: &mut World) {
    world.insert_resource(MapDescriptor {
        seed: snapshot.map.seed,
        width: snapshot.map.width,
        height: snapshot.map.height,
        gen: snapshot.map.gen,
    });
    if let Some(terrain) = snapshot.map.terrain.as_ref().and_then(|c| c.to_terrain()) {
        world.insert_resource(terrain);
    }
}

/// Highest station id anything in the snapshot still mentions.
///
/// Demolishing a stop removes the station but not every reference to it — a
/// stale service score, a Town Talk line, a line stop. Handing that id out
/// again after a load would attach the ghost to a brand new platform, so the
/// restored counter is pushed past the highest id the world has ever spoken of,
/// not merely past the ones still standing.
fn station_id_high_water(snapshot: &WorldSnapshot) -> u64 {
    let mut high = 0u64;
    let mut bump = |id: StationId| high = high.max(id.0);

    for station in &snapshot.stations.stations {
        bump(station.id);
    }
    for entry in &snapshot.stations.service {
        bump(entry.station);
    }
    for line in snapshot.lines.iter() {
        for stop in &line.stops {
            bump(*stop);
        }
    }
    for entry in &snapshot.peeps.town_talk {
        if let Some(id) = entry.station_id {
            bump(id);
        }
    }
    for station in &snapshot.peeps.spawned_for {
        bump(*station);
    }
    for household in &snapshot.peeps.households {
        bump(household.home_station);
    }
    for (station, _) in &snapshot.peeps.districts {
        bump(*station);
    }
    high
}

/// Advance a station registry's id counter to `target` without leaving stations
/// behind: insert a scratch record far off the map, then remove it again.
fn skip_station_ids(stations: &mut StationRegistry, from: u64, target: u64, layer: u8) {
    let mut next = from;
    while next <= target {
        let filler = stations.insert(
            "",
            TileCoord {
                x: i32::MIN,
                y: i32::MIN.saturating_add(next as i32),
            },
            layer,
        );
        stations.remove(filler);
        next = next.saturating_add(1);
    }
}

/// Rebuild the station / industry registries.
///
/// Neither registry can be deserialised directly, so we replay inserts in
/// ascending id order and the original ids come back. Holes left by demolition
/// are stepped over, and the counter finishes past every id the world still
/// mentions — see [`station_id_high_water`].
fn restore_stations(snapshot: &WorldSnapshot, world: &mut World, report: &mut RestoreReport) {
    let mut stations = StationRegistry::new();
    let layer = snapshot
        .stations
        .stations
        .first()
        .map(|s| s.layer)
        .unwrap_or(crate::track::GROUND_LAYER);
    let mut next = 1u64;
    for station in &snapshot.stations.stations {
        skip_station_ids(
            &mut stations,
            next,
            station.id.0.saturating_sub(1),
            station.layer,
        );
        let id = stations.insert_tier(
            station.name.clone(),
            station.tile,
            station.layer,
            station.tier,
            station.paid_cents,
        );
        if id != station.id {
            report.warn(format!(
                "station “{}” came back as id {} instead of {}",
                station.name, id.0, station.id.0
            ));
        }
        next = id.0.saturating_add(1);
    }
    skip_station_ids(&mut stations, next, station_id_high_water(snapshot), layer);
    world.insert_resource(stations);

    let mut industries = IndustryRegistry::new();
    for industry in &snapshot.stations.industries {
        let id = industries.insert(
            industry.name.clone(),
            industry.tile,
            industry.produces,
            industry.consumes,
        );
        if id != industry.id {
            report.warn(format!(
                "industry “{}” came back as id {} instead of {}",
                industry.name, id.0, industry.id.0
            ));
        }
    }
    world.insert_resource(industries);

    let mut service = StationService {
        scores: Default::default(),
        tick: snapshot.stations.service_tick,
    };
    for entry in &snapshot.stations.service {
        let ServiceScoreSnapshot {
            station,
            deliveries,
            last_arrival_tick,
            waiting_passengers,
            peep_waiting,
            score,
            tier,
        } = *entry;
        service.scores.insert(
            station,
            StationServiceScore {
                deliveries,
                last_arrival_tick,
                waiting_passengers,
                peep_waiting,
                score,
                tier,
            },
        );
    }
    world.insert_resource(service);
}

fn restore_trains(snapshot: &WorldSnapshot, world: &mut World) {
    world.insert_resource(snapshot.trains.yard.clone());

    for train in &snapshot.trains.placed {
        let mut entity = world.spawn((
            train.train,
            train.location.clone(),
            train.cargo.clone(),
        ));
        if let Some(on_line) = train.on_line {
            entity.insert(on_line);
        }
    }

    world.insert_resource(snapshot.trains.occupancy.clone());
}

fn restore_town(snapshot: &WorldSnapshot, world: &mut World) {
    let mut density = TownDensity::default();
    for (tile, value) in &snapshot.town.density {
        density.set(*tile, *value);
    }
    world.insert_resource(density);
}

/// Advance a household registry's id counter to `target`, leaving no families.
fn skip_household_ids(households: &mut HouseholdRegistry, from: u64, target: u64) {
    let mut next = from;
    while next <= target {
        let filler = households.insert(TileCoord { x: 0, y: 0 }, StationId(0), 0);
        households.remove(filler);
        next = next.saturating_add(1);
    }
}

fn restore_peeps(snapshot: &WorldSnapshot, world: &mut World) {
    for peep in &snapshot.peeps.peeps {
        let mut entity = world.spawn(peep.peep.clone());
        if let Some(waiting) = peep.waiting.clone() {
            entity.insert(waiting);
        }
        if let Some(routine) = peep.routine {
            entity.insert(routine);
        }
        if let Some(journey) = peep.journey.clone() {
            entity.insert(journey);
        }
        if let Some(position) = peep.position {
            entity.insert(position);
        }
        if let Some(memory) = peep.memory.clone() {
            entity.insert(memory);
        }
        if let Some(detail) = peep.detail {
            entity.insert(detail);
        }
    }

    // Households: same id dance as stations. A family that moved away leaves a
    // hole, every remaining peep still names their own household, and the
    // counter must clear the highest id any peep still remembers.
    let mut households = HouseholdRegistry::new();
    let high_water = snapshot
        .peeps
        .households
        .iter()
        .map(|h| h.id.0)
        .chain(snapshot.peeps.peeps.iter().map(|p| p.peep.household.0))
        .max()
        .unwrap_or(0);
    let mut next = 1u64;
    for household in &snapshot.peeps.households {
        skip_household_ids(&mut households, next, household.id.0.saturating_sub(1));
        let id = households.insert(
            household.home,
            household.home_station,
            household.moved_in_tick,
        );
        if let Some(restored) = households.get_mut(id) {
            *restored = household.clone();
        }
        next = id.0.saturating_add(1);
    }
    skip_household_ids(&mut households, next, high_water);
    world.insert_resource(households);

    let mut spawn_state = PeepSpawnState {
        next_id: snapshot.peeps.next_id,
        ..Default::default()
    };
    for station in &snapshot.peeps.spawned_for {
        spawn_state.spawned_for.insert(*station);
    }
    world.insert_resource(spawn_state);

    // The feed is newest-first; push oldest-first so it lands the same way up.
    let mut feed = ComplaintFeed::default();
    for entry in snapshot.peeps.town_talk.iter().rev() {
        feed.push(ComplaintEntry::from(entry));
    }
    world.insert_resource(feed);

    let mut flow = DistrictFlow::default();
    for (station, state) in &snapshot.peeps.districts {
        *flow.entry(*station) = *state;
    }
    for (from, to) in &snapshot.peeps.pending_trips {
        flow.request_trip(*from, *to);
    }
    world.insert_resource(flow);

    // Only the tunables are carried; the rebalance counter and the two readouts
    // start fresh and are rewritten by the next reshuffle. See [`BudgetSnapshot`].
    world.insert_resource(snapshot.peeps.budget.to_budget());
}
