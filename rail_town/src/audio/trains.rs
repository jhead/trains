//! Train sound (brief §3.2) — the noise the player's network makes on its own.
//!
//! Each audible train owns one endless [`VoiceKind::Rolling`] voice whose speed,
//! mass and distance are written every frame. Transit and freight are the same
//! generator with different mass: freight sits lower, its sleepers come slower
//! and deeper, and it grows a groan that a railcar never has. A heavy goods
//! train is audibly heavy.
//!
//! Departures, arrivals and whistles are one-shots keyed off the sim's dwell
//! counter. **The whistle is deliberately rare.** A distant whistle across a
//! valley is one of the best sounds available to this game and it is ruined by
//! overuse, so it needs a departure, a long global cooldown, and a coin toss.
//!
//! Level crossings do not exist in the sim, so a crossing is inferred: a track
//! tile inside a built-up district is where a road would meet it. The bell rings
//! as a train approaches one and attenuates quickly, which is the behaviour the
//! brief asks for whether or not the sim ever grows real barriers.

use std::collections::HashMap;

use bevy::audio::{AudioSink, SpatialAudioSink};
use bevy::prelude::*;
use rail_map::tile_to_world;
use rail_sim::{
    ticks_for_piece, TileOccupancy, TownDensity, TrackNetwork, Train, TrainId, TrainKind,
    TrainLocation,
};

use super::bank::SfxBank;
use super::dsp::{approach, lerp, Rng};
use super::mixer::{gain, play, AudioClock, AudioMix, Cue, SoundCategory, VoiceBudget};
use super::voice::{LiveVoice, VoiceKind};
use super::voices::{spawn_positional, VoiceHandle};

/// Concurrent rolling voices. The nearest trains win; the rest of a busy
/// network is carried by the town bed, which is where a distant network belongs.
const MAX_ROLLING: usize = 6;

/// Fade time for a rolling voice arriving or leaving.
const ROLL_FADE_SECS: f32 = 0.45;

/// Ticks-per-tile of the fastest thing on the rails, for normalising speed.
const FASTEST_TICKS: f32 = 3.0;

/// Town density at which a track tile counts as a level crossing.
const CROSSING_DENSITY: f32 = 0.35;
/// Tiles ahead that count as "approaching" a crossing.
const CROSSING_LOOKAHEAD: usize = 3;
/// Interval between bell strikes.
const CROSSING_INTERVAL: f32 = 1.05;

/// Global cooldown between whistles, and the chance one sounds at all.
const WHISTLE_COOLDOWN: f32 = 34.0;
const WHISTLE_CHANCE: f32 = 0.4;

/// Speed of sound in world units per second, for the doppler trim. Tuned for a
/// shift you notice and never for one that swoops.
const DOPPLER_C: f32 = 2600.0;

#[derive(Debug, Clone, Copy)]
struct TrainState {
    id: TrainId,
    kind: TrainKind,
    at: Vec2,
    /// `0.0` stopped, `1.0` as fast as anything on the network goes.
    speed: f32,
    dwelling: bool,
    near_crossing: bool,
}

#[derive(Debug)]
struct RollingVoice {
    handle: VoiceHandle,
    /// Smoothed radial velocity, for the doppler trim.
    closing: f32,
    last_dist: f32,
    keep: bool,
}

#[derive(Resource, Debug)]
pub struct TrainAudio {
    voices: HashMap<u64, RollingVoice>,
    dwelling: HashMap<u64, bool>,
    next_bell: HashMap<u64, f64>,
    bell_flip: usize,
    last_whistle: f64,
    rng: Rng,
    seeded: bool,
}

impl Default for TrainAudio {
    fn default() -> Self {
        Self {
            voices: HashMap::new(),
            dwelling: HashMap::new(),
            next_bell: HashMap::new(),
            bell_flip: 0,
            last_whistle: f64::NEG_INFINITY,
            rng: Rng::new(0x7472_6169_6e73),
            seeded: false,
        }
    }
}

fn tile_world(tile: rail_sim::TileCoord) -> Vec2 {
    let (x, y) = tile_to_world(tile);
    Vec2::new(x, y)
}

/// Where a train is, and how fast, in presentation terms.
fn read_train(
    train: &Train,
    loc: &TrainLocation,
    network: &TrackNetwork,
    occupancy: &TileOccupancy,
    density: &TownDensity,
) -> Option<TrainState> {
    let piece = network.piece(loc.track)?;
    let here = tile_world(piece.tile);
    let held = occupancy.is_blocked(train.id);
    let stopped = loc.parked || loc.dwell_remaining > 0 || held;

    let needed = ticks_for_piece(train.kind, piece.max_grade, piece.curve).max(1);
    let (at, speed) = match loc.path.get(loc.path_index + 1).and_then(|id| network.piece(*id)) {
        Some(next) if !stopped => {
            let t = (loc.progress as f32 / needed as f32).clamp(0.0, 1.0);
            let there = tile_world(next.tile);
            (here + (there - here) * t, (FASTEST_TICKS / needed as f32).clamp(0.0, 1.0))
        }
        _ => (here, 0.0),
    };

    // A crossing is a track tile inside a built-up district.
    let mut near_crossing = false;
    if speed > 0.0 {
        for step in 1..=CROSSING_LOOKAHEAD {
            let Some(ahead) = loc
                .path
                .get(loc.path_index + step)
                .and_then(|id| network.piece(*id))
            else {
                break;
            };
            if density.get(ahead.tile) >= CROSSING_DENSITY {
                near_crossing = true;
                break;
            }
        }
    }

    Some(TrainState {
        id: train.id,
        kind: train.kind,
        at,
        speed,
        dwelling: loc.dwell_remaining > 0,
        near_crossing,
    })
}

/// Mass, `0.0`–`1.0`. Freight is heavy and it should sound it.
fn mass_of(kind: TrainKind) -> f32 {
    match kind {
        TrainKind::Transit => 0.22,
        TrainKind::Transport => 0.88,
    }
}

/// Keep one rolling voice per audible train, and fade out the rest.
#[allow(clippy::too_many_arguments)]
pub fn drive_rolling_voices(
    clock: Res<AudioClock>,
    mix: Res<AudioMix>,
    bank: Option<Res<SfxBank>>,
    network: Res<TrackNetwork>,
    occupancy: Res<TileOccupancy>,
    density: Res<TownDensity>,
    trains: Query<(&Train, &TrainLocation)>,
    mut audio: ResMut<TrainAudio>,
    mut budget: ResMut<VoiceBudget>,
    mut assets: ResMut<Assets<LiveVoice>>,
    mut transforms: Query<&mut Transform>,
    mut sinks: Query<&mut AudioSink>,
    mut spatial: Query<&mut SpatialAudioSink>,
    mut commands: Commands,
) {
    let dt = clock.real_delta.clamp(0.0, 0.25);

    let mut states: Vec<TrainState> = trains
        .iter()
        .filter_map(|(train, loc)| read_train(train, loc, &network, &occupancy, &density))
        .collect();
    states.sort_by(|a, b| {
        let da = (a.at - mix.listener).length_squared();
        let db = (b.at - mix.listener).length_squared();
        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
    });

    for voice in audio.voices.values_mut() {
        voice.keep = false;
    }

    let bus = mix.effects() * mix.effect_perspective();
    for state in states.iter().take(MAX_ROLLING) {
        let falloff = mix.falloff(state.at);
        if falloff <= 0.0 && !audio.voices.contains_key(&state.id.0) {
            continue;
        }

        if !audio.voices.contains_key(&state.id.0) {
            let handle = spawn_positional(
                &mut commands,
                &mut assets,
                VoiceKind::Rolling,
                state.id.0.wrapping_mul(2_654_435_761),
                state.at,
            );
            audio.voices.insert(
                state.id.0,
                RollingVoice {
                    handle,
                    closing: 0.0,
                    last_dist: (state.at - mix.listener).length(),
                    keep: true,
                },
            );
        }
        let Some(voice) = audio.voices.get_mut(&state.id.0) else {
            continue;
        };
        voice.keep = true;

        voice.handle.params.set_motion(state.speed);
        voice.handle.params.set_depth(mass_of(state.kind));
        voice.handle.params.set_tone(mix.brightness(state.at));

        if let Ok(mut transform) = transforms.get_mut(voice.handle.entity) {
            transform.translation.x = state.at.x;
            transform.translation.y = state.at.y;
        }

        // Doppler: a smoothed radial velocity, trimmed to a few per cent.
        let dist = (state.at - mix.listener).length();
        let raw = if dt > 0.0 { (dist - voice.last_dist) / dt } else { 0.0 };
        voice.last_dist = dist;
        voice.closing = approach(voice.closing, raw.clamp(-1500.0, 1500.0), dt, 0.18);
        let rate = (1.0 - voice.closing / DOPPLER_C).clamp(0.94, 1.06);
        voice.handle.set_pitch(rate, &sinks, &spatial);

        let target = gain::ROLLING * falloff * bus * (0.25 + 0.75 * state.speed);
        voice
            .handle
            .apply(target, dt, ROLL_FADE_SECS, &mut sinks, &mut spatial);
    }

    // Fade out and retire anything that lost its slot or left the map.
    let mut retired: Vec<u64> = Vec::new();
    for (id, voice) in audio.voices.iter_mut() {
        if voice.keep {
            continue;
        }
        voice
            .handle
            .apply(0.0, dt, ROLL_FADE_SECS, &mut sinks, &mut spatial);
        if voice.handle.gain <= 0.001 {
            commands.entity(voice.handle.entity).despawn();
            retired.push(*id);
        }
    }
    for id in retired {
        audio.voices.remove(&id);
    }

    // -- punctuation ------------------------------------------------------
    let Some(bank) = bank else {
        return;
    };
    let now = clock.elapsed;
    let live: Vec<u64> = states.iter().map(|s| s.id.0).collect();
    let seeded = audio.seeded;
    for state in states.iter() {
        let was = audio.dwelling.insert(state.id.0, state.dwelling);
        if !seeded {
            continue;
        }
        let Some(was) = was else { continue };
        if was == state.dwelling {
            continue;
        }
        if state.dwelling {
            arrival(&mut commands, &mut audio, &mut budget, &bank, &mix, state);
        } else {
            departure(&mut commands, &mut audio, &mut budget, &bank, &mix, state, now);
        }
    }
    audio.dwelling.retain(|id, _| live.contains(id));
    audio.next_bell.retain(|id, _| live.contains(id));
    audio.seeded = true;

    // Crossing bells.
    for state in states.iter() {
        if !state.near_crossing || state.speed <= 0.0 {
            audio.next_bell.remove(&state.id.0);
            continue;
        }
        let due = audio.next_bell.get(&state.id.0).copied().unwrap_or(f64::MIN);
        if now < due {
            continue;
        }
        audio
            .next_bell
            .insert(state.id.0, now + CROSSING_INTERVAL as f64);
        audio.bell_flip ^= 1;
        let flip = audio.bell_flip;
        play(
            &mut commands,
            &mut budget,
            &mix,
            Cue::world(
                bank.crossing.pick(flip, mix.brightness(state.at)),
                SoundCategory::World,
                gain::CROSSING,
                state.at,
            ),
        );
    }
}

/// Brakes on arrival, and a settle.
fn arrival(
    commands: &mut Commands,
    audio: &mut TrainAudio,
    budget: &mut VoiceBudget,
    bank: &SfxBank,
    mix: &AudioMix,
    state: &TrainState,
) {
    let index = audio.rng.below(bank.brake.variants());
    let speed = audio.rng.range(0.95, 1.05) * lerp(1.06, 0.9, mass_of(state.kind));
    play(
        commands,
        budget,
        mix,
        Cue::world(
            bank.brake.pick(index, mix.brightness(state.at)),
            SoundCategory::Train,
            gain::BRAKE,
            state.at,
        )
        .with_speed(speed),
    );
}

/// A chuff or a whine, and — rarely — a whistle.
#[allow(clippy::too_many_arguments)]
fn departure(
    commands: &mut Commands,
    audio: &mut TrainAudio,
    budget: &mut VoiceBudget,
    bank: &SfxBank,
    mix: &AudioMix,
    state: &TrainState,
    now: f64,
) {
    let set = match state.kind {
        TrainKind::Transit => &bank.depart_transit,
        TrainKind::Transport => &bank.depart_freight,
    };
    let index = audio.rng.below(set.variants());
    let speed = audio.rng.range(0.96, 1.04);
    play(
        commands,
        budget,
        mix,
        Cue::world(
            set.pick(index, mix.brightness(state.at)),
            SoundCategory::Train,
            gain::DEPARTURE,
            state.at,
        )
        .with_speed(speed),
    );

    // Sparing, by construction: a long cooldown *and* a coin toss.
    if now - audio.last_whistle < WHISTLE_COOLDOWN as f64 {
        return;
    }
    if !audio.rng.chance(WHISTLE_CHANCE) {
        return;
    }
    audio.last_whistle = now;
    let index = audio.rng.below(bank.whistle.variants());
    let speed = audio.rng.range(0.97, 1.03);
    play(
        commands,
        budget,
        mix,
        Cue::world(
            bank.whistle.pick(index, mix.brightness(state.at)),
            SoundCategory::Train,
            gain::WHISTLE,
            state.at,
        )
        .with_speed(speed),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freight_is_heavier_than_transit() {
        assert!(mass_of(TrainKind::Transport) > mass_of(TrainKind::Transit) * 3.0);
    }

    #[test]
    fn the_whistle_is_rare_by_construction() {
        // Brief §3.2: ruined by overuse. Roughly one every minute and a half of
        // continuous departures, at most.
        let expected_gap = WHISTLE_COOLDOWN / WHISTLE_CHANCE;
        assert!(expected_gap > 60.0, "mean gap of {expected_gap}s is too short");
    }

    #[test]
    fn the_doppler_trim_is_a_shift_not_a_swoop() {
        // A train doing a tile a second closing head-on.
        let fast = 32.0 * 4.0;
        let rate = (1.0 - -fast / DOPPLER_C).clamp(0.94, 1.06);
        assert!(rate > 1.0 && rate < 1.06, "rate {rate}");
    }

    #[test]
    fn crossing_bells_do_not_become_a_rattle() {
        assert!(CROSSING_INTERVAL >= 0.8, "a bell, not an alarm");
        assert!(gain::CROSSING < gain::BRAKE, "and it stays under the train");
    }
}
