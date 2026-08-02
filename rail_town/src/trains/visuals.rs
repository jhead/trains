//! Placeholder train sprites following sim [`TrainLocation`].

use bevy::prelude::*;
use rail_map::{tile_to_world, TILE_SIZE};
use rail_sim::{commands::TrainKind, Train, TrainId, TrainLocation, TrackNetwork};

#[derive(Component, Debug, Clone, Copy)]
pub struct TrainSprite {
    pub id: TrainId,
}

pub fn sync_train_sprites(
    mut commands: Commands,
    network: Res<TrackNetwork>,
    trains: Query<(&Train, &TrainLocation)>,
    mut sprites: Query<(Entity, &TrainSprite, &mut Transform)>,
) {
    let mut seen = Vec::new();
    for (train, loc) in trains.iter() {
        seen.push(train.id);
        let Some(piece) = network.piece(loc.track) else {
            continue;
        };
        let (wx, wy) = tile_to_world(piece.tile);
        let color = match train.kind {
            TrainKind::Transit => Color::srgb(0.2, 0.55, 0.9),
            TrainKind::Transport => Color::srgb(0.9, 0.65, 0.15),
        };

        if let Some((_, _, mut tf)) = sprites.iter_mut().find(|(_, s, _)| s.id == train.id) {
            tf.translation.x = wx;
            tf.translation.y = wy;
            tf.translation.z = 3.0;
        } else {
            commands.spawn((
                Sprite::from_color(color, Vec2::new(TILE_SIZE * 0.45, TILE_SIZE * 0.3)),
                Transform::from_xyz(wx, wy, 3.0),
                TrainSprite { id: train.id },
            ));
        }
    }

    for (entity, sprite, _) in sprites.iter() {
        if !seen.contains(&sprite.id) {
            commands.entity(entity).despawn();
        }
    }
}
