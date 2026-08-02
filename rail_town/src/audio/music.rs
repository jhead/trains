//! The score (brief §4) — sparse, long, and mostly absent.
//!
//! There is one music voice, spawned at startup and never stopped. A "cue" is
//! an envelope on its level: up over fourteen seconds, held for three to five
//! minutes, down over twenty, then three to eight minutes of nothing. The
//! ambience carries the gaps, which is the point — silence is a texture, not a
//! bug, and a score you notice arriving is a score you will resent by hour two.
//!
//! Nothing about a cue is dramatic. The harmony is chosen while the voice is
//! silent, it never resolves, it has no percussion and it cannot build. The one
//! contextual move is the palette: warm and with a third when the network is
//! thriving, open fifths when the town is thin, and a distinct register at dusk
//! — the prettiest minute of the day cycle deserves its own piece.

use bevy::audio::{AudioSink, SpatialAudioSink};
use bevy::prelude::*;

use crate::atmosphere::{DayPhase, TimeOfDay};

use super::ambience::AmbienceBeds;
use super::dsp::{smoothstep, Rng};
use super::mixer::{gain, AudioClock, AudioMix};
use super::voice::{LiveVoice, VoiceKind};
use super::voices::{spawn_bed, VoiceHandle};

/// How long after launch the first cue may enter. "A minute or two in, never at
/// launch" — a game that starts with music announces itself.
const FIRST_CUE_MIN: f32 = 95.0;
const FIRST_CUE_MAX: f32 = 140.0;

/// Cue length, in seconds (three to five minutes).
const CUE_MIN: f32 = 180.0;
const CUE_MAX: f32 = 300.0;

/// Silence between cues (three to eight minutes).
const GAP_MIN: f32 = 180.0;
const GAP_MAX: f32 = 480.0;

/// Envelope times. Long enough that the entrance is not an event.
const FADE_IN: f32 = 14.0;
const FADE_OUT: f32 = 20.0;

/// What the director is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Silent,
    Playing,
}

#[derive(Resource, Debug)]
pub struct MusicDirector {
    voice: Option<VoiceHandle>,
    stage: Stage,
    /// When the current stage ends, on the shared audio clock.
    until: f64,
    /// When the current cue began.
    began: f64,
    rng: Rng,
    started: bool,
}

impl Default for MusicDirector {
    fn default() -> Self {
        Self {
            voice: None,
            stage: Stage::Silent,
            until: 0.0,
            began: 0.0,
            rng: Rng::new(0x6d75_7369_63),
            started: false,
        }
    }
}

impl MusicDirector {
    /// Current music level. Read by the plugin tests, and by anything that ever
    /// wants to show the score state in a settings panel.
    #[allow(dead_code)]
    pub fn gain(&self) -> f32 {
        self.voice.as_ref().map(|v| v.gain).unwrap_or(0.0)
    }
}

pub fn spawn_music(
    mut commands: Commands,
    mut assets: ResMut<Assets<LiveVoice>>,
    mut director: ResMut<MusicDirector>,
) {
    let handle = spawn_bed(&mut commands, &mut assets, VoiceKind::Music, 0x4f_1a_9c_33);
    director.voice = Some(handle);
}

/// Envelope for a point inside a cue of length `len`.
fn cue_envelope(elapsed: f32, len: f32) -> f32 {
    let up = smoothstep(0.0, FADE_IN, elapsed);
    let down = smoothstep(0.0, FADE_OUT, len - elapsed);
    (up * down).clamp(0.0, 1.0)
}

pub fn drive_music(
    clock: Res<AudioClock>,
    mix: Res<AudioMix>,
    tod: Res<TimeOfDay>,
    beds: Res<AmbienceBeds>,
    mut director: ResMut<MusicDirector>,
    mut sinks: Query<&mut AudioSink>,
    mut spatial: Query<&mut SpatialAudioSink>,
) {
    let now = clock.elapsed;
    if !director.started {
        // Never at launch.
        let delay = director.rng.range(FIRST_CUE_MIN, FIRST_CUE_MAX);
        director.until = now + delay as f64;
        director.started = true;
    }

    let dt = clock.real_delta.clamp(0.0, 0.25);
    let mut target = 0.0;

    match director.stage {
        Stage::Silent => {
            if now >= director.until {
                let len = director.rng.range(CUE_MIN, CUE_MAX);
                director.stage = Stage::Playing;
                director.began = now;
                director.until = now + len as f64;

                // The harmony is chosen here, while the voice is silent, so the
                // root never sweeps between two keys mid-cue.
                if let Some(voice) = director.voice.as_ref() {
                    let thriving = smoothstep(0.05, 0.45, beds.town_density);
                    let dusk = tod.phase == DayPhase::Dusk;
                    voice.params.set_tone(if dusk {
                        0.85
                    } else {
                        0.25 + 0.65 * thriving
                    });
                    voice.params.set_depth(if dusk { 0.75 } else { 0.35 });
                    voice.params.set_density(0.3 + 0.4 * thriving);
                }
                let root = director.rng.unit();
                if let Some(voice) = director.voice.as_ref() {
                    voice.params.set_color(root);
                }
            }
        }
        Stage::Playing => {
            let len = (director.until - director.began) as f32;
            let elapsed = (now - director.began) as f32;
            target = gain::MUSIC * cue_envelope(elapsed, len) * mix.music();
            if now >= director.until {
                director.stage = Stage::Silent;
                let gap = director.rng.range(GAP_MIN, GAP_MAX);
                director.until = now + gap as f64;
            }
        }
    }

    if let Some(voice) = director.voice.as_mut() {
        // A one-second slew on top of the envelope: the duck from laying track
        // arrives as a lean, never as a gate.
        voice.apply(target, dt, 1.0, &mut sinks, &mut spatial);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_cue_is_never_at_launch() {
        assert!(FIRST_CUE_MIN > 60.0, "a minute or two in (§4)");
        assert!(FIRST_CUE_MAX < 240.0, "but not never");
    }

    #[test]
    fn cues_are_long_and_the_gaps_are_longer() {
        assert!((CUE_MIN - 180.0).abs() < 1.0 && (CUE_MAX - 300.0).abs() < 1.0);
        assert!(GAP_MIN >= CUE_MIN, "silence is a texture, not a pause");
        // Over a long session, music is present less than half the time.
        let duty = (CUE_MIN + CUE_MAX) / (CUE_MIN + CUE_MAX + GAP_MIN + GAP_MAX);
        assert!(duty < 0.5, "music plays {:.0}% of the time", duty * 100.0);
    }

    #[test]
    fn the_envelope_never_steps() {
        let len = 200.0;
        let mut prev = cue_envelope(0.0, len);
        assert_eq!(prev, 0.0, "a cue starts from silence");
        let steps = 20_000;
        let mut worst: f32 = 0.0;
        for i in 1..=steps {
            let t = len * i as f32 / steps as f32;
            let value = cue_envelope(t, len);
            worst = worst.max((value - prev).abs());
            prev = value;
        }
        assert_eq!(prev, 0.0, "and it ends in silence");
        // At 60 fps a frame is 1/12000 of a 200 s cue; this bound is far tighter.
        assert!(worst < 0.002, "envelope stepped by {worst}");
    }

    #[test]
    fn the_envelope_reaches_full_in_the_middle() {
        assert!(cue_envelope(100.0, 200.0) > 0.99);
        assert!(cue_envelope(FADE_IN * 0.5, 200.0) < 0.75);
        assert!(cue_envelope(200.0 - FADE_OUT * 0.5, 200.0) < 0.75);
    }

    #[test]
    fn a_cue_is_long_enough_to_fade_in_and_out_twice_over() {
        assert!(CUE_MIN > (FADE_IN + FADE_OUT) * 3.0);
    }
}
