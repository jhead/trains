//! [`SampleClip`] — a finite PCM one-shot, plus the offline renderer that bakes
//! the bank at startup.
//!
//! There are no audio assets and no artist, so every sound in the game is
//! synthesised into one of these at boot. A clip is a plain `Arc<[f32]>` of mono
//! samples at [`SAMPLE_RATE`]; playing one costs an `Arc` clone.
//!
//! Two invariants are enforced by [`Canvas::finish`] rather than by discipline:
//!
//! 1. **Nothing starts or ends on a discontinuity.** Every clip gets a
//!    raised-cosine head and tail, so no clip can click.
//! 2. **Every clip peaks at the same level.** Loudness is therefore decided in
//!    one place — the gain constants in [`super::mixer`] — instead of drifting
//!    with whatever the synthesis happened to produce. That is what keeps the
//!    dynamic range narrow (brief §7).

use core::time::Duration;
use std::sync::Arc;

use bevy::asset::Asset;
use bevy::audio::{Decodable, Source as RodioSource};
use bevy::reflect::TypePath;

use super::dsp::{exp_decay, lerp, raised_cosine, sine, svf_damp, svf_f, Rng, Svf};

/// Working sample rate for every synthesised sound.
///
/// 22.05 kHz is deliberate rather than lazy: the whole palette is soft and low,
/// nothing in it lives above about 8 kHz, and halving the rate halves both the
/// bake cost and the per-voice CPU on the audio thread.
pub const SAMPLE_RATE: u32 = 22_050;

/// [`SAMPLE_RATE`] as a float, for the synthesis maths.
pub const SR: f32 = SAMPLE_RATE as f32;

/// Peak every finished clip is normalised to.
///
/// Headroom below 1.0 so that two one-shots landing on the same sample cannot
/// clip the output before the mixer's own gains have had their say.
const NORMALISE_PEAK: f32 = 0.85;

/// Minimum head fade. The brief bans sharp transients at volume; 4 ms of
/// raised cosine is inaudible as a softening and decisive as a de-clicker.
const HEAD_FADE_SECS: f32 = 0.004;
/// Minimum tail fade.
const TAIL_FADE_SECS: f32 = 0.010;

/// A finite mono PCM one-shot.
#[derive(Asset, TypePath, Clone, Debug)]
pub struct SampleClip {
    pub data: Arc<[f32]>,
}

impl SampleClip {
    /// Length in seconds. Read by the bank's shape assertions.
    #[allow(dead_code)]
    pub fn secs(&self) -> f32 {
        self.data.len() as f32 / SR
    }
}

/// Cursor over a [`SampleClip`]'s samples.
pub struct ClipReader {
    data: Arc<[f32]>,
    pos: usize,
}

impl Iterator for ClipReader {
    type Item = f32;

    #[inline]
    fn next(&mut self) -> Option<f32> {
        let sample = self.data.get(self.pos).copied();
        if sample.is_some() {
            self.pos += 1;
        }
        sample
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.data.len().saturating_sub(self.pos);
        (left, Some(left))
    }
}

impl RodioSource for ClipReader {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        1
    }

    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f32(
            self.data.len() as f32 / SR,
        ))
    }
}

impl Decodable for SampleClip {
    type DecoderItem = f32;
    type Decoder = ClipReader;

    fn decoder(&self) -> ClipReader {
        ClipReader {
            data: self.data.clone(),
            pos: 0,
        }
    }
}

/// A scratch buffer that synthesis layers are summed into.
pub struct Canvas {
    buf: Vec<f32>,
}

impl Canvas {
    pub fn new(secs: f32) -> Self {
        let len = ((secs.max(0.005)) * SR).ceil() as usize;
        Self {
            buf: vec![0.0; len],
        }
    }

    #[inline]
    fn mix(&mut self, index: usize, value: f32) {
        if let Some(slot) = self.buf.get_mut(index) {
            *slot += value;
        }
    }

    /// Sample index for a time in seconds.
    #[inline]
    fn at(&self, secs: f32) -> usize {
        (secs.max(0.0) * SR) as usize
    }

    /// A decaying partial: soft attack, exponential tail, optional pitch drift.
    ///
    /// `drift` is a multiplicative ratio applied linearly across the life of the
    /// partial (`1.0` = steady). Real struck objects fall slightly in pitch as
    /// they settle; a touch of drift is most of the difference between "wood"
    /// and "sine wave".
    #[allow(clippy::too_many_arguments)]
    pub fn partial(
        &mut self,
        start: f32,
        freq: f32,
        amp: f32,
        attack: f32,
        tau: f32,
        dur: f32,
        drift: f32,
    ) {
        let start_i = self.at(start);
        let n = (dur * SR) as usize;
        let attack_n = (attack * SR).max(1.0) as usize;
        let mut phase = 0.0f32;
        for i in 0..n {
            let t = i as f32 / SR;
            let env = if i < attack_n {
                raised_cosine(i as f32 / attack_n as f32)
            } else {
                1.0
            } * exp_decay(t, tau);
            if env < 1e-4 && i > attack_n {
                break;
            }
            let f = freq * lerp(1.0, drift, t / dur.max(1e-4));
            phase += f / SR;
            if phase >= 1.0 {
                phase -= phase.floor();
            }
            self.mix(start_i + i, sine(phase) * env * amp);
        }
    }

    /// A vibrato'd partial — the whistle and the warmer chimes want a little
    /// life in the sustain so they do not read as a test tone.
    #[allow(clippy::too_many_arguments)]
    pub fn partial_vibrato(
        &mut self,
        start: f32,
        freq: f32,
        amp: f32,
        attack: f32,
        tau: f32,
        dur: f32,
        drift: f32,
        vib_hz: f32,
        vib_cents: f32,
    ) {
        let start_i = self.at(start);
        let n = (dur * SR) as usize;
        let attack_n = (attack * SR).max(1.0) as usize;
        let mut phase = 0.0f32;
        let mut vib = 0.0f32;
        for i in 0..n {
            let t = i as f32 / SR;
            let env = if i < attack_n {
                raised_cosine(i as f32 / attack_n as f32)
            } else {
                1.0
            } * exp_decay(t, tau);
            if env < 1e-4 && i > attack_n {
                break;
            }
            vib += vib_hz / SR;
            if vib >= 1.0 {
                vib -= 1.0;
            }
            let cents = sine(vib) * vib_cents;
            let f = freq * lerp(1.0, drift, t / dur.max(1e-4)) * (cents / 1200.0).exp2();
            phase += f / SR;
            if phase >= 1.0 {
                phase -= phase.floor();
            }
            self.mix(start_i + i, sine(phase) * env * amp);
        }
    }

    /// A band of noise with a resonant centre — ballast, debris, surf, chatter.
    ///
    /// `center_end` sweeps the band over the life of the burst; brakes and the
    /// airy panel sweeps are the same call with different endpoints.
    #[allow(clippy::too_many_arguments)]
    pub fn noise_band(
        &mut self,
        rng: &mut Rng,
        start: f32,
        dur: f32,
        center: f32,
        center_end: f32,
        q: f32,
        amp: f32,
        attack: f32,
        tau: f32,
    ) {
        let start_i = self.at(start);
        let n = (dur * SR) as usize;
        let attack_n = (attack * SR).max(1.0) as usize;
        let damp = svf_damp(q);
        let mut svf = Svf::default();
        for i in 0..n {
            let t = i as f32 / SR;
            let env = if i < attack_n {
                raised_cosine(i as f32 / attack_n as f32)
            } else {
                1.0
            } * exp_decay(t, tau);
            let progress = i as f32 / n.max(1) as f32;
            let f = svf_f(lerp(center, center_end, progress), SR);
            let out = svf.step(rng.bipolar(), f, damp);
            if env < 1e-4 && i > attack_n {
                break;
            }
            self.mix(start_i + i, out.band * env * amp);
        }
    }

    /// Low-passed noise — settling debris, breath, air.
    #[allow(clippy::too_many_arguments)]
    pub fn noise_soft(
        &mut self,
        rng: &mut Rng,
        start: f32,
        dur: f32,
        cutoff: f32,
        amp: f32,
        attack: f32,
        tau: f32,
    ) {
        let start_i = self.at(start);
        let n = (dur * SR) as usize;
        let attack_n = (attack * SR).max(1.0) as usize;
        let mut svf = Svf::default();
        let f = svf_f(cutoff, SR);
        for i in 0..n {
            let t = i as f32 / SR;
            let env = if i < attack_n {
                raised_cosine(i as f32 / attack_n as f32)
            } else {
                1.0
            } * exp_decay(t, tau);
            if env < 1e-4 && i > attack_n {
                break;
            }
            let out = svf.step(rng.bipolar(), f, svf_damp(0.7));
            self.mix(start_i + i, out.low * env * amp);
        }
    }

    /// Darken the whole buffer — the "heard from across the valley" pass.
    ///
    /// Distance is not just quieter, it is duller (brief §7). One clip family is
    /// baked twice, near and far, and the mixer picks by distance.
    pub fn low_pass(&mut self, cutoff: f32) {
        let mut svf = Svf::default();
        let f = svf_f(cutoff, SR);
        for sample in self.buf.iter_mut() {
            *sample = svf.step(*sample, f, svf_damp(0.7)).low;
        }
    }

    /// De-click both ends, normalise, and freeze into a [`SampleClip`].
    ///
    /// The fades come **first** so that normalisation sees the final waveform:
    /// every finished clip therefore peaks at exactly [`NORMALISE_PEAK`], and
    /// relative loudness is decided by the mixer's gain table alone.
    pub fn finish(mut self) -> SampleClip {
        let len = self.buf.len();
        let head = ((HEAD_FADE_SECS * SR) as usize).min(len / 4).max(1);
        let tail = ((TAIL_FADE_SECS * SR) as usize).min(len / 4).max(1);
        for i in 0..head {
            self.buf[i] *= raised_cosine(i as f32 / head as f32);
        }
        for i in 0..tail {
            let idx = len - 1 - i;
            self.buf[idx] *= raised_cosine(i as f32 / tail as f32);
        }

        // Straight normalisation: the scale is exact, so the finished peak is
        // exactly `NORMALISE_PEAK` and no saturation colours the result.
        let peak = self.buf.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
        if peak > 1e-6 {
            let scale = NORMALISE_PEAK / peak;
            for sample in self.buf.iter_mut() {
                *sample *= scale;
            }
        }

        SampleClip {
            data: self.buf.into(),
        }
    }
}

/// The attack every clip is allowed before [`worst_envelope_step`] starts
/// looking. Percussion is *supposed* to arrive fast; what it must not do is
/// arrive from a discontinuity, which [`HEAD_FADE_SECS`] and the soft-attack
/// assertions cover instead.
#[cfg(test)]
pub const ATTACK_GRACE_SECS: f32 = 0.012;

/// Worst frame-to-frame jump in a clip's envelope after its attack, as a
/// fraction of its peak.
///
/// **This, not the raw sample slope, is what a click is.** A 5 kHz partial at
/// full scale legitimately moves almost the whole range between two samples at
/// 22 kHz; what the ear reads as a crack is the *envelope* arriving or
/// vanishing instantly. Windowed RMS catches that and ignores the waveform
/// underneath it. A hard cut anywhere — a layer starting at full level, a clip
/// truncated mid-signal — shows up here as a step near `1.0`.
#[cfg(test)]
pub fn worst_envelope_step(clip: &SampleClip) -> f32 {
    // Twelve milliseconds is about one cycle of the lowest thing in the bank
    // (a 70 Hz bridge thump). A shorter window would measure the waveform of a
    // bass partial rather than its envelope and report a click that is not one.
    let window = (0.012 * SR) as usize;
    let peak = clip.data.iter().fold(0.0f32, |a, s| a.max(s.abs()));
    if peak < 1e-6 || window == 0 {
        return 0.0;
    }
    let skip = (ATTACK_GRACE_SECS * SR) as usize;
    let mut worst = 0.0f32;
    let mut previous: Option<f32> = None;
    for (i, chunk) in clip.data.chunks(window).enumerate() {
        let rms =
            (chunk.iter().map(|s| (s * s) as f64).sum::<f64>() / chunk.len() as f64).sqrt() as f32;
        if i * window >= skip {
            if let Some(previous) = previous {
                worst = worst.max((rms - previous).abs());
            }
            previous = Some(rms);
        }
    }
    // The last window is compared against silence too: a clip truncated
    // mid-signal is exactly the failure this is here to catch.
    worst.max(previous.unwrap_or(0.0)) / peak
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(clip: &SampleClip) -> f32 {
        clip.data.iter().fold(0.0f32, |a, s| a.max(s.abs()))
    }

    #[test]
    fn a_finished_clip_is_normalised_and_silent_at_both_ends() {
        let mut canvas = Canvas::new(0.2);
        canvas.partial(0.0, 440.0, 0.02, 0.001, 0.05, 0.2, 1.0);
        let clip = canvas.finish();
        assert!((peak(&clip) - NORMALISE_PEAK).abs() < 0.02, "peak {}", peak(&clip));
        assert_eq!(clip.data[0], 0.0, "clips must start from silence");
        assert_eq!(*clip.data.last().unwrap(), 0.0, "clips must end in silence");
    }

    #[test]
    fn a_percussive_clip_still_has_no_discontinuity() {
        let mut rng = Rng::new(3);
        let mut canvas = Canvas::new(0.35);
        canvas.noise_band(&mut rng, 0.0, 0.08, 2600.0, 1800.0, 6.0, 1.0, 0.003, 0.02);
        canvas.partial(0.0, 90.0, 0.7, 0.004, 0.06, 0.3, 0.92);
        let clip = canvas.finish();
        let step = worst_envelope_step(&clip);
        assert!(step < 0.35, "envelope jumped by {step} of full scale");
    }

    #[test]
    fn the_de_click_check_catches_a_real_cut() {
        // Guard the guard: a clip truncated while it is still sounding must
        // fail, or the check above proves nothing.
        let mut canvas = Canvas::new(0.7);
        canvas.partial(0.0, 300.0, 1.0, 0.01, 0.35, 0.7, 1.0);
        let clip = canvas.finish();
        assert!(
            worst_envelope_step(&clip) < 0.35,
            "the intact clip is fine: {}",
            worst_envelope_step(&clip)
        );
        let cut = SampleClip {
            data: clip.data[..clip.data.len() / 3].into(),
        };
        assert!(
            worst_envelope_step(&cut) > 0.35,
            "a truncated clip must be detected"
        );
    }

    #[test]
    fn the_head_fade_holds_the_first_millisecond_down() {
        // No sharp transients at volume (§1): whatever the synthesis did, the
        // opening millisecond of any clip is a fraction of its peak.
        let mut rng = Rng::new(9);
        let mut canvas = Canvas::new(0.2);
        // A deliberately brutal source: full-scale noise from sample zero.
        canvas.noise_band(&mut rng, 0.0, 0.2, 3000.0, 3000.0, 1.0, 1.0, 0.0, 10.0);
        let clip = canvas.finish();
        let ms = (0.001 * SR) as usize;
        let early = clip.data[..ms].iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(
            early < peak(&clip) * 0.25,
            "first ms reached {early} of {}",
            peak(&clip)
        );
    }

    #[test]
    fn everything_stays_finite() {
        let mut rng = Rng::new(11);
        let mut canvas = Canvas::new(0.5);
        canvas.noise_band(&mut rng, 0.0, 0.5, 40.0, 9000.0, 30.0, 4.0, 0.0, 1.0);
        canvas.partial_vibrato(0.0, 700.0, 1.0, 0.01, 0.4, 0.5, 0.9, 6.0, 40.0);
        canvas.low_pass(300.0);
        let clip = canvas.finish();
        assert!(clip.data.iter().all(|s| s.is_finite()));
        assert!(peak(&clip) <= 1.0);
    }

    #[test]
    fn the_reader_returns_every_sample_once_then_stops() {
        let mut canvas = Canvas::new(0.02);
        canvas.partial(0.0, 300.0, 0.5, 0.001, 0.01, 0.02, 1.0);
        let clip = canvas.finish();
        let expected = clip.data.len();
        let mut reader = clip.decoder();
        assert_eq!(reader.by_ref().count(), expected);
        assert!(reader.next().is_none());
        assert_eq!(clip.decoder().sample_rate(), SAMPLE_RATE);
        assert_eq!(clip.decoder().channels(), 1);
    }

    #[test]
    fn low_pass_removes_high_content() {
        let mut bright = Canvas::new(0.3);
        bright.partial(0.0, 5000.0, 1.0, 0.005, 0.2, 0.3, 1.0);
        let bright_energy: f32 = bright.buf.iter().map(|s| s * s).sum();

        let mut dull = Canvas::new(0.3);
        dull.partial(0.0, 5000.0, 1.0, 0.005, 0.2, 0.3, 1.0);
        dull.low_pass(500.0);
        let dull_energy: f32 = dull.buf.iter().map(|s| s * s).sum();

        assert!(
            dull_energy < bright_energy * 0.25,
            "distance pass must actually muffle: {dull_energy} vs {bright_energy}"
        );
    }
}
