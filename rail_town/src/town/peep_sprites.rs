//! Walking peeps — drawn at their actual position along their journey.
//!
//! Brief 06 §4.1: *"A peep walking down a lane to catch the morning train is
//! the game's whole thesis rendered at two pixels tall."* So the sprite reads
//! [`PeepPosition`] straight from the sim rather than parking everyone on a
//! tile centre, and it draws four directions × a two-frame walk cycle from the
//! art manifest (01 §7).
//!
//! The lane is a real one: the sim walks a peep tile to tile along a cached
//! route over walkable ground (`rail_sim::peeps::WalkRoute`), so the position
//! this module rounds to whole texels never sits on water, and the facing it
//! draws is the direction the peep is actually travelling along that route.
//!
//! # Pixel contract (art 01 §2)
//!
//! - Positions round to **whole world texels** after the sim's fractional
//!   integration — the sim owns sub-tile motion, the renderer owns the grid.
//! - **No runtime rotation.** Direction picks a different arrangement of parts;
//!   nothing is ever transformed into place.
//! - Placeholder parts use the **real palette and real dimensions**, so final
//!   art is a texture swap and never a layout change.
//! - Mood tint stays inside the world palette. `HI` / `WARN` / `OK` are
//!   diagnostic-only and never appear on a peep.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{world_to_tile, MapGrid, TILE_SIZE};
// The journey types live behind `rail_sim::peeps` until they are re-exported
// from the crate root — see the wiring note in the peeps module docs.
use rail_sim::peeps::{Facing, Journey, PeepDetail, PeepFocus, PeepPosition};
use rail_sim::{Mood, Peep, PeepId};

use crate::map::MapCamera;
use crate::palette::{
    BALLAST_D, BALLAST_M, GRASS_M, HILL_M, PLASTER_L, PLASTER_M, ROOF_SLATE_D, ROOF_SLATE_M,
    ROOF_TILE_D, ROOF_TILE_M, SAND_L, SAND_M, TIE_D, WOOD_D, WOOD_M,
};

/// One world texel at 1:1 — the unit every peep dimension is quoted in.
const TEXEL: f32 = TILE_SIZE / 32.0;

/// Peeps draw between buildings (`0.5`) and trains (`3.0`), row-sorted within
/// the band so a peep south of a building draws in front of it (art 01 §6.1).
const PEEP_Z_BASE: f32 = 2.0;
const PEEP_Z_SPAN: f32 = 0.8;

/// Transparent pick box on the parent entity. The visible peep is only a few
/// texels across, which is far too small a click target; selection reads the
/// parent's `custom_size`, so the box is what the player actually clicks.
const PICK_BOX: f32 = TILE_SIZE * 0.42;

#[derive(Component, Debug, Clone, Copy)]
pub struct PeepSprite {
    pub id: PeepId,
}

#[derive(Component)]
pub(crate) struct PeepTorso;

#[derive(Component)]
pub(crate) struct PeepHead;

#[derive(Component)]
pub(crate) struct PeepLegs;

/// Publish the camera's region of interest for the sim's bounded simulation.
///
/// The sim never reads a camera (brief 06 §4.1) — this is the only direction
/// that information travels.
pub fn sync_peep_focus(
    windows: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Transform, &Projection), With<MapCamera>>,
    mut focus: ResMut<PeepFocus>,
) {
    let Ok((transform, projection)) = camera.single() else {
        focus.clear();
        return;
    };
    let Projection::Orthographic(ortho) = projection else {
        return;
    };
    let (width, height) = windows
        .single()
        .map(|w| (w.width(), w.height()))
        .unwrap_or((1280.0, 720.0));

    let half_w = width * ortho.scale * 0.5 / TILE_SIZE;
    let half_h = height * ortho.scale * 0.5 / TILE_SIZE;
    let center = world_to_tile(transform.translation.x, transform.translation.y);
    // One tile of margin so peeps are promoted just before they walk on screen.
    focus.look_at(center, half_w.max(half_h).ceil() as i32 + 1);
}

/// Spawn / update / retire peep sprites from sim journey state.
pub fn sync_peep_sprites(
    mut commands: Commands,
    map: Res<MapGrid>,
    peeps: Query<(&Peep, &PeepPosition, &Journey, &PeepDetail)>,
    mut sprites: Query<(
        Entity,
        &PeepSprite,
        &mut Transform,
        &mut Visibility,
        &Children,
    )>,
    mut parts: Query<
        (
            &mut Sprite,
            &mut Transform,
            Option<&PeepTorso>,
            Option<&PeepHead>,
            Option<&PeepLegs>,
        ),
        Without<PeepSprite>,
    >,
) {
    let mut live: std::collections::HashMap<PeepId, Pose> =
        std::collections::HashMap::with_capacity(peeps.iter().len());
    for (peep, pos, journey, detail) in &peeps {
        // Only the bounded full-detail set gets sprites; the rest are district flow.
        if !detail.is_full() {
            continue;
        }
        live.insert(peep.id, pose_for(peep, pos, journey, map.height));
    }

    for (entity, marker, mut transform, mut visibility, children) in sprites.iter_mut() {
        let Some(pose) = live.remove(&marker.id) else {
            commands.entity(entity).despawn();
            continue;
        };
        transform.translation = pose.translation;
        *visibility = if pose.visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        for child in children.iter() {
            let Ok((mut sprite, mut child_tf, torso, head, legs)) = parts.get_mut(child) else {
                continue;
            };
            let part = if torso.is_some() {
                pose.torso
            } else if head.is_some() {
                pose.head
            } else if legs.is_some() {
                pose.legs
            } else {
                continue;
            };
            apply_part(&mut sprite, &mut child_tf, part);
        }
    }

    for (id, pose) in live {
        commands
            .spawn((
                PeepSprite { id },
                Sprite::from_color(Color::srgba(0.0, 0.0, 0.0, 0.0), Vec2::splat(PICK_BOX)),
                Transform::from_translation(pose.translation),
                if pose.visible {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                },
            ))
            .with_children(|parent| {
                spawn_part(parent, pose.legs, PeepLegs);
                spawn_part(parent, pose.torso, PeepTorso);
                spawn_part(parent, pose.head, PeepHead);
            });
    }
}

fn spawn_part(parent: &mut ChildSpawnerCommands, part: Part, marker: impl Component) {
    let mut sprite = Sprite::from_color(part.color, part.size);
    sprite.custom_size = Some(part.size);
    parent.spawn((
        marker,
        sprite,
        Transform::from_xyz(part.offset.x, part.offset.y, part.z),
    ));
}

fn apply_part(sprite: &mut Sprite, transform: &mut Transform, part: Part) {
    sprite.color = part.color;
    sprite.custom_size = Some(part.size);
    transform.translation.x = part.offset.x;
    transform.translation.y = part.offset.y;
    transform.translation.z = part.z;
}

#[derive(Clone, Copy)]
struct Part {
    size: Vec2,
    color: Color,
    offset: Vec2,
    z: f32,
}

#[derive(Clone, Copy)]
struct Pose {
    translation: Vec3,
    visible: bool,
    torso: Part,
    head: Part,
    legs: Part,
}

/// Build the whole peep from its sim state. Pure, so it can be reasoned about
/// (and tested) without a `World`.
fn pose_for(peep: &Peep, pos: &PeepPosition, journey: &Journey, map_height: u32) -> Pose {
    // Sub-tile motion is the sim's; whole texels are the renderer's.
    let wx = ((pos.x + 0.5) * TILE_SIZE).round();
    let wy = ((pos.y + 0.5) * TILE_SIZE).round();
    let rows = map_height.max(1) as f32;
    let depth = ((rows - pos.y).clamp(0.0, rows) / rows) * PEEP_Z_SPAN;

    let width = f32::from(peep.body.width_texels()) * TEXEL;
    let full_height = f32::from(peep.body.height_texels()) * TEXEL;
    // Posture: a frustrated peep stands slumped, a texel shorter (§4.3).
    let slump = if peep.mood == Mood::Frustrated {
        TEXEL
    } else {
        0.0
    };
    let head_h = 2.0 * TEXEL;
    let legs_h = 2.0 * TEXEL;
    let torso_h = (full_height - head_h - legs_h - slump).max(TEXEL);

    // Two-frame walk: the free leg swings, and the body bobs a single texel.
    let walking = pos.walking && journey.stage.is_walking();
    let frame = if walking { pos.step & 1 } else { 0 };
    let swing = if walking {
        if frame == 0 {
            -TEXEL
        } else {
            TEXEL
        }
    } else {
        0.0
    };
    let bob = if walking && frame == 1 { TEXEL } else { 0.0 };

    let clothing = mood_tint(clothing_colour(peep.portrait), peep.mood);
    let hair = hair_colour(peep.portrait);

    let legs_y = -(torso_h * 0.5) - legs_h * 0.5;
    let head_y = torso_h * 0.5 + head_h * 0.5;

    // Direction is a different arrangement of parts, never a rotation.
    let (head_dx, head_colour, legs_dx) = match pos.facing {
        Facing::North => (0.0, hair, swing),
        Facing::South => (0.0, skin_colour(peep.portrait), swing),
        Facing::East => (TEXEL, skin_colour(peep.portrait), swing.abs()),
        Facing::West => (-TEXEL, skin_colour(peep.portrait), -swing.abs()),
    };

    Pose {
        translation: Vec3::new(wx, wy + bob, PEEP_Z_BASE + depth),
        visible: journey.is_visible(),
        torso: Part {
            size: Vec2::new(width, torso_h),
            color: clothing,
            offset: Vec2::new(0.0, 0.0),
            z: 0.02,
        },
        head: Part {
            size: Vec2::new(head_h, head_h),
            color: head_colour,
            offset: Vec2::new(head_dx, head_y),
            z: 0.03,
        },
        legs: Part {
            size: Vec2::new(TEXEL, legs_h),
            color: trousers_colour(peep.portrait),
            offset: Vec2::new(legs_dx, legs_y),
            z: 0.01,
        },
    }
}

/// Clothing ramp — world palette only, one entry per portrait variant.
fn clothing_colour(variant: u8) -> Color {
    const RAMP: [Color; 8] = [
        PLASTER_M,
        WOOD_M,
        ROOF_SLATE_M,
        HILL_M,
        SAND_M,
        BALLAST_M,
        ROOF_TILE_M,
        GRASS_M,
    ];
    RAMP[(variant as usize) % RAMP.len()]
}

fn hair_colour(variant: u8) -> Color {
    const RAMP: [Color; 4] = [WOOD_D, TIE_D, BALLAST_D, ROOF_SLATE_D];
    RAMP[(variant as usize / 2) % RAMP.len()]
}

fn skin_colour(variant: u8) -> Color {
    const RAMP: [Color; 3] = [PLASTER_L, SAND_L, PLASTER_M];
    RAMP[(variant as usize) % RAMP.len()]
}

fn trousers_colour(variant: u8) -> Color {
    const RAMP: [Color; 3] = [WOOD_D, ROOF_SLATE_D, BALLAST_D];
    RAMP[(variant as usize) % RAMP.len()]
}

/// A small mood tint, readable at a glance on a crowded platform (§4.3).
///
/// Stays inside the world palette: content warms toward plaster, uneasy drains
/// toward ballast grey, frustrated pulls toward the dull roof-tile red.
fn mood_tint(base: Color, mood: Mood) -> Color {
    match mood {
        Mood::Content => mix(base, PLASTER_L, 0.22),
        Mood::Uneasy => mix(base, BALLAST_M, 0.28),
        Mood::Frustrated => mix(base, ROOF_TILE_D, 0.42),
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let a = a.to_srgba();
    let b = b.to_srgba();
    let t = t.clamp(0.0, 1.0);
    Color::srgb(
        a.red + (b.red - a.red) * t,
        a.green + (b.green - a.green) * t,
        a.blue + (b.blue - a.blue) * t,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::peeps::{BodyType, HouseholdId, JourneyStage, Routine};
    use rail_sim::{StationId, TileCoord};

    fn peep(mood: Mood, portrait: u8) -> Peep {
        let mut p = Peep::new(
            PeepId(1),
            "Mara Aldertone",
            TileCoord { x: 3, y: 3 },
            HouseholdId(1),
            0,
        );
        p.mood = mood;
        p.portrait = portrait;
        p.body = BodyType::Slight;
        p
    }

    fn journey(stage: JourneyStage) -> Journey {
        let routine = Routine::from_seed(
            1,
            TileCoord { x: 3, y: 3 },
            StationId(1),
            TileCoord { x: 9, y: 9 },
            StationId(2),
        );
        let mut j = Journey::new(&routine);
        j.set_stage(stage);
        j
    }

    #[test]
    fn positions_land_on_whole_texels() {
        let mut pos = PeepPosition::at_tile(TileCoord { x: 4, y: 6 }, 3);
        pos.x += 0.37;
        pos.y -= 0.11;
        let pose = pose_for(
            &peep(Mood::Content, 0),
            &pos,
            &journey(JourneyStage::WalkingToStation),
            64,
        );
        assert_eq!(pose.translation.x, pose.translation.x.round());
        assert_eq!(pose.translation.y, pose.translation.y.round());
    }

    #[test]
    fn nothing_ever_rotates() {
        // Direction is expressed by part offsets; a rotated sprite would resample.
        let mut poses = Vec::new();
        for facing in Facing::ALL {
            let mut pos = PeepPosition::at_tile(TileCoord { x: 2, y: 2 }, 1);
            pos.facing = facing;
            pos.walking = true;
            poses.push(pose_for(
                &peep(Mood::Content, 0),
                &pos,
                &journey(JourneyStage::WalkingToStation),
                64,
            ));
        }
        let east = poses[Facing::East.index()];
        let west = poses[Facing::West.index()];
        assert!(
            east.head.offset.x > 0.0 && west.head.offset.x < 0.0,
            "east / west must differ by arrangement, not rotation"
        );
        let north = poses[Facing::North.index()];
        let south = poses[Facing::South.index()];
        assert_ne!(
            north.head.color.to_srgba(),
            south.head.color.to_srgba(),
            "back of the head should not read as a face"
        );
    }

    #[test]
    fn the_walk_cycle_has_exactly_two_frames() {
        let mut a = PeepPosition::at_tile(TileCoord { x: 2, y: 2 }, 1);
        a.walking = true;
        a.step = 0;
        let mut b = a;
        b.step = 1;

        let pose_a = pose_for(
            &peep(Mood::Content, 0),
            &a,
            &journey(JourneyStage::WalkingToStation),
            64,
        );
        let pose_b = pose_for(
            &peep(Mood::Content, 0),
            &b,
            &journey(JourneyStage::WalkingToStation),
            64,
        );
        assert_ne!(pose_a.legs.offset.x, pose_b.legs.offset.x);
        assert_ne!(pose_a.translation.y, pose_b.translation.y, "no bob");
    }

    #[test]
    fn a_standing_peep_does_not_animate() {
        let mut pos = PeepPosition::at_tile(TileCoord { x: 2, y: 2 }, 1);
        pos.walking = false;
        pos.step = 1;
        let pose = pose_for(
            &peep(Mood::Content, 0),
            &pos,
            &journey(JourneyStage::WaitingOnPlatform),
            64,
        );
        assert_eq!(pose.legs.offset.x, 0.0);
    }

    #[test]
    fn riding_peeps_are_hidden_inside_the_train() {
        let pos = PeepPosition::at_tile(TileCoord { x: 2, y: 2 }, 1);
        let pose = pose_for(
            &peep(Mood::Content, 0),
            &pos,
            &journey(JourneyStage::Riding),
            64,
        );
        assert!(!pose.visible);
    }

    #[test]
    fn mood_tint_stays_in_the_world_palette() {
        use crate::palette::{HI, OK, WARN};
        let forbidden = [HI.to_srgba(), WARN.to_srgba(), OK.to_srgba()];
        for mood in [Mood::Content, Mood::Uneasy, Mood::Frustrated] {
            for variant in 0..8u8 {
                let c = mood_tint(clothing_colour(variant), mood).to_srgba();
                assert!(
                    !forbidden.contains(&c),
                    "{mood:?}/{variant} used a diagnostic accent in world art"
                );
            }
        }
        // …and the three moods still read differently from one another.
        let base = clothing_colour(0);
        let content = mood_tint(base, Mood::Content).to_srgba();
        let frustrated = mood_tint(base, Mood::Frustrated).to_srgba();
        assert_ne!(content, frustrated);
    }

    #[test]
    fn frustration_shows_as_posture_not_only_colour() {
        let pos = PeepPosition::at_tile(TileCoord { x: 2, y: 2 }, 1);
        let stage = journey(JourneyStage::WaitingOnPlatform);
        let calm = pose_for(&peep(Mood::Content, 0), &pos, &stage, 64);
        let cross = pose_for(&peep(Mood::Frustrated, 0), &pos, &stage, 64);
        assert!(
            cross.torso.size.y < calm.torso.size.y,
            "a frustrated peep should stand slumped"
        );
    }

    #[test]
    fn southern_peeps_draw_in_front_of_northern_ones() {
        let north = PeepPosition::at_tile(TileCoord { x: 2, y: 40 }, 1);
        let south = PeepPosition::at_tile(TileCoord { x: 2, y: 4 }, 1);
        let stage = journey(JourneyStage::WaitingOnPlatform);
        let a = pose_for(&peep(Mood::Content, 0), &north, &stage, 64);
        let b = pose_for(&peep(Mood::Content, 0), &south, &stage, 64);
        assert!(b.translation.z > a.translation.z);
        assert!(a.translation.z >= PEEP_Z_BASE);
        assert!(
            b.translation.z < PEEP_Z_BASE + 1.0,
            "must stay under trains"
        );
    }

    /// The playtest fix, seen from the renderer: the drawn peep walks the route
    /// the sim planned, tile by tile, on whole texels, facing where it is going
    /// — and never over water.
    #[test]
    fn the_sprite_follows_the_walk_route_round_the_water() {
        use rail_sim::peeps::{WalkRoute, WalkRouter, WalkStep, WalkWorld, WALK_TILES_PER_TICK};
        use rail_sim::TrackTerrain;

        // A river at x = 3 with a single dry ford at y = 4.
        let (w, h) = (8u32, 6u32);
        let terrain = TrackTerrain::new(
            w,
            h,
            (0..w * h).map(|i| {
                let x = (i % w) as i32;
                let y = (i / w) as i32;
                (x == 3 && y != 4, 0i8)
            }),
        );
        let world = WalkWorld::new(&terrain, None);
        let mut router = WalkRouter::default();
        let mut route = WalkRoute::default();
        let mut pos = PeepPosition::at_tile(TileCoord { x: 1, y: 1 }, 5);
        let stage = journey(JourneyStage::WalkingToStation);
        let walker = peep(Mood::Content, 0);
        let goal = TileCoord { x: 6, y: 1 };

        let mut visited: Vec<TileCoord> = Vec::new();
        let mut facings: Vec<Facing> = Vec::new();
        let mut arrived = false;
        for _ in 0..4_000 {
            router.begin_tick();
            let step = route.advance(&mut pos, goal, WALK_TILES_PER_TICK, &world, &mut router);
            assert_ne!(step, WalkStep::NoRoute, "the ford should be reachable");

            let pose = pose_for(&walker, &pos, &stage, h);
            // Pixel contract: sub-tile motion is the sim's, whole texels are ours.
            assert_eq!(pose.translation.x, pose.translation.x.round());
            assert_eq!(pose.translation.y, pose.translation.y.round());
            assert!(
                !terrain.is_water(pos.tile()),
                "drew a peep standing on water at {:?}",
                pos.tile()
            );

            if visited.last() != Some(&pos.tile()) {
                visited.push(pos.tile());
            }
            if facings.last() != Some(&pos.facing) {
                facings.push(pos.facing);
            }
            if step == WalkStep::Arrived {
                arrived = true;
                break;
            }
        }

        assert!(arrived, "the peep never finished the route");
        assert_eq!(
            visited,
            route.tiles(),
            "the sprite did not follow the planned route tile by tile"
        );
        assert_eq!(visited.last(), Some(&goal));
        assert!(
            facings.contains(&Facing::North) && facings.contains(&Facing::East),
            "facing must come from the direction of travel: {facings:?}"
        );

        // Turning is a different arrangement of parts, never a rotation.
        let mut north = pos;
        north.facing = Facing::North;
        let mut east = pos;
        east.facing = Facing::East;
        assert_ne!(
            pose_for(&walker, &north, &stage, h).head.offset.x,
            pose_for(&walker, &east, &stage, h).head.offset.x
        );
    }

    #[test]
    fn parts_are_whole_texel_multiples() {
        let pos = PeepPosition::at_tile(TileCoord { x: 1, y: 1 }, 1);
        for mood in [Mood::Content, Mood::Frustrated] {
            let pose = pose_for(
                &peep(mood, 0),
                &pos,
                &journey(JourneyStage::WaitingOnPlatform),
                64,
            );
            for part in [pose.torso, pose.head, pose.legs] {
                let w = part.size.x / TEXEL;
                let h = part.size.y / TEXEL;
                assert!(
                    (w - w.round()).abs() < 1e-3,
                    "width {w} is not whole texels"
                );
                assert!(
                    (h - h.round()).abs() < 1e-3,
                    "height {h} is not whole texels"
                );
            }
        }
    }
}
