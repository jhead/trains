//! Drag-to-build / right-drag demolish → sim [`CommandBuffer`].
//!
//! ## Build
//! Press → drag → release. Live ghost every frame. Default snaps to ortho/45°;
//! Shift requires an exact straight; Ctrl places a single tile. After a
//! successful commit the endpoint stays as the continuous-build anchor.
//!
//! ## Demolish
//! Right-drag (in Build or Demolish tool) refunds along the snapped path.
//! Esc clears the build anchor.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{world_to_tile, MapGrid};
use rail_sim::commands::{AutoFillTrack, Demolish, PlaceTrack};
use rail_sim::ids::TileCoord;
use rail_sim::{
    CommandBuffer, CommandKind, Money, TrackNetwork, TrackTerrain, GROUND_LAYER,
};

use crate::map::MapCamera;
use crate::ui::UiBlocksWorld;

use super::feedback::{push_reject, BuildFeedback};
use super::preview::{preview_build, preview_demolish, BuildPreview, DemolishPreview};
use super::propose::PathMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Resource)]
pub enum BuildTool {
    #[default]
    Build,
    Demolish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragKind {
    Build,
    Demolish,
}

#[derive(Debug, Clone, Default, Resource)]
pub struct TrackToolState {
    pub tool: BuildTool,
    /// Continuous-build / in-progress anchor.
    pub anchor: Option<TileCoord>,
    pub drag: Option<DragKind>,
    pub drag_origin: Option<TileCoord>,
    pub hover_tile: Option<TileCoord>,
    pub path_mode: PathMode,
    pub build_preview: Option<BuildPreview>,
    pub demolish_preview: Option<DemolishPreview>,
    /// When true (train place mode), ignore build/demolish pointer input.
    pub suppress_build_click: bool,
}

/// Cursor tile under the map camera, if on-map.
pub fn cursor_tile(
    windows: &Query<&Window, With<PrimaryWindow>>,
    camera_q: &Query<(&Camera, &GlobalTransform), With<MapCamera>>,
    map: &MapGrid,
) -> Option<TileCoord> {
    let window = windows.single().ok()?;
    let cursor = window.cursor_position()?;
    let (camera, cam_transform) = camera_q.single().ok()?;
    let world = camera.viewport_to_world_2d(cam_transform, cursor).ok()?;
    let tile = world_to_tile(world.x, world.y);
    if map.contains(tile) {
        Some(tile)
    } else {
        None
    }
}

fn path_mode_from_keys(keys: &ButtonInput<KeyCode>) -> PathMode {
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    if ctrl {
        PathMode::SingleTile
    } else if shift {
        PathMode::ExactStraight
    } else {
        PathMode::Autofill
    }
}

pub fn track_tool_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MapCamera>>,
    map: Res<MapGrid>,
    network: Res<TrackNetwork>,
    terrain: Option<Res<TrackTerrain>>,
    money: Res<Money>,
    mut buffer: ResMut<CommandBuffer>,
    mut state: ResMut<TrackToolState>,
    mut feedback: ResMut<BuildFeedback>,
    ui_blocks: Res<UiBlocksWorld>,
) {
    if keys.just_pressed(KeyCode::KeyB) {
        state.tool = BuildTool::Build;
        state.suppress_build_click = false;
    }
    if keys.just_pressed(KeyCode::KeyX) {
        state.tool = BuildTool::Demolish;
        state.suppress_build_click = false;
        state.drag = None;
    }
    if keys.just_pressed(KeyCode::Escape) {
        state.anchor = None;
        state.drag = None;
        state.drag_origin = None;
        state.build_preview = None;
        state.demolish_preview = None;
    }

    let hover = cursor_tile(&windows, &camera_q, &map);
    state.hover_tile = hover;
    state.path_mode = path_mode_from_keys(&keys);

    if state.suppress_build_click {
        state.drag = None;
        state.build_preview = None;
        state.demolish_preview = None;
        return;
    }

    // Don't start a new drag through UI chrome.
    if ui_blocks.0 && state.drag.is_none() {
        state.build_preview = None;
        state.demolish_preview = None;
        return;
    }

    let Some(terrain) = terrain else {
        return;
    };

    if mouse.just_pressed(MouseButton::Left) {
        if let Some(tile) = hover {
            match state.tool {
                BuildTool::Build => {
                    let origin = if state.path_mode == PathMode::SingleTile {
                        tile
                    } else {
                        state.anchor.unwrap_or(tile)
                    };
                    state.anchor = Some(origin);
                    state.drag = Some(DragKind::Build);
                    state.drag_origin = Some(origin);
                }
                BuildTool::Demolish => {
                    state.drag = Some(DragKind::Demolish);
                    state.drag_origin = Some(tile);
                }
            }
        }
    }

    if mouse.just_pressed(MouseButton::Right) {
        if let Some(tile) = hover {
            state.drag = Some(DragKind::Demolish);
            state.drag_origin = Some(tile);
            state.build_preview = None;
        }
    }

    if let Some(kind) = state.drag {
        let origin = state.drag_origin.or(state.anchor);
        let tip = hover.or(origin);
        if let (Some(from), Some(to)) = (origin, tip) {
            match kind {
                DragKind::Build => {
                    state.demolish_preview = None;
                    state.build_preview = Some(preview_build(
                        &network,
                        &terrain,
                        &money,
                        from,
                        to,
                        state.path_mode,
                    ));
                }
                DragKind::Demolish => {
                    state.build_preview = None;
                    state.demolish_preview = Some(preview_demolish(&network, &money, from, to));
                }
            }
        }
    } else if state.tool == BuildTool::Build {
        if let (Some(from), Some(to)) = (state.anchor, hover) {
            state.build_preview = Some(preview_build(
                &network,
                &terrain,
                &money,
                from,
                to,
                state.path_mode,
            ));
            state.demolish_preview = None;
        } else {
            state.build_preview = None;
            state.demolish_preview = None;
        }
    } else {
        state.build_preview = None;
        state.demolish_preview = None;
    }

    let left_up = mouse.just_released(MouseButton::Left);
    let right_up = mouse.just_released(MouseButton::Right);

    if left_up && state.drag == Some(DragKind::Build) {
        commit_build(&mut state, &mut buffer, &mut feedback, &network);
        state.drag = None;
        state.drag_origin = None;
    } else if (left_up || right_up) && state.drag == Some(DragKind::Demolish) {
        commit_demolish(&mut state, &mut buffer, &mut feedback, &network);
        state.drag = None;
        state.drag_origin = None;
    }
}

fn commit_build(
    state: &mut TrackToolState,
    buffer: &mut CommandBuffer,
    feedback: &mut BuildFeedback,
    network: &TrackNetwork,
) {
    let Some(preview) = state.build_preview.clone() else {
        return;
    };
    if let Some(reject) = &preview.reject {
        push_reject(feedback, reject);
        return;
    }
    if !preview.can_commit {
        return;
    }

    let from = state.drag_origin.or(state.anchor);
    let Some(from) = from else {
        return;
    };
    let to = preview.endpoint;

    match state.path_mode {
        PathMode::SingleTile => {
            buffer.push(CommandKind::PlaceTrack(PlaceTrack {
                tile: to,
                layer: GROUND_LAYER,
            }));
            state.anchor = Some(to);
        }
        PathMode::Autofill | PathMode::ExactStraight => {
            if from == to {
                if network.id_at(to, GROUND_LAYER).is_none() {
                    buffer.push(CommandKind::PlaceTrack(PlaceTrack {
                        tile: to,
                        layer: GROUND_LAYER,
                    }));
                }
                state.anchor = Some(to);
            } else {
                buffer.push(CommandKind::AutoFillTrack(AutoFillTrack {
                    from,
                    to,
                    layer: GROUND_LAYER,
                }));
                state.anchor = Some(to);
            }
        }
    }
    state.build_preview = None;
}

fn commit_demolish(
    state: &mut TrackToolState,
    buffer: &mut CommandBuffer,
    feedback: &mut BuildFeedback,
    network: &TrackNetwork,
) {
    let Some(preview) = state.demolish_preview.clone() else {
        if state.tool == BuildTool::Build {
            state.anchor = None;
        }
        return;
    };
    if let Some(reject) = &preview.reject {
        push_reject(feedback, reject);
        if preview.track_count == 0 && state.tool == BuildTool::Build {
            state.anchor = None;
        }
        state.demolish_preview = None;
        return;
    }

    for &tile in &preview.tiles {
        if let Some(id) = network.id_at(tile, GROUND_LAYER) {
            buffer.push(CommandKind::Demolish(Demolish { track: id }));
        }
    }
    state.demolish_preview = None;
}
