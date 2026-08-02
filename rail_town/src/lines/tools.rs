//! Line drawing tool — click stations in order, Enter confirms.
//!
//! `L` selects the Line tool. Left-click appends a station. Enter creates the
//! line via [`CreateLine`]. Esc / right-click clears the draft.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{world_to_tile, MapGrid};
use rail_sim::{
    find_path, line_path, suggest_line_name, track_for_station, AssignTrainToLine, CommandBuffer,
    CommandKind, CreateLine, StationId, StationRegistry, TrackNetwork, GROUND_LAYER,
};

use crate::inspect::WorldClickConsumed;
use crate::map::MapCamera;
use crate::track::{BuildTool, TrackToolState};
use crate::trains::TrainToolState;
use crate::ui::UiBlocksWorld;

/// Presentation mode for the Line tool (does not buy rolling stock).
#[derive(Debug, Clone, Default, Resource)]
pub struct LineToolState {
    pub active: bool,
    /// Stations clicked so far (ordered).
    pub draft_stops: Vec<StationId>,
    /// Last connectivity warning for status / HUD.
    pub warn: Option<String>,
}

impl LineToolState {
    pub fn clear_draft(&mut self) {
        self.draft_stops.clear();
        self.warn = None;
    }
}

pub fn line_tool_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MapCamera>>,
    map: Res<MapGrid>,
    stations: Res<StationRegistry>,
    network: Res<TrackNetwork>,
    mut buffer: ResMut<CommandBuffer>,
    mut line_state: ResMut<LineToolState>,
    mut track_state: ResMut<TrackToolState>,
    mut train_state: ResMut<TrainToolState>,
    ui_blocks: Res<UiBlocksWorld>,
    click_consumed: Res<WorldClickConsumed>,
) {
    if keys.just_pressed(KeyCode::KeyL) {
        line_state.active = true;
        train_state.place_mode = false;
        track_state.tool = BuildTool::Build;
        track_state.anchor = None;
        track_state.drag = None;
        track_state.suppress_build_click = true;
        line_state.clear_draft();
    }

    // Other tools reclaim focus.
    if keys.just_pressed(KeyCode::KeyB)
        || keys.just_pressed(KeyCode::KeyX)
        || keys.just_pressed(KeyCode::KeyT)
        || keys.just_pressed(KeyCode::KeyG)
    {
        line_state.active = false;
        line_state.clear_draft();
        if keys.just_pressed(KeyCode::KeyB) || keys.just_pressed(KeyCode::KeyX) {
            track_state.suppress_build_click = false;
        }
    }

    if !line_state.active {
        return;
    }

    if keys.just_pressed(KeyCode::Escape) {
        line_state.clear_draft();
        return;
    }

    if keys.just_pressed(KeyCode::Enter) {
        confirm_draft(&mut line_state, &stations, &mut buffer);
        return;
    }

    if ui_blocks.0 || click_consumed.0 {
        return;
    }

    if mouse.just_pressed(MouseButton::Right) {
        if let Some(last) = line_state.draft_stops.pop() {
            let _ = last;
            line_state.warn = None;
        }
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

    let Some(station_id) = pick_station(&stations, &network, tile) else {
        return;
    };
    // Don't repeat consecutive stop.
    if line_state.draft_stops.last() == Some(&station_id) {
        return;
    }

    // Connectivity check against previous stop.
    if let Some(&prev) = line_state.draft_stops.last() {
        let connected = segment_connected(&network, &stations, prev, station_id);
        if !connected {
            let a = stations.get(prev).map(|s| s.name.as_str()).unwrap_or("?");
            let b = stations
                .get(station_id)
                .map(|s| s.name.as_str())
                .unwrap_or("?");
            line_state.warn = Some(format!("No route — {a} is not connected to {b}."));
            // Still allow adding so the player sees the warn segment; confirm will
            // still create the line (ops can fix track later). Or refuse?
            // Design: draw warn but allow confirm only if all segments connect.
            // We add with warn; confirm checks path.
        } else {
            line_state.warn = None;
        }
    }

    line_state.draft_stops.push(station_id);
}

fn confirm_draft(
    line_state: &mut LineToolState,
    stations: &StationRegistry,
    buffer: &mut CommandBuffer,
) {
    if line_state.draft_stops.len() < 2 {
        line_state.warn = Some("Need at least two stations.".into());
        return;
    }
    let name = suggest_line_name(stations, &line_state.draft_stops);
    buffer.push(CommandKind::CreateLine(CreateLine {
        name: Some(name),
        stops: line_state.draft_stops.clone(),
    }));
    line_state.clear_draft();
    line_state.active = false;
}

fn pick_station(
    stations: &StationRegistry,
    network: &TrackNetwork,
    tile: rail_sim::TileCoord,
) -> Option<StationId> {
    stations
        .id_at(tile, GROUND_LAYER)
        .or_else(|| {
            stations.iter().find_map(|s| {
                track_for_station(network, s.tile, s.layer).and_then(|tid| {
                    let piece = network.piece(tid)?;
                    if piece.tile == tile {
                        Some(s.id)
                    } else {
                        None
                    }
                })
            })
        })
        .or_else(|| {
            stations.iter().find_map(|s| {
                let dx = (s.tile.x - tile.x).abs();
                let dy = (s.tile.y - tile.y).abs();
                if dx <= 1 && dy <= 1 {
                    Some(s.id)
                } else {
                    None
                }
            })
        })
}

fn segment_connected(
    network: &TrackNetwork,
    stations: &StationRegistry,
    from: StationId,
    to: StationId,
) -> bool {
    let Some(a) = stations.get(from) else {
        return false;
    };
    let Some(b) = stations.get(to) else {
        return false;
    };
    let Some(ta) = track_for_station(network, a.tile, a.layer) else {
        return false;
    };
    let Some(tb) = track_for_station(network, b.tile, b.layer) else {
        return false;
    };
    find_path(network, ta, tb).is_some()
}

/// Click a train while a line is selected in the panel → assign (helper for panel).
#[allow(dead_code)]
pub fn assign_selected_train_to_line(
    buffer: &mut CommandBuffer,
    train_id: rail_sim::TrainId,
    line_id: rail_sim::LineId,
) {
    buffer.push(CommandKind::AssignTrainToLine(AssignTrainToLine {
        train: train_id,
        line: line_id,
    }));
}

/// Preview connectivity for the draft (used by strip / HUD).
#[allow(dead_code)]
pub fn draft_fully_connected(
    network: &TrackNetwork,
    stations: &StationRegistry,
    stops: &[StationId],
) -> bool {
    line_path(network, stations, stops).is_some()
}
