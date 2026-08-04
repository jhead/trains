//! Live ghost sprites for proposed build / demolish tiles.
//!
//! **Iso prototype**: every footprint here is a tinted tile diamond rather than
//! a square, because the ghost is how the player reads which tile the cursor
//! resolved to — a square over a diamond grid says nothing. Sizes are the same
//! fractions of a tile as before, so the vocabulary (hover faint, anchor small
//! and solid, place bright, invalid `WARN`) is unchanged.

use bevy::prelude::*;
use rail_map::tile_to_world;
use rail_sim::ids::TileCoord;

use crate::map::IsoDiamond;

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
    diamond: Option<Res<IsoDiamond>>,
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
    let Some(diamond) = diamond else {
        return;
    };

    if let Some(tile) = state.hover_tile {
        spawn_tile(
            &mut commands,
            &diamond,
            tile,
            HI.with_alpha(0.22),
            0.98,
            2.0,
            HoverHighlight,
        );
    }

    if let Some(anchor) = state.anchor {
        spawn_tile(
            &mut commands,
            &diamond,
            anchor,
            HI.with_alpha(0.55),
            0.35,
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
                    (base, if is_bridge { 0.75 } else { 0.6 })
                }
                TileGhostKind::Existing => (HI.with_alpha(0.2), 0.6),
                TileGhostKind::Invalid => (WARN.with_alpha(0.6), 0.85),
            };
            spawn_tile(
                &mut commands,
                &diamond,
                ghost.tile,
                color,
                size,
                2.5,
                GhostSprite,
            );
        }
    }

    if let Some(preview) = &state.demolish_preview {
        for &tile in &preview.tiles {
            spawn_tile(
                &mut commands,
                &diamond,
                tile,
                WARN.with_alpha(0.5),
                0.6,
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
                        &diamond,
                        tip,
                        WARN.with_alpha(0.25),
                        0.5,
                        2.4,
                        GhostSprite,
                    );
                }
            }
        }
    }
}

/// `scale` is the fraction of a tile the diamond covers.
fn spawn_tile<M: Component>(
    commands: &mut Commands,
    diamond: &IsoDiamond,
    tile: TileCoord,
    color: Color,
    scale: f32,
    z: f32,
    marker: M,
) {
    let (wx, wy) = tile_to_world(tile);
    commands.spawn((
        marker,
        diamond.sprite(color, scale),
        Transform::from_xyz(wx, wy, z),
    ));
}
