//! Isometric terrain — the diamond renderer, in place of the chunk compositor.
//!
//! [`super::chunk`] paints axis-aligned tiles into 16 × 16 chunk textures, which
//! cannot be reprojected: a diamond grid is not a rectangle of rectangles. So on
//! this branch the compositor is bypassed and terrain is drawn per tile:
//!
//! - **one 64 × 32 diamond** per tile, from a baked atlas keyed on
//!   (material, fill shade, world-hashed variant) — the same three inputs
//!   [`super::material`] gives the flat renderer, so the colours are the game's
//!   colours and a band step reads as the same value step it does on `main`.
//! - **up to two cliff faces** per tile, where it stands above its south or west
//!   neighbour. Those are the two faces the camera can see: the projection puts
//!   the camera over the map's south-west corner, so a tile's near corner is its
//!   south-west one and the two silhouette edges below it face south and west.
//!
//! 4096 tiles is 4096 sprites plus faces, which the pixel contract's §2.5 (bake
//! on edit, one quad per chunk) exists to prevent. It is fine here for one
//! reason: every tile draws from the *same* atlas image, so Bevy batches the lot
//! into one draw call. Nothing is re-baked per frame; a map swap re-spawns.
//!
//! # The face crop, and why the overhang does not show
//!
//! A face is a parallelogram: 32 px wide, slanted 1:2, `depth` px thick. Baking
//! one cell per (material, shade, side, depth) is thousands of cells, so instead
//! one *full-depth* cell is baked per (material, shade, side) and cropped to
//! `16 + depth` rows. The crop's bottom edge is flat where the true face's is
//! slanted, which overhangs by up to 16 px at one end.
//!
//! That overhang is exactly the quarter of the neighbouring diamond between its
//! centre and the two vertices the face meets — and the neighbour is one row
//! *nearer* the camera, so it draws after, opaque, over the top. The one place
//! it can show is the map border, where there is no neighbour, and there the
//! flat bottom is the plinth the whole map stands on. See
//! `a_face_overhang_lands_inside_the_neighbour_it_abuts`.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::math::Rect;
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rail_map::{tile_to_world, MapGrid, ISO_LIFT};
use rail_sim::ids::TileCoord;
use rail_sim::PathWear;

use crate::map::iso_depth::{depth_z, TERRAIN_LAYER};
use crate::map::paths::{
    path_mark, path_tones, path_variant_for, PathMark, PATH_DUST, PATH_FILL, PATH_VARIANTS,
};
use crate::palette::OUTLINE;

use super::atlas::PATH_LEVELS;
use super::material::{
    material_of, rgba, shade_for, world_hash, Material, BAND_STEP, FILL_SHADES, MATERIALS,
    MATERIAL_COUNT, VARIANTS,
};

/// Diamond cell size, in texels. 2:1, as the projection demands.
pub const TOP_W: u32 = 64;
pub const TOP_H: u32 = 32;
/// Face cell width — half a diamond.
pub const FACE_W: u32 = 32;
/// Deepest face the atlas can serve, in texels. 128 px is 32 height units at
/// [`ISO_LIFT`], and the generator's tallest wall is 18.
pub const MAX_FACE: u32 = 128;
/// A face cell is the slanted rim plus the full drop.
pub const FACE_H: u32 = TOP_H / 2 + MAX_FACE;

/// How far below the lowest tile the map's border plinth drops, in height units.
const BORDER_DROP: i16 = 5;

/// Ballast-grade grain: how much of a tile's fill takes its light / dark mark.
const SPECKLE_PERMILLE: u32 = 90;
const SPECKLE_SALT: u32 = 0x3B1F_77C5;
const GRAIN_SALT: u32 = 0x91C2_04ED;

/// The two faces a tile can show, in draw order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Lower-left of the diamond, facing the map's west. The lit one.
    West,
    /// Lower-right of the diamond, facing the map's south. The shaded one.
    South,
}

impl Side {
    #[inline]
    fn index(self) -> usize {
        match self {
            Side::West => 0,
            Side::South => 1,
        }
    }

    /// Screen-x offset of the face's centre from the tile's.
    #[inline]
    fn x_offset(self) -> f32 {
        match self {
            Side::West => -(FACE_W as f32) * 0.5,
            Side::South => FACE_W as f32 * 0.5,
        }
    }

    /// The neighbour this face looks at.
    #[inline]
    fn neighbour(self, coord: TileCoord) -> TileCoord {
        match self {
            Side::West => TileCoord {
                x: coord.x - 1,
                y: coord.y,
            },
            Side::South => TileCoord {
                x: coord.x,
                y: coord.y - 1,
            },
        }
    }
}

// ── Atlas layout ───────────────────────────────────────────────────────────

/// Flat variants per (material, shade) — more than the top-down renderer's
/// [`VARIANTS`], and for a reason the projection creates. A diamond's grain is
/// baked in cell space, so every tile sharing a variant repeats the same texels
/// at the same offset; on the diamond lattice those repeats line up into visible
/// diagonal corduroy across a big field of grass, which three variants did not
/// hide and eight do.
pub const ISO_VARIANTS: usize = 8;
const _: () = assert!(ISO_VARIANTS >= VARIANTS);
const VARIANT_SALT: u32 = 0x5C77_A103;

/// Which flat variant a tile draws. World-anchored, so the grain belongs to the
/// ground rather than to the screen.
#[inline]
pub fn iso_variant_for(coord: TileCoord) -> usize {
    (world_hash(coord.x, coord.y, VARIANT_SALT) % ISO_VARIANTS as u32) as usize
}

const TOP_CELLS: u32 = (MATERIAL_COUNT * FILL_SHADES * ISO_VARIANTS) as u32;
const FACE_CELLS: u32 = (MATERIAL_COUNT * FILL_SHADES) as u32 * 2;
/// Cells per atlas row. Wrapping keeps the atlas a sane square-ish texture
/// instead of one 7680-texel strip.
const TOP_COLS: u32 = 24;
const TOP_ROWS: u32 = TOP_CELLS.div_ceil(TOP_COLS);
const FACE_Y: u32 = TOP_ROWS * TOP_H;

/// Worn-ground diamonds: 3 fill shades x 3 wear levels x 4 mask variants.
///
/// Keyed on the ground's *fill shade* rather than its material, because the
/// path ramp is a pure function of that (see [`crate::map::paths`]) — which is
/// what keeps this 36 cells instead of 180.
const PATH_CELLS: u32 = (FILL_SHADES * PATH_LEVELS * PATH_VARIANTS) as u32;
const PATH_Y: u32 = FACE_Y + FACE_H;
const PATH_ROWS: u32 = PATH_CELLS.div_ceil(TOP_COLS);

const ATLAS_W: u32 = if TOP_COLS * TOP_W > FACE_CELLS * FACE_W {
    TOP_COLS * TOP_W
} else {
    FACE_CELLS * FACE_W
};
const ATLAS_H: u32 = PATH_Y + PATH_ROWS * TOP_H;

#[inline]
fn top_index(material: Material, shade: usize, variant: usize) -> u32 {
    ((material.index() * FILL_SHADES + shade.min(FILL_SHADES - 1)) * ISO_VARIANTS
        + variant.min(ISO_VARIANTS - 1)) as u32
}

#[inline]
fn top_origin(material: Material, shade: usize, variant: usize) -> (u32, u32) {
    let i = top_index(material, shade, variant);
    ((i % TOP_COLS) * TOP_W, (i / TOP_COLS) * TOP_H)
}

#[inline]
fn face_index(material: Material, shade: usize, side: Side) -> u32 {
    ((material.index() * FILL_SHADES + shade.min(FILL_SHADES - 1)) * 2 + side.index()) as u32
}

/// Sub-rect of the atlas for a tile's diamond top.
pub fn top_rect(material: Material, shade: usize, variant: usize) -> Rect {
    let (x, y) = top_origin(material, shade, variant);
    Rect::new(x as f32, y as f32, (x + TOP_W) as f32, (y + TOP_H) as f32)
}

/// Sub-rect for a face, cropped to `depth` texels of drop.
pub fn face_rect(material: Material, shade: usize, side: Side, depth: u32) -> Rect {
    let x = (face_index(material, shade, side) * FACE_W) as f32;
    let rows = TOP_H / 2 + depth.min(MAX_FACE);
    Rect::new(x, FACE_Y as f32, x + FACE_W as f32, (FACE_Y + rows) as f32)
}

#[inline]
fn path_index(shade: usize, level: u8, variant: usize) -> u32 {
    let level = (level.clamp(1, PATH_LEVELS as u8) as usize) - 1;
    ((shade.min(FILL_SHADES - 1) * PATH_LEVELS + level) * PATH_VARIANTS
        + variant.min(PATH_VARIANTS - 1)) as u32
}

#[inline]
fn path_origin(shade: usize, level: u8, variant: usize) -> (u32, u32) {
    let i = path_index(shade, level, variant);
    ((i % TOP_COLS) * TOP_W, PATH_Y + (i / TOP_COLS) * TOP_H)
}

/// Sub-rect of the atlas for a tile's worn ground. `level` is 1..=3; clean
/// ground has no cell, because it draws nothing at all.
pub fn path_rect(shade: usize, level: u8, variant: usize) -> Rect {
    let (x, y) = path_origin(shade, level, variant);
    Rect::new(x as f32, y as f32, (x + TOP_W) as f32, (y + TOP_H) as f32)
}

// ── Painting ───────────────────────────────────────────────────────────────

struct Canvas {
    px: Vec<u8>,
}

impl Canvas {
    fn new() -> Self {
        Self {
            px: vec![0u8; (ATLAS_W * ATLAS_H) as usize * 4],
        }
    }

    #[inline]
    fn put(&mut self, x: u32, y: u32, color: [u8; 4]) {
        if x >= ATLAS_W || y >= ATLAS_H {
            return;
        }
        let o = ((y * ATLAS_W + x) * 4) as usize;
        self.px[o..o + 4].copy_from_slice(&color);
    }
}

/// Half-width of the diamond at image row `row`, in texels.
///
/// Rows step two texels a side, which is what makes the 2:1 slant land on whole
/// pixels and neighbouring diamonds interlock without a seam.
#[inline]
fn diamond_half(row: u32) -> u32 {
    let d = if row < TOP_H / 2 {
        row
    } else {
        TOP_H - 1 - row
    };
    2 * d + 2
}

/// One face's tone: a solid colour, or two to dither between.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tone {
    pub solid: Color,
    /// Checkerboarded with `solid` where the ramp has run out of rungs.
    pub dither: Option<Color>,
}

impl Tone {
    #[inline]
    fn solid(color: Color) -> Self {
        Self {
            solid: color,
            dither: None,
        }
    }

    #[inline]
    fn at(&self, col: u32, row: u32) -> Color {
        match self.dither {
            Some(other) if (col + row).is_multiple_of(2) => other,
            _ => self.solid,
        }
    }
}

/// The ladder a material's faces climb down: its fill, every visibly darker step
/// of its own ramp, then [`OUTLINE`] — the game's one shadow key and the floor
/// under every material.
fn shade_ladder(material: Material, shade: usize) -> Vec<Color> {
    let base = material.step(shade);
    let ramp = material.ramp();
    let mut ladder = vec![base];
    for step in ramp[..shade.min(super::material::SHADES - 1)].iter().rev() {
        if !ladder.contains(step) {
            ladder.push(*step);
        }
    }
    ladder.push(OUTLINE);
    ladder
}

/// The two visible faces' tones: lit (west) one rung down, shaded (south) two.
///
/// The awkward case is a material already at the bottom of its ramp — plains at
/// sea level is `GRASS_D`, and it is most of the land on most maps. Its ladder
/// is two rungs, fill and `OUTLINE`, so taking rungs literally painted both its
/// faces flat `OUTLINE`: every low bank in the game read as a hole cut in the
/// map rather than as ground in shadow. Where a rung is missing the face is
/// **checkerboarded** with the rung above instead, which is the pixel-art way to
/// find a value between two colours without inventing a third.
fn face_tones(material: Material, shade: usize) -> (Tone, Tone) {
    let ladder = shade_ladder(material, shade);
    match ladder.len() {
        // Fill, one dark step, another, floor: both faces get a real colour.
        n if n >= 4 => (Tone::solid(ladder[1]), Tone::solid(ladder[2])),
        // Fill, one dark step, floor: the shaded face is halfway to the floor.
        3 => (
            Tone::solid(ladder[1]),
            Tone {
                solid: ladder[2],
                dither: Some(ladder[1]),
            },
        ),
        // Fill and floor only: the lit face is halfway down, the shaded one is
        // the floor.
        _ => (
            Tone {
                solid: ladder[1],
                dither: Some(ladder[0]),
            },
            Tone::solid(ladder[1]),
        ),
    }
}

/// One tile's diamond: flat fill, world-hashed grain, and a lit north-west rim.
fn paint_top(canvas: &mut Canvas, material: Material, shade: usize, variant: usize) {
    let (ox, oy) = top_origin(material, shade, variant);
    let base = rgba(material.step(shade));
    let dark = rgba(material.shadow(shade));
    let light = material.texture_mark(shade).map(rgba);
    // Light comes from the upper left in iso — the near-universal convention,
    // and the one that separates the two visible faces. (`main`'s top-down art
    // lights from the south-west; a lit south face and a lit west face are the
    // same face pair here, and would flatten a cliff into one tone.)
    let rim = light.unwrap_or(base);

    // Variants must not be near neighbours in the hash, or the grain of two of
    // them differs by a handful of texels and the repeat is still visible.
    let salt = SPECKLE_SALT.wrapping_add((variant as u32).wrapping_mul(0x9E37_79B9));
    for row in 0..TOP_H {
        let half = diamond_half(row);
        let x0 = TOP_W / 2 - half;
        let x1 = TOP_W / 2 + half;
        for x in x0..x1 {
            let color = match world_hash(x as i32, row as i32, salt) % 1000 {
                n if n < SPECKLE_PERMILLE => dark,
                n if n < SPECKLE_PERMILLE * 2 => light.unwrap_or(base),
                _ => base,
            };
            canvas.put(ox + x, oy + row, color);
        }
        // The north-west rim: one texel at the left end of every row in the
        // diamond's upper half — the edge turned toward the light. Two texels
        // was legible from orbit: adjacent tiles' rims line up into unbroken
        // diagonals, so whatever this costs it costs across the whole field,
        // and at two it ploughed the grass into furrows.
        if row < TOP_H / 2 {
            canvas.put(ox + x0, oy + row, rim);
        }
    }
}

/// One face cell: the slanted rim, the drop, and a strata line per band step.
fn paint_face(canvas: &mut Canvas, material: Material, shade: usize, side: Side) {
    let ox = face_index(material, shade, side) * FACE_W;
    let oy = FACE_Y;
    let (west, south) = face_tones(material, shade);
    let tone = match side {
        Side::West => west,
        Side::South => south,
    };
    let lip = rgba(material.step(shade));
    // A ledge line reads against the fill it sits on, so it is the *other*
    // face's tone: dark on the lit face, light on the shaded one.
    let strata = rgba(match side {
        Side::West => south.solid,
        Side::South => west.solid,
    });
    // One strata line per elevation band, so a tall wall counts its own height.
    let band_px = (BAND_STEP as f32 * ISO_LIFT).round().max(4.0) as u32;

    for col in 0..FACE_W {
        // Top of the face at this column, following the diamond's lower rim.
        // West runs down-to-up left to right, south runs up-to-down.
        let top = match side {
            Side::West => col / 2,
            Side::South => (TOP_H / 2).saturating_sub(col.div_ceil(2)),
        };
        for row in top..FACE_H {
            let below = row - top;
            let color = if below == 0 {
                // The lit lip where the ground breaks: the tile's own fill,
                // one texel, so the top edge of a cliff reads as an edge.
                lip
            } else if below % band_px == 0 {
                strata
            } else if world_hash(col as i32, row as i32, GRAIN_SALT) % 1000 < SPECKLE_PERMILLE {
                rgba(tone.at(col + 1, row))
            } else {
                rgba(tone.at(col, row))
            };
            canvas.put(ox + col, oy + row, color);
        }
    }
}

/// One tile's worn ground: bare earth scattered over the diamond, thinning at
/// the rim so a lane's edge is ragged rather than a drawn outline.
///
/// Painted into the same 64 x 32 diamond the ground uses, from the same mask
/// the chunk compositor stamps from above, so one lane is the same lane in
/// either projection. Everything the mask leaves alone stays transparent and
/// the tile's own grass shows through.
fn paint_path(canvas: &mut Canvas, shade: usize, level: u8, variant: usize) {
    let (ox, oy) = path_origin(shade, level, variant);
    let fill = rgba(PATH_FILL[shade.min(FILL_SHADES - 1)]);
    let dust = rgba(PATH_DUST[shade.min(FILL_SHADES - 1)]);
    for row in 0..TOP_H {
        let half = diamond_half(row);
        for x in (TOP_W / 2 - half)..(TOP_W / 2 + half) {
            match path_mark(level, variant, x, row, TOP_W, TOP_H) {
                Some(PathMark::Fill) => canvas.put(ox + x, oy + row, fill),
                Some(PathMark::Dust) => canvas.put(ox + x, oy + row, dust),
                None => {}
            }
        }
    }
}

fn build_atlas() -> Image {
    let mut canvas = Canvas::new();
    for material in MATERIALS {
        for shade in 0..FILL_SHADES {
            for variant in 0..ISO_VARIANTS {
                paint_top(&mut canvas, material, shade, variant);
            }
            paint_face(&mut canvas, material, shade, Side::West);
            paint_face(&mut canvas, material, shade, Side::South);
        }
    }
    for shade in 0..FILL_SHADES {
        for level in 1..=PATH_LEVELS as u8 {
            for variant in 0..PATH_VARIANTS {
                paint_path(&mut canvas, shade, level, variant);
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: ATLAS_W,
            height: ATLAS_H,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        canvas.px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::nearest();
    image
}

// ── Spawning ───────────────────────────────────────────────────────────────

/// A plain white diamond the size of one tile, for everything that used to draw
/// a tile-shaped square: build ghosts, catchment rings, overlay cells.
///
/// A square footprint over a diamond grid does not read as "this tile" — it
/// reads as debris. Tinting a white mask keeps every one of those call sites a
/// one-line change.
fn build_diamond_mask() -> Image {
    let mut px = vec![0u8; (TOP_W * TOP_H) as usize * 4];
    for row in 0..TOP_H {
        let half = diamond_half(row);
        for x in (TOP_W / 2 - half)..(TOP_W / 2 + half) {
            let o = ((row * TOP_W + x) * 4) as usize;
            px[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: TOP_W,
            height: TOP_H,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// Handle to the tile-shaped diamond mask, and the one place that decides what
/// shape "one tile" is for a ghost, a ring or an overlay cell.
///
/// Top-down keeps its own footprints exactly as they were — a build ghost is a
/// thin bar along a track, a hover is very nearly a whole square — so each call
/// site passes both: the fraction of a diamond it wants in isometric, and the
/// literal size it drew before. Neither view is a compromise for the other.
#[derive(Resource, Debug, Clone)]
pub struct TileMark(pub Handle<Image>);

impl TileMark {
    /// A tinted tile footprint: `flat` world units in top-down, or `scale` of a
    /// diamond in isometric.
    pub fn sprite(&self, color: Color, scale: f32, flat: Vec2) -> Sprite {
        if !rail_map::projection_is_iso() {
            return Sprite::from_color(color, flat);
        }
        Sprite {
            image: self.0.clone(),
            color,
            custom_size: Some(Vec2::new(
                rail_map::ISO_TILE_W * scale,
                rail_map::ISO_TILE_H * scale,
            )),
            ..default()
        }
    }

    /// The common case: a square of `scale` tiles flat, a diamond of `scale`
    /// isometric.
    pub fn square(&self, color: Color, scale: f32) -> Sprite {
        self.sprite(color, scale, Vec2::splat(rail_map::TILE_SIZE * scale))
    }
}

/// Every sprite this module owns, so a map swap can clear the lot.
#[derive(Component, Debug, Clone, Copy)]
pub struct IsoTerrain;

/// The one atlas every terrain sprite samples, so they all batch together.
#[derive(Resource)]
pub struct IsoTerrainAtlas(pub Handle<Image>);

/// Terrain the sprites were built from — see [`super::chunk::TerrainDirty`] for
/// why a change tick on [`MapGrid`] is not the same question.
#[derive(Resource, Default)]
pub struct IsoTerrainState {
    signature: Option<u64>,
}

fn terrain_signature(map: &MapGrid) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |byte: u8| {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    eat(map.width as u8);
    eat((map.width >> 8) as u8);
    eat(map.height as u8);
    eat((map.height >> 8) as u8);
    for tile in map.tiles() {
        eat(tile.height as u8);
        eat(tile.water as u8);
        eat(tile.kind as u8);
    }
    hash
}

/// Surface height a tile presents to its neighbours; off-map is the plinth.
#[inline]
fn surface(map: &MapGrid, coord: TileCoord, floor: i16) -> i16 {
    match map.get(coord) {
        Some(tile) => rail_map::surface_height_of(tile) as i16,
        None => floor,
    }
}

/// The height everything off the map hangs down to — the plinth the world
/// stands on, so the border has an edge rather than a raw seam.
fn plinth_floor(map: &MapGrid) -> i16 {
    map.tiles()
        .iter()
        .map(|t| rail_map::surface_height_of(t) as i16)
        .min()
        .unwrap_or(0)
        - BORDER_DROP
}

/// Drop below a tile on one side, in texels. Zero when the neighbour is level
/// or higher.
fn face_depth(map: &MapGrid, coord: TileCoord, side: Side, floor: i16) -> u32 {
    let here = surface(map, coord, floor);
    let there = surface(map, side.neighbour(coord), floor);
    ((here - there).max(0) as f32 * ISO_LIFT).round() as u32
}

/// Despawn the old terrain and draw `map` from scratch.
fn spawn_terrain(commands: &mut Commands, map: &MapGrid, atlas: &Handle<Image>) -> usize {
    let floor = plinth_floor(map);
    let mut batch: Vec<(IsoTerrain, Sprite, Transform)> = Vec::with_capacity(map.tiles().len() * 2);
    for y in 0..map.height as i32 {
        for x in 0..map.width as i32 {
            let coord = TileCoord { x, y };
            let tile = map.tile(coord);
            let material = material_of(tile.kind);
            let shade = shade_for(tile.kind, tile.height);
            let variant = iso_variant_for(coord);
            let (sx, sy) = tile_to_world(coord);
            let z = depth_z(x + y, TERRAIN_LAYER);

            batch.push((
                IsoTerrain,
                Sprite {
                    image: atlas.clone(),
                    rect: Some(top_rect(material, shade, variant)),
                    ..default()
                },
                Transform::from_xyz(sx, sy, z),
            ));

            for side in [Side::West, Side::South] {
                let depth = face_depth(map, coord, side, floor).min(MAX_FACE);
                if depth == 0 {
                    continue;
                }
                let rows = (TOP_H / 2 + depth) as f32;
                batch.push((
                    IsoTerrain,
                    Sprite {
                        image: atlas.clone(),
                        rect: Some(face_rect(material, shade, side, depth)),
                        ..default()
                    },
                    // The cell's first row sits on the tile's own centre line,
                    // so the crop hangs straight down from there.
                    Transform::from_xyz(
                        sx + side.x_offset(),
                        sy - rows * 0.5,
                        // Behind its own tile's top, in front of the row behind.
                        z - 0.001,
                    ),
                ));
            }
        }
    }
    let count = batch.len();
    commands.spawn_batch(batch);
    count
}

/// Bake the diamond atlas and the tile-shaped mask, without drawing anything.
///
/// The projection is a runtime choice, so this runs at startup whatever the
/// view opens in: a flip into isometric then costs a re-spawn and no bake at
/// all. The mask is wanted in both views — it is what every ghost, catchment
/// ring and overlay cell tints — so it is built here too.
pub fn setup_iso_atlas(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let started = Instant::now();
    let handle = images.add(build_atlas());
    info!(
        "iso terrain: {}x{} atlas baked in {:?}",
        ATLAS_W,
        ATLAS_H,
        started.elapsed()
    );
    commands.insert_resource(IsoTerrainAtlas(handle));
    commands.insert_resource(TileMark(images.add(build_diamond_mask())));
}

/// Redraw when the terrain really moves — a new map, a loaded save, an edit —
/// or when there is nothing drawn at all, which is how a flip into this view
/// arrives.
pub fn rebuild_iso_terrain(
    mut commands: Commands,
    map: Res<MapGrid>,
    atlas: Option<Res<IsoTerrainAtlas>>,
    mut state: ResMut<IsoTerrainState>,
    existing: Query<Entity, With<IsoTerrain>>,
) {
    let _perf = crate::overlays::perf::scope("rebuild_iso_terrain");
    let absent = existing.is_empty();
    if !absent && (!map.is_changed() || map.is_added()) {
        return;
    }
    let signature = terrain_signature(&map);
    if !absent && state.signature == Some(signature) {
        return;
    }
    state.signature = Some(signature);
    let Some(atlas) = atlas else {
        return;
    };
    let started = Instant::now();
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    crate::map::projection::set_iso_heights(&map);
    let sprites = spawn_terrain(&mut commands, &map, &atlas.0);
    // The number the projection flip is really paying: everything else about a
    // flip is a handful of microseconds, and this is the whole map re-spawning.
    info!(
        "iso terrain: {} sprites for {} tiles in {:?}",
        sprites,
        map.tiles().len(),
        started.elapsed()
    );
}

/// A drawn desire path, and the tile it belongs to.
///
/// Also carries [`IsoTerrain`], for two reasons that both matter: it opts the
/// sprite out of [`iso_depth_sort`](crate::map::iso_sort::iso_depth_sort) — a
/// path sets its own z at spawn like the ground it lies on — and it means the
/// projection flip and the map-swap rebuild sweep paths up along with
/// everything else this module owns, without either of them having to know
/// paths exist.
#[derive(Component, Debug, Clone, Copy)]
pub struct IsoPath(pub TileCoord);

/// z above the tile's own diamond, and far below the next layer.
///
/// A path is not a *thing on* the ground, it is the ground being different, so
/// it sits as close to the terrain as a distinct sprite can: one thousandth,
/// against the 0.015 that separates one layer from the next.
const PATH_LIFT: f32 = 0.001;

/// The path sprite a tile should draw, if it should draw one.
fn path_sprite(map: &MapGrid, atlas: &Handle<Image>, coord: TileCoord, level: u8) -> Option<Sprite> {
    if level == 0 {
        return None;
    }
    let tile = map.get(coord)?;
    if tile.water {
        return None;
    }
    let shade = shade_for(tile.kind, tile.height);
    // Ground with no grass on it has nothing to wear (brief 16 §3.2).
    path_tones(material_of(tile.kind), shade)?;
    Some(Sprite {
        image: atlas.clone(),
        rect: Some(path_rect(shade, level, path_variant_for(coord))),
        ..default()
    })
}

fn path_transform(coord: TileCoord) -> Transform {
    let (sx, sy) = tile_to_world(coord);
    Transform::from_xyz(sx, sy, depth_z(coord.x + coord.y, TERRAIN_LAYER) + PATH_LIFT)
}

/// Draw the paths the sim says are there, and nothing more.
///
/// **This is the system the whole quantisation exists for.** The sim publishes
/// level transitions rather than wear, so on almost every frame of a living
/// town `changes` is empty and this returns having done nothing at all — no
/// query iteration, no lookup table, no allocation. A tile whose wear climbed
/// from 700 to 701 costs exactly one branch.
///
/// Runs after [`rebuild_iso_terrain`], which despawns everything this module
/// owns when the map itself moves; finding no path sprites is how this learns
/// that it has to draw them all again.
pub fn sync_iso_paths(
    mut commands: Commands,
    map: Res<MapGrid>,
    atlas: Option<Res<IsoTerrainAtlas>>,
    paths: Option<ResMut<PathWear>>,
    existing: Query<(Entity, &IsoPath)>,
) {
    let _perf = crate::overlays::perf::scope("sync_iso_paths");
    let (Some(atlas), Some(mut paths)) = (atlas, paths) else {
        return;
    };
    let (changes, mut resync) = paths.drain_changes();

    // Nothing drawn but something to draw: a flip into this view, or a map
    // swap that took the paths with the terrain.
    //
    // Deliberately *not* keyed on `map.is_changed()`. Bevy change detection
    // answers "did anybody write this resource", and the border slice's portal
    // mirror writes `MapGrid` every frame — keying on it would despawn and
    // respawn every path sprite in town, every frame, which is precisely the
    // shape of the regression `chunk::TerrainDirty` carries scar tissue for. A
    // terrain change that matters despawns this module's sprites through
    // `rebuild_iso_terrain`, and an empty query is how that arrives here.
    resync |= existing.is_empty() && paths.drawn_tiles().next().is_some();

    if resync {
        for (entity, _) in &existing {
            commands.entity(entity).despawn();
        }
        let batch: Vec<(IsoTerrain, IsoPath, Sprite, Transform)> = paths
            .drawn_tiles()
            .filter_map(|(tile, level)| {
                let sprite = path_sprite(&map, &atlas.0, tile, level)?;
                Some((IsoTerrain, IsoPath(tile), sprite, path_transform(tile)))
            })
            .collect();
        commands.spawn_batch(batch);
        return;
    }
    if changes.is_empty() {
        return;
    }

    // Only now, with something genuinely to do, is the lookup worth building.
    let drawn: std::collections::HashMap<(i32, i32), Entity> = existing
        .iter()
        .map(|(entity, path)| ((path.0.x, path.0.y), entity))
        .collect();

    for change in changes {
        let key = (change.tile.x, change.tile.y);
        match (drawn.get(&key), path_sprite(&map, &atlas.0, change.tile, change.level)) {
            // Grew, shrank, or simply changed step: re-point the atlas rect.
            (Some(&entity), Some(sprite)) => {
                commands.entity(entity).insert(sprite);
            }
            // Newly worn ground.
            (None, Some(sprite)) => {
                commands.spawn((
                    IsoTerrain,
                    IsoPath(change.tile),
                    sprite,
                    path_transform(change.tile),
                ));
            }
            // Grassed back over, or ground that never draws a path at all.
            (Some(&entity), None) => {
                commands.entity(entity).despawn();
            }
            (None, None) => {}
        }
    }
}

/// Drop every diamond and cliff face — the flip out of isometric.
pub fn despawn_iso_terrain(
    commands: &mut Commands,
    state: &mut IsoTerrainState,
    existing: &Query<Entity, With<IsoTerrain>>,
) {
    for entity in existing.iter() {
        commands.entity(entity).despawn();
    }
    state.signature = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_map::{generate_map, TerrainKind, Tile, DEFAULT_MAP_SEED};

    fn canvas() -> Canvas {
        let mut c = Canvas::new();
        for material in MATERIALS {
            for shade in 0..FILL_SHADES {
                for variant in 0..ISO_VARIANTS {
                    paint_top(&mut c, material, shade, variant);
                }
                paint_face(&mut c, material, shade, Side::West);
                paint_face(&mut c, material, shade, Side::South);
            }
        }
        for shade in 0..FILL_SHADES {
            for level in 1..=PATH_LEVELS as u8 {
                for variant in 0..PATH_VARIANTS {
                    paint_path(&mut c, shade, level, variant);
                }
            }
        }
        c
    }

    fn texel(c: &Canvas, x: u32, y: u32) -> [u8; 4] {
        let o = ((y * ATLAS_W + x) * 4) as usize;
        [c.px[o], c.px[o + 1], c.px[o + 2], c.px[o + 3]]
    }

    #[test]
    fn the_diamond_is_two_to_one_and_has_no_holes() {
        let c = canvas();
        let rect = top_rect(Material::Grass, 1, 0);
        let (ox, oy) = (rect.min.x as u32, rect.min.y as u32);
        let mut covered = 0u32;
        for row in 0..TOP_H {
            let half = diamond_half(row);
            for x in 0..TOP_W {
                let inside = x >= TOP_W / 2 - half && x < TOP_W / 2 + half;
                let opaque = texel(&c, ox + x, oy + row)[3] == 255;
                assert_eq!(inside, opaque, "({x}, {row}) disagrees with the mask");
                covered += u32::from(opaque);
            }
        }
        // A diamond covers half its bounding box, give or take the two-texel
        // rows that make the slant land on whole pixels.
        let area = TOP_W * TOP_H;
        assert!(covered > area / 2 && covered < area * 2 / 3, "{covered}");
        // Widest in the middle, narrowest at the points — a diamond, not a box.
        assert_eq!(diamond_half(TOP_H / 2 - 1) * 2, TOP_W);
        assert_eq!(diamond_half(0), 2);
    }

    /// Neighbouring diamonds interlock: no seam of background between them.
    #[test]
    fn diamonds_tile_the_plane_without_gaps() {
        let c = canvas();
        let rect = top_rect(Material::Grass, 1, 0);
        let (ox, oy) = (rect.min.x as u32, rect.min.y as u32);
        // A tile's own coverage, in screen offsets from its centre.
        let mut covered = std::collections::HashSet::new();
        let mut stamp = |dx: i32, dy: i32| {
            for row in 0..TOP_H {
                let half = diamond_half(row);
                for x in (TOP_W / 2 - half)..(TOP_W / 2 + half) {
                    if texel(&c, ox + x, oy + row)[3] == 255 {
                        covered.insert((
                            dx + x as i32 - TOP_W as i32 / 2,
                            dy + row as i32 - TOP_H as i32 / 2,
                        ));
                    }
                }
            }
        };
        // The tile and its eight neighbours, at their projected offsets.
        for (dx, dy) in [
            (0, 0),
            (32, -16),
            (-32, 16),
            (-32, -16),
            (32, 16),
            (64, 0),
            (-64, 0),
            (0, -32),
            (0, 32),
        ] {
            stamp(dx, dy);
        }
        // The centre tile's neighbourhood must be solid: sweep the inner region
        // and find no hole.
        for y in -12..=12 {
            for x in -24..=24 {
                assert!(
                    covered.contains(&(x, y)),
                    "hole at ({x}, {y}) between interlocking diamonds"
                );
            }
        }
    }

    #[test]
    fn a_face_is_opaque_from_its_rim_down() {
        let c = canvas();
        for side in [Side::West, Side::South] {
            let rect = face_rect(Material::Rock, 2, side, MAX_FACE);
            let ox = rect.min.x as u32;
            for col in 0..FACE_W {
                let top = match side {
                    Side::West => col / 2,
                    Side::South => (TOP_H / 2).saturating_sub(col.div_ceil(2)),
                };
                assert_eq!(
                    texel(&c, ox + col, FACE_Y + top)[3],
                    255,
                    "{side:?} face column {col} has no rim"
                );
                // The bottom of the *face band* — which is no longer the bottom
                // of the atlas, now that the path diamonds sit below it.
                assert_eq!(
                    texel(&c, ox + col, FACE_Y + FACE_H - 1)[3],
                    255,
                    "{side:?} face column {col} does not reach the bottom"
                );
                if top > 0 {
                    assert_eq!(
                        texel(&c, ox + col, FACE_Y + top - 1)[3],
                        0,
                        "{side:?} face column {col} spills above its rim"
                    );
                }
            }
        }
    }

    /// The two visible faces must not read as one tone, or a corner vanishes.
    #[test]
    fn the_lit_face_and_the_shaded_face_differ_where_the_ramp_allows() {
        use crate::map::terrain::material::lightness;
        // Mean lightness of a tone, dither included.
        let value = |t: Tone| match t.dither {
            Some(other) => (lightness(t.solid) + lightness(other)) * 0.5,
            None => lightness(t.solid),
        };
        for material in MATERIALS {
            // Every fill shade, including the bottom of the ramp — which is
            // where both faces used to collapse to flat `OUTLINE`.
            for shade in 0..FILL_SHADES {
                let (west, south) = face_tones(material, shade);
                let top = lightness(material.step(shade));
                assert!(
                    value(west) > value(south),
                    "{material:?} shade {shade} lights the wrong face"
                );
                assert!(
                    top > value(west),
                    "{material:?} shade {shade} face is not darker than its own ground"
                );
                assert!(
                    west != south,
                    "{material:?} shade {shade} draws one tone on two faces"
                );
            }
        }
    }

    /// The face crop's flat bottom overhangs the true slant by up to half a
    /// diamond. That overhang has to land inside the neighbour it abuts, which
    /// draws one row nearer the camera and covers it.
    #[test]
    fn a_face_overhang_lands_inside_the_neighbour_it_abuts() {
        // Work in screen offsets from the tile centre, `depth` px of drop.
        let depth = 40.0f32;
        for side in [Side::West, Side::South] {
            // Where the neighbour's centre sits once its own (lower) lift is in.
            let (nx, ny) = match side {
                Side::West => (-(TOP_W as f32) / 2.0, -(TOP_H as f32) / 2.0 - depth),
                Side::South => (TOP_W as f32 / 2.0, -(TOP_H as f32) / 2.0 - depth),
            };
            // Bottom-most corner of the crop: the far end of the flat bottom.
            let (cx, cy) = match side {
                Side::West => (-(TOP_W as f32) / 2.0, -(TOP_H as f32) / 2.0 - depth),
                Side::South => (TOP_W as f32 / 2.0, -(TOP_H as f32) / 2.0 - depth),
            };
            // Every corner of the overhang triangle is inside the neighbour's
            // diamond: |dx| / 2 + |dy| <= 16.
            let inside = |px: f32, py: f32| {
                (px - nx).abs() * 0.5 + (py - ny).abs() <= TOP_H as f32 / 2.0 + 0.001
            };
            assert!(inside(cx, cy), "{side:?} crop corner escapes its neighbour");
            // The two rim endpoints the overhang hangs from.
            let rim = match side {
                Side::West => [(-(TOP_W as f32) / 2.0, -depth), (0.0, -16.0 - depth)],
                Side::South => [(0.0, -16.0 - depth), (TOP_W as f32 / 2.0, -depth)],
            };
            for (px, py) in rim {
                assert!(
                    inside(px, py),
                    "{side:?} overhang corner ({px}, {py}) escapes"
                );
            }
        }
    }

    #[test]
    fn faces_appear_exactly_where_the_ground_steps_down() {
        let mut map = MapGrid::empty(8, 8, 1);
        for y in 0..8i32 {
            for x in 0..8i32 {
                *map.get_mut(TileCoord { x, y }).unwrap() = Tile {
                    height: if x >= 4 { 9 } else { 0 },
                    water: false,
                    kind: if x >= 4 {
                        TerrainKind::Hills
                    } else {
                        TerrainKind::Plains
                    },
                };
            }
        }
        let floor = -BORDER_DROP;
        // The first column of the plateau shows a west face; the rest do not.
        let step = TileCoord { x: 4, y: 4 };
        assert_eq!(
            face_depth(&map, step, Side::West, floor),
            (9.0 * ISO_LIFT) as u32
        );
        assert_eq!(
            face_depth(&map, TileCoord { x: 5, y: 4 }, Side::West, floor),
            0
        );
        // Flat ground along the run shows nothing to the south.
        assert_eq!(face_depth(&map, step, Side::South, floor), 0);
        // ... except on the border, where the plinth drops.
        assert!(face_depth(&map, TileCoord { x: 4, y: 0 }, Side::South, floor) > 0);
    }

    #[test]
    fn water_never_carves_a_cliff_out_of_its_bed() {
        let mut map = MapGrid::empty(8, 8, 1);
        for y in 0..8i32 {
            for x in 0..8i32 {
                *map.get_mut(TileCoord { x, y }).unwrap() = Tile {
                    height: -6,
                    water: true,
                    kind: TerrainKind::Water,
                };
            }
        }
        // A lake is flat: every interior tile is level with its neighbours.
        for y in 1..8i32 {
            for x in 1..8i32 {
                let c = TileCoord { x, y };
                assert_eq!(face_depth(&map, c, Side::West, -6), 0);
                assert_eq!(face_depth(&map, c, Side::South, -6), 0);
            }
        }
    }

    #[test]
    fn a_real_map_stays_inside_the_atlas_depth() {
        let map = generate_map(64, 64, DEFAULT_MAP_SEED);
        let floor = plinth_floor(&map);
        let mut deepest = 0;
        for y in 0..64i32 {
            for x in 0..64i32 {
                for side in [Side::West, Side::South] {
                    deepest = deepest.max(face_depth(&map, TileCoord { x, y }, side, floor));
                }
            }
        }
        assert!(deepest > 0, "a generated map drew no cliffs at all");
        assert!(
            deepest <= MAX_FACE,
            "a face wants {deepest} texels, atlas serves {MAX_FACE}"
        );
    }

    /// Hold the isometric projection for a test. This renderer only ever runs
    /// under it, and `spawn_terrain` places every diamond through
    /// `tile_to_world`, so a test that did not install it would be measuring
    /// diamonds laid out on a square grid.
    fn iso() -> crate::map::tests::ProjectionGuard {
        crate::map::tests::ProjectionGuard::new(rail_map::Projection::Iso)
    }

    fn terrain_app(width: u32, height: u32) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<Assets<Image>>();
        app.init_resource::<IsoTerrainState>();
        app.insert_resource(generate_map(width, height, DEFAULT_MAP_SEED));
        // No separate spawn step: the atlas is baked at startup and
        // `rebuild_iso_terrain` builds whatever it finds missing, which is the
        // path a flip into this view takes as well.
        app.add_systems(Startup, setup_iso_atlas);
        app.add_systems(Update, rebuild_iso_terrain);
        // `rebuild_iso_terrain` installs the height field, so this app writes
        // the projection globals from its own schedule the way a real one does
        // — without `MapPlugin` here to own them on its behalf. Last, so every
        // schedule the plugins added is already there to be pinned.
        crate::map::tests::own_globals_for(&mut app);
        app
    }

    fn terrain_sprites(app: &mut App) -> usize {
        app.world_mut()
            .query_filtered::<Entity, With<IsoTerrain>>()
            .iter(app.world())
            .count()
    }

    /// The whole map is one draw's worth of sprites, all from one atlas.
    #[test]
    fn every_tile_gets_a_diamond_from_the_one_atlas() {
        let _iso = iso();
        let mut app = terrain_app(32, 32);
        app.update();
        let tiles = 32 * 32;
        let drawn = terrain_sprites(&mut app);
        assert!(drawn >= tiles, "{drawn} sprites for {tiles} tiles");
        // Tops plus at most two faces each, and a real map is mostly flat.
        assert!(drawn < tiles * 3);

        let atlas = app.world().resource::<IsoTerrainAtlas>().0.clone();
        let handles: std::collections::HashSet<_> = app
            .world_mut()
            .query_filtered::<&Sprite, With<IsoTerrain>>()
            .iter(app.world())
            .map(|s| s.image.clone())
            .collect();
        assert_eq!(
            handles.into_iter().collect::<Vec<_>>(),
            vec![atlas],
            "terrain must all batch from one texture"
        );
    }

    /// A new world replaces the old one — sprites, heights and all. This is the
    /// path that used to leave a previous map's terrain on screen.
    #[test]
    fn swapping_the_map_redraws_it_and_moves_the_lift_with_it() {
        let _iso = iso();
        let mut app = terrain_app(32, 32);
        app.update();
        let before = terrain_sprites(&mut app);
        assert!(before > 0);

        // An idle frame costs nothing.
        app.update();
        assert_eq!(terrain_sprites(&mut app), before);

        // A different world of a different size.
        let mut swapped = generate_map(20, 20, 7);
        let peak = TileCoord { x: 10, y: 10 };
        {
            let tile = swapped.get_mut(peak).unwrap();
            tile.water = false;
            tile.height = 15;
        }
        app.insert_resource(swapped);
        app.update();
        let after = terrain_sprites(&mut app);
        assert!(after > 0);
        assert!(after < before, "a smaller world drew more sprites");
        // The projection's height field followed the world, so the peak lifts.
        assert_eq!(rail_map::tile_height(peak), 15);
        assert_eq!(
            rail_map::tile_to_world(peak).1 - rail_map::tile_to_world_flat(peak).1,
            15.0 * ISO_LIFT
        );
        crate::map::projection::clear_iso_heights();
    }

    // ── A picture, without a GPU ───────────────────────────────────────────
    //
    // `super::chunk`'s tests bake composited texels and measure them; this does
    // the same and then writes the result out, because on this branch the thing
    // under review is what it *looks like*. The compositor below is the renderer
    // in miniature — same atlas, same rects, same order — so a bug you can see
    // in the file is a bug that is on screen.

    /// Composite the map *and a railway* into an RGBA buffer, far row first.
    ///
    /// Terrain and track interleave by diagonal row exactly as `depth_z` makes
    /// the renderer interleave them — all of a row's ground, then all of its
    /// track, then the row in front. That ordering is the whole reason a
    /// climbing leg draws over the terrace it crosses and a descending one is
    /// read as a cutting, so a compositor that got it wrong would be lying
    /// about the one thing these pictures exist to show.
    fn composite_iso_with_track(
        map: &MapGrid,
        atlas: &Canvas,
        view: (i32, i32, u32, u32),
        network: &rail_sim::TrackNetwork,
    ) -> Vec<u8> {
        let (vx, vy, vw, vh) = view;
        let mut out = composite_iso(map, atlas, view);

        let mut order: Vec<(TileCoord, u16, bool)> = network
            .iter()
            .map(|p| (p.tile, p.links.0, p.is_bridge()))
            .collect();
        order.sort_by_key(|(c, ..)| std::cmp::Reverse(c.x + c.y));
        // Track sits above the ground of its own row but behind the row in
        // front, and every rail tile here is on ground that has already been
        // laid, so painting them far-row-first over the finished terrain is the
        // same picture the renderer builds.
        for (coord, links, bridge) in order {
            let (cell, px) = crate::track::test_cell(coord, links, bridge);
            let (sx, sy) = tile_to_world(coord);
            let left = sx - cell as f32 / 2.0;
            let top = -sy - cell as f32 / 2.0;
            for row in 0..cell as i32 {
                let dy = top as i32 + row - vy;
                if dy < 0 || dy >= vh as i32 {
                    continue;
                }
                for col in 0..cell as i32 {
                    let dx = left as i32 + col - vx;
                    if dx < 0 || dx >= vw as i32 {
                        continue;
                    }
                    let s = ((row as u32 * cell + col as u32) * 4) as usize;
                    if px[s + 3] == 0 {
                        continue;
                    }
                    let d = ((dy as u32 * vw + dx as u32) * 4) as usize;
                    out[d..d + 4].copy_from_slice(&px[s..s + 4]);
                }
            }
        }
        out
    }

    /// Composite the map into an RGBA buffer, far row first.
    fn composite_iso(map: &MapGrid, atlas: &Canvas, view: (i32, i32, u32, u32)) -> Vec<u8> {
        composite_iso_worn(map, atlas, &PathWear::default(), view)
    }

    fn composite_iso_worn(
        map: &MapGrid,
        atlas: &Canvas,
        paths: &PathWear,
        view: (i32, i32, u32, u32),
    ) -> Vec<u8> {
        let (vx, vy, vw, vh) = view;
        // `BG0`, so the plinth reads against the same ground the game clears to.
        let bg = rgba(crate::palette::BG0);
        let mut out = vec![0u8; (vw * vh) as usize * 4];
        for px in out.chunks_exact_mut(4) {
            px.copy_from_slice(&bg);
        }

        // Blit one atlas rect, clipped to the view. Screen y is up; image y down.
        let mut blit = |rect: Rect, left: f32, top: f32| {
            let (sw, sh) = (
                (rect.max.x - rect.min.x) as i32,
                (rect.max.y - rect.min.y) as i32,
            );
            for row in 0..sh {
                let dy = top as i32 + row - vy;
                if dy < 0 || dy >= vh as i32 {
                    continue;
                }
                for col in 0..sw {
                    let dx = left as i32 + col - vx;
                    if dx < 0 || dx >= vw as i32 {
                        continue;
                    }
                    let s = (((rect.min.y as i32 + row) as u32 * ATLAS_W
                        + (rect.min.x as i32 + col) as u32)
                        * 4) as usize;
                    if atlas.px[s + 3] == 0 {
                        continue;
                    }
                    let d = ((dy as u32 * vw + dx as u32) * 4) as usize;
                    out[d..d + 4].copy_from_slice(&atlas.px[s..s + 4]);
                }
            }
        };

        let floor = plinth_floor(map);
        let mut order: Vec<TileCoord> = (0..map.height as i32)
            .flat_map(|y| (0..map.width as i32).map(move |x| TileCoord { x, y }))
            .collect();
        // Far rows first, so the near ones paint over them — which is exactly
        // what `iso_depth::depth_z` makes the renderer do.
        order.sort_by_key(|c| std::cmp::Reverse(c.x + c.y));

        for coord in order {
            let tile = map.tile(coord);
            let material = material_of(tile.kind);
            let shade = shade_for(tile.kind, tile.height);
            let (sx, sy) = tile_to_world(coord);
            // Screen y up -> image y down: the top of the map is image row 0.
            let top = -sy - TOP_H as f32 / 2.0;
            blit(
                top_rect(material, shade, iso_variant_for(coord)),
                sx - TOP_W as f32 / 2.0,
                top,
            );
            // The worn ground, straight over its own diamond — exactly what
            // `sync_iso_paths` spawns at `PATH_LIFT` above the tile.
            let level = paths.level_at(coord);
            if level > 0 && !tile.water && path_tones(material, shade).is_some() {
                blit(
                    path_rect(shade, level, path_variant_for(coord)),
                    sx - TOP_W as f32 / 2.0,
                    top,
                );
            }
            for side in [Side::West, Side::South] {
                let depth = face_depth(map, coord, side, floor).min(MAX_FACE);
                if depth == 0 {
                    continue;
                }
                blit(
                    face_rect(material, shade, side, depth),
                    sx + side.x_offset() - FACE_W as f32 / 2.0,
                    -sy,
                );
            }
        }
        out
    }

    /// Minimal PNG writer: stored (uncompressed) deflate blocks in a zlib
    /// wrapper. No dependency, and the file only has to open once.
    fn write_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> std::io::Result<()> {
        fn crc32(bytes: &[u8]) -> u32 {
            let mut c: u32 = 0xffff_ffff;
            for &b in bytes {
                c ^= b as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 {
                        0xedb8_8320 ^ (c >> 1)
                    } else {
                        c >> 1
                    };
                }
            }
            !c
        }
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            let mut crc_input = kind.to_vec();
            crc_input.extend_from_slice(body);
            out.extend_from_slice(&crc_input);
            out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
        }

        // Filter byte 0 in front of every scanline.
        let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
        for row in 0..h as usize {
            raw.push(0u8);
            let start = row * w as usize * 4;
            raw.extend_from_slice(&rgba[start..start + w as usize * 4]);
        }

        let mut z = vec![0x78, 0x01]; // zlib header, no compression preset
        let mut adler: (u32, u32) = (1, 0);
        for &b in &raw {
            adler.0 = (adler.0 + b as u32) % 65521;
            adler.1 = (adler.1 + adler.0) % 65521;
        }
        for (i, block) in raw.chunks(65535).enumerate() {
            let last = (i + 1) * 65535 >= raw.len();
            z.push(u8::from(last));
            z.extend_from_slice(&(block.len() as u16).to_le_bytes());
            z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
            z.extend_from_slice(block);
        }
        z.extend_from_slice(&((adler.1 << 16) | adler.0).to_be_bytes());

        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, no interlace
        chunk(&mut png, b"IHDR", &ihdr);
        chunk(&mut png, b"IDAT", &z);
        chunk(&mut png, b"IEND", &[]);
        std::fs::write(path, png)
    }

    /// Write a composited iso view of the default seed to `/tmp`.
    ///
    /// `cargo test -p rail_town --bin rail_town -- --ignored dump_iso_screenshot --nocapture`
    #[ignore = "writes a file; run it deliberately to look at the projection"]
    #[test]
    fn dump_iso_screenshot() {
        let _iso = iso();
        let map = generate_map(64, 64, DEFAULT_MAP_SEED);
        crate::map::projection::set_iso_heights(&map);
        let atlas = canvas();

        // A window on the middle of the map, roughly a 1440p frame's worth.
        let (cx, cy) = rail_map::map_center_world(map.width, map.height);
        let (vw, vh) = (1920u32, 1080u32);
        let view = (
            (cx - vw as f32 / 2.0) as i32,
            (-cy - vh as f32 / 2.0) as i32,
            vw,
            vh,
        );
        let pixels = composite_iso(&map, &atlas, view);

        // Nothing may be left transparent: the plinth and the background cover
        // everything the tiles do not.
        assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));

        let path = std::path::Path::new("/tmp/rail_town_iso.png");
        write_png(path, vw, vh, &pixels).expect("write the screenshot");
        eprintln!("wrote {}", path.display());
        crate::map::projection::clear_iso_heights();
    }

    /// Write the two pictures brief 15 has to be judged on.
    ///
    /// `cargo test -p rail_town --bin rail_town -- --ignored dump_iso_track --nocapture`
    ///
    /// Purpose-built scenes rather than the default seed, because the thing
    /// under review is what a railway does where the ground steps, and the
    /// default map's track is on the flat.
    #[ignore = "writes files; run it deliberately to look at the track"]
    #[test]
    fn dump_iso_track() {
        let _guard = crate::map::tests::ProjectionGuard::new(rail_map::Projection::Iso);
        let atlas = canvas();

        // ── A climbing S-curve on a hillside ───────────────────────────────
        //
        // Ground that rises to the north-east in bands, and a route that leans
        // into the slope, straightens, then leans the other way — so every leg
        // in the curve climbs, and the sleeper rhythm has to carry around the
        // bend as well as across each boundary.
        let mut hill = MapGrid::empty(28, 28, 1);
        for y in 0..28i32 {
            for x in 0..28i32 {
                let tile = hill.get_mut(TileCoord { x, y }).unwrap();
                tile.height = ((x + y) / 2).clamp(0, 12) as i8;
                tile.kind = if tile.height >= 8 {
                    TerrainKind::Mountain
                } else if tile.height >= 4 {
                    TerrainKind::Hills
                } else {
                    TerrainKind::Plains
                };
            }
        }
        // Out of the rose, adjacent steps only, so the curve is one a player
        // could lay: east, easing left to north, then back right to east.
        let mut route = vec![TileCoord { x: 3, y: 5 }];
        for step in [
            (1, 0),
            (2, 1),
            (1, 1),
            (1, 2),
            (0, 1),
            (0, 1),
            (1, 2),
            (1, 1),
            (2, 1),
            (1, 0),
            (1, 0),
            (2, 1),
            (1, 1),
            (1, 2),
        ] {
            let last = *route.last().unwrap();
            route.push(TileCoord {
                x: last.x + step.0,
                y: last.y + step.1,
            });
        }
        write_scene(
            &hill,
            &atlas,
            &route,
            "/tmp/rail_town_iso_track_hillside.png",
            2,
        );

        // ── A straight ramp beside a cliff face ────────────────────────────
        //
        // A plateau six bands up with a hard southern edge, and a notch cut
        // five tiles wide for a straight run to climb through. The full
        // 24 px face stands either side of the cutting, so the frame holds the
        // ramp and the terrace it crosses at once — a railway refusing to
        // follow the contour, next to the contour it is refusing.
        const TOP: i32 = 6;
        const NOTCH: i32 = 10;
        let mut cliff = MapGrid::empty(24, 24, 1);
        for y in 0..24i32 {
            for x in 0..24i32 {
                let tile = cliff.get_mut(TileCoord { x, y }).unwrap();
                let plateau = if y <= 11 { 0 } else { TOP };
                tile.height = if (x - NOTCH).abs() <= 2 {
                    // The cutting floor: level, then two a tile up to the top.
                    plateau.min(((y - 11) * 2).max(0))
                } else {
                    plateau
                } as i8;
                tile.kind = if tile.height >= 4 {
                    TerrainKind::Hills
                } else {
                    TerrainKind::Plains
                };
            }
        }
        let straight: Vec<TileCoord> = (5..20).map(|y| TileCoord { x: NOTCH, y }).collect();
        write_scene(&cliff, &atlas, &straight, "/tmp/rail_town_iso_track_cliff.png", 2);

        crate::map::projection::clear_iso_heights();
    }

    /// Lay a railway through the real placement rules, frame it, and write it
    /// out at an integer zoom.
    ///
    /// The route goes down through `rail_sim::try_place_path` rather than being
    /// asserted into place, so a picture can only ever be of a railway a player
    /// could actually have built — grades, terrain and half-step clearances all
    /// checked by the code that checks them in the game.
    fn write_scene(map: &MapGrid, atlas: &Canvas, route: &[TileCoord], path: &str, zoom: u32) {
        use rail_sim::{Money, MoneyLedger, TrackNetwork, TrackTerrain, GROUND_LAYER};

        crate::map::projection::set_iso_heights(map);
        let terrain = TrackTerrain::new(
            map.width,
            map.height,
            map.tiles().iter().map(|t| (t.water, t.height)),
        );
        let mut network = TrackNetwork::new();
        rail_sim::track::try_place_path(
            &mut network,
            &mut Money::new(1_000_000_000),
            &mut MoneyLedger::default(),
            &terrain,
            route,
            GROUND_LAYER,
        )
        .unwrap_or_else(|e| panic!("{path}: the scene's route is not buildable: {e:?}"));
        assert_eq!(network.len(), route.len(), "{path}: the route lost a tile");
        // Every consecutive pair has to have actually linked, or the picture is
        // of a broken railway and proves nothing.
        for pair in route.windows(2) {
            let dir = rail_sim::track::dir_index(pair[0], pair[1]).expect("a rose step");
            assert!(
                network
                    .at(pair[0], GROUND_LAYER)
                    .is_some_and(|p| p.links.has(dir)),
                "{path}: {:?} did not link to {:?}",
                pair[0],
                pair[1]
            );
        }

        // Centre on the middle of the run, and show a window the zoom will fill.
        let (cx, cy) = tile_to_world(route[route.len() / 2]);
        let (out_w, out_h) = (1280u32, 720u32);
        let (vw, vh) = (out_w / zoom, out_h / zoom);
        let view = (
            (cx - vw as f32 / 2.0) as i32,
            (-cy - vh as f32 / 2.0) as i32,
            vw,
            vh,
        );
        let pixels = composite_iso_with_track(map, atlas, view, &network);
        assert!(
            pixels.chunks_exact(4).all(|p| p[3] == 255),
            "{path}: the frame has a hole in it"
        );

        // Nearest-neighbour, whole numbers only — the pixel contract's zoom
        // rungs (01 §2.1), so what is written is what a zoomed-in player sees.
        let mut scaled = vec![0u8; (out_w * out_h) as usize * 4];
        for y in 0..out_h {
            for x in 0..out_w {
                let s = (((y / zoom) * vw + (x / zoom)) * 4) as usize;
                let d = ((y * out_w + x) * 4) as usize;
                scaled[d..d + 4].copy_from_slice(&pixels[s..s + 4]);
            }
        }
        write_png(std::path::Path::new(path), out_w, out_h, &scaled).expect("write the screenshot");
        eprintln!("wrote {path}");
    }

    // ── Desire paths (brief 16 §3.4) ───────────────────────────────────────

    /// A lane of worn tiles running east across the middle of a map, at every
    /// wear level, so a picture of it shows the whole ladder at once.
    fn worn_lane(map: &MapGrid) -> PathWear {
        let mut paths = PathWear::new(map.width, map.height);
        let y = map.height as i32 / 2;
        for (i, x) in (4..map.width as i32 - 4).enumerate() {
            let footfalls = match i % 9 {
                0..=2 => 4,   // Faint
                3..=5 => 10,  // Worn
                _ => 40,      // Bare
            };
            for _ in 0..footfalls {
                paths.add_footfall(TileCoord { x, y });
            }
        }
        paths
    }

    /// Writes a picture of a worn lane in isometric.
    ///
    /// `cargo test -p rail_town --bin rail_town -- --ignored dump_iso_paths_screenshot --nocapture`
    #[ignore = "writes a file; run it deliberately to look at the paths"]
    #[test]
    fn dump_iso_paths_screenshot() {
        let _iso = iso();
        let map = generate_map(64, 64, DEFAULT_MAP_SEED);
        crate::map::projection::set_iso_heights(&map);
        let atlas = canvas();
        let paths = worn_lane(&map);

        let (cx, cy) = rail_map::map_center_world(map.width, map.height);
        let (vw, vh) = (1920u32, 1080u32);
        let view = (
            (cx - vw as f32 / 2.0) as i32,
            (-cy - vh as f32 / 2.0) as i32,
            vw,
            vh,
        );
        let pixels = composite_iso_worn(&map, &atlas, &paths, view);
        assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));

        let path = std::path::Path::new("/tmp/rail_town_iso_paths.png");
        write_png(path, vw, vh, &pixels).expect("write the screenshot");
        eprintln!("wrote {}", path.display());

        // A 3x nearest-neighbour crop of the lane, because the whole point of
        // three wear steps is that they are three *different* steps, and at 1:1
        // in a 1920-wide frame that is a judgement nobody can actually make.
        let (zw, zh, zoom) = (480u32, 120u32, 3u32);
        let (ox, oy) = (vw / 2 - zw / 2, vh / 2 - zh / 2);
        let mut crop = vec![0u8; (zw * zoom * zh * zoom) as usize * 4];
        for y in 0..zh * zoom {
            for x in 0..zw * zoom {
                let s = (((oy + y / zoom) * vw + ox + x / zoom) * 4) as usize;
                let d = ((y * zw * zoom + x) * 4) as usize;
                crop[d..d + 4].copy_from_slice(&pixels[s..s + 4]);
            }
        }
        let zoomed = std::path::Path::new("/tmp/rail_town_iso_paths_zoom.png");
        write_png(zoomed, zw * zoom, zh * zoom, &crop).expect("write the zoom");
        eprintln!("wrote {}", zoomed.display());
        crate::map::projection::clear_iso_heights();
    }

    #[test]
    fn a_bridge_deck_and_a_beach_never_draw_a_path_in_isometric() {
        let _iso = iso();
        let mut app = paths_app(24, 24);
        {
            let mut map = app.world_mut().resource_mut::<MapGrid>();
            for (x, kind, height, water) in [
                (4i32, TerrainKind::Water, -4i8, true),
                (8, TerrainKind::Beach, 0, false),
                (12, TerrainKind::Mountain, 16, false),
            ] {
                *map.get_mut(TileCoord { x, y: 6 }).unwrap() = Tile {
                    height,
                    water,
                    kind,
                };
            }
        }
        // Walk all of them to saturation, and one ordinary grass tile too.
        for x in [4i32, 8, 12, 16] {
            wear(&mut app, TileCoord { x, y: 6 }, 40);
        }
        app.update();
        assert_eq!(
            path_sprites(&mut app),
            vec![TileCoord { x: 16, y: 6 }],
            "only ground with grass on it may draw a path"
        );
    }

    /// The same lane, measured rather than looked at.
    #[test]
    fn a_worn_lane_changes_the_picture_in_isometric() {
        let _iso = iso();
        let map = flat_grass(24, 24);
        crate::map::projection::set_iso_heights(&map);
        let atlas = canvas();
        let (cx, cy) = rail_map::map_center_world(map.width, map.height);
        let (vw, vh) = (900u32, 600u32);
        let view = (
            (cx - vw as f32 / 2.0) as i32,
            (-cy - vh as f32 / 2.0) as i32,
            vw,
            vh,
        );

        let clean = composite_iso(&map, &atlas, view);
        let worn = composite_iso_worn(&map, &atlas, &worn_lane(&map), view);
        let changed = clean
            .chunks_exact(4)
            .zip(worn.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        assert!(changed > 2_000, "the lane barely marked the ground: {changed}");

        // Every texel the lane changed is a path colour and nothing else.
        let allowed: std::collections::HashSet<[u8; 4]> = (0..FILL_SHADES)
            .flat_map(|s| [rgba(PATH_FILL[s]), rgba(PATH_DUST[s])])
            .collect();
        for (a, b) in clean.chunks_exact(4).zip(worn.chunks_exact(4)) {
            if a == b {
                continue;
            }
            let px = [b[0], b[1], b[2], b[3]];
            assert!(allowed.contains(&px), "a path drew off its own ramp: {px:?}");
        }
        crate::map::projection::clear_iso_heights();
    }

    /// Flat plains, so every tile is band-0 grass and can take a path. A
    /// generated map would put water and mountain under the test's fixtures.
    fn flat_grass(w: u32, h: u32) -> MapGrid {
        let mut map = MapGrid::empty(w, h, 1);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                *map.get_mut(TileCoord { x, y }).unwrap() = Tile {
                    height: 0,
                    water: false,
                    kind: TerrainKind::Plains,
                };
            }
        }
        map
    }

    fn paths_app(width: u32, height: u32) -> App {
        let mut app = terrain_app(width, height);
        app.insert_resource(flat_grass(width, height));
        // Sized, not defaulted: a `PathWear` of no size records no footfalls,
        // and a test that never wears anything passes for the wrong reason.
        app.insert_resource(PathWear::new(width, height));
        app.add_systems(Update, sync_iso_paths.after(rebuild_iso_terrain));
        app
    }

    fn path_sprites(app: &mut App) -> Vec<TileCoord> {
        let mut tiles: Vec<TileCoord> = app
            .world_mut()
            .query::<&IsoPath>()
            .iter(app.world())
            .map(|p| p.0)
            .collect();
        tiles.sort_by_key(|t| (t.y, t.x));
        tiles
    }

    fn wear(app: &mut App, tile: TileCoord, footfalls: u32) {
        let mut paths = app.world_mut().resource_mut::<PathWear>();
        let before = paths.wear_at(tile);
        for _ in 0..footfalls {
            paths.add_footfall(tile);
        }
        assert!(
            paths.wear_at(tile) > before,
            "the fixture recorded no wear at {tile:?} — is the map sized?"
        );
    }

    #[test]
    fn a_worn_tile_gets_a_sprite_and_a_clean_one_does_not() {
        let _iso = iso();
        let mut app = paths_app(32, 32);
        app.update();
        assert!(path_sprites(&mut app).is_empty(), "clean ground drew a path");

        // Under the threshold: still nothing.
        let tile = TileCoord { x: 6, y: 6 };
        wear(&mut app, tile, 3);
        app.update();
        assert!(path_sprites(&mut app).is_empty(), "sub-threshold wear drew");

        // Over it: exactly one sprite, on that tile.
        wear(&mut app, tile, 1);
        app.update();
        assert_eq!(path_sprites(&mut app), vec![tile]);

        // It draws from the shared atlas, so it batches with the ground.
        let atlas = app.world().resource::<IsoTerrainAtlas>().0.clone();
        let sprite = app
            .world_mut()
            .query_filtered::<&Sprite, With<IsoPath>>()
            .iter(app.world())
            .next()
            .expect("a path sprite")
            .clone();
        assert_eq!(sprite.image, atlas);
        let rect = sprite.rect.expect("a path samples one cell");
        assert!(rect.min.y >= PATH_Y as f32, "a path sampled the ground's cells");
    }

    #[test]
    fn deepening_a_path_repoints_the_same_sprite() {
        let _iso = iso();
        let mut app = paths_app(32, 32);
        let tile = TileCoord { x: 9, y: 9 };
        wear(&mut app, tile, 4);
        app.update();
        let faint = app
            .world_mut()
            .query_filtered::<&Sprite, With<IsoPath>>()
            .iter(app.world())
            .next()
            .expect("a path sprite")
            .rect;

        wear(&mut app, tile, 6);
        app.update();
        let sprites: Vec<Rect> = app
            .world_mut()
            .query_filtered::<&Sprite, With<IsoPath>>()
            .iter(app.world())
            .filter_map(|s| s.rect)
            .collect();
        assert_eq!(sprites.len(), 1, "a deeper path spawned a second sprite");
        assert_ne!(Some(sprites[0]), faint, "the sprite kept its old art");
    }

    #[test]
    fn ground_that_grasses_over_loses_its_sprite() {
        let _iso = iso();
        let mut app = paths_app(32, 32);
        let tile = TileCoord { x: 3, y: 11 };
        wear(&mut app, tile, 4);
        app.update();
        assert_eq!(path_sprites(&mut app), vec![tile]);

        // Regrow it all the way back to clean ground.
        {
            let mut paths = app.world_mut().resource_mut::<PathWear>();
            for _ in 0..400 {
                paths.regrow();
            }
            assert_eq!(paths.level_at(tile), 0);
        }
        app.update();
        assert!(path_sprites(&mut app).is_empty(), "the path outlived its wear");
    }

    /// The budget: wear that crosses no threshold must not touch a sprite.
    #[test]
    fn wear_that_changes_no_level_spawns_nothing() {
        let _iso = iso();
        let mut app = paths_app(32, 32);
        app.update();

        for _ in 0..3 {
            for y in 0..10i32 {
                for x in 0..10i32 {
                    wear(&mut app, TileCoord { x, y }, 1);
                }
            }
        }
        for _ in 0..8 {
            app.update();
            assert!(
                path_sprites(&mut app).is_empty(),
                "sub-threshold wear drew a path"
            );
        }
    }

    #[test]
    fn a_resync_redraws_every_path() {
        let _iso = iso();
        let mut app = paths_app(32, 32);
        let tiles = [TileCoord { x: 2, y: 2 }, TileCoord { x: 5, y: 7 }];
        for tile in tiles {
            wear(&mut app, tile, 4);
        }
        app.update();
        assert_eq!(path_sprites(&mut app), vec![tiles[0], tiles[1]]);

        // What a save-load looks like from here.
        app.world_mut().resource_mut::<PathWear>().request_resync();
        app.update();
        assert_eq!(path_sprites(&mut app), vec![tiles[0], tiles[1]]);
        assert!(!app.world().resource::<PathWear>().needs_resync());
    }

    #[test]
    fn every_path_cell_of_the_atlas_is_painted() {
        let c = canvas();
        for shade in 0..FILL_SHADES {
            for level in 1..=PATH_LEVELS as u8 {
                for variant in 0..PATH_VARIANTS {
                    let r = path_rect(shade, level, variant);
                    // The centre of a diamond is the densest part of the mask,
                    // so at every level something is drawn there.
                    let mut painted = 0;
                    for dy in 0..8u32 {
                        for dx in 0..16u32 {
                            let px = texel(
                                &c,
                                r.min.x as u32 + TOP_W / 2 - 8 + dx,
                                r.min.y as u32 + TOP_H / 2 - 4 + dy,
                            );
                            if px[3] != 0 {
                                painted += 1;
                            }
                        }
                    }
                    assert!(
                        painted > 8,
                        "shade {shade} level {level} variant {variant} is blank"
                    );
                }
            }
        }
    }

    /// A path is a mask over the ground, never a second opaque diamond.
    #[test]
    fn a_path_cell_leaves_the_grass_it_has_not_worn() {
        let c = canvas();
        for shade in 0..FILL_SHADES {
            let r = path_rect(shade, 1, 0);
            let mut clear = 0;
            for row in 0..TOP_H {
                let half = diamond_half(row);
                for x in (TOP_W / 2 - half)..(TOP_W / 2 + half) {
                    if texel(&c, r.min.x as u32 + x, r.min.y as u32 + row)[3] == 0 {
                        clear += 1;
                    }
                }
            }
            assert!(
                clear > 300,
                "a faint path at shade {shade} left only {clear} texels of grass"
            );
        }
    }

    #[test]
    fn every_cell_of_the_atlas_is_painted() {
        let c = canvas();
        for material in MATERIALS {
            for shade in 0..FILL_SHADES {
                for variant in 0..ISO_VARIANTS {
                    let r = top_rect(material, shade, variant);
                    assert_eq!(
                        texel(&c, r.min.x as u32 + TOP_W / 2, r.min.y as u32 + TOP_H / 2)[3],
                        255,
                        "{material:?} shade {shade} variant {variant} top is blank"
                    );
                }
                for side in [Side::West, Side::South] {
                    let r = face_rect(material, shade, side, MAX_FACE);
                    assert_eq!(
                        texel(&c, r.min.x as u32 + FACE_W / 2, FACE_Y + FACE_H / 2)[3],
                        255,
                        "{material:?} shade {shade} {side:?} face is blank"
                    );
                }
            }
        }
    }
}
