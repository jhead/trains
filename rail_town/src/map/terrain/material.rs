//! Terrain materials, ramps, and world-anchored variant selection.
//!
//! Materials are ordered low → high and the index doubles as **autotile
//! priority**: a higher material always laps onto a lower one, so every
//! boundary in [01 §6.2] is drawn exactly once, on the low tile.
//!
//! Every colour comes from [`crate::palette`] — nothing here mixes, blends or
//! tints. Ramps are four steps: dark · mid · light · cap, where the cap is the
//! material's reserved extreme (`WATER_F` shallows, `SNOW` above the top band)
//! and simply repeats the light step for materials that have none.

use bevy::prelude::Color;
use rail_map::TerrainKind;
use rail_sim::ids::TileCoord;

use crate::palette::{
    GRASS_D, GRASS_L, GRASS_M, HILL_D, HILL_L, HILL_M, OUTLINE, ROCK_D, ROCK_L, ROCK_M, SAND_D,
    SAND_L, SAND_M, SNOW, WATER_D, WATER_F, WATER_L, WATER_M,
};

/// Steps per material ramp: dark · mid · light · cap.
pub const SHADES: usize = 4;
/// Flat variants per (material, shade), chosen by world hash (brief 01 §6.2.3).
pub const VARIANTS: usize = 3;

/// Height units per legible elevation band (brief 02 §2.3).
pub const BAND_STEP: i8 = 3;

/// Autotiling materials, low → high priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Material {
    Water,
    Sand,
    Grass,
    Hill,
    Rock,
}

/// Every material, in priority order.
pub const MATERIALS: [Material; 5] = [
    Material::Water,
    Material::Sand,
    Material::Grass,
    Material::Hill,
    Material::Rock,
];
pub const MATERIAL_COUNT: usize = MATERIALS.len();
/// Boundaries between consecutive materials: water↔beach, beach↔grass,
/// grass↔hills, hills↔mountain.
pub const BOUNDARY_COUNT: usize = MATERIAL_COUNT - 1;

impl Material {
    #[inline]
    pub fn index(self) -> usize {
        match self {
            Material::Water => 0,
            Material::Sand => 1,
            Material::Grass => 2,
            Material::Hill => 3,
            Material::Rock => 4,
        }
    }

    /// Four-step ramp for this material (brief 01 §3.2).
    #[inline]
    pub fn ramp(self) -> [Color; SHADES] {
        match self {
            Material::Water => [WATER_D, WATER_M, WATER_L, WATER_F],
            Material::Sand => [SAND_D, SAND_M, SAND_L, SAND_L],
            Material::Grass => [GRASS_D, GRASS_M, GRASS_L, GRASS_L],
            Material::Hill => [HILL_D, HILL_M, HILL_L, HILL_L],
            Material::Rock => [ROCK_D, ROCK_M, ROCK_L, SNOW],
        }
    }

    /// One ramp step, clamped.
    #[inline]
    pub fn step(self, shade: usize) -> Color {
        self.ramp()[shade.min(SHADES - 1)]
    }

    /// The nearest visibly darker step, skipping a cap that merely repeats the
    /// light step. At the bottom of a ramp there is nowhere left to go, so
    /// speckle falls back to [`OUTLINE`] — the one shadow colour in the game,
    /// and the palette's stated cool-violet shadow key (brief 01 §3.1).
    #[inline]
    pub fn shadow(self, shade: usize) -> Color {
        let ramp = self.ramp();
        let base = self.step(shade);
        for step in ramp[..shade.min(SHADES - 1)].iter().rev() {
            if *step != base {
                return *step;
            }
        }
        OUTLINE
    }

    /// The nearest visibly lighter step, or the colour itself at the top.
    #[inline]
    pub fn highlight(self, shade: usize) -> Color {
        let ramp = self.ramp();
        let base = self.step(shade);
        for step in &ramp[(shade.min(SHADES - 1) + 1).min(SHADES)..] {
            if *step != base {
                return *step;
            }
        }
        base
    }
}

#[inline]
pub fn material_of(kind: TerrainKind) -> Material {
    match kind {
        TerrainKind::Water => Material::Water,
        TerrainKind::Beach => Material::Sand,
        TerrainKind::Plains => Material::Grass,
        TerrainKind::Hills => Material::Hill,
        TerrainKind::Mountain => Material::Rock,
    }
}

/// Ramp step for a tile's elevation.
///
/// Flat ground stays in the bottom two-thirds of its ramp (brief 01 §3.3) — no
/// land material reaches its light step from height alone below the mountain
/// peaks, so an expanse of plains can never out-shout the track laid across it.
/// The light step arrives as a *drawn* sun lip on the tile edge instead.
///
/// Water is the deliberate exception: its upper steps are shallows, and depth
/// banding is what stops open sea being one flat blue field (brief 01 §6.2.3).
#[inline]
pub fn shade_for(kind: TerrainKind, height: i8) -> usize {
    match kind {
        TerrainKind::Water => match height {
            ..=-8 => 0,
            -7..=-4 => 1,
            -3..=-2 => 2,
            _ => 3,
        },
        TerrainKind::Beach => usize::from(height > 0),
        TerrainKind::Plains => usize::from(height > 3),
        TerrainKind::Hills => usize::from(height > 8),
        TerrainKind::Mountain => match height {
            ..=12 => 0,
            13..=14 => 1,
            15 => 2,
            _ => 3,
        },
    }
}

/// Discrete elevation band — the step the terrace contour is drawn at.
#[inline]
pub fn elevation_band(height: i16) -> i32 {
    (height as i32).div_euclid(BAND_STEP as i32)
}

/// Height a tile presents to its neighbours for cliff purposes.
///
/// Water reads as its surface, not its bed, so a beach beside a deep channel is
/// a shoreline rather than a nine-band cliff.
#[inline]
pub fn surface_height(height: i8, water: bool) -> i16 {
    if water {
        0
    } else {
        height as i16
    }
}

/// Map height / kind onto the binding terrain ramps (brief 01 §3).
///
/// Shared kind+height → colour contract for schematic reads: this is what the
/// Map View plate ([`crate::map`]'s `schematic`) fills a tile with. The world
/// render goes through the atlas instead.
#[inline]
pub fn terrain_color(kind: TerrainKind, height: i8) -> Color {
    material_of(kind).step(shade_for(kind, height))
}

/// Palette colour as straight sRGB bytes for the atlas.
#[inline]
pub fn rgba(color: Color) -> [u8; 4] {
    let s = color.to_srgba();
    [
        (s.red * 255.0).round() as u8,
        (s.green * 255.0).round() as u8,
        (s.blue * 255.0).round() as u8,
        255,
    ]
}

pub(crate) use crate::hash::world_hash;

const VARIANT_SALT: u32 = 0x7E44_A1C3;

/// Which flat variant a tile draws, so large expanses do not tile visibly.
#[inline]
pub fn variant_for(coord: TileCoord) -> usize {
    (world_hash(coord.x, coord.y, VARIANT_SALT) % VARIANTS as u32) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_depth_uses_ramp_not_flat_blue() {
        assert_eq!(terrain_color(TerrainKind::Water, -10), WATER_D);
        assert_eq!(terrain_color(TerrainKind::Water, -1), WATER_F);
        assert_ne!(
            terrain_color(TerrainKind::Water, -10),
            terrain_color(TerrainKind::Water, -1)
        );
    }

    #[test]
    fn plains_never_use_grass_light() {
        for h in 2..=5 {
            let c = terrain_color(TerrainKind::Plains, h);
            assert_ne!(c, GRASS_L);
            assert!(c == GRASS_D || c == GRASS_M);
        }
    }

    #[test]
    fn mountain_and_hills_step_with_height() {
        assert_ne!(
            terrain_color(TerrainKind::Hills, 6),
            terrain_color(TerrainKind::Hills, 10)
        );
        assert_ne!(
            terrain_color(TerrainKind::Mountain, 11),
            terrain_color(TerrainKind::Mountain, 16)
        );
    }

    #[test]
    fn flat_land_never_fills_with_a_light_step() {
        // Brief 01 §3.3: the light step is for slope-facing tiles only. Flat
        // ground that is too bright competes with track and loses the frame.
        let bands: [(TerrainKind, std::ops::RangeInclusive<i8>); 3] = [
            (TerrainKind::Beach, -1..=1),
            (TerrainKind::Plains, 0..=5),
            (TerrainKind::Hills, 5..=10),
        ];
        for (kind, range) in bands {
            for h in range {
                let shade = shade_for(kind, h);
                assert!(shade <= 1, "{kind:?} reached shade {shade} at h={h}");
            }
        }
        for h in 0..=5 {
            assert_ne!(terrain_color(TerrainKind::Plains, h), GRASS_L);
        }
        for h in 5..=10 {
            assert_ne!(terrain_color(TerrainKind::Hills, h), HILL_L);
        }
    }

    #[test]
    fn ramp_neighbours_are_visibly_distinct() {
        for material in MATERIALS {
            for shade in 0..SHADES {
                assert_ne!(
                    material.shadow(shade),
                    material.step(shade),
                    "{material:?} shade {shade} has an invisible shadow"
                );
            }
        }
    }

    #[test]
    fn material_order_is_autotile_priority() {
        for (i, m) in MATERIALS.iter().enumerate() {
            assert_eq!(m.index(), i);
        }
        assert!(Material::Water < Material::Sand);
        assert!(Material::Hill < Material::Rock);
    }

    #[test]
    fn world_hash_is_position_stable_and_well_spread() {
        // Same coordinate → same value, always (no time, no screen input).
        assert_eq!(world_hash(12, -7, 3), world_hash(12, -7, 3));
        assert_ne!(world_hash(12, -7, 3), world_hash(13, -7, 3));
        assert_ne!(world_hash(12, -7, 3), world_hash(12, -6, 3));

        // Variants spread across a field rather than banding.
        let mut counts = [0usize; VARIANTS];
        for y in 0..64i32 {
            for x in 0..64i32 {
                counts[variant_for(TileCoord { x, y })] += 1;
            }
        }
        for c in counts {
            assert!(c > 900, "variant distribution skewed: {counts:?}");
        }
    }

    #[test]
    fn water_surface_hides_the_bed_from_cliff_maths() {
        assert_eq!(surface_height(-9, true), 0);
        assert_eq!(surface_height(6, false), 6);
    }
}
