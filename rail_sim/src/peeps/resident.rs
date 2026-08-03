//! Peep identity, spawning, wait / mood, and moving away.
//!
//! A peep is a *person*: a full name from the combinatorial pool, a household
//! they share a building with, a routine, and a memory of how their journeys
//! have gone (brief 06 §4.2). Mood is caused by that accumulated experience and
//! expressed on the sprite, in Town Talk, and in their decisions (§4.3).

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::ids::{StationId, TileCoord};
use crate::stations::{Station, StationRegistry, StationService};
use crate::town::TownDensity;
use crate::track::TrackTerrain;

use super::budget::{PeepBudget, PeepDetail};
use super::complaints::{ComplaintEntry, ComplaintFeed, TalkKind, COMPLAINT_WAIT_SECS};
use super::household::{HouseholdRegistry, VacatedHomes, HOUSEHOLD_MAX};
use super::journey::{Journey, JourneyStage, PeepPosition};
use super::memory::JourneyMemory;
use super::names::{full_name, hash64, portrait_variant, BodyType};
use super::routine::Routine;
use super::walk::WalkRoute;
use super::{HouseholdId, PeepId, PeepSpawnState};

/// Sim-seconds advanced per FixedUpdate tick while a peep is waiting.
pub const SIM_SECONDS_PER_TICK: u32 = 10;

/// Headcount a district starts with the moment it gets a station.
pub const PEEPS_PER_STATION: usize = 6;

/// Households seeded per station (members are drawn per household).
pub const HOUSEHOLDS_PER_STATION: usize = 3;

/// Extra residents per unit of built density in the station's growth ring.
///
/// This is the last link in the brief's chain — *line reaches a place → station
/// serves it → the district thickens → more people*. Peeps arrive **because**
/// buildings went up, not on a timer.
pub const PEEPS_PER_DENSITY: f32 = 0.4;

/// Ceiling on one district's headcount — town scale, not city scale (06 §7).
pub const MAX_PEEPS_PER_STATION: usize = 48;

/// Ceiling on the whole map's simulated residents.
pub const MAX_TOWN_POPULATION: usize = 400;

/// Ticks between move-ins once a district is past its starting headcount.
pub const MOVE_IN_INTERVAL_TICKS: u64 = 60;

/// Closest / furthest a home sits from its station, in tiles.
pub const HOME_MIN_RADIUS: i32 = 2;
pub const HOME_MAX_RADIUS: i32 = 4;

/// Service score at which an emptied district attracts new residents again —
/// restoring service has to be able to bring people back (brief 06 §8.4).
pub const REPOPULATE_SCORE: u8 = 60;

/// Ticks between a capped district asking for a better station.
///
/// About a minute of real time at the fixed timestep, and staggered by station
/// id so two full districts never ask on the same tick. 06 §6: these are
/// invitations, and an invitation repeated is a nag.
pub const DISTRICT_FULL_TALK_TICKS: u64 = 3_600;

/// Cooldown (ticks) after a complaint before the same peep can complain again.
const COMPLAINT_COOLDOWN_TICKS: u32 = 90;

/// Cooldown (ticks) after praise before the same peep can praise again.
const PRAISE_COOLDOWN_TICKS: u32 = 180;

/// Service score at or above which good-service praise may fire.
const PRAISE_SCORE_MIN: u8 = 80;

/// Wait must stay under this many sim-seconds for praise.
const PRAISE_WAIT_MAX_SECS: u32 = 4 * 60;

/// How fast a wait unwinds once a peep is off the platform.
const WAIT_RECOVERY_SECS: u32 = 30;

/// Mood, caused by accumulated experience (§4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mood {
    Content,
    Uneasy,
    Frustrated,
}

impl Default for Mood {
    fn default() -> Self {
        Self::Content
    }
}

impl Mood {
    pub fn label(self) -> &'static str {
        match self {
            Self::Content => "Content",
            Self::Uneasy => "Uneasy",
            Self::Frustrated => "Frustrated",
        }
    }

    /// Town Talk / sprite glyph.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Content => "+",
            Self::Uneasy => "~",
            Self::Frustrated => "x",
        }
    }
}

/// A named resident of the town.
#[derive(Component, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peep {
    pub id: PeepId,
    /// Full name — `"Mara Aldertone"`. Shown wherever there is room.
    pub name: String,
    /// The building they live in.
    pub home: TileCoord,
    pub mood: Mood,
    /// Family they share a home with.
    pub household: HouseholdId,
    /// Procedural portrait body type.
    pub body: BodyType,
    /// Palette variant for hair / clothing.
    pub portrait: u8,
    /// Sim tick they moved in.
    pub moved_in_tick: u64,
}

impl Peep {
    /// Build a resident from the parts a save file has to keep.
    ///
    /// Body type and portrait variant are re-derived from the id rather than
    /// stored, so a restored peep looks like exactly the same person.
    pub fn new(
        id: PeepId,
        name: impl Into<String>,
        home: TileCoord,
        household: HouseholdId,
        moved_in_tick: u64,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            home,
            mood: Mood::Content,
            household,
            body: BodyType::from_seed(id.0),
            portrait: portrait_variant(id.0),
            moved_in_tick,
        }
    }

    /// `"Mara"` — used where space is tight, like the Town Talk ticker.
    pub fn given_name(&self) -> &str {
        self.name.split(' ').next().unwrap_or(&self.name)
    }

    /// `"Aldertone"` — shared with the rest of the household.
    pub fn family_name(&self) -> &str {
        self.name.split(' ').nth(1).unwrap_or("")
    }

    /// Whole sim-days lived in town.
    pub fn days_in_town(&self, tick: u64) -> u64 {
        tick.saturating_sub(self.moved_in_tick) / super::TICKS_PER_DAY
    }

    /// The line that makes the Peep card land (05 §3.3).
    pub fn tenure_line(&self, place: &str, tick: u64) -> String {
        let days = self.days_in_town(tick);
        match days {
            0 => format!("{} has just moved to {place}.", self.given_name()),
            1 => format!("{} has lived in {place} for a day.", self.given_name()),
            n => format!("{} has lived in {place} for {n} days.", self.given_name()),
        }
    }
}

/// The station a peep is attached to, and their live platform wait.
///
/// Present on **every** peep, not only the ones standing on a platform:
/// `station` is the stop this peep uses for their current leg and `wait_secs`
/// is `0` unless they are actually waiting. That keeps the station panel's
/// "who is waiting, worst first" list honest without a second component.
#[derive(Component, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingAtStation {
    pub station: StationId,
    /// Accumulated wait in sim-seconds.
    pub wait_secs: u32,
    pub ticks_since_complaint: u32,
    pub ticks_since_praise: u32,
}

impl WaitingAtStation {
    pub fn at(station: StationId) -> Self {
        Self {
            station,
            wait_secs: 0,
            ticks_since_complaint: COMPLAINT_COOLDOWN_TICKS, // allow the first complaint
            ticks_since_praise: PRAISE_COOLDOWN_TICKS,
        }
    }

    pub fn wait_minutes(&self) -> u32 {
        self.wait_secs / 60
    }
}

/// Move households into districts that can support them.
///
/// A brand-new station gets its starting families at once; after that the
/// district grows toward the headcount its **built density** supports, one
/// household at a time. A district that has emptied out repopulates only once
/// service recovers, so a player who fixes a neglected line sees people move
/// back in (brief 06 §8.4).
#[allow(clippy::too_many_arguments)]
pub fn spawn_peep_households(
    mut commands: Commands,
    stations: Res<StationRegistry>,
    service: Res<StationService>,
    density: Option<Res<TownDensity>>,
    terrain: Option<Res<TrackTerrain>>,
    mut households: ResMut<HouseholdRegistry>,
    mut state: ResMut<PeepSpawnState>,
    mut budget: ResMut<PeepBudget>,
    mut feed: ResMut<ComplaintFeed>,
) {
    if stations.is_empty() || households.population() >= MAX_TOWN_POPULATION {
        return;
    }
    let tick = service.tick;
    let mut station_ids: Vec<StationId> = stations.iter().map(|s| s.id).collect();
    station_ids.sort_by_key(|s| s.0);

    for station in station_ids.iter().copied() {
        let Some(home_station) = stations.get(station) else {
            continue;
        };
        let here: usize = households
            .at_station(station)
            .map(|h| h.members.len())
            .sum();
        let first_time = state.spawned_for.insert(station);

        // How many people this district can support right now.
        let target = if here == 0 {
            if !first_time && service.score(station).score < REPOPULATE_SCORE {
                // Neglected district — nobody is moving in until service returns.
                continue;
            }
            PEEPS_PER_STATION
        } else {
            let supported = district_capacity(home_station, density.as_deref());
            if here >= supported {
                // 06 §6 — "a district that has grown to its cap asks for a
                // better station." Nobody was asking; the cap was simply where
                // growth stopped, silently, which reads as the game running out
                // rather than as the player having something to do next.
                speak_district_full(&mut feed, home_station, here, tick);
                continue;
            }
            // Move-ins are an event, not a ramp — one household at a time.
            if tick % MOVE_IN_INTERVAL_TICKS != 0 {
                continue;
            }
            here + 1
        };

        let mut seeded = here;
        let mut house = 0usize;
        while seeded < target && house < HOUSEHOLDS_PER_STATION * 2 {
            let seed = hash64(
                station.0.wrapping_mul(0x9e37) ^ households.next_id().wrapping_add(house as u64),
            );
            let home = pick_home_tile(home_station.tile, seed, terrain.as_deref());
            let household = households.insert(home, station, tick);

            let dest_station = pick_destination(&station_ids, station, seed);
            let dest_tile = stations
                .get(dest_station)
                .map(|s| s.tile)
                .unwrap_or(home_station.tile);
            let destination = pick_home_tile(dest_tile, seed ^ 0x5bd1, terrain.as_deref());

            let members = 1 + (hash64(seed ^ 0x77) % HOUSEHOLD_MAX as u64) as usize;
            for member in 0..members.min(HOUSEHOLD_MAX) {
                state.next_id = state.next_id.saturating_add(1);
                let id = PeepId(state.next_id);
                households.add_member(household, id);

                let routine = Routine::from_seed(
                    hash64(id.0 ^ 0xbeef),
                    home,
                    station,
                    destination,
                    dest_station,
                );
                let peep = Peep {
                    id,
                    name: full_name(id.0.wrapping_add(member as u64), household.0),
                    home,
                    mood: Mood::Content,
                    household,
                    body: BodyType::from_seed(id.0),
                    portrait: portrait_variant(id.0),
                    moved_in_tick: tick,
                };
                commands.spawn((
                    peep,
                    routine,
                    Journey::new(&routine),
                    PeepPosition::at_tile(home, id.0),
                    JourneyMemory::default(),
                    WaitingAtStation::at(station),
                    PeepDetail::Abstract,
                    // A recomputable cache, never saved — see `peeps::walk`.
                    WalkRoute::default(),
                ));
                seeded += 1;
            }
            house += 1;
        }
        budget.invalidate();
    }
}

/// The town asks for a better station, in its own plain voice.
///
/// Only when the **stop** is what is holding the district back. A district
/// still filling in its streets is growing, not full, and telling the player to
/// rebuild their station would be a lie — 06 §6's invitations are only worth
/// anything while they are true. A stop already at the top of the ladder says
/// nothing either: an invitation the player cannot accept is noise.
fn speak_district_full(feed: &mut ComplaintFeed, station: &Station, here: usize, tick: u64) {
    if station.tier.next_upgrade().is_none() || here < tier_headcount_cap(station) {
        return;
    }
    // Staggered by id, so two full districts never speak on the same tick, and
    // stateless, so it survives a save/load without a cooldown to serialise.
    if tick % DISTRICT_FULL_TALK_TICKS != station.id.0 % DISTRICT_FULL_TALK_TICKS {
        return;
    }
    if feed.town_spoke_recently(
        TalkKind::Opportunity,
        station.id,
        tick,
        DISTRICT_FULL_TALK_TICKS,
    ) {
        return;
    }
    feed.push(ComplaintEntry {
        kind: TalkKind::Opportunity,
        // Whole-sentence town line: no station name, so it reads as the place
        // speaking rather than as a resident being quoted.
        peep_name: format!("{} is full - a bigger station would grow it", station.name),
        station_name: String::new(),
        wait_minutes: 0,
        sim_tick: tick,
        peep_id: None,
        station_id: Some(station.id),
        tile: Some(station.tile),
        count: 1,
    });
}

/// Headcount ceiling this stop's **tier** alone imposes.
///
/// The same expression [`district_capacity`] clamps with, named once so that
/// *"is the station the thing holding this district back?"* has a single answer.
pub fn tier_headcount_cap(station: &Station) -> usize {
    (station.tier.capacity() as usize)
        .saturating_mul(2)
        .max(PEEPS_PER_STATION)
}

/// Headcount a district supports, from its built density and its stop's tier.
///
/// Tier is a hard ceiling, not a modifier: a halt cannot serve a block of flats
/// however built-up the streets around it get. That is the mechanism behind
/// *"a district that has grown to its cap asks for a better station"* (06 §6).
pub fn district_capacity(station: &Station, density: Option<&TownDensity>) -> usize {
    let radius = station.tier.catchment().max(1);
    let tier_cap = tier_headcount_cap(station);
    let Some(density) = density else {
        return PEEPS_PER_STATION.min(tier_cap);
    };
    let mut built = 0.0_f32;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            built += density.get(TileCoord {
                x: station.tile.x + dx,
                y: station.tile.y + dy,
            });
        }
    }
    let supported = PEEPS_PER_STATION as f32 + built * PEEPS_PER_DENSITY;
    (supported.round() as usize)
        .min(tier_cap)
        .min(MAX_PEEPS_PER_STATION)
}

/// A home tile a short walk from the station — walking has to be worth watching.
fn pick_home_tile(station: TileCoord, seed: u64, terrain: Option<&TrackTerrain>) -> TileCoord {
    let span = (HOME_MAX_RADIUS - HOME_MIN_RADIUS + 1).max(1) as u64;
    for attempt in 0..8u64 {
        let h = hash64(seed ^ (attempt << 32));
        let radius = HOME_MIN_RADIUS + (h % span) as i32;
        let (dx, dy) = match (h >> 8) % 4 {
            0 => (
                radius,
                ((h >> 16) % (radius as u64 * 2 + 1)) as i32 - radius,
            ),
            1 => (
                -radius,
                ((h >> 16) % (radius as u64 * 2 + 1)) as i32 - radius,
            ),
            2 => (
                ((h >> 16) % (radius as u64 * 2 + 1)) as i32 - radius,
                radius,
            ),
            _ => (
                ((h >> 16) % (radius as u64 * 2 + 1)) as i32 - radius,
                -radius,
            ),
        };
        let candidate = TileCoord {
            x: station.x + dx,
            y: station.y + dy,
        };
        let ok = match terrain {
            Some(t) => t.contains(candidate) && !t.is_water(candidate),
            None => true,
        };
        if ok {
            return candidate;
        }
    }
    station
}

fn pick_destination(stations: &[StationId], home: StationId, seed: u64) -> StationId {
    let others: Vec<StationId> = stations.iter().copied().filter(|s| *s != home).collect();
    if others.is_empty() {
        return home;
    }
    others[(hash64(seed ^ 0x1f1f) % others.len() as u64) as usize]
}

/// Accumulate platform wait, set mood from experience, and voice it in Town Talk.
pub fn advance_peep_waits(
    mut peeps: Query<(
        &mut Peep,
        &mut WaitingAtStation,
        Option<&Journey>,
        Option<&JourneyMemory>,
    )>,
    stations: Res<StationRegistry>,
    service: Res<StationService>,
    mut feed: ResMut<ComplaintFeed>,
) {
    let tick = service.tick;
    for (mut peep, mut waiting, journey, memory) in peeps.iter_mut() {
        waiting.ticks_since_complaint = waiting.ticks_since_complaint.saturating_add(1);
        waiting.ticks_since_praise = waiting.ticks_since_praise.saturating_add(1);

        // No journey component at all means "standing on a platform" — that is
        // how the wait model behaved before journeys existed, and tests rely on it.
        let on_platform = journey.map_or(true, |j| j.stage.is_waiting());
        let score = service.score(waiting.station).score;

        if on_platform {
            // Poor service → wait accumulates faster (up to 2×); good service slows it.
            let pace = if score >= 80 {
                1
            } else if score >= 40 {
                SIM_SECONDS_PER_TICK
            } else {
                SIM_SECONDS_PER_TICK.saturating_mul(2)
            };
            waiting.wait_secs = waiting.wait_secs.saturating_add(pace);
        } else {
            waiting.wait_secs = waiting.wait_secs.saturating_sub(WAIT_RECOVERY_SECS);
        }

        peep.mood = mood_from_experience(waiting.wait_secs, score, memory);

        let Some(station) = stations.get(waiting.station) else {
            continue;
        };

        // Praise: good service and a short wait — keeps the feed lively, not only nagging.
        if score >= PRAISE_SCORE_MIN
            && waiting.wait_secs < PRAISE_WAIT_MAX_SECS
            && waiting.ticks_since_praise >= PRAISE_COOLDOWN_TICKS
            && peep.mood == Mood::Content
            && journey.map_or(true, |j| j.stage != JourneyStage::AtHome)
        {
            feed.push(ComplaintEntry {
                kind: TalkKind::Praise,
                peep_name: peep.given_name().to_string(),
                station_name: station.name.clone(),
                wait_minutes: 0,
                sim_tick: tick,
                peep_id: Some(peep.id),
                station_id: Some(station.id),
                tile: Some(station.tile),
                count: 1,
            });
            waiting.ticks_since_praise = 0;
        }

        if !on_platform || waiting.wait_secs < COMPLAINT_WAIT_SECS {
            continue;
        }
        if waiting.ticks_since_complaint < COMPLAINT_COOLDOWN_TICKS {
            continue;
        }
        let mins = (waiting.wait_secs / 60).max(1);
        feed.push(ComplaintEntry {
            kind: TalkKind::Complaint,
            peep_name: peep.given_name().to_string(),
            station_name: station.name.clone(),
            wait_minutes: mins,
            sim_tick: tick,
            peep_id: Some(peep.id),
            station_id: Some(station.id),
            tile: Some(station.tile),
            count: 1,
        });
        waiting.ticks_since_complaint = 0;
        peep.mood = Mood::Frustrated;
    }
}

/// Mood from accumulated experience, not from a single number.
pub fn mood_from_experience(wait_secs: u32, score: u8, memory: Option<&JourneyMemory>) -> Mood {
    let patience = memory.map_or(COMPLAINT_WAIT_SECS, |m| m.patience_secs());
    let sour = memory.is_some_and(|m| m.wants_to_leave());
    if sour || wait_secs >= patience || score < 25 {
        Mood::Frustrated
    } else if wait_secs >= patience / 2 || score < 50 || memory.is_some_and(|m| m.bad_streak > 0) {
        Mood::Uneasy
    } else {
        Mood::Content
    }
}

/// Sustained frustration means they move away — as a household, on foot (§4.3).
#[allow(clippy::too_many_arguments)]
pub fn peeps_move_away(
    mut commands: Commands,
    stations: Res<StationRegistry>,
    service: Res<StationService>,
    mut households: ResMut<HouseholdRegistry>,
    mut feed: ResMut<ComplaintFeed>,
    mut budget: ResMut<PeepBudget>,
    mut vacated: Option<ResMut<VacatedHomes>>,
    mut peeps: Query<(Entity, &Peep, &mut Journey, &PeepPosition, &JourneyMemory)>,
) {
    let tick = service.tick;

    // Which households have had enough? Everyone under the roof has to agree —
    // a family leaving is a bigger event than a person leaving.
    let mut fed_up: std::collections::HashMap<HouseholdId, (u32, u32, u32)> =
        std::collections::HashMap::new();
    for (_, peep, _, _, memory) in peeps.iter() {
        let entry = fed_up.entry(peep.household).or_insert((0, 0, 0));
        entry.0 += 1;
        if memory.wants_to_leave() {
            entry.1 += 1;
        }
        let worst = memory
            .recent
            .iter()
            .map(|r| r.total_secs / 60)
            .max()
            .unwrap_or(0);
        entry.2 = entry.2.max(worst);
    }

    let mut leaving: std::collections::HashSet<HouseholdId> = std::collections::HashSet::new();
    let mut staying: std::collections::HashSet<HouseholdId> = std::collections::HashSet::new();
    for (id, (members, unhappy, worst_minutes)) in fed_up {
        if members == 0 || unhappy < members {
            // Somebody in the house has had a good journey again — decline is
            // recoverable at every stage (brief 06 §3.2), so they unpack.
            if let Some(household) = households.get_mut(id) {
                if household.leaving {
                    household.leaving = false;
                    staying.insert(id);
                }
            }
            continue;
        }
        let Some(household) = households.get_mut(id) else {
            continue;
        };
        if household.leaving {
            leaving.insert(id);
            continue;
        }
        household.leaving = true;
        leaving.insert(id);

        let station_name = stations
            .get(household.home_station)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "town".into());
        let minutes = worst_minutes.max(1);
        feed.push(ComplaintEntry {
            kind: TalkKind::Warning,
            peep_name: format!(
                "{} left {station_name} - {minutes} minutes to anywhere",
                household.plural()
            ),
            station_name: String::new(),
            wait_minutes: 0,
            sim_tick: tick,
            peep_id: None,
            station_id: Some(household.home_station),
            tile: Some(household.home),
            count: 1,
        });
    }

    let mut departed: Vec<(HouseholdId, PeepId)> = Vec::new();
    for (entity, peep, mut journey, pos, _) in peeps.iter_mut() {
        if staying.contains(&peep.household) {
            if journey.stage == JourneyStage::LeavingTown {
                journey.set_stage(JourneyStage::AtHome);
            }
            continue;
        }
        if !leaving.contains(&peep.household) {
            continue;
        }
        if journey.stage != JourneyStage::LeavingTown {
            journey.target = stations
                .get(journey.from_station)
                .map(|s| s.tile)
                .unwrap_or(peep.home);
            journey.riding = None;
            journey.set_stage(JourneyStage::LeavingTown);
            continue;
        }
        // Walked out with their luggage — gone once they reach the station.
        let arrived = pos.tile() == journey.target;
        if arrived || journey.stage_ticks > LEAVE_TIMEOUT_TICKS {
            departed.push((peep.household, peep.id));
            commands.entity(entity).despawn();
        }
    }

    for (household, peep) in departed {
        if households.remove_member(household, peep) {
            // The last one out empties the house — and the town is told *which*
            // house, so the boards go up on theirs rather than on whichever lot
            // the density field happened to shed next (06 §3.2).
            if let (Some(gone), Some(vacated)) = (households.remove(household), vacated.as_mut()) {
                vacated.mark(gone.home);
            }
        }
        budget.invalidate();
    }
}

/// Ticks after which a departing peep is considered gone even if they cannot
/// reach their station (no walkable route, station demolished under them).
const LEAVE_TIMEOUT_TICKS: u32 = 20 * super::journey::WALK_TICKS_PER_TILE;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stations::StationServiceScore;
    use crate::track::GROUND_LAYER;
    use bevy_app::App;

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    #[test]
    fn long_wait_emits_complaint() {
        let mut app = App::new();
        app.init_resource::<ComplaintFeed>()
            .init_resource::<StationRegistry>()
            .init_resource::<StationService>()
            .init_resource::<PeepSpawnState>();

        let station_id = {
            let mut stations = app.world_mut().resource_mut::<StationRegistry>();
            stations.insert("Eastgate", tile(4, 4), GROUND_LAYER)
        };
        {
            let mut service = app.world_mut().resource_mut::<StationService>();
            service.scores.insert(
                station_id,
                StationServiceScore {
                    score: 10,
                    ..Default::default()
                },
            );
        }

        app.world_mut().spawn((
            peep_named("Mara Aldertone", PeepId(1), tile(4, 4)),
            WaitingAtStation {
                station: station_id,
                wait_secs: COMPLAINT_WAIT_SECS - 5,
                ticks_since_complaint: COMPLAINT_COOLDOWN_TICKS,
                ticks_since_praise: 0,
            },
        ));

        // One tick at poor-service pace (2×) should cross the threshold.
        app.add_systems(bevy_app::Update, advance_peep_waits);
        app.update();

        let feed = app.world().resource::<ComplaintFeed>();
        assert!(
            !feed.is_empty(),
            "expected a wait complaint after exceeding threshold"
        );
        let line = feed.latest_line().unwrap();
        assert!(
            line.contains("Mara") && line.contains("Eastgate") && line.contains("min"),
            "unexpected complaint line: {line}"
        );
    }

    fn peep_named(name: &str, id: PeepId, home: TileCoord) -> Peep {
        Peep {
            id,
            name: name.into(),
            home,
            mood: Mood::Content,
            household: HouseholdId(1),
            body: BodyType::Slight,
            portrait: 0,
            moved_in_tick: 0,
        }
    }

    #[test]
    fn full_names_split_into_given_and_family() {
        let p = peep_named("Mara Aldertone", PeepId(1), tile(0, 0));
        assert_eq!(p.given_name(), "Mara");
        assert_eq!(p.family_name(), "Aldertone");
        assert_eq!(p.name, "Mara Aldertone");
    }

    #[test]
    fn tenure_line_counts_days() {
        let p = peep_named("Mara Aldertone", PeepId(1), tile(0, 0));
        assert_eq!(
            p.tenure_line("Eastgate", 0),
            "Mara has just moved to Eastgate."
        );
        assert_eq!(
            p.tenure_line("Eastgate", super::super::TICKS_PER_DAY * 14),
            "Mara has lived in Eastgate for 14 days."
        );
    }

    #[test]
    fn wait_only_accrues_on_the_platform() {
        let mut app = App::new();
        app.init_resource::<ComplaintFeed>()
            .init_resource::<StationRegistry>()
            .init_resource::<StationService>();
        let station_id = {
            let mut stations = app.world_mut().resource_mut::<StationRegistry>();
            stations.insert("Eastgate", tile(2, 2), GROUND_LAYER)
        };

        let routine = Routine::from_seed(1, tile(2, 2), station_id, tile(9, 9), StationId(99));
        let mut at_home = Journey::new(&routine);
        at_home.set_stage(JourneyStage::AtHome);
        let home_entity = app
            .world_mut()
            .spawn((
                peep_named("Theo Finch", PeepId(2), tile(2, 2)),
                WaitingAtStation {
                    station: station_id,
                    wait_secs: 600,
                    ticks_since_complaint: 0,
                    ticks_since_praise: 0,
                },
                at_home,
            ))
            .id();

        let mut on_platform = Journey::new(&routine);
        on_platform.set_stage(JourneyStage::WaitingOnPlatform);
        let platform_entity = app
            .world_mut()
            .spawn((
                peep_named("Nia Rowe", PeepId(3), tile(2, 2)),
                WaitingAtStation {
                    station: station_id,
                    wait_secs: 600,
                    ticks_since_complaint: 0,
                    ticks_since_praise: 0,
                },
                on_platform,
            ))
            .id();

        app.add_systems(bevy_app::Update, advance_peep_waits);
        app.update();

        let home_wait = app
            .world()
            .entity(home_entity)
            .get::<WaitingAtStation>()
            .unwrap()
            .wait_secs;
        let platform_wait = app
            .world()
            .entity(platform_entity)
            .get::<WaitingAtStation>()
            .unwrap()
            .wait_secs;
        assert!(home_wait < 600, "a peep at home should not be waiting");
        assert!(platform_wait > 600, "a peep on the platform should be");
    }

    #[test]
    fn mood_comes_from_history_not_just_the_current_wait() {
        use super::super::memory::{JourneyOutcome, JourneyRecord};

        let mut happy = JourneyMemory::default();
        for _ in 0..4 {
            happy.record(JourneyRecord {
                from: StationId(1),
                to: StationId(2),
                wait_secs: 30,
                total_secs: 120,
                outcome: JourneyOutcome::Good,
                ended_tick: 0,
            });
        }
        let mut sour = JourneyMemory::default();
        for _ in 0..4 {
            sour.record(JourneyRecord {
                from: StationId(1),
                to: StationId(2),
                wait_secs: 900,
                total_secs: 1800,
                outcome: JourneyOutcome::GaveUp,
                ended_tick: 0,
            });
        }

        let wait = 8 * 60;
        assert_eq!(
            mood_from_experience(wait, 70, Some(&happy)),
            Mood::Content,
            "four good commutes should buy tolerance for one slow morning"
        );
        assert_eq!(
            mood_from_experience(wait, 70, Some(&sour)),
            Mood::Frustrated,
            "four bad commutes should leave them frustrated"
        );
    }

    #[test]
    fn homes_sit_a_short_walk_from_the_station() {
        for seed in 0..64u64 {
            let home = pick_home_tile(tile(20, 20), seed, None);
            let d = (home.x - 20).abs().max((home.y - 20).abs());
            assert!(
                (HOME_MIN_RADIUS..=HOME_MAX_RADIUS).contains(&d),
                "home {home:?} is {d} tiles from the station"
            );
        }
    }

    #[test]
    fn destinations_avoid_the_home_station_when_possible() {
        let ids = vec![StationId(1), StationId(2), StationId(3)];
        for seed in 0..32u64 {
            assert_ne!(pick_destination(&ids, StationId(1), seed), StationId(1));
        }
        // A one-station town has nowhere else to go; the trip becomes a walk.
        assert_eq!(
            pick_destination(&[StationId(1)], StationId(1), 5),
            StationId(1)
        );
    }

    fn station_at(tile: TileCoord, tier: crate::stations::StationTier) -> Station {
        let mut reg = StationRegistry::new();
        let id = reg.insert_tier("Eastgate", tile, GROUND_LAYER, tier, 0);
        reg.get(id).cloned().expect("station")
    }

    #[test]
    fn district_capacity_follows_built_density_and_station_tier() {
        use crate::stations::StationTier;

        let at = tile(20, 20);
        let station = station_at(at, StationTier::Station);
        assert_eq!(district_capacity(&station, None), PEEPS_PER_STATION);

        let mut density = TownDensity::default();
        assert_eq!(
            district_capacity(&station, Some(&density)),
            PEEPS_PER_STATION,
            "an unbuilt district supports only its starting families"
        );

        let radius = StationTier::Interchange.catchment();
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                density.set(tile(at.x + dx, at.y + dy), 0.9);
            }
        }
        let thick = district_capacity(&station, Some(&density));
        assert!(
            thick > PEEPS_PER_STATION,
            "a thickened district must support more people ({thick})"
        );
        assert!(thick <= MAX_PEEPS_PER_STATION, "town scale, not city scale");

        // A halt caps the same streets lower — that is what makes the district
        // ask for a better station instead of growing forever.
        let halt = station_at(at, StationTier::Halt);
        let interchange = station_at(at, StationTier::Interchange);
        assert!(
            district_capacity(&halt, Some(&density))
                < district_capacity(&interchange, Some(&density)),
            "station tier must be a real ceiling on district headcount"
        );
    }

    #[test]
    fn an_emptied_district_only_repopulates_once_service_returns() {
        let mut app = App::new();
        app.init_resource::<StationRegistry>()
            .init_resource::<StationService>()
            .init_resource::<PeepSpawnState>()
            .init_resource::<HouseholdRegistry>()
            .init_resource::<ComplaintFeed>()
            .init_resource::<PeepBudget>();
        let station = {
            let mut stations = app.world_mut().resource_mut::<StationRegistry>();
            stations.insert("Westbrook", tile(8, 8), GROUND_LAYER)
        };
        // Pretend everyone already left this district.
        app.world_mut()
            .resource_mut::<PeepSpawnState>()
            .spawned_for
            .insert(station);
        app.world_mut()
            .resource_mut::<StationService>()
            .ensure(station)
            .score = 10;

        app.add_systems(bevy_app::Update, spawn_peep_households);
        app.update();
        assert_eq!(
            app.world().resource::<HouseholdRegistry>().population(),
            0,
            "nobody moves into a district with no service"
        );

        app.world_mut()
            .resource_mut::<StationService>()
            .ensure(station)
            .score = REPOPULATE_SCORE + 10;
        app.update();
        assert!(
            app.world().resource::<HouseholdRegistry>().population() > 0,
            "restoring service must bring people back"
        );
    }

    /// A district filled to its stop's ceiling, with the stagger gate open.
    fn a_full_district(tier: crate::stations::StationTier) -> App {
        let mut app = App::new();
        app.init_resource::<StationRegistry>()
            .init_resource::<StationService>()
            .init_resource::<PeepSpawnState>()
            .init_resource::<HouseholdRegistry>()
            .init_resource::<ComplaintFeed>()
            .init_resource::<PeepBudget>();
        let station = {
            let mut stations = app.world_mut().resource_mut::<StationRegistry>();
            stations.insert_tier("Eastgate", tile(10, 10), GROUND_LAYER, tier, 0)
        };
        let cap = {
            let stations = app.world().resource::<StationRegistry>();
            tier_headcount_cap(stations.get(station).expect("station"))
        };
        {
            let mut households = app.world_mut().resource_mut::<HouseholdRegistry>();
            let id = households.insert(tile(11, 11), station, 0);
            for i in 0..cap {
                households.add_member(id, PeepId(i as u64 + 1));
            }
        }
        // The gate is `tick % N == id % N`; both are small, so this opens it.
        app.world_mut().resource_mut::<StationService>().tick = station.0;
        app.add_systems(bevy_app::Update, spawn_peep_households);
        app
    }

    fn talk_lines(app: &App) -> Vec<String> {
        app.world()
            .resource::<ComplaintFeed>()
            .iter()
            .map(|e| e.display_line())
            .collect()
    }

    /// 06 §6 — *"a district that has grown to its cap asks for a better
    /// station."* It used to just stop growing, silently.
    #[test]
    fn a_district_at_its_stations_ceiling_asks_for_a_bigger_one() {
        use crate::stations::StationTier;

        let mut app = a_full_district(StationTier::Halt);
        app.update();
        let talk = talk_lines(&app);
        assert!(
            talk.iter()
                .any(|l| l == "Eastgate is full - a bigger station would grow it"),
            "the town never asked: {talk:?}"
        );
        assert!(talk.iter().all(|l| l.is_ascii()), "{talk:?}");

        // And it asks once, not every tick.
        for _ in 0..4 {
            app.update();
        }
        let asks = talk_lines(&app)
            .iter()
            .filter(|l| l.contains("is full"))
            .count();
        assert_eq!(asks, 1, "the invitation must not become a nag: {asks} lines");
    }

    #[test]
    fn a_stop_at_the_top_of_the_ladder_says_nothing() {
        use crate::stations::StationTier;

        // There is no bigger station to ask for, and an invitation the player
        // cannot accept is noise.
        let mut app = a_full_district(StationTier::Interchange);
        app.update();
        assert!(
            !talk_lines(&app).iter().any(|l| l.contains("is full")),
            "an interchange asked to be upgraded to nothing"
        );
    }

    #[test]
    fn a_growing_district_is_not_told_its_station_is_too_small() {
        use crate::stations::StationTier;

        // Room left under the tier ceiling means the streets are what is
        // filling in, not the platform that is short.
        let mut app = a_full_district(StationTier::Station);
        {
            let mut households = app.world_mut().resource_mut::<HouseholdRegistry>();
            let id = households.iter().map(|h| h.id).next().expect("household");
            households.remove_member(id, PeepId(1));
        }
        app.update();
        assert!(
            !talk_lines(&app).iter().any(|l| l.contains("is full")),
            "a district under its ceiling must not be told to rebuild the stop"
        );
    }

    #[test]
    fn a_fed_up_household_leaves_together_and_town_talk_names_them() {
        use super::super::memory::{JourneyOutcome, JourneyRecord};

        let mut app = App::new();
        app.init_resource::<StationRegistry>()
            .init_resource::<StationService>()
            .init_resource::<HouseholdRegistry>()
            .init_resource::<ComplaintFeed>()
            .init_resource::<PeepBudget>();
        let station = {
            let mut stations = app.world_mut().resource_mut::<StationRegistry>();
            stations.insert("Westbrook", tile(6, 6), GROUND_LAYER)
        };
        let household = {
            let mut households = app.world_mut().resource_mut::<HouseholdRegistry>();
            let id = households.insert(tile(6, 6), station, 0);
            households.add_member(id, PeepId(1));
            households.add_member(id, PeepId(2));
            id
        };

        let mut sour = JourneyMemory::default();
        for _ in 0..super::super::memory::BAD_JOURNEYS_TO_LEAVE {
            sour.record(JourneyRecord {
                from: station,
                to: StationId(9),
                wait_secs: 900,
                total_secs: 22 * 60,
                outcome: JourneyOutcome::GaveUp,
                ended_tick: 0,
            });
        }
        let routine = Routine::from_seed(1, tile(6, 6), station, tile(9, 9), StationId(9));
        for id in [PeepId(1), PeepId(2)] {
            app.world_mut().spawn((
                Peep::new(id, "Mara Aldertone", tile(6, 6), household, 0),
                routine,
                Journey::new(&routine),
                PeepPosition::at_tile(tile(6, 6), id.0),
                sour.clone(),
                WaitingAtStation::at(station),
                PeepDetail::Full,
            ));
        }

        app.init_resource::<VacatedHomes>();
        app.add_systems(bevy_app::Update, peeps_move_away);
        app.update();

        let talk: Vec<String> = app
            .world()
            .resource::<ComplaintFeed>()
            .iter()
            .map(|e| e.display_line())
            .collect();
        assert!(
            talk.iter()
                .any(|l| l.contains("left Westbrook") && l.contains("minutes to anywhere")),
            "Town Talk did not name the departing household: {talk:?}"
        );

        // They walk out with their luggage first, then they are gone.
        let mut stages = app.world_mut().query::<&Journey>();
        assert!(stages
            .iter(app.world())
            .all(|j| j.stage == JourneyStage::LeavingTown));

        app.update();
        let mut remaining = app.world_mut().query::<&Peep>();
        assert_eq!(
            remaining.iter(app.world()).count(),
            0,
            "the whole family should have gone, not just one of them"
        );
        assert_eq!(app.world().resource::<HouseholdRegistry>().len(), 0);

        // …and the town is told which house it was, so the boards go up on
        // theirs rather than on whichever lot the density field sheds next.
        let vacated = app.world_mut().resource_mut::<VacatedHomes>().drain();
        assert_eq!(
            vacated,
            vec![tile(6, 6)],
            "a named departure must name its own home tile"
        );
    }

    #[test]
    fn seeding_creates_named_households_that_share_a_home() {
        let mut app = App::new();
        app.init_resource::<ComplaintFeed>()
            .init_resource::<StationRegistry>()
            .init_resource::<StationService>()
            .init_resource::<PeepSpawnState>()
            .init_resource::<HouseholdRegistry>()
            .init_resource::<PeepBudget>();
        {
            let mut stations = app.world_mut().resource_mut::<StationRegistry>();
            stations.insert("Eastgate", tile(10, 10), GROUND_LAYER);
            stations.insert("Millhaven", tile(30, 30), GROUND_LAYER);
        }
        app.add_systems(bevy_app::Update, spawn_peep_households);
        app.update();

        let population = {
            let households = app.world().resource::<HouseholdRegistry>();
            assert!(households.population() >= PEEPS_PER_STATION * 2);
            for household in households.iter() {
                assert!(!household.members.is_empty());
                assert!(!household.family.is_empty());
            }
            households.population()
        };

        let mut peeps = app.world_mut().query::<(&Peep, &Routine, &Journey)>();
        let mut count = 0;
        for (peep, routine, journey) in peeps.iter(app.world()) {
            count += 1;
            assert!(
                peep.name.contains(' '),
                "expected a full name: {}",
                peep.name
            );
            assert_eq!(peep.home, routine.home);
            assert_eq!(journey.stage, JourneyStage::AtHome);
        }
        assert_eq!(count, population);
    }
}
