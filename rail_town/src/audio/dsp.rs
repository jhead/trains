//! Synthesis primitives shared by the offline clip renderer and the live voices.
//!
//! Everything in this file is deterministic, allocation-free and cheap enough to
//! run on the audio callback thread. No trigonometry beyond `sin` / `cos` per
//! *control block* where it can be helped, and every filter is guarded against
//! blowing up — a NaN on the audio thread is a burst of full-scale noise, which
//! is precisely the "never startle" failure the brief forbids
//! (`docs/design/10-audio-and-feel.md` §1).

use core::f32::consts::{PI, TAU};

/// A small deterministic PRNG (SplitMix64).
///
/// Used both offline (baking the one-shot bank) and live (gust envelopes, event
/// scheduling). Deterministic so a given world always sounds the same, and so
/// the tests can assert on rendered buffers.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub const fn new(seed: u64) -> Self {
        // Any odd seed; 0 would still work for SplitMix64 but this keeps the
        // first draw from a fresh `Rng::new(0)` from being trivially small.
        Self(seed ^ 0xa076_1d64_78bd_642f)
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    #[inline]
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / 16_777_216.0
    }

    /// Uniform in `[-1, 1)`.
    #[inline]
    pub fn bipolar(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    /// Uniform in `[lo, hi)`.
    #[inline]
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    #[inline]
    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    /// Uniform integer in `[0, n)`; `0` when `n == 0`.
    #[inline]
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }
}

/// One-pole smoother. `lp` low-passes, `hp` returns the complement.
#[derive(Debug, Clone, Copy, Default)]
pub struct OnePole {
    y: f32,
}

impl OnePole {
    #[inline]
    pub fn lp(&mut self, x: f32, a: f32) -> f32 {
        self.y += (x - self.y) * a;
        if !self.y.is_finite() {
            self.y = 0.0;
        }
        self.y
    }

    #[inline]
    pub fn hp(&mut self, x: f32, a: f32) -> f32 {
        x - self.lp(x, a)
    }
}

/// One-pole coefficient for a cutoff in Hz.
#[inline]
pub fn pole_coeff(cutoff_hz: f32, sample_rate: f32) -> f32 {
    let fc = cutoff_hz.clamp(0.5, sample_rate * 0.45);
    (1.0 - (-TAU * fc / sample_rate).exp()).clamp(0.0, 1.0)
}

/// Gain that restores roughly unit RMS after a one-pole low-pass with
/// coefficient `a` has been applied to unit-variance noise.
///
/// Filtering noise throws away most of its energy. Without this every layer
/// would need a hand-tuned make-up gain, and the balance between the ambience
/// beds would drift every time somebody moved a cutoff.
#[inline]
pub fn lp_norm(a: f32) -> f32 {
    ((2.0 - a) / a.max(1e-6)).sqrt().clamp(1.0, 64.0)
}

/// The same idea for the [`Svf`] band output. Approximate — the beds are
/// balanced by ear against the RMS test in this module, not by this formula
/// alone — but close enough that a cutoff change does not rebalance the mix.
#[inline]
pub fn band_norm(f: f32, damp: f32) -> f32 {
    (2.0 * damp / f.max(1e-6)).sqrt().clamp(0.5, 64.0)
}

/// Chamberlin state-variable filter. Gives low / band / high in one pass and
/// tolerates per-sample cutoff modulation, which is what the swept sounds want.
#[derive(Debug, Clone, Copy, Default)]
pub struct Svf {
    low: f32,
    band: f32,
}

/// One sample of [`Svf`] output.
#[derive(Debug, Clone, Copy)]
pub struct SvfOut {
    pub low: f32,
    pub band: f32,
    /// The high-pass tap. Nothing in the palette is bright enough to want it
    /// yet; it costs nothing to expose and the filter computes it anyway.
    #[allow(dead_code)]
    pub high: f32,
}

impl Svf {
    #[inline]
    pub fn step(&mut self, x: f32, f: f32, damp: f32) -> SvfOut {
        // `f` above ~0.9 goes unstable; `damp` below ~0.02 rings forever.
        let f = f.clamp(0.0005, 0.85);
        let damp = damp.clamp(0.03, 2.0);
        self.low += f * self.band;
        let high = x - self.low - damp * self.band;
        self.band += f * high;
        if !self.low.is_finite() || !self.band.is_finite() {
            self.low = 0.0;
            self.band = 0.0;
        }
        SvfOut {
            low: self.low,
            band: self.band,
            high,
        }
    }
}

/// [`Svf`] tuning coefficient for a centre frequency.
#[inline]
pub fn svf_f(center_hz: f32, sample_rate: f32) -> f32 {
    let hz = center_hz.clamp(1.0, sample_rate * 0.24);
    2.0 * (PI * hz / sample_rate).sin()
}

/// Damping for a "Q"-like resonance figure. Higher `q` rings longer.
#[inline]
pub fn svf_damp(q: f32) -> f32 {
    1.0 / q.clamp(0.25, 30.0)
}

/// Bounded saturation. Transparent below about ±0.3, so it only ever engages on
/// an accidental sum — a limiter, not a colour.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    x.tanh()
}

/// Raised-cosine ramp over `t` in `0..=1`. Every fade in this module uses it:
/// linear ramps on short envelopes are audible as a click on the corner.
#[inline]
pub fn raised_cosine(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    0.5 - 0.5 * (t * PI).cos()
}

/// Exponential decay to `exp(-t / tau)`.
#[inline]
pub fn exp_decay(t: f32, tau: f32) -> f32 {
    (-t / tau.max(1e-4)).exp()
}

/// Frame-rate independent approach toward `target` with time constant `tau`.
///
/// This is the only smoothing the ECS side uses. Every gain the mixer writes
/// goes through it, which is what makes "never startle" structural rather than
/// a rule somebody has to remember.
#[inline]
pub fn approach(current: f32, target: f32, dt: f32, tau: f32) -> f32 {
    if tau <= 1e-4 || dt <= 0.0 {
        return target;
    }
    let a = 1.0 - (-dt / tau).exp();
    let next = current + (target - current) * a;
    if next.is_finite() {
        next
    } else {
        target
    }
}

/// Linear interpolation.
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// `0` below `lo`, `1` above `hi`, smooth in between.
#[inline]
pub fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    if (hi - lo).abs() < f32::EPSILON {
        return if x < lo { 0.0 } else { 1.0 };
    }
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Semitones above a root, as a frequency ratio.
#[inline]
pub fn semitones(n: f32) -> f32 {
    (n / 12.0).exp2()
}

/// A cheap band-limited-ish sine from a normalised phase in `0..1`.
#[inline]
pub fn sine(phase: f32) -> f32 {
    (phase * TAU).sin()
}

/// Advance and wrap a normalised phase accumulator.
#[inline]
pub fn advance_phase(phase: &mut f32, hz: f32, sample_rate: f32) -> f32 {
    *phase += hz / sample_rate;
    if *phase >= 1.0 {
        *phase -= phase.floor();
    }
    if !phase.is_finite() {
        *phase = 0.0;
    }
    *phase
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_in_range() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..2000 {
            let x = a.unit();
            assert_eq!(x, b.unit());
            assert!((0.0..1.0).contains(&x), "unit() out of range: {x}");
            assert!((-1.0..1.0).contains(&a.bipolar()));
            let _ = b.bipolar();
        }
        assert_ne!(Rng::new(7).next_u64(), Rng::new(8).next_u64());
    }

    #[test]
    fn filters_stay_bounded_on_full_scale_noise() {
        // A NaN or a runaway here is a full-scale burst in the player's ears.
        let mut rng = Rng::new(1);
        let mut svf = Svf::default();
        let mut pole = OnePole::default();
        let mut worst = 0.0f32;
        for i in 0..40_000 {
            let x = rng.bipolar();
            // Sweep the cutoff across the whole legal range while we do it.
            let f = svf_f(60.0 + 5000.0 * (i as f32 / 40_000.0), 22_050.0);
            let out = svf.step(x, f, svf_damp(6.0));
            let lp = pole.lp(out.band, pole_coeff(800.0, 22_050.0));
            for v in [out.low, out.band, out.high, lp] {
                assert!(v.is_finite(), "filter produced {v}");
                worst = worst.max(v.abs());
            }
        }
        assert!(worst < 50.0, "filter resonance ran away: {worst}");
    }

    #[test]
    fn approach_converges_and_never_overshoots() {
        let mut v = 0.0;
        for _ in 0..600 {
            v = approach(v, 1.0, 1.0 / 60.0, 0.25);
            assert!((0.0..=1.0).contains(&v), "overshoot: {v}");
        }
        assert!(v > 0.999, "did not converge: {v}");
        // Zero time constant is an immediate jump, not a divide by zero.
        assert_eq!(approach(0.0, 0.5, 1.0 / 60.0, 0.0), 0.5);
    }

    #[test]
    fn soft_clip_is_transparent_quiet_and_bounded_loud() {
        assert!((soft_clip(0.05) - 0.05).abs() < 1e-3);
        assert!(soft_clip(40.0) <= 1.0);
        assert!(soft_clip(-40.0) >= -1.0);
    }

    #[test]
    fn raised_cosine_has_zero_slope_at_both_ends() {
        assert_eq!(raised_cosine(0.0), 0.0);
        assert!((raised_cosine(1.0) - 1.0).abs() < 1e-6);
        // Flat corners are what keep a 3 ms attack from reading as a click.
        assert!(raised_cosine(0.02) < 0.01);
        assert!(raised_cosine(0.98) > 0.99);
    }

    #[test]
    fn semitones_matches_an_octave() {
        assert!((semitones(12.0) - 2.0).abs() < 1e-5);
        assert!((semitones(0.0) - 1.0).abs() < 1e-6);
    }
}
