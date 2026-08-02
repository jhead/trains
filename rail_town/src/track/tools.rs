//! Click-to-build / demolish → sim [`CommandBuffer`].
//!
//! ## Autofill (two-click anchors)
//! In **Build** mode, the first click places an anchor tile; the second click
//! pushes [`AutoFillTrack`] on an orthogonal or 45° diagonal. Esc / right-click
//! clears the pending anchor. Hold **Shift** while clicking to place a single
//! tile without starting an autofill pair.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{world_to_tile, MapGrid};
use rail_sim::commands::{AutoFillTrack, Demolish, PlaceTrack};
use rail_sim::{CommandBuffer, CommandKind, TrackNetwork, GROUND_LAYER};

use crate::map::MapCamera;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Resource)]
pub enum BuildTool {
    #[default]
    Build,
    Demolish,
}

#[derive(Debug, Clone, Default, Resource)]
pub struct TrackToolState {
    pub tool: BuildTool,
    /// First anchor for two-click autofill (Build mode).
    pub autofill_from: Option<rail_sim::TileCoord>,
    /// When true (train place mode), ignore left-click build/demolish.
    pub suppress_build_click: bool,
}

pub fn track_tool_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MapCamera>>,
    map: Res<MapGrid>,
    network: Res<TrackNetwork>,
    mut buffer: ResMut<CommandBuffer>,
    mut state: ResMut<TrackToolState>,
) {
    if keys.just_pressed(KeyCode::KeyB) {
        state.tool = BuildTool::Build;
        state.autofill_from = None;
        state.suppress_build_click = false;
    }
    if keys.just_pressed(KeyCode::KeyX) {
        state.tool = BuildTool::Demolish;
        state.autofill_from = None;
        state.suppress_build_click = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        state.autofill_from = None;
    }

    if mouse.just_pressed(MouseButton::Right) {
        state.autofill_from = None;
        return;
    }

    if state.suppress_build_click {
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
    let Ok((camera, cam_transform)) = camera_q.single() else {
        return;
    };
    let Ok(world) = camera.viewport_to_world_2d(cam_transform, cursor) else {
        return;
    };
    let tile = world_to_tile(world.x, world.y);
    if !map.contains(tile) {
        return;
    }

    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);

    match state.tool {
        BuildTool::Build => {
            if shift {
                buffer.push(CommandKind::PlaceTrack(PlaceTrack {
                    tile,
                    layer: GROUND_LAYER,
                }));
                return;
            }
            if let Some(from) = state.autofill_from.take() {
                if from != tile {
                    buffer.push(CommandKind::AutoFillTrack(AutoFillTrack {
                        from,
                        to: tile,
                        layer: GROUND_LAYER,
                    }));
                }
            } else {
                // Place the anchor tile, then wait for the second click to autofill.
                buffer.push(CommandKind::PlaceTrack(PlaceTrack {
                    tile,
                    layer: GROUND_LAYER,
                }));
                state.autofill_from = Some(tile);
            }
        }
        BuildTool::Demolish => {
            if let Some(id) = network.id_at(tile, GROUND_LAYER) {
                buffer.push(CommandKind::Demolish(Demolish { track: id }));
            }
        }
    }
}
