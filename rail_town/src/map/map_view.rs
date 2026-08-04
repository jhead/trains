//! Map View (`M`) — schematic whole-map read at 4 screen texels per tile.
//!
//! Uses camera ortho scale `TILE_SIZE / 4` so the network fits without
//! fractional world zoom factors (brief 01 §2.1 / 05 §6). Click flies there
//! via [`CameraFocusRequest`] and exits; drag-build is suppressed while active.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{top_down_map_center, top_down_world_to_tile, MapGrid, TILE_SIZE};

use crate::input::{ControlAction, KeyBindings};
use crate::map::camera::{
    default_zoom_index_for, ortho_scale_for_zoom, zoom_factor_at, CameraFocusRequest,
    CameraZoomIndex, MapCamera,
};
use crate::palette::{BG1, OUTLINE};
use crate::track::TrackToolState;
use crate::ui::kit::{micro_font, text_accent, SPACE_2, STATUS_H};
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
            saved_zoom_index: default_zoom_index_for(rail_map::projection()),
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
                // Below the whole top chrome block, not a guess at its height —
                // the old literal predated the menu and health rows and left the
                // banner overlapping the status strip.
                top: Val::Px(STATUS_H + SPACE_2),
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
                Text::new("Map View  -  click to fly  -  M"),
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
    bindings: Res<KeyBindings>,
    map: Res<MapGrid>,
    mut state: ResMut<MapViewState>,
    mut q: Query<(&mut Transform, &mut Projection, &mut CameraZoomIndex), With<MapCamera>>,
    mut banner: Query<&mut Node, With<MapViewBanner>>,
    mut track: ResMut<TrackToolState>,
) {
    if !bindings.just_pressed(&keys, ControlAction::MapView) {
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
        // The plate's extent, not the world's: the Map View looks at the
        // schematic, and the schematic is laid out in tile order whichever way
        // the world is being drawn.
        let (cx, cy) = top_down_map_center(map.width, map.height);
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
    // The click lands on the *plate*, which is a plan drawing — so it resolves
    // in plate coordinates. Where the camera then has to fly is that tile's
    // place in whichever projection the world is drawn in, which is the one
    // conversion that makes the view work in both.
    let tile = top_down_world_to_tile(world.x, world.y);
    if !map.contains(tile) {
        return;
    }
    let (wx, wy) = rail_map::tile_to_world(tile);
    focus.0 = Some(Vec2::new(wx, wy));
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
    use rail_sim::ids::TileCoord;

    #[test]
    fn map_view_scale_is_four_texels_per_tile() {
        assert!((map_view_ortho_scale() - (TILE_SIZE / 4.0)).abs() < f32::EPSILON);
        assert!((TILE_SIZE / map_view_ortho_scale() - 4.0).abs() < f32::EPSILON);
    }

    /// The Map View works in both projections because the plate is a plan
    /// drawing and the *fly* is the one thing that has to cross back.
    ///
    /// A click resolves in plate coordinates — always tile order, always the
    /// same answer — and the camera is then sent to where that tile stands in
    /// whichever projection the world is drawn in. Without the second half, a
    /// click in isometric would fly the camera to a top-down coordinate and land
    /// somewhere off the map entirely.
    #[test]
    fn a_click_resolves_on_the_plate_and_flies_to_the_world() {
        let tile = TileCoord { x: 12, y: 41 };
        // The middle of that tile on the plate, which is the same point however
        // the world is drawn.
        let plate = {
            let (x, y) = rail_map::top_down_tile_to_world(tile);
            Vec2::new(x, y)
        };

        for projection in [rail_map::Projection::TopDown, rail_map::Projection::Iso] {
            let _guard = crate::map::tests::ProjectionGuard::new(projection);
            assert_eq!(
                top_down_world_to_tile(plate.x, plate.y),
                tile,
                "the plate must read the same in {projection:?}"
            );
            // Where the fly sends the camera, and what is under it when it
            // arrives — which has to be the tile that was clicked.
            let (wx, wy) = rail_map::tile_to_world(tile);
            assert_eq!(rail_map::world_to_tile(wx, wy), tile);
            if projection == rail_map::Projection::Iso {
                assert_ne!(
                    Vec2::new(wx, wy),
                    plate,
                    "isometric must not fly to the plate's own coordinate"
                );
            }
        }
    }
}
