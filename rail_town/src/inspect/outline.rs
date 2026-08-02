//! 1-texel `railS` selection outline in the world.

use bevy::prelude::*;
use rail_map::TILE_SIZE;
use rail_sim::{IndustryRegistry, StationRegistry, TrackNetwork};

use crate::palette::RAIL_S;
use crate::stations::{IndustrySprite, StationSprite};
use crate::track::TrackSprite;
use crate::trains::TrainSprite;
use crate::town::PeepSprite;

use super::pick::Selectable;
use super::selection::Selection;

/// 1 world unit ≈ 1 texel at 1×; expand sprite by 2 so a 1-texel ring shows.
const OUTLINE_PAD: f32 = 2.0;

#[derive(Component)]
pub struct SelectionOutline;

pub fn setup_selection_outline(mut commands: Commands) {
    commands.spawn((
        SelectionOutline,
        Sprite::from_color(RAIL_S, Vec2::splat(TILE_SIZE * 0.6 + OUTLINE_PAD)),
        Transform::from_xyz(0.0, 0.0, 0.0),
        Visibility::Hidden,
    ));
}

pub fn sync_selection_outline(
    selection: Res<Selection>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    network: Res<TrackNetwork>,
    peep_sprites: Query<(&PeepSprite, &Transform, &Sprite), Without<SelectionOutline>>,
    train_sprites: Query<(&TrainSprite, &Transform, &Sprite), Without<SelectionOutline>>,
    station_sprites: Query<(&StationSprite, &Transform, &Sprite), Without<SelectionOutline>>,
    industry_sprites: Query<(&IndustrySprite, &Transform, &Sprite), Without<SelectionOutline>>,
    track_sprites: Query<(&TrackSprite, &Transform, &Sprite), Without<SelectionOutline>>,
    mut outline: Query<
        (&mut Transform, &mut Sprite, &mut Visibility),
        With<SelectionOutline>,
    >,
) {
    let Ok((mut tf, mut sprite, mut vis)) = outline.single_mut() else {
        return;
    };

    let Some(sel) = selection.0 else {
        *vis = Visibility::Hidden;
        return;
    };

    let pose = match sel {
        Selectable::Peep(id) => peep_sprites.iter().find(|(s, _, _)| s.id == id).map(|(_, t, s)| {
            (
                t.translation,
                s.custom_size.unwrap_or(Vec2::splat(TILE_SIZE * 0.28)),
            )
        }),
        Selectable::Train(id) => train_sprites.iter().find(|(s, _, _)| s.id == id).map(|(_, t, s)| {
            (
                t.translation,
                s.custom_size
                    .unwrap_or(Vec2::new(TILE_SIZE * 0.55, TILE_SIZE * 0.22)),
            )
        }),
        Selectable::Station(id) => station_sprites
            .iter()
            .find(|(s, _, _)| s.id == id)
            .map(|(_, t, s)| {
                (
                    t.translation,
                    s.custom_size.unwrap_or(Vec2::splat(TILE_SIZE * 0.55)),
                )
            })
            .or_else(|| {
                stations.get(id).map(|st| {
                    let (x, y) = rail_map::tile_to_world(st.tile);
                    (Vec3::new(x, y, 2.0), Vec2::splat(TILE_SIZE * 0.55))
                })
            }),
        Selectable::Industry(id) => industry_sprites
            .iter()
            .find(|(s, _, _)| s.id == id)
            .map(|(_, t, s)| {
                (
                    t.translation,
                    s.custom_size.unwrap_or(Vec2::splat(TILE_SIZE * 0.5)),
                )
            })
            .or_else(|| {
                industries.get(id).map(|ind| {
                    let (x, y) = rail_map::tile_to_world(ind.tile);
                    (Vec3::new(x, y, 2.0), Vec2::splat(TILE_SIZE * 0.5))
                })
            }),
        Selectable::Track(id) => track_sprites
            .iter()
            .find(|(s, _, _)| s.id == id)
            .map(|(_, t, s)| {
                (
                    t.translation,
                    s.custom_size
                        .unwrap_or(Vec2::new(TILE_SIZE * 0.7, TILE_SIZE * 0.35)),
                )
            })
            .or_else(|| {
                network.piece(id).map(|p| {
                    let (x, y) = rail_map::tile_to_world(p.tile);
                    (
                        Vec3::new(x, y, 1.0),
                        Vec2::new(TILE_SIZE * 0.7, TILE_SIZE * 0.35),
                    )
                })
            }),
    };

    let Some((pos, size)) = pose else {
        *vis = Visibility::Hidden;
        return;
    };

    // Larger railS plate just behind the sprite → 1-texel bright ring.
    *vis = Visibility::Visible;
    tf.translation.x = pos.x;
    tf.translation.y = pos.y;
    tf.translation.z = pos.z - 0.05;
    sprite.color = RAIL_S;
    sprite.custom_size = Some(size + Vec2::splat(OUTLINE_PAD));
}
