//! Buses, listener, distance, zoom, ducking and voice limits (brief §7).
//!
//! Every sound in the game gets its final gain from here, which is what keeps
//! the dynamic range narrow: the loudest thing available is a nearby whistle at
//! [`gain::WHISTLE`] and the quietest is a UI tick at [`gain::UI_CLICK`], about
//! ten decibels apart, and nothing can escape the calculation.
//!
//! ## Position
//!
//! `bevy_audio`'s spatial support is stereo panning with a `1/d²` law, which at
//! 32 world units per tile would go silent within one tile. So the panning is
//! kept (it is gentle and it is the thing that makes the map a space) and the
//! distance law is replaced: [`AudioMix::falloff`] is a smooth rolloff that
//! reaches exactly zero at the audible radius, and [`AudioMix::brightness`]
//! decides how much top end survives the trip. Far sounds are dull as well as
//! quiet.
//!
//! ## Zoom
//!
//! At 1× the player is looking at a landscape and the audible radius is wide;
//! at 3× they are down among the trains and it is tight. Map View pushes the
//! mix all the way out — the schematic read is not a place you stand in.

use bevy::audio::{PlaybackSettings, SpatialListener, SpatialScale, Volume};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::TILE_SIZE;

use crate::atmosphere::AmbientClock;
use crate::map::{MapCamera, MapViewState};

use super::clip::SampleClip;

/// Spatial scale for the emitter positions.
///
/// Chosen so that `1/d²` is clamped to 1 across the whole map: rodio's distance
/// law is neutralised and only its panning survives. Distance is ours.
const SPATIAL_SCALE: f32 = 1.0 / 8192.0;

/// Ear separation, in world units. Four tiles: panning saturates for anything
/// more than a few tiles off centre, and stays gentle throughout.
const EAR_GAP: f32 = TILE_SIZE * 4.0;

/// Audible radius at 1× and at 3×, in tiles.
const RADIUS_TILES_WIDE: f32 = 30.0;
const RADIUS_TILES_CLOSE: f32 = 13.0;

/// Below this final linear gain a sound is not worth a voice.
const MIN_AUDIBLE: f32 = 0.004;

/// How long the focus mute takes. Not instant — a hard cut is a transient too.
const FOCUS_FADE_SECS: f32 = 0.22;

/// Duck depth and recovery. "Nothing ducks hard" (§7).
const MUSIC_DUCK_FLOOR: f32 = 0.58;
const AMBIENCE_DUCK_FLOOR: f32 = 0.80;
const DUCK_RELEASE_SECS: f32 = 1.6;

/// Reference gains, all linear, all in one place.
///
/// The whole point of the table is that the ratios are visible: the loudest
/// entry is under four times the quietest, which is the brief's narrow dynamic
/// range expressed as numbers rather than as an intention.
pub mod gain {
    pub const CLACK: f32 = 0.26;
    pub const BRIDGE: f32 = 0.30;
    pub const STATION: f32 = 0.30;
    pub const DEMOLISH: f32 = 0.24;
    pub const INVALID: f32 = 0.17;
    pub const TOOL_SWITCH: f32 = 0.11;
    /// The quietest positional family: a building site heard across a field.
    pub const HAMMER: f32 = 0.10;

    pub const WHISTLE: f32 = 0.34;
    pub const BRAKE: f32 = 0.20;
    pub const DEPARTURE: f32 = 0.20;
    pub const CROSSING: f32 = 0.13;
    /// Per moving train, before distance.
    ///
    /// Lowered from `0.17` after a playtest heard it as "white noise... always
    /// too loud": with the camera near the line, one train's roar out-levelled
    /// the entire ambience bed and read as broken ambience rather than as a
    /// train. One train now sits just *under* the landscape it crosses
    /// ([`AMBIENCE_TOTAL`]) — present, directional, and gone when it stops
    /// (the speed floor lives in `trains.rs`, priced by the same session).
    pub const ROLLING: f32 = 0.11;

    pub const UI_CLICK: f32 = 0.10;
    pub const PANEL: f32 = 0.11;
    pub const TOGGLE: f32 = 0.12;
    pub const MONEY_GAIN: f32 = 0.17;
    pub const MONEY_SPEND: f32 = 0.12;
    pub const ALERT: f32 = 0.16;
    pub const MILESTONE: f32 = 0.26;

    /// Total for the whole ambience bed, shared out between the layers.
    ///
    /// Lowered from `0.34` after the playtest. The bed is the only thing in the
    /// game that is *never* absent, so it is the only thing whose level is
    /// judged over two hours rather than over two seconds — and the wind, which
    /// is most of it, has come down further still inside its own generator.
    pub const AMBIENCE_TOTAL: f32 = 0.28;
    /// Peak for a music cue.
    ///
    /// The score normalises its own output to roughly the same peak as the
    /// ambience generators (see [`super::score`]), so this number means the
    /// same thing as every other one in the table: still the guest, still under
    /// the whistle, and a little above the bed it sits on because a plucked
    /// string is transient and a wind bed is not.
    pub const MUSIC: f32 = 0.34;
}

/// Voice-limit categories. Nearest instances win within each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundCategory {
    Build,
    Train,
    World,
    Ui,
}

impl SoundCategory {
    fn index(self) -> usize {
        match self {
            Self::Build => 0,
            Self::Train => 1,
            Self::World => 2,
            Self::Ui => 3,
        }
    }

    /// Concurrent one-shots allowed. A large network never turns to mush (§7).
    fn limit(self) -> u16 {
        match self {
            Self::Build => 10,
            Self::Train => 8,
            Self::World => 6,
            Self::Ui => 4,
        }
    }
}

const CATEGORIES: usize = 4;

/// A monotonic clock shared with the visual ambience.
///
/// [`AmbientClock`] is the atmosphere module's wall-clock seconds; it freezes
/// with the sim and wraps every 24 minutes. Audio needs a monotonic timeline for
/// the music schedule, so this accumulates its deltas and unwraps them. Sharing
/// it is what makes a paused world *held* in both channels rather than held in
/// one and drifting in the other.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct AudioClock {
    /// Seconds since startup, frozen while the sim is paused.
    pub elapsed: f64,
    /// This frame's contribution to [`Self::elapsed`]; zero while paused.
    pub delta: f32,
    /// Real frame time. Smoothing uses this so a pause never freezes a fade.
    pub real_delta: f32,
    pub running: bool,
    last_ambient: f32,
    seeded: bool,
}

/// Wrap point of [`AmbientClock`], mirrored here so the unwrap is exact.
const AMBIENT_WRAP: f32 = 1440.0;

pub fn advance_audio_clock(
    ambient: Res<AmbientClock>,
    real: Res<Time<Real>>,
    mut clock: ResMut<AudioClock>,
) {
    clock.real_delta = real.delta_secs();
    if !clock.seeded {
        clock.last_ambient = ambient.secs;
        clock.seeded = true;
    }
    let mut delta = ambient.secs - clock.last_ambient;
    if delta < 0.0 {
        delta += AMBIENT_WRAP;
    }
    // A wrap and a genuine jump look alike; cap at a plausible frame.
    if !(0.0..=1.0).contains(&delta) {
        delta = 0.0;
    }
    clock.last_ambient = ambient.secs;
    clock.delta = delta;
    clock.running = delta > 0.0;
    clock.elapsed += delta as f64;
}

/// Bus gains, listener state and the ducking envelopes.
#[derive(Resource, Debug, Clone)]
pub struct AudioMix {
    pub master: f32,
    pub music_bus: f32,
    pub ambience_bus: f32,
    pub effects_bus: f32,
    pub ui_bus: f32,
    /// Player's "Mute unfocused" choice, mirrored from `Settings`.
    pub mute_unfocused: bool,
    /// Undoes `bevy_audio`'s `GlobalVolume`, which the shell drives from the
    /// master slider and which is multiplied into a one-shot when its sink is
    /// created. Without it a one-shot would be attenuated by the master twice
    /// while the live voices were attenuated once, and lowering the master
    /// would silence the railway long before it silenced the wind.
    pub global_trim: f32,
    /// Smoothed window focus, `0.0` when the game is in the background.
    pub focus: f32,
    /// Smoothed music duck, `1.0` when nothing is ducking it.
    pub music_duck: f32,
    pub ambience_duck: f32,
    /// Where the player is listening from, in world units.
    pub listener: Vec2,
    /// Effective zoom: 1–3 in play, lower in Map View.
    pub zoom: f32,
    /// `0.0` at 1× (a landscape), `1.0` at 3× (among the trains).
    pub detail: f32,
    /// Audible radius in world units.
    pub radius: f32,
}

impl Default for AudioMix {
    fn default() -> Self {
        Self {
            master: 0.8,
            music_bus: 0.7,
            ambience_bus: 0.7,
            effects_bus: 0.8,
            ui_bus: 0.6,
            mute_unfocused: true,
            global_trim: 1.0,
            focus: 1.0,
            music_duck: 1.0,
            ambience_duck: 1.0,
            listener: Vec2::ZERO,
            zoom: 2.0,
            detail: 0.5,
            radius: RADIUS_TILES_WIDE * TILE_SIZE,
        }
    }
}

impl AudioMix {
    pub fn effects(&self) -> f32 {
        self.master * self.effects_bus * self.focus
    }

    pub fn ui(&self) -> f32 {
        self.master * self.ui_bus * self.focus
    }

    pub fn music(&self) -> f32 {
        self.master * self.music_bus * self.focus * self.music_duck
    }

    pub fn ambience(&self) -> f32 {
        self.master * self.ambience_bus * self.focus * self.ambience_duck
    }

    /// Distance attenuation: `1.0` at the listener, exactly `0.0` at the edge of
    /// the audible radius so out-of-range sounds cost nothing at all.
    pub fn falloff(&self, at: Vec2) -> f32 {
        let d = (at - self.listener).length() / self.radius.max(1.0);
        let t = (1.0 - d).clamp(0.0, 1.0);
        t * t
    }

    /// How much top end survives the trip. Feeds the near/far clip choice and
    /// the `tone` parameter of the live voices.
    pub fn brightness(&self, at: Vec2) -> f32 {
        let d = (at - self.listener).length() / self.radius.max(1.0);
        (1.0 - d * 1.2).clamp(0.0, 1.0)
    }

    /// Zoom's contribution to effect level: wide and distant at 1×, present at 3×.
    pub fn effect_perspective(&self) -> f32 {
        0.78 + 0.34 * self.detail
    }

    /// The complement for the bed: the landscape is bigger when you can see more.
    pub fn ambience_perspective(&self) -> f32 {
        1.12 - 0.26 * self.detail
    }
}

/// Ducking requests, raised by whatever caused them and released here.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct Duck {
    /// Raised while the player is laying track, so the clacks have room (§4).
    pub build: f32,
    /// Raised by a new alert.
    pub alert: f32,
}

impl Duck {
    pub fn on_build(&mut self) {
        self.build = 1.0;
    }

    pub fn on_alert(&mut self) {
        self.alert = 1.0;
    }
}

/// The farthest live instance in a category, and how far away it is.
type Farthest = Option<(Entity, f32)>;

/// Live one-shot voices per category, and the farthest of each.
#[derive(Resource, Debug, Default)]
pub struct VoiceBudget {
    live: [u16; CATEGORIES],
    farthest: [Farthest; CATEGORIES],
}

/// What the budget says about a requested sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    Play,
    /// Room was made by dropping a more distant instance.
    Replace(Entity),
    Deny,
}

impl VoiceBudget {
    /// Ask for a voice at `dist` world units from the listener.
    pub fn claim(&mut self, category: SoundCategory, dist: f32) -> Claim {
        let i = category.index();
        if self.live[i] < category.limit() {
            self.live[i] += 1;
            return Claim::Play;
        }
        match self.farthest[i] {
            Some((entity, far)) if far > dist => {
                self.farthest[i] = None;
                Claim::Replace(entity)
            }
            _ => Claim::Deny,
        }
    }
}

/// Marks a playing one-shot so the budget can count and rank it.
#[derive(Component, Debug, Clone, Copy)]
pub struct OneShot {
    pub category: SoundCategory,
    pub dist: f32,
}

/// The single spatial listener, following the camera.
#[derive(Component, Debug)]
pub struct AudioListener;

pub fn spawn_listener(mut commands: Commands) {
    commands.spawn((
        AudioListener,
        SpatialListener::new(EAR_GAP),
        Transform::default(),
    ));
}

/// The map camera, read for its position and its zoom, and excluded from the
/// listener query so the two `Transform` borrows stay disjoint.
type CameraView<'w, 's> =
    Query<'w, 's, (&'static Transform, &'static Projection), (With<MapCamera>, Without<AudioListener>)>;

/// Track the camera, and read zoom out of its projection.
pub fn sync_listener(
    camera: CameraView,
    map_view: Option<Res<MapViewState>>,
    mut listener: Query<&mut Transform, With<AudioListener>>,
    mut mix: ResMut<AudioMix>,
) {
    let Ok((cam, projection)) = camera.single() else {
        return;
    };
    mix.listener = cam.translation.truncate();
    if let Ok(mut transform) = listener.single_mut() {
        transform.translation = mix.listener.extend(0.0);
    }

    if let Projection::Orthographic(ortho) = projection {
        // Ortho scale is 1/zoom in play and TILE_SIZE/4 in Map View.
        mix.zoom = (1.0 / ortho.scale.max(1e-4)).clamp(0.1, 3.0);
    }
    if map_view.is_some_and(|state| state.active) {
        // The schematic read is not a place you stand in — pull all the way out.
        mix.zoom = 0.1;
    }
    mix.detail = ((mix.zoom - 1.0) / 2.0).clamp(0.0, 1.0);
    mix.radius = (RADIUS_TILES_WIDE + (RADIUS_TILES_CLOSE - RADIUS_TILES_WIDE) * mix.detail)
        * TILE_SIZE;
}

/// How fast a bus follows the settings panel. Fast enough to feel like the key
/// press caused it, slow enough that a ten-point step is a lean not a click.
const BUS_SLEW_SECS: f32 = 0.08;

/// Copy the player's volumes into the mix (brief §7, shell design §5).
///
/// **This is the writer [`AudioMix`] never had.** Every level in this module is
/// a product of [`AudioMix::master`] and one bus, and until now both sides of
/// that product were constants — so the Audio tab stored five numbers that
/// nothing read, which is exactly what the playtest found ("changing the volume
/// in the settings doesn't appear to change the sounds' volume").
///
/// Two details make the difference between working and appearing to work:
///
/// - **The buses are slewed, not assigned.** A jump from 80% to 70% on a live
///   ambience bed is a step, and a step is a startle (§1).
/// - **The live voices get it through [`super::voices::VoiceHandle::apply`]**,
///   which writes the sink every frame. `bevy_audio`'s own `GlobalVolume` is
///   applied once when a sink is created and never again, so it can only ever
///   reach one-shots — the beds, the trains and the score are precisely the
///   sounds it cannot touch, and they are the ones that are always playing.
///
/// `Settings` is optional so the audio plugin still builds headless, where the
/// shell does not exist.
pub fn apply_settings(
    settings: Option<Res<crate::shell::Settings>>,
    clock: Res<AudioClock>,
    mut mix: ResMut<AudioMix>,
) {
    let Some(settings) = settings else {
        return;
    };
    let dt = clock.real_delta.clamp(0.0, 0.25);
    let audio = &settings.audio;
    let pct = |v: u32| (v as f32 / 100.0).clamp(0.0, 1.0);
    mix.master = super::dsp::approach(mix.master, pct(audio.master), dt, BUS_SLEW_SECS);
    mix.music_bus = super::dsp::approach(mix.music_bus, pct(audio.music), dt, BUS_SLEW_SECS);
    mix.ambience_bus =
        super::dsp::approach(mix.ambience_bus, pct(audio.ambience), dt, BUS_SLEW_SECS);
    mix.effects_bus = super::dsp::approach(mix.effects_bus, pct(audio.effects), dt, BUS_SLEW_SECS);
    mix.ui_bus = super::dsp::approach(mix.ui_bus, pct(audio.ui), dt, BUS_SLEW_SECS);
    mix.mute_unfocused = audio.mute_on_focus_loss;
}

/// Focus mute and duck release.
pub fn refresh_mix(
    clock: Res<AudioClock>,
    windows: Query<&Window, With<PrimaryWindow>>,
    global: Option<Res<bevy::audio::GlobalVolume>>,
    mut duck: ResMut<Duck>,
    mut mix: ResMut<AudioMix>,
) {
    let dt = clock.real_delta.clamp(0.0, 0.25);

    // Mute on focus loss, by default (§7) — faded, because a cut is a transient.
    let focused = windows.iter().next().map(|w| w.focused).unwrap_or(true);
    let target = if focused || !mix.mute_unfocused { 1.0 } else { 0.0 };
    mix.focus = super::dsp::approach(mix.focus, target, dt, FOCUS_FADE_SECS);

    // See [`AudioMix::global_trim`]. Measured rather than assumed, so this stays
    // correct whoever else writes `GlobalVolume`.
    mix.global_trim = global
        .map(|g| (1.0 / g.volume.to_linear().max(0.02)).clamp(1.0, 16.0))
        .unwrap_or(1.0);

    duck.build = super::dsp::approach(duck.build, 0.0, dt, DUCK_RELEASE_SECS);
    duck.alert = super::dsp::approach(duck.alert, 0.0, dt, DUCK_RELEASE_SECS);

    let music_target = 1.0 - (1.0 - MUSIC_DUCK_FLOOR) * duck.build.clamp(0.0, 1.0);
    let ambience_target = 1.0 - (1.0 - AMBIENCE_DUCK_FLOOR) * duck.alert.clamp(0.0, 1.0);
    mix.music_duck = super::dsp::approach(mix.music_duck, music_target, dt, 0.35);
    mix.ambience_duck = super::dsp::approach(mix.ambience_duck, ambience_target, dt, 0.35);
}

/// Recount live one-shots so the next frame's claims are accurate.
pub fn refresh_budget(voices: Query<(Entity, &OneShot)>, mut budget: ResMut<VoiceBudget>) {
    budget.live = [0; CATEGORIES];
    budget.farthest = [None; CATEGORIES];
    for (entity, one_shot) in voices.iter() {
        let i = one_shot.category.index();
        budget.live[i] += 1;
        let farther = budget.farthest[i].map(|(_, d)| one_shot.dist > d).unwrap_or(true);
        if farther {
            budget.farthest[i] = Some((entity, one_shot.dist));
        }
    }
}

/// A request to play a one-shot.
pub struct Cue {
    pub clip: Handle<SampleClip>,
    pub category: SoundCategory,
    /// Pre-distance linear gain, normally one of the [`gain`] constants.
    pub gain: f32,
    /// Playback rate. Small jitter here is most of what stops a repeated sound
    /// from becoming a repeated *recording*.
    pub speed: f32,
    /// World position, or `None` for interface sound.
    pub at: Option<Vec2>,
}

impl Cue {
    pub fn world(clip: Handle<SampleClip>, category: SoundCategory, gain: f32, at: Vec2) -> Self {
        Self {
            clip,
            category,
            gain,
            speed: 1.0,
            at: Some(at),
        }
    }

    pub fn ui(clip: Handle<SampleClip>, gain: f32) -> Self {
        Self {
            clip,
            category: SoundCategory::Ui,
            gain,
            speed: 1.0,
            at: None,
        }
    }

    pub fn with_speed(mut self, speed: f32) -> Self {
        self.speed = speed.clamp(0.5, 2.0);
        self
    }
}

/// Play a one-shot, honouring distance, buses, focus and voice limits.
///
/// Returns `false` when the sound was inaudible or lost the voice race — the
/// caller does not need to care which.
pub fn play(commands: &mut Commands, budget: &mut VoiceBudget, mix: &AudioMix, cue: Cue) -> bool {
    let (volume, dist) = match cue.at {
        Some(at) => {
            let level = cue.gain * mix.falloff(at) * mix.effects() * mix.effect_perspective();
            (level, (at - mix.listener).length())
        }
        None => (cue.gain * mix.ui(), 0.0),
    };
    if volume < MIN_AUDIBLE {
        return false;
    }
    // `bevy_audio` folds `GlobalVolume` into the sink at creation; this takes it
    // back out so the master is applied exactly once, here.
    let volume = volume * mix.global_trim;

    match budget.claim(cue.category, dist) {
        Claim::Play => {}
        Claim::Replace(entity) => {
            commands.entity(entity).despawn();
        }
        Claim::Deny => return false,
    }

    let mut settings = PlaybackSettings::DESPAWN
        .with_volume(Volume::Linear(volume))
        .with_speed(cue.speed);
    if let Some(at) = cue.at {
        settings = settings
            .with_spatial(true)
            .with_spatial_scale(SpatialScale::new_2d(SPATIAL_SCALE));
        commands.spawn((
            AudioPlayer(cue.clip),
            settings,
            Transform::from_xyz(at.x, at.y, 0.0),
            OneShot {
                category: cue.category,
                dist,
            },
        ));
    } else {
        commands.spawn((
            AudioPlayer(cue.clip),
            settings,
            OneShot {
                category: cue.category,
                dist,
            },
        ));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_reaches_silence_at_the_radius() {
        let mut mix = AudioMix::default();
        mix.listener = Vec2::ZERO;
        mix.radius = 100.0;
        assert!((mix.falloff(Vec2::ZERO) - 1.0).abs() < 1e-6);
        assert!(mix.falloff(Vec2::new(50.0, 0.0)) < 1.0);
        assert_eq!(mix.falloff(Vec2::new(100.0, 0.0)), 0.0);
        assert_eq!(mix.falloff(Vec2::new(5000.0, 0.0)), 0.0);
        // Monotone: no distance is louder than a nearer one.
        let mut last = 1.0;
        for i in 0..=100 {
            let g = mix.falloff(Vec2::new(i as f32, 0.0));
            assert!(g <= last + 1e-6, "falloff rose at {i}");
            last = g;
        }
    }

    #[test]
    fn distance_is_dull_as_well_as_quiet() {
        let mut mix = AudioMix::default();
        mix.radius = 100.0;
        assert!(mix.brightness(Vec2::ZERO) > 0.99);
        assert!(mix.brightness(Vec2::new(90.0, 0.0)) < 0.1);
    }

    #[test]
    fn the_dynamic_range_is_narrow() {
        // Acceptance: nothing is dramatically louder than anything else (§7).
        let loudest = gain::WHISTLE;
        let quietest = gain::UI_CLICK;
        let span_db = 20.0 * (loudest / quietest).log10();
        assert!(span_db < 12.0, "dynamic range spans {span_db:.1} dB");
        // And the whistle really is the loudest single event in the game.
        for g in [
            gain::CLACK,
            gain::BRIDGE,
            gain::STATION,
            gain::DEMOLISH,
            gain::MILESTONE,
            gain::MUSIC,
            gain::AMBIENCE_TOTAL,
        ] {
            assert!(g <= gain::WHISTLE, "{g} beats the whistle");
        }
    }

    #[test]
    fn the_invalid_thud_is_quieter_than_the_clack() {
        // It is heard on every rejected drag; it must never dominate.
        assert!(gain::INVALID < gain::CLACK);
    }

    #[test]
    fn voice_limits_keep_the_nearest() {
        let mut budget = VoiceBudget::default();
        let limit = SoundCategory::Build.limit();
        for _ in 0..limit {
            assert_eq!(budget.claim(SoundCategory::Build, 10.0), Claim::Play);
        }
        // Full, and nothing to displace yet.
        assert_eq!(budget.claim(SoundCategory::Build, 10.0), Claim::Deny);

        budget.farthest[SoundCategory::Build.index()] = Some((Entity::from_raw_u32(7).unwrap(), 900.0));
        // A nearer sound displaces the farthest one; a farther one does not.
        assert!(matches!(
            budget.claim(SoundCategory::Build, 20.0),
            Claim::Replace(_)
        ));
        budget.farthest[SoundCategory::Build.index()] = Some((Entity::from_raw_u32(7).unwrap(), 30.0));
        assert_eq!(budget.claim(SoundCategory::Build, 900.0), Claim::Deny);

        // Categories do not steal from each other.
        assert_eq!(budget.claim(SoundCategory::Ui, 0.0), Claim::Play);
    }

    #[test]
    fn zoom_moves_the_mix_the_right_way() {
        let mut wide = AudioMix::default();
        wide.detail = 0.0;
        let mut close = AudioMix::default();
        close.detail = 1.0;
        assert!(close.effect_perspective() > wide.effect_perspective());
        assert!(close.ambience_perspective() < wide.ambience_perspective());
    }

    #[test]
    fn ducking_never_goes_hard() {
        assert!(MUSIC_DUCK_FLOOR > 0.5, "music must stay present");
        assert!(AMBIENCE_DUCK_FLOOR > 0.7, "ambience barely moves");
    }

    #[test]
    fn focus_loss_silences_every_bus() {
        let mut mix = AudioMix::default();
        mix.focus = 0.0;
        assert_eq!(mix.effects(), 0.0);
        assert_eq!(mix.ui(), 0.0);
        assert_eq!(mix.music(), 0.0);
        assert_eq!(mix.ambience(), 0.0);
    }
}
