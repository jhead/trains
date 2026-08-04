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

use std::collections::{HashMap, HashSet};

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rail_map::tile_to_world;
use rail_sim::ids::TileCoord;
use rail_sim::track::{DIR16, DIR_COUNT};
use rail_sim::{TileOccupancy, TrackEdit, TrackId, TrackNetwork};

use crate::hash::world_hash;
use crate::palette::{
    BALLAST_D, BALLAST_L, BALLAST_M, RAIL_D, RAIL_L, RAIL_M, RAIL_S, TIE_D, TIE_L, TIE_M, WOOD_D,
    WOOD_M,
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
/// **Iso prototype**: the projection stretches the ground plane to twice its
/// width, so the widest leg (`(1, -2)`, projecting to 48 texels of run) plus its
/// ballast wants ±60. 128 gives that with room to spare; the cell is cached per
/// link mask and only masks that occur are ever baked, so the extra texels cost
/// nothing that matters.
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
/// Sleeper spacing along the run.
const SLEEPER_SPACING: f32 = 4.0;
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
#[derive(Component, Debug, Clone, Copy)]
pub struct TrackSprite {
    pub id: TrackId,
    /// The link mask this art was baked for; a change means a re-bake.
    pub links: u16,
    pub bridge: bool,
}

/// The railhead gleam layer for one piece, a child of its [`TrackSprite`].
#[derive(Component, Debug, Clone, Copy)]
pub struct TrackPolish {
    pub id: TrackId,
}

/// What a baked cell is keyed on. Everything else about a piece is position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ArtKey {
    links: u16,
    bridge: bool,
    variant: u32,
}

/// Baked cells, kept for the life of the session.
///
/// Content-addressed, so a new map or a loaded save reuses whatever it already
/// has and bakes only what is new.
#[derive(Default)]
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
/// **Iso prototype**: both are *projected*, so every painter below draws on the
/// ground plane instead of on the screen plane. This is the whole of the track
/// reprojection — the bed, the sleepers, the rail bodies and the railheads all
/// walk `along · t + across · s` in ground units and land on the diamond grid
/// without any of them knowing the projection exists.
///
/// The vectors are deliberately **not** re-normalised after projecting. A leg
/// running south-east projects to 1.41× its ground length and a leg running
/// north-east to 0.71×, and that foreshortening is exactly what makes a rail
/// read as lying on the ground rather than floating over it. `across` is the
/// projection of the ground-plane perpendicular, not the screen-space
/// perpendicular of the projected run, so the sleepers lie flat too.
fn axes(dir: usize) -> (Vec2, Vec2) {
    let (dx, dy) = DIR16[dir];
    let v = Vec2::new(dx as f32, dy as f32).normalize();
    let perp = Vec2::new(-v.y, v.x);
    (project(v), project(perp))
}

/// Ground-plane vector to screen-plane vector. The projection is linear, so it
/// applies to a direction exactly as it applies to a point.
#[inline]
fn project(v: Vec2) -> Vec2 {
    let (x, y) = rail_map::project(v.x, v.y);
    Vec2::new(x, y)
}

/// Half the length of a link in texels — the share this piece draws.
fn leg_reach(dir: usize) -> f32 {
    let (dx, dy) = DIR16[dir];
    Vec2::new(dx as f32, dy as f32).length() * TEXELS_PER_TILE * 0.5
}

/// Paint one cell for a link mask.
fn paint_cell(key: ArtKey, pass: Pass) -> Canvas {
    let mut canvas = Canvas::new();
    let dirs: Vec<usize> = (0..DIR_COUNT)
        .filter(|&i| key.links & (1 << i) != 0)
        .collect();

    // An isolated piece still has to read as track: give it a short stub along
    // the default axis so a single tile is not an anonymous blob.
    let legs: Vec<(usize, f32)> = if dirs.is_empty() {
        vec![
            (2, TEXELS_PER_TILE * 0.5),
            (6, TEXELS_PER_TILE * 0.5),
        ]
    } else {
        dirs.iter().map(|&d| (d, leg_reach(d))).collect()
    };

    // Bed, then sleepers, then rail bodies, then railheads — each stage across
    // *every* leg before the next begins, exactly as a hand would draw it. That
    // ordering is what makes a junction read as one piece of track: no leg's
    // ballast can bury another leg's rail, and no leg's web can bury another
    // leg's head. It also keeps the two layers in register, so a polish texel is
    // always over a railhead in the base.
    if pass == Pass::Base {
        for &(dir, reach) in &legs {
            paint_bed(&mut canvas, key, dir, reach);
        }
        for &(dir, reach) in &legs {
            paint_sleepers(&mut canvas, key, dir, reach);
        }
        for &(dir, reach) in &legs {
            paint_rail_bodies(&mut canvas, dir, reach);
        }
    }
    for &(dir, reach) in &legs {
        paint_railheads(&mut canvas, dir, reach, pass);
    }
    canvas
}

/// Ballast bed, or bridge deck planking where the piece spans water.
fn paint_bed(canvas: &mut Canvas, key: ArtKey, dir: usize, reach: f32) {
    let (along, across) = axes(dir);
    let mut t = 0.0;
    while t <= reach {
        let mut s = -BALLAST_HALF;
        while s <= BALLAST_HALF {
            let p = along * t + across * s;
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
        t += WALK_STEP;
    }
}

/// Sleepers across the run, one in five taking the light step.
fn paint_sleepers(canvas: &mut Canvas, key: ArtKey, dir: usize, reach: f32) {
    if key.bridge {
        // The deck *is* the sleepers on a bridge.
        return;
    }
    let (along, across) = axes(dir);
    let mut index = 0u32;
    let mut t = 0.0;
    while t <= reach {
        let anchor = along * t;
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
            let p = anchor + across * s;
            canvas.put(p.x.round() as i32, p.y.round() as i32, color);
            s += WALK_STEP;
        }
        index += 1;
        t += SLEEPER_SPACING;
    }
}

/// The web either side of each railhead: shadow on one flank, body on the other
/// (§5.3, rail body half-width 1).
fn paint_rail_bodies(canvas: &mut Canvas, dir: usize, reach: f32) {
    let (along, across) = axes(dir);
    for side in [-RAIL_GAUGE_HALF, RAIL_GAUGE_HALF] {
        let mut t = 0.0;
        while t <= reach {
            for w in [-RAIL_BODY_HALF, RAIL_BODY_HALF] {
                let color = if w < 0 { RAIL_D } else { RAIL_M };
                let p = along * t + across * (side + w as f32);
                canvas.put(p.x.round() as i32, p.y.round() as i32, rgba(color));
            }
            t += WALK_STEP;
        }
    }
}

/// The one texel at rail top, at gauge, on both rails.
///
/// [`Pass::Polish`] paints the same texels in `railS` and nothing else, which is
/// what the gleam fades up.
fn paint_railheads(canvas: &mut Canvas, dir: usize, reach: f32, pass: Pass) {
    let (along, across) = axes(dir);
    let color = rgba(match pass {
        Pass::Base => RAIL_L,
        Pass::Polish => RAIL_S,
    });
    for side in [-RAIL_GAUGE_HALF, RAIL_GAUGE_HALF] {
        let mut t = 0.0;
        while t <= reach {
            let p = along * t + across * side;
            canvas.put(p.x.round() as i32, p.y.round() as i32, color);
            t += WALK_STEP;
        }
    }
}

// ── Systems ────────────────────────────────────────────────────────────────

/// A track sprite's transform. Always identity rotation — see the module docs.
fn track_transform(tile: TileCoord) -> Transform {
    let (wx, wy) = tile_to_world(tile);
    Transform::from_xyz(wx, wy, TRACK_Z)
}

/// Re-bake the pieces an edit changed, and the neighbours whose links moved
/// with it.
///
/// Placing one tile changes the link mask of everything within a direction step
/// of it, including the far ends of any half-step that used to run across it, so
/// the neighbourhood is re-read rather than just the edited tile.
pub fn apply_track_sprites(
    mut commands: Commands,
    mut edits: MessageReader<TrackEdit>,
    network: Res<TrackNetwork>,
    mut images: ResMut<Assets<Image>>,
    mut art: Local<TrackArt>,
    existing: Query<(Entity, &TrackSprite)>,
) {
    let _perf = crate::overlays::perf::scope("apply_track_sprites");
    let mut touched: HashSet<TrackId> = HashSet::new();
    let mut gone: HashSet<TrackId> = HashSet::new();

    for edit in edits.read() {
        match *edit {
            TrackEdit::Placed { id, tile, layer, .. } => {
                touched.insert(id);
                mark_neighbours(&network, tile, layer, &mut touched);
            }
            TrackEdit::Removed { id, tile, layer } => {
                gone.insert(id);
                mark_neighbours(&network, tile, layer, &mut touched);
            }
            TrackEdit::Failed { .. } => {}
        }
    }

    // A wholesale swap — a load, or a new map — brings its own pieces with no
    // edit messages behind them, and its ids are its own.
    let rebuild_all = network.is_added();
    if rebuild_all {
        touched.extend(network.iter().map(|p| p.id));
    }

    if touched.is_empty() && gone.is_empty() && !rebuild_all {
        return;
    }

    for (entity, sprite) in existing.iter() {
        if rebuild_all || gone.contains(&sprite.id) {
            commands.entity(entity).despawn();
            continue;
        }
        let Some(piece) = network.piece(sprite.id).filter(|_| touched.contains(&sprite.id)) else {
            continue;
        };
        if sprite.links == piece.links.0 && sprite.bridge == piece.is_bridge() {
            // Art already matches the mask; leave the entity alone.
            touched.remove(&sprite.id);
        } else {
            // Mask moved: drop the stale art and let the spawn pass redo it.
            commands.entity(entity).despawn();
        }
    }

    // What is left in `touched` is exactly what needs art: newly placed pieces,
    // neighbours whose mask moved, and everything after a wholesale swap.
    for id in touched {
        let Some(piece) = network.piece(id) else {
            continue;
        };
        let key = ArtKey {
            links: piece.links.0,
            bridge: piece.is_bridge(),
            variant: variant_for(piece.tile),
        };
        let (base, polish) = art.get(&mut images, key);
        commands
            .spawn((
                Sprite {
                    image: base,
                    ..default()
                },
                track_transform(piece.tile),
                TrackSprite {
                    id,
                    links: key.links,
                    bridge: key.bridge,
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

/// Mark every piece within one direction step of `tile` as needing a re-bake.
fn mark_neighbours(
    network: &TrackNetwork,
    tile: TileCoord,
    layer: u8,
    touched: &mut HashSet<TrackId>,
) {
    for dir in 0..DIR_COUNT {
        let (dx, dy) = DIR16[dir];
        let n = TileCoord {
            x: tile.x + dx,
            y: tile.y + dy,
        };
        if let Some(id) = network.id_at(n, layer) {
            touched.insert(id);
        }
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

    fn key(links: u16, bridge: bool) -> ArtKey {
        ArtKey {
            links,
            bridge,
            variant: 0,
        }
    }

    fn opaque(px: [u8; 4]) -> bool {
        px[3] > 0
    }

    /// §2.2, the whole reason the direction count is load-bearing: no track
    /// sprite is ever rotated.
    #[test]
    fn track_sprites_are_never_rotated() {
        for x in -3..=3 {
            for y in -3..=3 {
                let tf = track_transform(TileCoord { x, y });
                assert_eq!(tf.rotation, Quat::IDENTITY);
                assert_eq!(tf.scale, Vec3::ONE);
            }
        }
    }

    /// Direction is expressed by picking a different cell, so every one of the
    /// sixteen has to produce different pixels from every other.
    #[test]
    fn all_sixteen_directions_are_distinct_sprites() {
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

    /// The §5.3 line weights, measured off the baked art rather than asserted
    /// in a comment. An east leg is axis-aligned, so a column of it reads the
    /// cross-section directly.
    // Iso prototype: reads the §5.3 cross-section down a screen column, which is
    // only the cross-section while an east leg is axis-aligned. It no longer is.
    #[ignore = "iso prototype: pins the top-down cross-section to a screen column"]
    #[test]
    fn the_cross_section_matches_the_brief() {
        let canvas = paint_cell(key(1 << 2, false), Pass::Base);
        // Ten texels along the leg, clear of the sleeper at t = 0.
        let x = 10;
        let bed: Vec<i32> = (-20..=20).filter(|&y| opaque(canvas.at(x, y))).collect();
        assert_eq!(
            (*bed.first().unwrap(), *bed.last().unwrap()),
            (-BALLAST_HALF as i32, BALLAST_HALF as i32),
            "ballast bed is 8 either side of the centreline"
        );

        // Rails at ±4 with a bright head on the centre texel of each.
        for gauge in [-4, 4] {
            assert_eq!(
                canvas.at(x, gauge),
                rgba(RAIL_L),
                "railhead missing at gauge {gauge}"
            );
            assert_eq!(canvas.at(x, gauge - 1), rgba(RAIL_D));
            assert_eq!(canvas.at(x, gauge + 1), rgba(RAIL_M));
        }
        // Gauge really is 8 centre to centre.
        assert_eq!(RAIL_GAUGE_HALF * 2.0, 8.0);
    }

    // Iso prototype: same reason — sleeper pitch and length are measured along
    // screen axes, and the projection foreshortens both.
    #[ignore = "iso prototype: measures sleeper pitch along a screen axis"]
    #[test]
    fn sleepers_sit_at_the_briefs_spacing_and_length() {
        let canvas = paint_cell(key(1 << 2, false), Pass::Base);
        let is_tie = |x: i32, y: i32| {
            let px = canvas.at(x, y);
            px == rgba(TIE_D) || px == rgba(TIE_M) || px == rgba(TIE_L)
        };
        // Sleepers every 4 texels along the run; between them, no tie colour
        // outside the rails.
        for n in 0..3 {
            let x = n * SLEEPER_SPACING as i32;
            assert!(is_tie(x, 6), "no sleeper at x={x}");
            assert!(!is_tie(x + 2, 6), "sleeper bled between pitches at x={x}");
        }
        // Length 14 → ±7, and nothing beyond.
        assert!(is_tie(4, SLEEPER_HALF as i32));
        assert!(!is_tie(4, SLEEPER_HALF as i32 + 1));
    }

    /// The polish layer is railhead only, so fading it up cannot smear the
    /// ballast — and it stays in register with the base even at a junction,
    /// where several legs' rails cross each other.
    #[test]
    fn the_polish_layer_holds_only_railheads() {
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

    /// Iso prototype: rewritten to count colours rather than probe fixed texels
    /// — the claim ("a bridge is timber, not ballast, and still carries rail")
    /// is the same, but where a given texel lands is now the projection's
    /// business.
    #[test]
    fn a_bridge_decks_in_timber_instead_of_ballast() {
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
        let lone = paint_cell(key(0, false), Pass::Base);
        let painted = (-CENTER..CENTER)
            .flat_map(|x| (-CENTER..CENTER).map(move |y| (x, y)))
            .filter(|&(x, y)| opaque(lone.at(x, y)))
            .count();
        assert!(painted > 500, "a lone tile should still show a stub");
        // Iso prototype: the stub still carries railhead, but which texel it
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

    #[test]
    fn the_cell_honours_the_pixel_contract() {
        assert_eq!(CELL % TILE_SIZE as u32, 0);
        assert_eq!(TEXELS_PER_TILE, TILE_SIZE);
        let image = cell_image(paint_cell(key(1 << 2, false), Pass::Base));
        assert_eq!(image.width(), CELL);
        assert_eq!(image.height(), CELL);
        assert!(matches!(image.sampler, ImageSampler::Descriptor(_)));
    }
}
