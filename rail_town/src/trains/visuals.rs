//! Train sprites following sim [`TrainLocation`].
//!
//! Position lerps along the current → next track tile using `progress` /
//! `ticks_for_piece`, plus fixed-timestep overstep while the sim is running.
//!
//! # Facing is a baked sprite, never a transform
//!
//! Brief 01 §2.2 allows exactly one way to express direction: choose a
//! different sprite. Facing therefore selects an entry from [`super::bank`],
//! which is baked at the **realised** bearings of the track rose — `atan2` of
//! the sixteen [`DIR16`](rail_sim::track::DIR16) lattice vectors, plus the
//! midpoints between adjacent pairs so a curve sweeps rather than snaps. See
//! that module for why an even 11.25° bank would be wrong.
//!
//! Nothing here writes a rotation, a mirror or a non-uniform scale: every train
//! transform is identity rotation and unit scale, and the whole vocabulary is
//! selection.
//!
//! Congestion is visible without a panel (`docs/design/07-trains-and-lines.md`
//! §4.1): a held train freezes on its stop line and raises a stop indicator, and
//! its smoke goes idle because puffs are emitted per tile crossed and a held
//! train crosses nothing. A row of held trains therefore reads as a queue.
//!
//! # A consist follows the track, not the locomotive
//!
//! 07 §6: *"Cars follow the locomotive along the path it took, so a train
//! articulates correctly through curves."* Each car is placed by walking
//! **backwards along the train's own path** by a fixed coupling distance — not
//! by offsetting from the engine, which would cut the corner and put a carriage
//! through the inside of every curve. A car on a different leg to its engine
//! therefore holds that leg's bearing, and the consist bends.
//!
//! Every vehicle carries its own [`GroundAnchor`], which is what makes a
//! consist crossing a change of level read as following the ground: the
//! projection lifts each car by the height of the tile *it* is over, and a
//! projection flip moves all of them without this module knowing it happened.
//! Nothing here projects anything by hand.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use rail_map::{tile_to_ground, tile_to_world, TILE_SIZE};
use rail_sim::ids::TileCoord;
use rail_sim::track::dir_index;
use rail_sim::{
    cars_of, commands::TrainKind, ticks_for_consist_piece, SimClock, TileOccupancy, TrackNetwork,
    TrackPiece, Train, TrainConsist, TrainId, TrainLocation,
};

use super::bank::{entry_for_dir, facing_entry, TrainBank, TrainPart};
use crate::map::GroundAnchor;
use crate::palette::{ROCK_L, WARN};

const TRAIN_Z: f32 = 3.0;

/// Coupling distance between vehicle centres, in tiles.
///
/// The engine body is 0.56 of a tile long and a car is 0.41, so 0.55 leaves a
/// texel or two of daylight at the coupling — enough that a consist reads as
/// separate vehicles, close enough that it never reads as two trains.
const CAR_SPACING_TILES: f32 = 0.55;
/// Facing when a train has nowhere to go and no history: east, the default axis.
const DEFAULT_DIR: usize = 2;

/// Ticks held before the stop indicator shows — a one-tick pause is not news.
const STOP_INDICATOR_AFTER_TICKS: u16 = 2;
const STOP_INDICATOR_SIZE: f32 = TILE_SIZE * 0.18;
const STOP_INDICATOR_LIFT: f32 = TILE_SIZE * 0.40;

/// Smoke: emitted per tile crossed, drifting, five-second life (art direction §5.4).
const SMOKE_Z: f32 = 2.6;
const SMOKE_LIFE_SECS: f32 = 5.0;
/// World units a puff rises over a full life.
const SMOKE_RISE: f32 = TILE_SIZE * 0.7;
/// World units per second of sideways drift at full strength.
const SMOKE_DRIFT: f32 = TILE_SIZE * 0.12;
const SMOKE_SIZE: f32 = TILE_SIZE * 0.16;
const SMOKE_ALPHA: f32 = 0.5;
/// Hard cap so a busy network cannot flood the world with sprites.
const SMOKE_MAX_PUFFS: usize = 512;

#[derive(Component, Debug, Clone, Copy)]
pub struct TrainSprite {
    pub id: TrainId,
}

/// One trailing car of a consist. `index` is 1 for the first car behind the
/// engine, and a car whose index is past the train's length is despawned — so
/// selling a car takes its sprite with it.
#[derive(Component, Debug, Clone, Copy)]
pub struct TrainCarSprite {
    pub id: TrainId,
    pub index: u8,
}

/// Held-train marker riding above its train sprite.
#[derive(Component, Debug, Clone, Copy)]
pub struct TrainStopIndicator {
    pub id: TrainId,
}

/// One drifting smoke puff, dropped where a train crossed a tile.
#[derive(Component, Debug, Clone, Copy)]
pub struct SmokePuff {
    age: f32,
    drift: f32,
}

#[allow(clippy::too_many_arguments)]
pub fn sync_train_sprites(
    mut commands: Commands,
    network: Res<TrackNetwork>,
    clock: Res<SimClock>,
    occupancy: Res<TileOccupancy>,
    fixed_time: Res<Time<Fixed>>,
    mut images: ResMut<Assets<Image>>,
    mut bank: Local<TrainBank>,
    trains: Query<(&Train, &TrainLocation, Option<&TrainConsist>)>,
    mut sprites: Query<(
        Entity,
        &TrainSprite,
        &mut Transform,
        &mut Sprite,
        &mut GroundAnchor,
    )>,
    mut cars: Query<
        (
            Entity,
            &TrainCarSprite,
            &mut Transform,
            &mut Sprite,
            &mut GroundAnchor,
        ),
        Without<TrainSprite>,
    >,
) {
    let _perf = crate::overlays::perf::scope("sync_train_sprites");
    let overstep = if clock.paused {
        0.0
    } else {
        fixed_time.overstep_fraction()
    };

    let mut by_id: HashMap<TrainId, Entity> = HashMap::with_capacity(sprites.iter().len());
    for (entity, sprite, ..) in sprites.iter() {
        by_id.insert(sprite.id, entity);
    }
    let mut car_by_slot: HashMap<(TrainId, u8), Entity> = HashMap::with_capacity(cars.iter().len());
    for (entity, car, ..) in cars.iter() {
        car_by_slot.insert((car.id, car.index), entity);
    }

    let mut seen = HashSet::with_capacity(trains.iter().len());
    // Every `(train, car index)` still drawn this frame. A consist that grew
    // gains a slot here and a train that was sold loses all of them.
    let mut seen_cars: HashSet<(TrainId, u8)> = HashSet::new();
    for (train, loc, consist) in trains.iter() {
        seen.insert(train.id);
        let Some(piece) = network.piece(loc.track) else {
            continue;
        };
        let held = occupancy.is_blocked(train.id);
        let length = cars_of(consist);
        let pose = present_train(train.kind, length, piece, loc, &network, overstep, held);
        let image = bank.get(&mut images, train.kind, pose.facing);

        if let Some(&entity) = by_id.get(&train.id) {
            let Ok((_, _, mut tf, mut sprite, mut anchor)) = sprites.get_mut(entity) else {
                continue;
            };
            place(&mut tf, &mut anchor, &pose, TRAIN_Z);
            // Turning is picking a different cell out of the bank. Nothing else
            // about the sprite moves — no rotation, no mirror, no stretch.
            if sprite.image != image {
                sprite.image = image;
            }
        } else {
            let anchor = pose.anchor();
            commands
                .spawn((
                    Sprite {
                        image,
                        ..default()
                    },
                    anchor.transform(TRAIN_Z),
                    anchor,
                    TrainSprite { id: train.id },
                ))
                .with_children(|train_sprite| {
                    train_sprite.spawn((
                        Sprite::from_color(WARN, Vec2::splat(STOP_INDICATOR_SIZE)),
                        Transform::from_xyz(0.0, STOP_INDICATOR_LIFT, 0.1),
                        Visibility::Hidden,
                        TrainStopIndicator { id: train.id },
                    ));
                });
        }

        for index in 1..length {
            let pose = present_car(&pose, index, loc, &network);
            let image = bank.get_part(&mut images, train.kind, TrainPart::Car, pose.facing);
            seen_cars.insert((train.id, index));
            if let Some(&entity) = car_by_slot.get(&(train.id, index)) {
                let Ok((_, _, mut tf, mut sprite, mut anchor)) = cars.get_mut(entity) else {
                    continue;
                };
                // A car sits a hair below its engine so a consist overlapping at
                // a curve stacks the way a train does: engine on top.
                place(&mut tf, &mut anchor, &pose, TRAIN_Z - 0.01);
                if sprite.image != image {
                    sprite.image = image;
                }
            } else {
                let anchor = pose.anchor();
                commands.spawn((
                    Sprite {
                        image,
                        ..default()
                    },
                    anchor.transform(TRAIN_Z - 0.01),
                    anchor,
                    TrainCarSprite {
                        id: train.id,
                        index,
                    },
                ));
            }
        }
    }

    for (entity, sprite, ..) in sprites.iter() {
        if !seen.contains(&sprite.id) {
            commands.entity(entity).despawn();
        }
    }
    // A car whose train was sold, or whose slot was shortened, goes with it.
    for (entity, car, ..) in cars.iter() {
        if !seen_cars.contains(&(car.id, car.index)) {
            commands.entity(entity).despawn();
        }
    }
}

/// Put a vehicle where its pose says, on layer `z`.
///
/// Writes the anchor *and* the transform the anchor implies, so the sprite is
/// correct on this frame rather than on the one after
/// [`anchor_world_sprites`](crate::map::projection::anchor_world_sprites) next
/// runs — and both agree exactly, because both ask the projection.
fn place(transform: &mut Transform, anchor: &mut GroundAnchor, pose: &TrainPose, z: f32) {
    let wanted = pose.anchor();
    if *anchor != wanted {
        *anchor = wanted;
    }
    let world = wanted.world();
    transform.translation.x = world.x;
    transform.translation.y = world.y;
    transform.translation.z = z;
}

/// Raise the stop indicator on trains the sim is holding.
///
/// §4.1 — the player should see *that* a train is stuck without selecting it;
/// the inspector already answers *why*.
pub fn sync_train_stop_indicators(
    occupancy: Res<TileOccupancy>,
    mut indicators: Query<(&TrainStopIndicator, &mut Visibility)>,
) {
    let _perf = crate::overlays::perf::scope("sync_train_stop_indicators");
    for (indicator, mut visibility) in indicators.iter_mut() {
        let held = occupancy.held_ticks(indicator.id) >= STOP_INDICATOR_AFTER_TICKS;
        let wanted = if held {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Drop a puff each time a train crosses into a new tile, and age the rest.
///
/// A held train advances no tiles, so its smoke goes idle on its own — the
/// stopped state is legible from the world art alone.
pub fn sync_train_smoke(
    mut commands: Commands,
    time: Res<Time>,
    network: Res<TrackNetwork>,
    trains: Query<(&Train, &TrainLocation)>,
    mut puffs: Query<(Entity, &mut SmokePuff, &mut Transform, &mut Sprite)>,
    mut last_step: Local<HashMap<TrainId, usize>>,
) {
    let _perf = crate::overlays::perf::scope("sync_train_smoke");
    let dt = time.delta_secs();
    for (entity, mut puff, mut transform, mut sprite) in puffs.iter_mut() {
        puff.age += dt;
        if puff.age >= SMOKE_LIFE_SECS {
            commands.entity(entity).despawn();
            continue;
        }
        transform.translation.y += SMOKE_RISE * dt / SMOKE_LIFE_SECS;
        transform.translation.x += puff.drift * SMOKE_DRIFT * dt;
        sprite.color = sprite.color.with_alpha(puff_alpha(puff.age));
    }

    let mut live: HashSet<TrainId> = HashSet::new();
    let room = SMOKE_MAX_PUFFS.saturating_sub(puffs.iter().len());
    let mut budget = room;
    for (train, loc) in trains.iter() {
        live.insert(train.id);
        let previous = last_step.insert(train.id, loc.path_index);
        let crossed = previous.is_some_and(|step| step != loc.path_index);
        if !crossed || loc.parked || loc.dwell_remaining > 0 || budget == 0 {
            continue;
        }
        let Some(piece) = network.piece(loc.track) else {
            continue;
        };
        let (x, y) = tile_to_world(piece.tile);
        budget -= 1;
        commands.spawn((
            Sprite::from_color(ROCK_L.with_alpha(SMOKE_ALPHA), Vec2::splat(SMOKE_SIZE)),
            Transform::from_xyz(x, y, SMOKE_Z),
            SmokePuff {
                age: 0.0,
                drift: puff_drift(train.id, loc.path_index),
            },
        ));
    }
    last_step.retain(|id, _| live.contains(id));
}

/// Fade in over the first beat, then out over the rest of the life.
fn puff_alpha(age: f32) -> f32 {
    let t = (age / SMOKE_LIFE_SECS).clamp(0.0, 1.0);
    SMOKE_ALPHA * (1.0 - t) * (1.0 - t)
}

/// Deterministic per-puff sideways drift in \[-1, 1\] — presentation only, so
/// the sim never sees it.
fn puff_drift(id: TrainId, step: usize) -> f32 {
    let mixed = id
        .0
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(step as u64)
        .wrapping_mul(0xbf58_476d_1ce4_e5b9);
    ((mixed >> 40) as f32 / 8_388_608.0) - 1.0
}

/// Where one vehicle stands, on the ground plane, and which cell it shows.
///
/// Ground rather than world: the projection turns it into a screen position and
/// applies whatever lift the tile underneath has, which is the only way a
/// consist on a grade reads as standing on it. See [`GroundAnchor`].
struct TrainPose {
    ground: Vec2,
    /// Index into the facing bank — the sprite to show, not an angle to apply.
    facing: usize,
}

impl TrainPose {
    fn anchor(&self) -> GroundAnchor {
        GroundAnchor::new(self.ground.x, self.ground.y)
    }
}

/// Ground-plane centre of a tile.
fn ground_of(tile: TileCoord) -> Vec2 {
    let (gx, gy) = tile_to_ground(tile);
    Vec2::new(gx, gy)
}

fn present_train(
    kind: TrainKind,
    cars: u8,
    piece: &TrackPiece,
    loc: &TrainLocation,
    network: &TrackNetwork,
    overstep: f32,
    held: bool,
) -> TrainPose {
    let here = ground_of(piece.tile);
    // The direction the train arrived on, which is the facing to hold when it
    // has nowhere left to go — a train at the end of its path should not swing
    // round to east.
    let arrived = leg_dir(network, loc, loc.path_index.checked_sub(1));

    let standing = TrainPose {
        ground: here,
        facing: entry_for_dir(arrived.unwrap_or(DEFAULT_DIR)),
    };
    let Some(&next_id) = loc.path.get(loc.path_index.saturating_add(1)) else {
        return standing;
    };
    let Some(next) = network.piece(next_id) else {
        return standing;
    };

    // The consist's own pace: a longer train crosses a tile in more ticks, and
    // interpolating against the single-car figure would run the sprite ahead of
    // the simulation and snap it back at every tile boundary.
    let needed = ticks_for_consist_piece(kind, cars, piece.max_grade, piece.curve);
    // A held train sits dead still on its stop line: adding overstep would creep
    // it forward every frame and snap it back, which reads as a shuffling queue.
    let step = if loc.parked || loc.dwell_remaining > 0 || held {
        0.0
    } else {
        overstep
    };
    let t = lerp_fraction(loc.progress, needed, step);

    // The leg being run, and the one after it: a curve is a sweep through the
    // node between two legs, so the facing needs both ends of it.
    let dir = dir_index(piece.tile, next.tile).unwrap_or(DEFAULT_DIR);
    let leaving = leg_dir(network, loc, Some(loc.path_index + 1));

    TrainPose {
        ground: here.lerp(ground_of(next.tile), t),
        facing: facing_entry(arrived, dir, leaving, t.clamp(0.0, 1.0)),
    }
}

/// Where the `index`-th car sits, walking back along the path the train took.
///
/// `index` counts from 1 (the vehicle immediately behind the engine). The walk
/// is along the **route**, so a car still on the previous leg holds the previous
/// leg's bearing and the consist bends through the curve rather than skidding
/// across its inside.
///
/// A train that has not travelled far enough to have a tail — one just placed,
/// or one on the first tile of its path — bunches its cars up at the start of
/// the route rather than inventing track behind itself. That is a second of
/// overlap when a train enters service, against the alternative of a carriage
/// standing on the grass.
fn present_car(
    head: &TrainPose,
    index: u8,
    loc: &TrainLocation,
    network: &TrackNetwork,
) -> TrainPose {
    let mut remaining = CAR_SPACING_TILES * TILE_SIZE * f32::from(index.max(1));
    let mut point = head.ground;
    // `path_index` is the tile the engine is leaving; everything before it is
    // ground the train has already covered.
    let mut leg = loc.path_index;
    let mut facing = head.facing;

    loop {
        let Some(previous) = loc.path.get(leg).and_then(|id| network.piece(*id)) else {
            break;
        };
        let behind = ground_of(previous.tile);
        let span = point.distance(behind);
        if span >= remaining {
            let t = if span > f32::EPSILON {
                remaining / span
            } else {
                0.0
            };
            let at = point.lerp(behind, t);
            // The bearing of the leg this car is standing on, swept through the
            // nodes at either end exactly as the engine's is.
            let ahead = loc
                .path
                .get(leg + 1)
                .copied()
                .unwrap_or(loc.track);
            if let Some(next) = network.piece(ahead) {
                if let Some(dir) = dir_index(previous.tile, next.tile) {
                    let along = 1.0 - t;
                    facing = facing_entry(
                        leg_dir(network, loc, leg.checked_sub(1)),
                        dir,
                        leg_dir(network, loc, Some(leg + 1)),
                        along.clamp(0.0, 1.0),
                    );
                }
            }
            return TrainPose { ground: at, facing };
        }
        remaining -= span;
        point = behind;
        let Some(earlier) = leg.checked_sub(1) else {
            break;
        };
        leg = earlier;
    }

    // Ran out of travelled path: sit on the oldest tile the route knows.
    TrainPose {
        ground: point,
        facing,
    }
}

/// Direction of the path leg starting at `index`, if there is one.
///
/// The path is track ids, and a direction is a pair of tiles, so both ends have
/// to still exist in the network — a leg can be demolished out from under a
/// train between frames.
fn leg_dir(network: &TrackNetwork, loc: &TrainLocation, index: Option<usize>) -> Option<usize> {
    let index = index?;
    let from = network.piece(*loc.path.get(index)?)?.tile;
    let to = network.piece(*loc.path.get(index + 1)?)?.tile;
    dir_index(from, to)
}

/// Sub-tile blend in \[0, 1\] toward the next tile center.
fn lerp_fraction(progress: u16, needed: u16, overstep: f32) -> f32 {
    let denom = needed.max(1) as f32;
    ((progress as f32) + overstep.clamp(0.0, 1.0)) / denom
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerp_at_endpoints() {
        assert!((lerp_fraction(0, 4, 0.0) - 0.0).abs() < f32::EPSILON);
        assert!((lerp_fraction(4, 4, 0.0) - 1.0).abs() < f32::EPSILON);
        assert!((lerp_fraction(2, 4, 0.0) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn overstep_smooths_within_tick() {
        let t = lerp_fraction(1, 4, 0.5);
        assert!((t - 1.5 / 4.0).abs() < 1e-5);
    }

    /// The old stand-in expressed facing as a stretched rectangle plus
    /// `flip_x`, which gave NE and SE the same appearance. Every one of the
    /// sixteen now selects its own bank entry.
    #[test]
    fn each_of_the_sixteen_selects_its_own_facing() {
        let entries: Vec<usize> = (0..rail_sim::track::DIR_COUNT)
            .map(|dir| facing_entry(None, dir, None, 0.5))
            .collect();
        let unique: HashSet<usize> = entries.iter().copied().collect();
        assert_eq!(unique.len(), rail_sim::track::DIR_COUNT, "{entries:?}");
        // The pairs the old stretch-and-mirror scheme collapsed together.
        assert_ne!(entries[1], entries[3], "NE and SE must differ");
        assert_ne!(entries[9], entries[10], "ENE and ESE must differ");
        assert_ne!(entries[2], entries[6], "E and W must differ");
    }

    /// Brief 01 §2.2, measured on the sprites the system actually spawns: a
    /// train carries no rotation, no mirror and no stretch, on any heading. The
    /// facing lives entirely in which image the sprite holds.
    #[test]
    fn train_sprites_are_never_rotated_mirrored_or_stretched() {
        use rail_sim::track::{step, try_place_track, TrackTerrain};
        use rail_sim::{Money, MoneyLedger, TileCoord, GROUND_LAYER};

        // Each heading is checked in its own app, so the cells are compared by
        // their texels rather than by handle: a fresh `Assets<Image>` reissues
        // the same ids.
        let mut seen_cells: HashSet<Vec<u8>> = HashSet::new();
        for dir in 0..rail_sim::track::DIR_COUNT {
            let terrain = TrackTerrain::new(24, 24, (0..24 * 24).map(|_| (false, 0i8)));
            let mut network = TrackNetwork::new();
            let mut money = Money::new(10_000_000);
            let mut ledger = MoneyLedger::default();
            let start = TileCoord { x: 8, y: 8 };
            let mut ids = Vec::new();
            for tile in [start, step(start, dir)] {
                ids.push(
                    try_place_track(
                        &mut network,
                        &mut money,
                        &mut ledger,
                        &terrain,
                        tile,
                        GROUND_LAYER,
                    )
                    .expect("track")
                    .id,
                );
            }

            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .init_resource::<Assets<Image>>()
                .init_resource::<SimClock>()
                .init_resource::<TileOccupancy>()
                .insert_resource(network)
                .add_systems(Update, sync_train_sprites);
            app.world_mut().spawn((
                Train {
                    id: TrainId(1),
                    kind: TrainKind::Transit,
                },
                TrainLocation {
                    track: ids[0],
                    path: ids.clone(),
                    path_index: 0,
                    progress: 0,
                    parked: false,
                    dwell_remaining: 0,
                },
            ));
            app.update();

            let (transform, sprite) = app
                .world_mut()
                .query_filtered::<(&Transform, &Sprite), With<TrainSprite>>()
                .single(app.world())
                .map(|(t, s)| (*t, s.clone()))
                .expect("a train sprite");
            assert_eq!(transform.rotation, Quat::IDENTITY, "dir {dir} rotated");
            assert_eq!(transform.scale, Vec3::ONE, "dir {dir} scaled");
            assert!(!sprite.flip_x, "dir {dir} mirrored");
            assert!(!sprite.flip_y, "dir {dir} mirrored");
            assert_eq!(sprite.custom_size, None, "dir {dir} stretched");
            // ... and each heading really did select a different cell.
            let cell = app
                .world()
                .resource::<Assets<Image>>()
                .get(&sprite.image)
                .and_then(|image| image.data.clone())
                .expect("the facing cell");
            assert!(
                seen_cells.insert(cell),
                "dir {dir} drew the same cell as another heading"
            );
        }
        assert_eq!(seen_cells.len(), rail_sim::track::DIR_COUNT);
    }

    /// **Report A, half one.** "I cannot see it" pointed first at the sprite, so
    /// this pins the freight train's art to the ground it stands on — in both
    /// projections, because the game boots isometric and the top-down view is a
    /// key away. It passes, which is what moved the diagnosis off the sprite
    /// bank and onto the silence around the verb (see [`super::tools`]).
    #[test]
    fn a_freight_train_gets_a_sprite_standing_on_its_own_tile_in_either_view() {
        use rail_sim::track::{try_place_track, TrackTerrain};
        use rail_sim::{Money, MoneyLedger, TileCoord, GROUND_LAYER};

        for projection in [rail_map::Projection::TopDown, rail_map::Projection::Iso] {
            let _guard = crate::map::tests::ProjectionGuard::new(projection);
            let terrain = TrackTerrain::new(24, 24, (0..24 * 24).map(|_| (false, 0i8)));
            let mut network = TrackNetwork::new();
            let mut money = Money::new(10_000_000);
            let mut ledger = MoneyLedger::default();
            let tile = TileCoord { x: 8, y: 8 };
            let id = try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                tile,
                GROUND_LAYER,
            )
            .expect("track")
            .id;

            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .init_resource::<Assets<Image>>()
                .init_resource::<SimClock>()
                .init_resource::<TileOccupancy>()
                .insert_resource(network)
                .add_systems(Update, sync_train_sprites);
            app.world_mut().spawn((
                Train {
                    id: TrainId(7),
                    kind: TrainKind::Transport,
                },
                TrainLocation::at_track(id),
            ));
            app.update();

            let (transform, sprite) = app
                .world_mut()
                .query_filtered::<(&Transform, &Sprite), With<TrainSprite>>()
                .single(app.world())
                .map(|(t, s)| (*t, s.clone()))
                .expect("a freight train draws something");
            let (wx, wy) = tile_to_world(tile);
            assert_eq!(
                (transform.translation.x, transform.translation.y),
                (wx, wy),
                "{projection:?}: the freight sprite is off its own tile"
            );
            assert_eq!(transform.translation.z, TRAIN_Z);
            // ... and the cell it holds has paint on it. An empty texture is a
            // sprite that exists and shows nothing, which is the same bug from
            // the player's side. (That the two kinds draw differently is
            // `bank::the_two_kinds_do_not_draw_alike`.)
            let cell = app
                .world()
                .resource::<Assets<Image>>()
                .get(&sprite.image)
                .and_then(|image| image.data.clone())
                .expect("a baked cell");
            assert!(
                cell.chunks_exact(4).any(|texel| texel[3] > 0),
                "{projection:?}: the freight cell is blank"
            );
        }
    }

    // ─ Consists ────────────────────────────────────────────

    use rail_sim::track::{try_place_track, TrackTerrain};
    use rail_sim::{Money, MoneyLedger, GROUND_LAYER};

    /// A world with `tiles` of track laid in order, and one train on it.
    fn consist_app(tiles: &[(i32, i32)], cars: u8, at: usize) -> (App, Vec<TileCoord>) {
        let terrain = TrackTerrain::new(24, 24, (0..24 * 24).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut money = Money::new(10_000_000);
        let mut ledger = MoneyLedger::default();
        let mut ids = Vec::new();
        let mut coords = Vec::new();
        for &(x, y) in tiles {
            let tile = TileCoord { x, y };
            ids.push(
                try_place_track(
                    &mut network,
                    &mut money,
                    &mut ledger,
                    &terrain,
                    tile,
                    GROUND_LAYER,
                )
                .expect("track")
                .id,
            );
            coords.push(tile);
        }

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Image>>()
            .init_resource::<SimClock>()
            .init_resource::<TileOccupancy>()
            .insert_resource(network)
            .add_systems(Update, sync_train_sprites);
        let mut loc = TrainLocation::at_track(ids[0]);
        loc.set_path(ids.clone());
        loc.track = ids[at];
        loc.path_index = at;
        app.world_mut().spawn((
            Train {
                id: TrainId(1),
                kind: TrainKind::Transit,
            },
            loc,
            TrainConsist { cars, laden: 0 },
        ));
        (app, coords)
    }

    /// Every vehicle the app drew, in consist order: the engine first.
    fn consist_ground(app: &mut App) -> Vec<Vec2> {
        let engine = app
            .world_mut()
            .query_filtered::<&GroundAnchor, With<TrainSprite>>()
            .single(app.world())
            .map(|a| a.0)
            .expect("an engine");
        let mut cars: Vec<(u8, Vec2)> = app
            .world_mut()
            .query::<(&TrainCarSprite, &GroundAnchor)>()
            .iter(app.world())
            .map(|(car, anchor)| (car.index, anchor.0))
            .collect();
        cars.sort_by_key(|(index, _)| *index);
        std::iter::once(engine).chain(cars.into_iter().map(|(_, at)| at)).collect()
    }

    /// **The owner's ask, drawn.** A three-car train is three vehicles on the
    /// map, coupled in a line behind the engine — not one sprite with a number
    /// on it.
    #[test]
    fn a_consist_draws_one_vehicle_per_car_trailing_the_engine() {
        let _guard = crate::map::tests::ProjectionGuard::new(rail_map::Projection::TopDown);
        let straight: Vec<(i32, i32)> = (4..=12).map(|x| (x, 8)).collect();
        let (mut app, _) = consist_app(&straight, 3, 4);
        app.update();

        let vehicles = consist_ground(&mut app);
        assert_eq!(vehicles.len(), 3, "an engine and two cars");

        // Each one is behind the last, along the way the train came (west).
        for pair in vehicles.windows(2) {
            assert!(
                pair[1].x < pair[0].x,
                "a car must trail the vehicle in front of it: {vehicles:?}"
            );
        }
        // Coupled, not scattered: the spacing is the coupling distance.
        let want = CAR_SPACING_TILES * TILE_SIZE;
        for pair in vehicles.windows(2) {
            let gap = pair[0].distance(pair[1]);
            assert!(
                (gap - want).abs() < 0.5,
                "coupling gap {gap} should be about {want}"
            );
        }
        // A single-car train is exactly what it always was: one sprite.
        let (mut app, _) = consist_app(&straight, 1, 4);
        app.update();
        assert_eq!(consist_ground(&mut app).len(), 1);
    }

    /// **Articulation** (07 §6). A car on the leg before the engine's holds
    /// *that* leg's bearing, so a consist bends through a curve instead of
    /// sliding across the inside of it.
    #[test]
    fn cars_follow_the_path_round_a_corner_rather_than_cutting_it() {
        let _guard = crate::map::tests::ProjectionGuard::new(rail_map::Projection::TopDown);
        // An L: east along y=8 to x=10, then south down x=10.
        let mut tiles: Vec<(i32, i32)> = (4..=10).map(|x| (x, 8)).collect();
        tiles.extend((9..=13).map(|y| (10, y)));
        // Two tiles past the corner, so the engine is southbound and the tail is
        // still on the eastbound leg.
        let corner_index = 6;
        let (mut app, coords) = consist_app(&tiles, 3, corner_index + 1);
        app.update();

        let vehicles = consist_ground(&mut app);
        let corner = {
            let (gx, gy) = tile_to_ground(coords[corner_index]);
            Vec2::new(gx, gy)
        };
        // The engine is south of the corner; the last car is west of it. A
        // straight-line offset from the engine would have put that car *inside*
        // the corner, south-west of it, on no track at all.
        assert!(vehicles[0].y > corner.y, "the engine has turned south");
        let tail = *vehicles.last().expect("a tail");
        assert!(
            tail.x < corner.x - 1.0,
            "the tail should still be on the eastbound leg: {vehicles:?}"
        );
        assert!(
            (tail.y - corner.y).abs() < 1.0,
            "and level with the leg it is on, not cutting the corner"
        );

        // The facings say the same thing: a vehicle on the eastbound leg is not
        // pointing the way the engine on the southbound leg is pointing.
        let (loc, network) = {
            let network = app.world().resource::<TrackNetwork>().clone();
            let loc = app
                .world_mut()
                .query::<&TrainLocation>()
                .single(app.world())
                .cloned()
                .expect("the train");
            (loc, network)
        };
        let piece = network.piece(loc.track).expect("a tile under it");
        let engine = present_train(TrainKind::Transit, 3, piece, &loc, &network, 0.0, false);
        let tail = present_car(&engine, 2, &loc, &network);
        assert_ne!(
            engine.facing, tail.facing,
            "a consist through a curve holds two bearings at once, or it is not \
             articulating"
        );
    }

    /// **Both projections, on the consist.** Every vehicle stands on the ground
    /// its anchor names, in either view — the class invariant `map::projection`
    /// sweeps for, checked here because the sweep's app has no trains in it.
    #[test]
    fn every_car_stands_on_its_own_ground_in_both_projections() {
        let straight: Vec<(i32, i32)> = (4..=12).map(|x| (x, 8)).collect();
        for projection in [rail_map::Projection::TopDown, rail_map::Projection::Iso] {
            let _guard = crate::map::tests::ProjectionGuard::new(projection);
            let (mut app, coords) = consist_app(&straight, 3, 5);
            app.update();

            let drawn: Vec<(Vec2, Vec3)> = app
                .world_mut()
                .query::<(&GroundAnchor, &Transform)>()
                .iter(app.world())
                .map(|(anchor, tf)| (anchor.0, tf.translation))
                .collect();
            assert_eq!(drawn.len(), 3, "{projection:?}: three vehicles");

            for (ground, at) in &drawn {
                // The transform is exactly what the projection makes of the
                // anchor — nothing here is projected by hand.
                let (wx, wy) = rail_map::ground_to_world(ground.x, ground.y);
                assert_eq!(
                    (at.x, at.y),
                    (wx, wy),
                    "{projection:?}: a vehicle anchored at {ground:?} is drawn at \
                     ({}, {})",
                    at.x,
                    at.y
                );
                // …and it resolves back to a tile the train is actually on.
                let tile = rail_map::world_to_tile(at.x, at.y);
                assert!(
                    coords.contains(&tile),
                    "{projection:?}: a vehicle at {tile:?} is off the railway"
                );
            }
        }
    }

    /// A consist that grows gains a car; one that is sold takes its cars with
    /// it. A sprite for a car nobody owns is the same class of bug as a train
    /// the player cannot find.
    #[test]
    fn car_sprites_appear_and_leave_with_the_cars_they_draw() {
        let _guard = crate::map::tests::ProjectionGuard::new(rail_map::Projection::TopDown);
        let straight: Vec<(i32, i32)> = (4..=12).map(|x| (x, 8)).collect();
        let (mut app, _) = consist_app(&straight, 1, 4);
        app.update();
        assert_eq!(consist_ground(&mut app).len(), 1);

        // Couple a car on.
        {
            let mut q = app.world_mut().query::<&mut TrainConsist>();
            let world = app.world_mut();
            for mut consist in q.iter_mut(world) {
                consist.cars = 3;
            }
        }
        app.update();
        assert_eq!(consist_ground(&mut app).len(), 3, "the cars appear");

        // Shorten it again: the extra sprite has to go.
        {
            let mut q = app.world_mut().query::<&mut TrainConsist>();
            let world = app.world_mut();
            for mut consist in q.iter_mut(world) {
                consist.cars = 2;
            }
        }
        app.update();
        app.update();
        assert_eq!(consist_ground(&mut app).len(), 2);

        // Sell the train: everything it was made of leaves the map.
        let entity = app
            .world_mut()
            .query_filtered::<Entity, With<Train>>()
            .single(app.world())
            .expect("the train");
        app.world_mut().entity_mut(entity).despawn();
        app.update();
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&TrainCarSprite>()
                .iter(app.world())
                .count(),
            0,
            "a sold train leaves no carriages behind"
        );
    }

    /// A car draws a *car*, not a second engine: shorter, and no headlamp.
    /// Three identical bodies in a line would read as three trains queueing,
    /// which is a state this game genuinely has.
    #[test]
    fn a_car_is_drawn_as_a_car_and_not_as_another_engine() {
        let _guard = crate::map::tests::ProjectionGuard::new(rail_map::Projection::TopDown);
        let straight: Vec<(i32, i32)> = (4..=12).map(|x| (x, 8)).collect();
        let (mut app, _) = consist_app(&straight, 2, 4);
        app.update();

        let engine = app
            .world_mut()
            .query_filtered::<&Sprite, With<TrainSprite>>()
            .single(app.world())
            .map(|s| s.image.clone())
            .expect("an engine");
        let car = app
            .world_mut()
            .query_filtered::<&Sprite, With<TrainCarSprite>>()
            .single(app.world())
            .map(|s| s.image.clone())
            .expect("a car");

        let images = app.world().resource::<Assets<Image>>();
        let engine_px = images.get(&engine).and_then(|i| i.data.clone()).expect("baked");
        let car_px = images.get(&car).and_then(|i| i.data.clone()).expect("baked");
        assert_ne!(engine_px, car_px, "a car must not draw as the engine");
        let painted = |px: &Vec<u8>| px.chunks_exact(4).filter(|t| t[3] > 0).count();
        assert!(painted(&car_px) > 0, "the car cell is blank");
        assert!(
            painted(&car_px) < painted(&engine_px),
            "a car is shorter than the engine that pulls it"
        );
    }

    #[test]
    fn puff_fades_to_nothing_over_its_life() {
        assert!(puff_alpha(0.0) > puff_alpha(SMOKE_LIFE_SECS * 0.5));
        assert!(puff_alpha(SMOKE_LIFE_SECS) <= f32::EPSILON);
    }

    #[test]
    fn puff_drift_is_bounded_and_stable() {
        for id in 1..40u64 {
            for step in 0..8usize {
                let d = puff_drift(TrainId(id), step);
                assert!((-1.0..=1.0).contains(&d), "drift {d} out of range");
                assert_eq!(d, puff_drift(TrainId(id), step), "drift must be stable");
            }
        }
        assert_ne!(puff_drift(TrainId(1), 0), puff_drift(TrainId(1), 1));
    }
}
