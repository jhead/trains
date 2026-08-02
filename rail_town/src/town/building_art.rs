//! Procedural building art, baked once into a single atlas.
//!
//! There is no artist and no texture assets, so every silhouette here is drawn
//! texel-by-texel from the binding ramps in [`crate::palette`] (brief 01 §3).
//! The bake happens once at boot ([`bake_atlas`]) and never per frame — the
//! atlas is pure data, so the same frame index always draws the same pixels
//! wherever it appears (pixel contract §2.4: nothing hashes on screen or time).
//!
//! # Cell geometry
//!
//! A tile is 32 texels; a tile holds four 16×16 lots. Each atlas cell is
//! [`CELL_W`] × [`CELL_H`] = 16 × 32: the bottom 16 rows are the lot footprint
//! and the building draws upward past it, which is the fake front face from
//! brief 01 §6.1. Cell-local coordinates put `(0, 0)` at the **bottom left**.
//!
//! # Frame layout
//!
//! ```text
//!   0   .. 255   building     key * 4 + decay
//! 256   .. 319   settle       SETTLE_BASE + key      (2-frame settle, squashed)
//! 320   .. 383   lit windows  LIT_BASE    + key      (night layer, see below)
//! 384   .. 401   effects      stake / scaffold / scar / rural props
//! ```
//!
//! `key` is [`BuildingKind::key`] — family × tier × variant × roof.
//!
//! # Night lighting
//!
//! Lit windows are a **separate sprite layer** (brief 01 §3.4) owned elsewhere.
//! Every healthy building frame has a matching mask frame at
//! [`BuildingKind::lit_frame`] holding only its windows in `WIN_LIT`, drawn in
//! the same cell at the same texels — so the night layer is a second sprite
//! with the same transform, anchor and `flip_x`, and it lines up exactly.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler, TextureAtlasLayout, ToExtents};
use bevy::prelude::*;
use bevy::render::render_resource::{TextureDimension, TextureFormat};

use crate::palette::{
    BALLAST_M, GRASS_D, GRASS_L, GRASS_M, OUTLINE, PLASTER_D, PLASTER_L, PLASTER_M, ROCK_D, ROCK_L,
    ROCK_M, ROOF_SLATE_D, ROOF_SLATE_L, ROOF_SLATE_M, ROOF_TILE_D, ROOF_TILE_L, ROOF_TILE_M, SAND_D,
    SAND_L, SAND_M, WIN_DARK, WIN_LIT, WOOD_D, WOOD_L, WOOD_M,
};

// ─ Cell + atlas geometry ───────────────────────────────

/// Texel width of one lot cell (a lot is 16 × 16 of ground).
pub const CELL_W: u32 = 16;
/// Texel height of one lot cell; the top 16 rows are the fake front face.
pub const CELL_H: u32 = 32;
/// Cells per atlas row.
const ATLAS_COLS: u32 = 16;

const W: i32 = CELL_W as i32;

// ─ Frame index layout ──────────────────────────────────

const FAMILIES: usize = 2;
const TIERS: usize = 4;
const VARIANTS: usize = 4;
const ROOFS: usize = 2;
const DECAYS: usize = 4;

/// Distinct (family, tier, variant, roof) combinations.
pub const KEY_COUNT: usize = FAMILIES * TIERS * VARIANTS * ROOFS;
const BUILDING_FRAMES: usize = KEY_COUNT * DECAYS;
const SETTLE_BASE: usize = BUILDING_FRAMES;
const LIT_BASE: usize = SETTLE_BASE + KEY_COUNT;
const FX_BASE: usize = LIT_BASE + KEY_COUNT;

/// Surveyor's stake, two hashed variants (brief 06 §3.1 step 1).
pub const FRAME_STAKE: usize = FX_BASE;
/// Scaffold: four height classes × two frames (`hold` then `topped out`).
pub const FRAME_SCAFFOLD: usize = FX_BASE + 2;
/// Cleared lot with a persisting foundation scar, two variants.
pub const FRAME_SCAR: usize = FX_BASE + 10;
/// Rural props: field, haystack, hedge, lane, stone wall, tree.
pub const FRAME_RURAL: usize = FX_BASE + 12;
/// Number of rural prop frames.
#[allow(dead_code)] // Part of the atlas contract; used by tests and other slices.
pub const RURAL_PROPS: usize = 6;

/// Total baked frames.
pub const FRAME_COUNT: usize = FX_BASE + 18;

// ─ Kinds ───────────────────────────────────────────────

/// Which silhouette family a lot draws from.
///
/// [`Family::Town`] carries the four canonical tiers of brief 06 §2.1;
/// [`Family::Works`] is the goods-district set of §2.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    /// Cottage · Townhouse · Shopfront · Block.
    Town,
    /// Shed · Workshop · Yard · Warehouse.
    Works,
}

/// The two roof materials (brief 01 §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Roof {
    Tile,
    Slate,
}

/// Decline stages that change the *building* art (brief 06 §3.2).
///
/// `Cleared` is not here: a cleared lot draws [`FRAME_SCAR`] instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Decay {
    /// Occupied and served.
    Healthy,
    /// Windows go dark — the earliest, gentlest signal.
    Dimmed,
    /// Boarded windows, peeling paint, overgrown yard.
    Boarded,
    /// Holed roof, collapsed boards, overgrowth up the walls.
    Derelict,
}

impl Decay {
    /// One step further into decline, or `None` at the end of the ladder.
    pub fn worse(self) -> Option<Self> {
        match self {
            Self::Healthy => Some(Self::Dimmed),
            Self::Dimmed => Some(Self::Boarded),
            Self::Boarded => Some(Self::Derelict),
            Self::Derelict => None,
        }
    }

    /// One step back toward health — every stage reverses visibly.
    pub fn better(self) -> Option<Self> {
        match self {
            Self::Healthy => None,
            Self::Dimmed => Some(Self::Healthy),
            Self::Boarded => Some(Self::Dimmed),
            Self::Derelict => Some(Self::Boarded),
        }
    }
}

/// A fully resolved building: silhouette family, tier, variant and roof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BuildingKind {
    pub family: Family,
    /// `0..=3`, matching the tier table in brief 06 §2.1.
    pub tier: u8,
    /// `0..=3` — four variants per tier.
    pub variant: u8,
    pub roof: Roof,
}

impl BuildingKind {
    /// Dense index over (family, tier, variant, roof).
    pub fn key(self) -> usize {
        let family = self.family as usize;
        let tier = (self.tier as usize).min(TIERS - 1);
        let variant = (self.variant as usize).min(VARIANTS - 1);
        ((family * TIERS + tier) * VARIANTS + variant) * ROOFS + self.roof as usize
    }

    /// Atlas frame for this building at `decay`.
    pub fn frame(self, decay: Decay) -> usize {
        self.key() * DECAYS + decay as usize
    }

    /// Atlas frame for the squashed first half of the two-frame settle.
    pub fn settle_frame(self) -> usize {
        SETTLE_BASE + self.key()
    }

    /// Atlas frame holding only this building's windows in `WIN_LIT`.
    pub fn lit_frame(self) -> usize {
        LIT_BASE + self.key()
    }

    /// Height class `0..=3` used to pick the scaffold that precedes this build.
    pub fn scaffold_class(self) -> usize {
        self.tier as usize
    }
}

/// One window opening in cell-local texels, `y` measured from the cell bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WinRect {
    pub x: u8,
    pub y: u8,
    pub w: u8,
    pub h: u8,
}

// ─ Baked atlas resource ────────────────────────────────

/// Handles and metadata for the baked town atlas.
///
/// Inserted on the first run of the town sync system and never rebuilt.
#[derive(Resource, Debug, Clone)]
pub struct BuildingAtlas {
    pub image: Handle<Image>,
    pub layout: Handle<TextureAtlasLayout>,
    windows: Vec<Vec<WinRect>>,
}

impl BuildingAtlas {
    /// Window openings for `frame`, in cell-local texels from the cell bottom.
    ///
    /// Empty for effect frames and for frames whose windows are boarded over.
    /// The night-lighting layer normally wants the whole-frame mask instead
    /// ([`BuildingKind::lit_frame`]); this is here for per-window effects.
    #[allow(dead_code)] // Read by the night-lighting layer once `mod.rs` re-exports it.
    pub fn windows(&self, frame: usize) -> &[WinRect] {
        self.windows.get(frame).map(Vec::as_slice).unwrap_or(&[])
    }

    /// A sprite drawing `frame` from this atlas.
    pub fn sprite(&self, frame: usize) -> Sprite {
        Sprite::from_atlas_image(
            self.image.clone(),
            TextureAtlas {
                layout: self.layout.clone(),
                index: frame,
            },
        )
    }
}

/// World-anchored hash. Integer inputs only — never screen position, never time.
pub fn world_hash(x: i32, y: i32, salt: u32) -> u32 {
    let mut h = 0x9E37_79B9_u32;
    h ^= (x as u32).wrapping_mul(0x85EB_CA6B);
    h = h.rotate_left(13).wrapping_mul(0xC2B2_AE35);
    h ^= (y as u32).wrapping_mul(0x27D4_EB2F);
    h = h.rotate_left(11).wrapping_mul(0x1656_67B1);
    h ^= salt.wrapping_mul(0x9E37_79B1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^ (h >> 16)
}

/// Bake every frame into one nearest-sampled atlas image.
pub fn bake_atlas(
    images: &mut Assets<Image>,
    layouts: &mut Assets<TextureAtlasLayout>,
) -> BuildingAtlas {
    let rows = FRAME_COUNT.div_ceil(ATLAS_COLS as usize) as u32;
    let aw = ATLAS_COLS * CELL_W;
    let ah = rows * CELL_H;
    let mut data = vec![0u8; (aw * ah * 4) as usize];
    let mut windows = vec![Vec::new(); (ATLAS_COLS * rows) as usize];

    for (frame, slot) in windows.iter_mut().enumerate().take(FRAME_COUNT) {
        let (canvas, wins) = draw_frame(frame);
        blit(&mut data, aw, frame as u32, &canvas);
        *slot = wins;
    }

    let mut image = Image::new(
        UVec2::new(aw, ah).to_extents(),
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    // The app does not set `ImagePlugin::default_nearest`, so pin it here:
    // one texel is one screen pixel times a whole number (contract §2.1).
    image.sampler = ImageSampler::nearest();

    let layout = TextureAtlasLayout::from_grid(
        UVec2::new(CELL_W, CELL_H),
        ATLAS_COLS,
        rows,
        None,
        None,
    );

    BuildingAtlas {
        image: images.add(image),
        layout: layouts.add(layout),
        windows,
    }
}

fn blit(data: &mut [u8], atlas_w: u32, frame: u32, canvas: &Canvas) {
    let cx = (frame % ATLAS_COLS) * CELL_W;
    let cy = (frame / ATLAS_COLS) * CELL_H;
    for y in 0..CELL_H {
        // Canvas row 0 is the bottom; atlas row 0 is the top.
        let src_y = CELL_H - 1 - y;
        for x in 0..CELL_W {
            let px = canvas.get(x as i32, src_y as i32);
            if px[3] == 0 {
                continue;
            }
            let o = (((cy + y) * atlas_w + cx + x) * 4) as usize;
            data[o..o + 4].copy_from_slice(&px);
        }
    }
}

// ─ Canvas ──────────────────────────────────────────────

struct Canvas {
    px: Vec<[u8; 4]>,
}

impl Canvas {
    fn new() -> Self {
        Self {
            px: vec![[0; 4]; (CELL_W * CELL_H) as usize],
        }
    }

    #[inline]
    fn inside(x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < CELL_W as i32 && y < CELL_H as i32
    }

    #[inline]
    fn set(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if Self::inside(x, y) {
            self.px[(y * W + x) as usize] = c;
        }
    }

    /// Paint only where something is already drawn (keeps silhouettes clean).
    #[inline]
    fn tint(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if Self::inside(x, y) && self.px[(y * W + x) as usize][3] != 0 {
            self.px[(y * W + x) as usize] = c;
        }
    }

    #[inline]
    fn get(&self, x: i32, y: i32) -> [u8; 4] {
        if Self::inside(x, y) {
            self.px[(y * W + x) as usize]
        } else {
            [0; 4]
        }
    }

    fn hline(&mut self, x: i32, y: i32, len: i32, c: [u8; 4]) {
        for i in 0..len {
            self.set(x + i, y, c);
        }
    }

    fn vline(&mut self, x: i32, y: i32, len: i32, c: [u8; 4]) {
        for i in 0..len {
            self.set(x, y + i, c);
        }
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: [u8; 4]) {
        for j in 0..h {
            self.hline(x, y + j, w, c);
        }
    }

    /// Bounding box of drawn texels as `(x0, y0, x1, y1)` inclusive.
    fn bounds(&self) -> Option<(i32, i32, i32, i32)> {
        let (mut x0, mut y0, mut x1, mut y1) = (W, CELL_H as i32, -1, -1);
        for y in 0..CELL_H as i32 {
            for x in 0..W {
                if self.px[(y * W + x) as usize][3] != 0 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        (x1 >= 0).then_some((x0, y0, x1, y1))
    }
}

// ─ Colour helpers ──────────────────────────────────────

#[derive(Clone, Copy)]
struct Ramp {
    d: [u8; 4],
    m: [u8; 4],
    l: [u8; 4],
}

fn rgba(c: Color) -> [u8; 4] {
    let s = c.to_srgba();
    [
        (s.red * 255.0).round() as u8,
        (s.green * 255.0).round() as u8,
        (s.blue * 255.0).round() as u8,
        255,
    ]
}

fn rgba_a(c: Color, a: u8) -> [u8; 4] {
    let mut p = rgba(c);
    p[3] = a;
    p
}

fn plaster() -> Ramp {
    Ramp {
        d: rgba(PLASTER_D),
        m: rgba(PLASTER_M),
        l: rgba(PLASTER_L),
    }
}

fn timber() -> Ramp {
    Ramp {
        d: rgba(WOOD_D),
        m: rgba(WOOD_M),
        l: rgba(WOOD_L),
    }
}

fn stone() -> Ramp {
    Ramp {
        d: rgba(ROCK_D),
        m: rgba(ROCK_M),
        l: rgba(ROCK_L),
    }
}

fn roof_ramp(roof: Roof) -> Ramp {
    match roof {
        Roof::Tile => Ramp {
            d: rgba(ROOF_TILE_D),
            m: rgba(ROOF_TILE_M),
            l: rgba(ROOF_TILE_L),
        },
        Roof::Slate => Ramp {
            d: rgba(ROOF_SLATE_D),
            m: rgba(ROOF_SLATE_M),
            l: rgba(ROOF_SLATE_L),
        },
    }
}

fn glass() -> [u8; 4] {
    rgba(WIN_DARK)
}

/// Reflected sky in a pane — the pixel that vanishes when windows go dark.
fn glint() -> [u8; 4] {
    rgba(ROOF_SLATE_L)
}

/// Contact shadow under a building so it sits on the ground rather than floating.
///
/// `OUTLINE` is the only outline colour in the game (brief 01 §3.1); the alpha
/// is local to this module — see the report note about promoting it.
const SHADOW_ALPHA: u8 = 150;

// ─ Shared drawing primitives ───────────────────────────

fn ground_shadow(c: &mut Canvas, x: i32, w: i32) {
    c.hline(x - 1, 0, w + 2, rgba_a(OUTLINE, SHADOW_ALPHA));
}

fn draw_wall(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, r: Ramp) {
    c.rect(x, y, w, h, r.m);
    c.vline(x, y, h, r.l);
    c.vline(x + w - 1, y, h, r.d);
}

fn draw_window(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, sill: [u8; 4], wins: &mut Vec<WinRect>) {
    c.rect(x, y, w, h, glass());
    c.set(x, y + h - 1, glint());
    c.hline(x, y - 1, w, sill);
    wins.push(WinRect {
        x: x.clamp(0, W - 1) as u8,
        y: y.clamp(0, CELL_H as i32 - 1) as u8,
        w: w.max(1) as u8,
        h: h.max(1) as u8,
    });
}

fn draw_door(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32) {
    let t = timber();
    c.rect(x, y, w, h, t.d);
    c.vline(x, y, h, t.m);
    c.hline(x, y + h - 1, w, t.m);
}

/// Pitched roof over a `w`-wide wall whose top row is `y - 1`. Returns the ridge row.
fn draw_pitch(c: &mut Canvas, x: i32, y: i32, w: i32, rows: i32, r: Ramp) -> i32 {
    c.hline(x - 1, y, w + 2, r.d);
    for i in 1..rows {
        let rw = (w - 2 * (i - 1)).max(1);
        let rx = x + (i - 1);
        if i == rows - 1 {
            c.hline(rx, y + i, rw, r.l);
        } else {
            c.hline(rx, y + i, rw, r.m);
            c.set(rx, y + i, r.l);
            c.set(rx + rw - 1, y + i, r.d);
        }
    }
    y + rows - 1
}

/// Flat roof cap. Returns the row above the parapet.
fn draw_parapet(c: &mut Canvas, x: i32, y: i32, w: i32, r: Ramp) -> i32 {
    c.hline(x - 1, y, w + 2, r.d);
    c.hline(x - 1, y + 1, w + 2, r.l);
    y + 2
}

fn draw_chimney(c: &mut Canvas, x: i32, y: i32, h: i32) {
    let p = plaster();
    c.rect(x, y, 2, h, p.d);
    c.vline(x, y, h, p.m);
    c.hline(x, y + h - 1, 2, rgba(OUTLINE));
}

fn draw_stack(c: &mut Canvas, x: i32, y: i32, h: i32) {
    let s = stone();
    c.rect(x, y, 2, h, s.m);
    c.vline(x + 1, y, h, s.d);
    c.hline(x, y + h - 1, 2, s.l);
}

// ─ Town family ─────────────────────────────────────────

/// Tier 1 — single storey, pitched roof, one chimney.
fn cottage(c: &mut Canvas, v: u8, roof: Ramp, squash: i32, wins: &mut Vec<WinRect>) {
    let v = v as i32;
    let w = 9 + (v % 2) * 2;
    let h = (6 + v / 2 - squash).max(4);
    let x = (W - w) / 2;
    let p = plaster();

    ground_shadow(c, x, w);
    draw_wall(c, x, 1, w, h, p);
    c.hline(x, 1, w, timber().d);

    let top = 1 + h;
    let wy = top - 3;
    draw_window(c, x + 1, wy, 2, 2, p.l, wins);
    draw_window(c, x + w - 3, wy, 2, 2, p.l, wins);

    let dx = x + w / 2 - 1 + if v >= 2 { 1 } else { 0 };
    draw_door(c, dx, 2, 2, 3);
    if v >= 2 {
        c.hline(dx - 1, 5, 4, timber().m);
    }

    let ridge = draw_pitch(c, x, top, w, 4, roof);
    let chim = if v < 2 { x + 1 } else { x + w - 3 };
    draw_chimney(c, chim, top, ridge - top + 3);
}

/// Tier 2 — two storeys, shared party walls, small yard.
fn townhouse(c: &mut Canvas, v: u8, roof: Ramp, squash: i32, wins: &mut Vec<WinRect>) {
    let v = v as i32;
    let w = 12 + (v % 2) * 2;
    let h = (13 + v / 2 - squash).max(9);
    let x = (W - w) / 2;
    let p = plaster();
    let t = timber();

    // Yard: a low garden wall reaching past the footprint, with tufts.
    c.hline(x - 1, 0, w + 2, t.m);
    let mut i = 0;
    while i < w + 2 {
        c.set(x - 1 + i, 0, t.d);
        i += 3;
    }
    c.set(x - 1, 1, rgba(GRASS_D));
    c.set(x + w, 1, rgba(GRASS_D));

    draw_wall(c, x, 1, w, h, p);
    // Shared walls: the party wall on each side is what makes a terrace read.
    c.vline(x, 1, h, t.d);
    c.vline(x + w - 1, 1, h, t.d);

    draw_door(c, x + w / 2 - 1, 2, 2, 4);

    let cols = if w >= 14 { 3 } else { 2 };
    let span = w - 5;
    let course = h - 4;
    c.hline(x, course, w, p.d);
    for row in 0..2 {
        let wy = if row == 0 { 6 } else { h - 3 };
        for ci in 0..cols {
            let wx = x + 2 + if cols > 1 { ci * span / (cols - 1) } else { 0 };
            draw_window(c, wx, wy, 2, 3, p.l, wins);
        }
    }

    let top = 1 + h;
    if v >= 2 {
        // Mansard: two shallow courses then a flat cap.
        c.hline(x - 1, top, w + 2, roof.d);
        c.hline(x, top + 1, w, roof.m);
        draw_parapet(c, x + 1, top + 2, w - 2, roof);
        draw_chimney(c, x + 1, top, 4);
    } else {
        let ridge = draw_pitch(c, x, top, w, 3, roof);
        draw_chimney(c, x + 1, top, ridge - top + 2);
    }
}

/// Tier 3 — ground-floor commerce, awning, sign.
fn shopfront(c: &mut Canvas, v: u8, roof: Ramp, squash: i32, wins: &mut Vec<WinRect>) {
    let v = v as i32;
    let w = 12 + (v % 2) * 2;
    let h = (17 + v / 2 - squash).max(12);
    let x = (W - w) / 2;
    let p = plaster();

    ground_shadow(c, x, w);
    draw_wall(c, x, 1, w, h, p);
    c.hline(x, 1, w, timber().d);

    // Shop glass runs almost the full frontage — one wide, warm pane at night.
    // It sits on a dark stall riser, not a bright plinth: the ground floor is
    // where the light comes from after dusk, so it stays quiet by day.
    let gx = x + 1;
    let gw = w - 5;
    draw_window(c, gx, 2, gw, 4, timber().d, wins);
    let mut m = gx + 3;
    while m < gx + gw {
        c.vline(m, 2, 4, p.m);
        m += 3;
    }
    draw_door(c, x + w - 4, 2, 3, 5);

    // Awning, projecting one texel past the wall on each side. Striped, but
    // kept off the light step: contrast is a budget and track owns it.
    for px in (x - 1)..(x + w + 1) {
        let stripe = if (px + v).rem_euclid(2) == 0 {
            roof.m
        } else {
            p.m
        };
        c.set(px, 7, stripe);
        c.set(px, 6, roof.d);
    }

    // Sign board with lettering dashes.
    let sx = x + 2;
    let sw = w - 4;
    c.rect(sx, 8, sw, 2, timber().d);
    let mut lx = sx + 1;
    while lx < sx + sw - 1 {
        c.set(lx, 9, timber().l);
        lx += 2;
    }

    // Two storeys of lodgings over the shop: a tall row, then attic panes.
    for ci in 0..3 {
        let wx = x + 1 + ci * ((w - 4) / 2);
        if 11 + 3 <= h {
            draw_window(c, wx, 11, 2, 3, p.l, wins);
        }
        if 15 + 2 <= h {
            draw_window(c, wx, 15, 2, 2, p.l, wins);
        }
    }

    let top = 1 + h;
    if v >= 2 {
        draw_parapet(c, x, top, w, roof);
        c.rect(x + 2, top + 2, 3, 2, roof.m);
    } else {
        draw_pitch(c, x, top, w, 3, roof);
    }
}

/// Tier 4 — three to four storeys, flat roof, courtyard entry.
fn block(c: &mut Canvas, v: u8, roof: Ramp, squash: i32, wins: &mut Vec<WinRect>) {
    let v = v as i32;
    let w = 13 + (v % 2) * 2;
    let h = (21 + (v / 2) * 3 - squash).max(15);
    let x = (W - w) / 2;
    let p = plaster();

    ground_shadow(c, x, w);
    draw_wall(c, x, 1, w, h, p);
    c.hline(x, 1, w, rgba(BALLAST_M));

    // Carriage arch through to the courtyard.
    let ex = x + w / 2 - 2;
    c.rect(ex, 1, 4, 6, rgba(OUTLINE));
    c.hline(ex, 6, 4, p.l);
    c.hline(ex, 1, 4, rgba(BALLAST_M));

    // Storey band, then a regular window grid above the arch.
    c.hline(x, 7, w, p.d);
    let cols = [x + 2, x + w / 2 - 1, x + w - 4];
    let mut wy = 9;
    while wy + 2 <= h {
        for cx in cols {
            draw_window(c, cx, wy, 2, 2, p.l, wins);
        }
        wy += 5;
    }

    let top = 1 + h;
    let above = draw_parapet(c, x, top, w, roof);
    // Roof furniture: stair head one side, vent the other.
    c.rect(x + 2, above, 3, 2, roof.m);
    c.hline(x + 2, above + 1, 3, roof.l);
    draw_stack(c, x + w - 4, above, 3);
}

// ─ Works family ────────────────────────────────────────

/// Works tier 1 — a timber shed with a corrugated roof.
fn shed(c: &mut Canvas, v: u8, roof: Ramp, squash: i32, wins: &mut Vec<WinRect>) {
    let v = v as i32;
    let w = 10 + (v % 2) * 2;
    let h = (5 + v / 2 - squash).max(4);
    let x = (W - w) / 2;
    let t = timber();

    ground_shadow(c, x, w);
    draw_wall(c, x, 1, w, h, t);
    let mut px = x + 2;
    while px < x + w - 1 {
        c.vline(px, 1, h, t.d);
        px += 3;
    }

    draw_door(c, x + w / 2 - 2, 1, 4, h - 1);
    // Every building carries at least one pane, so every building can light up.
    draw_window(c, x + 1, h - 2, 2, 2, t.l, wins);

    // Corrugation: alternating columns rather than a flat plane.
    let top = 1 + h;
    c.hline(x - 1, top, w + 2, roof.d);
    for px in (x - 1)..(x + w + 1) {
        c.set(px, top + 1, if px.rem_euclid(2) == 0 { roof.m } else { roof.l });
    }
}

/// Works tier 2 — sawtooth workshop with a stack.
fn workshop(c: &mut Canvas, v: u8, roof: Ramp, squash: i32, wins: &mut Vec<WinRect>) {
    let v = v as i32;
    let w = 12 + (v % 2) * 2;
    let h = (8 + v / 2 - squash).max(6);
    let x = (W - w) / 2;
    let p = plaster();

    ground_shadow(c, x, w);
    draw_wall(c, x, 1, w, h, p);
    c.hline(x, 1, w, timber().d);
    draw_door(c, x + 1, 2, 4, 5);
    for ci in 0..2 {
        draw_window(c, x + 6 + ci * 3, 3, 2, 3, p.l, wins);
    }

    // Two sawtooth bays — a glazed vertical face, then a slope falling away.
    // Nothing else in the game has this silhouette, which is the point.
    let top = 1 + h;
    let bay = w / 2;
    c.hline(x - 1, top - 1, w + 2, roof.d);
    for tooth in 0..2 {
        let tx = x + tooth * bay;
        c.vline(tx, top, 4, glass());
        c.vline(tx + 1, top, 4, glass());
        wins.push(WinRect {
            x: tx.clamp(0, W - 1) as u8,
            y: top as u8,
            w: 2,
            h: 4,
        });
        for i in 2..bay {
            let fall = ((i - 1) * 3 / (bay - 2).max(1)).min(3);
            let colh = 4 - fall;
            c.vline(tx + i, top, colh, roof.m);
            c.set(tx + i, top + colh - 1, roof.l);
        }
    }
    draw_stack(c, x + w - 3, top, 6);
}

/// Works tier 3 — a fenced yard of stacked crates around a small hut.
fn yard(c: &mut Canvas, v: u8, roof: Ramp, squash: i32, wins: &mut Vec<WinRect>) {
    let v = v as i32;
    let w = 14;
    let x = (W - w) / 2;
    let t = timber();
    let mirror = v % 2;
    let hut_h = (5 + v / 2 - squash).max(4);

    ground_shadow(c, x, w);

    // Perimeter fence.
    c.hline(x, 4, w, t.m);
    let mut px = x;
    while px < x + w {
        c.vline(px, 1, 4, t.d);
        px += 3;
    }

    // Hut at one end, mirrored by variant.
    let hx = if mirror == 0 { x } else { x + w - 6 };
    let p = plaster();
    draw_wall(c, hx, 1, 6, hut_h, p);
    draw_door(c, hx + 2, 2, 2, 3);
    draw_window(c, hx + (if mirror == 0 { 4 } else { 1 }), hut_h - 1, 2, 2, p.l, wins);
    draw_pitch(c, hx, 1 + hut_h, 6, 3, roof);

    // Crate stacks — height and side both move with the variant.
    let base = if mirror == 0 { x + 7 } else { x };
    for k in 0..2 {
        let cx = base + k * 4;
        let stack = 2 + (v / 2) + ((v + k) % 2);
        for s in 0..stack {
            let cy = 1 + s * 3;
            c.rect(cx, cy, 3, 3, t.m);
            c.hline(cx, cy + 2, 3, t.l);
            c.vline(cx + 2, cy, 3, t.d);
            c.hline(cx, cy, 3, t.d);
        }
    }

    // Gantry standing in the yard, legs on the ground, hook over the crates.
    c.vline(base - 1, 1, 16, t.d);
    c.vline(base + 4, 1, 16, t.d);
    c.hline(base - 1, 17, 6, t.d);
    c.hline(base - 1, 16, 6, t.m);
    c.vline(base + 1, 14, 3, t.m);
    c.hline(base, 13, 3, t.l);
}

/// Works tier 4 — warehouse with loading doors, clerestory and stack.
fn warehouse(c: &mut Canvas, v: u8, roof: Ramp, squash: i32, wins: &mut Vec<WinRect>) {
    let v = v as i32;
    let w = 14 + (v % 2);
    let h = (13 + (v / 2) * 2 - squash).max(10);
    let x = (W - w) / 2;
    let p = plaster();

    ground_shadow(c, x, w);
    draw_wall(c, x, 1, w, h, p);
    c.rect(x, 1, w, 2, rgba(BALLAST_M));

    draw_door(c, x + 1, 1, 4, 7);
    draw_door(c, x + w - 5, 1, 4, 7);
    c.hline(x + 1, 8, 4, timber().l);
    c.hline(x + w - 5, 8, 4, timber().l);

    // Clerestory band — a run of small panes just under the eaves.
    let cy = h - 3;
    let mut px = x + 2;
    while px + 1 < x + w - 2 {
        draw_window(c, px, cy, 1, 2, p.l, wins);
        px += 3;
    }

    let top = 1 + h;
    draw_pitch(c, x, top, w, 3, roof);
    c.rect(x + 3, top + 2, 2, 2, roof.d);
    c.rect(x + w - 5, top + 2, 2, 2, roof.d);
    draw_stack(c, x + w - 3, top, 6);
}

// ─ Decay passes ────────────────────────────────────────

/// Ramp steps used by the peeling / darkening passes.
fn shade_table() -> Vec<([u8; 4], [u8; 4])> {
    let (p, t, r1, r2) = (
        plaster(),
        timber(),
        roof_ramp(Roof::Tile),
        roof_ramp(Roof::Slate),
    );
    vec![
        (p.l, p.m),
        (p.m, p.d),
        (t.l, t.m),
        (t.m, t.d),
        (r1.l, r1.m),
        (r1.m, r1.d),
        (r2.l, r2.m),
        (r2.m, r2.d),
        (rgba(ROCK_L), rgba(ROCK_M)),
        (rgba(ROCK_M), rgba(ROCK_D)),
    ]
}

fn darker(table: &[([u8; 4], [u8; 4])], px: [u8; 4]) -> Option<[u8; 4]> {
    table.iter().find(|(a, _)| *a == px).map(|(_, b)| *b)
}

/// Stage 1 — windows go dark. The glint leaves the panes and the sills dull.
fn apply_dimmed(c: &mut Canvas, wins: &[WinRect]) {
    let table = shade_table();
    for wr in wins {
        let (x, y, w, h) = (wr.x as i32, wr.y as i32, wr.w as i32, wr.h as i32);
        c.rect(x, y, w, h, glass());
        for i in 0..w {
            let below = c.get(x + i, y - 1);
            if let Some(d) = darker(&table, below) {
                c.set(x + i, y - 1, d);
            }
        }
    }
}

/// Stage 2 — boarded windows, peeling paint, an overgrown yard.
fn apply_boarded(c: &mut Canvas, wins: &[WinRect], key: u32) {
    let table = shade_table();
    let t = timber();

    for wr in wins {
        let (x, y, w, h) = (wr.x as i32, wr.y as i32, wr.w as i32, wr.h as i32);
        c.rect(x, y, w, h, t.m);
        c.hline(x, y + h / 2, w, t.d);
        c.set(x, y + h - 1, t.l);
    }

    // Peeling paint: patchy, world-anchored on the frame key.
    for y in 1..CELL_H as i32 {
        for x in 0..W {
            let px = c.get(x, y);
            if px[3] == 0 {
                continue;
            }
            let roll = world_hash(x + key as i32 * 7, y, 0x9101) % 100;
            if roll < 45 {
                if let Some(d) = darker(&table, px) {
                    c.set(x, y, d);
                }
            }
        }
    }

    if let Some((x0, _, x1, y1)) = c.bounds() {
        // Overgrown yard climbing the base course.
        for y in 1..=3 {
            for x in x0..=x1 {
                if c.get(x, y)[3] == 0 {
                    continue;
                }
                let roll = world_hash(x, y + key as i32 * 13, 0x5AA5) % 100;
                if roll < (44 - y * 10) as u32 {
                    c.set(x, y, rgba(GRASS_D));
                }
            }
        }
        c.set(x0 - 1, 1, rgba(GRASS_D));
        c.set(x1 + 1, 1, rgba(GRASS_D));
        // A few slipped roof tiles.
        for x in x0..=x1 {
            let roll = world_hash(x, y1, 0x3C3C) % 100;
            if roll < 14 {
                c.tint(x, y1, t.d);
            }
        }
    }
}

/// Stage 3 — derelict: holed roof, fallen boards, overgrowth up the walls.
fn apply_derelict(c: &mut Canvas, wins: &[WinRect], key: u32) {
    apply_boarded(c, wins, key);
    let p = plaster();
    let t = timber();

    for y in 1..CELL_H as i32 {
        for x in 0..W {
            let px = c.get(x, y);
            if px[3] == 0 {
                continue;
            }
            if px == p.m || px == p.l {
                c.set(x, y, p.d);
            } else if px == t.m || px == t.l {
                c.set(x, y, t.d);
            }
        }
    }

    if let Some((x0, y0, x1, y1)) = c.bounds() {
        // Punch holes through the top two fifths — the roof.
        let roof_from = y0 + (y1 - y0) * 3 / 5;
        for y in roof_from..=y1 {
            for x in x0..=x1 {
                if c.get(x, y)[3] == 0 {
                    continue;
                }
                if world_hash(x + key as i32 * 5, y, 0x7B7B) % 100 < 22 {
                    c.set(x, y, [0; 4]);
                    c.tint(x, y - 1, t.d);
                }
            }
        }
        // Overgrowth reaches higher.
        for y in 1..=6 {
            for x in x0..=x1 {
                if c.get(x, y)[3] == 0 {
                    continue;
                }
                if world_hash(x, y + key as i32 * 3, 0x2E2E) % 100 < (34 - y * 4) as u32 {
                    c.set(x, y, rgba(GRASS_D));
                }
            }
        }
    }

    // Boards fall away, leaving open dark holes.
    for (i, wr) in wins.iter().enumerate() {
        let (x, y, w, h) = (wr.x as i32, wr.y as i32, wr.w as i32, wr.h as i32);
        for j in 0..h {
            for k in 0..w {
                if world_hash(k + i as i32 * 11, j + key as i32, 0x4D4D) % 100 < 38 {
                    c.tint(x + k, y + j, glass());
                }
            }
        }
    }
}

// ─ Effect frames ───────────────────────────────────────

/// A surveyor's stake — tiny, easy to miss, the first hint of a build.
fn stake(c: &mut Canvas, v: usize) {
    let t = timber();
    c.hline(6, 0, 4, rgba_a(OUTLINE, SHADOW_ALPHA));
    c.vline(8, 1, 5, t.m);
    c.set(8, 5, t.l);
    if v == 0 {
        c.hline(9, 5, 2, t.l);
    } else {
        c.hline(6, 5, 2, t.l);
    }
    // Chalked plot corners.
    for (x, y) in [(4, 1), (11, 1), (4, 2), (11, 2)] {
        c.set(x, y, rgba(PLASTER_L));
    }
    c.set(5, 1, rgba(GRASS_D));
    c.set(10, 1, rgba(GRASS_D));
}

/// Scaffold for height class `class`, frame `f` (0 = going up, 1 = topped out).
fn scaffold(c: &mut Canvas, class: usize, f: usize) {
    let sh = [8, 13, 16, 22][class.min(3)];
    let w = [10, 13, 13, 15][class.min(3)];
    let x = (W - w) / 2;
    let t = timber();
    let p = plaster();

    ground_shadow(c, x, w);

    // Partial wall rising inside the scaffold.
    let built = if f == 0 { sh / 2 } else { sh * 3 / 4 };
    c.rect(x + 1, 1, w - 2, built.max(1), p.d);
    c.vline(x + 1, 1, built.max(1), p.m);

    c.vline(x, 1, sh, t.m);
    c.vline(x + w - 1, 1, sh, t.m);
    if class >= 2 {
        c.vline(x + w / 2, 1, sh, t.m);
    }

    let mut y = 4;
    while y < sh {
        c.hline(x, y, w, t.l);
        y += 4;
    }
    if f == 1 {
        c.hline(x, sh, w, t.l);
        // Dust puff at the base.
        for (dx, dy) in [(-2, 1), (-1, 2), (w, 1), (w + 1, 2), (w / 2, 1)] {
            c.set(x + dx, dy, rgba_a(PLASTER_L, 170));
        }
    }
}

/// Cleared lot: the foundation scar persists (brief 06 §3.2 step 4).
fn scar(c: &mut Canvas, v: usize) {
    let s = stone();
    let w = 9 + (v as i32) * 2;
    let x = (W - w) / 2;

    c.hline(x - 1, 0, w + 2, rgba_a(OUTLINE, 110));
    c.rect(x, 1, w, 4, rgba(GRASS_D));
    c.hline(x, 1, w, s.d);
    c.hline(x, 4, w, s.d);
    c.vline(x, 1, 4, s.d);
    c.vline(x + w - 1, 1, 4, s.d);
    let mut i = x;
    while i < x + w {
        c.set(i, 4, s.m);
        i += 3;
    }
    c.set(x + 2, 3, rgba(PLASTER_D));
    c.set(x + w - 3, 2, rgba(PLASTER_D));
    c.set(x + 3, 2, rgba(GRASS_M));
    c.set(x + w - 4, 3, rgba(GRASS_M));
}

/// Countryside props — the unserved map must look *deliberately* rural.
fn rural(c: &mut Canvas, which: usize) {
    let t = timber();
    match which {
        // Ploughed field.
        0 => {
            for row in 0..7 {
                let col = if row % 2 == 0 {
                    rgba(SAND_D)
                } else {
                    rgba(GRASS_D)
                };
                c.hline(1, 1 + row, 14, col);
            }
            c.vline(1, 1, 7, t.d);
            c.vline(14, 1, 7, t.d);
        }
        // Haystack.
        1 => {
            c.hline(4, 0, 8, rgba_a(OUTLINE, 130));
            c.hline(5, 1, 6, rgba(SAND_D));
            c.hline(5, 2, 6, rgba(SAND_M));
            c.hline(6, 3, 4, rgba(SAND_M));
            c.hline(6, 4, 4, rgba(SAND_L));
            c.hline(7, 5, 2, rgba(SAND_L));
            c.set(7, 6, t.d);
        }
        // Hedgerow.
        2 => {
            c.rect(1, 1, 14, 3, rgba(GRASS_D));
            for x in 1..15 {
                if world_hash(x, 0, 0x11AA).is_multiple_of(3) {
                    c.set(x, 3, rgba(GRASS_M));
                }
                if world_hash(x, 1, 0x11AA).is_multiple_of(5) {
                    c.set(x, 1, t.d);
                }
            }
        }
        // Lane with a farm gate.
        3 => {
            c.rect(3, 1, 10, 8, rgba(GRASS_D));
            c.rect(4, 1, 2, 8, rgba(SAND_M));
            c.rect(10, 1, 2, 8, rgba(SAND_M));
            c.hline(3, 5, 10, t.m);
            c.vline(3, 3, 4, t.d);
            c.vline(12, 3, 4, t.d);
        }
        // Dry-stone wall.
        4 => {
            let s = stone();
            for x in 2..14 {
                c.set(x, 1, if x % 2 == 0 { s.m } else { s.d });
                c.set(x, 2, if x % 2 == 0 { s.d } else { s.m });
            }
            c.hline(2, 3, 12, s.l);
        }
        // Lone tree — still, deliberately (brief 01 §6.3).
        _ => {
            c.hline(6, 0, 4, rgba_a(OUTLINE, 130));
            c.rect(7, 1, 2, 3, t.d);
            let rows = [(6, 5), (5, 7), (5, 7), (5, 7), (6, 5), (7, 3)];
            for (i, (rx, rw)) in rows.iter().enumerate() {
                c.hline(*rx, 4 + i as i32, *rw, rgba(GRASS_D));
            }
            for i in 0..6 {
                let x = 5 + (world_hash(i, 3, 0x77CC) % 7) as i32;
                let y = 4 + (world_hash(i, 5, 0x77CC) % 6) as i32;
                c.tint(x, y, rgba(GRASS_M));
            }
            c.set(6, 8, rgba(GRASS_L));
            c.set(7, 9, rgba(GRASS_L));
        }
    }
}

// ─ Frame dispatch ──────────────────────────────────────

fn kind_from_key(key: usize) -> BuildingKind {
    let roof = if key.is_multiple_of(ROOFS) {
        Roof::Tile
    } else {
        Roof::Slate
    };
    let rest = key / ROOFS;
    let variant = (rest % VARIANTS) as u8;
    let rest = rest / VARIANTS;
    let tier = (rest % TIERS) as u8;
    let family = if rest / TIERS == 0 {
        Family::Town
    } else {
        Family::Works
    };
    BuildingKind {
        family,
        tier,
        variant,
        roof,
    }
}

fn draw_kind(kind: BuildingKind, squash: i32) -> (Canvas, Vec<WinRect>) {
    let mut c = Canvas::new();
    let mut wins = Vec::new();
    let roof = roof_ramp(kind.roof);
    match (kind.family, kind.tier) {
        (Family::Town, 0) => cottage(&mut c, kind.variant, roof, squash, &mut wins),
        (Family::Town, 1) => townhouse(&mut c, kind.variant, roof, squash, &mut wins),
        (Family::Town, 2) => shopfront(&mut c, kind.variant, roof, squash, &mut wins),
        (Family::Town, _) => block(&mut c, kind.variant, roof, squash, &mut wins),
        (Family::Works, 0) => shed(&mut c, kind.variant, roof, squash, &mut wins),
        (Family::Works, 1) => workshop(&mut c, kind.variant, roof, squash, &mut wins),
        (Family::Works, 2) => yard(&mut c, kind.variant, roof, squash, &mut wins),
        (Family::Works, _) => warehouse(&mut c, kind.variant, roof, squash, &mut wins),
    }
    (c, wins)
}

fn draw_lit(key: usize) -> Canvas {
    let kind = kind_from_key(key);
    let (_, wins) = draw_kind(kind, 0);
    let mut c = Canvas::new();
    let lit = rgba(WIN_LIT);
    let spill = rgba_a(WIN_LIT, 70);
    for (i, wr) in wins.iter().enumerate() {
        // Not every room is occupied — a fully lit facade reads as a decal.
        // The first window always burns, so an occupied building is never dark:
        // a lit district at nightfall is the point (brief 06 §8.6).
        if i > 0 && world_hash(key as i32, i as i32, 0x10CE) % 100 >= 74 {
            continue;
        }
        let (x, y, w, h) = (wr.x as i32, wr.y as i32, wr.w as i32, wr.h as i32);
        c.rect(x, y, w, h, lit);
        c.hline(x, y - 1, w, spill);
    }
    c
}

fn draw_frame(frame: usize) -> (Canvas, Vec<WinRect>) {
    if frame < BUILDING_FRAMES {
        let key = frame / DECAYS;
        let decay = match frame % DECAYS {
            0 => Decay::Healthy,
            1 => Decay::Dimmed,
            2 => Decay::Boarded,
            _ => Decay::Derelict,
        };
        let (mut c, wins) = draw_kind(kind_from_key(key), 0);
        match decay {
            Decay::Healthy => {}
            Decay::Dimmed => apply_dimmed(&mut c, &wins),
            Decay::Boarded => apply_boarded(&mut c, &wins, key as u32),
            Decay::Derelict => apply_derelict(&mut c, &wins, key as u32),
        }
        // Boarded / derelict openings are not window openings any more.
        let reported = match decay {
            Decay::Healthy | Decay::Dimmed => wins,
            _ => Vec::new(),
        };
        return (c, reported);
    }
    if frame < LIT_BASE {
        let (c, wins) = draw_kind(kind_from_key(frame - SETTLE_BASE), 1);
        return (c, wins);
    }
    if frame < FX_BASE {
        return (draw_lit(frame - LIT_BASE), Vec::new());
    }

    let mut c = Canvas::new();
    let fx = frame - FX_BASE;
    match fx {
        0 | 1 => stake(&mut c, fx),
        2..=9 => scaffold(&mut c, (fx - 2) / 2, (fx - 2) % 2),
        10 | 11 => scar(&mut c, fx - 10),
        _ => rural(&mut c, fx - 12),
    }
    (c, Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_ranges_do_not_overlap() {
        assert_eq!(BUILDING_FRAMES, 256);
        assert_eq!(SETTLE_BASE, 256);
        assert_eq!(LIT_BASE, 320);
        assert_eq!(FX_BASE, 384);
        assert_eq!(FRAME_COUNT, 402);
        assert_eq!(FRAME_RURAL + RURAL_PROPS, FRAME_COUNT);
    }

    #[test]
    fn key_round_trips_through_kind() {
        for key in 0..KEY_COUNT {
            assert_eq!(kind_from_key(key).key(), key);
        }
    }

    #[test]
    fn every_frame_draws_something() {
        for frame in 0..FRAME_COUNT {
            let (c, _) = draw_frame(frame);
            assert!(
                c.bounds().is_some(),
                "frame {frame} baked to an empty cell — flat nothing is not a placeholder"
            );
        }
    }

    #[test]
    fn healthy_town_buildings_have_windows() {
        for tier in 0..4u8 {
            for variant in 0..4u8 {
                let kind = BuildingKind {
                    family: Family::Town,
                    tier,
                    variant,
                    roof: Roof::Tile,
                };
                let (_, wins) = draw_kind(kind, 0);
                assert!(
                    !wins.is_empty(),
                    "town tier {tier} variant {variant} has no windows to light at night"
                );
            }
        }
    }

    #[test]
    fn tiers_have_distinct_silhouettes() {
        for family in [Family::Town, Family::Works] {
            for variant in 0..4u8 {
                let mut heights = Vec::new();
                for tier in 0..4u8 {
                    let kind = BuildingKind {
                        family,
                        tier,
                        variant,
                        roof: Roof::Slate,
                    };
                    let (c, _) = draw_kind(kind, 0);
                    let (_, _, _, y1) = c.bounds().expect("tier draws pixels");
                    heights.push(y1);
                }
                for i in 1..heights.len() {
                    assert!(
                        heights[i] > heights[i - 1],
                        "{family:?} variant {variant} tier {i} must stand taller than tier {}: {heights:?}",
                        i - 1
                    );
                }
            }
        }
    }

    #[test]
    fn variants_within_a_tier_actually_differ() {
        for family in [Family::Town, Family::Works] {
            for tier in 0..4u8 {
                let mut seen: Vec<Vec<[u8; 4]>> = Vec::new();
                for variant in 0..4u8 {
                    let kind = BuildingKind {
                        family,
                        tier,
                        variant,
                        roof: Roof::Tile,
                    };
                    let px = draw_kind(kind, 0).0.px;
                    assert!(
                        !seen.contains(&px),
                        "{family:?} tier {tier} variant {variant} duplicates another variant"
                    );
                    seen.push(px);
                }
            }
        }
    }

    #[test]
    fn roof_material_changes_the_roof() {
        for family in [Family::Town, Family::Works] {
            for tier in 0..4u8 {
                let tiled = draw_kind(
                    BuildingKind {
                        family,
                        tier,
                        variant: 0,
                        roof: Roof::Tile,
                    },
                    0,
                )
                .0;
                let slated = draw_kind(
                    BuildingKind {
                        family,
                        tier,
                        variant: 0,
                        roof: Roof::Slate,
                    },
                    0,
                )
                .0;
                assert_ne!(
                    tiled.px, slated.px,
                    "{family:?} tier {tier} draws the same roof in both materials"
                );
            }
        }
    }

    #[test]
    fn settle_frame_is_shorter_than_the_finished_building() {
        for tier in 0..4u8 {
            let kind = BuildingKind {
                family: Family::Town,
                tier,
                variant: 1,
                roof: Roof::Tile,
            };
            let (full, _) = draw_kind(kind, 0);
            let (squashed, _) = draw_kind(kind, 1);
            let a = full.bounds().unwrap().3;
            let b = squashed.bounds().unwrap().3;
            assert!(b < a, "tier {tier} settle frame must sit lower ({b} < {a})");
        }
    }

    #[test]
    fn decline_stages_all_differ() {
        let kind = BuildingKind {
            family: Family::Town,
            tier: 1,
            variant: 2,
            roof: Roof::Tile,
        };
        let frames: Vec<Vec<[u8; 4]>> = [Decay::Healthy, Decay::Dimmed, Decay::Boarded, Decay::Derelict]
            .iter()
            .map(|d| draw_frame(kind.frame(*d)).0.px)
            .collect();
        for i in 0..frames.len() {
            for j in (i + 1)..frames.len() {
                assert_ne!(frames[i], frames[j], "decline stages {i} and {j} look identical");
            }
        }
    }

    #[test]
    fn lit_mask_lands_inside_the_dark_windows() {
        for key in 0..KEY_COUNT {
            let kind = kind_from_key(key);
            let lit = draw_frame(kind.lit_frame()).0;
            let healthy = draw_frame(kind.frame(Decay::Healthy)).0;
            for y in 0..CELL_H as i32 {
                for x in 0..W {
                    if lit.get(x, y)[3] == 255 {
                        assert_ne!(
                            healthy.get(x, y)[3],
                            0,
                            "lit texel ({x},{y}) of key {key} hangs off the building"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn boarded_frames_report_no_open_windows() {
        let kind = BuildingKind {
            family: Family::Town,
            tier: 2,
            variant: 0,
            roof: Roof::Slate,
        };
        assert!(draw_frame(kind.frame(Decay::Boarded)).1.is_empty());
        assert!(draw_frame(kind.frame(Decay::Derelict)).1.is_empty());
        assert!(!draw_frame(kind.frame(Decay::Healthy)).1.is_empty());
    }

    #[test]
    fn art_only_uses_palette_colours() {
        let allowed: Vec<[u8; 3]> = [
            OUTLINE, PLASTER_D, PLASTER_M, PLASTER_L, WOOD_D, WOOD_M, WOOD_L, ROOF_TILE_D,
            ROOF_TILE_M, ROOF_TILE_L, ROOF_SLATE_D, ROOF_SLATE_M, ROOF_SLATE_L, WIN_DARK, WIN_LIT,
            GRASS_D, GRASS_M, GRASS_L, SAND_D, SAND_M, SAND_L, ROCK_D, ROCK_M, ROCK_L, BALLAST_M,
        ]
        .iter()
        .map(|c| {
            let p = rgba(*c);
            [p[0], p[1], p[2]]
        })
        .collect();

        for frame in 0..FRAME_COUNT {
            let (c, _) = draw_frame(frame);
            for px in &c.px {
                if px[3] == 0 {
                    continue;
                }
                assert!(
                    allowed.contains(&[px[0], px[1], px[2]]),
                    "frame {frame} used off-palette colour {px:?}"
                );
            }
        }
    }

    #[test]
    fn no_diagnostic_accents_in_world_art() {
        // hi / warn / ok are UI only (brief 01 §3.1).
        let banned = [
            rgba(crate::palette::HI),
            rgba(crate::palette::WARN),
            rgba(crate::palette::OK),
        ];
        for frame in 0..FRAME_COUNT {
            let (c, _) = draw_frame(frame);
            for px in &c.px {
                assert!(
                    !banned.iter().any(|b| b[0] == px[0] && b[1] == px[1] && b[2] == px[2]),
                    "frame {frame} used a diagnostic accent"
                );
            }
        }
    }

    #[test]
    fn hash_is_stable_and_spreads() {
        assert_eq!(world_hash(3, 7, 1), world_hash(3, 7, 1));
        assert_ne!(world_hash(3, 7, 1), world_hash(7, 3, 1));
        assert_ne!(world_hash(3, 7, 1), world_hash(3, 7, 2));
        let mut buckets = [0usize; 4];
        for x in 0..32 {
            for y in 0..32 {
                buckets[(world_hash(x, y, 9) % 4) as usize] += 1;
            }
        }
        for b in buckets {
            assert!(b > 150, "hash is lumpy: {buckets:?}");
        }
    }
}
