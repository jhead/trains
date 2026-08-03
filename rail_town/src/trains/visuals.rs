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

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use rail_map::{tile_to_world, TILE_SIZE};
use rail_sim::track::dir_index;
use rail_sim::{
    commands::TrainKind, ticks_for_piece, SimClock, TileOccupancy, TrackNetwork, TrackPiece, Train,
    TrainId, TrainLocation,
};

use super::bank::{entry_for_dir, facing_entry, TrainBank};
use crate::palette::{ROCK_L, WARN};

const TRAIN_Z: f32 = 3.0;
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

pub fn sync_train_sprites(
    mut commands: Commands,
    network: Res<TrackNetwork>,
    clock: Res<SimClock>,
    occupancy: Res<TileOccupancy>,
    fixed_time: Res<Time<Fixed>>,
    mut images: ResMut<Assets<Image>>,
    mut bank: Local<TrainBank>,
    trains: Query<(&Train, &TrainLocation)>,
    mut sprites: Query<(Entity, &TrainSprite, &mut Transform, &mut Sprite)>,
) {
    let _perf = crate::overlays::perf::scope("sync_train_sprites");
    let overstep = if clock.paused {
        0.0
    } else {
        fixed_time.overstep_fraction()
    };

    let mut by_id: HashMap<TrainId, Entity> = HashMap::with_capacity(sprites.iter().len());
    for (entity, sprite, _, _) in sprites.iter() {
        by_id.insert(sprite.id, entity);
    }

    let mut seen = HashSet::with_capacity(trains.iter().len());
    for (train, loc) in trains.iter() {
        seen.insert(train.id);
        let Some(piece) = network.piece(loc.track) else {
            continue;
        };
        let held = occupancy.is_blocked(train.id);
        let pose = present_train(train.kind, piece, loc, &network, overstep, held);
        let image = bank.get(&mut images, train.kind, pose.facing);

        if let Some(&entity) = by_id.get(&train.id) {
            let Ok((_, _, mut tf, mut sprite)) = sprites.get_mut(entity) else {
                continue;
            };
            tf.translation.x = pose.x;
            tf.translation.y = pose.y;
            tf.translation.z = TRAIN_Z;
            // Turning is picking a different cell out of the bank. Nothing else
            // about the sprite moves — no rotation, no mirror, no stretch.
            if sprite.image != image {
                sprite.image = image;
            }
        } else {
            commands
                .spawn((
                    Sprite {
                        image,
                        ..default()
                    },
                    Transform::from_xyz(pose.x, pose.y, TRAIN_Z),
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
    }

    for (entity, sprite, _, _) in sprites.iter() {
        if !seen.contains(&sprite.id) {
            commands.entity(entity).despawn();
        }
    }
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

struct TrainPose {
    x: f32,
    y: f32,
    /// Index into the facing bank — the sprite to show, not an angle to apply.
    facing: usize,
}

fn present_train(
    kind: TrainKind,
    piece: &TrackPiece,
    loc: &TrainLocation,
    network: &TrackNetwork,
    overstep: f32,
    held: bool,
) -> TrainPose {
    let (cx, cy) = tile_to_world(piece.tile);
    // The direction the train arrived on, which is the facing to hold when it
    // has nowhere left to go — a train at the end of its path should not swing
    // round to east.
    let arrived = leg_dir(network, loc, loc.path_index.checked_sub(1));

    let Some(&next_id) = loc.path.get(loc.path_index.saturating_add(1)) else {
        return TrainPose {
            x: cx,
            y: cy,
            facing: entry_for_dir(arrived.unwrap_or(DEFAULT_DIR)),
        };
    };
    let Some(next) = network.piece(next_id) else {
        return TrainPose {
            x: cx,
            y: cy,
            facing: entry_for_dir(arrived.unwrap_or(DEFAULT_DIR)),
        };
    };

    let needed = ticks_for_piece(kind, piece.max_grade, piece.curve);
    // A held train sits dead still on its stop line: adding overstep would creep
    // it forward every frame and snap it back, which reads as a shuffling queue.
    let step = if loc.parked || loc.dwell_remaining > 0 || held {
        0.0
    } else {
        overstep
    };
    let t = lerp_fraction(loc.progress, needed, step);
    let (nx, ny) = tile_to_world(next.tile);

    // The leg being run, and the one after it: a curve is a sweep through the
    // node between two legs, so the facing needs both ends of it.
    let dir = dir_index(piece.tile, next.tile).unwrap_or(DEFAULT_DIR);
    let leaving = leg_dir(network, loc, Some(loc.path_index + 1));

    TrainPose {
        x: cx + (nx - cx) * t,
        y: cy + (ny - cy) * t,
        facing: facing_entry(arrived, dir, leaving, t.clamp(0.0, 1.0)),
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
