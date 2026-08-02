//! Placeholder sprites for seeded stations and industries.

use bevy::prelude::*;
use rail_map::{tile_to_world, TILE_SIZE};
use rail_sim::{IndustryId, IndustryRegistry, StationId, StationRegistry};

#[derive(Component, Debug, Clone, Copy)]
pub struct StationSprite {
    pub id: StationId,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct IndustrySprite {
    pub id: IndustryId,
}

pub fn sync_station_industry_sprites(
    mut commands: Commands,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    existing_stations: Query<(Entity, &StationSprite)>,
    existing_industries: Query<(Entity, &IndustrySprite)>,
) {
    let station_sprites: Vec<(Entity, StationId)> = existing_stations
        .iter()
        .map(|(e, s)| (e, s.id))
        .collect();
    let industry_sprites: Vec<(Entity, IndustryId)> = existing_industries
        .iter()
        .map(|(e, s)| (e, s.id))
        .collect();

    for (entity, id) in &station_sprites {
        if stations.get(*id).is_none() {
            commands.entity(*entity).despawn();
        }
    }
    for station in stations.iter() {
        if station_sprites.iter().any(|(_, id)| *id == station.id) {
            continue;
        }
        let (wx, wy) = tile_to_world(station.tile);
        commands.spawn((
            Sprite::from_color(
                Color::srgb(0.85, 0.25, 0.22),
                Vec2::new(TILE_SIZE * 0.55, TILE_SIZE * 0.55),
            ),
            Transform::from_xyz(wx, wy, 2.0),
            StationSprite { id: station.id },
            Name::new(station.name.clone()),
        ));
    }

    for (entity, id) in &industry_sprites {
        if industries.get(*id).is_none() {
            commands.entity(*entity).despawn();
        }
    }
    for ind in industries.iter() {
        if industry_sprites.iter().any(|(_, id)| *id == ind.id) {
            continue;
        }
        let (wx, wy) = tile_to_world(ind.tile);
        let color = if ind.produces.is_some() {
            Color::srgb(0.75, 0.55, 0.2)
        } else {
            Color::srgb(0.45, 0.4, 0.7)
        };
        commands.spawn((
            Sprite::from_color(color, Vec2::new(TILE_SIZE * 0.5, TILE_SIZE * 0.5)),
            Transform::from_xyz(wx, wy, 2.0),
            IndustrySprite { id: ind.id },
            Name::new(ind.name.clone()),
        ));
    }
}
