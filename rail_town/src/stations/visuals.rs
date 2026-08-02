//! Placeholder sprites for stations and industries.
//!
//! Newly revealed demand (open opportunities) uses a brighter amber tint so
//! unserved anchors read as the next thing to reach. Player-built platforms
//! read their **tier** off the registry: a halt is a chip on the line, an
//! interchange is a block you cannot miss.

use bevy::prelude::*;
use rail_map::{tile_to_world, TILE_SIZE};
use rail_sim::stations::StationTier;
use rail_sim::{DemandSpawner, IndustryId, IndustryRegistry, StationId, StationRegistry};

#[derive(Component, Debug, Clone, Copy)]
pub struct StationSprite {
    pub id: StationId,
}

/// Sprite footprint for a tier, as a fraction of the tile.
///
/// Scales with platform count so the four grades are told apart at a glance.
pub fn tier_sprite_scale(tier: StationTier) -> f32 {
    match tier {
        StationTier::Halt => 0.4,
        StationTier::Station => 0.55,
        StationTier::Terminus => 0.65,
        StationTier::Interchange => 0.75,
    }
}

#[derive(Component, Debug, Clone, Copy)]
pub struct IndustrySprite {
    pub id: IndustryId,
}

/// Marker: this sprite is a session-spawned opportunity still off-network.
#[derive(Component, Debug, Clone, Copy)]
pub struct NewDemandMarker;

pub fn sync_station_industry_sprites(
    mut commands: Commands,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    demand: Res<DemandSpawner>,
    existing_stations: Query<(Entity, &StationSprite, Option<&NewDemandMarker>)>,
    existing_industries: Query<(Entity, &IndustrySprite, Option<&NewDemandMarker>)>,
    mut sprites: Query<&mut Sprite>,
) {
    let _perf = crate::overlays::perf::scope("sync_station_industry_sprites");
    let station_sprites: Vec<(Entity, StationId, bool)> = existing_stations
        .iter()
        .map(|(e, s, m)| (e, s.id, m.is_some()))
        .collect();
    let industry_sprites: Vec<(Entity, IndustryId, bool)> = existing_industries
        .iter()
        .map(|(e, s, m)| (e, s.id, m.is_some()))
        .collect();

    for (entity, id, _) in &station_sprites {
        if stations.get(*id).is_none() {
            commands.entity(*entity).despawn();
        }
    }
    for station in stations.iter() {
        let is_new = demand.is_open_station(station.id);
        // Unserved anchors keep their own read; built platforms scale by tier.
        let color = if is_new {
            Color::srgb(0.95, 0.72, 0.2)
        } else {
            Color::srgb(0.85, 0.25, 0.22)
        };
        let scale = if is_new {
            0.65
        } else {
            tier_sprite_scale(station.tier)
        };
        let size = Vec2::splat(TILE_SIZE * scale);

        if let Some((entity, _, was_new)) = station_sprites
            .iter()
            .find(|(_, id, _)| *id == station.id)
        {
            // Tint / footprint can change when demand connects or the stop is upgraded.
            if let Ok(mut sprite) = sprites.get_mut(*entity) {
                sprite.color = color;
                sprite.custom_size = Some(size);
            }
            if is_new && !was_new {
                commands.entity(*entity).insert(NewDemandMarker);
            } else if !is_new && *was_new {
                commands.entity(*entity).remove::<NewDemandMarker>();
            }
            // Follow the registry's tile, always. A world swap reissues ids from
            // one, so a reused sprite can be matched to a station standing
            // somewhere else entirely -- which left the old world's platforms
            // floating in the void beyond the new map's edge.
            let (wx, wy) = tile_to_world(station.tile);
            commands
                .entity(*entity)
                .insert(Transform::from_xyz(wx, wy, 2.0));
            continue;
        }
        let (wx, wy) = tile_to_world(station.tile);
        let mut e = commands.spawn((
            Sprite::from_color(color, size),
            Transform::from_xyz(wx, wy, 2.0),
            StationSprite { id: station.id },
            Name::new(station.name.clone()),
        ));
        if is_new {
            e.insert(NewDemandMarker);
        }
    }

    for (entity, id, _) in &industry_sprites {
        if industries.get(*id).is_none() {
            commands.entity(*entity).despawn();
        }
    }
    for ind in industries.iter() {
        let is_new = demand.is_open_industry(ind.id);
        let base = if ind.produces.is_some() {
            Color::srgb(0.75, 0.55, 0.2)
        } else {
            Color::srgb(0.45, 0.4, 0.7)
        };
        let color = if is_new {
            Color::srgb(0.95, 0.78, 0.35)
        } else {
            base
        };
        if let Some((entity, _, was_new)) = industry_sprites
            .iter()
            .find(|(_, id, _)| *id == ind.id)
        {
            if let Ok(mut sprite) = sprites.get_mut(*entity) {
                sprite.color = color;
            }
            if is_new && !was_new {
                commands.entity(*entity).insert(NewDemandMarker);
            } else if !is_new && *was_new {
                commands.entity(*entity).remove::<NewDemandMarker>();
            }
            // See the station branch: ids are reissued on a world swap, so a
            // reused sprite must be moved to where the registry says it is.
            let (wx, wy) = tile_to_world(ind.tile);
            commands
                .entity(*entity)
                .insert(Transform::from_xyz(wx, wy, 2.0));
            continue;
        }
        let (wx, wy) = tile_to_world(ind.tile);
        let size = if is_new {
            Vec2::new(TILE_SIZE * 0.6, TILE_SIZE * 0.6)
        } else {
            Vec2::new(TILE_SIZE * 0.5, TILE_SIZE * 0.5)
        };
        let mut e = commands.spawn((
            Sprite::from_color(color, size),
            Transform::from_xyz(wx, wy, 2.0),
            IndustrySprite { id: ind.id },
            Name::new(ind.name.clone()),
        ));
        if is_new {
            e.insert(NewDemandMarker);
        }
    }
}
