//! When the score plays (brief §4). *What* it plays is [`super::score`].
//!
//! There is one music voice, spawned at startup and never stopped. A "cue" is an
//! envelope on its level: up over fourteen seconds, held for three to five
//! minutes, down over twenty, then three to eight minutes of nothing. The
//! ambience carries the gaps, which is the point — silence is a texture, not a
//! bug, and a score you notice arriving is a score you will resent by hour two.
//!
//! # Every cue opens at bar one, and no two cues in a row are the same piece
//!
//! A world is composed as four short pieces, each with its own key, motif and
//! progression. If the generator ran free underneath the envelope, each cue
//! would fade in wherever it happened to be. So the director numbers its cues
//! and writes the number to
//! [`VoiceParams::set_cue`](super::voice::VoiceParams::set_cue); the audio
//! thread reads a change as "start the next piece from bar one" and a zero as
//! "stop, and cost nothing until asked again". The cue number is also what
//! selects the piece -- [`Score::piece_for`](super::score::Score::piece_for)
//! rotates through the four -- so the music always begins somewhere a listener
//! can follow, and never begins the same way twice running. The playtest note
//! that started this was "it's really repetitive"; the counter is the fix.
//!
//! # The seed
//!
//! The map's own seed, so a world has its own four tunes and always the same
//! four. Change the map, and the next silence is used to rebuild the voice
//! around the new seed. Nothing is ever re-seeded while a cue is audible.
//!
//! # Context
//!
//! Three continuous controls, all chosen while the voice is silent so that
//! nothing ever sweeps mid-cue: warmth from the town density around the camera,
//! density (how much of the composition survives) from the same, and a dusk flag
//! from [`TimeOfDay`]. None of them is dramatic and none is a comment on
//! failure — a declining town gets a sparser, darker reading of the same piece,
//! not a sad one.

use bevy::audio::{AudioSink, SpatialAudioSink};
use bevy::prelude::*;
use rail_map::MapGrid;

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

/// Below this the fade is over and the generator may be put back to sleep.
const SLEEP_BELOW: f32 = 0.0004;

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
    /// Increments once per cue; the audio thread restarts the piece on a change.
    cue: u32,
    /// Map seed the current voice was composed from.
    seed: u64,
}

impl Default for MusicDirector {
    fn default() -> Self {
        Self {
            voice: None,
            stage: Stage::Silent,
            until: 0.0,
            began: 0.0,
            rng: Rng::new(0x006d_7573_6963),
            started: false,
            cue: 0,
            seed: u64::MAX,
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

/// The seed a world's tune is composed from.
///
/// Mixed rather than used raw so that two maps whose seeds differ by one do not
/// get two pieces that differ by one decision.
fn tune_seed(map_seed: u64) -> u64 {
    map_seed
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .rotate_left(29)
        ^ 0x0072_6169_6c74_6f77
}

pub fn spawn_music(
    mut commands: Commands,
    map: Option<Res<MapGrid>>,
    mut assets: ResMut<Assets<LiveVoice>>,
    mut director: ResMut<MusicDirector>,
) {
    let seed = map.map(|m| m.seed).unwrap_or(0);
    director.seed = seed;
    let handle = spawn_bed(
        &mut commands,
        &mut assets,
        VoiceKind::Music,
        tune_seed(seed),
    );
    director.voice = Some(handle);
}

/// Envelope for a point inside a cue of length `len`.
fn cue_envelope(elapsed: f32, len: f32) -> f32 {
    let up = smoothstep(0.0, FADE_IN, elapsed);
    let down = smoothstep(0.0, FADE_OUT, len - elapsed);
    (up * down).clamp(0.0, 1.0)
}

#[allow(clippy::too_many_arguments)]
pub fn drive_music(
    mut commands: Commands,
    clock: Res<AudioClock>,
    mix: Res<AudioMix>,
    tod: Res<TimeOfDay>,
    map: Option<Res<MapGrid>>,
    beds: Res<AmbienceBeds>,
    mut assets: ResMut<Assets<LiveVoice>>,
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
            // A new world is a new tune. Rebuilding the voice means rebuilding
            // its decoder, so it only ever happens between cues, with the sink
            // already at zero.
            if let Some(seed) = map.as_ref().map(|m| m.seed) {
                if seed != director.seed && director.gain() <= SLEEP_BELOW {
                    if let Some(old) = director.voice.take() {
                        commands.entity(old.entity).despawn();
                    }
                    director.seed = seed;
                    director.voice = Some(spawn_bed(
                        &mut commands,
                        &mut assets,
                        VoiceKind::Music,
                        tune_seed(seed),
                    ));
                }
            }

            if now >= director.until {
                let len = director.rng.range(CUE_MIN, CUE_MAX);
                director.stage = Stage::Playing;
                director.began = now;
                director.until = now + len as f64;
                director.cue = director.cue.wrapping_add(1).max(1);

                // The reading is chosen here, while the voice is silent, so no
                // control ever sweeps under a sounding note.
                let thriving = smoothstep(0.05, 0.45, beds.town_density);
                let dusk = f32::from(tod.phase == DayPhase::Dusk);
                let cue = director.cue;
                if let Some(voice) = director.voice.as_ref() {
                    // Warmth: a more open bell and a brighter pad when the
                    // network is doing well.
                    voice.params.set_tone(0.30 + 0.60 * thriving);
                    // Density: how much of the composition survives. A thin town
                    // gets the pad, the downbeat and the tune; a thriving one
                    // gets the walking bass and the plucked arpeggio too. The
                    // range reaches low enough that the thinning is actually
                    // audible rather than theoretical.
                    voice.params.set_density(0.38 + 0.57 * thriving);
                    // The dusk reading: the same music, softer and darker. Not
                    // lower - the octave drop this used to do is most of what
                    // made the old score read as gloom.
                    voice.params.set_color(dusk);
                    voice.params.set_depth(0.5);
                    voice.params.set_cue(cue);
                }
            }
        }
        Stage::Playing => {
            let len = (director.until - director.began) as f32;
            let elapsed = (now - director.began) as f32;
            target = gain::MUSIC * cue_envelope(elapsed, len);
            if now >= director.until {
                director.stage = Stage::Silent;
                let gap = director.rng.range(GAP_MIN, GAP_MAX);
                director.until = now + gap as f64;
            }
        }
    }

    // Once the fade-out has actually finished, tell the generator to stop. It
    // then costs one comparison per sample for the next three to eight minutes,
    // and the next cue restarts the piece from bar one.
    if director.stage == Stage::Silent && director.gain() <= SLEEP_BELOW {
        if let Some(voice) = director.voice.as_ref() {
            voice.params.set_cue(0);
        }
    }

    let bus = mix.music();
    if let Some(voice) = director.voice.as_mut() {
        // A one-second slew on top of the envelope: the duck from laying track
        // arrives as a lean, never as a gate.
        voice.apply(target, bus, dt, 1.0, &mut sinks, &mut spatial);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_cue_is_never_at_launch() {
        assert!(FIRST_CUE_MIN > 60.0, "a minute or two in (S4)");
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
    fn a_cue_is_never_much_longer_than_the_piece() {
        // If a cue could run to several times the length of the composition the
        // loop point would be heard as a loop. One pass, sometimes a little
        // more, is the target.
        let piece = super::super::score::Score::new(1).secs();
        assert!(
            CUE_MAX < piece * 1.35,
            "a {CUE_MAX} s cue would go round a {piece:.0} s piece too often"
        );
        assert!(
            CUE_MIN > piece * 0.55,
            "a {CUE_MIN} s cue would not reach the second theme of a {piece:.0} s piece"
        );
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

    #[test]
    fn a_world_gets_its_own_tune_and_keeps_it() {
        assert_eq!(tune_seed(42), tune_seed(42), "the same map, the same music");
        assert_ne!(tune_seed(42), tune_seed(43));
        // Adjacent map seeds must not produce adjacent generator seeds, or two
        // neighbouring worlds would share most of their decisions.
        let a = tune_seed(42);
        let b = tune_seed(43);
        assert!(
            (a ^ b).count_ones() > 8,
            "seeds 42 and 43 differ in only {} bits",
            (a ^ b).count_ones()
        );
    }
}
