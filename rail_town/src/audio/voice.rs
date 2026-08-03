//! [`LiveVoice`] — an endless procedural source with live, lock-free controls.
//!
//! The ambience beds, the train rolling loops and the music are all generated a
//! sample at a time on the audio thread rather than looped from a buffer. That
//! is not showing off: the brief's hard constraint is that **nothing loops
//! audibly** (§1, §4), and a generator has no loop point to hear. It also means
//! a bed costs a few hundred bytes instead of a megabyte, and that its character
//! can be steered continuously from the ECS.
//!
//! Control flows one way and lock-free: the ECS writes [`VoiceParams`] (five
//! `f32`s behind relaxed atomics), the audio thread reads them once per control
//! block and slews toward them. Nothing on the audio thread allocates, locks, or
//! can block the ECS.
//!
//! Loudness is **not** a parameter. Gain lives on the sink, where the mixer
//! smooths it every frame; a voice only decides what it sounds like, never how
//! loud it is.

use core::time::Duration;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use bevy::asset::Asset;
use bevy::audio::{Decodable, Source as RodioSource};
use bevy::reflect::TypePath;

use super::clip::{SAMPLE_RATE, SR};
use super::dsp::{
    advance_phase, band_norm, exp_decay, lerp, lp_norm, pole_coeff, raised_cosine, sine, soft_clip,
    svf_damp, svf_f, OnePole, Rng, Svf,
};

/// Samples between parameter reads. 64 at 22.05 kHz is 2.9 ms — far finer than
/// the ECS writes them, and coarse enough that the atomics are free.
const CONTROL_BLOCK: u32 = 64;

/// Concurrent one-shot events inside a single voice (birds, sleepers, notes).
const MAX_GRAINS: usize = 10;

/// Slew time for parameter changes, in control blocks.
///
/// About 180 ms. Fast enough to track a train accelerating, slow enough that a
/// jump in the ECS value can never arrive as a step.
const PARAM_SLEW: f32 = 0.02;

/// Which generator a [`LiveVoice`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VoiceKind {
    /// Base layer everywhere; thinner and higher at altitude.
    Wind,
    /// Surf at the coast, running water inland.
    Water,
    /// Leaves and birds, with long gaps.
    Forest,
    /// Crickets and the near-silence of a night map.
    Night,
    /// Distant murmur, scaling with density — the emotional layer.
    Town,
    /// Muffled machinery, only while working.
    Industry,
    /// A platform crowd, scaling with how many are waiting (brief 10 §3.3).
    Murmur,
    /// A train in motion: roll, sleepers, rumble.
    Rolling,
    /// The sparse ambient score.
    Music,
}

impl VoiceKind {
    /// Voices whose character is a landscape rather than an object.
    pub const BEDS: [VoiceKind; 7] = [
        VoiceKind::Wind,
        VoiceKind::Water,
        VoiceKind::Forest,
        VoiceKind::Night,
        VoiceKind::Town,
        VoiceKind::Industry,
        VoiceKind::Murmur,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Wind => "wind",
            Self::Water => "water",
            Self::Forest => "forest",
            Self::Night => "night",
            Self::Town => "town",
            Self::Industry => "industry",
            Self::Murmur => "murmur",
            Self::Rolling => "rolling",
            Self::Music => "music",
        }
    }
}

/// Five continuous controls, shared between the ECS and the audio thread.
///
/// All are `0.0..=1.0`. The meaning is per-kind and documented on each setter's
/// caller; the ranges are uniform so a voice can never be driven out of bounds
/// by a caller that misremembers the scale.
///
/// [`Self::cue`] is the exception: a counter rather than a level, and the only
/// control that is not smoothed. The score needs to be told *when a cue begins*
/// so that every cue opens with the theme rather than wherever the generator
/// happened to have got to, and a monotonically increasing integer is the
/// cheapest edge-trigger there is across a lock-free boundary.
#[derive(Debug, Default)]
pub struct VoiceParams {
    tone: AtomicU32,
    motion: AtomicU32,
    depth: AtomicU32,
    density: AtomicU32,
    color: AtomicU32,
    cue: AtomicU32,
}

macro_rules! param {
    ($get:ident, $set:ident, $field:ident, $doc:literal) => {
        #[doc = $doc]
        #[inline]
        pub fn $get(&self) -> f32 {
            f32::from_bits(self.$field.load(Ordering::Relaxed))
        }

        #[doc = $doc]
        #[inline]
        pub fn $set(&self, value: f32) {
            let clamped = if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            };
            self.$field.store(clamped.to_bits(), Ordering::Relaxed);
        }
    };
}

impl VoiceParams {
    param!(
        tone,
        set_tone,
        tone,
        "Brightness. Distance and altitude close this down; it is the low-pass the brief asks for in S7."
    );
    param!(
        motion,
        set_motion,
        motion,
        "Rate of movement: gust speed, wave period, wheel speed, work rate."
    );
    param!(
        depth,
        set_depth,
        depth,
        "Weight and low end: freight mass, town size, music register."
    );
    param!(
        density,
        set_density,
        density,
        "Sparse-event rate: birdsong, crickets, chatter, how often a note enters."
    );
    param!(
        color,
        set_color,
        color,
        "Per-kind variant: river vs surf, day vs night town, which musical root."
    );

    /// Which cue the score is playing. `0` is silence; any change restarts the
    /// piece from bar one.
    #[inline]
    pub fn cue(&self) -> u32 {
        self.cue.load(Ordering::Relaxed)
    }

    /// Set the cue number. See [`Self::cue`].
    #[inline]
    pub fn set_cue(&self, value: u32) {
        self.cue.store(value, Ordering::Relaxed);
    }

    /// A params block with sensible mid-scale defaults.
    pub fn new() -> Arc<Self> {
        let params = Arc::new(Self::default());
        params.set_tone(0.5);
        params.set_motion(0.4);
        params.set_depth(0.5);
        params.set_density(0.3);
        params.set_color(0.0);
        params.set_cue(0);
        params
    }
}

/// An endless generated sound with live controls.
#[derive(Asset, TypePath, Clone, Debug)]
pub struct LiveVoice {
    pub kind: VoiceKind,
    pub params: Arc<VoiceParams>,
    pub seed: u64,
}

impl LiveVoice {
    pub fn new(kind: VoiceKind, seed: u64) -> Self {
        Self {
            kind,
            params: VoiceParams::new(),
            seed,
        }
    }
}

impl Decodable for LiveVoice {
    type DecoderItem = f32;
    type Decoder = VoiceRender;

    fn decoder(&self) -> VoiceRender {
        VoiceRender::new(self.kind, self.params.clone(), self.seed)
    }
}

/// One scheduled event inside a voice — a chirp, a sleeper, a held note.
#[derive(Debug, Clone, Copy, Default)]
struct Grain {
    active: bool,
    /// Samples still to wait before the grain sounds (staggered chirps).
    delay: u32,
    age: u32,
    len: u32,
    attack: u32,
    tau: f32,
    amp: f32,
    freq: f32,
    freq_end: f32,
    /// Resonance for noise grains.
    q: f32,
    /// `true` for band-passed noise (clatter, sleepers), `false` for a partial.
    noise: bool,
    phase: f32,
    svf: Svf,
}

impl Grain {
    #[allow(clippy::too_many_arguments)]
    fn tone(delay: u32, freq: f32, freq_end: f32, amp: f32, attack: f32, tau: f32, dur: f32) -> Self {
        Self {
            active: true,
            delay,
            age: 0,
            len: (dur * SR) as u32,
            attack: (attack * SR).max(1.0) as u32,
            tau,
            amp,
            freq,
            freq_end,
            q: 1.0,
            noise: false,
            phase: 0.0,
            svf: Svf::default(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn band(
        delay: u32,
        freq: f32,
        freq_end: f32,
        q: f32,
        amp: f32,
        attack: f32,
        tau: f32,
        dur: f32,
    ) -> Self {
        Self {
            active: true,
            delay,
            age: 0,
            len: (dur * SR) as u32,
            attack: (attack * SR).max(1.0) as u32,
            tau,
            amp,
            freq,
            freq_end,
            q,
            noise: true,
            phase: 0.0,
            svf: Svf::default(),
        }
    }

    #[inline]
    fn step(&mut self, rng: &mut Rng) -> f32 {
        if !self.active {
            return 0.0;
        }
        if self.delay > 0 {
            self.delay -= 1;
            return 0.0;
        }
        if self.age >= self.len {
            self.active = false;
            return 0.0;
        }
        let t = self.age as f32 / SR;
        let env = if self.age < self.attack {
            raised_cosine(self.age as f32 / self.attack as f32)
        } else {
            1.0
        } * exp_decay(t, self.tau);
        let progress = self.age as f32 / self.len as f32;
        let f = lerp(self.freq, self.freq_end, progress);
        let out = if self.noise {
            let damp = svf_damp(self.q);
            self.svf.step(rng.bipolar(), svf_f(f, SR), damp).band * band_norm(svf_f(f, SR), damp)
        } else {
            advance_phase(&mut self.phase, f, SR);
            sine(self.phase)
        };
        self.age += 1;
        if env < 1e-4 && self.age > self.attack {
            self.active = false;
        }
        out * env * self.amp
    }
}

/// The audio-thread half of a [`LiveVoice`].
pub struct VoiceRender {
    kind: VoiceKind,
    params: Arc<VoiceParams>,
    rng: Rng,

    // Slewed copies of the controls.
    tone: f32,
    motion: f32,
    depth: f32,
    density: f32,
    color: f32,

    lp_a: OnePole,
    lp_b: OnePole,
    lp_c: OnePole,
    out_lp: OnePole,
    svf_a: Svf,
    svf_b: Svf,

    ph_a: f32,
    ph_b: f32,
    lfo_a: f32,
    lfo_b: f32,
    lfo_c: f32,

    grains: [Grain; MAX_GRAINS],
    /// Samples until the next sparse event.
    next_event: u32,
    ctl: u32,

    /// The composed piece and its plucked strings — only for
    /// [`VoiceKind::Music`].
    ///
    /// Boxed and optional so the six ambience beds do not each carry twelve
    /// unused delay lines. The allocation happens in [`Decodable::decoder`],
    /// which `bevy_audio` calls from the ECS when the sink is created, never on
    /// the audio callback.
    score: Option<Box<super::score::Score>>,
    /// The cue number the score is following, read every control block.
    cue: u32,
}

impl VoiceRender {
    fn new(kind: VoiceKind, params: Arc<VoiceParams>, seed: u64) -> Self {
        let mut render = Self {
            kind,
            rng: Rng::new(seed.wrapping_mul(0x2545_f491_4f6c_dd1d) ^ 0x1234_5678),
            tone: params.tone(),
            motion: params.motion(),
            depth: params.depth(),
            density: params.density(),
            color: params.color(),
            params,
            lp_a: OnePole::default(),
            lp_b: OnePole::default(),
            lp_c: OnePole::default(),
            out_lp: OnePole::default(),
            svf_a: Svf::default(),
            svf_b: Svf::default(),
            ph_a: 0.0,
            ph_b: 0.0,
            lfo_a: 0.0,
            lfo_b: 0.37,
            lfo_c: 0.71,
            grains: [Grain::default(); MAX_GRAINS],
            next_event: 0,
            ctl: 0,
            score: match kind {
                VoiceKind::Music => Some(Box::new(super::score::Score::new(seed))),
                _ => None,
            },
            cue: 0,
        };
        // Decorrelate the slow modulators so two voices of the same kind never
        // breathe in lockstep.
        render.lfo_a = render.rng.unit();
        render.lfo_b = render.rng.unit();
        render.lfo_c = render.rng.unit();
        render
    }

    #[inline]
    fn read_controls(&mut self) {
        self.ctl += 1;
        if self.ctl < CONTROL_BLOCK {
            return;
        }
        self.ctl = 0;
        let slew = PARAM_SLEW;
        self.tone += (self.params.tone() - self.tone) * slew;
        self.motion += (self.params.motion() - self.motion) * slew;
        self.depth += (self.params.depth() - self.depth) * slew;
        self.density += (self.params.density() - self.density) * slew;
        // The variant selector jumps: it is only ever changed while the voice is
        // silent, and slewing it would sweep through the values in between.
        self.color = self.params.color();
        self.cue = self.params.cue();
    }

    /// Free a grain slot, preferring one that has already finished.
    fn spawn_grain(&mut self, grain: Grain) {
        if let Some(slot) = self.grains.iter_mut().find(|g| !g.active) {
            *slot = grain;
        }
    }

    fn grains_out(&mut self) -> f32 {
        let mut sum = 0.0;
        for i in 0..MAX_GRAINS {
            if self.grains[i].active {
                let mut grain = self.grains[i];
                sum += grain.step(&mut self.rng);
                self.grains[i] = grain;
            }
        }
        sum
    }

    /// Two incommensurate slow sines. Used everywhere a "random" slow drift is
    /// wanted: it never repeats on any timescale a player will notice, and
    /// unlike filtered noise it needs no make-up gain.
    #[inline]
    fn drift(&mut self, hz_a: f32, hz_b: f32) -> f32 {
        let a = sine(advance_phase(&mut self.lfo_a, hz_a, SR));
        let b = sine(advance_phase(&mut self.lfo_b, hz_b, SR));
        0.5 * a + 0.5 * b
    }

    /// Schedule `next_event` for a mean interval in seconds, with jitter.
    #[inline]
    fn schedule(&mut self, mean_secs: f32) {
        let jitter = self.rng.range(0.45, 1.75);
        self.next_event = ((mean_secs * jitter).clamp(0.02, 90.0) * SR) as u32;
    }

    // -- generators ---------------------------------------------------------

    fn wind(&mut self) -> f32 {
        let n = self.rng.bipolar();
        // Thinner and higher at altitude (brief §2): `tone` is the altitude.
        let cut = lerp(340.0, 1500.0, self.tone);
        let a1 = pole_coeff(cut, SR);
        let a2 = pole_coeff(cut * 1.8, SR);
        let hiss = self.lp_a.lp(self.lp_b.lp(n, a1), a2) * lp_norm(a1) * lp_norm(a2);
        let body_a = pole_coeff(110.0, SR);
        let body = self.lp_c.lp(n, body_a) * lp_norm(body_a);

        let gust = 0.60 + 0.40 * self.drift(0.031 + 0.06 * self.motion, 0.017);
        let weight = 1.0 - self.tone;
        // Playtest: "there's this white noise that seems to play continually. I
        // think it's kinda nice but should be brought down in volume a bit."
        //
        // This is it. The wind is the one bed that is never absent, so it is
        // the one bed whose level is judged over two hours rather than over two
        // seconds, and it was reaching the mix at about -23 dBFS. The hiss is
        // down 4.6 dB here, `gain::AMBIENCE_TOTAL` takes another 1.7, and the
        // ambience slider — which now actually applies — takes 3 more at its
        // default of 70%. It lands near -32 dBFS: still plainly there, no
        // longer the loudest thing in a quiet frame. The gust and the body keep
        // their shape, so it is quieter without becoming thinner.
        (hiss * (0.19 + 0.19 * self.tone) + body * 0.18 * weight) * gust
    }

    fn water(&mut self) -> f32 {
        let n = self.rng.bipolar();
        // `color` is coast (1) versus river (0): surf swells, a river does not.
        let swell_depth = 0.15 + 0.55 * self.color;
        let swell = 1.0 - swell_depth
            + swell_depth * (0.5 + 0.5 * self.drift(0.075 + 0.06 * self.motion, 0.041));
        // Surf breaks in the upper band; a river is narrower and lower.
        let center = lerp(520.0, 1500.0, self.tone);
        let f = svf_f(center, SR);
        let damp = svf_damp(lerp(1.6, 0.9, self.color));
        let band = self.svf_a.step(n, f, damp).band * band_norm(f, damp);
        let low_a = pole_coeff(150.0, SR);
        let low = self.lp_a.lp(n, low_a) * lp_norm(low_a);
        (band * 0.30 + low * 0.16) * swell.powf(1.6)
    }

    fn forest(&mut self) -> f32 {
        // Leaves: a quiet high band that moves with the same wind as the bed.
        let n = self.rng.bipolar();
        let hp_a = pole_coeff(900.0, SR);
        let lp_a = pole_coeff(4200.0, SR);
        let leaves = self.lp_b.lp(self.lp_a.hp(n, hp_a), lp_a) * lp_norm(lp_a) * 0.18;
        let rustle = 0.55 + 0.45 * self.drift(0.043, 0.021);

        // Birds: sparse by construction. At full density the mean gap is still
        // over a second — "sparse, with long gaps" is the brief's word for it.
        if self.next_event == 0 {
            let mean = lerp(14.0, 1.6, self.density);
            self.schedule(mean);
            if self.density > 0.02 {
                let base = self.rng.range(2100.0, 3900.0);
                let calls = 1 + self.rng.below(3);
                for c in 0..calls {
                    let delay = (self.rng.range(0.0, 0.10) * SR) as u32 + (c as u32 * 1500);
                    let up = self.rng.chance(0.6);
                    let (from, to) = if up {
                        (base * 0.82, base * 1.18)
                    } else {
                        (base * 1.15, base * 0.86)
                    };
                    let amp = self.rng.range(0.10, 0.20) * (0.4 + 0.6 * self.density);
                    self.spawn_grain(Grain::tone(delay, from, to, amp, 0.008, 0.035, 0.07));
                }
            }
        } else {
            self.next_event -= 1;
        }

        leaves * rustle + self.grains_out()
    }

    fn night(&mut self) -> f32 {
        // Near-silence with a texture in it. The bed is barely there; the
        // crickets are the content.
        let n = self.rng.bipolar();
        let a = pole_coeff(260.0, SR);
        let hush = self.lp_a.lp(n, a) * lp_norm(a) * 0.09;

        if self.next_event == 0 {
            let mean = lerp(9.0, 1.1, self.density);
            self.schedule(mean);
            if self.density > 0.02 {
                // A cricket is a short trill, not a beep: three pulses.
                let freq = self.rng.range(3600.0, 4700.0);
                let amp = self.rng.range(0.05, 0.11) * (0.4 + 0.6 * self.density);
                for p in 0..3 {
                    let delay = (p as f32 * 0.045 * SR) as u32;
                    self.spawn_grain(Grain::band(delay, freq, freq, 14.0, amp, 0.004, 0.012, 0.03));
                }
            } else if self.rng.chance(0.05) {
                // A distant owl, twice a night at most.
                let f = self.rng.range(300.0, 380.0);
                self.spawn_grain(Grain::tone(0, f, f * 0.97, 0.10, 0.09, 0.30, 0.55));
                self.spawn_grain(Grain::tone(
                    (0.45 * SR) as u32,
                    f * 0.99,
                    f * 0.95,
                    0.08,
                    0.09,
                    0.26,
                    0.5,
                ));
            }
        } else {
            self.next_event -= 1;
        }

        hush + self.grains_out()
    }

    fn town(&mut self) -> f32 {
        // The emotional layer (brief §2). Everything here scales with density so
        // a thriving district is audibly alive and a declining one audibly
        // quiet — the sink gain alone would only make it *quieter*, which is
        // not the same thing as *emptier*.
        let n = self.rng.bipolar();
        let live = self.density;

        // Murmur: a voice-shaped band that opens up as the town fills.
        let center = lerp(240.0, 560.0, self.tone);
        let f = svf_f(center, SR);
        let damp = svf_damp(1.1);
        let murmur = self.svf_a.step(n, f, damp).band * band_norm(f, damp);
        // A second formant only appears once there is a crowd to make it.
        let f2 = svf_f(lerp(700.0, 1150.0, self.tone), SR);
        let damp2 = svf_damp(1.4);
        let upper =
            self.svf_b.step(n, f2, damp2).band * band_norm(f2, damp2) * (live * live * 0.5);
        let breath = 0.45 + 0.55 * (0.5 + 0.5 * self.drift(0.055, 0.029));

        // Activity: doors, carts, work. The rate is the diagnostic — at a dead
        // town it is one event every twenty seconds, at a thriving one it is
        // most of a second.
        if self.next_event == 0 {
            let mean = lerp(22.0, 0.85, live);
            self.schedule(mean);
            if live > 0.03 {
                let bright = 0.35 + 0.65 * self.tone;
                let freq = self.rng.range(600.0, 1900.0) * bright;
                let amp = self.rng.range(0.05, 0.13) * (0.3 + 0.7 * live);
                let slide = self.rng.range(0.8, 1.0);
                let tail = self.rng.range(0.02, 0.09);
                self.spawn_grain(Grain::band(
                    0, freq, freq * slide, 5.0, amp, 0.006, tail, 0.16,
                ));
                // Voices carry further than footsteps, and only from a crowd.
                if live > 0.5 && self.rng.chance(0.35) {
                    let v = self.rng.range(230.0, 480.0);
                    let delay = (self.rng.range(0.0, 0.4) * SR) as u32;
                    let bend = self.rng.range(0.86, 1.14);
                    self.spawn_grain(Grain::tone(
                        delay, v, v * bend, 0.05 * live, 0.05, 0.14, 0.30,
                    ));
                }
            }
        } else {
            self.next_event -= 1;
        }

        (murmur * 0.32 + upper * 0.22) * breath * (0.25 + 0.75 * live) + self.grains_out()
    }

    fn murmur(&mut self) -> f32 {
        // Brief 10 §3.3: a platform crowd, scaling with how many are waiting.
        // `density` is that count, normalised; `tone` is how near the platform
        // is. Deliberately *not* the town bed with a different weight — a
        // crowd is voices, and voices sit in a narrower, higher band than the
        // doors-and-carts activity of a district.
        let n = self.rng.bipolar();
        let crowd = self.density;

        // A vowel-shaped pair of formants: the sound of people rather than the
        // sound of a place.
        let f1 = svf_f(lerp(320.0, 520.0, self.tone), SR);
        let d1 = svf_damp(1.6);
        let low = self.svf_a.step(n, f1, d1).band * band_norm(f1, d1);
        let f2 = svf_f(lerp(900.0, 1400.0, self.tone), SR);
        let d2 = svf_damp(2.2);
        let high = self.svf_b.step(n, f2, d2).band * band_norm(f2, d2) * crowd;
        // Conversation arrives in swells, never at a steady level.
        let swell = 0.55 + 0.45 * (0.5 + 0.5 * self.drift(0.09, 0.041));

        // The odd voice carrying over the rest. Sparse: this is texture, not an
        // alarm, and a platform that chattered would be the loudest thing on a
        // quiet map.
        if self.next_event == 0 {
            self.schedule(lerp(11.0, 1.6, crowd));
            if crowd > 0.05 {
                let v = self.rng.range(190.0, 420.0);
                let delay = (self.rng.range(0.0, 0.35) * SR) as u32;
                let bend = self.rng.range(0.88, 1.12);
                self.spawn_grain(Grain::tone(
                    delay,
                    v,
                    v * bend,
                    0.05 * crowd,
                    0.06,
                    0.16,
                    0.34,
                ));
            }
        } else {
            self.next_event -= 1;
        }

        (low * 0.30 + high * 0.20) * swell * crowd + self.grains_out() * 0.7
    }

    fn industry(&mut self) -> f32 {
        // Muffled, and only while working. Never a beat: the thump breathes at
        // well under 1 Hz and has a 40 ms attack.
        let n = self.rng.bipolar();
        let a = pole_coeff(lerp(180.0, 420.0, self.tone), SR);
        let hiss = self.lp_a.lp(n, a) * lp_norm(a) * 0.16;

        let thump_hz = lerp(0.30, 0.85, self.motion);
        let cycle = advance_phase(&mut self.ph_a, thump_hz, SR);
        // A soft swell rather than a hit: raised cosine up, exponential down.
        let shape = if cycle < 0.22 {
            raised_cosine(cycle / 0.22)
        } else {
            exp_decay(cycle - 0.22, 0.16)
        };
        let low = sine(advance_phase(&mut self.ph_b, lerp(38.0, 58.0, self.depth), SR));
        let thump = low * shape * 0.30 * (0.4 + 0.6 * self.depth);

        if self.next_event == 0 {
            self.schedule(lerp(16.0, 3.5, self.motion));
            let f = self.rng.range(900.0, 2200.0);
            self.spawn_grain(Grain::band(0, f, f * 0.9, 8.0, 0.05, 0.008, 0.05, 0.12));
        } else {
            self.next_event -= 1;
        }

        let dry = hiss + thump + self.grains_out() * 0.5;
        // Everything industrial is heard through a wall.
        let muffle = pole_coeff(lerp(500.0, 1400.0, self.tone), SR);
        self.out_lp.lp(dry, muffle)
    }

    fn rolling(&mut self) -> f32 {
        // `motion` is speed, `depth` is mass, `tone` is how near it is.
        let speed = self.motion;
        let mass = self.depth;
        let n = self.rng.bipolar();

        // Wheel roar: a band that rises and opens with speed.
        let center = lerp(150.0, 620.0, speed) * lerp(1.0, 0.66, mass);
        let f = svf_f(center, SR);
        let damp = svf_damp(0.9);
        let roar = self.svf_a.step(n, f, damp).band * band_norm(f, damp) * speed;

        // Rumble: the weight under it. Freight sits lower and louder.
        let rumble_hz = lerp(64.0, 38.0, mass);
        let rumble = sine(advance_phase(&mut self.ph_a, rumble_hz, SR))
            * (0.10 + 0.22 * mass)
            * speed;
        let groan = if mass > 0.5 {
            sine(advance_phase(&mut self.ph_b, rumble_hz * 1.5, SR)) * 0.07 * speed * (mass - 0.5)
        } else {
            0.0
        };

        // Sleepers: the rhythmic half of a train. Rate follows speed, and the
        // small jitter is what stops it becoming a drum machine.
        if self.next_event == 0 {
            let rate = lerp(1.2, 11.0, speed) * lerp(1.0, 0.72, mass);
            self.next_event = ((SR / rate.max(0.4)) * self.rng.range(0.93, 1.07)) as u32;
            if speed > 0.03 {
                let f = lerp(1250.0, 620.0, mass) * self.rng.range(0.92, 1.08);
                let amp = (0.10 + 0.20 * speed) * (0.7 + 0.5 * mass);
                self.spawn_grain(Grain::band(0, f, f * 0.75, 7.0, amp, 0.003, 0.016, 0.05));
                self.spawn_grain(Grain::tone(
                    0,
                    lerp(110.0, 74.0, mass),
                    lerp(96.0, 64.0, mass),
                    amp * 0.5,
                    0.004,
                    0.03,
                    0.09,
                ));
            }
        } else {
            self.next_event -= 1;
        }

        let dry = roar * 0.34 + rumble + groan + self.grains_out();
        // Distance is dull as well as quiet (brief §7).
        let cut = lerp(600.0, 7000.0, self.tone);
        self.out_lp.lp(dry, pole_coeff(cut, SR))
    }

    /// The score. All of the composition and all of the plucked-string
    /// synthesis live in [`super::score`]; this is only the wiring.
    ///
    /// The parameter mapping, once, so both sides can be read against it:
    ///
    /// | Control | Meaning for the score |
    /// | --- | --- |
    /// | `cue` | which cue is playing; a change restarts the piece |
    /// | `tone` | warmth — how the network is doing |
    /// | `density` | how much of the composition survives the thinning |
    /// | `color` | the dusk variant, `0` day and `1` evening |
    ///
    /// A slow breath goes on top. It is a tenth of a decibel and it is there
    /// for the same reason a real player is never quite steady.
    fn music(&mut self) -> f32 {
        let (cue, warmth, density, dusk) = (self.cue, self.tone, self.density, self.color);
        let voices = match self.score.as_mut() {
            Some(score) => score.step(cue, warmth, density, dusk),
            None => return 0.0,
        };
        if cue == 0 {
            return 0.0;
        }
        let breathe = 0.94 + 0.06 * self.drift(0.037, 0.023);
        voices * breathe
    }
}

impl Iterator for VoiceRender {
    type Item = f32;

    #[inline]
    fn next(&mut self) -> Option<f32> {
        self.read_controls();
        let raw = match self.kind {
            VoiceKind::Wind => self.wind(),
            VoiceKind::Water => self.water(),
            VoiceKind::Forest => self.forest(),
            VoiceKind::Night => self.night(),
            VoiceKind::Town => self.town(),
            VoiceKind::Industry => self.industry(),
            VoiceKind::Murmur => self.murmur(),
            VoiceKind::Rolling => self.rolling(),
            VoiceKind::Music => self.music(),
        };
        Some(soft_clip(if raw.is_finite() { raw } else { 0.0 }))
    }
}

impl RodioSource for VoiceRender {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        1
    }

    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    /// `None` means endless. A bed has no end and therefore no loop point —
    /// which is how "nothing loops audibly" is guaranteed rather than hoped for.
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render `secs` of a voice and report `(rms, peak)`.
    fn measure(kind: VoiceKind, secs: f32, setup: impl Fn(&VoiceParams)) -> (f32, f32) {
        let voice = LiveVoice::new(kind, 12345);
        setup(&voice.params);
        let mut render = voice.decoder();
        let n = (secs * SR) as usize;
        let mut sum_sq = 0.0f64;
        let mut peak = 0.0f32;
        for _ in 0..n {
            let s = render.next().expect("voices are endless");
            assert!(s.is_finite(), "voice {:?} produced {s}", kind);
            sum_sq += (s * s) as f64;
            peak = peak.max(s.abs());
        }
        (((sum_sq / n as f64).sqrt()) as f32, peak)
    }

    fn full(p: &VoiceParams) {
        p.set_tone(0.7);
        p.set_motion(0.7);
        p.set_depth(0.6);
        p.set_density(0.9);
        p.set_color(0.6);
        // The score only sounds while a cue is running; without this the music
        // voice would pass every loudness check by being silent.
        p.set_cue(1);
    }

    #[test]
    fn every_voice_is_finite_and_bounded() {
        for kind in [
            VoiceKind::Wind,
            VoiceKind::Water,
            VoiceKind::Forest,
            VoiceKind::Night,
            VoiceKind::Town,
            VoiceKind::Industry,
            VoiceKind::Murmur,
            VoiceKind::Rolling,
            VoiceKind::Music,
        ] {
            let (_, peak) = measure(kind, 3.0, full);
            assert!(peak <= 1.0, "{kind:?} peaked at {peak}");
        }
    }

    #[test]
    fn no_voice_is_dramatically_louder_than_another() {
        // Brief §7: the dynamic range is narrow. Measured, not asserted in a
        // comment. Music is allowed to sit lower — it is a guest, not the bed.
        let mut levels = Vec::new();
        for kind in VoiceKind::BEDS {
            let (rms, _) = measure(kind, 4.0, full);
            levels.push((kind, rms));
        }
        let loudest = levels.iter().fold(0.0f32, |a, (_, r)| a.max(*r));
        let quietest = levels
            .iter()
            .fold(f32::INFINITY, |a, (_, r)| a.min(*r));
        assert!(quietest > 0.004, "a bed is effectively silent: {levels:?}");
        assert!(
            loudest / quietest < 12.0,
            "beds span {:.1}x - too wide: {levels:?}",
            loudest / quietest
        );
    }

    #[test]
    fn a_dead_town_is_quieter_than_a_thriving_one() {
        // Acceptance bar 1: a player can tell with their eyes closed whether the
        // town near the camera is thriving.
        let (dead, _) = measure(VoiceKind::Town, 6.0, |p| {
            p.set_tone(0.3);
            p.set_density(0.02);
        });
        let (alive, _) = measure(VoiceKind::Town, 6.0, |p| {
            p.set_tone(0.8);
            p.set_density(1.0);
        });
        assert!(
            alive > dead * 2.0,
            "thriving {alive} should clearly outweigh declining {dead}"
        );
    }

    #[test]
    fn an_empty_platform_is_silent_and_a_full_one_is_not() {
        // Brief 10 §3.3 — the murmur scales with how many are waiting, and it
        // is texture rather than an alarm: audible, never prominent.
        let (empty, _) = measure(VoiceKind::Murmur, 4.0, |p| {
            p.set_tone(0.6);
            p.set_density(0.0);
        });
        let (crowd, _) = measure(VoiceKind::Murmur, 4.0, |p| {
            p.set_tone(0.6);
            p.set_density(1.0);
        });
        assert_eq!(empty, 0.0, "an empty platform must make no sound at all");
        assert!(crowd > 0.004, "a full platform is inaudible: {crowd}");

        let (town, _) = measure(VoiceKind::Town, 4.0, full);
        assert!(
            crowd < town * 1.5,
            "the crowd {crowd} should not out-shout the town {town}"
        );
    }

    #[test]
    fn a_stopped_train_is_effectively_silent() {
        let (moving, _) = measure(VoiceKind::Rolling, 3.0, |p| {
            p.set_motion(1.0);
            p.set_tone(1.0);
        });
        let (stopped, _) = measure(VoiceKind::Rolling, 3.0, |p| {
            p.set_motion(0.0);
            p.set_tone(1.0);
        });
        assert!(stopped < moving * 0.2, "stopped {stopped} vs moving {moving}");
    }

    #[test]
    fn distance_is_dull_as_well_as_quiet() {
        // A far train must lose its top end, not just its level.
        let bright = high_energy(VoiceKind::Rolling, 1.0);
        let dull = high_energy(VoiceKind::Rolling, 0.0);
        assert!(dull < bright * 0.5, "far {dull} vs near {bright}");
    }

    /// Crude high-frequency energy: the mean absolute first difference, which
    /// rises with treble content.
    fn high_energy(kind: VoiceKind, tone: f32) -> f32 {
        let voice = LiveVoice::new(kind, 999);
        voice.params.set_tone(tone);
        voice.params.set_motion(0.9);
        voice.params.set_depth(0.4);
        let mut render = voice.decoder();
        let mut prev = 0.0;
        let mut sum = 0.0f64;
        let n = (SR * 2.0) as usize;
        for _ in 0..n {
            let s = render.next().unwrap();
            sum += (s - prev).abs() as f64;
            prev = s;
        }
        (sum / n as f64) as f32
    }

    #[test]
    fn params_round_trip_and_clamp() {
        let p = VoiceParams::new();
        p.set_tone(0.25);
        assert_eq!(p.tone(), 0.25);
        p.set_tone(4.0);
        assert_eq!(p.tone(), 1.0);
        p.set_tone(-3.0);
        assert_eq!(p.tone(), 0.0);
        p.set_density(f32::NAN);
        assert_eq!(p.density(), 0.0);
    }

    #[test]
    fn a_voice_never_ends() {
        let voice = LiveVoice::new(VoiceKind::Wind, 1);
        let mut render = voice.decoder();
        assert!(render.total_duration().is_none(), "a bed must be endless");
        for _ in 0..50_000 {
            assert!(render.next().is_some());
        }
    }

    #[test]
    fn the_score_is_silent_until_a_cue_asks_for_it() {
        // The whole music voice is gated on the cue number, which is what buys
        // the three-to-eight minutes of silence between cues for free: with no
        // cue, no note is scheduled and no string is ringing, so the generator
        // costs a comparison per sample.
        let voice = LiveVoice::new(VoiceKind::Music, 4);
        voice.params.set_density(1.0);
        voice.params.set_tone(0.9);
        let mut render = voice.decoder();
        for _ in 0..(SR as usize * 4) {
            assert_eq!(render.next().unwrap(), 0.0, "the score played without a cue");
        }
        voice.params.set_cue(1);
        let mut peak = 0.0f32;
        for _ in 0..(SR as usize * 20) {
            peak = peak.max(render.next().unwrap().abs());
        }
        assert!(peak > 0.05, "the cue produced nothing: {peak}");
    }

    #[test]
    fn the_score_is_plucked_but_never_percussive() {
        // A pluck is allowed an attack — that is what makes it a string rather
        // than a pad — but the *envelope* must never arrive at once. Measured
        // over 10 ms windows, which is what the ear integrates a transient
        // over; the raw sample slope of a 440 Hz partial says nothing.
        let voice = LiveVoice::new(VoiceKind::Music, 4);
        full(&voice.params);
        let mut render = voice.decoder();
        let buf: Vec<f32> = (0..(SR as usize * 30)).map(|_| render.next().unwrap()).collect();
        let peak = buf.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(peak > 0.05 && peak < 0.8, "the score peaked at {peak}");
        let window = (0.010 * SR) as usize;
        let mut worst = 0.0f32;
        let mut previous = 0.0f32;
        for chunk in buf.chunks(window) {
            let rms = (chunk.iter().map(|s| (s * s) as f64).sum::<f64>() / chunk.len() as f64)
                .sqrt() as f32;
            worst = worst.max((rms - previous).abs());
            previous = rms;
        }
        assert!(
            worst < peak * 0.42,
            "the score's envelope jumped by {worst} against a peak of {peak}"
        );
    }
}
