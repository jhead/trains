//! Placeholder building blocks from [`TownDensity`].

use bevy::prelude::*;
use rail_map::{tile_to_world, TILE_SIZE};
use rail_sim::{TileCoord, TownDensity};

#[derive(Component, Debug, Clone, Copy)]
pub struct BuildingSprite {
    pub tile: TileCoord,
}

pub fn sync_building_sprites(
    mut commands: Commands,
    density: Res<TownDensity>,
    mut existing: Query<(Entity, &BuildingSprite, &mut Sprite, &mut Transform)>,
) {
    let mut seen = std::collections::HashSet::new();

    for (entity, building, mut sprite, mut transform) in existing.iter_mut() {
        let d = density.get(building.tile);
        if d < 0.08 {
            commands.entity(entity).despawn();
            continue;
        }
        seen.insert((building.tile.x, building.tile.y));
        apply_building_look(d, &mut sprite, &mut transform, building.tile);
    }

    for (tile, d) in density.iter() {
        if d < 0.08 || seen.contains(&(tile.x, tile.y)) {
            continue;
        }
        let (wx, wy) = tile_to_world(tile);
        let mut sprite = Sprite::from_color(Color::srgb(0.55, 0.48, 0.4), Vec2::splat(4.0));
        let mut transform = Transform::from_xyz(wx, wy, 0.5);
        apply_building_look(d, &mut sprite, &mut transform, tile);
        commands.spawn((sprite, transform, BuildingSprite { tile }));
    }
}

fn apply_building_look(d: f32, sprite: &mut Sprite, transform: &mut Transform, tile: TileCoord) {
    let (wx, wy) = tile_to_world(tile);
    let size = TILE_SIZE * (0.15 + 0.55 * d.clamp(0.0, 1.0));
    sprite.custom_size = Some(Vec2::new(size * 0.7, size));
    let warmth = 0.35 + 0.45 * d;
    sprite.color = Color::srgb(0.45 + warmth * 0.25, 0.38 + warmth * 0.15, 0.32);
    transform.translation.x = wx;
    transform.translation.y = wy;
    transform.translation.z = 0.5;
}
