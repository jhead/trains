//! The ambience bed (brief §2) — a continuous landscape composed from whatever
//! the camera is looking at.
//!
//! Six layers cross-fade against each other every frame from a coarse sample of
//! the terrain, the town and the clock, so panning from coast to mountain to
//! town is an audible journey rather than a set of switches.
//!
//! **The town layer is the emotional one.** It does not merely get quieter as a
//! district declines — its activity rate collapses, its upper formant
//! disappears and it darkens, because a half-empty street sounds *emptier*, not
//! just *further away*. That is acceptance bar 1: a player can tell with their
//! eyes closed whether the town near the camera is thriving.
//!
//! Time of day comes from [`TimeOfDay`], and the modulation clock is the same
//! [`AmbientClock`](crate::atmosphere::AmbientClock) the visual ambience uses,
//! so the two freeze together on pause.

use bevy::audio::{AudioSink, SpatialAudioSink};
use bevy::prelude::*;
use rail_map::{MapGrid, TerrainKind, TILE_SIZE};
use rail_sim::{IndustryRegistry, TileCoord, TownDensity};

use crate::atmosphere::TimeOfDay;

use super::dsp::{lerp, smoothstep};
use super::mixer::{gain, AudioClock, AudioMix};
use super::voice::{LiveVoice, VoiceKind};
use super::voices::{spawn_bed, VoiceHandle};

/// Tiles sampled across the visible area, per axis. 13×13 is 169 lookups a
/// frame — free, and fine-grained enough that a coastline crossfades smoothly.
const SAMPLES: i32 = 13;

/// Cross-fade time constant for the beds. Slow: panning should feel like
/// arriving somewhere, not like flipping a switch.
const BED_FADE_SECS: f32 = 2.2;

/// The town layer fades slower still — growth and decline are not events.
const TOWN_FADE_SECS: f32 = 4.0;

/// The bed voices and their last computed weights.
#[derive(Resource, Debug, Default)]
pub struct AmbienceBeds {
    pub voices: Vec<VoiceHandle>,
    /// Mean town density in view, kept for the music director.
    pub town_density: f32,
    /// Mean wild-land fraction in view, kept for diagnostics and tests.
    pub wild: f32,
}

impl AmbienceBeds {
    fn voice(&mut self, kind: VoiceKind) -> Option<&mut VoiceHandle> {
        self.voices.iter_mut().find(|v| v.kind == kind)
    }
}

pub fn spawn_ambience(
    mut commands: Commands,
    mut assets: ResMut<Assets<LiveVoice>>,
    mut beds: ResMut<AmbienceBeds>,
) {
    for (i, kind) in VoiceKind::BEDS.iter().enumerate() {
        let handle = spawn_bed(&mut commands, &mut assets, *kind, 0x5eed + i as u64 * 977);
        beds.voices.push(handle);
    }
}

/// What the camera is looking at, as numbers the beds can use.
#[derive(Debug, Clone, Copy, Default)]
struct Scene {
    water: f32,
    coast: f32,
    wild: f32,
    altitude: f32,
    town: f32,
    industry: f32,
}

fn sample_scene(
    map: &MapGrid,
    density: &TownDensity,
    industries: &IndustryRegistry,
    center: Vec2,
    radius: f32,
) -> Scene {
    let mut scene = Scene::default();
    let mut taken = 0.0f32;
    let step = (2.0 * radius / SAMPLES as f32).max(TILE_SIZE);
    let half = SAMPLES / 2;
    for sy in -half..=half {
        for sx in -half..=half {
            let world = center + Vec2::new(sx as f32 * step, sy as f32 * step);
            let tile = rail_map::world_to_tile(world.x, world.y);
            let Some(t) = map.get(tile) else {
                continue;
            };
            taken += 1.0;
            if t.water || t.kind == TerrainKind::Water {
                scene.water += 1.0;
            }
            if t.kind == TerrainKind::Beach {
                scene.coast += 1.0;
            }
            if matches!(t.kind, TerrainKind::Plains | TerrainKind::Hills) {
                scene.wild += 1.0 - density.get(tile).min(1.0);
            }
            scene.altitude += (t.height.max(0) as f32 / 6.0).clamp(0.0, 1.0);
            scene.town += density.get(tile);
        }
    }
    if taken > 0.0 {
        scene.water /= taken;
        scene.coast /= taken;
        scene.wild /= taken;
        scene.altitude /= taken;
        scene.town /= taken;
    }

    // Industry is counted rather than sampled: there are few of them and they
    // are single tiles, so a grid would miss them.
    let mut near = 0.0f32;
    for industry in industries.iter() {
        let at = tile_center(industry.tile);
        if (at - center).length() < radius * 1.2 {
            near += 1.0;
        }
    }
    scene.industry = (near / 3.0).clamp(0.0, 1.0);
    scene
}

fn tile_center(tile: TileCoord) -> Vec2 {
    let (x, y) = rail_map::tile_to_world(tile);
    Vec2::new(x, y)
}

/// Dawn chorus, morning, thinning through the afternoon, gone by night.
///
/// Authored as keyframes on the same cycle position the tint pass uses, and
/// interpolated with smoothstep so both ends of the day meet at zero. **The
/// wrap seam is the one that matters**: a curve that is loud at `0.0` and
/// silent at `1.0` would switch the dawn chorus on like a light every day.
const BIRD_STOPS: [(f32, f32); 6] = [
    (0.00, 0.00), // first light, about to begin
    (0.06, 1.00), // the chorus
    (0.16, 0.55),
    (0.45, 0.30),
    (0.62, 0.00), // gone by the end of dusk
    (1.00, 0.00), // and silent all night
];

fn birdsong_at(fraction: f32) -> f32 {
    let f = fraction.rem_euclid(1.0);
    let mut previous = BIRD_STOPS[0];
    for &stop in BIRD_STOPS.iter().skip(1) {
        if f <= stop.0 {
            let t = smoothstep(previous.0, stop.0, f);
            return lerp(previous.1, stop.1, t);
        }
        previous = stop;
    }
    BIRD_STOPS[BIRD_STOPS.len() - 1].1
}

/// Compose and cross-fade the bed.
#[allow(clippy::too_many_arguments)]
pub fn drive_ambience(
    map: Res<MapGrid>,
    density: Res<TownDensity>,
    industries: Res<IndustryRegistry>,
    tod: Res<TimeOfDay>,
    clock: Res<AudioClock>,
    mix: Res<AudioMix>,
    mut beds: ResMut<AmbienceBeds>,
    mut sinks: Query<&mut AudioSink>,
    mut spatial: Query<&mut SpatialAudioSink>,
) {
    if beds.voices.is_empty() {
        return;
    }
    let dt = clock.real_delta.clamp(0.0, 0.25);
    let scene = sample_scene(
        &map,
        &density,
        &industries,
        mix.listener,
        mix.radius.max(TILE_SIZE * 6.0),
    );
    beds.town_density = scene.town;
    beds.wild = scene.wild;

    let dark = tod.window_lit.clamp(0.0, 1.0);
    let birds = birdsong_at(tod.fraction);

    // -- weights ----------------------------------------------------------
    // Wind is the base layer everywhere and never leaves.
    let w_wind = 0.42 + 0.30 * scene.altitude + 0.12 * (1.0 - scene.town);
    let w_water = smoothstep(0.01, 0.32, scene.water) * (0.9 + 0.2 * scene.coast);
    let w_forest = smoothstep(0.05, 0.55, scene.wild) * (0.30 + 0.70 * birds) * (1.0 - dark * 0.85);
    let w_night = dark * (0.25 + 0.75 * scene.wild) * 0.8;
    // A town sleeps but never quite stops; a thriving one is audible at night.
    let w_town = smoothstep(0.015, 0.45, scene.town) * (1.0 - 0.55 * dark);
    let w_industry = scene.industry * (1.0 - 0.7 * dark) * smoothstep(0.0, 0.25, scene.town + 0.15);

    let weights = [
        (VoiceKind::Wind, w_wind),
        (VoiceKind::Water, w_water),
        (VoiceKind::Forest, w_forest),
        (VoiceKind::Night, w_night),
        (VoiceKind::Town, w_town),
        (VoiceKind::Industry, w_industry),
    ];
    // Share out one bed's worth of level. When only wind is present the bed is
    // genuinely quieter — silence is a texture, not a gap to fill (§1).
    let total: f32 = weights.iter().map(|(_, w)| w).sum::<f32>().max(1.0);
    let bus = gain::AMBIENCE_TOTAL * mix.ambience() * mix.ambience_perspective();

    // -- parameters -------------------------------------------------------
    if let Some(v) = beds.voice(VoiceKind::Wind) {
        // Thinner and higher at altitude.
        v.params.set_tone(0.25 + 0.65 * scene.altitude);
        v.params.set_motion(0.3 + 0.5 * scene.altitude);
        v.params.set_depth(1.0 - scene.altitude);
    }
    if let Some(v) = beds.voice(VoiceKind::Water) {
        let coastal = smoothstep(0.02, 0.25, scene.coast);
        v.params.set_color(coastal);
        v.params.set_tone(0.35 + 0.45 * coastal);
        v.params.set_motion(0.35 + 0.3 * coastal);
    }
    if let Some(v) = beds.voice(VoiceKind::Forest) {
        v.params.set_density(birds);
        v.params.set_tone(0.5);
    }
    if let Some(v) = beds.voice(VoiceKind::Night) {
        v.params.set_density(dark * (0.2 + 0.8 * scene.wild));
    }
    if let Some(v) = beds.voice(VoiceKind::Town) {
        // The diagnostic: activity rate and brightness both track density, so a
        // declining district goes empty rather than merely distant.
        let life = smoothstep(0.02, 0.55, scene.town);
        v.params.set_density(life * (1.0 - 0.75 * dark));
        v.params.set_tone(0.25 + 0.6 * life * (1.0 - 0.5 * dark));
        v.params.set_depth(life);
        v.params.set_color(dark);
    }
    if let Some(v) = beds.voice(VoiceKind::Industry) {
        // "Only while working": the work rate follows the serviced town around
        // it, so a cut-off industry falls quiet within a minute.
        v.params.set_motion(smoothstep(0.02, 0.4, scene.town) * (1.0 - 0.6 * dark));
        v.params.set_tone(0.3);
        v.params.set_depth(0.6);
    }

    // -- levels -----------------------------------------------------------
    for (kind, weight) in weights {
        let tau = if kind == VoiceKind::Town {
            TOWN_FADE_SECS
        } else {
            BED_FADE_SECS
        };
        let target = bus * (weight / total);
        if let Some(voice) = beds.voice(kind) {
            voice.apply(target, dt, tau, &mut sinks, &mut spatial);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn birdsong_peaks_at_dawn_and_stops_at_night() {
        let dawn = birdsong_at(0.06);
        let day = birdsong_at(0.30);
        let dusk = birdsong_at(0.58);
        let night = birdsong_at(0.85);
        assert!(dawn > day, "dawn {dawn} should beat day {day}");
        assert!(day > dusk, "day {day} should beat dusk {dusk}");
        assert_eq!(night, 0.0, "birds do not sing at night");
        for step in 0..500 {
            let f = step as f32 / 500.0;
            assert!((0.0..=1.0).contains(&birdsong_at(f)));
        }
    }

    #[test]
    fn birdsong_never_jumps_including_across_the_wrap() {
        // Every ambient curve has to be a crossfade; a step is a startle. One
        // step here is a fifth of a sim second at 1x.
        let steps = 4000;
        let mut worst = 0.0f32;
        for i in 0..steps {
            let a = birdsong_at(i as f32 / steps as f32);
            let b = birdsong_at((i + 1) as f32 / steps as f32);
            worst = worst.max((a - b).abs());
        }
        assert!(worst < 0.02, "birdsong jumps by {worst}");
        // Explicitly: midnight is silent on both sides of the seam.
        assert_eq!(birdsong_at(0.999), 0.0);
        assert_eq!(birdsong_at(0.0), 0.0);
    }

    fn scene_of(map: &MapGrid, density: &TownDensity, at: Vec2) -> Scene {
        sample_scene(map, density, &IndustryRegistry::default(), at, TILE_SIZE * 12.0)
    }

    #[test]
    fn the_scene_reads_water_and_town_separately() {
        let map = rail_map::generate_map(48, 48, 42);
        let mut density = TownDensity::default();
        for y in 10..18 {
            for x in 10..18 {
                density.set(TileCoord { x, y }, 0.9);
            }
        }
        let over_town = scene_of(&map, &density, tile_center(TileCoord { x: 14, y: 14 }));
        let elsewhere = scene_of(&map, &density, tile_center(TileCoord { x: 40, y: 40 }));
        assert!(
            over_town.town > elsewhere.town,
            "town {} vs {}",
            over_town.town,
            elsewhere.town
        );
        assert!((0.0..=1.0).contains(&over_town.water));
        assert!((0.0..=1.0).contains(&over_town.wild));
        assert!((0.0..=1.0).contains(&over_town.altitude));
    }

    #[test]
    fn an_empty_map_still_has_wind() {
        // Brief §2: wind is the base layer everywhere. Even over nothing, the
        // bed is not silence.
        let scene = Scene::default();
        let w_wind = 0.42 + 0.30 * scene.altitude + 0.12 * (1.0 - scene.town);
        assert!(w_wind > 0.4);
    }
}
