//! Placeholder train sprites following sim [`TrainLocation`].
//!
//! Position lerps along the current → next track tile using `progress` /
//! `ticks_for_piece`, plus fixed-timestep overstep while the sim is running.
//! Facing uses axis-aligned size (elongate along travel) and `flip_x` — no
//! runtime rotation (art direction).
//!
//! Congestion is visible without a panel (`docs/design/07-trains-and-lines.md`
//! §4.1): a held train freezes on its stop line and raises a stop indicator, and
//! its smoke goes idle because puffs are emitted per tile crossed and a held
//! train crosses nothing. A row of held trains therefore reads as a queue.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use rail_map::{tile_to_world, TILE_SIZE};
use rail_sim::{
    commands::TrainKind, ticks_for_piece, SimClock, TileCoord, TileOccupancy, TrackNetwork,
    TrackPiece, Train, TrainId, TrainLocation,
};

use crate::palette::{ROCK_L, WARN};

const TRAIN_Z: f32 = 3.0;
/// Length along travel (world units).
const ALONG: f32 = TILE_SIZE * 0.55;
/// Thickness across the rail.
const ACROSS: f32 = TILE_SIZE * 0.22;

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
    trains: Query<(&Train, &TrainLocation)>,
    mut sprites: Query<(Entity, &TrainSprite, &mut Transform, &mut Sprite)>,
) {
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
        let color = match train.kind {
            TrainKind::Transit => Color::srgb(0.2, 0.55, 0.9),
            TrainKind::Transport => Color::srgb(0.9, 0.65, 0.15),
        };

        if let Some(&entity) = by_id.get(&train.id) {
            let Ok((_, _, mut tf, mut sprite)) = sprites.get_mut(entity) else {
                continue;
            };
            tf.translation.x = pose.x;
            tf.translation.y = pose.y;
            tf.translation.z = TRAIN_Z;
            sprite.custom_size = Some(pose.size);
            sprite.flip_x = pose.flip_x;
            sprite.color = color;
        } else {
            let mut sprite = Sprite::from_color(color, pose.size);
            sprite.flip_x = pose.flip_x;
            commands
                .spawn((
                    sprite,
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
    size: Vec2,
    flip_x: bool,
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
    let idle = TrainPose {
        x: cx,
        y: cy,
        size: Vec2::new(ALONG, ACROSS),
        flip_x: false,
    };

    let Some(&next_id) = loc.path.get(loc.path_index.saturating_add(1)) else {
        return idle;
    };
    let Some(next) = network.piece(next_id) else {
        return idle;
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
    let (size, flip_x) = facing_sprite(piece.tile, next.tile);

    TrainPose {
        x: cx + (nx - cx) * t,
        y: cy + (ny - cy) * t,
        size,
        flip_x,
    }
}

/// Sub-tile blend in \[0, 1\] toward the next tile center.
fn lerp_fraction(progress: u16, needed: u16, overstep: f32) -> f32 {
    let denom = needed.max(1) as f32;
    ((progress as f32) + overstep.clamp(0.0, 1.0)) / denom
}

/// Axis-aligned size elongated along travel; `flip_x` when heading west.
fn facing_sprite(from: TileCoord, to: TileCoord) -> (Vec2, bool) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = ((dx * dx + dy * dy) as f32).sqrt().max(1.0);
    let nx = dx as f32 / len;
    let ny = dy as f32 / len;
    let size = Vec2::new(
        ACROSS + (ALONG - ACROSS) * nx.abs(),
        ACROSS + (ALONG - ACROSS) * ny.abs(),
    );
    (size, dx < 0)
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

    #[test]
    fn facing_elongates_cardinals() {
        let (east, flip) = facing_sprite(TileCoord { x: 0, y: 0 }, TileCoord { x: 1, y: 0 });
        assert!(!flip);
        assert!(east.x > east.y);

        let (west, flip) = facing_sprite(TileCoord { x: 1, y: 0 }, TileCoord { x: 0, y: 0 });
        assert!(flip);
        assert!(west.x > west.y);

        let (north, flip) = facing_sprite(TileCoord { x: 0, y: 0 }, TileCoord { x: 0, y: 1 });
        assert!(!flip);
        assert!(north.y > north.x);
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
