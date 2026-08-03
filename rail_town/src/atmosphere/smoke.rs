//! Chimney smoke — a four-frame plume on occupied buildings (brief 01 §6.3).
//!
//! Gated on density twice: a house only gets a chimney once its tile is
//! properly built up, and a dense tile gets a second plume at its own phase.
//! That keeps smoke as a read of where the town is *living* rather than as a
//! uniform fog over every roof.
//!
//! The puff steps through four whole-texel positions as it rises and thins — it
//! does not slide, and it does not rotate. Phase is world-hashed so no two
//! chimneys puff together, and the plume bakes with the window layer, off the
//! same quantized density level.

use std::collections::HashMap;

use bevy::prelude::*;
use rail_map::tile_to_world;
use rail_sim::TileCoord;

use super::bake::{building_extent, DensityLevels};
use crate::hash::{frame_at, hash_offset, hash_phase};
use super::{AmbientClock, CHIMNEY_SMOKE_Z};
use crate::palette::BALLAST_L;

/// Full plume loop in seconds (brief §6.3: ~3 s, four frames).
pub(crate) const CHIMNEY_SMOKE_PERIOD: f32 = 3.0;

const SMOKE_PHASE_SALT: u32 = 0x534d_4f4b;
const SMOKE_SECOND_SALT: u32 = 0x5354_4143;
const SMOKE_OFFSET_SALT: u32 = 0x4348_494d;

/// Density level a tile must reach before its chimney is lit — about 45%
/// density, which is a district that is genuinely occupied rather than one
/// house on a lane.
const SMOKE_MIN_LEVEL: u8 = 7;
/// Level at which a tile earns a second, separately phased plume (~78%).
const SECOND_PLUME_LEVEL: u8 = 12;

/// A puff is two texels square.
const PUFF_TEXELS: f32 = 2.0;
/// How far a chimney may sit off the roof's centre line, in texels.
const CHIMNEY_SCATTER: i32 = 2;

/// Four-frame plume: `(rise above the roof, drift, alpha)`. Whole texels only.
const SMOKE_FRAMES: [(f32, f32, f32); 4] = [
    (2.0, 0.0, 0.42),
    (5.0, 1.0, 0.34),
    (8.0, 2.0, 0.22),
    (11.0, 2.0, 0.10),
];

/// One rising puff.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct ChimneySmoke {
    /// Roofline the plume rises from, in whole world texels.
    origin: Vec2,
    phase: f32,
    frame: u8,
}

/// Baked plume entities per tile.
#[derive(Resource, Default)]
pub(crate) struct SmokeLayer {
    tiles: HashMap<(i32, i32), Vec<Entity>>,
}

/// Rebuild chimneys for tiles whose density level moved.
pub(crate) fn bake_chimney_smoke(
    mut commands: Commands,
    levels: Res<DensityLevels>,
    mut layer: ResMut<SmokeLayer>,
) {
    let _perf = crate::overlays::perf::scope("bake_chimney_smoke");
    if levels.changed().is_empty() {
        return;
    }

    for change in levels.changed() {
        let key = (change.tile.x, change.tile.y);
        if let Some(previous) = layer.tiles.remove(&key) {
            for entity in previous {
                commands.entity(entity).despawn();
            }
        }
        let Some(level) = change.level else {
            continue;
        };
        if level < SMOKE_MIN_LEVEL {
            continue;
        }
        let mut plumes = vec![spawn_plume(&mut commands, change.tile, level, SMOKE_PHASE_SALT)];
        if level >= SECOND_PLUME_LEVEL {
            plumes.push(spawn_plume(
                &mut commands,
                change.tile,
                level,
                SMOKE_SECOND_SALT,
            ));
        }
        layer.tiles.insert(key, plumes);
    }
}

fn spawn_plume(commands: &mut Commands, tile: TileCoord, level: u8, salt: u32) -> Entity {
    let (_, height) = building_extent(level);
    let (cx, cy) = tile_to_world(tile);
    // World-anchored chimney placement: the same house always smokes from the
    // same corner of its roof.
    let offset = hash_offset(tile.x, tile.y, salt ^ SMOKE_OFFSET_SALT, CHIMNEY_SCATTER);
    let origin = Vec2::new(cx + offset as f32, cy + (height / 2) as f32);
    let phase = hash_phase(tile.x, tile.y, salt, CHIMNEY_SMOKE_PERIOD);
    let (rise, drift, alpha) = SMOKE_FRAMES[0];

    commands
        .spawn((
            ChimneySmoke {
                origin,
                phase,
                frame: 0,
            },
            Sprite::from_color(BALLAST_L.with_alpha(alpha), Vec2::splat(PUFF_TEXELS)),
            Transform::from_xyz(origin.x + drift, origin.y + rise, CHIMNEY_SMOKE_Z),
        ))
        .id()
}

/// Advance plumes; touch only the puffs whose frame turned over.
pub(crate) fn step_chimney_smoke(
    ambient: Res<AmbientClock>,
    mut plumes: Query<(&mut ChimneySmoke, &mut Sprite, &mut Transform)>,
) {
    let _perf = crate::overlays::perf::scope("step_chimney_smoke");
    for (mut smoke, mut sprite, mut transform) in plumes.iter_mut() {
        let frame = frame_at(
            ambient.secs,
            smoke.phase,
            CHIMNEY_SMOKE_PERIOD,
            SMOKE_FRAMES.len() as u32,
        ) as u8;
        if frame == smoke.frame {
            continue;
        }
        smoke.frame = frame;
        let (rise, drift, alpha) = SMOKE_FRAMES[frame as usize];
        sprite.color = BALLAST_L.with_alpha(alpha);
        transform.translation.x = smoke.origin.x + drift;
        transform.translation.y = smoke.origin.y + rise;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atmosphere::bake::{density_level, DENSITY_STEPS};

    #[test]
    fn plume_matches_the_brief() {
        assert!((CHIMNEY_SMOKE_PERIOD - 3.0).abs() < f32::EPSILON);
        assert_eq!(SMOKE_FRAMES.len(), 4);
    }

    #[test]
    fn smoke_is_gated_on_density() {
        assert!(density_level(0.20).expect("built") < SMOKE_MIN_LEVEL);
        assert!(density_level(0.50).expect("built") >= SMOKE_MIN_LEVEL);
        assert!(density_level(0.50).expect("built") < SECOND_PLUME_LEVEL);
        assert!(density_level(0.85).expect("built") >= SECOND_PLUME_LEVEL);
        assert!(SMOKE_MIN_LEVEL < SECOND_PLUME_LEVEL);
        assert!(SECOND_PLUME_LEVEL < DENSITY_STEPS);
    }

    #[test]
    fn plume_rises_and_thins_on_whole_texels() {
        let mut last_rise = -1.0;
        let mut last_alpha = 1.0;
        for (rise, drift, alpha) in SMOKE_FRAMES {
            assert_eq!(rise.fract(), 0.0);
            assert_eq!(drift.fract(), 0.0);
            assert!(rise > last_rise, "smoke must rise");
            assert!(alpha < last_alpha, "smoke must thin");
            last_rise = rise;
            last_alpha = alpha;
        }
        assert_eq!(PUFF_TEXELS as i32 % 2, 0);
    }

    #[test]
    fn chimneys_do_not_puff_in_unison() {
        let mut on_frame = [0; 4];
        for y in 0..24 {
            for x in 0..24 {
                let phase = hash_phase(x, y, SMOKE_PHASE_SALT, CHIMNEY_SMOKE_PERIOD);
                on_frame[frame_at(1.7, phase, CHIMNEY_SMOKE_PERIOD, 4) as usize] += 1;
            }
        }
        assert!(
            on_frame.iter().all(|&n| n > 100),
            "plumes should spread across the loop: {on_frame:?}"
        );
    }

    #[test]
    fn the_second_plume_has_its_own_phase() {
        let first = hash_phase(6, 9, SMOKE_PHASE_SALT, CHIMNEY_SMOKE_PERIOD);
        let second = hash_phase(6, 9, SMOKE_SECOND_SALT, CHIMNEY_SMOKE_PERIOD);
        assert!((first - second).abs() > 0.05);
    }

    #[test]
    fn chimneys_sit_on_the_roofline() {
        for level in SMOKE_MIN_LEVEL..DENSITY_STEPS {
            let (width, height) = building_extent(level);
            let offset = hash_offset(3, 4, SMOKE_PHASE_SALT ^ SMOKE_OFFSET_SALT, CHIMNEY_SCATTER);
            assert!(offset.abs() <= width / 2, "chimney overhangs the wall");
            assert_eq!(height % 2, 0, "roofline must be a whole texel");
        }
    }
}
