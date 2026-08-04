//! Desire paths, drawn — the art contract both renderers share.
//!
//! The sim decides *where* the town has walked (`rail_sim::peeps::wear`); this
//! module decides what that looks like, once, so the chunk compositor and the
//! diamond atlas cannot disagree about it. Brief 16 §3.
//!
//! # The rule that shapes everything here
//!
//! Brief 01 §3.3 and [02 §2.3] make **lightness carry elevation** — the realised
//! band ladder climbs 23.5 · 35.2 · 36.0 · 47.4 L\*, and a player traces a cheap
//! route across a map by reading value. A bare-earth path painted the obvious
//! way, a dusty ochre on dark grass, lands at 39 L\* on ground that fills at
//! 23.5: a band and a half of *apparent elevation*, painted onto flat ground, by
//! a feature that has nothing to do with height. It would put a phantom ridge
//! along every lane in town.
//!
//! > **A path may shift hue as far as it likes. Its value must stay inside
//! > [`MAX_PATH_VALUE_SHIFT`] of the ground it lies on.**
//!
//! The palette makes that mostly easy. The `TIE` ramp is a warm twin of the
//! grass and hill ramps rung for rung, so the path ramp is a pure function of
//! the ground's own fill shade — no material table, no special cases:
//!
//! | Ground fill | Band | L\* | Path fill | L\* | Δ |
//! | --- | --- | --- | --- | --- | --- |
//! | `GRASS_D` | 0 | 23.5 | `TIE_M` | 28.2 | +4.7 |
//! | `GRASS_M` | 1 | 35.2 | `TIE_L` | 39.5 | +4.3 |
//! | `HILL_M` | 2 | 36.0 | `TIE_L` | 39.5 | +3.5 |
//! | `HILL_L` | 3 | 47.4 | `SAND_M` | 53.5 | +6.1 |
//!
//! The last rung is the tight one and is worth recording rather than hiding:
//! **the palette holds no warm tone within 6 L\* of `HILL_L`.** `SAND_M` at
//! +6.1 is the closest there is, which is a little over half a band step and
//! under two-thirds of one. It is accepted for three reasons: the high hill band
//! is a minority of a calm map; a peep pays [`WALK_CLIMB_COST`] to climb, so
//! routes prefer the flat and paths are rare up there anyway; and the hue swing
//! green → sand is far too large for the mark to read as anything but a change
//! of material. The two bands that carry most of every map are inside 5 L\*.
//!
//! `a_path_never_climbs_the_elevation_ladder` measures all of it rather than
//! trusting this table.
//!
//! [`WALK_CLIMB_COST`]: rail_sim::peeps::WALK_CLIMB_COST

use bevy::prelude::*;

use crate::hash::world_hash;
use crate::palette::{SAND_L, SAND_M, TIE_L, TIE_M};

use super::terrain::material::{Material, FILL_SHADES};

/// The path ramp: bare earth at the value of the ground it lies on.
pub const PATH_FILL: [Color; FILL_SHADES] = [TIE_M, TIE_L, SAND_M];

/// The sparse light speckle on dry, well-trodden earth — one rung up.
pub const PATH_DUST: [Color; FILL_SHADES] = [TIE_L, SAND_M, SAND_L];

/// Mask variants per level, world-hashed so a long lane does not repeat.
pub const PATH_VARIANTS: usize = 4;

/// The most a path may shift the value of the ground it lies on, in L\*.
///
/// The value ladder spends about 11.5 L\* on a legible elevation band, so this
/// is under two-thirds of a band: enough headroom for the palette's warm tones,
/// never enough to promote a tile a whole band and invent a ridge.
///
/// Spent by `a_path_never_climbs_the_elevation_ladder` rather than by any code
/// path — like the palette constants themselves, this is a rule the art has to
/// keep, and the test is where it is kept.
#[allow(dead_code)]
pub const MAX_PATH_VALUE_SHIFT: f32 = 7.0;

/// The tighter bound on the two bands that carry most of every map — grass at
/// sea level and grass at the first step up.
#[allow(dead_code)]
pub const MAX_PATH_VALUE_SHIFT_LOWLAND: f32 = 5.0;

/// Coverage per wear level, in permille, at the **centre** of a tile.
///
/// Level 0 is clean ground and is never drawn — not a faint tint, not an alpha
/// wash, nothing. Coverage over the whole tile is lower than these, because
/// [`EDGE_FALLOFF`] thins the rim; `each_level_covers_more_ground_than_the_last`
/// measures what actually lands.
pub const PATH_COVERAGE: [u32; 4] = [0, 400, 800, 1000];

/// How much less likely a texel is to be worn at the tile's rim than at its
/// centre, in permille.
///
/// Strong enough to matter. At the first pass this was a gentle *radial*
/// falloff, and the picture showed why that is wrong: the norm has to be the
/// shape of the tile. A 64 x 32 diamond only fills half its bounding box, so a
/// radial term barely bit inside one at all and a run of Bare tiles read as a
/// chain of hard-edged diamonds — tiles, not a path. Measured on a tile-shaped
/// norm (see [`path_mark`]) the rim now genuinely thins, and the lane keeps the
/// ragged edge that is the whole of its softness.
const EDGE_FALLOFF: u32 = 350;

/// Share of worn texels that take the dust mark, in permille. Matches the grain
/// density the terrain's own art uses, so a path is speckled like ground.
const DUST_PERMILLE: u32 = 90;

/// Dust appears only on earth dry enough to raise any — Worn and Bare.
const DUST_FROM_LEVEL: u8 = 2;

const PATH_VARIANT_SALT: u32 = 0x2D19_B7A5;
const PATH_MASK_SALT: u32 = 0x74C3_11EF;
const PATH_DUST_SALT: u32 = 0x1AF0_9C63;

/// What one texel of a path cell draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathMark {
    /// Bare earth.
    Fill,
    /// A lighter fleck of dry dust.
    Dust,
}

/// Which mask variant a tile draws. World-anchored, so the mark belongs to the
/// ground rather than to the screen (brief 01 §6.2.3).
#[inline]
pub fn path_variant_for(coord: rail_sim::ids::TileCoord) -> usize {
    (world_hash(coord.x, coord.y, PATH_VARIANT_SALT) % PATH_VARIANTS as u32) as usize
}

/// The path tones for ground of a given material and fill shade, or `None`
/// where a path cannot be drawn at all.
///
/// Only **grass and hills** wear. Beach sand is already bare earth and there is
/// nothing to wear barer; the mountain band is impassable on foot so no footfall
/// can land there; water is impassable except on a bridge deck, which is a built
/// structure and not ground (brief 16 §3.2).
#[inline]
pub fn path_tones(material: Material, shade: usize) -> Option<(Color, Color)> {
    match material {
        Material::Grass | Material::Hill => {
            let shade = shade.min(FILL_SHADES - 1);
            Some((PATH_FILL[shade], PATH_DUST[shade]))
        }
        Material::Water | Material::Sand | Material::Rock => None,
    }
}

/// Whether the texel at `(u, v)` of a `w` x `h` tile is worn, and how.
///
/// `(u, v)` is a position inside the tile's own art in whatever geometry the
/// caller draws — a 32 x 32 square from above, a 64 x 32 diamond in isometric.
/// Both hand the same normalised position to the same hash, so one lane looks
/// like the same lane in either projection.
///
/// The mask is a **scatter**, not a region: the boundary between path and grass
/// is ragged by construction, which is the pixel-art way to draw a soft edge
/// without the alpha ramp brief 01 §2 forbids. Every texel is either grass or
/// earth; the softness lives in the distribution.
pub fn path_mark(level: u8, variant: usize, u: u32, v: u32, w: u32, h: u32) -> Option<PathMark> {
    let coverage = *PATH_COVERAGE.get(level as usize)?;
    if coverage == 0 || w == 0 || h == 0 {
        return None;
    }

    // Normalised offset from the tile's centre, +/-1000 at the bounding edges.
    let dx = (2 * u as i64 - w as i64) * 1000 / w as i64;
    let dy = (2 * v as i64 - h as i64) * 1000 / h as i64;
    // Manhattan, not Euclidean: `|dx| + |dy| = 1000` is exactly the rim of a
    // 2:1 diamond, and the edge midpoint of a square. One norm, both geometries,
    // and in each of them the falloff reaches full strength at the tile's own
    // boundary rather than at the corner of a box the tile does not fill.
    let m = (dx.abs() + dy.abs()).min(1000) as u32;
    let threshold = coverage * (1000 - EDGE_FALLOFF * m / 1000) / 1000;

    // Variants must not be near neighbours in the hash, or two of them differ
    // by a handful of texels and the repeat is still visible — the same lesson
    // the isometric fill grain learned.
    let salt = PATH_MASK_SALT.wrapping_add((variant as u32).wrapping_mul(0x9E37_79B9));
    if world_hash(u as i32, v as i32, salt) % 1000 >= threshold {
        return None;
    }
    if level >= DUST_FROM_LEVEL
        && world_hash(u as i32, v as i32, PATH_DUST_SALT) % 1000 < DUST_PERMILLE
    {
        return Some(PathMark::Dust);
    }
    Some(PathMark::Fill)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::terrain::material::{rgba, shade_for, texel_lightness, BAND_STEP};
    use rail_map::TerrainKind;
    use rail_sim::ids::TileCoord;

    fn lightness(color: Color) -> f32 {
        texel_lightness(rgba(color))
    }

    /// The load-bearing one. Value carries elevation; a path must not.
    #[test]
    fn a_path_never_climbs_the_elevation_ladder() {
        // Every walkable band the generator actually produces: grass at 0 and
        // 4, hills at 7 and 10. (14 and up is the mountain band, which is
        // impassable on foot, and beach is already bare.)
        let ladder = [
            (TerrainKind::Plains, 0i8, Material::Grass),
            (TerrainKind::Plains, 4, Material::Grass),
            (TerrainKind::Hills, 7, Material::Hill),
            (TerrainKind::Hills, 10, Material::Hill),
        ];

        for (band, (kind, height, material)) in ladder.into_iter().enumerate() {
            let shade = shade_for(kind, height);
            let ground = lightness(material.step(shade));
            let (fill, _) = path_tones(material, shade).expect("grass and hills wear");
            let delta = lightness(fill) - ground;

            assert!(
                delta.abs() <= MAX_PATH_VALUE_SHIFT,
                "a path on {kind:?} at height {height} shifts value by {delta:.1} L*, \
                 which is enough to read as a change of elevation"
            );
            // The lowland bands are most of most maps, and are held tighter.
            if band < 2 {
                assert!(
                    delta.abs() <= MAX_PATH_VALUE_SHIFT_LOWLAND,
                    "a path on the lowland band {band} shifts value by {delta:.1} L*"
                );
            }
            // …and it is a real, visible mark rather than a tone the eye cannot
            // separate from the grass.
            assert!(
                delta > 1.0,
                "a path on {kind:?} at height {height} is invisible ({delta:.1} L*)"
            );
        }
        assert_eq!(BAND_STEP, 3, "the band spacing this bound was measured at");
    }

    /// The bound is only meaningful next to the ladder it is protecting.
    #[test]
    fn the_bound_is_a_fraction_of_a_real_band_step() {
        // The step the value ladder actually spends on a legible band, at the
        // rung where value is doing the work: grass at sea level to grass one
        // step up.
        let step = lightness(Material::Grass.step(shade_for(TerrainKind::Plains, 4)))
            - lightness(Material::Grass.step(shade_for(TerrainKind::Plains, 0)));
        assert!(step > 11.0, "the band ladder no longer climbs as measured: {step:.1}");
        assert!(
            MAX_PATH_VALUE_SHIFT < step * 0.67,
            "the path bound {MAX_PATH_VALUE_SHIFT} is no longer a fraction of a band {step:.1}"
        );
    }

    #[test]
    fn only_ground_with_grass_on_it_wears() {
        for shade in 0..FILL_SHADES {
            assert!(path_tones(Material::Grass, shade).is_some());
            assert!(path_tones(Material::Hill, shade).is_some());
            assert!(
                path_tones(Material::Sand, shade).is_none(),
                "beach is already bare earth"
            );
            assert!(path_tones(Material::Rock, shade).is_none());
            assert!(path_tones(Material::Water, shade).is_none());
        }
    }

    #[test]
    fn the_path_ramp_climbs_with_the_ground_it_lies_on() {
        for shade in 1..FILL_SHADES {
            assert!(
                lightness(PATH_FILL[shade]) > lightness(PATH_FILL[shade - 1]),
                "the path ramp must climb with the band ladder"
            );
            assert!(
                lightness(PATH_DUST[shade]) > lightness(PATH_FILL[shade]),
                "dust is a light mark, not a dark one"
            );
        }
    }

    /// Measured coverage per level, over a whole tile, averaged across variants.
    fn coverage_permille(level: u8, w: u32, h: u32) -> u32 {
        let mut worn = 0u32;
        let mut total = 0u32;
        for variant in 0..PATH_VARIANTS {
            for v in 0..h {
                for u in 0..w {
                    total += 1;
                    if path_mark(level, variant, u, v, w, h).is_some() {
                        worn += 1;
                    }
                }
            }
        }
        worn * 1000 / total
    }

    #[test]
    fn each_level_covers_more_ground_than_the_last() {
        let clean = coverage_permille(0, 32, 32);
        let faint = coverage_permille(1, 32, 32);
        let worn = coverage_permille(2, 32, 32);
        let bare = coverage_permille(3, 32, 32);

        assert_eq!(clean, 0, "clean ground draws nothing at all");
        assert!(faint > 240 && faint < 360, "faint coverage was {faint}permille");
        assert!(worn > 540 && worn < 660, "worn coverage was {worn}permille");
        assert!(bare > 700 && bare < 820, "bare coverage was {bare}permille");
        assert!(clean < faint && faint < worn && worn < bare);
        // Even at its deepest a path is trodden earth with grass clinging on,
        // never a repainted tile — that is what keeps it ground and not a road.
        assert!(bare < 850, "bare covered the tile: {bare}permille");
    }

    #[test]
    fn the_mask_is_the_same_lane_in_both_geometries() {
        // A 32 x 32 square and a 64 x 32 diamond hand the same normalised
        // position to the same hash, so the two views agree about the ground.
        for level in 1..4u8 {
            let flat = coverage_permille(level, 32, 32);
            let iso = coverage_permille(level, 64, 32);
            let gap = flat.abs_diff(iso);
            assert!(
                gap < 60,
                "level {level} covers {flat}permille from above but {iso}permille \
                 in isometric"
            );
        }
    }

    #[test]
    fn a_worn_tile_thins_out_toward_its_corners() {
        // Sampled at Worn, where the falloff is meant to show.
        let mut centre = 0u32;
        let mut corner = 0u32;
        for variant in 0..PATH_VARIANTS {
            for v in 12..20 {
                for u in 12..20 {
                    if path_mark(2, variant, u, v, 32, 32).is_some() {
                        centre += 1;
                    }
                }
            }
            for (u0, v0) in [(0, 0), (24, 0), (0, 24), (24, 24)] {
                for v in v0..v0 + 8 {
                    for u in u0..u0 + 8 {
                        if path_mark(2, variant, u, v, 32, 32).is_some() {
                            corner += 1;
                        }
                    }
                }
            }
        }
        // Four corner blocks against one centre block.
        assert!(
            corner / 4 < centre,
            "the edge does not thin out: centre {centre}, corners {}",
            corner / 4
        );
    }

    #[test]
    fn dust_is_sparse_and_only_on_well_trodden_earth() {
        for level in 1..4u8 {
            let mut dust = 0u32;
            let mut worn = 0u32;
            for variant in 0..PATH_VARIANTS {
                for v in 0..32 {
                    for u in 0..32 {
                        match path_mark(level, variant, u, v, 32, 32) {
                            Some(PathMark::Dust) => {
                                dust += 1;
                                worn += 1;
                            }
                            Some(PathMark::Fill) => worn += 1,
                            None => {}
                        }
                    }
                }
            }
            if level < DUST_FROM_LEVEL {
                assert_eq!(dust, 0, "a faint scuff has no dry dust on it");
            } else {
                assert!(dust > 0, "level {level} raised no dust at all");
                assert!(
                    dust * 1000 / worn < 150,
                    "level {level} is {} permille dust — that is a texture, not a fleck",
                    dust * 1000 / worn
                );
            }
        }
    }

    #[test]
    fn the_mask_is_stable_and_the_variants_differ() {
        assert_eq!(path_mark(2, 0, 7, 9, 32, 32), path_mark(2, 0, 7, 9, 32, 32));
        let differ = (0..32)
            .flat_map(|u| (0..32).map(move |v| (u, v)))
            .filter(|&(u, v)| path_mark(2, 0, u, v, 32, 32) != path_mark(2, 1, u, v, 32, 32))
            .count();
        assert!(differ > 100, "two variants share {differ} texels of difference");
    }

    #[test]
    fn variants_are_world_anchored() {
        let a = path_variant_for(TileCoord { x: 12, y: -7 });
        assert_eq!(a, path_variant_for(TileCoord { x: 12, y: -7 }));
        assert!(a < PATH_VARIANTS);
        let spread: std::collections::HashSet<usize> = (0..64)
            .map(|i| path_variant_for(TileCoord { x: i, y: i * 3 }))
            .collect();
        assert!(spread.len() >= 3, "the variant hash barely spreads: {spread:?}");
    }
}
