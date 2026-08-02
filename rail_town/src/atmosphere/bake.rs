//! Density levels — the one place town density turns into "the art changed".
//!
//! [`TownDensity`] is a continuous field that the growth system nudges every
//! sim tick, so "has it changed?" is always yes and change detection buys
//! nothing. Quantizing it into [`DENSITY_STEPS`] gives a value that only moves
//! when the town has visibly grown, which is the event the window and chimney
//! layers actually want to rebuild on (brief 01 §2.5: bake on data change,
//! never per frame).
//!
//! Quantizing is also the pixel-art answer: a building's window grid should
//! step up as the district thickens, not creep by a texel a second.

use std::collections::HashMap;

use bevy::prelude::*;
use rail_map::TILE_SIZE;
use rail_sim::{TileCoord, TownDensity};

/// Density steps kept for baking. Sixteen is fine enough that growth reads as
/// continuous and coarse enough that a tile rebuilds a handful of times over a
/// whole game.
pub(crate) const DENSITY_STEPS: u8 = 16;

/// Density below which `town/buildings.rs` draws no building at all.
///
/// Mirrored rather than imported: that module belongs to the town slice, and a
/// window with no wall behind it is worse than a missing window.
pub(crate) const BUILDING_MIN_DENSITY: f32 = 0.08;

/// Quantized density level for a tile, or `None` where no building is drawn.
pub(crate) fn density_level(density: f32) -> Option<u8> {
    if density < BUILDING_MIN_DENSITY {
        return None;
    }
    let step = (density.clamp(0.0, 1.0) * DENSITY_STEPS as f32).floor() as u8;
    Some(step.min(DENSITY_STEPS - 1))
}

/// Representative density for a level — its midpoint.
pub(crate) fn level_density(level: u8) -> f32 {
    (level.min(DENSITY_STEPS - 1) as f32 + 0.5) / DENSITY_STEPS as f32
}

/// Placeholder building footprint at `level`, in whole texels.
///
/// Mirrors `apply_building_look` in `town/buildings.rs` so our layers sit on
/// their sprite, evaluated at the quantized density and rounded to even texels
/// so a centred child lands on texel boundaries (pixel contract §2.1).
pub(crate) fn building_extent(level: u8) -> (i32, i32) {
    let size = TILE_SIZE * (0.15 + 0.55 * level_density(level));
    (even_floor(size * 0.7), even_floor(size))
}

fn even_floor(v: f32) -> i32 {
    let n = v.floor().max(2.0) as i32;
    n - (n % 2)
}

/// A tile whose quantized density level moved this frame.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LevelChange {
    pub tile: TileCoord,
    /// `None` when the tile dropped below the building threshold.
    pub level: Option<u8>,
}

/// Quantized density per tile, plus the changes seen this frame.
#[derive(Resource, Default)]
pub(crate) struct DensityLevels {
    levels: HashMap<(i32, i32), u8>,
    changed: Vec<LevelChange>,
}

impl DensityLevels {
    pub(crate) fn changed(&self) -> &[LevelChange] {
        &self.changed
    }
}

/// Diff [`TownDensity`] against the last baked levels.
///
/// Runs before the bakers each frame; on a settled town it produces an empty
/// change list and every consumer returns immediately.
pub(crate) fn track_density_levels(density: Res<TownDensity>, mut levels: ResMut<DensityLevels>) {
    let _perf = crate::overlays::perf::scope("track_density_levels");
    levels.changed.clear();

    for (tile, d) in density.iter() {
        let key = (tile.x, tile.y);
        let level = density_level(d);
        let previous = levels.levels.get(&key).copied();
        if previous == level {
            continue;
        }
        match level {
            Some(l) => {
                levels.levels.insert(key, l);
            }
            None => {
                levels.levels.remove(&key);
            }
        }
        levels.changed.push(LevelChange { tile, level });
    }

    // Tiles that vanished from the sparse map entirely (density decayed to
    // nothing) still owe their sprites a despawn.
    let dropped: Vec<(i32, i32)> = levels
        .levels
        .keys()
        .copied()
        .filter(|&(x, y)| density_level(density.get(TileCoord { x, y })).is_none())
        .collect();
    for key in dropped {
        levels.levels.remove(&key);
        levels.changed.push(LevelChange {
            tile: TileCoord {
                x: key.0,
                y: key.1,
            },
            level: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tiles_have_no_level() {
        assert_eq!(density_level(0.0), None);
        assert_eq!(density_level(BUILDING_MIN_DENSITY - 0.001), None);
        assert!(density_level(BUILDING_MIN_DENSITY).is_some());
    }

    #[test]
    fn levels_are_monotonic_and_bounded() {
        let mut last = 0;
        let mut d = BUILDING_MIN_DENSITY;
        while d <= 1.0 {
            let level = density_level(d).expect("above threshold");
            assert!(level >= last);
            assert!(level < DENSITY_STEPS);
            last = level;
            d += 0.01;
        }
        assert_eq!(density_level(1.0), Some(DENSITY_STEPS - 1));
    }

    #[test]
    fn growth_only_rebuilds_when_a_level_lands() {
        // A tick of growth (4% toward target) must not rebake every frame.
        let a = density_level(0.500);
        let b = density_level(0.504);
        assert_eq!(a, b, "sub-step growth must not change the baked level");
        assert_ne!(density_level(0.49), density_level(0.51));
    }

    #[test]
    fn building_extents_are_even_and_grow_with_density() {
        let (w0, h0) = building_extent(1);
        let (w1, h1) = building_extent(DENSITY_STEPS - 1);
        assert!(w1 > w0 && h1 > h0);
        for level in 0..DENSITY_STEPS {
            let (w, h) = building_extent(level);
            assert_eq!(w % 2, 0, "width must be even at level {level}");
            assert_eq!(h % 2, 0, "height must be even at level {level}");
            assert!(w >= 2 && h >= 2);
            // Never wider than the tile it stands on.
            assert!((w as f32) <= TILE_SIZE && (h as f32) <= TILE_SIZE);
        }
    }
}
