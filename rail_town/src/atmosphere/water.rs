//! Water shimmer and coast foam (brief 01 §6.3).
//!
//! Two loops, both baked once from [`MapGrid`] because terrain never changes
//! after generation, and both phased from the world hash so the sea never
//! pulses in unison — a thousand tiles blinking on the same beat is the exact
//! failure this discipline exists to prevent.
//!
//! - **Shimmer**: two frames, ~1.2 s, on open water. One tile in
//!   [`SHIMMER_ONE_IN`] carries a glint, scattered inside its tile by the hash,
//!   so the sea reads as textured rather than as gridded.
//! - **Foam**: three frames, ~2.4 s, on water tiles that touch land, drawn as a
//!   lip along the shared edge. "A coastline is a *line*" (brief §6.2).
//!
//! Both stay inside the water ramp: a glint is one step up from the band it
//! sits on, never a new colour and never the `hi` accent (brief §3.1).
//!
//! # Baked once *per world*, not once per process
//!
//! "Terrain never changes after generation" is true of a world and false of a
//! session: New Map and Load both replace [`MapGrid`] wholesale. Baking on
//! `Startup` alone left the *first* world's sea painted over every world after
//! it — glints and foam standing on dry land, and hanging past the edge of a map
//! smaller than the one they were baked from. [`rebuild_water_decals`] keys the
//! rebake on a signature of the water itself, exactly as `map/terrain/chunk.rs`
//! does: a write to `MapGrid` that does not move any water costs one hash and
//! nothing else, so the layer can sit in `Update` without ever rebuilding a
//! frame it did not have to.
//!
//! # Every offset in here is a distance on the ground, not on the screen
//!
//! A decal decorates a *patch of water*, so every position it is built from —
//! the tile centre, the scatter inside a tile, the inset out to a shared edge —
//! is a point on the ground plane, and [`rail_map::ground_to_world`] is the only
//! thing that turns one into a place on screen. Adding an offset *after* that
//! call displaces the decal along the screen axes instead of along the ground,
//! and in isometric the two are 45 degrees and a factor of two apart: a lip
//! inset fifteen texels toward its northern neighbour landed on the tile's
//! north-**east** corner, out in open water, drawn across the shore instead of
//! along it. That is what the reported "the outline does not match where the
//! water is" was.
//!
//! Three rules follow, and each is asserted rather than remembered:
//!
//! - **Position is ground-space, and one system owns the transform.** Foam wears
//!   a [`GroundAnchor`], so a spawner cannot put it in the wrong place and an
//!   I-key flip reaches it without the layer re-baking. Glints keep their own
//!   step system — they carry a screen-space shift per frame that an anchor
//!   would fight — and re-derive from their ground point every frame instead.
//! - **A lip runs along the edge in whichever view is live.** Which screen axis
//!   that is comes from projecting the edge's own ground direction
//!   ([`foam_size`]), not from a flag decided at bake time. Both families of
//!   diamond edge run at half a texel of rise per texel of run, so in isometric
//!   every lip lies flat; from above they split into the two axes as before.
//! - **Offsets stay on the diamond's lattice.** Isometric's screen `y` is the
//!   *mean* of the two ground axes, so a ground offset whose components sum to
//!   an odd number puts a sprite on a half texel, and a two-texel glint smeared
//!   across three rows is exactly what "the shimmer looks misaligned" is. The
//!   edge inset is even, and the glint scatter is rolled on the diamond's own
//!   axes so its components always sum to an even number without losing spread.

use bevy::prelude::*;
use rail_map::{MapGrid, TILE_SIZE};
use rail_sim::TileCoord;

use crate::hash::{frame_at, hash_offset, hash_phase, world_hash};
use super::{AmbientClock, COAST_FOAM_Z, WATER_DECAL_Z};
use crate::map::GroundAnchor;
use crate::palette::{WATER_F, WATER_L, WATER_M};

/// Full shimmer loop in seconds (brief §6.3: ~1.2 s, two frames).
pub(crate) const WATER_SHIMMER_PERIOD: f32 = 1.2;
/// Full foam loop in seconds (brief §6.3: ~2.4 s, three frames).
pub(crate) const COAST_FOAM_PERIOD: f32 = 2.4;

const SHIMMER_PHASE_SALT: u32 = 0x5348_494d;
const SHIMMER_PICK_SALT: u32 = 0x474c_4e54;
const SHIMMER_X_SALT: u32 = 0x4f46_5358;
const SHIMMER_Y_SALT: u32 = 0x4f46_5359;
const FOAM_PHASE_SALT: u32 = 0x464f_414d;

/// One open-water tile in this many carries a glint. Every tile glinting is a
/// texture; a scattered few are a sea.
const SHIMMER_ONE_IN: u32 = 4;
/// How far from the tile centre a glint may be scattered, in **ground** texels,
/// on either axis. Well inside the 16 a tile has to give, in either projection:
/// a ground offset `(ox, oy)` lands `|ox - oy|` across and `|ox + oy| / 2` up
/// the diamond, and both of those stay under [`SHIMMER_SCATTER`] itself.
const SHIMMER_SCATTER: i32 = 6;
/// Glint thickness in texels (even, so a centred sprite is texel-aligned).
const GLINT_THICKNESS: f32 = 2.0;
/// Foam lip thickness in texels.
const FOAM_THICKNESS: f32 = 2.0;
/// Distance from a tile centre out to its edge lip, in **ground** texels.
///
/// Even, and that is load-bearing rather than tidy: isometric's screen `y` is
/// the mean of the two ground axes, so an odd inset would stand every lip in
/// the game on a half texel. Two texels shy of the boundary at 16, so a lip
/// two thick still finishes inside the water it belongs to.
const FOAM_INSET: f32 = TILE_SIZE / 2.0 - 2.0;

/// Two-frame shimmer: `(length, sideways shift, alpha)`. Lengths and shifts are
/// whole even texels — the glint moves by a texel, it does not slide.
const SHIMMER_FRAMES: [(f32, f32, f32); 2] = [(6.0, 0.0, 0.40), (4.0, 2.0, 0.28)];

/// Three-frame foam: `(length, alpha)`. The lap runs out, thins, and settles.
const FOAM_FRAMES: [(f32, f32); 3] = [(12.0, 0.50), (8.0, 0.34), (10.0, 0.22)];

/// A glint on open water.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct WaterShimmer {
    /// Where the glint sits on the **ground plane**, in texels. Ground rather
    /// than world, so a decal belongs to a patch of water and comes back to the
    /// same patch whichever way the world is being drawn.
    ground: Vec2,
    color: Color,
    phase: f32,
    frame: u8,
}

/// A ground-plane point, projected for the view being drawn.
#[inline]
fn world_of(ground: Vec2) -> Vec2 {
    let (x, y) = rail_map::ground_to_world(ground.x, ground.y);
    Vec2::new(x, y)
}

/// A foam lip along one land-facing edge of a water tile.
///
/// Where it stands is a [`GroundAnchor`]'s business, not this component's — see
/// the module docs. What is left here is the loop and the direction the lip
/// runs in, kept on the **ground plane** so the screen axis can be re-derived
/// for whichever view is being drawn.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct CoastFoam {
    phase: f32,
    frame: u8,
    /// Ground-plane direction the shared edge runs along — the perpendicular of
    /// the edge the lip faces. A lip on a north-facing edge runs east-west.
    along: Vec2,
}

/// A lip of `length`, laid along its edge in the projection being drawn.
///
/// The long axis is whichever way the edge's own ground direction points once
/// projected, so nothing here has to know what a diamond is. From above the two
/// families of edge stay on their own axes exactly as they always did. In
/// isometric both run at half a texel of rise per texel of run — up-and-right
/// for an east-west edge, up-and-left for a north-south one — so both are
/// predominantly across the screen and both lie flat, within three texels of
/// the shoreline at the ends of a twelve-texel lip.
fn foam_size(along: Vec2, length: f32) -> Vec2 {
    let (dx, dy) = rail_map::project_offset(along.x, along.y);
    if dx.abs() >= dy.abs() {
        Vec2::new(length, FOAM_THICKNESS)
    } else {
        Vec2::new(FOAM_THICKNESS, length)
    }
}

/// Signature of the water this layer was last baked from.
///
/// Bevy change detection answers *did anybody write `MapGrid`*, which is not the
/// same question as *did the water move*. Hashing is cheap next to respawning a
/// few thousand sprites, so the change flag is the gate and this is the truth
/// (the same discipline as [`crate::map::terrain`]'s `TerrainDirty`).
#[derive(Resource, Default)]
pub(crate) struct WaterDecals {
    signature: Option<u64>,
}

/// FNV-1a over everything the decals are drawn from.
///
/// Only water and elevation: a glint's colour comes from the depth band, and
/// foam comes from which neighbours are dry. The seed, portals and terrain kind
/// change no decal, so none of them should cost a rebake.
fn water_signature(map: &MapGrid) -> u64 {
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
        eat(tile.water as u8);
        // Only depth under water matters; dry land contributes nothing but its
        // dryness, so terraforming a hill never rebakes the sea.
        eat(if tile.water { tile.height as u8 } else { 0 });
    }
    hash
}

/// Bake every water decal for the world on screen.
pub(crate) fn bake_water_decals(
    mut commands: Commands,
    map: Res<MapGrid>,
    mut decals: ResMut<WaterDecals>,
) {
    decals.signature = Some(water_signature(&map));
    for y in 0..map.height as i32 {
        for x in 0..map.width as i32 {
            let tile = TileCoord { x, y };
            if !map.tile(tile).water {
                continue;
            }
            let edges = land_edges(&map, tile);
            if edges.is_empty() {
                spawn_shimmer(&mut commands, tile, map.tile(tile).height);
            } else {
                // Coastal tiles get foam instead of a glint: two loops on one
                // tile is busier than the shoreline can carry.
                for edge in edges {
                    spawn_foam(&mut commands, tile, edge);
                }
            }
        }
    }
}

/// Re-bake the sea when the world underneath it is replaced. Idle otherwise.
///
/// Swapping [`MapGrid`] — a new map, a loaded save — is the whole trigger, and
/// the signature is what keeps that safe to leave in a per-frame schedule.
pub(crate) fn rebuild_water_decals(
    mut commands: Commands,
    map: Res<MapGrid>,
    mut decals: ResMut<WaterDecals>,
    shimmers: Query<Entity, With<WaterShimmer>>,
    foam: Query<Entity, With<CoastFoam>>,
) {
    let _perf = crate::overlays::perf::scope("rebuild_water_decals");
    if !map.is_changed() {
        return;
    }
    let signature = water_signature(&map);
    // `is_added` is the startup bake, which has already run this frame.
    if map.is_added() || decals.signature == Some(signature) {
        decals.signature = Some(signature);
        return;
    }

    for entity in shimmers.iter().chain(foam.iter()) {
        commands.entity(entity).despawn();
    }
    bake_water_decals(commands, map, decals);
}

/// Cardinal directions from `tile` that face land.
fn land_edges(map: &MapGrid, tile: TileCoord) -> Vec<(i32, i32)> {
    let mut edges = Vec::new();
    for (dx, dy) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
        let neighbour = TileCoord {
            x: tile.x + dx,
            y: tile.y + dy,
        };
        if map.get(neighbour).is_some_and(|t| !t.water) {
            edges.push((dx, dy));
        }
    }
    edges
}

/// Where inside its tile a glint sits, as a ground-plane offset in texels.
///
/// Rolled on the **diamond's** axes rather than the map's: two hashes pick how
/// far the glint moves along each diagonal, and the ground offset is their sum
/// and difference. That is the same `SHIMMER_SCATTER` box it always was — both
/// components still cover the full range — with one property added, that they
/// always sum to an even number. Isometric's screen `y` is `(gx + gy) / 2`, so
/// an odd sum is a glint on a half texel: two texels of light resampled across
/// three rows, dimmer and blurred, which is what the sea looked like.
#[inline]
fn glint_scatter(tile: TileCoord) -> (i32, i32) {
    let half = SHIMMER_SCATTER / 2;
    let a = hash_offset(tile.x, tile.y, SHIMMER_X_SALT, half);
    let b = hash_offset(tile.x, tile.y, SHIMMER_Y_SALT, half);
    (a + b, a - b)
}

fn spawn_shimmer(commands: &mut Commands, tile: TileCoord, height: i8) {
    if world_hash(tile.x, tile.y, SHIMMER_PICK_SALT) % SHIMMER_ONE_IN != 0 {
        return;
    }
    // Scattered on the **ground plane**, so a glint belongs to a patch of water
    // rather than to a patch of screen — the same rule §2.4 applies to every
    // other world-anchored decoration.
    let (gx, gy) = rail_map::tile_to_ground(tile);
    let (ox, oy) = glint_scatter(tile);
    let ground = Vec2::new(gx + ox as f32, gy + oy as f32);
    let origin = world_of(ground);
    let color = shimmer_color(height);
    let phase = hash_phase(tile.x, tile.y, SHIMMER_PHASE_SALT, WATER_SHIMMER_PERIOD);
    let (length, shift, alpha) = SHIMMER_FRAMES[0];

    commands.spawn((
        WaterShimmer {
            ground,
            color,
            phase,
            frame: 0,
        },
        Sprite::from_color(color.with_alpha(alpha), Vec2::new(length, GLINT_THICKNESS)),
        Transform::from_xyz(origin.x + shift, origin.y, WATER_DECAL_Z),
    ));
}

/// One step up the water ramp from the band this tile is drawn in.
///
/// Follows `map::terrain::material::shade_for`'s water table — inland depths
/// 1/2/3 fill `WATER_L`/`WATER_M`/`WATER_D` and the sea floors out at
/// `WATER_D` — so the glint is always the next step above its own fill, with
/// `WATER_F` staying a shallows-only accent (brief 01 §3.2).
fn shimmer_color(height: i8) -> Color {
    match height {
        ..=-3 => WATER_M,
        -2 => WATER_L,
        _ => WATER_F,
    }
}

fn spawn_foam(commands: &mut Commands, tile: TileCoord, edge: (i32, i32)) {
    // Out to the shared edge on the **ground plane**, then projected — never
    // projected and then displaced, which is a walk along the screen's axes and
    // put every lip on a corner of its tile in isometric.
    let (cx, cy) = rail_map::tile_to_ground(tile);
    let anchor = GroundAnchor::new(
        cx + edge.0 as f32 * FOAM_INSET,
        cy + edge.1 as f32 * FOAM_INSET,
    );
    // The lip runs *along* the edge, which is the perpendicular of the direction
    // it faces. Kept on the ground so a flip can re-derive the screen axis.
    let along = Vec2::new(edge.1 as f32, edge.0 as f32);
    let (length, alpha) = FOAM_FRAMES[0];
    let phase = hash_phase(
        tile.x + edge.0,
        tile.y + edge.1,
        FOAM_PHASE_SALT,
        COAST_FOAM_PERIOD,
    );

    commands.spawn((
        CoastFoam {
            phase,
            frame: 0,
            along,
        },
        Sprite::from_color(WATER_F.with_alpha(alpha), foam_size(along, length)),
        anchor,
        // The anchor owns `x` and `y` from here on; this is the same value, so
        // the lip is on its shore on the frame it appears rather than the one
        // after.
        anchor.transform(COAST_FOAM_Z),
    ));
}

/// Advance glints; touch only the sprites whose frame actually turned over.
pub(crate) fn step_water_shimmer(
    ambient: Res<AmbientClock>,
    mut shimmers: Query<(&mut WaterShimmer, &mut Sprite, &mut Transform)>,
) {
    let _perf = crate::overlays::perf::scope("step_water_shimmer");
    for (mut shimmer, mut sprite, mut transform) in shimmers.iter_mut() {
        let frame = frame_at(
            ambient.secs,
            shimmer.phase,
            WATER_SHIMMER_PERIOD,
            SHIMMER_FRAMES.len() as u32,
        ) as u8;
        let (length, shift, alpha) = SHIMMER_FRAMES[frame as usize];
        let origin = world_of(shimmer.ground);
        let (x, y) = (origin.x + shift, origin.y);
        // Same shape as the chimney plumes: the question is "is the glint where
        // it should be", not "has its frame turned over", so a projection flip
        // under a mid-frame glint is answered on the frame it happens.
        if frame == shimmer.frame && transform.translation.x == x && transform.translation.y == y {
            continue;
        }
        shimmer.frame = frame;
        sprite.custom_size = Some(Vec2::new(length, GLINT_THICKNESS));
        sprite.color = shimmer.color.with_alpha(alpha);
        transform.translation.x = x;
        transform.translation.y = y;
    }
}

/// Advance the shoreline lap.
pub(crate) fn step_coast_foam(
    ambient: Res<AmbientClock>,
    mut foam: Query<(&mut CoastFoam, &mut Sprite)>,
) {
    let _perf = crate::overlays::perf::scope("step_coast_foam");
    for (mut foam, mut sprite) in foam.iter_mut() {
        let frame = frame_at(
            ambient.secs,
            foam.phase,
            COAST_FOAM_PERIOD,
            FOAM_FRAMES.len() as u32,
        ) as u8;
        let (length, alpha) = FOAM_FRAMES[frame as usize];
        let size = foam_size(foam.along, length);
        // The same shape the glints and the chimney plumes ask: "is the lip
        // lying the way it should be", not "has its frame turned over". A lip
        // caught mid-frame by a projection flip is re-laid on the frame the
        // flip happens rather than whenever its loop next comes round. Where it
        // stands is the `GroundAnchor`'s business and is already handled.
        if frame == foam.frame && sprite.custom_size == Some(size) {
            continue;
        }
        foam.frame = frame;
        sprite.custom_size = Some(size);
        sprite.color = WATER_F.with_alpha(alpha);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_map::{generate_map, TerrainKind};

    /// A `w` x `h` map that is entirely water below `wet_rows`, land above.
    fn banded_map(w: u32, h: u32, wet_rows: i32) -> MapGrid {
        let mut map = MapGrid::empty(w, h, 1);
        for y in 0..wet_rows {
            for x in 0..w as i32 {
                if let Some(tile) = map.get_mut(TileCoord { x, y }) {
                    tile.water = true;
                    tile.kind = TerrainKind::Water;
                    tile.height = -3;
                }
            }
        }
        map
    }

    /// A generated map with a lake cut into it, so the coastline is a real
    /// wiggly one rather than a straight band.
    fn lakey_map(w: u32, h: u32, seed: u64) -> MapGrid {
        let mut map = generate_map(w, h, seed);
        let (cx, cy) = (w as i32 / 2, h as i32 / 2);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let (dx, dy) = ((x - cx) as f32, (y - cy) as f32);
                let wobble = ((x * 7 + y * 3) % 5) as f32;
                if dx * dx + dy * dy < (10.0 + wobble) * (10.0 + wobble) {
                    if let Some(tile) = map.get_mut(TileCoord { x, y }) {
                        tile.water = true;
                        tile.kind = TerrainKind::Water;
                        tile.height = -3;
                    }
                }
            }
        }
        map
    }

    fn decal_app(map: MapGrid) -> App {
        let mut app = App::new();
        app.insert_resource(map)
            .init_resource::<WaterDecals>()
            .init_resource::<AmbientClock>()
            .add_systems(Startup, bake_water_decals)
            // `anchor_world_sprites` is the real system out of `map::projection`
            // — the whole point of the anchor is that this layer does not own a
            // second copy of "put the sprite where its ground is".
            .add_systems(
                Update,
                (
                    rebuild_water_decals,
                    crate::map::projection::anchor_world_sprites,
                    step_water_shimmer,
                    step_coast_foam,
                ),
            );
        app
    }

    /// Install `map` as the world the projection's lift is read from, the way
    /// `map::projection::follow_map_heights` does for the running game.
    fn install(map: &MapGrid) {
        rail_map::set_iso_heights(map);
    }

    /// Every water decal, as `(ground point, drawn position)`.
    ///
    /// The ground point is the decal's own — an anchor's for foam, the carried
    /// scatter for a glint — which is what makes the assertions below about the
    /// *water* rather than about a screen coordinate.
    fn decals(app: &mut App) -> Vec<(&'static str, Vec2, Vec2)> {
        let mut out = Vec::new();
        {
            let mut q = app.world_mut().query::<(&WaterShimmer, &Transform)>();
            for (glint, tf) in q.iter(app.world()) {
                out.push(("glint", glint.ground, tf.translation.truncate()));
            }
        }
        {
            let mut q = app.world_mut().query::<(&CoastFoam, &GroundAnchor, &Transform)>();
            for (_, anchor, tf) in q.iter(app.world()) {
                out.push(("foam", anchor.0, tf.translation.truncate()));
            }
        }
        out
    }

    fn decal_positions(app: &mut App) -> Vec<Vec2> {
        let mut q = app
            .world_mut()
            .query_filtered::<&Transform, Or<(With<WaterShimmer>, With<CoastFoam>)>>();
        q.iter(app.world())
            .map(|t| t.translation.truncate())
            .collect()
    }

    #[test]
    fn a_new_map_repaints_the_sea_instead_of_inheriting_it() {
        // The reported bug: after New Map, water shimmered off the edge of the
        // map and over ground where no water existed. The decals were baked on
        // `Startup` and never again, so the first world's sea stayed painted
        // over every world after it.
        let mut app = decal_app(banded_map(32, 32, 16));
        app.update();
        let before = decal_positions(&mut app);
        assert!(!before.is_empty(), "the first world has a sea to draw");

        // A smaller, drier world takes its place.
        app.world_mut().insert_resource(banded_map(12, 12, 3));
        app.update();

        let after = decal_positions(&mut app);
        assert!(!after.is_empty(), "the new world has a sea of its own");
        // In isometric the map's screen extent is a diamond, not a square, so
        // the bound is the projected one. The claim is unchanged — no decal may
        // land outside the world it belongs to.
        let edge = 12.0 * TILE_SIZE;
        let (west, _) = rail_map::project(0.0, edge);
        let (east, _) = rail_map::project(edge, 0.0);
        let (_, top) = rail_map::project(edge, edge);
        for p in &after {
            let inside = p.x >= west - TILE_SIZE
                && p.x <= east + TILE_SIZE
                && p.y >= -TILE_SIZE
                && p.y <= top + TILE_SIZE;
            assert!(inside, "a decal is drawn off the map at {p:?}");
        }
        // Nothing survives on the dry rows the old map had water on. Resolved
        // back through the projection rather than read off the screen row,
        // which no longer corresponds to a map row.
        for p in &after {
            let tile = rail_map::world_to_tile(p.x, p.y);
            let msg = "the old map's water is still drawn where this map has none";
            assert!(tile.y <= 3, "{msg}: {tile:?}");
        }
    }

    #[test]
    fn an_unchanged_world_never_rebakes() {
        // A write to `MapGrid` for a reason that has nothing to do with water
        // must cost one hash, not a few thousand respawned sprites. This is the
        // FPS-regression shape, so it is asserted rather than commented.
        let mut app = decal_app(banded_map(24, 24, 8));
        app.update();
        let before = decal_positions(&mut app);

        // Touch the map without moving any water.
        app.world_mut().resource_mut::<MapGrid>().seed = 99;
        app.update();

        assert_eq!(
            decal_positions(&mut app),
            before,
            "a non-water write must not rebake the sea"
        );
    }

    // ── Every decal is standing on the water it decorates ──────────────────

    /// The tile a decal's own ground point belongs to, and the tile a player
    /// looking at the screen would say it was drawn on. They are the same tile
    /// except where a cliff genuinely stands in the way.
    fn resolve(ground: Vec2, drawn: Vec2) -> (TileCoord, TileCoord) {
        (
            rail_map::top_down_world_to_tile(ground.x, ground.y),
            rail_map::world_to_tile(drawn.x, drawn.y),
        )
    }

    /// Assert every water decal is on water, in whichever projection is live.
    ///
    /// Three claims, and the shipped foam failed all three in isometric:
    ///
    /// 1. Its ground point is inside a **water** tile. A decal decorates a patch
    ///    of water, so this is what "belongs to" means.
    /// 2. It is *drawn* at that ground point projected, to the texel. Anything
    ///    that adds an offset after projecting fails here, whichever direction
    ///    it walks off in.
    /// 3. The tile under it on screen is water too — the observable symptom, and
    ///    the one a player reports. The single exception isometric genuinely has
    ///    is stated as a rule rather than tolerated: a tile nearer the camera
    ///    and standing higher really does hide the water behind it, which is the
    ///    same exception `map::projection`'s picking sweep documents.
    fn assert_every_decal_stands_on_water(app: &mut App, map: &MapGrid) -> usize {
        let mut checked = 0;
        let mut occluded = 0;
        for (kind, ground, drawn) in decals(app) {
            let (own, seen) = resolve(ground, drawn);
            let view = rail_map::projection().label();
            assert!(
                map.get(own).is_some_and(|t| t.water),
                "a {kind} is anchored to {own:?}, which is not water"
            );
            let (wx, wy) = rail_map::ground_to_world(ground.x, ground.y);
            // Glints carry a whole-texel screen shift per frame; the claim is
            // that the decal has not wandered off its own ground, not that it
            // is nailed to the exact centre of it.
            let slack = SHIMMER_FRAMES.iter().map(|f| f.1).fold(0.0, f32::max);
            assert!(
                (drawn.x - wx).abs() <= slack && (drawn.y - wy).abs() <= slack,
                "a {kind} on the ground at {ground:?} is drawn at {drawn:?}, but \
                 that ground is at ({wx}, {wy}) in {view}"
            );
            if !map.get(seen).is_some_and(|t| t.water) {
                // Not water underneath: the only honest reason is a cliff in
                // front of it.
                assert!(
                    seen.x + seen.y < own.x + own.y
                        && rail_map::tile_height(seen) > rail_map::tile_height(own),
                    "a {kind} belonging to the water at {own:?} is drawn on dry \
                     {seen:?} in {view}, and {seen:?} is not standing in front of \
                     it — it is a glint on the grass"
                );
                occluded += 1;
            }
            checked += 1;
        }
        assert!(checked > 100, "the sweep saw almost nothing: {checked}");
        assert!(
            occluded * 10 < checked,
            "{occluded} of {checked} water decals are hidden behind cliffs, which \
             is too many to be cliffs"
        );
        checked
    }

    /// A glint on the grass is the observable symptom of every way a decal can
    /// disagree with its water, in either projection.
    ///
    /// # The bug
    ///
    /// `spawn_foam` took the tile's *projected* centre and then walked
    /// `FOAM_INSET` along the screen's own axes to reach the shared edge. From
    /// above those are the same walk. In isometric a step of fifteen texels up
    /// the screen is a step of fifteen texels along **both** ground axes, so a
    /// lip meant for the northern edge landed on the north-east corner — a
    /// diagonal, out in open water, drawn across the shore rather than along it.
    /// Forty-two of four hundred and fifteen lips on a lake map came out on dry
    /// land; the rest were merely in the wrong part of the right tile, which is
    /// what the owner saw as an outline that did not match the water.
    #[test]
    fn every_water_decal_stands_on_the_water_it_decorates() {
        for mode in [rail_map::Projection::TopDown, rail_map::Projection::Iso] {
            let _guard = crate::map::tests::ProjectionGuard::new(mode);
            let map = lakey_map(48, 48, 4_242);
            install(&map);
            let mut app = decal_app(map.clone());
            app.update();
            assert_every_decal_stands_on_water(&mut app, &map);
        }
    }

    /// The other half: the view is not part of the world, so pressing I has to
    /// leave every decal on the same patch of water.
    ///
    /// The shipped foam had no ground position at all and no system that wrote
    /// its transform, and a flip does not touch `MapGrid` so the layer never
    /// re-baked. Every lip therefore kept the coordinates of the view it was
    /// born in: after one flip out of isometric, 337 of 415 were on dry land and
    /// 208 stood at a negative `x` the top-down world does not even have.
    #[test]
    fn the_water_layer_follows_a_projection_flip() {
        let _guard = crate::map::tests::ProjectionGuard::new(rail_map::Projection::Iso);
        let map = lakey_map(48, 48, 4_242);
        install(&map);
        let mut app = decal_app(map.clone());
        app.update();

        let iso: Vec<Vec2> = decals(&mut app).iter().map(|d| d.2).collect();
        assert_every_decal_stands_on_water(&mut app, &map);

        // Into top-down, exactly as the I key does it: move the flag, and do not
        // touch the world.
        rail_map::set_projection(rail_map::Projection::TopDown);
        app.update();
        let flat: Vec<Vec2> = decals(&mut app).iter().map(|d| d.2).collect();
        assert_eq!(iso.len(), flat.len(), "the flip lost or duplicated decals");
        let moved = iso.iter().zip(&flat).filter(|(a, b)| a != b).count();
        assert!(
            moved * 2 > iso.len(),
            "only {moved} of {} decals moved; the flip is not reaching them",
            iso.len()
        );
        assert_every_decal_stands_on_water(&mut app, &map);

        // ... and back is exactly where it started, to the texel.
        rail_map::set_projection(rail_map::Projection::Iso);
        app.update();
        let back: Vec<Vec2> = decals(&mut app).iter().map(|d| d.2).collect();
        assert_eq!(back, iso, "a flip and a flip back moved the sea");
    }

    /// A lip lies *along* its shore in either view, and never across it.
    ///
    /// From above the two families of edge are the two screen axes. In isometric
    /// both run at half a texel of rise per texel of run, so both lie flat — the
    /// shipped code drew east- and west-facing lips as tall thin bars, standing
    /// perpendicular to the coast they were drawing.
    #[test]
    fn a_foam_lip_runs_along_its_edge_in_either_view() {
        let north = Vec2::new(1.0, 0.0); // a north- or south-facing edge
        let east = Vec2::new(0.0, 1.0); // an east- or west-facing edge
        {
            let _g = crate::map::tests::ProjectionGuard::new(rail_map::Projection::TopDown);
            assert_eq!(foam_size(north, 12.0), Vec2::new(12.0, FOAM_THICKNESS));
            assert_eq!(foam_size(east, 12.0), Vec2::new(FOAM_THICKNESS, 12.0));
        }
        {
            let _g = crate::map::tests::ProjectionGuard::new(rail_map::Projection::Iso);
            for along in [north, east] {
                assert_eq!(
                    foam_size(along, 12.0),
                    Vec2::new(12.0, FOAM_THICKNESS),
                    "a diamond edge runs across the screen, not up it"
                );
                // The claim underneath: whichever way the edge faces, its
                // projected direction really is twice as wide as it is tall.
                let (dx, dy) = rail_map::project_offset(along.x, along.y);
                assert!((dx.abs() - 2.0 * dy.abs()).abs() < 1e-6);
            }
        }
    }

    /// Nothing lands on a half texel, in either view.
    ///
    /// Isometric's screen `y` is the *mean* of the two ground axes, so a ground
    /// offset with an odd sum halves. A two-texel glint centred on a half texel
    /// is resampled across three rows: dimmer, blurred, and crawling as the
    /// camera moves, which is the other half of "the shimmer looks misaligned".
    #[test]
    fn every_decal_lands_on_a_whole_texel_in_either_view() {
        for mode in [rail_map::Projection::TopDown, rail_map::Projection::Iso] {
            let _guard = crate::map::tests::ProjectionGuard::new(mode);
            let map = lakey_map(48, 48, 4_242);
            install(&map);
            let mut app = decal_app(map.clone());
            app.update();
            let mut checked = 0;
            for (kind, ground, drawn) in decals(&mut app) {
                assert_eq!(
                    (drawn.x.fract(), drawn.y.fract()),
                    (0.0, 0.0),
                    "a {kind} from the ground at {ground:?} is drawn at {drawn:?} \
                     in {}, off the texel grid",
                    mode.label()
                );
                checked += 1;
            }
            assert!(checked > 100);
        }
    }

    #[test]
    fn a_glint_scatters_on_the_diamond_lattice() {
        // Both components still cover the full box the constant names, and their
        // sum is always even — which is what keeps the projected point whole.
        let (mut lo, mut hi) = (0, 0);
        for y in 0..64 {
            for x in 0..64 {
                let (ox, oy) = glint_scatter(TileCoord { x, y });
                assert_eq!((ox + oy) % 2, 0, "({ox}, {oy}) puts a glint on a half texel");
                assert!(ox.abs() <= SHIMMER_SCATTER && oy.abs() <= SHIMMER_SCATTER);
                lo = lo.min(ox).min(oy);
                hi = hi.max(ox).max(oy);
            }
        }
        assert_eq!((lo, hi), (-SHIMMER_SCATTER, SHIMMER_SCATTER));
    }

    #[test]
    fn loops_match_the_brief() {
        assert!((WATER_SHIMMER_PERIOD - 1.2).abs() < f32::EPSILON);
        assert!((COAST_FOAM_PERIOD - 2.4).abs() < f32::EPSILON);
        assert_eq!(SHIMMER_FRAMES.len(), 2);
        assert_eq!(FOAM_FRAMES.len(), 3);
    }

    #[test]
    fn decal_geometry_is_texel_aligned() {
        for (length, shift, _) in SHIMMER_FRAMES {
            assert_eq!(length.fract(), 0.0);
            assert_eq!(length as i32 % 2, 0, "even length keeps a centred sprite aligned");
            assert_eq!(shift.fract(), 0.0);
        }
        for (length, _) in FOAM_FRAMES {
            assert_eq!(length as i32 % 2, 0);
        }
        assert_eq!(FOAM_INSET.fract(), 0.0);
        // Even, not merely whole: isometric's screen y is the mean of the two
        // ground axes, so an odd inset stands every lip on a half texel.
        assert_eq!(FOAM_INSET as i32 % 2, 0, "an odd inset halves in isometric");
        assert_eq!(GLINT_THICKNESS as i32 % 2, 0);
        assert_eq!(FOAM_THICKNESS as i32 % 2, 0);
        // The foam lip stays inside its own tile.
        assert!(FOAM_INSET + FOAM_THICKNESS / 2.0 <= TILE_SIZE / 2.0);
    }

    #[test]
    fn shimmer_steps_up_the_water_ramp() {
        use crate::map::terrain_color;
        use rail_map::TerrainKind;
        // The glint is exactly one ramp step above the fill it sits on, at
        // every depth the terrain table can produce.
        for (height, glint) in [(-10, WATER_M), (-3, WATER_M), (-2, WATER_L), (-1, WATER_F)] {
            assert_eq!(shimmer_color(height), glint);
            assert_ne!(
                terrain_color(TerrainKind::Water, height),
                shimmer_color(height),
                "a glint the colour of its own fill is invisible at {height}"
            );
        }
    }

    #[test]
    fn coastal_tiles_are_found_on_a_real_map() {
        let map = generate_map(64, 64, 42);
        let mut coastal = 0;
        let mut open = 0;
        for y in 0..64 {
            for x in 0..64 {
                let tile = TileCoord { x, y };
                if !map.tile(tile).water {
                    continue;
                }
                if land_edges(&map, tile).is_empty() {
                    open += 1;
                } else {
                    coastal += 1;
                }
            }
        }
        assert!(coastal > 0, "a map with a coast must produce foam");
        assert!(open > 0, "a map with a sea must produce glints");
    }

    #[test]
    fn glints_are_scattered_not_gridded() {
        // Both the pick and the offset are world-hashed, so neighbouring
        // glints must not line up in rows.
        let mut offsets = std::collections::HashSet::new();
        let mut picked = 0;
        for y in 0..48 {
            for x in 0..48 {
                if world_hash(x, y, SHIMMER_PICK_SALT) % SHIMMER_ONE_IN != 0 {
                    continue;
                }
                picked += 1;
                offsets.insert((
                    hash_offset(x, y, SHIMMER_X_SALT, SHIMMER_SCATTER),
                    hash_offset(x, y, SHIMMER_Y_SALT, SHIMMER_SCATTER),
                ));
            }
        }
        let total = 48 * 48;
        let expected = total / SHIMMER_ONE_IN as i32;
        assert!(
            (picked - expected).abs() < expected / 3,
            "glint density drifted: {picked} of {total}"
        );
        assert!(offsets.len() > 40, "glints cluster on too few offsets");
    }

    #[test]
    fn phases_are_world_anchored_and_stable() {
        let a = hash_phase(12, 34, SHIMMER_PHASE_SALT, WATER_SHIMMER_PERIOD);
        let b = hash_phase(12, 34, SHIMMER_PHASE_SALT, WATER_SHIMMER_PERIOD);
        assert_eq!(a, b);
        assert!(a >= 0.0 && a < WATER_SHIMMER_PERIOD);

        // At any instant the sea is split across both frames, never in unison.
        let mut on_frame = [0; 2];
        for y in 0..40 {
            for x in 0..40 {
                let phase = hash_phase(x, y, SHIMMER_PHASE_SALT, WATER_SHIMMER_PERIOD);
                let frame = frame_at(7.3, phase, WATER_SHIMMER_PERIOD, 2) as usize;
                on_frame[frame] += 1;
            }
        }
        assert!(on_frame[0] > 400 && on_frame[1] > 400, "sea pulses as one: {on_frame:?}");
    }
}
