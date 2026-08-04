//! Live ghost sprites for proposed build / demolish tiles.
//!
//! The ghost is how the player reads which tile the cursor resolved to, so its
//! footprint has to be the shape of a tile — a square from above, a tinted
//! diamond in isometric, because a square over a diamond grid says nothing.
//! Each call site therefore names both: the fraction of a diamond it wants and
//! the exact top-down rectangle it always drew. The vocabulary is the same
//! either way (hover faint, anchor small and solid, place bright, invalid
//! `WARN`), and neither view is a compromise for the other.

use bevy::prelude::*;
use rail_map::{tile_to_world, TILE_SIZE};
use rail_sim::ids::TileCoord;

use crate::map::TileMark;

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
    mark: Option<Res<TileMark>>,
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

    // Headless tests boot without the terrain plugin, so there is no mask to
    // tint; a ghost is presentation only and simply does not draw.
    let Some(mark) = mark else {
        return;
    };

    if let Some(tile) = state.hover_tile {
        spawn_tile(
            &mut commands,
            &mark,
            tile,
            HI.with_alpha(0.22),
            0.98,
            Vec2::splat(TILE_SIZE * 0.98),
            2.0,
            HoverHighlight,
        );
    }

    if let Some(anchor) = state.anchor {
        spawn_tile(
            &mut commands,
            &mark,
            anchor,
            HI.with_alpha(0.55),
            0.35,
            Vec2::splat(TILE_SIZE * 0.35),
            2.2,
            AnchorMarker,
        );
    }

    if let Some(preview) = &state.build_preview {
        let whole_warn = preview.reject.is_some();
        for ghost in &preview.tiles {
            let (color, scale, flat) = match ghost.kind {
                TileGhostKind::Place { is_bridge, .. } => {
                    let base = if ghost.valid && !whole_warn {
                        HI.with_alpha(0.55)
                    } else {
                        WARN.with_alpha(0.55)
                    };
                    if is_bridge {
                        (base, 0.75, Vec2::new(TILE_SIZE * 0.75, TILE_SIZE * 0.28))
                    } else {
                        (base, 0.6, RAIL_BAR)
                    }
                }
                TileGhostKind::Existing => (HI.with_alpha(0.2), 0.6, RAIL_BAR),
                TileGhostKind::Invalid => (
                    WARN.with_alpha(0.6),
                    0.85,
                    Vec2::splat(TILE_SIZE * 0.85),
                ),
            };
            spawn_tile(
                &mut commands,
                &mark,
                ghost.tile,
                color,
                scale,
                flat,
                2.5,
                GhostSprite,
            );
        }
    }

    if let Some(preview) = &state.demolish_preview {
        for &tile in &preview.tiles {
            spawn_tile(
                &mut commands,
                &mark,
                tile,
                WARN.with_alpha(0.5),
                0.6,
                RAIL_BAR,
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
                        &mark,
                        tip,
                        WARN.with_alpha(0.25),
                        0.5,
                        Vec2::splat(TILE_SIZE * 0.5),
                        2.4,
                        GhostSprite,
                    );
                }
            }
        }
    }
}

/// The bar a track ghost draws from above: a rail's width along the tile.
const RAIL_BAR: Vec2 = Vec2::new(TILE_SIZE * 0.7, TILE_SIZE * 0.2);

/// `scale` is the fraction of a diamond the isometric mark covers; `flat` is
/// the literal size the top-down one is drawn at.
fn spawn_tile<M: Component>(
    commands: &mut Commands,
    mark: &TileMark,
    tile: TileCoord,
    color: Color,
    scale: f32,
    flat: Vec2,
    z: f32,
    marker: M,
) {
    let (wx, wy) = tile_to_world(tile);
    commands.spawn((
        marker,
        mark.sprite(color, scale, flat),
        Transform::from_xyz(wx, wy, z),
    ));
}
