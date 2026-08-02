//! Binding world / UI palette from [`docs/design/01-art-direction.md`](../../docs/design/01-art-direction.md) §3.
//!
//! Named constants only — placeholders and final art must use these colours.
//! Diagnostic accents (`HI`, `WARN`, `OK`) are for UI / overlays / selection, never world art.
//!
//! Constants not yet referenced by presentation stay available for other Phase A owners.

#![allow(dead_code)]

use bevy::prelude::Color;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::srgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
}

// ─ Ground / structure ──────────────────────────────────
pub const BG0: Color = rgb(0x12, 0x11, 0x1a);
pub const BG1: Color = rgb(0x1b, 0x1a, 0x26);
pub const OUTLINE: Color = rgb(0x24, 0x1f, 0x2e);

// ─ Ballast (cool violet-grey) ──────────────────────────
pub const BALLAST_D: Color = rgb(0x3b, 0x35, 0x46);
pub const BALLAST_M: Color = rgb(0x57, 0x50, 0x5f);
pub const BALLAST_L: Color = rgb(0x77, 0x6e, 0x77);

// ─ Sleepers (warm brown) ───────────────────────────────
pub const TIE_D: Color = rgb(0x40, 0x29, 0x1c);
pub const TIE_M: Color = rgb(0x5c, 0x3b, 0x26);
pub const TIE_L: Color = rgb(0x7d, 0x54, 0x36);

// ─ Rail (cold steel; RAIL_S is the polished head) ───────
pub const RAIL_D: Color = rgb(0x4a, 0x4f, 0x5c);
pub const RAIL_M: Color = rgb(0x7f, 0x88, 0x99);
pub const RAIL_L: Color = rgb(0xb9, 0xc2, 0xcf);
pub const RAIL_S: Color = rgb(0xe8, 0xee, 0xf5);

// ─ Grass ───────────────────────────────────────────────
pub const GRASS_D: Color = rgb(0x2a, 0x3d, 0x24);
pub const GRASS_M: Color = rgb(0x3f, 0x5a, 0x30);
pub const GRASS_L: Color = rgb(0x5c, 0x7a, 0x3a);

// ─ Water ───────────────────────────────────────────────
pub const WATER_D: Color = rgb(0x16, 0x28, 0x3d);
pub const WATER_M: Color = rgb(0x22, 0x40, 0x5c);
pub const WATER_L: Color = rgb(0x33, 0x5b, 0x78);
/// Foam / shallows only.
pub const WATER_F: Color = rgb(0x5d, 0x8e, 0xa3);

// ─ Beach & bare earth ──────────────────────────────────
pub const SAND_D: Color = rgb(0x6b, 0x5a, 0x3e);
pub const SAND_M: Color = rgb(0x93, 0x7d, 0x55);
pub const SAND_L: Color = rgb(0xbd, 0xa8, 0x7a);

// ─ Hills (grass ramp shifted ochre) ────────────────────
pub const HILL_D: Color = rgb(0x34, 0x40, 0x1f);
pub const HILL_M: Color = rgb(0x4c, 0x5a, 0x2a);
pub const HILL_L: Color = rgb(0x6a, 0x76, 0x38);

// ─ Mountain rock (shares the ballast hue family) ───────
pub const ROCK_D: Color = rgb(0x3d, 0x39, 0x44);
pub const ROCK_M: Color = rgb(0x56, 0x52, 0x60);
pub const ROCK_L: Color = rgb(0x7b, 0x76, 0x84);
/// Only above the top elevation band.
pub const SNOW: Color = rgb(0xcf, 0xd2, 0xdd);

// ─ Buildings — plaster ─────────────────────────────────
pub const PLASTER_D: Color = rgb(0x6d, 0x5f, 0x4e);
pub const PLASTER_M: Color = rgb(0x97, 0x84, 0x6b);
pub const PLASTER_L: Color = rgb(0xc0, 0xab, 0x8c);

// ─ Buildings — timber ──────────────────────────────────
pub const WOOD_D: Color = rgb(0x3a, 0x2a, 0x1d);
pub const WOOD_M: Color = rgb(0x5a, 0x40, 0x29);
/// Same lumber as [`TIE_L`].
pub const WOOD_L: Color = TIE_L;

// ─ Roofs ───────────────────────────────────────────────
pub const ROOF_TILE_D: Color = rgb(0x4a, 0x26, 0x22);
pub const ROOF_TILE_M: Color = rgb(0x6e, 0x3a, 0x30);
pub const ROOF_TILE_L: Color = rgb(0x8f, 0x4e, 0x3e);
pub const ROOF_SLATE_D: Color = rgb(0x2c, 0x31, 0x3c);
pub const ROOF_SLATE_M: Color = rgb(0x41, 0x48, 0x55);
pub const ROOF_SLATE_L: Color = rgb(0x5b, 0x63, 0x71);

// ─ Windows ─────────────────────────────────────────────
pub const WIN_DARK: Color = rgb(0x2a, 0x2f, 0x3a);
pub const WIN_LIT: Color = rgb(0xf2, 0xd9, 0x8a);

// ─ Diagnostic only — NEVER in world art ────────────────
pub const HI: Color = rgb(0xf2, 0xc1, 0x4e);
pub const WARN: Color = rgb(0xe8, 0x62, 0x4a);
pub const OK: Color = rgb(0x6f, 0xd0, 0x8c);

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::Color;

    fn channels(c: Color) -> (u8, u8, u8) {
        let s = c.to_srgba();
        (
            (s.red * 255.0).round() as u8,
            (s.green * 255.0).round() as u8,
            (s.blue * 255.0).round() as u8,
        )
    }

    #[test]
    fn spine_hex_values_match_brief() {
        assert_eq!(channels(BG0), (0x12, 0x11, 0x1a));
        assert_eq!(channels(GRASS_D), (0x2a, 0x3d, 0x24));
        assert_eq!(channels(GRASS_M), (0x3f, 0x5a, 0x30));
        assert_eq!(channels(WATER_D), (0x16, 0x28, 0x3d));
        assert_eq!(channels(WATER_M), (0x22, 0x40, 0x5c));
        assert_eq!(channels(WATER_L), (0x33, 0x5b, 0x78));
        assert_eq!(channels(SAND_D), (0x6b, 0x5a, 0x3e));
        assert_eq!(channels(HILL_M), (0x4c, 0x5a, 0x2a));
        assert_eq!(channels(ROCK_D), (0x3d, 0x39, 0x44));
        assert_eq!(channels(SNOW), (0xcf, 0xd2, 0xdd));
        assert_eq!(channels(RAIL_S), (0xe8, 0xee, 0xf5));
    }

    #[test]
    fn wood_l_matches_tie_l() {
        assert_eq!(channels(WOOD_L), channels(TIE_L));
    }
}
