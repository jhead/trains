//! Spawning and driving the long-lived [`LiveVoice`] entities.
//!
//! A live voice is spawned once and never restarted. Its level is written to the
//! sink every frame through [`VoiceHandle::apply`], which slews toward the
//! target — so a voice can be brought in, taken out, or cross-faded against its
//! neighbours without anything ever stepping.

use std::sync::Arc;

use bevy::audio::{AudioSink, AudioSinkPlayback, PlaybackSettings, SpatialAudioSink, SpatialScale, Volume};
use bevy::prelude::*;

use super::dsp::approach;
use super::voice::{LiveVoice, VoiceKind, VoiceParams};

/// Spatial scale for positional voices; see [`super::mixer`].
const SPATIAL_SCALE: f32 = 1.0 / 8192.0;

/// Below this the sink is left at zero rather than at an inaudible trickle.
const SILENCE: f32 = 0.0008;

/// How fast a bus level reaches the sink.
///
/// **This is separate from the voice's own fade on purpose.** A bed cross-fades
/// over seconds because arriving somewhere should feel like arriving; a volume
/// slider must answer immediately or the player concludes it does nothing —
/// which is exactly what the playtest reported. A quarter of a second is fast
/// enough to feel connected to the key press and slow enough not to step.
const BUS_SLEW_SECS: f32 = 0.08;

/// Marks a positional voice's entity so its transform can be updated without a
/// world-wide `Query<&mut Transform>` fighting the camera and sprite systems
/// for scheduling.
#[derive(Component, Debug)]
pub struct VoiceEmitter;

/// A spawned live voice and its controls.
#[derive(Debug, Clone)]
pub struct VoiceHandle {
    pub entity: Entity,
    pub params: Arc<VoiceParams>,
    /// Current (smoothed) weight — how much of this voice the scene wants,
    /// before the buses have had their say.
    pub gain: f32,
    /// Current (smoothed) bus level. Tracked separately so that a slider moves
    /// promptly while a cross-fade still takes its seconds.
    pub bus: f32,
    pub kind: VoiceKind,
}

impl VoiceHandle {
    /// Slew toward `target`, scale by `bus`, and write the result to the sink.
    ///
    /// `tau` is the fade time constant for the *weight*: beds get seconds,
    /// trains get a fraction of one. Nothing gets zero. `bus` — master ×
    /// category × focus × duck — is smoothed separately and much faster; see
    /// [`BUS_SLEW_SECS`].
    pub fn apply(
        &mut self,
        target: f32,
        bus: f32,
        dt: f32,
        tau: f32,
        sinks: &mut Query<&mut AudioSink>,
        spatial: &mut Query<&mut SpatialAudioSink>,
    ) {
        self.gain = approach(self.gain, target.max(0.0), dt, tau);
        self.bus = approach(self.bus, bus.clamp(0.0, 4.0), dt, BUS_SLEW_SECS);
        let level = self.gain * self.bus;
        let level = if level < SILENCE { 0.0 } else { level };
        if let Ok(mut sink) = sinks.get_mut(self.entity) {
            sink.set_volume(Volume::Linear(level));
        } else if let Ok(mut sink) = spatial.get_mut(self.entity) {
            sink.set_volume(Volume::Linear(level));
        }
    }

    /// Doppler / speed trim for positional voices. Kept tiny on purpose: a
    /// passing train should shift, not swoop.
    pub fn set_pitch(&self, rate: f32, sinks: &Query<&mut AudioSink>, spatial: &Query<&mut SpatialAudioSink>) {
        let rate = rate.clamp(0.9, 1.1);
        if let Ok(sink) = sinks.get(self.entity) {
            sink.set_speed(rate);
        } else if let Ok(sink) = spatial.get(self.entity) {
            sink.set_speed(rate);
        }
    }
}

/// Spawn a non-positional voice (ambience bed, music).
pub fn spawn_bed(
    commands: &mut Commands,
    assets: &mut Assets<LiveVoice>,
    kind: VoiceKind,
    seed: u64,
) -> VoiceHandle {
    let voice = LiveVoice::new(kind, seed);
    let params = voice.params.clone();
    let handle = assets.add(voice);
    let entity = commands
        .spawn((
            AudioPlayer(handle),
            // `Once` on an endless source: it plays until the entity is gone.
            PlaybackSettings::ONCE.with_volume(Volume::Linear(0.0)),
            Name::new(format!("audio:{}", kind.label())),
        ))
        .id();
    VoiceHandle {
        entity,
        params,
        gain: 0.0,
        bus: 0.0,
        kind,
    }
}

/// Spawn a positional voice (a train).
pub fn spawn_positional(
    commands: &mut Commands,
    assets: &mut Assets<LiveVoice>,
    kind: VoiceKind,
    seed: u64,
    at: Vec2,
) -> VoiceHandle {
    let voice = LiveVoice::new(kind, seed);
    let params = voice.params.clone();
    let handle = assets.add(voice);
    let entity = commands
        .spawn((
            AudioPlayer(handle),
            PlaybackSettings::ONCE
                .with_volume(Volume::Linear(0.0))
                .with_spatial(true)
                .with_spatial_scale(SpatialScale::new_2d(SPATIAL_SCALE)),
            Transform::from_xyz(at.x, at.y, 0.0),
            VoiceEmitter,
            Name::new(format!("audio:{}", kind.label())),
        ))
        .id();
    VoiceHandle {
        entity,
        params,
        gain: 0.0,
        bus: 0.0,
        kind,
    }
}
