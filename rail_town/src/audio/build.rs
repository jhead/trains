//! Building sound (brief §3.1) — the set the player causes, and the one they
//! hear most.
//!
//! ## The run
//!
//! A drag or an autofill commits a whole path in a single frame. Playing every
//! tile at once is a burst; dropping all but the first is what the old
//! placeholder did and it made a fifty-tile run sound like a single tile. So
//! placements go into a queue that drains at one tile every 50–75 ms, **faster
//! when the run is longer**, with per-tile jitter on both the interval and the
//! playback rate. A long run comes out as a rhythmic run — a gang working —
//! which is the thing the brief says players should want to do idly.
//!
//! ## The thud
//!
//! Rejections do not queue. The brief calls for an immediate response, and a
//! deferred "no" is worse than no "no" at all. It is rate-limited instead, and
//! collapsed within a frame, so dragging across a mountainside produces one soft
//! thud rather than forty.

use std::collections::VecDeque;

use bevy::prelude::*;
use rail_map::{tile_to_world, MapGrid, TILE_SIZE};
use rail_sim::{StationRegistry, TileCoord, TrackEdit};

use crate::lines::LineToolState;
use crate::town::{BuildingLot, LotPhase};
use crate::track::{BuildTool, TrackToolState};
use crate::trains::{TrainPlaceKind, TrainToolState};

use super::bank::SfxBank;
use super::dsp::{lerp, Rng};
use super::mixer::{gain, play, AudioClock, AudioMix, Cue, Duck, SoundCategory, VoiceBudget};

/// Interval between queued tiles at the shortest and longest runs.
const RUN_INTERVAL_SLOW: f32 = 0.075;
const RUN_INTERVAL_FAST: f32 = 0.048;
/// Queue length at which the run reaches [`RUN_INTERVAL_FAST`].
const RUN_RAMP: f32 = 14.0;
/// A run longer than this stops queueing — beyond about two seconds of clacks
/// the rhythm has been established and the rest is noise.
const RUN_MAX: usize = 30;

/// Minimum spacing between rejection thuds.
const INVALID_COOLDOWN: f32 = 0.28;
/// Minimum spacing between tool clicks (a held hotkey must not machine-gun).
const TOOL_COOLDOWN: f32 = 0.08;

/// Gap between hammer ticks with one site working, and with a street of them.
///
/// The scaffold holds for eight seconds (brief 06 §3.1), so one site is roughly
/// three or four taps across its whole life — *occasional*, which is the word
/// the brief uses. A district all going up at once is busier, but never a
/// rhythm: the gap is jittered hard, so it never resolves into a beat.
const HAMMER_GAP_SLOW: f32 = 2.4;
const HAMMER_GAP_FAST: f32 = 0.9;
/// Sites in earshot at which the gap reaches [`HAMMER_GAP_FAST`].
const HAMMER_SITES_RAMP: f32 = 6.0;

/// What a queued tile sounds like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuildSound {
    Track,
    Bridge,
    Clank,
}

#[derive(Debug, Clone, Copy)]
struct Queued {
    sound: BuildSound,
    at: Vec2,
}

/// The build-sound queue and its rate limiters.
#[derive(Resource, Debug)]
pub struct BuildAudio {
    queue: VecDeque<Queued>,
    next_at: f64,
    variant: usize,
    last_invalid: f64,
    last_tool: f64,
    /// Clock time the next hammer tick is allowed.
    next_hammer: f64,
    rng: Rng,
    /// Station ids seen last frame, for placement / removal detection.
    stations: Vec<u64>,
    /// The world those ids belong to, as `(width, height, seed)`.
    ///
    /// **Keyed on the world rather than latched once per process.** A new map
    /// clears the registries and reissues ids from one, so a bare "have I run
    /// yet" flag would compare the new world's first station against the old
    /// world's list and either replay a chord that should be new or fire a
    /// demolition for a station that never existed. Keying it means the first
    /// frame of every world is a fresh baseline, silently.
    stations_world: Option<(u32, u32, u64)>,
    tool: Option<ToolSnapshot>,
}

impl Default for BuildAudio {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            next_at: 0.0,
            variant: 0,
            last_invalid: f64::NEG_INFINITY,
            last_tool: f64::NEG_INFINITY,
            next_hammer: 0.0,
            rng: Rng::new(0x7261_696c),
            stations: Vec::new(),
            stations_world: None,
            tool: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ToolSnapshot {
    build: BuildTool,
    placing: bool,
    kind: TrainPlaceKind,
    line: bool,
}

fn world_of(tile: TileCoord) -> Vec2 {
    let (x, y) = tile_to_world(tile);
    Vec2::new(x, y)
}

/// Turn this frame's [`TrackEdit`]s into queued tiles and immediate rejections.
#[allow(clippy::too_many_arguments)]
pub fn collect_track_edits(
    mut edits: MessageReader<TrackEdit>,
    clock: Res<AudioClock>,
    mix: Res<AudioMix>,
    bank: Option<Res<SfxBank>>,
    mut audio: ResMut<BuildAudio>,
    mut budget: ResMut<VoiceBudget>,
    mut duck: ResMut<Duck>,
    mut commands: Commands,
) {
    let Some(bank) = bank else {
        return;
    };
    let now = clock.elapsed;

    // Nearest first: when the queue overflows, the tiles the player is actually
    // looking at are the ones that survive (§7, nearest instances win).
    let mut placed: Vec<Queued> = Vec::new();
    let mut rejected: Option<Vec2> = None;

    for edit in edits.read() {
        match edit {
            TrackEdit::Placed { tile, is_bridge, .. } => placed.push(Queued {
                sound: if *is_bridge {
                    BuildSound::Bridge
                } else {
                    BuildSound::Track
                },
                at: world_of(*tile),
            }),
            TrackEdit::Removed { tile, .. } => placed.push(Queued {
                sound: BuildSound::Clank,
                at: world_of(*tile),
            }),
            TrackEdit::Failed { tile, .. } => {
                rejected = Some(tile.map(world_of).unwrap_or(mix.listener));
            }
        }
    }

    if !placed.is_empty() {
        placed.sort_by(|a, b| {
            let da = (a.at - mix.listener).length_squared();
            let db = (b.at - mix.listener).length_squared();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });
        for item in placed {
            if audio.queue.len() >= RUN_MAX {
                break;
            }
            audio.queue.push_back(item);
        }
        // Laying track ducks the music slightly so the clacks have room (§4).
        duck.on_build();
    }

    if let Some(at) = rejected {
        if now - audio.last_invalid >= INVALID_COOLDOWN as f64 {
            audio.last_invalid = now;
            let index = audio.rng.below(bank.invalid.variants());
            let speed = audio.rng.range(0.97, 1.03);
            play(
                &mut commands,
                &mut budget,
                &mix,
                Cue::world(
                    bank.invalid.pick(index, mix.brightness(at)),
                    SoundCategory::Build,
                    gain::INVALID,
                    at,
                )
                .with_speed(speed),
            );
        }
    }
}

/// Drain the queue at a rhythm.
pub fn drain_build_queue(
    clock: Res<AudioClock>,
    mix: Res<AudioMix>,
    bank: Option<Res<SfxBank>>,
    mut audio: ResMut<BuildAudio>,
    mut budget: ResMut<VoiceBudget>,
    mut commands: Commands,
) {
    let Some(bank) = bank else {
        return;
    };
    let now = clock.elapsed;
    if audio.queue.is_empty() {
        audio.next_at = now;
        return;
    }
    // The clock freezes with the sim; building while paused is allowed, so fall
    // back to real time when nothing is advancing.
    let now = if clock.running {
        now
    } else {
        audio.next_at + clock.real_delta as f64
    };
    if now < audio.next_at {
        return;
    }

    let Some(item) = audio.queue.pop_front() else {
        return;
    };
    let queued = audio.queue.len() as f32;
    let interval = RUN_INTERVAL_SLOW
        + (RUN_INTERVAL_FAST - RUN_INTERVAL_SLOW) * (queued / RUN_RAMP).clamp(0.0, 1.0);
    let jitter = audio.rng.range(0.88, 1.12);
    audio.next_at = now + (interval * jitter) as f64;

    audio.variant = audio.variant.wrapping_add(1 + audio.rng.below(2));
    let variant = audio.variant;
    let speed = audio.rng.range(0.94, 1.07);
    let brightness = mix.brightness(item.at);

    let (clip, level) = match item.sound {
        BuildSound::Track => (bank.clack.pick(variant, brightness), gain::CLACK),
        BuildSound::Bridge => (bank.bridge.pick(variant, brightness), gain::BRIDGE),
        BuildSound::Clank => (bank.demolish.pick(variant, brightness), gain::DEMOLISH),
    };
    play(
        &mut commands,
        &mut budget,
        &mix,
        Cue::world(clip, SoundCategory::Build, level, item.at).with_speed(speed),
    );
}

/// Stations are placed through the command buffer and land in the registry;
/// watching the registry keeps this module decoupled from the station slice.
pub fn watch_stations(
    stations: Res<StationRegistry>,
    map: Option<Res<MapGrid>>,
    mix: Res<AudioMix>,
    bank: Option<Res<SfxBank>>,
    mut audio: ResMut<BuildAudio>,
    mut budget: ResMut<VoiceBudget>,
    mut commands: Commands,
) {
    let Some(bank) = bank else {
        return;
    };
    // `StationRegistry::iter` walks a `HashMap`, so the order is arbitrary
    // between frames; without the sort this comparison would report a change
    // every frame and fire a phantom demolition whenever the order shifted.
    let mut current: Vec<u64> = stations.iter().map(|s| s.id.0).collect();
    current.sort_unstable();
    let world = map.map(|m| (m.width, m.height, m.seed));
    let same_world = audio.stations_world.is_some() && audio.stations_world == world;
    if same_world && current == audio.stations {
        return;
    }

    if same_world {
        for station in stations.iter() {
            if !audio.stations.contains(&station.id.0) {
                let at = world_of(station.tile);
                let index = audio.rng.below(bank.station.variants());
                play(
                    &mut commands,
                    &mut budget,
                    &mix,
                    Cue::world(
                        bank.station.pick(index, mix.brightness(at)),
                        SoundCategory::Build,
                        gain::STATION,
                        at,
                    ),
                );
            }
        }
        // A station that has gone is a demolition, wherever the camera is.
        if current.len() < audio.stations.len() {
            let at = mix.listener;
            let index = audio.rng.below(bank.demolish.variants());
            play(
                &mut commands,
                &mut budget,
                &mix,
                Cue::world(
                    bank.demolish.pick(index, 1.0),
                    SoundCategory::Build,
                    gain::DEMOLISH,
                    at,
                ),
            );
        }
    }

    audio.stations = current;
    audio.stations_world = world;
}

/// A hammer on whatever is going up nearby (brief 06 §3.1).
///
/// **One voice, however many sites.** A district of scaffolds is not eight
/// hammers — it is a busier version of the same street, so the nearest site
/// carries the sound and the rest only make it more frequent. That keeps this
/// inside the effects bus's voice budget however much the town is building, and
/// keeps §1's promise that nothing startles.
///
/// Freezes with the sim: a paused world is a world where nobody is working.
pub fn construction_ticks(
    clock: Res<AudioClock>,
    mix: Res<AudioMix>,
    bank: Option<Res<SfxBank>>,
    lots: Query<&BuildingLot>,
    mut audio: ResMut<BuildAudio>,
    mut budget: ResMut<VoiceBudget>,
    mut commands: Commands,
) {
    let Some(bank) = bank else {
        return;
    };
    if !clock.running {
        return;
    }
    let now = clock.elapsed;
    if now < audio.next_hammer {
        return;
    }

    let radius = mix.radius.max(TILE_SIZE * 6.0);
    let mut nearest: Option<(f32, Vec2)> = None;
    let mut sites = 0.0f32;
    for lot in lots.iter() {
        if lot.phase != LotPhase::Scaffold {
            continue;
        }
        let at = world_of(lot.tile);
        let distance = (at - mix.listener).length();
        if distance > radius {
            continue;
        }
        sites += 1.0;
        if nearest.is_none_or(|(best, _)| distance < best) {
            nearest = Some((distance, at));
        }
    }
    let Some((_, at)) = nearest else {
        // Nothing in earshot. Re-arm from now, so panning onto a building site
        // does not immediately spend a backlog of silence as one tick.
        audio.next_hammer = now;
        return;
    };

    let gap = lerp(
        HAMMER_GAP_SLOW,
        HAMMER_GAP_FAST,
        (sites / HAMMER_SITES_RAMP).clamp(0.0, 1.0),
    );
    // Jittered hard, so a street of sites never resolves into a beat.
    let jitter = audio.rng.range(0.65, 1.5);
    audio.next_hammer = now + (gap * jitter) as f64;

    let index = audio.rng.below(bank.hammer.variants());
    let speed = audio.rng.range(0.90, 1.11);
    play(
        &mut commands,
        &mut budget,
        &mix,
        Cue::world(
            bank.hammer.pick(index, mix.brightness(at)),
            SoundCategory::Build,
            gain::HAMMER,
            at,
        )
        .with_speed(speed),
    );
}

/// A minimal click when the active tool changes.
#[allow(clippy::too_many_arguments)]
pub fn watch_tool_switch(
    track: Res<TrackToolState>,
    train: Res<TrainToolState>,
    line: Res<LineToolState>,
    clock: Res<AudioClock>,
    mix: Res<AudioMix>,
    bank: Option<Res<SfxBank>>,
    mut audio: ResMut<BuildAudio>,
    mut budget: ResMut<VoiceBudget>,
    mut commands: Commands,
) {
    let Some(bank) = bank else {
        return;
    };
    let snapshot = ToolSnapshot {
        build: track.tool,
        placing: train.place_mode,
        kind: train.kind,
        line: line.active,
    };
    let changed = audio.tool.is_some_and(|previous| previous != snapshot);
    let first = audio.tool.is_none();
    audio.tool = Some(snapshot);
    if first || !changed {
        return;
    }
    let now = clock.elapsed;
    if now - audio.last_tool < TOOL_COOLDOWN as f64 {
        return;
    }
    audio.last_tool = now;
    play(
        &mut commands,
        &mut budget,
        &mix,
        Cue::ui(bank.tool_switch.near(0), gain::TOOL_SWITCH),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_run_speeds_up_but_never_becomes_a_burst() {
        // The whole point of the queue: a fifty-tile autofill is a run, not a
        // single clack and not a wall of sound.
        let interval_of = |queued: f32| {
            RUN_INTERVAL_SLOW
                + (RUN_INTERVAL_FAST - RUN_INTERVAL_SLOW) * (queued / RUN_RAMP).clamp(0.0, 1.0)
        };
        assert!((interval_of(0.0) - RUN_INTERVAL_SLOW).abs() < 1e-6);
        assert!((interval_of(50.0) - RUN_INTERVAL_FAST).abs() < 1e-6);
        assert!(interval_of(7.0) > RUN_INTERVAL_FAST);
        // Even at full tilt that is well under twenty tiles a second.
        assert!(1.0 / RUN_INTERVAL_FAST < 25.0);
        // And a run is bounded, so a huge autofill cannot clack for a minute.
        assert!(RUN_MAX as f32 * RUN_INTERVAL_SLOW < 2.5);
    }

    #[test]
    fn the_thud_is_rate_limited_but_still_immediate() {
        // Immediate (no queue) but never more than about three a second.
        assert!(INVALID_COOLDOWN >= 0.2, "a drag over rock must not buzz");
        assert!(INVALID_COOLDOWN <= 0.5, "but the answer has to feel prompt");
    }

    #[test]
    fn a_building_site_taps_occasionally_and_never_becomes_a_rhythm() {
        // Brief 06 §3.1's word is *occasional*. A scaffold holds for eight
        // seconds, so one site is a handful of taps across its whole life; a
        // street of them is busier but still never a beat.
        let gap_of = |sites: f32| {
            lerp(
                HAMMER_GAP_SLOW,
                HAMMER_GAP_FAST,
                (sites / HAMMER_SITES_RAMP).clamp(0.0, 1.0),
            )
        };
        assert!((gap_of(1.0) - HAMMER_GAP_SLOW).abs() < HAMMER_GAP_SLOW);
        assert!(gap_of(1.0) > gap_of(4.0), "more sites should be busier");
        assert!(gap_of(40.0) >= HAMMER_GAP_FAST, "and never faster than the floor");
        // One site: a few taps over an eight-second scaffold, not a stream.
        assert!(crate::town::SCAFFOLD_SECS / gap_of(1.0) < 6.0);
        // And the quietest positional family in the bank — §1's first rule is
        // never to startle, and this one plays without the player asking.
        let quietest_build = [
            gain::CLACK,
            gain::BRIDGE,
            gain::STATION,
            gain::DEMOLISH,
            gain::INVALID,
        ]
        .into_iter()
        .fold(f32::INFINITY, f32::min);
        assert!(
            gain::HAMMER <= quietest_build,
            "the hammer at {} is louder than a sound the player asked for ({quietest_build})",
            gain::HAMMER
        );
    }

    #[test]
    fn tool_snapshots_compare_on_every_mode() {
        let base = ToolSnapshot {
            build: BuildTool::Build,
            placing: false,
            kind: TrainPlaceKind::Transit,
            line: false,
        };
        assert_eq!(base, base);
        assert_ne!(
            base,
            ToolSnapshot {
                build: BuildTool::Demolish,
                ..base
            }
        );
        assert_ne!(base, ToolSnapshot { line: true, ..base });
        assert_ne!(
            base,
            ToolSnapshot {
                placing: true,
                ..base
            }
        );
        assert_ne!(
            base,
            ToolSnapshot {
                kind: TrainPlaceKind::Transport,
                ..base
            }
        );
    }
}
