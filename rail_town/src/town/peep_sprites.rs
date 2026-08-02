//! Placeholder peep dots near their home / station tiles.

use bevy::prelude::*;
use rail_map::{tile_to_world, TILE_SIZE};
use rail_sim::{Mood, Peep, PeepId, WaitingAtStation};

#[derive(Component, Debug, Clone, Copy)]
pub struct PeepSprite {
    pub id: PeepId,
}

pub fn sync_peep_sprites(
    mut commands: Commands,
    peeps: Query<(&Peep, &WaitingAtStation)>,
    mut sprites: Query<(Entity, &PeepSprite, &mut Sprite, &mut Transform)>,
) {
    let mut live = std::collections::HashMap::new();
    for (peep, waiting) in &peeps {
        live.insert(peep.id, (peep.mood, peep.home, waiting.station.0));
    }

    for (entity, marker, mut sprite, mut transform) in sprites.iter_mut() {
        if let Some(&(mood, home, station_n)) = live.get(&marker.id) {
            sprite.color = mood_color(mood);
            sprite.custom_size = Some(Vec2::splat(TILE_SIZE * 0.28));
            let (wx, wy) = tile_to_world(home);
            let ox = ((marker.id.0 % 3) as f32 - 1.0) * (TILE_SIZE * 0.22);
            let oy = ((station_n % 3) as f32 - 1.0) * (TILE_SIZE * 0.18);
            transform.translation = Vec3::new(wx + ox, wy + oy, 2.0);
            live.remove(&marker.id);
        } else {
            commands.entity(entity).despawn();
        }
    }

    for (id, (mood, home, station_n)) in live {
        let (wx, wy) = tile_to_world(home);
        let ox = ((id.0 % 3) as f32 - 1.0) * (TILE_SIZE * 0.22);
        let oy = ((station_n % 3) as f32 - 1.0) * (TILE_SIZE * 0.18);
        commands.spawn((
            Sprite::from_color(mood_color(mood), Vec2::splat(TILE_SIZE * 0.28)),
            Transform::from_xyz(wx + ox, wy + oy, 2.0),
            PeepSprite { id },
        ));
    }
}

fn mood_color(mood: Mood) -> Color {
    match mood {
        Mood::Content => Color::srgb(0.35, 0.75, 0.45),
        Mood::Uneasy => Color::srgb(0.85, 0.7, 0.25),
        Mood::Frustrated => Color::srgb(0.85, 0.35, 0.3),
    }
}
