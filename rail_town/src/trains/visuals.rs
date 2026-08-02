//! Placeholder train sprites following sim [`TrainLocation`].
//!
//! Position lerps along the current → next track tile using `progress` /
//! `ticks_for_piece`, plus fixed-timestep overstep while the sim is running.
//! Facing uses axis-aligned size (elongate along travel) and `flip_x` — no
//! runtime rotation (art direction).

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use rail_map::{tile_to_world, TILE_SIZE};
use rail_sim::{
    commands::TrainKind, ticks_for_piece, SimClock, TileCoord, Train, TrainId, TrainLocation,
    TrackNetwork, TrackPiece,
};

const TRAIN_Z: f32 = 3.0;
/// Length along travel (world units).
const ALONG: f32 = TILE_SIZE * 0.55;
/// Thickness across the rail.
const ACROSS: f32 = TILE_SIZE * 0.22;

#[derive(Component, Debug, Clone, Copy)]
pub struct TrainSprite {
    pub id: TrainId,
}

pub fn sync_train_sprites(
    mut commands: Commands,
    network: Res<TrackNetwork>,
    clock: Res<SimClock>,
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
        let pose = present_train(piece, loc, &network, overstep);
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
            commands.spawn((
                sprite,
                Transform::from_xyz(pose.x, pose.y, TRAIN_Z),
                TrainSprite { id: train.id },
            ));
        }
    }

    for (entity, sprite, _, _) in sprites.iter() {
        if !seen.contains(&sprite.id) {
            commands.entity(entity).despawn();
        }
    }
}

struct TrainPose {
    x: f32,
    y: f32,
    size: Vec2,
    flip_x: bool,
}

fn present_train(
    piece: &TrackPiece,
    loc: &TrainLocation,
    network: &TrackNetwork,
    overstep: f32,
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

    let needed = ticks_for_piece(piece.max_grade, piece.curve);
    let step = if loc.parked { 0.0 } else { overstep };
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
}
