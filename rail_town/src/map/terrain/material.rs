//! Terrain materials, ramps, and world-anchored variant selection.
//!
//! Materials are ordered low → high and the index doubles as **autotile
//! priority**: a higher material always laps onto a lower one, so every
//! boundary in [01 §6.2] is drawn exactly once, on the low tile.
//!
//! Every colour comes from [`crate::palette`] — nothing here mixes, blends or
//! tints. Ramps are four steps: dark · mid · light · cap, where the cap is the
//! material's reserved extreme (`WATER_F` foam, `SNOW`) and simply repeats the
//! light step for materials that have none.
//!
//! **A cap is never a flat fill.** [`shade_for`] cannot return one; a cap is
//! reachable only through [`Material::light_mark`], which is to say on drawn
//! crests and lips. Brief 01 §3.2 reserves both colours, and snow spread flat
//! over every peak was the brightest thing in the game on 5.5% of the map.

use bevy::prelude::Color;
use rail_map::TerrainKind;
use rail_sim::ids::TileCoord;

use crate::palette::{
    GRASS_D, GRASS_L, GRASS_M, HILL_D, HILL_L, HILL_M, OUTLINE, ROCK_D, ROCK_L, ROCK_M, SAND_D,
    SAND_L, SAND_M, SNOW, WATER_D, WATER_F, WATER_L, WATER_M,
};

/// Steps per material ramp: dark · mid · light · cap.
pub const SHADES: usize = 4;
/// Steps a flat tile may fill with — the ramp minus its reserved extreme.
///
/// A cap is a drawn-edge colour, so [`shade_for`] cannot return one *and* the
/// atlas holds no base cell for it. A field of `SNOW` is not a bug waiting to be
/// reintroduced; it is a tile that cannot be addressed (brief 01 §3.2).
pub const FILL_SHADES: usize = SHADES - 1;
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

    /// The nearest visibly lighter step, or `None` at the top of the ramp.
    ///
    /// Deliberately not the mirror of [`Self::shadow`]. At the bottom there is a
    /// colour to borrow — [`OUTLINE`], the game's one shadow key — but above a
    /// ramp's cap there is nothing that still belongs to the material, and
    /// borrowing another ramp's light step would break material identity at 1×.
    ///
    /// So the top of a ramp returns `None` and the caller decides. It used to
    /// return the base colour, which meant every light mark on a capped tile was
    /// painted in the fill colour: six invisible rects per snow tile and five
    /// per shallow-water tile, costing texels and drawing nothing.
    #[inline]
    pub fn light_mark(self, shade: usize) -> Option<Color> {
        let ramp = self.ramp();
        let base = self.step(shade);
        ramp[(shade.min(SHADES - 1) + 1).min(SHADES)..]
            .iter()
            .find(|step| **step != base)
            .copied()
    }

    /// The material's reserved extreme, if it has one.
    ///
    /// `SNOW` and `WATER_F` exist for *drawn* marks — the crest of a wall, the
    /// lip of a coastline, the lap of foam over a shallow. Materials whose cap
    /// merely repeats their light step have no reserved colour at all.
    #[inline]
    pub fn reserved_cap(self) -> Option<Color> {
        let ramp = self.ramp();
        (ramp[SHADES - 1] != ramp[SHADES - 2]).then_some(ramp[SHADES - 1])
    }

    /// The light mark a *flat* tile's texture may spend.
    ///
    /// [`Self::light_mark`] minus the reserved cap. Speckle covers roughly one
    /// texel in eighty of every tile of a material, so letting it reach the cap
    /// would put `SNOW` across every mountain field and `WATER_F` across every
    /// shallow — quietly, and everywhere. Brief 01 §3.2 reserves them, and the
    /// lit crest only means "track cannot climb this" if nothing else is white.
    #[inline]
    pub fn texture_mark(self, shade: usize) -> Option<Color> {
        self.light_mark(shade)
            .filter(|light| Some(*light) != self.reserved_cap())
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

/// Ramp step for a tile's elevation — the terrain value ladder.
///
/// # The ladder
///
/// `rail_map` bands elevation at heights 0 · 4 · 7 · 10 · 13 · 16 and hands the
/// renderer one material per band: grass, grass, hill, hill, rock, rock. Brief
/// 02 §2.3 asks that a player "trace the cheapest route across a map with their
/// finger", and **value** is the only channel that survives being read at a
/// glance. Hue says what the ground *is*; lightness has to say how *high* it is.
/// So the six realised fills climb:
///
/// | Band | Height | Fill | L\* |
/// | --- | --- | --- | --- |
/// | 0 | 0 | `GRASS_D` | 23.5 |
/// | 1 | 4 | `GRASS_M` | 35.2 |
/// | 2 | 7 | `HILL_M` | 36.0 |
/// | 3 | 10 | `HILL_L` | 47.4 |
/// | 4 | 13 | `ROCK_L` | 50.5 |
/// | 5 | 16 | `ROCK_L` | 50.5 — the wall; see below |
///
/// It used to run 23.5 · 35.2 · 25.2 · 36.0 · 35.7 · 84.3 — light, dark, light,
/// dark, white — with bands 1, 3 and 4 inside 0.8 L\* of one another. Height was
/// not readable as value at all, which turns every ridge into an invisible tax.
///
/// # Two trades, both taken deliberately
///
/// **Bands 3 and 4 fill with their material's light step**, which brief 01 §3.3
/// rule 1 reserves for slope-facing tiles. The two rules cannot both hold: the
/// grass, hill and rock ramps land on the same three lightness tiers (≈24 · ≈36
/// · ≈48), so six strictly climbing rungs cannot be cut from two steps of three
/// ramps — the "bottom two-thirds" set *is* the broken ladder above. §2.3 is the
/// load-bearing contract and wins. §3.3's purpose survives regardless: the
/// widest value gap in the palette, `RAIL_S` on `BALLAST_D` (23.4 → 93.8), sits
/// wholly *inside* the track sprite, so track carries its own contrast onto any
/// ground it crosses. See `band_ladder_climbs_in_luminance` for the measured
/// separation against ballast, buildings and station markers.
///
/// **Bands 4 and 5 share a fill.** Rock has exactly one step above `HILL_L`, and
/// its cap is `SNOW` — L\* 84.3, brighter than the `hi` accent, and §3.2 reserves
/// it. So the ladder ends at `ROCK_L` and the wall is told apart by its *edge*
/// instead: an impassable face draws a full banded cliff with a `SNOW` crest
/// ([`super::autotile::resolve_tile`], [`super::atlas`]). Height is the fill;
/// legality is the crest. Two channels, two meanings, neither overloaded.
///
/// # Water
///
/// Depth banding is what stops open water being one flat blue field (brief 01
/// §6.2.3). Inland water clamps at depth 3, so the three inland depths take the
/// three real steps of the ramp — shallow is light, deep is dark — and the
/// reserved `WATER_F` is left to the foam and glint decals that draw *over* the
/// terrain (`atmosphere::water`). Sea reaches depth 6 and stays on the dark step
/// it shares with the inland floor, so the ramp never doubles back.
#[inline]
pub fn shade_for(kind: TerrainKind, height: i8) -> usize {
    match kind {
        // Depth 3 is `INLAND_DEPTH_MAX`; 4..6 is open sea, already at the floor.
        TerrainKind::Water => match height {
            ..=-3 => 0,
            -2 => 1,
            _ => 2,
        },
        // Beach is a shoreline rather than an elevation band: every one of them
        // sits at band 0, and the coast is drawn as a line (brief 01 §6.2.1).
        TerrainKind::Beach => usize::from(height > 0),
        TerrainKind::Plains => usize::from(height > 3),
        TerrainKind::Hills => 1 + usize::from(height > 8),
        // Both mountain bands, buildable (13) and wall (≥ `MOUNTAIN_HEIGHT_MIN`),
        // fill with `ROCK_L`. Never shade 3: that is `SNOW`.
        TerrainKind::Mountain => 2,
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

/// CIE L\* lightness of a colour, 0 (black) to 100 (white).
///
/// The band ladder's contract is about value, so it has to be *measured* rather
/// than eyeballed: three of the six bands once landed within 0.8 L\* of each
/// other while looking like plainly different colours. Perceptual lightness is
/// what a player reads at a glance, so it is what the tests assert on.
#[cfg(test)]
pub(crate) fn lightness(color: Color) -> f32 {
    let s = color.to_srgba();
    let linear = |c: f32| {
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let y = 0.2126 * linear(s.red) + 0.7152 * linear(s.green) + 0.0722 * linear(s.blue);
    if y > 0.008_856 {
        116.0 * y.cbrt() - 16.0
    } else {
        903.3 * y
    }
}

/// L\* of an atlas texel, for tests that measure composited art.
#[cfg(test)]
pub(crate) fn texel_lightness(px: [u8; 4]) -> f32 {
    lightness(Color::srgb(
        px[0] as f32 / 255.0,
        px[1] as f32 / 255.0,
        px[2] as f32 / 255.0,
    ))
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

    /// The six heights `rail_map` bands elevation at, and the kind it gives each
    /// (`gen.rs`: bands 0–1 plains, 2–3 hills, 4–5 mountain). Mirrored rather
    /// than imported — `BAND_HEIGHTS` is private to the generator.
    const LADDER: [(TerrainKind, i8); 6] = [
        (TerrainKind::Plains, 0),
        (TerrainKind::Plains, 4),
        (TerrainKind::Hills, 7),
        (TerrainKind::Hills, 10),
        (TerrainKind::Mountain, 13),
        (TerrainKind::Mountain, 16),
    ];

    #[test]
    fn water_depth_uses_ramp_not_flat_blue() {
        assert_eq!(terrain_color(TerrainKind::Water, -6), WATER_D);
        assert_eq!(terrain_color(TerrainKind::Water, -1), WATER_L);
        assert_ne!(
            terrain_color(TerrainKind::Water, -6),
            terrain_color(TerrainKind::Water, -1)
        );
    }

    #[test]
    fn inland_water_reads_as_three_depths_and_never_as_foam() {
        // Inland water clamps at depth 3 (`INLAND_DEPTH_MAX`), and most maps are
        // landlocked, so these three tiles are what a river actually *is*. They
        // used to be two colours, one of which was `WATER_F` — the reserved foam
        // step (brief 01 §3.2) — so every river was near enough one flat band
        // with the foam colour spread across its middle.
        let inland: Vec<[u8; 4]> = (1..=3)
            .map(|depth| rgba(terrain_color(TerrainKind::Water, -depth)))
            .collect();
        assert_eq!(
            inland
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            3,
            "the three inland depths must realise three colours: {inland:?}"
        );
        for c in &inland {
            assert_ne!(
                *c,
                rgba(WATER_F),
                "foam is drawn over water, never as water"
            );
        }
        for pair in inland.windows(2) {
            assert!(
                texel_lightness(pair[1]) < texel_lightness(pair[0]),
                "deeper water must be darker: {pair:?}"
            );
        }
        // Sea keeps going down to depth 6 and never doubles back up the ramp.
        for depth in 4..=6 {
            assert_eq!(rgba(terrain_color(TerrainKind::Water, -depth)), inland[2]);
        }
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
    fn hills_step_with_height_and_rock_tops_out() {
        assert_ne!(
            terrain_color(TerrainKind::Hills, 7),
            terrain_color(TerrainKind::Hills, 10)
        );
        // Both mountain bands share `ROCK_L`; what separates the wall from the
        // buildable band is its crest, not its fill. See `autotile`.
        assert_eq!(terrain_color(TerrainKind::Mountain, 13), ROCK_L);
        assert_eq!(terrain_color(TerrainKind::Mountain, 16), ROCK_L);
    }

    #[test]
    fn band_ladder_climbs_in_luminance() {
        // Brief 02 §2.3: a player traces the cheapest route with a finger before
        // laying track. That only works if height reads as value. The ladder used
        // to measure 23.5 · 35.2 · 25.2 · 36.0 · 35.7 · 84.3 — light, dark, light,
        // dark, white, with three bands inside 0.8 L* of each other.
        let ladder: Vec<f32> = LADDER
            .iter()
            .map(|&(kind, h)| lightness(terrain_color(kind, h)))
            .collect();

        // Bands 0..4 are everything track can be built on, and they climb.
        for band in 1..5 {
            assert!(
                ladder[band] > ladder[band - 1],
                "band {band} is not above band {}: {ladder:?}",
                band - 1
            );
        }
        // Band 5 is the wall. It may not sink below the ground it towers over,
        // and it cannot climb further without `SNOW` — so it ties, and the drawn
        // crest carries the difference (`atlas::paint_cliff`).
        assert!(ladder[5] >= ladder[4], "the wall must not read as lower");

        // Nothing on the ground may out-shout the track laid across it. The
        // brightest terrain fill is `ROCK_L` at L* 50.5; the railhead is L* 93.8
        // and the polished-head gap lives inside the track sprite, so the widest
        // value gap in the palette is never terrain's to spend (brief 01 §3.3).
        for l in &ladder {
            assert!(*l < 55.0, "a terrain fill is competing with track: {l}");
        }

        // Separation from what sits *on* the terrain, measured rather than hoped.
        // Everything the player must pick out of the landscape — the railhead
        // they trace, a lit window, a build ghost or a station marker in `hi` —
        // clears the brightest ground by a wide margin. Snow did not: `hi` on
        // `SNOW` was 80.5 against 84.3, an accent invisible on 5.5% of the map.
        use crate::palette::{BALLAST_D, HI, PLASTER_L, RAIL_S, ROOF_SLATE_M, WIN_LIT};
        let brightest = ladder.iter().cloned().fold(f32::MIN, f32::max);
        for (name, marker) in [
            ("railhead", RAIL_S),
            ("station plaster", PLASTER_L),
            ("lit window", WIN_LIT),
            ("hi accent", HI),
        ] {
            assert!(
                lightness(marker) - brightest > 15.0,
                "{name} is swallowed by the brightest ground ({} vs {brightest})",
                lightness(marker)
            );
        }
        // Dark objects work the other way: a ballast bed and a slate roof read
        // as silhouettes, so they must stay below every fill they can sit on.
        // Band 0 is the tight one — `BALLAST_D` 23.4 against `GRASS_D` 23.5 is a
        // hue read, not a value read — but the bed is never alone on the tile,
        // and the gap it does own, `RAIL_S` over `BALLAST_D`, is the widest in
        // the palette and travels with the sprite (brief 01 §3.3 rule 2).
        for (name, marker) in [("ballast bed", BALLAST_D), ("slate roof", ROOF_SLATE_M)] {
            assert!(
                lightness(marker) <= ladder[0] + 8.0,
                "{name} has drifted up into the ground it is drawn on"
            );
        }
        assert!(
            lightness(RAIL_S) - lightness(BALLAST_D) > 2.0 * (brightest - ladder[0]),
            "track's own value gap must outrun the whole terrain ladder"
        );
    }

    #[test]
    fn flat_land_never_fills_with_a_reserved_cap() {
        // Brief 01 §3.2 reserves two colours: `SNOW`, and `WATER_F` for "foam /
        // shallows only". Neither is a fill — they are reached through
        // `light_mark`, on drawn crests and lips. Mountain is included here
        // deliberately: it is the case this test used to omit, and snow-filled
        // peaks were the brightest thing in the game across 5.5% of the map.
        let bands: [(TerrainKind, std::ops::RangeInclusive<i8>); 5] = [
            (TerrainKind::Water, -8..=0),
            (TerrainKind::Beach, -1..=1),
            (TerrainKind::Plains, 0..=5),
            (TerrainKind::Hills, 5..=10),
            (TerrainKind::Mountain, 11..=32),
        ];
        for (kind, range) in bands {
            for h in range {
                let shade = shade_for(kind, h);
                assert!(
                    shade < SHADES - 1,
                    "{kind:?} filled with its reserved cap at h={h}"
                );
                let c = terrain_color(kind, h);
                assert_ne!(c, SNOW, "{kind:?} filled with snow at h={h}");
                assert_ne!(c, WATER_F, "{kind:?} filled with foam at h={h}");
            }
        }
        // Where §3.3's "flat ground never uses the light step" still holds, it
        // holds strictly. Bands 3–5 bend it; see `shade_for` for the trade.
        for h in 0..=5 {
            assert_ne!(terrain_color(TerrainKind::Plains, h), GRASS_L);
        }
        for h in -1..=1 {
            assert_ne!(terrain_color(TerrainKind::Beach, h), SAND_L);
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
    fn a_light_mark_is_never_the_colour_it_is_painted_on() {
        // The bug this replaces: `highlight` returned the base colour at the top
        // of a ramp, so the painters stamped light marks in the fill colour —
        // invisible rects, paid for and never seen. `None` now says so, and the
        // painters fall back or skip (`atlas::paint_base`, `atlas::paint_sun_lip`).
        for material in MATERIALS {
            for shade in 0..SHADES {
                let base = material.step(shade);
                match material.light_mark(shade) {
                    Some(light) => assert_ne!(
                        light, base,
                        "{material:?} shade {shade} paints an invisible light mark"
                    ),
                    // Only the top of a ramp may have nowhere to go, and every
                    // material's cap is its own last step.
                    None => assert_eq!(
                        base,
                        material.step(SHADES - 1),
                        "{material:?} shade {shade} lost its light step mid-ramp"
                    ),
                }
            }
        }
        // The two reserved extremes exist purely as light marks.
        assert_eq!(Material::Rock.light_mark(2), Some(SNOW));
        assert_eq!(Material::Water.light_mark(2), Some(WATER_F));
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
