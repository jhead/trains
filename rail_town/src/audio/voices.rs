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

/// A spawned live voice and its controls.
#[derive(Debug, Clone)]
pub struct VoiceHandle {
    pub entity: Entity,
    pub params: Arc<VoiceParams>,
    /// Current (smoothed) linear gain.
    pub gain: f32,
    pub kind: VoiceKind,
}

impl VoiceHandle {
    /// Slew toward `target` and write the result to the sink.
    ///
    /// `tau` is the fade time constant: beds get seconds, trains get a fraction
    /// of one. Nothing gets zero.
    pub fn apply(
        &mut self,
        target: f32,
        dt: f32,
        tau: f32,
        sinks: &mut Query<&mut AudioSink>,
        spatial: &mut Query<&mut SpatialAudioSink>,
    ) {
        self.gain = approach(self.gain, target.max(0.0), dt, tau);
        let level = if self.gain < SILENCE { 0.0 } else { self.gain };
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
            Name::new(format!("audio:{}", kind.label())),
        ))
        .id();
    VoiceHandle {
        entity,
        params,
        gain: 0.0,
        kind,
    }
}
