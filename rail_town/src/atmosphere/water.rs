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

use bevy::prelude::*;
use rail_map::{tile_to_world, MapGrid, TILE_SIZE};
use rail_sim::TileCoord;

use crate::hash::{frame_at, hash_offset, hash_phase, world_hash};
use super::{AmbientClock, COAST_FOAM_Z, WATER_DECAL_Z};
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
/// How far from the tile centre a glint may be scattered, in texels.
const SHIMMER_SCATTER: i32 = 6;
/// Glint thickness in texels (even, so a centred sprite is texel-aligned).
const GLINT_THICKNESS: f32 = 2.0;
/// Foam lip thickness in texels.
const FOAM_THICKNESS: f32 = 2.0;
/// Distance from a tile centre to its edge lip, in texels.
const FOAM_INSET: f32 = TILE_SIZE / 2.0 - 1.0;

/// Two-frame shimmer: `(length, sideways shift, alpha)`. Lengths and shifts are
/// whole even texels — the glint moves by a texel, it does not slide.
const SHIMMER_FRAMES: [(f32, f32, f32); 2] = [(6.0, 0.0, 0.40), (4.0, 2.0, 0.28)];

/// Three-frame foam: `(length, alpha)`. The lap runs out, thins, and settles.
const FOAM_FRAMES: [(f32, f32); 3] = [(12.0, 0.50), (8.0, 0.34), (10.0, 0.22)];

/// A glint on open water.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct WaterShimmer {
    origin: Vec2,
    color: Color,
    phase: f32,
    frame: u8,
}

/// A foam lip along one land-facing edge of a water tile.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct CoastFoam {
    phase: f32,
    frame: u8,
    /// Foam runs along the edge: horizontal edges stretch in X, vertical in Y.
    horizontal: bool,
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

fn spawn_shimmer(commands: &mut Commands, tile: TileCoord, height: i8) {
    if world_hash(tile.x, tile.y, SHIMMER_PICK_SALT) % SHIMMER_ONE_IN != 0 {
        return;
    }
    let (cx, cy) = tile_to_world(tile);
    let origin = Vec2::new(
        cx + hash_offset(tile.x, tile.y, SHIMMER_X_SALT, SHIMMER_SCATTER) as f32,
        cy + hash_offset(tile.x, tile.y, SHIMMER_Y_SALT, SHIMMER_SCATTER) as f32,
    );
    let color = shimmer_color(height);
    let phase = hash_phase(tile.x, tile.y, SHIMMER_PHASE_SALT, WATER_SHIMMER_PERIOD);
    let (length, shift, alpha) = SHIMMER_FRAMES[0];

    commands.spawn((
        WaterShimmer {
            origin,
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
/// Mirrors the depth bands in `map/spawn.rs` — that module owns terrain, so the
/// bands are duplicated rather than reached into.
fn shimmer_color(height: i8) -> Color {
    match height {
        ..=-8 => WATER_M,
        -7..=-4 => WATER_L,
        _ => WATER_F,
    }
}

fn spawn_foam(commands: &mut Commands, tile: TileCoord, edge: (i32, i32)) {
    let (cx, cy) = tile_to_world(tile);
    let horizontal = edge.1 != 0;
    let (length, alpha) = FOAM_FRAMES[0];
    let size = if horizontal {
        Vec2::new(length, FOAM_THICKNESS)
    } else {
        Vec2::new(FOAM_THICKNESS, length)
    };
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
            horizontal,
        },
        Sprite::from_color(WATER_F.with_alpha(alpha), size),
        Transform::from_xyz(
            cx + edge.0 as f32 * FOAM_INSET,
            cy + edge.1 as f32 * FOAM_INSET,
            COAST_FOAM_Z,
        ),
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
        if frame == shimmer.frame {
            continue;
        }
        shimmer.frame = frame;
        let (length, shift, alpha) = SHIMMER_FRAMES[frame as usize];
        sprite.custom_size = Some(Vec2::new(length, GLINT_THICKNESS));
        sprite.color = shimmer.color.with_alpha(alpha);
        transform.translation.x = shimmer.origin.x + shift;
        transform.translation.y = shimmer.origin.y;
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
        if frame == foam.frame {
            continue;
        }
        foam.frame = frame;
        let (length, alpha) = FOAM_FRAMES[frame as usize];
        sprite.custom_size = Some(if foam.horizontal {
            Vec2::new(length, FOAM_THICKNESS)
        } else {
            Vec2::new(FOAM_THICKNESS, length)
        });
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

    fn decal_app(map: MapGrid) -> App {
        let mut app = App::new();
        app.insert_resource(map)
            .init_resource::<WaterDecals>()
            .add_systems(Startup, bake_water_decals)
            .add_systems(Update, rebuild_water_decals);
        app
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
        let limit = 12.0 * TILE_SIZE;
        for p in &after {
            assert!(
                p.x >= -TILE_SIZE && p.y >= -TILE_SIZE && p.x <= limit && p.y <= limit,
                "a decal is drawn off the map at {p:?}"
            );
        }
        // Nothing survives on the dry rows the old map had water on.
        let dry_row_y = 8.0 * TILE_SIZE;
        assert!(
            !after.iter().any(|p| p.y > dry_row_y),
            "the old map's water is still drawn where this map has none"
        );
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
        assert_eq!(GLINT_THICKNESS as i32 % 2, 0);
        assert_eq!(FOAM_THICKNESS as i32 % 2, 0);
        // The foam lip stays inside its own tile.
        assert!(FOAM_INSET + FOAM_THICKNESS / 2.0 <= TILE_SIZE / 2.0);
    }

    #[test]
    fn shimmer_steps_up_the_water_ramp() {
        assert_eq!(shimmer_color(-10), WATER_M);
        assert_eq!(shimmer_color(-5), WATER_L);
        assert_eq!(shimmer_color(-1), WATER_F);
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
