//! Placeholder track sprites; bake-on-edit via [`TrackEdit`] messages.
//!
//! Discrete placeholder bars (not continuous Bresenham). Orientation uses the
//! first linked 8-dir when present so straight runs read clearly.

use bevy::prelude::*;
use rail_map::{tile_to_world, TILE_SIZE};
use rail_sim::track::DIR8;
use rail_sim::{TrackEdit, TrackId, TrackNetwork};

/// Marker on a track placeholder sprite.
#[derive(Component, Debug, Clone, Copy)]
pub struct TrackSprite {
    pub id: TrackId,
}

pub fn apply_track_sprites(
    mut commands: Commands,
    mut edits: MessageReader<TrackEdit>,
    network: Res<TrackNetwork>,
    existing: Query<(Entity, &TrackSprite)>,
) {
    for edit in edits.read() {
        match *edit {
            TrackEdit::Placed {
                id,
                tile,
                is_bridge,
                ..
            } => {
                let (wx, wy) = tile_to_world(tile);
                let color = if is_bridge {
                    Color::srgb(0.55, 0.42, 0.28)
                } else {
                    Color::srgb(0.22, 0.22, 0.24)
                };
                let size = Vec2::new(TILE_SIZE * 0.7, TILE_SIZE * 0.2);
                let mut transform = Transform::from_xyz(wx, wy, 1.0);
                if let Some(piece) = network.piece(id) {
                    if let Some(dir) = (0..8).find(|&i| piece.links.has(i)) {
                        let (dx, dy) = DIR8[dir];
                        let angle = (dy as f32).atan2(dx as f32);
                        transform.rotation = Quat::from_rotation_z(angle);
                    }
                }
                commands.spawn((
                    Sprite::from_color(color, size),
                    transform,
                    TrackSprite { id },
                ));
            }
            TrackEdit::Removed { id, .. } => {
                for (entity, sprite) in existing.iter() {
                    if sprite.id == id {
                        commands.entity(entity).despawn();
                    }
                }
            }
            TrackEdit::Failed { .. } => {}
        }
    }
}
