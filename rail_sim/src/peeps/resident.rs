//! Peep components and wait / complaint Advance systems.

use bevy_ecs::prelude::*;

use crate::ids::{StationId, TileCoord};
use crate::stations::{StationRegistry, StationService};

use super::complaints::{ComplaintEntry, ComplaintFeed, TalkKind, COMPLAINT_WAIT_SECS};
use super::{PeepId, PeepSpawnState};

/// Sim-seconds advanced per FixedUpdate tick while a peep is waiting.
pub const SIM_SECONDS_PER_TICK: u32 = 10;

/// How many named residents to attach per station.
pub const PEEPS_PER_STATION: usize = 2;

/// Cooldown (ticks) after a complaint before the same peep can complain again.
const COMPLAINT_COOLDOWN_TICKS: u32 = 90;

/// Cooldown (ticks) after praise before the same peep can praise again.
const PRAISE_COOLDOWN_TICKS: u32 = 180;

/// Service score at or above which good-service praise may fire.
const PRAISE_SCORE_MIN: u8 = 80;

/// Wait must stay under this many sim-seconds for praise.
const PRAISE_WAIT_MAX_SECS: u32 = 4 * 60;

const NAMES: &[&str] = &[
    "Mara", "Jon", "Elise", "Theo", "Nia", "Owen", "Priya", "Sam", "Vera", "Cole", "Asha",
    "Reed",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// A named resident visible in the town.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct Peep {
    pub id: PeepId,
    pub name: String,
    pub home: TileCoord,
    pub mood: Mood,
}

/// Peep currently waiting at a station (accumulates wait time).
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct WaitingAtStation {
    pub station: StationId,
    /// Accumulated wait in sim-seconds.
    pub wait_secs: u32,
    pub ticks_since_complaint: u32,
    pub ticks_since_praise: u32,
}

/// Spawn peeps for any station that does not yet have residents.
pub fn spawn_peeps_for_stations(
    mut commands: Commands,
    stations: Res<StationRegistry>,
    mut state: ResMut<PeepSpawnState>,
) {
    for station in stations.iter() {
        if !state.spawned_for.insert(station.id) {
            continue;
        }
        for _ in 0..PEEPS_PER_STATION {
            state.next_id = state.next_id.saturating_add(1);
            let id = PeepId(state.next_id);
            let name = NAMES[(id.0 as usize - 1) % NAMES.len()].to_string();
            commands.spawn((
                Peep {
                    id,
                    name,
                    home: station.tile,
                    mood: Mood::Content,
                },
                WaitingAtStation {
                    station: station.id,
                    wait_secs: 0,
                    ticks_since_complaint: COMPLAINT_COOLDOWN_TICKS, // allow first complaint
                    ticks_since_praise: PRAISE_COOLDOWN_TICKS,
                },
            ));
        }
    }
}

/// Accumulate wait time from service quality; emit complaints / praise into Town Talk.
pub fn advance_peep_waits(
    mut peeps: Query<(&mut Peep, &mut WaitingAtStation)>,
    stations: Res<StationRegistry>,
    service: Res<StationService>,
    mut feed: ResMut<ComplaintFeed>,
) {
    let tick = service.tick;
    for (mut peep, mut waiting) in peeps.iter_mut() {
        waiting.ticks_since_complaint = waiting.ticks_since_complaint.saturating_add(1);
        waiting.ticks_since_praise = waiting.ticks_since_praise.saturating_add(1);

        let score = service.score(waiting.station).score;
        // Poor service → wait accumulates faster (up to 2×); good service slows it.
        let pace = if score >= 80 {
            1
        } else if score >= 40 {
            SIM_SECONDS_PER_TICK
        } else {
            SIM_SECONDS_PER_TICK.saturating_mul(2)
        };
        waiting.wait_secs = waiting.wait_secs.saturating_add(pace);

        peep.mood = mood_from_wait(waiting.wait_secs, score);

        let Some(station) = stations.get(waiting.station) else {
            continue;
        };

        // Praise: good service and a short wait — keeps the feed lively, not only nagging.
        if score >= PRAISE_SCORE_MIN
            && waiting.wait_secs < PRAISE_WAIT_MAX_SECS
            && waiting.ticks_since_praise >= PRAISE_COOLDOWN_TICKS
            && peep.mood == Mood::Content
        {
            feed.push(ComplaintEntry {
                kind: TalkKind::Praise,
                peep_name: peep.name.clone(),
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

        if waiting.wait_secs < COMPLAINT_WAIT_SECS {
            continue;
        }
        if waiting.ticks_since_complaint < COMPLAINT_COOLDOWN_TICKS {
            continue;
        }
        let mins = (waiting.wait_secs / 60).max(1);
        feed.push(ComplaintEntry {
            kind: TalkKind::Complaint,
            peep_name: peep.name.clone(),
            station_name: station.name.clone(),
            wait_minutes: mins,
            sim_tick: tick,
            peep_id: Some(peep.id),
            station_id: Some(station.id),
            tile: Some(station.tile),
            count: 1,
        });
        waiting.wait_secs = 0;
        waiting.ticks_since_complaint = 0;
        peep.mood = Mood::Frustrated;
    }
}

fn mood_from_wait(wait_secs: u32, score: u8) -> Mood {
    if wait_secs >= COMPLAINT_WAIT_SECS || score < 25 {
        Mood::Frustrated
    } else if wait_secs >= COMPLAINT_WAIT_SECS / 2 || score < 50 {
        Mood::Uneasy
    } else {
        Mood::Content
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stations::StationServiceScore;
    use crate::track::GROUND_LAYER;
    use bevy_app::App;

    #[test]
    fn long_wait_emits_complaint() {
        let mut app = App::new();
        app.init_resource::<ComplaintFeed>()
            .init_resource::<StationRegistry>()
            .init_resource::<StationService>()
            .init_resource::<PeepSpawnState>();

        let station_id = {
            let mut stations = app.world_mut().resource_mut::<StationRegistry>();
            stations.insert("Eastgate", TileCoord { x: 4, y: 4 }, GROUND_LAYER)
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
            Peep {
                id: PeepId(1),
                name: "Mara".into(),
                home: TileCoord { x: 4, y: 4 },
                mood: Mood::Content,
            },
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
}
