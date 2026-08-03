//! Procedural terrain tile atlas, painted once at startup.
//!
//! There is no artist yet, so the tile art is generated — but it is generated at
//! the real dimensions (32 × 32 texels) and in the real palette, so swapping in
//! drawn art is a texture change and never a layout change (brief 01 §7).
//!
//! The atlas is a CPU-side pixel buffer, not a GPU texture: chunks composite
//! from it on the CPU and only the finished chunks are uploaded (§2.5).
//!
//! # Cell layout
//!
//! | Range | Contents |
//! | --- | --- |
//! | `0..45` | base — 5 materials × 3 fill steps × 3 world-hashed variants |
//! | `45..125` | transitions — 4 boundaries × (16 edge masks + 4 inner corners) |
//! | `125..133` | cliff faces — 4 directions × 2 severities |
//! | `133..137` | cliff corners — diagonal-only drops |
//! | `137..141` | terrace contours — the shadow at the foot of a band step |
//! | `141..171` | sun lips — 5 materials × 3 fill steps × the two sunlit edges |
//!
//! Ramps are four steps but only three of them are *fills*: a material's
//! reserved cap (`SNOW`, `WATER_F`) has no base cell and no lip cell of its own.
//! It is reachable only as the light mark on the step below it.

use bevy::prelude::Resource;
use rail_map::TILE_SIZE;

use crate::palette::{OUTLINE, ROCK_D, ROCK_M, SNOW, WATER_F};

use super::material::{
    rgba, world_hash, Material, BOUNDARY_COUNT, FILL_SHADES, MATERIALS, MATERIAL_COUNT, VARIANTS,
};

/// Texels per tile edge — the pixel contract's source tile size (brief 01 §2.1).
pub const CELL: u32 = 32;
/// Cells per atlas row.
const ATLAS_COLS: u32 = 16;

/// Transition pieces per boundary: 16 edge masks + 4 inner corners.
pub const TRANSITION_PIECES: usize = 20;
/// First piece index that is an inner corner rather than an edge mask.
pub const TRANSITION_CORNER: usize = 16;

pub const BASE_CELLS: usize = MATERIAL_COUNT * FILL_SHADES * VARIANTS;
pub const TRANSITION_BASE: usize = BASE_CELLS;
pub const CLIFF_BASE: usize = TRANSITION_BASE + BOUNDARY_COUNT * TRANSITION_PIECES;
pub const CLIFF_CORNER_BASE: usize = CLIFF_BASE + 8;
pub const TERRACE_BASE: usize = CLIFF_CORNER_BASE + 4;
pub const SUN_LIP_BASE: usize = TERRACE_BASE + 4;
pub const CELL_COUNT: usize = SUN_LIP_BASE + MATERIAL_COUNT * FILL_SHADES * 2;

/// Directions, image-space: row 0 is north, column 0 is west.
pub const DIR_N: usize = 0;
pub const DIR_E: usize = 1;
pub const DIR_S: usize = 2;
pub const DIR_W: usize = 3;

// ── Cell indexing ──────────────────────────────────────────────────────────

#[inline]
pub fn base_cell(material: Material, shade: usize, variant: usize) -> usize {
    ((material.index() * FILL_SHADES) + shade.min(FILL_SHADES - 1)) * VARIANTS
        + variant.min(VARIANTS - 1)
}

#[inline]
pub fn transition_cell(boundary: usize, piece: usize) -> usize {
    TRANSITION_BASE + boundary * TRANSITION_PIECES + piece
}

#[inline]
pub fn cliff_cell(dir: usize, severity: usize) -> usize {
    CLIFF_BASE + dir * 2 + severity
}

#[inline]
pub fn cliff_corner_cell(quadrant: usize) -> usize {
    CLIFF_CORNER_BASE + quadrant
}

#[inline]
pub fn terrace_cell(dir: usize) -> usize {
    TERRACE_BASE + dir
}

/// Lit lip on the high side of a band step. Only the two sunlit edges exist.
#[inline]
pub fn sun_lip_cell(material: Material, shade: usize, dir: usize) -> usize {
    debug_assert!(dir == DIR_S || dir == DIR_W);
    SUN_LIP_BASE
        + (material.index() * FILL_SHADES + shade.min(FILL_SHADES - 1)) * 2
        + usize::from(dir == DIR_W)
}

// ── Atlas ──────────────────────────────────────────────────────────────────

/// Painted tile art, addressed by cell index. Lives on the CPU for the whole
/// session; chunk compositing samples it row by row.
#[derive(Resource)]
pub struct TerrainAtlas {
    pixels: Vec<u8>,
    cols: u32,
}

impl TerrainAtlas {
    /// Paint every cell. Cost is a few hundred thousand texels — well under a
    /// frame, and it happens exactly once.
    pub fn build() -> Self {
        debug_assert_eq!(
            TILE_SIZE as u32, CELL,
            "atlas cell size must match the world tile size"
        );
        let cols = ATLAS_COLS;
        let rows = CELL_COUNT.div_ceil(cols as usize) as u32;
        let mut atlas = Self {
            pixels: vec![0u8; (cols * CELL) as usize * (rows * CELL) as usize * 4],
            cols,
        };

        for material in MATERIALS {
            for shade in 0..FILL_SHADES {
                for variant in 0..VARIANTS {
                    let index = base_cell(material, shade, variant);
                    paint_base(&mut atlas.cell(index), material, shade, variant);
                }
            }
        }
        for boundary in 0..BOUNDARY_COUNT {
            for piece in 0..TRANSITION_PIECES {
                let index = transition_cell(boundary, piece);
                paint_transition(&mut atlas.cell(index), boundary, piece);
            }
        }
        for dir in 0..4 {
            for severity in 0..2 {
                let index = cliff_cell(dir, severity);
                paint_cliff(&mut atlas.cell(index), dir, severity);
            }
            paint_terrace(&mut atlas.cell(terrace_cell(dir)), dir);
            paint_cliff_corner(&mut atlas.cell(cliff_corner_cell(dir)), dir);
        }
        for material in MATERIALS {
            for shade in 0..FILL_SHADES {
                for dir in [DIR_S, DIR_W] {
                    let index = sun_lip_cell(material, shade, dir);
                    paint_sun_lip(&mut atlas.cell(index), material, shade, dir);
                }
            }
        }

        atlas
    }

    #[inline]
    fn stride(&self) -> usize {
        (self.cols * CELL) as usize * 4
    }

    #[inline]
    fn cell_origin(&self, index: usize) -> usize {
        let cx = index as u32 % self.cols;
        let cy = index as u32 / self.cols;
        (cy * CELL) as usize * self.stride() + (cx * CELL) as usize * 4
    }

    fn cell(&mut self, index: usize) -> Cell<'_> {
        let stride = self.stride();
        let origin = self.cell_origin(index);
        Cell {
            px: &mut self.pixels,
            stride,
            origin,
        }
    }

    /// One texel row of a cell as RGBA bytes (`CELL * 4` long).
    #[inline]
    pub fn cell_row(&self, index: usize, row: u32) -> &[u8] {
        let start = self.cell_origin(index) + row as usize * self.stride();
        &self.pixels[start..start + CELL as usize * 4]
    }

    /// Total texels painted — reported alongside the startup timing.
    #[inline]
    pub fn texel_count(&self) -> usize {
        self.pixels.len() / 4
    }
}

// ── Painting primitives ────────────────────────────────────────────────────

struct Cell<'a> {
    px: &'a mut [u8],
    stride: usize,
    origin: usize,
}

impl Cell<'_> {
    #[inline]
    fn offset(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= CELL as i32 || y >= CELL as i32 {
            return None;
        }
        Some(self.origin + y as usize * self.stride + x as usize * 4)
    }

    #[inline]
    fn set(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if let Some(o) = self.offset(x, y) {
            self.px[o..o + 4].copy_from_slice(&color);
        }
    }

    /// Paint only where nothing has been painted yet — keeps accent lines from
    /// burying a neighbouring edge's band at a corner.
    #[inline]
    fn set_if_clear(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if let Some(o) = self.offset(x, y) {
            if self.px[o + 3] == 0 {
                self.px[o..o + 4].copy_from_slice(&color);
            }
        }
    }

    fn fill(&mut self, color: [u8; 4]) {
        for y in 0..CELL as i32 {
            for x in 0..CELL as i32 {
                self.set(x, y, color);
            }
        }
    }

    fn hrun(&mut self, x: i32, y: i32, len: i32, color: [u8; 4]) {
        for i in 0..len {
            self.set(x + i, y, color);
        }
    }

    fn vrun(&mut self, x: i32, y: i32, len: i32, color: [u8; 4]) {
        for i in 0..len {
            self.set(x, y + i, color);
        }
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: [u8; 4]) {
        for dy in 0..h {
            self.hrun(x, y + dy, w, color);
        }
    }
}

/// R2 low-discrepancy point inside the cell.
///
/// An additive recurrence spreads marks evenly where a raw hash clumps, and
/// even spacing is most of what makes speckle read as *placed* rather than as
/// noise. The sequence is a pure function of the cell, so the art is nailed to
/// the world the moment the cell is stamped (brief 01 §2.4).
#[inline]
fn scatter(n: u32, seed: u32) -> (i32, i32) {
    let x = seed.wrapping_add(n.wrapping_mul(0xC13F_A9A9));
    let y = seed
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add(n.wrapping_mul(0x91E1_0D6F));
    ((x >> 27) as i32, (y >> 27) as i32)
}

const BASE_SALT: u32 = 0x2B1D_7C05;
const EDGE_SALT: u32 = 0x51A3_9E17;
const CLIFF_SALT: u32 = 0x6D0F_3B29;

// ── Base tiles ─────────────────────────────────────────────────────────────

fn paint_base(cell: &mut Cell, material: Material, shade: usize, variant: usize) {
    let base = rgba(material.step(shade));
    let shadow = rgba(material.shadow(shade));
    let lit = material.texture_mark(shade).map(rgba);
    // At the bottom of a ramp there is nothing darker in the material's own
    // hue, and borrowing the violet shadow leaves grass looking dirty. Texture
    // the darkest step with the one above it instead: blades catching light.
    let dark = if shade == 0 {
        lit.unwrap_or(shadow)
    } else {
        shadow
    };
    // The mirror image at the top: a step with nothing lighter available to it
    // — the head of the ramp, or a material whose only lighter colour is its
    // reserved cap — would otherwise mark itself in the fill colour, six
    // invisible rects a tile. Fall back to the step below, so a plateau still
    // carries texture and every mark that is paid for is seen.
    let light = lit.unwrap_or(dark);
    cell.fill(base);

    let seed = world_hash(
        material.index() as i32,
        (shade * VARIANTS + variant) as i32,
        BASE_SALT,
    );

    match material {
        // Ripple dashes so open sea has structure rather than one flat field.
        Material::Water => {
            for n in 0..11 {
                let (x, y) = scatter(n, seed);
                cell.hrun(x, y, 3, dark);
            }
            for n in 11..16 {
                let (x, y) = scatter(n, seed);
                cell.hrun(x, y, 2, light);
            }
        }
        // Grain, plus a few wind ripples.
        Material::Sand => {
            for n in 0..24 {
                let (x, y) = scatter(n, seed);
                cell.set(x, y, dark);
            }
            for n in 24..31 {
                let (x, y) = scatter(n, seed);
                cell.set(x, y, light);
            }
            for n in 31..37 {
                let (x, y) = scatter(n, seed);
                cell.hrun(x, y, 3, dark);
            }
        }
        // Upright tufts.
        Material::Grass => {
            for n in 0..20 {
                let (x, y) = scatter(n, seed);
                cell.vrun(x, y, 2, dark);
            }
            for n in 20..26 {
                let (x, y) = scatter(n, seed);
                cell.set(x, y, light);
            }
        }
        // Contour hatching — short horizontal strokes read as slope from above.
        Material::Hill => {
            for n in 0..15 {
                let (x, y) = scatter(n, seed);
                cell.hrun(x, y, 3, dark);
            }
            for n in 15..20 {
                let (x, y) = scatter(n, seed);
                cell.hrun(x, y, 2, light);
            }
        }
        // Broken scree blocks.
        Material::Rock => {
            for n in 0..10 {
                let (x, y) = scatter(n, seed);
                cell.rect(x, y, 2, 2, dark);
            }
            for n in 10..16 {
                let (x, y) = scatter(n, seed);
                cell.rect(x, y, 2, 1, light);
            }
        }
    }
}

// ── Transitions ────────────────────────────────────────────────────────────

/// Base lip depth in texels before per-column wobble.
const LIP: i32 = 4;

/// Texel of a band at distance `depth` in from `edge`, `t` along it.
#[inline]
fn edge_texel(dir: usize, t: i32, depth: i32) -> (i32, i32) {
    let last = CELL as i32 - 1;
    match dir {
        DIR_N => (t, depth),
        DIR_E => (last - depth, t),
        DIR_S => (t, last - depth),
        _ => (depth, t),
    }
}

/// Lip depth along an edge — wobbles ±1 so a coastline is rough, not ruled.
#[inline]
fn lip_depth(boundary: usize, dir: usize, t: i32) -> i32 {
    LIP + (world_hash(t, dir as i32, EDGE_SALT.wrapping_add(boundary as u32)) % 3) as i32
}

fn paint_transition(cell: &mut Cell, boundary: usize, piece: usize) {
    let high = MATERIALS[boundary + 1];
    let band = rgba(high.step(0));
    let lip = rgba(high.step(1));
    // Sea meeting a sand lip gets foam; every other boundary gets the game's
    // one outline colour as the shadow the higher ground casts (brief 01 §6.2).
    let accent = rgba(if boundary == 0 { WATER_F } else { OUTLINE });

    if piece >= TRANSITION_CORNER {
        paint_inner_corner(cell, boundary, piece - TRANSITION_CORNER, band, lip, accent);
        return;
    }

    // Bands first, then lit lips, then accents — so an accent never buries a
    // neighbouring edge's band where two edges meet.
    for dir in 0..4 {
        if piece & (1 << dir) == 0 {
            continue;
        }
        for t in 0..CELL as i32 {
            let depth = lip_depth(boundary, dir, t);
            for d in 0..depth {
                let (x, y) = edge_texel(dir, t, d);
                cell.set(x, y, band);
            }
        }
    }
    for dir in 0..4 {
        if piece & (1 << dir) == 0 {
            continue;
        }
        for t in 0..CELL as i32 {
            let depth = lip_depth(boundary, dir, t);
            let (x, y) = edge_texel(dir, t, depth - 1);
            cell.set(x, y, lip);
        }
    }
    for dir in 0..4 {
        if piece & (1 << dir) == 0 {
            continue;
        }
        for t in 0..CELL as i32 {
            let depth = lip_depth(boundary, dir, t);
            let (x, y) = edge_texel(dir, t, depth);
            cell.set_if_clear(x, y, accent);
        }
    }
}

/// Corner texel of a quadrant: NE, SE, SW, NW.
#[inline]
fn quadrant_corner(quadrant: usize) -> (i32, i32) {
    let last = CELL as i32 - 1;
    match quadrant {
        0 => (last, 0),
        1 => (last, last),
        2 => (0, last),
        _ => (0, 0),
    }
}

/// A nub for a diagonal-only neighbour, so a staircase boundary has no notch.
fn paint_inner_corner(
    cell: &mut Cell,
    boundary: usize,
    quadrant: usize,
    band: [u8; 4],
    lip: [u8; 4],
    accent: [u8; 4],
) {
    let (cx, cy) = quadrant_corner(quadrant);
    let radius = LIP + 2 + (world_hash(quadrant as i32, boundary as i32, EDGE_SALT) % 2) as i32;
    let inner = (radius - 1) * (radius - 1);
    let mid = radius * radius;
    let outer = (radius + 1) * (radius + 1);

    for y in 0..CELL as i32 {
        for x in 0..CELL as i32 {
            let d2 = (x - cx) * (x - cx) + (y - cy) * (y - cy);
            if d2 <= inner {
                cell.set(x, y, band);
            } else if d2 <= mid {
                cell.set(x, y, lip);
            } else if d2 <= outer {
                cell.set_if_clear(x, y, accent);
            }
        }
    }
}

// ── Cliffs ─────────────────────────────────────────────────────────────────

/// Face depth in texels, `[step, cliff]` per direction.
///
/// South is the face the player actually reads, so it is by far the deepest;
/// north is only the shadowed rim of a drop seen from above.
const CLIFF_DEPTH: [[i32; 2]; 4] = [[2, 3], [4, 9], [8, 17], [4, 9]];

/// Rock bedding: which rows across a face fall to the dark step.
const STRATA: [bool; 8] = [false, false, true, false, false, false, true, false];

fn paint_cliff(cell: &mut Cell, dir: usize, severity: usize) {
    let deep = rgba(ROCK_D);
    let sunlit = dir == DIR_S || dir == DIR_W;
    // Severity 1 is a face `rail_sim` will not let track cross at any price;
    // severity 0 is a bank in shadow that it will (see `autotile`). Only the
    // wall gets the body and the crest of a proper cliff, so the brightest rock
    // in the landscape means exactly one thing: you cannot build through here.
    let body = rgba(if severity == 1 && sunlit {
        ROCK_M
    } else {
        ROCK_D
    });
    // `SNOW` lives here and nowhere else on flat-lit ground. It is the rock
    // ramp's reserved extreme (brief 01 §3.2) and it is spent on one texel row:
    // the lit crest of an impassable face, exactly as a sun lip spends a
    // material's light step on the crest of a band step. Filling peaks with it
    // put the game's brightest colour under the `hi` accent and lost both.
    let crest = rgba(if severity == 1 && sunlit {
        SNOW
    } else {
        ROCK_M
    });
    let depth = CLIFF_DEPTH[dir][severity];
    let last = CELL as i32 - 1;

    // North is a rim, not a face: a dark break of slope with a lit inner edge.
    if dir == DIR_N {
        for t in 0..CELL as i32 {
            let d = depth + (world_hash(t, 0, CLIFF_SALT) % 2) as i32;
            for k in 0..d {
                cell.set(t, k, deep);
            }
            cell.set(t, d - 1, rgba(ROCK_M));
        }
        return;
    }

    // `k` counts inward from the low edge, so the base of the face is k = 0 and
    // the crest — where the flat top begins — is the innermost texel.
    for t in 0..CELL as i32 {
        let jitter = (world_hash(t, dir as i32, CLIFF_SALT) % 2) as i32;
        let d = depth - jitter;
        for k in 0..d {
            let (x, y) = edge_texel(dir, t, k);
            let color = if k == d - 1 {
                crest
            } else if k == 0 {
                deep
            } else if severity == 1 && strata_row(dir, t, k) {
                deep
            } else {
                body
            };
            cell.set(x, y, color);
        }
    }

    // Fissures: broken vertical breaks down the deep south face. Kept sparse
    // and uneven — evenly spaced ticks under a lit rail read as fencing.
    if dir == DIR_S {
        for n in 0..(2 + severity as u32 * 4) {
            let h = world_hash(n as i32, severity as i32, CLIFF_SALT);
            let x = (h % CELL) as i32;
            let len = 2 + (h >> 8) % (depth as u32 - 2).max(1);
            let top = last - depth + 2 + ((h >> 16) % 3) as i32;
            for i in 0..len as i32 {
                cell.set(x, top + i, deep);
            }
        }
    }
}

/// Bedding runs horizontally, so it bands by row on every face.
#[inline]
fn strata_row(dir: usize, t: i32, k: i32) -> bool {
    let row = if dir == DIR_S { k } else { t };
    STRATA[(row.rem_euclid(STRATA.len() as i32)) as usize]
}

/// A drop that is diagonal only — fills the notch a staircase ridge would
/// leave. Deliberately small: a large one reads as a boulder dropped on the
/// grass rather than as the corner of a face.
fn paint_cliff_corner(cell: &mut Cell, quadrant: usize) {
    let (cx, cy) = quadrant_corner(quadrant);
    let southern = quadrant == 1 || quadrant == 2;
    let deep = rgba(ROCK_D);
    let crest = rgba(if southern { ROCK_M } else { ROCK_D });

    let radius = 6;
    for y in 0..CELL as i32 {
        for x in 0..CELL as i32 {
            let d2 = (x - cx) * (x - cx) + (y - cy) * (y - cy);
            if d2 < (radius - 1) * (radius - 1) {
                cell.set(x, y, deep);
            } else if d2 <= radius * radius {
                cell.set(x, y, crest);
            }
        }
    }
}

// ── Terraces ───────────────────────────────────────────────────────────────

/// A single-texel contour shadow at the foot of an elevation band step.
///
/// Bands are what let the player trace a route with a finger before laying
/// anything (brief 02 §2.3); the shadow is what makes the band edge visible.
///
/// It is broken and it wanders a texel on purpose. A solid ruled line reads as
/// ink around a field — background terrain does not get to spend that much
/// contrast (brief 01 §1) — while a broken one reads as a break of slope.
fn paint_terrace(cell: &mut Cell, dir: usize) {
    let shadow = rgba(OUTLINE);
    for t in 0..CELL as i32 {
        let h = world_hash(t, dir as i32, EDGE_SALT);
        if h % 5 == 0 {
            continue;
        }
        let (x, y) = edge_texel(dir, t, ((h >> 6) % 2) as i32);
        cell.set(x, y, shadow);
    }
}

/// The lit half of a band step, drawn on the high tile in its own ramp.
///
/// This is where a material's light step belongs — a drawn edge on ground that
/// falls away to the low south-western sun, never a flat fill. Paired with the
/// contour shadow below it, a one-band rise reads as a step from directly
/// above, which is what makes slope direction legible (brief 02 §2.3).
///
/// A material already sitting on its cap has nothing lighter to be lit with, and
/// a lip painted in the fill colour is a cell's worth of invisible texels. The
/// cell is left empty instead, and `autotile::resolve_tile` never asks for it.
fn paint_sun_lip(cell: &mut Cell, material: Material, shade: usize, dir: usize) {
    let Some(lit) = material.light_mark(shade).map(rgba) else {
        return;
    };
    for t in 0..CELL as i32 {
        let h = world_hash(t, dir as i32, EDGE_SALT.wrapping_add(shade as u32));
        // A broken lip reads as a lit edge; an unbroken one reads as a border.
        if h % 7 == 0 {
            continue;
        }
        let (x, y) = edge_texel(dir, t, ((h >> 6) % 2) as i32);
        cell.set(x, y, lit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atlas() -> TerrainAtlas {
        TerrainAtlas::build()
    }

    fn cell_pixels(atlas: &TerrainAtlas, index: usize) -> Vec<[u8; 4]> {
        (0..CELL)
            .flat_map(|row| {
                atlas
                    .cell_row(index, row)
                    .chunks_exact(4)
                    .map(|c| [c[0], c[1], c[2], c[3]])
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn cell_ranges_do_not_overlap() {
        assert_eq!(BASE_CELLS, 45);
        assert_eq!(CLIFF_BASE, 125);
        assert_eq!(CELL_COUNT, 171);
        assert!(base_cell(Material::Rock, FILL_SHADES - 1, VARIANTS - 1) < TRANSITION_BASE);
        assert!(transition_cell(BOUNDARY_COUNT - 1, TRANSITION_PIECES - 1) < CLIFF_BASE);
        assert!(cliff_cell(DIR_W, 1) < CLIFF_CORNER_BASE);
        assert!(cliff_corner_cell(3) < TERRACE_BASE);
        assert!(terrace_cell(DIR_W) < CELL_COUNT);
    }

    #[test]
    fn base_cells_are_fully_opaque_and_contiguous() {
        let a = atlas();
        for material in MATERIALS {
            for shade in 0..FILL_SHADES {
                for variant in 0..VARIANTS {
                    let px = cell_pixels(&a, base_cell(material, shade, variant));
                    assert_eq!(px.len(), (CELL * CELL) as usize);
                    assert!(
                        px.iter().all(|p| p[3] == 255),
                        "base tiles must leave no gaps ({material:?} {shade} {variant})"
                    );
                }
            }
        }
    }

    #[test]
    fn every_pixel_is_opaque_or_clear_never_blended() {
        // Nearest sampling with no anti-aliasing: partial alpha has no meaning.
        let a = atlas();
        for index in 0..CELL_COUNT {
            for p in cell_pixels(&a, index) {
                assert!(p[3] == 0 || p[3] == 255, "cell {index} has partial alpha");
            }
        }
    }

    #[test]
    fn variants_actually_differ() {
        let a = atlas();
        for material in MATERIALS {
            let v0 = cell_pixels(&a, base_cell(material, 1, 0));
            let v1 = cell_pixels(&a, base_cell(material, 1, 1));
            let v2 = cell_pixels(&a, base_cell(material, 1, 2));
            assert_ne!(v0, v1, "{material:?} variants 0/1 identical");
            assert_ne!(v1, v2, "{material:?} variants 1/2 identical");
        }
    }

    #[test]
    fn base_tiles_are_textured_not_flat_rectangles() {
        let a = atlas();
        for material in MATERIALS {
            for shade in 0..FILL_SHADES {
                let px = cell_pixels(&a, base_cell(material, shade, 0));
                let distinct = px.iter().collect::<std::collections::HashSet<_>>().len();
                assert!(
                    distinct >= 2,
                    "{material:?} shade {shade} is a flat rectangle"
                );
            }
        }
    }

    #[test]
    fn coastline_lays_sand_then_foam_then_water() {
        let a = atlas();
        // Water tile with sand to the north: reading south from the top edge we
        // must cross sand, a lit sand edge, then foam — a line, not a colour change.
        let cell = transition_cell(0, 1 << DIR_N);
        let px = cell_pixels(&a, cell);
        let sand_d = rgba(Material::Sand.step(0));
        let sand_m = rgba(Material::Sand.step(1));
        let foam = rgba(WATER_F);
        for x in 0..CELL as usize {
            let col: Vec<_> = (0..CELL as usize)
                .map(|y| px[y * CELL as usize + x])
                .collect();
            assert_eq!(col[0], sand_d, "coast column {x} does not start in sand");
            let lip = col
                .iter()
                .position(|p| *p == sand_m)
                .expect("lit sand edge");
            let foam_at = col.iter().position(|p| *p == foam).expect("foam line");
            assert!(
                lip < foam_at,
                "foam must sit outside the sand lip (column {x})"
            );
            assert_eq!(col[foam_at + 1][3], 0, "water must show past the foam");
        }
    }

    #[test]
    fn transition_edges_only_touch_their_own_edge() {
        let a = atlas();
        for boundary in 0..BOUNDARY_COUNT {
            let px = cell_pixels(&a, transition_cell(boundary, 1 << DIR_N));
            // Nothing painted on the far (south) half of a north-edge piece.
            for y in (CELL as usize / 2)..CELL as usize {
                for x in 0..CELL as usize {
                    assert_eq!(px[y * CELL as usize + x][3], 0);
                }
            }
        }
    }

    #[test]
    fn cliff_faces_are_banded_rock_not_a_gradient() {
        let a = atlas();
        // `SNOW` is the rock ramp's cap, spent on the crest of a wall.
        let rock = [
            rgba(ROCK_D),
            rgba(ROCK_M),
            rgba(crate::palette::ROCK_L),
            rgba(SNOW),
        ];
        for dir in 0..4 {
            for severity in 0..2 {
                let px = cell_pixels(&a, cliff_cell(dir, severity));
                let painted: Vec<_> = px.iter().filter(|p| p[3] == 255).collect();
                assert!(
                    !painted.is_empty(),
                    "cliff {dir}/{severity} painted nothing"
                );
                assert!(
                    painted.iter().all(|p| rock.contains(p)),
                    "cliff {dir}/{severity} left the rock ramp"
                );
                let steps = painted
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len();
                assert!(
                    steps >= 2,
                    "cliff {dir}/{severity} is one flat block, not banded"
                );
            }
        }
    }

    #[test]
    fn a_cliff_is_deeper_than_a_step() {
        let a = atlas();
        for dir in 0..4 {
            let count = |severity: usize| {
                cell_pixels(&a, cliff_cell(dir, severity))
                    .iter()
                    .filter(|p| p[3] == 255)
                    .count()
            };
            assert!(
                count(1) > count(0),
                "severity must read as height (dir {dir})"
            );
        }
    }

    #[test]
    fn south_face_is_the_one_the_player_reads() {
        let a = atlas();
        let area = |dir: usize| {
            cell_pixels(&a, cliff_cell(dir, 1))
                .iter()
                .filter(|p| p[3] == 255)
                .count()
        };
        assert!(area(DIR_S) > area(DIR_E));
        assert!(area(DIR_S) > area(DIR_N));
    }

    #[test]
    fn snow_is_a_crest_and_never_a_field() {
        // Brief 01 §3.2: `SNOW` is reserved. It used to be the flat fill for
        // every impassable peak — the brightest colour in the game, L* 84
        // against 35 for grass, on 5.5% of the map, all of it saying "you cannot
        // build here" in the loudest voice available. It now appears only where
        // a light step is drawn: the crest of a wall, and the lip on rock that
        // steps down to the sun.
        let a = atlas();
        let snow = rgba(SNOW);
        let lit_faces: std::collections::HashSet<usize> = [DIR_S, DIR_W]
            .into_iter()
            .map(|dir| cliff_cell(dir, 1))
            .collect();
        let rock_lips: std::collections::HashSet<usize> = [DIR_S, DIR_W]
            .into_iter()
            .map(|dir| sun_lip_cell(Material::Rock, 2, dir))
            .collect();

        for index in 0..CELL_COUNT {
            let snowy = cell_pixels(&a, index)
                .iter()
                .filter(|p| **p == snow)
                .count();
            if lit_faces.contains(&index) || rock_lips.contains(&index) {
                assert!(snowy > 0, "cell {index} is a lit rock crest with no snow");
                // A crest, not a field: one texel row of a 32×32 cell.
                assert!(
                    snowy <= CELL as usize * 2,
                    "cell {index} wears {snowy} texels of snow — that is a fill"
                );
            } else {
                assert_eq!(snowy, 0, "cell {index} spent snow off a lit rock crest");
            }
        }

        // In particular: no base tile, at any shade or variant, is ever snow.
        for shade in 0..FILL_SHADES {
            for variant in 0..VARIANTS {
                let px = cell_pixels(&a, base_cell(Material::Rock, shade, variant));
                assert!(
                    !px.contains(&snow),
                    "a mountain field is filled with snow at shade {shade}"
                );
            }
        }
    }

    #[test]
    fn light_marks_are_visible_or_are_not_painted() {
        // `Material::highlight` used to return the base colour at the top of a
        // ramp, so the atlas stamped six light rects per snow tile and five per
        // shallow-water tile in the fill colour: paid for, never seen. Every
        // light mark must now differ from the ground it lands on, and a lip with
        // nowhere to go must not be drawn at all.
        let a = atlas();
        for material in MATERIALS {
            for shade in 0..FILL_SHADES {
                let base = rgba(material.step(shade));
                let px = cell_pixels(&a, base_cell(material, shade, 0));
                assert!(
                    px.iter().any(|p| *p != base),
                    "{material:?} shade {shade} is an unmarked field"
                );
                if let Some(light) = material.texture_mark(shade) {
                    assert!(
                        px.contains(&rgba(light)),
                        "{material:?} shade {shade} lost its light marks"
                    );
                }
                if let Some(cap) = material.reserved_cap() {
                    assert!(
                        !px.contains(&rgba(cap)),
                        "{material:?} shade {shade} spent its reserved cap on flat texture"
                    );
                }

                for dir in [DIR_S, DIR_W] {
                    let lip = cell_pixels(&a, sun_lip_cell(material, shade, dir));
                    let painted = lip.iter().filter(|p| p[3] == 255).count();
                    match material.light_mark(shade) {
                        Some(light) => {
                            assert!(painted > 0, "{material:?} shade {shade} lip is empty");
                            assert!(
                                lip.iter().all(|p| p[3] == 0 || *p == rgba(light)),
                                "{material:?} shade {shade} lip left its ramp"
                            );
                        }
                        None => assert_eq!(
                            painted, 0,
                            "{material:?} shade {shade} painted a lip it cannot light"
                        ),
                    }
                }
            }
        }
    }

    #[test]
    fn atlas_uses_only_palette_colours() {
        use crate::palette::{
            GRASS_D, GRASS_L, GRASS_M, HILL_D, HILL_L, HILL_M, ROCK_L, SAND_D, SAND_L, SAND_M,
            WATER_D, WATER_L, WATER_M,
        };
        let allowed: std::collections::HashSet<[u8; 4]> = [
            WATER_D, WATER_M, WATER_L, WATER_F, SAND_D, SAND_M, SAND_L, GRASS_D, GRASS_M, GRASS_L,
            HILL_D, HILL_M, HILL_L, ROCK_D, ROCK_M, ROCK_L, SNOW, OUTLINE,
        ]
        .into_iter()
        .map(rgba)
        .collect();

        let a = atlas();
        for index in 0..CELL_COUNT {
            for p in cell_pixels(&a, index) {
                if p[3] == 0 {
                    continue;
                }
                assert!(allowed.contains(&p), "cell {index} used off-palette {p:?}");
            }
        }
    }

    #[test]
    fn diagnostic_accents_never_reach_world_art() {
        use crate::palette::{HI, OK, WARN};
        let banned: Vec<[u8; 4]> = [HI, WARN, OK].into_iter().map(rgba).collect();
        let a = atlas();
        for index in 0..CELL_COUNT {
            for p in cell_pixels(&a, index) {
                assert!(
                    !banned.contains(&p),
                    "cell {index} used a diagnostic accent"
                );
            }
        }
    }

    #[test]
    fn build_is_cheap_enough_for_startup() {
        let start = std::time::Instant::now();
        let a = TerrainAtlas::build();
        let elapsed = start.elapsed();
        let rows = CELL_COUNT.div_ceil(ATLAS_COLS as usize) as u32;
        assert_eq!(a.texel_count(), (ATLAS_COLS * CELL * rows * CELL) as usize);
        assert!(
            elapsed.as_millis() < 250,
            "atlas build took {elapsed:?}; budget is well under a second"
        );
    }
}
