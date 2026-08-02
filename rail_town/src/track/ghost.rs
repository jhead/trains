//! Live ghost sprites for proposed build / demolish tiles.

use bevy::prelude::*;
use rail_map::{tile_to_world, TILE_SIZE};
use rail_sim::ids::TileCoord;

use super::preview::TileGhostKind;
use super::tools::{DragKind, TrackToolState};

const HI: Color = Color::srgb(0xf2 as f32 / 255.0, 0xc1 as f32 / 255.0, 0x4e as f32 / 255.0);
const WARN: Color = Color::srgb(0xe8 as f32 / 255.0, 0x62 as f32 / 255.0, 0x4a as f32 / 255.0);

#[derive(Component)]
pub(crate) struct GhostSprite;

#[derive(Component)]
pub(crate) struct HoverHighlight;

#[derive(Component)]
pub(crate) struct AnchorMarker;

pub fn sync_track_ghosts(
    mut commands: Commands,
    state: Res<TrackToolState>,
    ghosts: Query<Entity, With<GhostSprite>>,
    hovers: Query<Entity, With<HoverHighlight>>,
    anchors: Query<Entity, With<AnchorMarker>>,
) {
    for entity in &ghosts {
        commands.entity(entity).despawn();
    }
    for entity in &hovers {
        commands.entity(entity).despawn();
    }
    for entity in &anchors {
        commands.entity(entity).despawn();
    }

    if let Some(tile) = state.hover_tile {
        spawn_tile(
            &mut commands,
            tile,
            HI.with_alpha(0.22),
            Vec2::splat(TILE_SIZE * 0.98),
            2.0,
            HoverHighlight,
        );
    }

    if let Some(anchor) = state.anchor {
        spawn_tile(
            &mut commands,
            anchor,
            HI.with_alpha(0.55),
            Vec2::splat(TILE_SIZE * 0.35),
            2.2,
            AnchorMarker,
        );
    }

    if let Some(preview) = &state.build_preview {
        let whole_warn = preview.reject.is_some();
        for ghost in &preview.tiles {
            let (color, size) = match ghost.kind {
                TileGhostKind::Place { is_bridge, .. } => {
                    let base = if ghost.valid && !whole_warn {
                        HI.with_alpha(0.55)
                    } else {
                        WARN.with_alpha(0.55)
                    };
                    let size = if is_bridge {
                        Vec2::new(TILE_SIZE * 0.75, TILE_SIZE * 0.28)
                    } else {
                        Vec2::new(TILE_SIZE * 0.7, TILE_SIZE * 0.2)
                    };
                    (base, size)
                }
                TileGhostKind::Existing => (HI.with_alpha(0.2), Vec2::new(TILE_SIZE * 0.7, TILE_SIZE * 0.2)),
                TileGhostKind::Invalid => (
                    WARN.with_alpha(0.6),
                    Vec2::splat(TILE_SIZE * 0.85),
                ),
            };
            spawn_tile(&mut commands, ghost.tile, color, size, 2.5, GhostSprite);
        }
    }

    if let Some(preview) = &state.demolish_preview {
        for &tile in &preview.tiles {
            spawn_tile(
                &mut commands,
                tile,
                WARN.with_alpha(0.5),
                Vec2::new(TILE_SIZE * 0.7, TILE_SIZE * 0.2),
                2.6,
                GhostSprite,
            );
        }
        // While demolish-dragging, also mark empty tip so the path reads.
        if state.drag == Some(DragKind::Demolish) {
            if let Some(tip) = state.hover_tile {
                if !preview.tiles.contains(&tip) {
                    spawn_tile(
                        &mut commands,
                        tip,
                        WARN.with_alpha(0.25),
                        Vec2::splat(TILE_SIZE * 0.5),
                        2.4,
                        GhostSprite,
                    );
                }
            }
        }
    }
}

fn spawn_tile<M: Component>(
    commands: &mut Commands,
    tile: TileCoord,
    color: Color,
    size: Vec2,
    z: f32,
    marker: M,
) {
    let (wx, wy) = tile_to_world(tile);
    commands.spawn((
        marker,
        Sprite::from_color(color, size),
        Transform::from_xyz(wx, wy, z),
    ));
}
