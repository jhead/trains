//! Procedural track art for the sixteen-direction graph, baked on edit.
//!
//! # Direction is a different sprite, never a rotation
//!
//! Brief 01 §2.2 is a contract: *"Direction is expressed by choosing a
//! different sprite, never by transforming one. A rotated sprite resamples, and
//! resampled pixel art is mush."* Nothing here ever writes a rotation — every
//! track transform is identity, and the whole vocabulary is selection.
//!
//! What is selected is a **composite keyed on the piece's 16-bit link mask**: a
//! hub, plus one leg per linked direction, painted into a single cell. That is
//! the same shape as the terrain autotiler and it is why sixteen directions cost
//! sixteen legs rather than 65,536 tiles — the mask picks which legs to stamp,
//! and only masks that actually occur are ever baked (§2.5, bake on edit).
//!
//! A half-step (knight's-move) leg reaches √5/2 ≈ 1.12 tiles from the centre, so
//! the cell is three tiles across. The two tiles a half-step crosses carry no
//! track piece of their own — that is what lets the shallow link exist at all —
//! so the two endpoint pieces each draw their half and meet in the middle.
//!
//! # Cross-section
//!
//! The numbers in brief 01 §5.3 are line weights, not suggestions, and they are
//! the constants below at 32 texels to the tile: ballast bed 8 half-width, rail
//! gauge 8 centre to centre, rail body 1 half-width, one railhead texel, sleeper
//! spacing 4, sleeper length 14. Every colour comes from [`crate::palette`].
//!
//! # Railhead polish
//!
//! §5.3 again: track a train has just crossed brightens toward `railS` and
//! decays over about four seconds, so a busy main line visibly gleams and a
//! branch nobody runs goes dull — the network's usage written into the world art
//! with no overlay and no numbers, and the cheapest half of "congestion must be
//! visible" (`07-trains-and-lines.md` §4.1).
//!
//! It is a second baked layer holding only the railhead texels in `railS`,
//! drawn over the piece and faded in by alpha. Tinting the base sprite cannot do
//! this — a sprite colour multiplies, so it can only ever darken.
//!
//! # Ground that moves
//!
//! In isometric a leg between tiles of different heights is a **ramp**, and the
//! ramp is the whole of `15-isometric-track.md`. [`super::iso_incline`] owns the
//! geometry — where the joint is, how far a leg has climbed at any point along
//! it, and the sleeper ladder that carries through the joint instead of
//! stuttering at it — and every painter below walks a [`LegWalk`], so the bed,
//! the sleepers, the rails and the gleam all climb together and stay in
//! register. There is no separate incline path here: the level case is the
//! ramped case with a grade of zero.
//!
//! Top-down is untouched by all of it, and `the_top_down_view_is_byte_identical`
//! is the pin that says so.

use std::collections::{HashMap, HashSet};

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
#[cfg_attr(not(test), allow(unused_imports))]
use rail_map::tile_to_world;
use rail_sim::ids::TileCoord;
use rail_sim::track::DIR_COUNT;
use rail_sim::{TileOccupancy, TrackEdit, TrackId, TrackNetwork};

#[cfg_attr(not(test), allow(unused_imports))]
use super::iso_incline::{self, LegGrades, LegWalk};
use crate::hash::world_hash;
use crate::map::GroundAnchor;
use crate::palette::{
    BALLAST_D, BALLAST_L, BALLAST_M, OUTLINE, RAIL_D, RAIL_L, RAIL_M, RAIL_S, TIE_D, TIE_L, TIE_M,
    WOOD_D, WOOD_M,
};

/// Seconds a crossing takes to fade back to unpolished rail.
const POLISH_DECAY_SECS: f32 = 4.0;
/// How far toward `railS` a freshly crossed tile goes.
const POLISH_MAX: f32 = 0.9;

/// Track sits above terrain and below buildings (brief 01 §6.1).
const TRACK_Z: f32 = 1.0;
/// The gleam layer, immediately over its own piece.
const POLISH_Z: f32 = 0.01;

// ── Cross-section, brief 01 §5.3, at 32 texels to the tile ─────────────────

/// Texels per tile edge — the pixel contract's source tile size (§2.1).
const TEXELS_PER_TILE: f32 = 32.0;
/// Cell edge in texels. Three tiles: a half-step leg reaches √5/2 tiles from
/// the centre, and the ballast is 8 wide beyond that.
///
/// Sized for the wider of the two projections: isometric stretches the ground
/// plane to twice its width, so the widest leg (`(1, -2)`, projecting to 48
/// texels of run) plus its ballast wants ±60. 128 gives that with room to
/// spare, and top-down simply leaves the outside of the cell transparent. The
/// cell is cached per (mask, projection) and only combinations that occur are
/// ever baked, so the extra texels cost nothing that matters.
const CELL: u32 = 128;
/// Cell centre, in texels from the top-left.
const CENTER: i32 = (CELL / 2) as i32;

/// Ballast bed, half-width.
const BALLAST_HALF: f32 = 8.0;
/// Rail gauge, centre to centre — so each rail sits 4 texels off the centreline.
const RAIL_GAUGE_HALF: f32 = 4.0;
/// Rail body, half-width. One texel either side of the head.
const RAIL_BODY_HALF: i32 = 1;
/// Sleeper length 14 → 7 either side of the centreline.
const SLEEPER_HALF: f32 = 7.0;
/// Bridge deck planking pitch, in texels along the run.
const PLANK_PITCH: f32 = 3.0;
/// One in five sleepers takes the light step, world-hashed.
const TIE_LIGHT_EVERY: u32 = 5;
/// Ballast speckle coverage (§5.3: `ballastM` at ~18%).
const SPECKLE_PERMILLE: u32 = 180;
/// World-hashed flat variants per material (§6.2.3).
const VARIANTS: u32 = 3;

/// Sub-texel step when walking a leg, so a diagonal run leaves no holes.
const WALK_STEP: f32 = 0.5;

/// Marker on a track piece's baked sprite.
///
/// Everything the art was baked from, so the reconcile in [`apply_track_sprites`]
/// can tell at a glance whether it is still the right drawing.
#[derive(Component, Debug, Clone, Copy)]
pub struct TrackSprite {
    pub id: TrackId,
    /// The link mask this art was baked for; a change means a re-bake.
    pub links: u16,
    pub bridge: bool,
    /// The projection it was baked for. A flip re-bakes rather than re-uses,
    /// because the bake walks projected direction axes.
    pub projection: rail_map::Projection,
    /// The per-leg height deltas this art was baked for. Same contract as
    /// `links`: a change means the ramps moved and the cell is stale.
    grades: LegGrades,
}

/// The railhead gleam layer for one piece, a child of its [`TrackSprite`].
#[derive(Component, Debug, Clone, Copy)]
pub struct TrackPolish {
    pub id: TrackId,
}

/// What a baked cell is keyed on. Everything else about a piece is position.
///
/// The projection is part of the key because the bake walks *projected*
/// direction axes (see [`LegWalk`]) — the same link mask is a different drawing
/// from above than it is in isometric. Keying on it rather than clearing the
/// bank means the second flip back re-uses cells instead of re-painting them:
/// a view the player is A/B-ing costs its bake once.
///
/// The **grades** are part of it for the same reason one step further out: an
/// isometric leg that climbs is a ramp rather than a level run, so the same mask
/// on a hillside is a different drawing again (brief 15). Level track keys on
/// [`LegGrades::LEVEL`] and therefore bakes exactly the vocabulary it always
/// baked — the bank only widens where the ground actually moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ArtKey {
    links: u16,
    bridge: bool,
    variant: u32,
    projection: rail_map::Projection,
    grades: LegGrades,
}

/// Baked cells, kept for the life of the session.
///
/// Content-addressed, so a new map or a loaded save reuses whatever it already
/// has and bakes only what is new.
///
/// A resource rather than a `Local` because the build ghost draws from it too:
/// brief 04 §2.2 wants the ghost to be *the actual track art*, and the only way
/// two systems cannot draw different pictures of the same piece is for them to
/// ask the same bank for the same key.
#[derive(Resource, Default)]
pub struct TrackArt {
    cache: HashMap<ArtKey, (Handle<Image>, Handle<Image>)>,
}

impl TrackArt {
    fn get(&mut self, images: &mut Assets<Image>, key: ArtKey) -> (Handle<Image>, Handle<Image>) {
        if let Some(handles) = self.cache.get(&key) {
            return handles.clone();
        }
        let base = images.add(cell_image(paint_cell(key, Pass::Base)));
        let polish = images.add(cell_image(paint_cell(key, Pass::Polish)));
        self.cache.insert(key, (base.clone(), polish.clone()));
        (base, polish)
    }

    /// Cells baked so far — the vocabulary actually in use.
    #[cfg(test)]
    pub fn baked(&self) -> usize {
        self.cache.len()
    }
}

/// Which layer a bake pass is painting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// Ballast, sleepers and rails.
    Base,
    /// Railhead texels only, in `railS`, for the gleam.
    Polish,
}

// ── Baking ─────────────────────────────────────────────────────────────────

const VARIANT_SALT: u32 = 0x51A4_C7D3;
const SPECKLE_SALT: u32 = 0x2C9E_11B7;
const TIE_SALT: u32 = 0x6D3B_84F1;

/// Which flat variant a tile takes. World-anchored, so the noise belongs to the
/// ground rather than to the screen.
fn variant_for(tile: TileCoord) -> u32 {
    world_hash(tile.x, tile.y, VARIANT_SALT) % VARIANTS
}

fn rgba(color: Color) -> [u8; 4] {
    let s = color.to_srgba();
    [
        (s.red * 255.0).round() as u8,
        (s.green * 255.0).round() as u8,
        (s.blue * 255.0).round() as u8,
        255,
    ]
}

/// A cell under construction: RGBA texels, origin top-left.
struct Canvas {
    px: Vec<u8>,
}

impl Canvas {
    fn new() -> Self {
        Self {
            px: vec![0u8; (CELL * CELL) as usize * 4],
        }
    }

    /// Paint one texel in cell-centred coordinates, y up (world orientation).
    fn put(&mut self, x: i32, y: i32, color: [u8; 4]) {
        let ix = CENTER + x;
        let iy = CENTER - y;
        if ix < 0 || iy < 0 || ix >= CELL as i32 || iy >= CELL as i32 {
            return;
        }
        let o = ((iy as u32 * CELL + ix as u32) * 4) as usize;
        self.px[o..o + 4].copy_from_slice(&color);
    }

    #[cfg(test)]
    fn at(&self, x: i32, y: i32) -> [u8; 4] {
        let ix = CENTER + x;
        let iy = CENTER - y;
        if ix < 0 || iy < 0 || ix >= CELL as i32 || iy >= CELL as i32 {
            return [0; 4];
        }
        let o = ((iy as u32 * CELL + ix as u32) * 4) as usize;
        [
            self.px[o],
            self.px[o + 1],
            self.px[o + 2],
            self.px[o + 3],
        ]
    }
}

fn cell_image(canvas: Canvas) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: CELL,
            height: CELL,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        canvas.px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    // One texel is one screen pixel times a whole number — never filtered,
    // never mipmapped (brief 01 §2.1).
    image.sampler = ImageSampler::nearest();
    image
}

/// Unit vector along a direction, and the perpendicular, in texel space.
///
/// From above these are the direction itself and its perpendicular, and the
/// §5.3 cross-section is measured straight down a screen column.
///
/// In isometric both are *projected*, so every painter below draws on the
/// ground plane instead of on the screen plane. That is the whole of the track
/// reprojection — the bed, the sleepers, the rail bodies and the railheads all
/// walk `along · t + across · s` in ground units and land on the diamond grid
/// without any of them knowing which projection they are in.
///
/// The projected vectors are deliberately **not** re-normalised. A leg running
/// south-east projects to 1.41× its ground length and a leg running north-east
/// to 0.71×, and that foreshortening is exactly what makes a rail read as lying
/// on the ground rather than floating over it. `across` is the projection of
/// the ground-plane perpendicular, not the screen-space perpendicular of the
/// projected run, so the sleepers lie flat too.
///
/// [`LegWalk`] owns the maths and adds the third term the isometric view needs
/// — the ramp. This is the level case of it, kept for the tests that measure a
/// cross-section straight down a leg.
#[cfg(test)]
fn axes(dir: usize, projection: rail_map::Projection) -> (Vec2, Vec2) {
    let walk = LegWalk::new(dir, 0, projection);
    (walk.along, walk.across)
}

/// Paint one cell for a link mask.
fn paint_cell(key: ArtKey, pass: Pass) -> Canvas {
    let mut canvas = Canvas::new();
    let dirs: Vec<usize> = (0..DIR_COUNT)
        .filter(|&i| key.links & (1 << i) != 0)
        .collect();

    // An isolated piece still has to read as track: give it a short stub along
    // the default axis so a single tile is not an anonymous blob.
    let legs: Vec<LegWalk> = if dirs.is_empty() {
        [2usize, 6]
            .iter()
            .map(|&d| LegWalk::new(d, 0, key.projection).stub(TEXELS_PER_TILE * 0.5))
            .collect()
    } else {
        dirs.iter()
            .map(|&d| LegWalk::new(d, key.grades.at(d), key.projection))
            .collect()
    };

    // Skirt, bed, then sleepers, then rail bodies, then railheads — each stage
    // across *every* leg before the next begins, exactly as a hand would draw
    // it. That ordering is what makes a junction read as one piece of track: no
    // leg's ballast can bury another leg's rail, and no leg's web can bury
    // another leg's head. It also keeps the two layers in register, so a polish
    // texel is always over a railhead in the base.
    //
    // The skirt goes first for the same reason: a climbing leg fills downward,
    // and a leg that descends across the screen must be able to draw its own bed
    // over that fill.
    if pass == Pass::Base {
        for leg in &legs {
            paint_skirt(&mut canvas, key, leg);
        }
        for leg in &legs {
            paint_bed(&mut canvas, key, leg);
        }
        for leg in &legs {
            paint_sleepers(&mut canvas, key, leg);
        }
        for leg in &legs {
            paint_rail_bodies(&mut canvas, key, leg);
        }
    }
    for leg in &legs {
        paint_railheads(&mut canvas, key, leg, pass);
    }
    canvas
}

/// The embankment under a climbing leg (brief 15 §3.3).
///
/// A leg that climbs runs above its own tile's surface — 2 px at the boundary
/// for the single step, 8 px at the steepest legal climb — so its bed would
/// float with ground showing under it. Every bed texel therefore fills straight
/// down to where the same texel would have sat unlifted: `ballastD` for the
/// bank, one `outline` texel on its foot, which is the game's one shadow key
/// used exactly as the terrain cliff faces use it.
///
/// A **descending** leg gets nothing, and that asymmetry is the point. Track
/// running below its own tile's surface is a *cutting*: it simply draws over the
/// ground it is notched into, because track sorts above terrain within a row.
/// Drawing a skirt there would be drawing earth in mid-air.
fn paint_skirt(canvas: &mut Canvas, key: ArtKey, leg: &LegWalk) {
    // A bridge stands on piers, not on an earth bank; ballast in a river would
    // be worse than the gap it filled.
    if key.bridge || leg.is_level() || leg.grade < 0 {
        return;
    }
    for t in leg.run_samples(WALK_STEP) {
        let lift = leg.lift(t);
        if lift < 1.0 {
            continue;
        }
        let mut s = -BALLAST_HALF;
        while s <= BALLAST_HALF {
            let ground = leg.flat_at(t, s);
            let (x, floor) = (ground.x.round() as i32, ground.y.round() as i32);
            let top = (ground.y + lift).round() as i32;
            for y in floor..top {
                canvas.put(x, y, rgba(if y == floor { OUTLINE } else { BALLAST_D }));
            }
            s += WALK_STEP;
        }
    }
}

/// Ballast bed, or bridge deck planking where the piece spans water.
fn paint_bed(canvas: &mut Canvas, key: ArtKey, leg: &LegWalk) {
    for t in leg.run_samples(WALK_STEP) {
        let mut s = -BALLAST_HALF;
        while s <= BALLAST_HALF {
            let p = leg.at(t, s);
            let (x, y) = (p.x.round() as i32, p.y.round() as i32);
            let color = if key.bridge {
                // Planking runs across the deck (§5.3).
                if (t / PLANK_PITCH).floor() as i32 % 2 == 0 {
                    WOOD_M
                } else {
                    WOOD_D
                }
            } else if s.abs() >= BALLAST_HALF - 1.0 {
                // The sun edge catches the light (§5.3).
                BALLAST_L
            } else if world_hash(x, y, SPECKLE_SALT.wrapping_add(key.variant)) % 1000
                < SPECKLE_PERMILLE
            {
                BALLAST_M
            } else {
                BALLAST_D
            };
            canvas.put(x, y, rgba(color));
            s += WALK_STEP;
        }
    }
}

/// Sleepers across the run, one in five taking the light step.
///
/// The ladder is pitched to the **link**, not to the tile, so both halves of a
/// link lay the same one from opposite ends and the rhythm carries through the
/// boundary instead of stuttering there — see [`iso_incline::sleeper_pitch`].
fn paint_sleepers(canvas: &mut Canvas, key: ArtKey, leg: &LegWalk) {
    if key.bridge {
        // The deck *is* the sleepers on a bridge.
        return;
    }
    for (index, t) in leg.sleepers().enumerate() {
        let anchor = leg.at(t, 0.0);
        let light = world_hash(
            anchor.x.round() as i32,
            anchor.y.round() as i32,
            TIE_SALT.wrapping_add(key.variant),
        ) % TIE_LIGHT_EVERY
            == 0;
        let color = rgba(if light {
            TIE_L
        } else if index % 2 == 0 {
            TIE_M
        } else {
            TIE_D
        });
        let mut s = -SLEEPER_HALF;
        while s <= SLEEPER_HALF {
            let p = leg.at(t, s);
            canvas.put(p.x.round() as i32, p.y.round() as i32, color);
            s += WALK_STEP;
        }
    }
}

/// The rail's shadow flank, under the head that [`paint_railheads`] lays on top.
///
/// From above this is §5.3 as written: `railD` one texel one side of the head,
/// `railM` the other, measured across the ground.
///
/// In isometric it is one texel of `railD`, offset one step across the run **in
/// screen space**. One ground unit across projects to 1.118 screen texels on a
/// 2:1 staircase, so a three-texel cross-section measured on the ground cannot
/// survive — the head painted half a step further along rounds onto the very
/// texel the flank wanted, and the flanks come back as speckle rather than as
/// line. Two texels chosen in screen space are two lines. Brief 15 §3.4.
fn paint_rail_bodies(canvas: &mut Canvas, key: ArtKey, leg: &LegWalk) {
    let iso = key.projection == rail_map::Projection::Iso;
    let (ox, oy) = leg.rail_shadow_offset();
    for side in [-RAIL_GAUGE_HALF, RAIL_GAUGE_HALF] {
        for t in leg.run_samples(WALK_STEP) {
            let p = leg.at(t, side);
            if iso {
                canvas.put(
                    p.x.round() as i32 + ox,
                    p.y.round() as i32 + oy,
                    rgba(RAIL_D),
                );
                continue;
            }
            for w in [-RAIL_BODY_HALF, RAIL_BODY_HALF] {
                let color = if w < 0 { RAIL_D } else { RAIL_M };
                let p = leg.at(t, side + w as f32);
                canvas.put(p.x.round() as i32, p.y.round() as i32, rgba(color));
            }
        }
    }
}

/// The one texel at rail top, at gauge, on both rails.
///
/// [`Pass::Polish`] paints the same texels in `railS` and nothing else, which is
/// what the gleam fades up.
fn paint_railheads(canvas: &mut Canvas, _key: ArtKey, leg: &LegWalk, pass: Pass) {
    let color = rgba(match pass {
        Pass::Base => RAIL_L,
        Pass::Polish => RAIL_S,
    });
    for side in [-RAIL_GAUGE_HALF, RAIL_GAUGE_HALF] {
        for t in leg.run_samples(WALK_STEP) {
            let p = leg.at(t, side);
            canvas.put(p.x.round() as i32, p.y.round() as i32, color);
        }
    }
}

// ── Systems ────────────────────────────────────────────────────────────────

/// Where a piece stands on the ground plane.
///
/// A track sprite is placed once and then left alone, which puts it in exactly
/// the class [`GroundAnchor`] exists for — so it wears one and
/// `map::projection::anchor_world_sprites` owns its position from then on.
///
/// That is not tidiness. A load replaces the map and the network in the same
/// `Update`, and whether this system runs before or after the one that does it
/// is not ordered. Landing on the wrong side meant baking the whole railway
/// against the *previous* world's elevation, permanently. With the anchor there
/// is no wrong side: whatever frame the heights arrive on, the pieces are over
/// their own tiles by the end of it.
fn track_anchor(tile: TileCoord) -> GroundAnchor {
    let (gx, gy) = rail_map::tile_to_ground(tile);
    GroundAnchor::new(gx, gy)
}

/// The cell a piece at `tile` with these links draws, in the live projection.
///
/// The one place a key is assembled, so a build ghost and the piece it becomes
/// cannot key differently — see [`ghost_cell`].
fn cell_key(tile: TileCoord, links: u16, bridge: bool) -> ArtKey {
    ArtKey {
        links,
        bridge,
        variant: variant_for(tile),
        projection: rail_map::projection(),
        grades: LegGrades::for_projection(tile, links),
    }
}

/// One baked cell's texels and its edge, for the GPU-free screenshot
/// compositor in `map::terrain::iso`'s tests.
///
/// The same paint the renderer uses, keyed the same way, so a picture written
/// out of it is a picture of what is on screen.
#[cfg(test)]
pub(crate) fn test_cell(tile: TileCoord, links: u16, bridge: bool) -> (u32, Vec<u8>) {
    (
        CELL,
        paint_cell(cell_key(tile, links, bridge), Pass::Base).px,
    )
}

/// The baked cell a build ghost draws for a tile it is proposing.
///
/// Brief 04 §2.2: the ghost is *the actual track art*, and brief 15 §5 says why
/// that stops being a nicety once the ground has gradient — a player deciding
/// whether to climb has to see the ramp before committing to it. Same bank, same
/// key builder, same image as the placed piece: they cannot drift.
pub fn ghost_cell(
    art: &mut TrackArt,
    images: &mut Assets<Image>,
    tile: TileCoord,
    links: u16,
    bridge: bool,
) -> Handle<Image> {
    art.get(images, cell_key(tile, links, bridge)).0
}

/// Reconcile the track sprites against the network.
///
/// # Why this reconciles instead of listening
///
/// It used to be driven purely by [`TrackEdit`] messages, with one extra
/// trigger for a wholesale swap: `network.is_added()`. That trigger does not
/// fire on a load, and the reason is a detail of Bevy's change detection —
/// `World::insert_resource` over a resource that **already exists** replaces the
/// value and sets its *changed* tick, but not its *added* tick. A load restores
/// `TrackNetwork` into a session that already had one, so `is_added()` is false,
/// no edit messages come with a restored network, and this system returned
/// early having done nothing. The player's railway came back in the simulation
/// and never appeared on screen; whatever track the previous world had drawn
/// stayed where it was. Stations did not have the bug because
/// `stations::visuals` has always reconciled against its registry.
///
/// So the question this asks is no longer "did something tell me?" but "does
/// what is drawn match what exists?", which has no trigger to miss:
///
/// - a piece with no sprite gets one,
/// - a sprite with no piece is despawned,
/// - a sprite whose link mask or bridge flag disagrees with its piece is
///   rebuilt.
///
/// That subsumes every trigger the old version needed. Placing one tile changes
/// the mask of everything within a direction step of it — including the far ends
/// of any half-step running across it — and the mask comparison finds those
/// without anyone having to walk the neighbourhood. A new map, a load and a
/// projection flip are all just "the drawing does not match", and so is a
/// re-bake after the projection changes, because [`ArtKey`] carries it.
///
/// The pixel contract's §2.5 is untouched: nothing is *baked* here that is not
/// new. The bank is content-addressed, so a reconcile that finds everything in
/// order costs one hash lookup per piece and paints nothing.
pub fn apply_track_sprites(
    mut commands: Commands,
    mut edits: MessageReader<TrackEdit>,
    network: Res<TrackNetwork>,
    mut images: ResMut<Assets<Image>>,
    mut art: ResMut<TrackArt>,
    existing: Query<(Entity, &TrackSprite)>,
) {
    let _perf = crate::overlays::perf::scope("apply_track_sprites");
    // The messages are still drained — a reader that stops reading loses rather
    // than queues — but nothing downstream depends on having seen them.
    edits.clear();

    // What is drawn, and whether it still agrees with the piece under it.
    let mut wanted: HashSet<TrackId> = network.iter().map(|p| p.id).collect();
    let projection = rail_map::projection();
    for (entity, sprite) in existing.iter() {
        match network.piece(sprite.id) {
            // Drawn correctly: leave the entity alone, and take it off the list
            // of pieces still wanting art.
            Some(piece)
                if sprite.links == piece.links.0
                    && sprite.bridge == piece.is_bridge()
                    && sprite.projection == projection
                    && sprite.grades == LegGrades::for_projection(piece.tile, piece.links.0) =>
            {
                wanted.remove(&sprite.id);
            }
            // Drawn, but the mask, the bridge flag, the projection or the
            // ground under it has moved. Drop the stale art; the spawn pass
            // below redoes it. (The grade recompute sits last in the guard so
            // the settled case pays it only after the cheap fields agree, and
            // in top-down it is [`LegGrades::LEVEL`] by definition — no height
            // reads.)
            Some(_) => commands.entity(entity).despawn(),
            // Drawn, but there is no such piece any more.
            None => commands.entity(entity).despawn(),
        }
    }

    if wanted.is_empty() {
        return;
    }

    // What is left is exactly what needs art: newly placed pieces, neighbours
    // whose mask moved, and everything at all after a load or a new world.
    for id in wanted {
        let Some(piece) = network.piece(id) else {
            continue;
        };
        let key = cell_key(piece.tile, piece.links.0, piece.is_bridge());
        let (base, polish) = art.get(&mut images, key);
        let anchor = track_anchor(piece.tile);
        commands
            .spawn((
                Sprite {
                    image: base,
                    ..default()
                },
                anchor,
                anchor.transform(TRACK_Z),
                TrackSprite {
                    id,
                    links: key.links,
                    bridge: key.bridge,
                    projection: key.projection,
                    grades: key.grades,
                },
            ))
            .with_children(|piece_entity| {
                piece_entity.spawn((
                    Sprite {
                        image: polish,
                        color: Color::srgba(1.0, 1.0, 1.0, 0.0),
                        ..default()
                    },
                    Transform::from_xyz(0.0, 0.0, POLISH_Z),
                    TrackPolish { id },
                ));
            });
    }
}

/// Brighten recently crossed track toward `railS`, fading over ~4 seconds.
///
/// The sim says *what* was crossed and when ([`TileOccupancy::last_crossed`]);
/// the fade is wall-clock and lives here, so the model stays fixed-step while
/// the gleam decays smoothly at any sim speed.
pub fn polish_railheads(
    time: Res<Time>,
    occupancy: Res<TileOccupancy>,
    mut polish: Query<(&TrackPolish, &mut Sprite)>,
    mut heat: Local<HashMap<TrackId, f32>>,
    mut seen_tick: Local<u64>,
) {
    let _perf = crate::overlays::perf::scope("polish_railheads");
    // Crossings recorded since the last frame we looked (FixedUpdate may have
    // run any number of times, or none).
    for (&id, &tick) in occupancy.last_crossed.iter() {
        if tick > *seen_tick {
            heat.insert(id, 1.0);
        }
    }
    *seen_tick = occupancy.tick;

    let fade = time.delta_secs() / POLISH_DECAY_SECS;
    heat.retain(|_, h| {
        *h -= fade;
        *h > 0.0
    });

    for (marker, mut sprite) in polish.iter_mut() {
        let wanted = gleam_alpha(heat.get(&marker.id).copied().unwrap_or(0.0));
        if sprite.color.alpha() != wanted {
            sprite.color.set_alpha(wanted);
        }
    }
}

/// How strongly the `railS` layer shows, for a heat in \[0, 1\].
fn gleam_alpha(heat: f32) -> f32 {
    heat.clamp(0.0, 1.0) * POLISH_MAX
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_map::TILE_SIZE;
    use rail_sim::track::is_half_step;

    /// The brief's nominal sleeper pitch, which every link is fitted a whole
    /// number of — see [`iso_incline::sleeper_pitch`].
    use iso_incline::NOMINAL_SLEEPER_PITCH as SLEEPER_SPACING;

    /// Hold the top-down projection for a test that bakes a cell.
    ///
    /// The bake reads the live projection (see [`ArtKey`]), which is a
    /// process-global, so every test in this module that paints anything has to
    /// pin it — otherwise a test that installs isometric and a test that does
    /// not will interleave and one of them will measure the other's drawing.
    fn flat() -> crate::map::tests::ProjectionGuard {
        crate::map::tests::ProjectionGuard::new(rail_map::Projection::TopDown)
    }

    /// A level cell key in whichever projection the test has installed.
    fn key(links: u16, bridge: bool) -> ArtKey {
        graded_key(links, bridge, LegGrades::LEVEL)
    }

    /// The same, on ground that moves.
    fn graded_key(links: u16, bridge: bool, grades: LegGrades) -> ArtKey {
        ArtKey {
            links,
            bridge,
            variant: 0,
            projection: rail_map::projection(),
            grades,
        }
    }

    fn opaque(px: [u8; 4]) -> bool {
        px[3] > 0
    }

    /// §2.2, the whole reason the direction count is load-bearing: no track
    /// sprite is ever rotated.
    #[test]
    fn track_sprites_are_never_rotated() {
        let _flat = flat();
        for x in -3..=3 {
            for y in -3..=3 {
                let tf = track_anchor(TileCoord { x, y }).transform(TRACK_Z);
                assert_eq!(tf.rotation, Quat::IDENTITY);
                assert_eq!(tf.scale, Vec3::ONE);
            }
        }
    }

    /// A piece's anchor and its tile have to be the same place, or the railway
    /// and the station on the same tile would disagree about where that tile is.
    #[test]
    fn a_piece_is_anchored_on_its_own_tile() {
        for projection in [rail_map::Projection::TopDown, rail_map::Projection::Iso] {
            let _guard = crate::map::tests::ProjectionGuard::new(projection);
            for x in -2..=6 {
                for y in -2..=6 {
                    let tile = TileCoord { x, y };
                    let (wx, wy) = rail_map::tile_to_world(tile);
                    assert_eq!(
                        track_anchor(tile).world(),
                        Vec2::new(wx, wy),
                        "{tile:?} in {projection:?}"
                    );
                }
            }
        }
    }

    /// Direction is expressed by picking a different cell, so every one of the
    /// sixteen has to produce different pixels from every other.
    #[test]
    fn all_sixteen_directions_are_distinct_sprites() {
        let _flat = flat();
        let cells: Vec<Vec<u8>> = (0..DIR_COUNT)
            .map(|d| paint_cell(key(1 << d, false), Pass::Base).px)
            .collect();
        for a in 0..DIR_COUNT {
            for b in (a + 1)..DIR_COUNT {
                assert_ne!(cells[a], cells[b], "dirs {a} and {b} bake identically");
            }
        }
    }

    /// A half-step leg reaches past its own tile, because the tiles it crosses
    /// carry no piece of their own to draw it.
    #[test]
    fn half_step_legs_reach_further_than_compass_legs() {
        let leg_reach = iso_incline::leg_reach;
        let ortho = leg_reach(2);
        let diagonal = leg_reach(1);
        let half = leg_reach(9);
        assert!((ortho - 16.0).abs() < 0.01, "ortho half-length {ortho}");
        assert!((diagonal - 22.63).abs() < 0.05, "diagonal {diagonal}");
        assert!((half - 35.78).abs() < 0.05, "half-step {half}");
        assert!(half > diagonal && diagonal > ortho);
        // And it still fits the cell with its ballast on.
        assert!(half + BALLAST_HALF < CENTER as f32);
        for d in 0..DIR_COUNT {
            assert_eq!(is_half_step(d), leg_reach(d) > TEXELS_PER_TILE);
        }
    }

    /// Sample the cell at `t` texels along a leg and `s` across it, in whichever
    /// projection the bake used.
    ///
    /// From above `along` and `across` are the screen axes and this is a screen
    /// column. In isometric both are projected, so the same `(t, s)` walks the
    /// *ground* plane — which is the point: the §5.3 cross-section is a fact
    /// about the railway, not about the screen, and it has to hold in both.
    fn at_run(canvas: &Canvas, dir: usize, t: f32, s: f32) -> [u8; 4] {
        let (along, across) = axes(dir, rail_map::projection());
        let p = along * t + across * s;
        canvas.at(p.x.round() as i32, p.y.round() as i32)
    }

    /// The §5.3 line weights, measured off the baked art rather than asserted
    /// in a comment — in both projections, because the bake walks the ground
    /// plane in each and the cross-section is a fact about the ground.
    #[test]
    fn the_cross_section_matches_the_brief() {
        for projection in [rail_map::Projection::TopDown, rail_map::Projection::Iso] {
            let _guard = crate::map::tests::ProjectionGuard::new(projection);
            let dir = 2; // due east
            let canvas = paint_cell(key(1 << dir, false), Pass::Base);
            // Ten texels along the leg, clear of the sleeper at t = 0.
            let t = 10.0;
            let half = BALLAST_HALF as i32;
            let bed: Vec<i32> = (-20..=20)
                .filter(|&s| opaque(at_run(&canvas, dir, t, s as f32)))
                .collect();
            // The bed's edge is a straight line from above and a 2:1 staircase
            // in isometric, where a sample one step out can land on the
            // neighbouring step's texel. So: 8 either side of the centreline is
            // solid in both, and only isometric is allowed the one texel of
            // staircase past it.
            let slack = match projection {
                rail_map::Projection::TopDown => 0,
                rail_map::Projection::Iso => 1,
            };
            for s in -half..=half {
                assert!(
                    bed.contains(&s),
                    "a hole in the ballast at {s} across, in {projection:?}"
                );
            }
            // Asymmetric on purpose: which flank the staircase shows depends on
            // which way the projected edge rounds, so each end is bounded
            // rather than pinned.
            let (lo, hi) = (*bed.first().unwrap(), *bed.last().unwrap());
            assert!(
                (-half - slack..=-half).contains(&lo) && (half..=half + slack).contains(&hi),
                "ballast bed runs {lo}..={hi} across in {projection:?}, not 8 either side"
            );

            // Rails at ±4 with a bright head on the centre texel of each.
            for gauge in [-RAIL_GAUGE_HALF, RAIL_GAUGE_HALF] {
                assert_eq!(
                    at_run(&canvas, dir, t, gauge),
                    rgba(RAIL_L),
                    "railhead missing at gauge {gauge} in {projection:?}"
                );
                // The one-texel web either side of the head is checked from
                // above only, and this is a real limit of the projection rather
                // than a gap in the test: one unit across projects to 1.12
                // texels on a 2:1 staircase, so the head painted half a step
                // further along the run rounds onto the very texel the web
                // wants. A three-texel rail — shadow, head, body — cannot
                // survive being drawn on a diamond lattice at 32 texels to the
                // tile. Either the rail gets wider or the flanks go.
                if projection == rail_map::Projection::TopDown {
                    assert_eq!(at_run(&canvas, dir, t, gauge - 1.0), rgba(RAIL_D));
                    assert_eq!(at_run(&canvas, dir, t, gauge + 1.0), rgba(RAIL_M));
                }
            }
            // Gauge really is 8 centre to centre.
            assert_eq!(RAIL_GAUGE_HALF * 2.0, 8.0);
        }
    }

    #[test]
    fn sleepers_sit_at_the_briefs_spacing_and_length() {
        for projection in [rail_map::Projection::TopDown, rail_map::Projection::Iso] {
            let _guard = crate::map::tests::ProjectionGuard::new(projection);
            let dir = 2; // due east
            let canvas = paint_cell(key(1 << dir, false), Pass::Base);
            let is_tie = |t: f32, s: f32| {
                let px = at_run(&canvas, dir, t, s);
                px == rgba(TIE_D) || px == rgba(TIE_M) || px == rgba(TIE_L)
            };
            // Sleepers every 4 texels along the run; between them, no tie colour
            // outside the rails.
            for n in 0..3 {
                let t = n as f32 * SLEEPER_SPACING;
                assert!(is_tie(t, 6.0), "no sleeper at t={t} in {projection:?}");
                assert!(
                    !is_tie(t + 2.0, 6.0),
                    "sleeper bled between pitches at t={t} in {projection:?}"
                );
            }
            // Length 14 → ±7, and nothing beyond.
            assert!(is_tie(SLEEPER_SPACING, SLEEPER_HALF));
            assert!(!is_tie(SLEEPER_SPACING, SLEEPER_HALF + 1.5));
        }
    }

    /// The same mask is a different drawing in each projection, and the bank
    /// keeps both — so the player who flips back and forth pays each bake once.
    #[test]
    fn the_bank_holds_a_cell_per_projection() {
        let flat = {
            let _guard = crate::map::tests::ProjectionGuard::new(rail_map::Projection::TopDown);
            (key(1 << 2, false), paint_cell(key(1 << 2, false), Pass::Base))
        };
        let iso = {
            let _guard = crate::map::tests::ProjectionGuard::new(rail_map::Projection::Iso);
            (key(1 << 2, false), paint_cell(key(1 << 2, false), Pass::Base))
        };
        assert_ne!(flat.0, iso.0, "the projection has to be part of the key");
        assert_ne!(flat.1.px, iso.1.px, "and the two keys have to draw apart");
    }

    /// The polish layer is railhead only, so fading it up cannot smear the
    /// ballast — and it stays in register with the base even at a junction,
    /// where several legs' rails cross each other.
    #[test]
    fn the_polish_layer_holds_only_railheads() {
        let _flat = flat();
        let masks = [
            1u16 << 2,                                     // a straight east leg
            (1 << 2) | (1 << 6),                           // straight through
            (1 << 0) | (1 << 4) | (1 << 8),                // turnout with a shallow leg
            (1 << 0) | (1 << 4) | (1 << 2) | (1 << 6),     // a flat crossing
            (1 << 9) | (1 << 13),                          // a shallow run
        ];
        for mask in masks {
            let base = paint_cell(key(mask, false), Pass::Base);
            let polish = paint_cell(key(mask, false), Pass::Polish);
            let mut painted = 0;
            for x in -CENTER..CENTER {
                for y in -CENTER..CENTER {
                    let p = polish.at(x, y);
                    if !opaque(p) {
                        continue;
                    }
                    painted += 1;
                    assert_eq!(p, rgba(RAIL_S), "polish layer must be railS only");
                    assert_eq!(
                        base.at(x, y),
                        rgba(RAIL_L),
                        "mask {mask:#x}: polish texel at ({x},{y}) is not over a railhead"
                    );
                }
            }
            assert!(painted > 0, "mask {mask:#x} should have railheads to polish");
            // Far less of the cell than the base layer covers.
            let base_painted = (-CENTER..CENTER)
                .flat_map(|x| (-CENTER..CENTER).map(move |y| (x, y)))
                .filter(|&(x, y)| opaque(base.at(x, y)))
                .count();
            assert!(painted * 4 < base_painted, "mask {mask:#x}");
        }
    }

    /// Counts colours rather than probing fixed texels
    /// — the claim ("a bridge is timber, not ballast, and still carries rail")
    /// is the same, but where a given texel lands is now the projection's
    /// business.
    #[test]
    fn a_bridge_decks_in_timber_instead_of_ballast() {
        let _flat = flat();
        let ground = paint_cell(key(1 << 2, false), Pass::Base);
        let bridge = paint_cell(key(1 << 2, true), Pass::Base);
        assert_ne!(ground.px, bridge.px);
        let count = |c: &Canvas, want: Color| {
            let want = rgba(want);
            c.px.chunks_exact(4).filter(|p| *p == want).count()
        };
        // One 16-unit leg of 16-unit-wide deck is ~256 ground units²; the
        // projection preserves area (its determinant is 1) and rasterising it
        // rounds some away, so the floor is well under that.
        let planks = count(&bridge, WOOD_D) + count(&bridge, WOOD_M);
        assert!(planks > 120, "deck should be planked: {planks} texels");
        assert_eq!(
            count(&ground, WOOD_D) + count(&ground, WOOD_M),
            0,
            "ballast should not be timber"
        );
        assert_eq!(
            count(&bridge, BALLAST_D) + count(&bridge, BALLAST_M),
            0,
            "a deck should not also be ballasted"
        );
        // Rails still run over the deck.
        assert!(count(&bridge, RAIL_L) > 20, "no railhead over the deck");
    }

    #[test]
    fn an_isolated_piece_still_reads_as_track() {
        let _flat = flat();
        let lone = paint_cell(key(0, false), Pass::Base);
        let painted = (-CENTER..CENTER)
            .flat_map(|x| (-CENTER..CENTER).map(move |y| (x, y)))
            .filter(|&(x, y)| opaque(lone.at(x, y)))
            .count();
        assert!(painted > 500, "a lone tile should still show a stub");
        // The stub still carries railhead, but which texel it
        // lands on is the projection's business, so count instead of probing.
        let heads = lone
            .px
            .chunks_exact(4)
            .filter(|p| *p == rgba(RAIL_L))
            .count();
        assert!(heads > 20, "a lone stub with no railhead: {heads}");
    }

    /// A junction is one cell, not a rotation of a straight — the mask picks
    /// which legs get stamped.
    #[test]
    fn a_turnout_composites_all_of_its_legs() {
        let _flat = flat();
        // N, S and NNE: a through route with a shallow diverging leg.
        let turnout = paint_cell(key((1 << 0) | (1 << 4) | (1 << 8), false), Pass::Base);
        let straight = paint_cell(key((1 << 0) | (1 << 4), false), Pass::Base);
        assert_ne!(turnout.px, straight.px);
        let painted = |c: &Canvas| {
            (-CENTER..CENTER)
                .flat_map(|x| (-CENTER..CENTER).map(move |y| (x, y)))
                .filter(|&(x, y)| opaque(c.at(x, y)))
                .count()
        };
        assert!(
            painted(&turnout) > painted(&straight),
            "the diverging leg has to add ballast"
        );
    }

    /// World-anchored, never screen- or time-anchored (§2.4).
    #[test]
    fn decoration_variants_are_world_anchored() {
        let _flat = flat();
        let a = variant_for(TileCoord { x: 4, y: 9 });
        assert_eq!(a, variant_for(TileCoord { x: 4, y: 9 }), "must be stable");
        assert!(a < VARIANTS);
        let spread: HashSet<u32> = (0..64)
            .map(|i| variant_for(TileCoord { x: i, y: i * 3 }))
            .collect();
        assert!(spread.len() > 1, "every tile took the same variant");
    }

    #[test]
    fn a_busy_line_gleams_and_a_quiet_one_does_not() {
        assert_eq!(gleam_alpha(0.0), 0.0);
        assert!(gleam_alpha(1.0) > gleam_alpha(0.1));
        assert!(gleam_alpha(1.0) <= 1.0);
        assert_eq!(gleam_alpha(5.0), gleam_alpha(1.0), "clamped");
    }

    #[test]
    fn gleam_decays_monotonically() {
        let mut previous = f32::MAX;
        for step in 0..=4 {
            let alpha = gleam_alpha(1.0 - step as f32 / POLISH_DECAY_SECS);
            assert!(alpha < previous, "step {step} did not fade");
            previous = alpha;
        }
        assert_eq!(previous, 0.0, "the gleam reaches zero after ~4s");
    }

    /// Nothing about the cell exceeds the pixel contract: the image is a whole
    /// number of tiles and the sampler is nearest.
    /// Bake on edit, once per distinct cell — the same mask must never be
    /// painted twice (§2.5).
    #[test]
    fn the_bank_bakes_each_cell_once() {
        let _flat = flat();
        let mut app = App::new();
        app.init_resource::<Assets<Image>>();
        let images = &mut app.world_mut().resource_mut::<Assets<Image>>();
        let mut art = TrackArt::default();

        let straight = key((1 << 2) | (1 << 6), false);
        let first = art.get(images, straight);
        let again = art.get(images, straight);
        assert_eq!(art.baked(), 1);
        assert_eq!(first.0.id(), again.0.id(), "cache must reuse the handle");
        assert_eq!(first.1.id(), again.1.id());

        // A different mask, a different variant and a bridge are all new cells.
        art.get(images, key((1 << 2) | (1 << 9), false));
        art.get(images, straight_variant(straight));
        art.get(images, key((1 << 2) | (1 << 6), true));
        assert_eq!(art.baked(), 4);
    }

    fn straight_variant(mut key: ArtKey) -> ArtKey {
        key.variant = 1;
        key
    }

    /// Every cell the golden hashes below were taken from, in this order:
    /// mask, then bridge on and off, then each variant, then base and polish.
    const GOLDEN_MASKS: [u16; 8] = [
        0,
        1 << 2,
        (1 << 2) | (1 << 6),
        (1 << 1) | (1 << 5),
        (1 << 9) | (1 << 13),
        (1 << 0) | (1 << 4) | (1 << 8),
        (1 << 0) | (1 << 2) | (1 << 4) | (1 << 6),
        u16::MAX,
    ];

    fn fnv(bytes: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    /// The shipping view, pinned.
    ///
    /// Brief 15 is the isometric brief and its acceptance bar says top-down is
    /// **byte-identical** — not "looks the same". These hashes were taken off
    /// the build before any of it landed. They are what lets the isometric side
    /// change the sleeper ladder, the rail flanks and the walk schedule without
    /// anyone having to argue about whether the flat view moved.
    ///
    /// If one of these fails, the change under it belongs behind a projection
    /// branch, or it is a deliberate decision to spend the flat view — in which
    /// case re-capture the table in the same commit and say so.
    #[rustfmt::skip]
    const TOP_DOWN_GOLDENS: [u64; 96] = [
        14279131206508433021, 4251134606262063145, 2548574493154120837, 4251134606262063145,
        4682904878814344161, 4251134606262063145, 15818763141173590427, 4251134606262063145,
        15818763141173590427, 4251134606262063145, 15818763141173590427, 4251134606262063145,
        20949475640176865, 1848616202308622185, 3442062059530353033, 1848616202308622185,
        10051392013674296515, 1848616202308622185, 2434013900955538527, 1848616202308622185,
        2434013900955538527, 1848616202308622185, 2434013900955538527, 1848616202308622185,
        14279131206508433021, 4251134606262063145, 2548574493154120837, 4251134606262063145,
        4682904878814344161, 4251134606262063145, 15818763141173590427, 4251134606262063145,
        15818763141173590427, 4251134606262063145, 15818763141173590427, 4251134606262063145,
        10021040785841463931, 5415026587788133417, 11145597436246719197, 5415026587788133417,
        11103567068349965329, 5415026587788133417, 8029028079134422891, 5415026587788133417,
        8029028079134422891, 5415026587788133417, 8029028079134422891, 5415026587788133417,
        7304259369244570080, 9282574325632799141, 11552551435609001860, 9282574325632799141,
        17813576736688771034, 9282574325632799141, 6277052934909526454, 9282574325632799141,
        6277052934909526454, 9282574325632799141, 6277052934909526454, 9282574325632799141,
        14545309150168582059, 12240603529453423137, 1727400699027849841, 12240603529453423137,
        14454735774239370567, 12240603529453423137, 1532373362082157337, 12240603529453423137,
        1532373362082157337, 12240603529453423137, 1532373362082157337, 12240603529453423137,
        12136577767953991463, 1763473954715865637, 15073668004836713331, 1763473954715865637,
        750752748192526065, 1763473954715865637, 11647193144100387911, 1763473954715865637,
        11647193144100387911, 1763473954715865637, 11647193144100387911, 1763473954715865637,
        10906804928568260663, 1245286076660785085, 3611001728013101295, 1245286076660785085,
        4542370361804784525, 1245286076660785085, 5782233378944642435, 1245286076660785085,
        5782233378944642435, 1245286076660785085, 5782233378944642435, 1245286076660785085,
    ];

    #[test]
    fn the_top_down_view_is_byte_identical() {
        let _flat = flat();
        let mut i = 0usize;
        for mask in GOLDEN_MASKS {
            for bridge in [false, true] {
                for variant in 0..VARIANTS {
                    let mut k = key(mask, bridge);
                    k.variant = variant;
                    for pass in [Pass::Base, Pass::Polish] {
                        assert_eq!(
                            fnv(&paint_cell(k, pass).px),
                            TOP_DOWN_GOLDENS[i],
                            "top-down cell moved: mask {mask:#06x} bridge {bridge} \
                             variant {variant} {pass:?}"
                        );
                        i += 1;
                    }
                }
            }
        }
        assert_eq!(i, TOP_DOWN_GOLDENS.len(), "the table lost its shape");
    }

    /// From above there is no lift, so a grade may not reach the drawing even
    /// if one is somehow keyed.
    #[test]
    fn a_grade_cannot_change_the_top_down_cell() {
        let _flat = flat();
        let mut climbing = LegGrades::LEVEL;
        for dir in 0..DIR_COUNT {
            climbing = LegGrades::around(TileCoord { x: 0, y: 0 }, 1 << dir, |c| {
                i8::from(c != TileCoord { x: 0, y: 0 })
            });
            let level = paint_cell(key(1 << dir, false), Pass::Base);
            let ramped = paint_cell(graded_key(1 << dir, false, climbing), Pass::Base);
            assert_eq!(level.px, ramped.px, "dir {dir} ramped from above");
        }
        assert_ne!(climbing, LegGrades::LEVEL, "the test climbed nothing");
    }

    // ── Connected, measurably (brief 15 §4) ────────────────────────────────
    //
    // Every test below composites the *two* cells that share a link into one
    // absolute screen space and measures across the joint, because that is
    // where the failure lives: each cell on its own has always been fine.

    /// The two halves of one link, painted and placed where the projection puts
    /// them.
    struct Joint {
        near: Canvas,
        far: Canvas,
        origin: (f32, f32),
        end: (f32, f32),
    }

    impl Joint {
        /// Lay `dir` out of `(8, 8)` onto ground that steps by `grade`, and bake
        /// both ends of it.
        ///
        /// The caller must be holding the projection guard: this installs a
        /// height field, which is the other process-global.
        fn build(dir: usize, grade: i8) -> Self {
            let a = TileCoord { x: 8, y: 8 };
            let b = rail_sim::track::step(a, dir);
            let mut map = rail_map::MapGrid::empty(24, 24, 1);
            // Everything at 4 so a downhill leg has somewhere to go.
            for tile in map.tiles_mut() {
                tile.height = 4;
            }
            map.get_mut(b).unwrap().height = 4 + grade;
            crate::map::projection::set_iso_heights(&map);

            let opposite = rail_sim::track::opposite_dir(dir);
            let height_of = rail_map::tile_height;
            Self {
                near: paint_cell(
                    graded_key(1 << dir, false, LegGrades::around(a, 1 << dir, height_of)),
                    Pass::Base,
                ),
                far: paint_cell(
                    graded_key(
                        1 << opposite,
                        false,
                        LegGrades::around(b, 1 << opposite, height_of),
                    ),
                    Pass::Base,
                ),
                origin: tile_to_world(a),
                end: tile_to_world(b),
            }
        }

        /// Is anything drawn at this absolute screen point, by either end?
        fn painted(&self, sx: f32, sy: f32) -> bool {
            let sample = |c: &Canvas, (ox, oy): (f32, f32)| {
                opaque(c.at((sx - ox).round() as i32, (sy - oy).round() as i32))
            };
            sample(&self.near, self.origin) || sample(&self.far, self.end)
        }

        /// Absolute screen position of a texel `(x, y)` in the near cell.
        fn near_point(&self, p: Vec2) -> (f32, f32) {
            (
                self.origin.0 + p.x.round(),
                self.origin.1 + p.y.round(),
            )
        }

        fn far_point(&self, p: Vec2) -> (f32, f32) {
            (self.end.0 + p.x.round(), self.end.1 + p.y.round())
        }
    }

    /// Every grade a leg can legally be drawn at. `MAX_GRADE` either way, plus
    /// the level case — and a leg onto water is exempt from the grade rule
    /// entirely, which is why the drawing goes past it.
    fn legal_grades() -> Vec<i8> {
        let max = rail_sim::MAX_GRADE as i8;
        (-max..=max).collect()
    }

    /// Clause 2 of §4, and the one the owner asked for: the two halves of a
    /// climbing leg end on the *same texel*, and that texel is the midpoint of
    /// the two tile centres. Asserted as equality — §2.2 shows there is nothing
    /// to round.
    #[test]
    fn a_climbing_leg_ends_where_the_other_half_begins() {
        let _iso = crate::map::tests::ProjectionGuard::new(rail_map::Projection::Iso);
        for dir in 0..DIR_COUNT {
            for grade in legal_grades() {
                let joint = Joint::build(dir, grade);
                let near = LegWalk::new(dir, grade, rail_map::Projection::Iso);
                let far = LegWalk::new(
                    rail_sim::track::opposite_dir(dir),
                    -grade,
                    rail_map::Projection::Iso,
                );

                // The run starts on one tile centre and finishes on the other.
                assert_eq!(joint.near_point(near.at(0.0, 0.0)), joint.origin);
                assert_eq!(joint.far_point(far.at(0.0, 0.0)), joint.end);

                // ... and the two halves meet at the midpoint of the two.
                let midpoint = (
                    (joint.origin.0 + joint.end.0) * 0.5,
                    (joint.origin.1 + joint.end.1) * 0.5,
                );
                assert_eq!(
                    joint.near_point(near.at(near.reach, 0.0)),
                    midpoint,
                    "dir {dir} grade {grade}: the near half does not reach the joint"
                );
                assert_eq!(
                    joint.far_point(far.at(far.reach, 0.0)),
                    midpoint,
                    "dir {dir} grade {grade}: the far half does not reach the joint"
                );

                // And both of them actually painted something there.
                assert!(
                    joint.painted(midpoint.0, midpoint.1),
                    "dir {dir} grade {grade}: nothing drawn at the joint"
                );
            }
        }
        crate::map::projection::clear_iso_heights();
    }

    /// Clauses 1 and 3: walk the whole run and find no hole, and no step where
    /// the two cells hand over. This is the test the flat prototype had to stop
    /// short of, run over ground that moves.
    #[test]
    fn a_run_is_unbroken_from_one_tile_centre_to_the_other() {
        let _iso = crate::map::tests::ProjectionGuard::new(rail_map::Projection::Iso);
        for dir in 0..DIR_COUNT {
            for grade in legal_grades() {
                let joint = Joint::build(dir, grade);
                // The straight screen segment between the two centres *is* the
                // run: the projection is affine in height, so the ramp is not
                // an approximation of this line, it is this line.
                let steps = 400;
                for i in 0..=steps {
                    let f = i as f32 / steps as f32;
                    let sx = joint.origin.0 + (joint.end.0 - joint.origin.0) * f;
                    let sy = joint.origin.1 + (joint.end.1 - joint.origin.1) * f;
                    assert!(
                        joint.painted(sx, sy),
                        "dir {dir} grade {grade}: a hole {:.0}% along the run at ({sx}, {sy})",
                        f * 100.0
                    );
                }
            }
        }
        crate::map::projection::clear_iso_heights();
    }

    /// Clause 4, and the flat view's own failure: the sleeper rhythm has to
    /// carry through the boundary. Before the ladder was pitched to the link,
    /// this measured 4, 4, 4 and then 6 or 7 at every diagonal joint.
    #[test]
    fn the_sleeper_rhythm_carries_across_the_boundary() {
        let _iso = crate::map::tests::ProjectionGuard::new(rail_map::Projection::Iso);
        let is_tie = |px: [u8; 4]| px == rgba(TIE_D) || px == rgba(TIE_M) || px == rgba(TIE_L);
        for dir in 0..DIR_COUNT {
            for grade in [0i8, 1, -1, 4] {
                let joint = Joint::build(dir, grade);
                let near = LegWalk::new(dir, grade, rail_map::Projection::Iso);
                let far = LegWalk::new(
                    rail_sim::track::opposite_dir(dir),
                    -grade,
                    rail_map::Projection::Iso,
                );
                let run = iso_incline::link_run(dir);
                let pitch = iso_incline::sleeper_pitch(dir, rail_map::Projection::Iso);

                // Where each end says its sleepers are, as one ladder along the
                // link, and painted where it says.
                let mut ladder: Vec<f32> = near
                    .sleepers()
                    .inspect(|&t| {
                        assert!(
                            is_tie(joint.near.at(
                                near.at(t, 0.0).x.round() as i32,
                                near.at(t, 0.0).y.round() as i32
                            )),
                            "dir {dir} grade {grade}: no sleeper painted at t={t}"
                        );
                    })
                    .collect();
                ladder.extend(far.sleepers().map(|t| run - t).inspect(|_| {}));
                ladder.sort_by(|a, b| a.partial_cmp(b).unwrap());
                ladder.dedup_by(|a, b| (*a - *b).abs() < 1e-3);

                for w in ladder.windows(2) {
                    assert!(
                        (w[1] - w[0] - pitch).abs() < 1e-3,
                        "dir {dir} grade {grade}: sleepers {:.2} and {:.2} are {:.2} apart, \
                         not {pitch:.2}",
                        w[0],
                        w[1],
                        w[1] - w[0]
                    );
                }
                assert!(
                    ladder.len() >= 8,
                    "dir {dir}: a link should carry at least eight sleepers"
                );
            }
        }
        crate::map::projection::clear_iso_heights();
    }

    /// A climbing leg never floats: the column under its bed is solid all the
    /// way down to where its own tile's surface is.
    ///
    /// That is the property the embankment exists for, and it is the one worth
    /// asserting rather than counting bank texels. On a leg that climbs the
    /// screen steeply — north-east projects to a straight vertical run — the
    /// bed's own earlier samples already cover everything below it and the
    /// skirt is drawn and then buried, correctly, because there was no gap to
    /// fill. On a shallow one the skirt is the only thing holding the bed up.
    #[test]
    fn a_climbing_leg_never_floats_over_its_own_ground() {
        let _iso = crate::map::tests::ProjectionGuard::new(rail_map::Projection::Iso);
        for dir in 0..DIR_COUNT {
            for grade in 1i8..=4 {
                let joint = Joint::build(dir, grade);
                let walk = LegWalk::new(dir, grade, rail_map::Projection::Iso);
                for k in 1..=8 {
                    let t = walk.reach * k as f32 / 8.0;
                    let ground = walk.flat_at(t, 0.0).y.round() as i32;
                    let bed = walk.at(t, 0.0).y.round() as i32;
                    let x = walk.at(t, 0.0).x.round() as i32;
                    for y in ground..=bed {
                        assert!(
                            opaque(joint.near.at(x, y)),
                            "dir {dir} grade {grade}: daylight under the bed at \
                             ({x}, {y}), {t:.1} along the run"
                        );
                    }
                }
            }
        }
        crate::map::projection::clear_iso_heights();
    }

    /// A descent is a cutting: the track draws over the ground it is notched
    /// into, and nothing draws earth beneath it in mid-air.
    #[test]
    fn a_descent_builds_no_bank_in_mid_air() {
        let _iso = crate::map::tests::ProjectionGuard::new(rail_map::Projection::Iso);
        let count = |c: &Canvas, want: Color| {
            let want = rgba(want);
            c.px.chunks_exact(4).filter(|p| *p == want).count()
        };
        for dir in 0..DIR_COUNT {
            assert_eq!(
                count(&Joint::build(dir, 0).near, OUTLINE),
                0,
                "dir {dir}: level track should have no bank under it"
            );
            assert_eq!(
                count(&Joint::build(dir, -2).near, OUTLINE),
                0,
                "dir {dir}: a descent is a cutting, not a bank in mid-air"
            );
        }
        // And a shallow climb — where the bed does not cover its own flat path
        // — really does show the bank, floor line and all.
        let shallow = Joint::build(3, 3); // south-east: a level screen run
        assert!(
            count(&shallow.near, OUTLINE) > 8,
            "a shallow climb should stand on a visible embankment"
        );
        assert!(
            count(&shallow.near, BALLAST_L) > 0,
            "the bank buried the bed's own sun edge"
        );
        crate::map::projection::clear_iso_heights();
    }

    /// The ramp has to reach every layer, or the rails float off their own
    /// sleepers half way up the hill.
    #[test]
    fn every_layer_climbs_together() {
        let _iso = crate::map::tests::ProjectionGuard::new(rail_map::Projection::Iso);
        for dir in [0usize, 2, 1, 9] {
            let joint = Joint::build(dir, 3);
            let walk = LegWalk::new(dir, 3, rail_map::Projection::Iso);
            // Three quarters of the way along, where the ramp is high enough
            // that a layer left behind would be unmistakable.
            let t = walk.reach * 0.75;
            let head = walk.at(t, RAIL_GAUGE_HALF);
            assert_eq!(
                joint.near.at(head.x.round() as i32, head.y.round() as i32),
                rgba(RAIL_L),
                "dir {dir}: the railhead did not climb with the bed"
            );
            let bed = walk.at(t, 0.0);
            assert!(
                opaque(joint.near.at(bed.x.round() as i32, bed.y.round() as i32)),
                "dir {dir}: the bed has a hole in the ramp"
            );
            // The polish layer is baked separately and must stay in register.
            let polish = paint_cell(
                graded_key(
                    1 << dir,
                    false,
                    LegGrades::around(TileCoord { x: 8, y: 8 }, 1 << dir, rail_map::tile_height),
                ),
                Pass::Polish,
            );
            assert_eq!(
                polish.at(head.x.round() as i32, head.y.round() as i32),
                rgba(RAIL_S),
                "dir {dir}: the gleam came off the ramp"
            );
        }
        crate::map::projection::clear_iso_heights();
    }

    /// Level ground keys on `LegGrades::LEVEL`, so a flat railway bakes exactly
    /// the vocabulary it baked before the ramps existed.
    #[test]
    fn level_track_does_not_widen_the_bank() {
        let _iso = crate::map::tests::ProjectionGuard::new(rail_map::Projection::Iso);
        let mut map = rail_map::MapGrid::empty(16, 16, 1);
        for tile in map.tiles_mut() {
            tile.height = 3;
        }
        crate::map::projection::set_iso_heights(&map);
        // The interior only. Off the map the height field answers 0, so a
        // border tile on high ground reads a drop into the void — which never
        // reaches a drawing, because a leg is only graded when it is *linked*,
        // and there is no track off the map to link to.
        let interior = || {
            (2..14i32).flat_map(|x| (2..14i32).map(move |y| TileCoord { x, y }))
        };
        for t in interior() {
            assert_eq!(
                LegGrades::for_projection(t, u16::MAX),
                LegGrades::LEVEL,
                "flat ground graded a leg at {t:?}"
            );
        }
        // One bump, and only the tiles that touch it take a new key.
        map.get_mut(TileCoord { x: 8, y: 8 }).unwrap().height = 4;
        crate::map::projection::set_iso_heights(&map);
        let graded = interior()
            .filter(|&t| LegGrades::for_projection(t, u16::MAX) != LegGrades::LEVEL)
            .count();
        // The bump itself plus its sixteen `DIR16` neighbours.
        assert_eq!(graded, 17, "a single bump touched {graded} keys");
        crate::map::projection::clear_iso_heights();
    }

    #[test]
    fn the_cell_honours_the_pixel_contract() {
        let _flat = flat();
        assert_eq!(CELL % TILE_SIZE as u32, 0);
        assert_eq!(TEXELS_PER_TILE, TILE_SIZE);
        let image = cell_image(paint_cell(key(1 << 2, false), Pass::Base));
        assert_eq!(image.width(), CELL);
        assert_eq!(image.height(), CELL);
        assert!(matches!(image.sampler, ImageSampler::Descriptor(_)));
    }
}
