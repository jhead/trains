//! The day cycle and its single full-screen tint pass (brief 01 §3.4).
//!
//! One quad, one colour, one alpha. The cycle is driven by `Time<Virtual>`
//! gated on [`SimClock`], which is what makes it obey pause and the speed
//! multiplier: [`crate::sim_bridge`] deliberately keeps virtual time running
//! while paused so commands still drain, so pause has to be read from the sim
//! clock rather than inferred from the delta.
//!
//! ## Phases
//!
//! | Phase | Tint | Character |
//! | --- | --- | --- |
//! | Dawn | `#c08a5a` @ 18% | long, warm, low contrast |
//! | Day | none | flat and neutral |
//! | Dusk | `#b06a4e` @ 22% | warm and saturated |
//! | Night | `#1b2340` @ 35% | cool, quiet, windows on |
//!
//! The brief specifies the night pass as a *multiply*. A flat sprite composites
//! with alpha, which agrees with a multiply exactly at the bright end (where
//! legibility lives) and lifts the deep shadows slightly instead of crushing
//! them. That is the safe direction to be wrong in for a palette whose darkest
//! step is `#12111a`, and it needs no custom material.
//!
//! Whatever the numbers say, [`tint_at`] will not return a pass that takes the
//! world below [`MIN_LEGIBILITY`]. Night is legible, not black.

use bevy::prelude::*;
use rail_sim::SimClock;

use super::DAY_TINT_Z;
use crate::map::MapCamera;

/// One full day at 1× sim speed, in seconds (brief §3.4: twelve minutes).
pub const DAY_CYCLE_SECS: f32 = 720.0;

/// The floor on how much of a fully lit world colour a tint may leave standing.
///
/// Measured on sRGB channel values, which is the conservative reading: the
/// render pipeline blends in linear space and lands brighter still.
pub const MIN_LEGIBILITY: f32 = 0.65;

/// Cycle position where each phase begins.
///
/// `0.0` is first light rather than midnight, so the wrap seam falls in the
/// middle of the long flat night instead of inside a transition. Day gets the
/// largest share — the town has to be readable most of the time.
const DAY_START: f32 = 0.14;
const DUSK_START: f32 = 0.54;
const NIGHT_START: f32 = 0.66;

/// The window layer's fade, as a share of the cycle (brief §3.4: about forty
/// seconds). Expressed as a fraction so it scales with sim speed exactly the
/// way the rest of the cycle does.
const WINDOW_FADE: f32 = 40.0 / DAY_CYCLE_SECS;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::srgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
}

/// Dawn tint — `#c08a5a`. Local to this module; a candidate for `palette.rs`
/// if time of day ever needs to be read outside atmosphere.
const DAWN_TINT: Color = rgb(0xc0, 0x8a, 0x5a);
/// Dusk tint — `#b06a4e`.
const DUSK_TINT: Color = rgb(0xb0, 0x6a, 0x4e);
/// Night tint — `#1b2340`.
const NIGHT_TINT: Color = rgb(0x1b, 0x23, 0x40);

/// Keyframes for the tint pass: `(cycle position, colour, alpha)`.
///
/// Day is authored as the neighbouring hue at zero alpha rather than as a
/// separate "none" colour, so each fade runs on a single hue and never drifts
/// through a muddy midpoint.
const TINT_STOPS: [(f32, Color, f32); 7] = [
    (0.00, NIGHT_TINT, 0.35),
    (0.05, DAWN_TINT, 0.18),
    (DAY_START, DAWN_TINT, 0.00),
    (DUSK_START, DUSK_TINT, 0.00),
    (0.60, DUSK_TINT, 0.22),
    (0.72, NIGHT_TINT, 0.35),
    (1.00, NIGHT_TINT, 0.35),
];

/// Below this the tint pass is switched off entirely rather than drawn clear.
const MIN_VISIBLE_ALPHA: f32 = 0.002;

/// Extra world units of tint quad beyond the visible area on every side.
///
/// The quad chases the camera in `Update`, and camera pan may land after it in
/// the same frame. A flat quad costs nothing to oversize, and this is cheaper
/// than coupling two plugins' system order together.
const TINT_MARGIN: f32 = 64.0;

/// Which quarter of the day the world is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DayPhase {
    Dawn,
    #[default]
    Day,
    Dusk,
    Night,
}

impl DayPhase {
    #[allow(dead_code)] // For the status strip / Phase E frame work.
    pub fn label(self) -> &'static str {
        match self {
            Self::Dawn => "Dawn",
            Self::Day => "Day",
            Self::Dusk => "Dusk",
            Self::Night => "Night",
        }
    }
}

/// Public read model for the day cycle.
///
/// Other presentation systems should read this rather than tracking their own
/// clock — the lit-window layer and any future weather or lamp systems all key
/// off the same fraction so they can never disagree about what time it is.
#[derive(Resource, Debug, Clone, Copy)]
pub struct TimeOfDay {
    /// Position in the cycle, `0.0..1.0`, where `0.0` is first light.
    pub fraction: f32,
    /// Coarse phase for tinting decisions and UI.
    #[allow(dead_code)] // Read by the status strip / any later lamp or weather owner.
    pub phase: DayPhase,
    /// How far the lit-window layer has faded up, `0.0..=1.0`.
    pub window_lit: f32,
}

impl Default for TimeOfDay {
    fn default() -> Self {
        // Open a new game in mid-morning: the first thing a player sees should
        // be the flat neutral read, not a transition.
        Self::at(0.20)
    }
}

impl TimeOfDay {
    /// Build the read model for a cycle position.
    pub fn at(fraction: f32) -> Self {
        let fraction = fraction.rem_euclid(1.0);
        Self {
            fraction,
            phase: phase_at(fraction),
            window_lit: window_lit_at(fraction),
        }
    }

    fn set_fraction(&mut self, fraction: f32) {
        *self = Self::at(fraction);
    }

    /// Current tint colour and alpha for the full-screen pass.
    pub fn tint(&self) -> (Color, f32) {
        tint_at(self.fraction)
    }

    /// Whether the world is in its dark half — windows lit, lamps on.
    #[allow(dead_code)] // For lamps / weather / SFX owners.
    pub fn is_dark(&self) -> bool {
        self.window_lit > 0.0
    }
}

/// Phase for a cycle position.
pub fn phase_at(fraction: f32) -> DayPhase {
    let f = fraction.rem_euclid(1.0);
    if f < DAY_START {
        DayPhase::Dawn
    } else if f < DUSK_START {
        DayPhase::Day
    } else if f < NIGHT_START {
        DayPhase::Dusk
    } else {
        DayPhase::Night
    }
}

/// How far the window layer has faded up at a cycle position.
///
/// Up over [`WINDOW_FADE`] from the start of dusk, held through night, back
/// down over the first of dawn. Windows are fully on by the dusk peak, which
/// is the moment the pass is prettiest and the moment the payoff should land.
pub fn window_lit_at(fraction: f32) -> f32 {
    let f = fraction.rem_euclid(1.0);
    if f < WINDOW_FADE {
        1.0 - f / WINDOW_FADE
    } else if f < DUSK_START {
        0.0
    } else if f < DUSK_START + WINDOW_FADE {
        (f - DUSK_START) / WINDOW_FADE
    } else {
        1.0
    }
}

/// Tint colour and alpha for a cycle position, floored at [`MIN_LEGIBILITY`].
pub fn tint_at(fraction: f32) -> (Color, f32) {
    let f = fraction.rem_euclid(1.0);
    let mut prev = TINT_STOPS[0];
    for &stop in TINT_STOPS.iter().skip(1) {
        if f <= stop.0 {
            let span = (stop.0 - prev.0).max(f32::EPSILON);
            let t = ((f - prev.0) / span).clamp(0.0, 1.0);
            let color = mix_srgb(prev.1, stop.1, t);
            let alpha = prev.2 + (stop.2 - prev.2) * t;
            return (color, clamp_to_legibility(color, alpha));
        }
        prev = stop;
    }
    let last = TINT_STOPS[TINT_STOPS.len() - 1];
    (last.1, clamp_to_legibility(last.1, last.2))
}

/// Share of a fully lit world colour that survives `alpha` of `tint`.
///
/// Alpha compositing gives `out = base·(1−a) + tint·a`; at `base = 1` that is
/// the darkest scaling the pass can apply to anything in the frame.
#[allow(dead_code)] // The floor is enforced by `clamp_to_legibility`; this reads it back.
pub fn legibility_factor(tint: Color, alpha: f32) -> f32 {
    let t = tint.to_srgba();
    let survives = |c: f32| (1.0 - alpha) + alpha * c;
    survives(t.red)
        .min(survives(t.green))
        .min(survives(t.blue))
}

/// Cap `alpha` so the pass can never take the world below [`MIN_LEGIBILITY`].
///
/// This is the brief's "never darker than 65%" as an invariant rather than a
/// note: a future edit that asks for a heavier night gets a lighter one.
fn clamp_to_legibility(tint: Color, alpha: f32) -> f32 {
    let t = tint.to_srgba();
    let darkest = t.red.min(t.green).min(t.blue);
    let headroom = 1.0 - darkest;
    if headroom <= f32::EPSILON {
        return alpha.clamp(0.0, 1.0);
    }
    let max_alpha = (1.0 - MIN_LEGIBILITY) / headroom;
    alpha.clamp(0.0, max_alpha.min(1.0))
}

/// Straight sRGB-space mix — the stops are authored as hex, so they are matched
/// exactly at the keyframes and the fade between them stays in the same key.
fn mix_srgb(a: Color, b: Color, t: f32) -> Color {
    let a = a.to_srgba();
    let b = b.to_srgba();
    Color::srgb(
        a.red + (b.red - a.red) * t,
        a.green + (b.green - a.green) * t,
        a.blue + (b.blue - a.blue) * t,
    )
}

/// Marker for the single full-screen tint quad.
#[derive(Component)]
pub(crate) struct DayTintQuad;

pub(crate) fn spawn_day_tint(mut commands: Commands) {
    commands.spawn((
        DayTintQuad,
        Sprite::from_color(NIGHT_TINT.with_alpha(0.0), Vec2::ONE),
        Transform::from_xyz(0.0, 0.0, DAY_TINT_Z),
        Visibility::Hidden,
    ));
}

/// Advance the cycle on sim time.
pub(crate) fn advance_time_of_day(
    clock: Res<SimClock>,
    time: Res<Time<Virtual>>,
    mut tod: ResMut<TimeOfDay>,
) {
    if !clock.is_running() {
        return;
    }
    // `Time<Virtual>` already carries the speed multiplier, so twelve minutes
    // at 1× is four at 3× without any extra arithmetic here.
    let next = tod.fraction + time.delta_secs() / DAY_CYCLE_SECS;
    tod.set_fraction(next);
}

/// Keep the tint quad over the viewport and coloured for the current phase.
pub(crate) fn sync_day_tint(
    tod: Res<TimeOfDay>,
    camera: Query<(&Transform, &Projection), (With<MapCamera>, Without<DayTintQuad>)>,
    mut quad: Query<(&mut Transform, &mut Sprite, &mut Visibility), With<DayTintQuad>>,
) {
    let Ok((cam, projection)) = camera.single() else {
        return;
    };
    let Projection::Orthographic(ortho) = projection else {
        return;
    };
    let Ok((mut transform, mut sprite, mut visibility)) = quad.single_mut() else {
        return;
    };

    let (color, alpha) = tod.tint();
    if alpha < MIN_VISIBLE_ALPHA {
        if *visibility != Visibility::Hidden {
            *visibility = Visibility::Hidden;
        }
        return;
    }
    if *visibility != Visibility::Visible {
        *visibility = Visibility::Visible;
    }

    sprite.color = color.with_alpha(alpha);
    let size = ortho.area.size() + Vec2::splat(TINT_MARGIN * 2.0);
    sprite.custom_size = Some(size.ceil());
    // Whole world texels, like everything else that moves with the camera.
    transform.translation.x = cam.translation.x.round();
    transform.translation.y = cam.translation.y.round();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channels(c: Color) -> (u8, u8, u8) {
        let s = c.to_srgba();
        (
            (s.red * 255.0).round() as u8,
            (s.green * 255.0).round() as u8,
            (s.blue * 255.0).round() as u8,
        )
    }

    #[test]
    fn cycle_is_twelve_minutes_at_normal_speed() {
        assert_eq!(DAY_CYCLE_SECS, 12.0 * 60.0);
    }

    #[test]
    fn phase_stops_are_ordered_and_cover_the_cycle() {
        assert!(0.0 < DAY_START && DAY_START < DUSK_START && DUSK_START < NIGHT_START);
        assert!(NIGHT_START < 1.0);
        assert_eq!(phase_at(0.01), DayPhase::Dawn);
        assert_eq!(phase_at(0.30), DayPhase::Day);
        assert_eq!(phase_at(0.60), DayPhase::Dusk);
        assert_eq!(phase_at(0.90), DayPhase::Night);
        // Wraps rather than clamps.
        assert_eq!(phase_at(1.90), DayPhase::Night);
        assert_eq!(phase_at(-0.05), DayPhase::Night);
    }

    #[test]
    fn tint_matches_the_brief_at_each_peak() {
        let (dawn, dawn_a) = tint_at(0.05);
        assert_eq!(channels(dawn), (0xc0, 0x8a, 0x5a));
        assert!((dawn_a - 0.18).abs() < 1e-4);

        let (_, day_a) = tint_at(0.30);
        assert!(day_a < MIN_VISIBLE_ALPHA, "day is untinted, got {day_a}");

        let (dusk, dusk_a) = tint_at(0.60);
        assert_eq!(channels(dusk), (0xb0, 0x6a, 0x4e));
        assert!((dusk_a - 0.22).abs() < 1e-4);

        let (night, night_a) = tint_at(0.85);
        assert_eq!(channels(night), (0x1b, 0x23, 0x40));
        assert!((night_a - 0.35).abs() < 1e-4);
    }

    #[test]
    fn no_point_in_the_cycle_goes_below_the_legibility_floor() {
        for step in 0..1440 {
            let f = step as f32 / 1440.0;
            let (color, alpha) = tint_at(f);
            let factor = legibility_factor(color, alpha);
            assert!(
                factor >= MIN_LEGIBILITY - 1e-4,
                "tint at {f} leaves only {factor} of the world"
            );
        }
    }

    #[test]
    fn a_heavier_night_is_clamped_rather_than_honoured() {
        // 60% of the night blue would take the world to ~46%.
        let asked = 0.6;
        let allowed = clamp_to_legibility(NIGHT_TINT, asked);
        assert!(allowed < asked);
        assert!(legibility_factor(NIGHT_TINT, allowed) >= MIN_LEGIBILITY - 1e-4);
        // The authored 35% is under the cap, so it passes through untouched.
        assert!((clamp_to_legibility(NIGHT_TINT, 0.35) - 0.35).abs() < 1e-6);
    }

    #[test]
    fn tint_never_jumps_including_across_the_wrap() {
        // One step below is half a sim second at 1×. The pass is a crossfade;
        // no step may read as a cut, least of all the 1.0 → 0.0 seam.
        let steps = 1440;
        let mut worst_alpha = 0.0f32;
        let mut worst_channel = 0.0f32;
        for i in 0..steps {
            let (c0, a0) = tint_at(i as f32 / steps as f32);
            let (c1, a1) = tint_at((i + 1) as f32 / steps as f32);
            worst_alpha = worst_alpha.max((a1 - a0).abs());
            let (s0, s1) = (c0.to_srgba(), c1.to_srgba());
            worst_channel = worst_channel
                .max((s1.red - s0.red).abs())
                .max((s1.green - s0.green).abs())
                .max((s1.blue - s0.blue).abs());
        }
        assert!(worst_alpha < 0.02, "tint alpha jumps by {worst_alpha}");
        assert!(worst_channel < 0.05, "tint hue jumps by {worst_channel}");
    }

    #[test]
    fn windows_fade_up_over_forty_seconds_at_dusk() {
        assert!(window_lit_at(0.30) == 0.0, "day windows stay dark");
        assert!(window_lit_at(DUSK_START) < 0.02);
        let half = DUSK_START + WINDOW_FADE * 0.5;
        assert!((window_lit_at(half) - 0.5).abs() < 0.02);
        assert!((window_lit_at(DUSK_START + WINDOW_FADE) - 1.0).abs() < 1e-4);
        assert_eq!(window_lit_at(0.85), 1.0, "night holds windows lit");

        // Forty sim seconds at 1x, in cycle terms.
        assert!((WINDOW_FADE * DAY_CYCLE_SECS - 40.0).abs() < 1e-3);
        // The fade completes inside dusk, before the phase turns to night.
        assert!(DUSK_START + WINDOW_FADE < NIGHT_START);
    }

    #[test]
    fn windows_fade_back_down_through_dawn() {
        assert_eq!(window_lit_at(0.0), 1.0);
        assert!(window_lit_at(WINDOW_FADE * 0.5) > 0.4);
        assert_eq!(window_lit_at(WINDOW_FADE), 0.0);
        assert!(WINDOW_FADE < DAY_START, "fade must finish inside dawn");
    }

    #[test]
    fn default_opens_in_daylight() {
        let tod = TimeOfDay::default();
        assert_eq!(tod.phase, DayPhase::Day);
        assert_eq!(tod.window_lit, 0.0);
        assert!(tod.tint().1 < MIN_VISIBLE_ALPHA);
    }

    #[test]
    fn advancing_wraps_and_keeps_the_read_model_in_step() {
        let mut tod = TimeOfDay::at(0.99);
        tod.set_fraction(tod.fraction + 0.02);
        assert!((tod.fraction - 0.01).abs() < 1e-5);
        assert_eq!(tod.phase, phase_at(tod.fraction));
        assert_eq!(tod.window_lit, window_lit_at(tod.fraction));
    }
}
