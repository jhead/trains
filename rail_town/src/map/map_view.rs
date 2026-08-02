//! Map View (`M`) — schematic whole-map read at 4 screen texels per tile.
//!
//! Uses camera ortho scale `TILE_SIZE / 4` so the network fits without
//! fractional world zoom factors (brief 01 §2.1 / 05 §6). Click flies there
//! via [`CameraFocusRequest`] and exits; drag-build is suppressed while active.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{map_center_world, world_to_tile, MapGrid, TILE_SIZE};

use crate::map::camera::{
    ortho_scale_for_zoom, zoom_factor_at, CameraFocusRequest, CameraZoomIndex, MapCamera,
    DEFAULT_ZOOM_INDEX,
};
use crate::palette::{BG1, OUTLINE};
use crate::track::TrackToolState;
use crate::ui::kit::{micro_font, text_accent, SPACE_2, SPACE_3};
use crate::ui::UiBlocksWorld;

/// Screen texels per map tile in Map View (design: 4).
pub const MAP_VIEW_TEXELS_PER_TILE: f32 = 4.0;

/// Ortho scale so each tile is [`MAP_VIEW_TEXELS_PER_TILE`] screen pixels.
#[inline]
pub fn map_view_ortho_scale() -> f32 {
    TILE_SIZE / MAP_VIEW_TEXELS_PER_TILE
}

#[derive(Resource, Debug, Clone)]
pub struct MapViewState {
    pub active: bool,
    saved_zoom_index: usize,
    saved_translation: Vec3,
}

impl Default for MapViewState {
    fn default() -> Self {
        Self {
            active: false,
            saved_zoom_index: DEFAULT_ZOOM_INDEX,
            saved_translation: Vec3::ZERO,
        }
    }
}

#[derive(Component)]
pub(crate) struct MapViewBanner;

pub fn setup_map_view_banner(mut commands: Commands) {
    commands
        .spawn((
            MapViewBanner,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(SPACE_3 + 28.0),
                left: Val::Percent(50.0),
                margin: UiRect::left(Val::Px(-110.0)),
                padding: UiRect::axes(Val::Px(SPACE_2), Val::Px(SPACE_2)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::ZERO,
                display: Display::None,
                ..default()
            },
            BackgroundColor(BG1),
            BorderColor::all(OUTLINE),
            ZIndex(8),
        ))
        .with_children(|p| {
            p.spawn((
                Text::new("Map View  ·  click to fly  ·  M"),
                micro_font(),
                text_accent(),
            ));
        });
}

fn set_banner(banner: &mut Query<&mut Node, With<MapViewBanner>>, active: bool) {
    if let Ok(mut node) = banner.single_mut() {
        node.display = if active {
            Display::Flex
        } else {
            Display::None
        };
    }
}

pub fn toggle_map_view(
    keys: Res<ButtonInput<KeyCode>>,
    map: Res<MapGrid>,
    mut state: ResMut<MapViewState>,
    mut q: Query<(&mut Transform, &mut Projection, &mut CameraZoomIndex), With<MapCamera>>,
    mut banner: Query<&mut Node, With<MapViewBanner>>,
    mut track: ResMut<TrackToolState>,
) {
    if !keys.just_pressed(KeyCode::KeyM) {
        return;
    }
    let Ok((mut transform, mut projection, mut zoom_index)) = q.single_mut() else {
        return;
    };
    let Projection::Orthographic(ortho) = projection.as_mut() else {
        return;
    };

    if state.active {
        exit_map_view(
            &mut state,
            &mut transform,
            &mut *ortho,
            &mut zoom_index,
            &mut track,
            true,
        );
    } else {
        state.saved_zoom_index = zoom_index.0;
        state.saved_translation = transform.translation;
        state.active = true;
        let (cx, cy) = map_center_world(map.width, map.height);
        transform.translation.x = cx.round();
        transform.translation.y = cy.round();
        ortho.scale = map_view_ortho_scale();
        track.suppress_build_click = true;
        track.drag = None;
        track.build_preview = None;
        track.demolish_preview = None;
    }

    set_banner(&mut banner, state.active);
}

fn exit_map_view(
    state: &mut MapViewState,
    transform: &mut Transform,
    ortho: &mut OrthographicProjection,
    zoom_index: &mut CameraZoomIndex,
    track: &mut TrackToolState,
    restore_translation: bool,
) {
    state.active = false;
    zoom_index.0 = state.saved_zoom_index;
    ortho.scale = ortho_scale_for_zoom(zoom_factor_at(zoom_index.0));
    if restore_translation {
        transform.translation = state.saved_translation;
        transform.translation.x = transform.translation.x.round();
        transform.translation.y = transform.translation.y.round();
    }
    track.suppress_build_click = false;
}

/// While Map View is on, keep the schematic ortho scale (wheel / +/- must not stick).
pub fn block_zoom_in_map_view(
    state: Res<MapViewState>,
    mut q: Query<&mut Projection, With<MapCamera>>,
) {
    if !state.active {
        return;
    }
    let Ok(mut projection) = q.single_mut() else {
        return;
    };
    let Projection::Orthographic(ortho) = projection.as_mut() else {
        return;
    };
    ortho.scale = map_view_ortho_scale();
}

pub fn map_view_click_fly(
    mouse: Res<ButtonInput<MouseButton>>,
    state: Res<MapViewState>,
    ui_blocks: Res<UiBlocksWorld>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MapCamera>>,
    map: Res<MapGrid>,
    mut focus: ResMut<CameraFocusRequest>,
) {
    if !state.active || ui_blocks.0 {
        return;
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((camera, cam_gt)) = camera_q.single() else {
        return;
    };
    let Ok(world) = camera.viewport_to_world_2d(cam_gt, cursor) else {
        return;
    };
    let tile = world_to_tile(world.x, world.y);
    if !map.contains(tile) {
        return;
    }
    focus.0 = Some(Vec2::new(world.x, world.y));
}

/// If a focus request arrives while Map View is open, restore play zoom first.
pub fn exit_map_view_before_focus(
    focus: Res<CameraFocusRequest>,
    mut state: ResMut<MapViewState>,
    mut q: Query<(&mut Transform, &mut Projection, &mut CameraZoomIndex), With<MapCamera>>,
    mut banner: Query<&mut Node, With<MapViewBanner>>,
    mut track: ResMut<TrackToolState>,
) {
    if !state.active || focus.0.is_none() {
        return;
    }
    let Ok((mut transform, mut projection, mut zoom_index)) = q.single_mut() else {
        return;
    };
    let Projection::Orthographic(ortho) = projection.as_mut() else {
        return;
    };
    exit_map_view(
        &mut state,
        &mut transform,
        &mut *ortho,
        &mut zoom_index,
        &mut track,
        false,
    );
    set_banner(&mut banner, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_view_scale_is_four_texels_per_tile() {
        assert!((map_view_ortho_scale() - (TILE_SIZE / 4.0)).abs() < f32::EPSILON);
        assert!((TILE_SIZE / map_view_ortho_scale() - 4.0).abs() < f32::EPSILON);
    }
}
