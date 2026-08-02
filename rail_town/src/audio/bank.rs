//! The one-shot bank — every discrete sound in the game, baked at startup.
//!
//! There are no audio assets and no artist, so this file *is* the sound design.
//! Each entry is a few layers summed onto a [`Canvas`] and normalised; the
//! numbers are the sounds, and they are worth reading as such.
//!
//! Two structural choices carry most of the brief:
//!
//! - **Every positional family is baked twice**, near and far, the far copy
//!   low-passed. Distance in this game is dull as well as quiet (§7), and a
//!   post-hoc filter on a one-shot is not available in `bevy_audio`.
//! - **Every family that repeats has a small set of variants.** The track clack
//!   has six, and the mixer adds playback-rate jitter on top, so a long run is a
//!   rhythmic run rather than the same 40 ms of PCM eleven times (§3.1).
//!
//! Bake cost is about a second of audio in total, once, at startup.

use bevy::asset::{Assets, Handle};
use bevy::prelude::*;

use super::clip::{Canvas, SampleClip};
use super::dsp::{semitones, Rng};

/// Below this mixer brightness a positional sound uses its far (muffled) copy.
const FAR_BRIGHTNESS: f32 = 0.45;

/// Cutoff for the far copy of a positional family.
const FAR_CUTOFF_HZ: f32 = 850.0;

/// A family of interchangeable takes, baked near and far.
#[derive(Debug, Clone)]
pub struct ClipSet {
    near: Vec<Handle<SampleClip>>,
    far: Vec<Handle<SampleClip>>,
}

impl ClipSet {
    /// Pick a take. `index` rotates through the variants; `brightness` is the
    /// mixer's near/far figure for the sound's position.
    pub fn pick(&self, index: usize, brightness: f32) -> Handle<SampleClip> {
        let bank = if brightness < FAR_BRIGHTNESS && !self.far.is_empty() {
            &self.far
        } else {
            &self.near
        };
        bank[index % bank.len()].clone()
    }

    /// Pick the near take — for UI, which has no position.
    pub fn near(&self, index: usize) -> Handle<SampleClip> {
        self.near[index % self.near.len()].clone()
    }

    pub fn variants(&self) -> usize {
        self.near.len()
    }
}

/// Handles for every one-shot in the game.
#[derive(Resource, Debug)]
pub struct SfxBank {
    // -- building (§3.1) --
    pub clack: ClipSet,
    pub bridge: ClipSet,
    pub station: ClipSet,
    pub demolish: ClipSet,
    pub invalid: ClipSet,
    pub tool_switch: ClipSet,
    // -- trains (§3.2) --
    pub whistle: ClipSet,
    pub brake: ClipSet,
    pub depart_transit: ClipSet,
    pub depart_freight: ClipSet,
    pub crossing: ClipSet,
    // -- interface (§5) --
    pub ui_click: ClipSet,
    pub panel_open: ClipSet,
    pub panel_close: ClipSet,
    pub toggle_on: ClipSet,
    pub toggle_off: ClipSet,
    pub money_gain: ClipSet,
    pub money_spend: ClipSet,
    pub alert: ClipSet,
    pub milestone: ClipSet,
}

/// Bake the whole bank. Runs once, in `Startup`.
pub fn bake_bank(mut commands: Commands, mut clips: ResMut<Assets<SampleClip>>) {
    let mut rng = Rng::new(0x5261_696c_546f_776e); // "RailTown"
    let mut ctx = Bake {
        clips: &mut clips,
        rng: &mut rng,
    };

    let bank = SfxBank {
        clack: ctx.positional(6, clack),
        bridge: ctx.positional(3, bridge),
        station: ctx.positional(2, station_chord),
        demolish: ctx.positional(3, demolish),
        invalid: ctx.positional(2, invalid),
        tool_switch: ctx.flat(1, tool_switch),
        whistle: ctx.positional(3, whistle),
        brake: ctx.positional(2, brake),
        depart_transit: ctx.positional(2, depart_transit),
        depart_freight: ctx.positional(2, depart_freight),
        crossing: ctx.positional(2, crossing_ding),
        ui_click: ctx.flat(2, ui_click),
        panel_open: ctx.flat(1, |_, r| panel_sweep(r, true)),
        panel_close: ctx.flat(1, |_, r| panel_sweep(r, false)),
        toggle_on: ctx.flat(1, |_, r| toggle(r, true)),
        toggle_off: ctx.flat(1, |_, r| toggle(r, false)),
        money_gain: ctx.flat(1, money_gain),
        money_spend: ctx.flat(1, money_spend),
        alert: ctx.flat(1, alert_two_note),
        milestone: ctx.flat(1, milestone),
    };

    commands.insert_resource(bank);
}

struct Bake<'a> {
    clips: &'a mut Assets<SampleClip>,
    rng: &'a mut Rng,
}

impl Bake<'_> {
    /// Bake `variants` takes, near and far.
    fn positional(&mut self, variants: usize, make: fn(usize, &mut Rng) -> Canvas) -> ClipSet {
        let mut near = Vec::with_capacity(variants);
        let mut far = Vec::with_capacity(variants);
        for v in 0..variants {
            near.push(self.clips.add(make(v, self.rng).finish()));
            let mut muffled = make(v, self.rng);
            muffled.low_pass(FAR_CUTOFF_HZ);
            far.push(self.clips.add(muffled.finish()));
        }
        ClipSet { near, far }
    }

    /// Bake `variants` takes with no far copy — interface sound has no distance.
    fn flat(&mut self, variants: usize, make: fn(usize, &mut Rng) -> Canvas) -> ClipSet {
        let near = (0..variants)
            .map(|v| self.clips.add(make(v, self.rng).finish()))
            .collect();
        ClipSet {
            near,
            far: Vec::new(),
        }
    }
}

// -- building -------------------------------------------------------------

/// **The signature sound.** Ballast and a sleeper, per tile.
///
/// Four layers, and all four matter: grit on top so it reads as stone, a
/// mid knock so it reads as timber, a low thump so it lands in the chest, and a
/// short body of soft noise so it does not sound synthetic. Six variants a
/// semitone or so apart; the mixer jitters rate and timing on top.
fn clack(variant: usize, rng: &mut Rng) -> Canvas {
    let pitch = semitones(-2.5 + variant as f32);
    let mut c = Canvas::new(0.18);
    // Ballast grit.
    c.noise_band(rng, 0.0, 0.055, 2500.0 * pitch, 1500.0 * pitch, 4.5, 1.0, 0.0025, 0.013);
    // Sleeper knock — the woody part.
    c.noise_band(rng, 0.001, 0.07, 800.0 * pitch, 690.0 * pitch, 9.0, 0.85, 0.003, 0.024);
    // The thump under it.
    c.partial(0.002, 96.0 * pitch, 0.60, 0.005, 0.048, 0.15, 0.90);
    // Body, so the burst is not bare noise.
    c.noise_soft(rng, 0.0, 0.06, 280.0, 0.35, 0.004, 0.030);
    c
}

/// Heavier than track, with the timber creaking as it takes the load.
fn bridge(variant: usize, rng: &mut Rng) -> Canvas {
    let pitch = semitones(-1.5 + variant as f32 * 1.5);
    let mut c = Canvas::new(0.65);
    c.partial(0.0, 70.0 * pitch, 0.75, 0.008, 0.105, 0.45, 0.86);
    c.noise_band(rng, 0.0, 0.22, 540.0 * pitch, 430.0 * pitch, 8.0, 0.55, 0.004, 0.055);
    // The creak: a slow rising groan with a wobble in it.
    c.partial_vibrato(0.04, 225.0 * pitch, 0.30, 0.035, 0.24, 0.55, 1.09, 5.5, 55.0);
    c.noise_soft(rng, 0.0, 0.35, 300.0, 0.32, 0.006, 0.13);
    c
}

/// The one moment of near-musicality in the build set: root, fifth, octave and
/// tenth, rolled slightly, over a soft settle.
fn station_chord(variant: usize, rng: &mut Rng) -> Canvas {
    let root = 196.0 * semitones(variant as f32 * 2.0);
    let mut c = Canvas::new(1.8);
    for (i, &degree) in [0.0, 7.0, 12.0, 16.0].iter().enumerate() {
        let f = root * semitones(degree);
        let amp = 0.55 / (1.0 + 0.35 * i as f32);
        c.partial(i as f32 * 0.045, f, amp, 0.030, 0.55, 1.6, 1.0);
        c.partial(i as f32 * 0.045, f * 2.0, amp * 0.16, 0.040, 0.30, 1.0, 1.0);
    }
    c.partial(0.0, root * 0.5, 0.28, 0.030, 0.40, 1.0, 1.0);
    c.noise_soft(rng, 0.0, 0.22, 220.0, 0.26, 0.006, 0.065);
    c
}

/// A dull clank and settling debris.
fn demolish(variant: usize, rng: &mut Rng) -> Canvas {
    let pitch = semitones(-1.0 + variant as f32 * 1.0);
    let mut c = Canvas::new(0.8);
    c.partial(0.0, 152.0 * pitch, 0.60, 0.006, 0.090, 0.40, 0.87);
    // Inharmonic partner — this is what makes it a clank and not a note.
    c.partial(0.0, 231.0 * pitch, 0.30, 0.006, 0.062, 0.28, 0.90);
    c.noise_band(rng, 0.0, 0.2, 1250.0, 720.0, 3.0, 0.42, 0.004, 0.042);
    // Settling debris: a scatter of small stones over the next half second.
    for _ in 0..7 {
        let t = rng.range(0.05, 0.45);
        let f = rng.range(900.0, 2600.0);
        let amp = rng.range(0.05, 0.14);
        c.noise_band(rng, t, 0.06, f, f * 0.8, 6.0, amp, 0.003, 0.013);
    }
    c.noise_soft(rng, 0.0, 0.45, 200.0, 0.30, 0.006, 0.135);
    c
}

/// **The invalid thud.** Quiet, unmistakable, and hard-limited in bandwidth.
///
/// The player will hear this hundreds of times an hour. Everything above about
/// 400 Hz is filtered off after the fact, so no amount of repetition can turn it
/// into a buzz, and the 12 ms attack means it can never crack.
fn invalid(variant: usize, rng: &mut Rng) -> Canvas {
    let pitch = semitones(-(variant as f32));
    let mut c = Canvas::new(0.42);
    c.partial(0.0, 74.0 * pitch, 0.85, 0.012, 0.085, 0.36, 0.93);
    c.partial(0.0, 139.0 * pitch, 0.22, 0.014, 0.050, 0.22, 0.95);
    c.noise_soft(rng, 0.0, 0.22, 150.0, 0.18, 0.012, 0.065);
    c.low_pass(420.0);
    c
}

/// A minimal click. Barely a sound; it exists so the tool change is felt.
fn tool_switch(_variant: usize, rng: &mut Rng) -> Canvas {
    let mut c = Canvas::new(0.07);
    c.noise_band(rng, 0.0, 0.03, 1500.0, 1100.0, 5.0, 0.5, 0.002, 0.007);
    c.partial(0.0, 430.0, 0.38, 0.003, 0.013, 0.055, 0.90);
    c.low_pass(2600.0);
    c
}

// -- trains ---------------------------------------------------------------

/// A chime whistle across a valley. The loudest thing in the game, and it is
/// still not very loud: a 120 ms attack, a chord rather than a shriek, and a
/// gentle fall on the way out.
fn whistle(variant: usize, rng: &mut Rng) -> Canvas {
    let base = 392.0 * semitones(-2.0 + variant as f32 * 2.0);
    let mut c = Canvas::new(1.7);
    for &(ratio, amp) in &[(1.0, 0.55), (1.19, 0.42), (1.5, 0.34), (2.0, 0.13)] {
        c.partial_vibrato(0.02, base * ratio, amp, 0.12, 1.05, 1.45, 0.965, 5.2, 11.0);
    }
    // Breath — a whistle is air before it is a pitch.
    c.noise_band(rng, 0.0, 1.45, 2400.0, 1900.0, 1.3, 0.11, 0.15, 0.85);
    c
}

/// Brakes on arrival: a band descending as the train slows, then air.
fn brake(variant: usize, rng: &mut Rng) -> Canvas {
    let scale = 1.0 + variant as f32 * 0.12;
    let mut c = Canvas::new(1.6);
    c.noise_band(rng, 0.0, 1.2, 2400.0 * scale, 700.0, 7.0, 0.70, 0.12, 0.55);
    c.noise_band(rng, 0.0, 1.2, 3500.0 * scale, 1200.0, 9.0, 0.32, 0.15, 0.45);
    c.noise_soft(rng, 0.95, 0.4, 1200.0, 0.30, 0.02, 0.12);
    c.low_pass(5000.0);
    c
}

/// A transit unit pulling away: a rising whine, no drama.
fn depart_transit(variant: usize, rng: &mut Rng) -> Canvas {
    let scale = 1.0 + variant as f32 * 0.08;
    let mut c = Canvas::new(1.1);
    c.noise_band(rng, 0.0, 0.95, 520.0 * scale, 1450.0 * scale, 6.0, 0.55, 0.15, 0.50);
    c.partial(0.05, 180.0 * scale, 0.35, 0.12, 0.45, 0.85, 1.55);
    c.low_pass(3200.0);
    c
}

/// A goods train leaning into it: three soft chuffs, audibly heavy.
fn depart_freight(variant: usize, rng: &mut Rng) -> Canvas {
    let scale = 1.0 + variant as f32 * 0.07;
    let mut c = Canvas::new(1.3);
    for i in 0..3 {
        let t = i as f32 * 0.27;
        c.noise_soft(rng, t, 0.24, 720.0 * scale, 0.55 - 0.09 * i as f32, 0.022, 0.095);
        c.partial(t, 62.0 * scale, 0.32, 0.010, 0.075, 0.22, 0.88);
    }
    c.low_pass(1500.0);
    c
}

/// Level-crossing bell. Two takes so the alternation is a bell, not a beep.
fn crossing_ding(variant: usize, _rng: &mut Rng) -> Canvas {
    let pitch = semitones(variant as f32 * 1.5);
    let mut c = Canvas::new(0.85);
    c.partial(0.0, 660.0 * pitch, 0.55, 0.008, 0.22, 0.72, 0.998);
    c.partial(0.0, 990.0 * pitch, 0.22, 0.010, 0.155, 0.55, 0.998);
    // A deliberately inharmonic upper partial — bells are not harmonic series.
    c.partial(0.0, 1670.0 * pitch, 0.13, 0.008, 0.095, 0.40, 0.997);
    c.low_pass(4000.0);
    c
}

// -- interface ------------------------------------------------------------

/// A soft low tick. Felt more than heard.
fn ui_click(variant: usize, rng: &mut Rng) -> Canvas {
    let pitch = semitones(variant as f32 * -2.0);
    let mut c = Canvas::new(0.09);
    c.partial(0.0, 520.0 * pitch, 0.5, 0.004, 0.019, 0.075, 0.93);
    c.noise_soft(rng, 0.0, 0.035, 900.0, 0.2, 0.003, 0.009);
    c.low_pass(1800.0);
    c
}

/// A brief airy sweep, up to open and down to close.
fn panel_sweep(rng: &mut Rng, open: bool) -> Canvas {
    let (from, to) = if open { (380.0, 1500.0) } else { (1500.0, 380.0) };
    let mut c = Canvas::new(0.32);
    c.noise_band(rng, 0.0, 0.27, from, to, 1.6, 1.0, 0.05, 0.145);
    c.low_pass(3000.0);
    c
}

/// Two tones, up for on.
fn toggle(_rng: &mut Rng, on: bool) -> Canvas {
    let (a, b) = if on { (392.0, 523.25) } else { (523.25, 392.0) };
    let mut c = Canvas::new(0.4);
    c.partial(0.0, a, 0.5, 0.006, 0.09, 0.22, 1.0);
    c.partial(0.075, b, 0.46, 0.006, 0.12, 0.28, 1.0);
    c.low_pass(2600.0);
    c
}

/// A warm low chime. Never plays more than once per aggregation window.
fn money_gain(_variant: usize, _rng: &mut Rng) -> Canvas {
    let mut c = Canvas::new(1.5);
    c.partial(0.0, 220.0, 0.55, 0.025, 0.45, 1.35, 1.0);
    c.partial(0.02, 330.0, 0.30, 0.030, 0.35, 1.10, 1.0);
    c.partial(0.05, 440.0, 0.15, 0.040, 0.28, 0.95, 1.0);
    c.partial(0.0, 110.0, 0.24, 0.030, 0.32, 0.90, 1.0);
    c.low_pass(2400.0);
    c
}

/// The softer counterpart: the same shape, a step down, less of it.
fn money_spend(_variant: usize, _rng: &mut Rng) -> Canvas {
    let mut c = Canvas::new(1.1);
    c.partial(0.0, 196.0, 0.50, 0.030, 0.30, 0.85, 1.0);
    c.partial(0.06, 147.0, 0.36, 0.035, 0.34, 0.95, 1.0);
    c.partial(0.0, 98.0, 0.22, 0.030, 0.26, 0.75, 1.0);
    c.low_pass(1600.0);
    c
}

/// Gentle, two-note, never urgent — a descending minor third.
fn alert_two_note(_variant: usize, _rng: &mut Rng) -> Canvas {
    let mut c = Canvas::new(1.1);
    c.partial(0.0, 392.0, 0.50, 0.020, 0.20, 0.55, 1.0);
    c.partial(0.0, 784.0, 0.10, 0.030, 0.12, 0.35, 1.0);
    c.partial(0.2, 329.63, 0.46, 0.020, 0.26, 0.75, 1.0);
    c.partial(0.2, 659.25, 0.09, 0.030, 0.14, 0.40, 1.0);
    c.low_pass(2600.0);
    c
}

/// The one genuinely warm moment. Rare enough to stay special.
fn milestone(_variant: usize, _rng: &mut Rng) -> Canvas {
    let root = 174.61;
    let mut c = Canvas::new(2.8);
    for (i, &degree) in [0.0, 7.0, 12.0, 16.0, 19.0].iter().enumerate() {
        let f = root * semitones(degree);
        c.partial(i as f32 * 0.13, f, 0.5 / (1.0 + 0.3 * i as f32), 0.040, 0.90, 2.5, 1.0);
    }
    c.partial(0.0, root * 0.5, 0.30, 0.045, 0.70, 2.0, 1.0);
    c.low_pass(3200.0);
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::clip::{worst_envelope_step, SR};

    fn peak(c: &SampleClip) -> f32 {
        c.data.iter().fold(0.0f32, |a, s| a.max(s.abs()))
    }

    /// Rough treble content — the mean absolute first difference.
    fn brightness(c: &SampleClip) -> f32 {
        let sum: f64 = c.data.windows(2).map(|w| (w[1] - w[0]).abs() as f64).sum();
        (sum / c.data.len().max(1) as f64) as f32
    }

    fn bake_all() -> Vec<(&'static str, SampleClip)> {
        let mut rng = Rng::new(1);
        let mut out: Vec<(&'static str, SampleClip)> = Vec::new();
        let mut push = |name: &'static str, c: Canvas| out.push((name, c.finish()));
        for v in 0..6 {
            push("clack", clack(v, &mut rng));
        }
        for v in 0..3 {
            push("bridge", bridge(v, &mut rng));
            push("demolish", demolish(v, &mut rng));
            push("whistle", whistle(v, &mut rng));
        }
        for v in 0..2 {
            push("station", station_chord(v, &mut rng));
            push("invalid", invalid(v, &mut rng));
            push("brake", brake(v, &mut rng));
            push("depart_transit", depart_transit(v, &mut rng));
            push("depart_freight", depart_freight(v, &mut rng));
            push("crossing", crossing_ding(v, &mut rng));
            push("ui_click", ui_click(v, &mut rng));
        }
        push("tool_switch", tool_switch(0, &mut rng));
        push("panel_open", panel_sweep(&mut rng, true));
        push("panel_close", panel_sweep(&mut rng, false));
        push("toggle_on", toggle(&mut rng, true));
        push("toggle_off", toggle(&mut rng, false));
        push("money_gain", money_gain(0, &mut rng));
        push("money_spend", money_spend(0, &mut rng));
        push("alert", alert_two_note(0, &mut rng));
        push("milestone", milestone(0, &mut rng));
        out
    }

    #[test]
    fn nothing_in_the_bank_can_startle() {
        // Brief §1, as a test over the whole bank: no clip clips, none starts or
        // ends on a step, and none contains a transient sharp enough to crack.
        for (name, clip) in bake_all() {
            assert!(clip.data.iter().all(|s| s.is_finite()), "{name} has a NaN");
            assert!(peak(&clip) <= 1.0, "{name} peaks at {}", peak(&clip));
            assert_eq!(clip.data[0], 0.0, "{name} starts on a step");
            assert_eq!(*clip.data.last().unwrap(), 0.0, "{name} ends on a step");
            let step = worst_envelope_step(&clip);
            assert!(step < 0.35, "{name} has a {step} envelope discontinuity");
            let ms = (0.001 * SR) as usize;
            let early = clip.data[..ms].iter().fold(0.0f32, |a, s| a.max(s.abs()));
            assert!(
                early < peak(&clip) * 0.3,
                "{name} is at {early} after one millisecond"
            );
        }
    }

    #[test]
    fn the_invalid_thud_has_no_bite_in_it() {
        // It will be heard often and must never become irritating (§3.1). It is
        // the dullest thing in the build set by a wide margin.
        let mut rng = Rng::new(2);
        let thud = invalid(0, &mut rng).finish();
        let clack_clip = clack(0, &mut rng).finish();
        assert!(
            brightness(&thud) < brightness(&clack_clip) * 0.35,
            "thud brightness {} vs clack {}",
            brightness(&thud),
            brightness(&clack_clip)
        );
        assert!(thud.secs() < 0.6, "and it is short: {}", thud.secs());
    }

    #[test]
    fn the_clack_variants_are_all_different() {
        let mut rng = Rng::new(3);
        let takes: Vec<SampleClip> = (0..6).map(|v| clack(v, &mut rng).finish()).collect();
        for i in 0..takes.len() {
            for j in (i + 1)..takes.len() {
                let a = &takes[i].data;
                let b = &takes[j].data;
                let n = a.len().min(b.len());
                let diff: f32 = (0..n).map(|k| (a[k] - b[k]).abs()).sum::<f32>() / n as f32;
                assert!(diff > 0.01, "clack takes {i} and {j} are near-identical");
            }
        }
        // And they are short enough to run rhythmically.
        assert!(takes[0].secs() < 0.25, "clack is {}s", takes[0].secs());
    }

    #[test]
    fn the_far_copy_of_a_family_is_duller() {
        let mut rng = Rng::new(4);
        let near = clack(0, &mut rng).finish();
        let mut far_canvas = clack(0, &mut Rng::new(4));
        far_canvas.low_pass(FAR_CUTOFF_HZ);
        let far = far_canvas.finish();
        assert!(
            brightness(&far) < brightness(&near) * 0.6,
            "far {} vs near {}",
            brightness(&far),
            brightness(&near)
        );
    }

    #[test]
    fn the_whistle_arrives_slowly() {
        // A sudden loud event is the one thing the brief forbids outright.
        let mut rng = Rng::new(5);
        let clip = whistle(0, &mut rng).finish();
        let ten_ms = (0.010 * SR) as usize;
        let early = clip.data[..ten_ms].iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(early < 0.15, "whistle is at {early} after 10 ms");
        assert!(clip.secs() > 1.0, "and it is long: {}s", clip.secs());
    }

    #[test]
    fn the_station_chord_is_the_musical_one() {
        // It should ring, unlike everything else in the build set.
        let mut rng = Rng::new(6);
        let chord = station_chord(0, &mut rng).finish();
        let clack_clip = clack(0, &mut rng).finish();
        assert!(chord.secs() > clack_clip.secs() * 5.0);
    }
}
