//! Keyboard buy + click-to-place trains at stations.
//!
//! - `T` — buy a **transit** train (if affordable) and select transit place mode
//! - `G` — buy a **transport** (goods) train and select goods place mode
//! - Left click on a station tile (or adjacent) — place the oldest unplaced train
//!   of the selected kind (station must have track on/adjacent)

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{world_to_tile, MapGrid};
use rail_sim::commands::{BuyTrain, PlaceTrain};
use rail_sim::{
    track_for_station, CommandBuffer, CommandKind, StationRegistry, TrainKind, TrainYard,
    TrackNetwork, GROUND_LAYER,
};

use crate::inspect::WorldClickConsumed;
use crate::map::MapCamera;
use crate::track::{BuildTool, TrackToolState};
use crate::ui::UiBlocksWorld;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrainPlaceKind {
    #[default]
    Transit,
    Transport,
}

#[derive(Debug, Clone, Default, Resource)]
pub struct TrainToolState {
    /// When true, left-click places a train instead of building track.
    pub place_mode: bool,
    pub kind: TrainPlaceKind,
}

impl TrainPlaceKind {
    fn to_sim(self) -> TrainKind {
        match self {
            Self::Transit => TrainKind::Transit,
            Self::Transport => TrainKind::Transport,
        }
    }
}

pub fn train_tool_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MapCamera>>,
    map: Res<MapGrid>,
    stations: Res<StationRegistry>,
    network: Res<TrackNetwork>,
    yard: Res<TrainYard>,
    mut buffer: ResMut<CommandBuffer>,
    mut train_state: ResMut<TrainToolState>,
    mut track_state: ResMut<TrackToolState>,
    ui_blocks: Res<UiBlocksWorld>,
    click_consumed: Res<WorldClickConsumed>,
) {
    if keys.just_pressed(KeyCode::KeyT) {
        buffer.push(CommandKind::BuyTrain(BuyTrain {
            kind: TrainKind::Transit,
        }));
        train_state.place_mode = true;
        train_state.kind = TrainPlaceKind::Transit;
        track_state.anchor = None;
        track_state.suppress_build_click = true;
    }
    if keys.just_pressed(KeyCode::KeyG) {
        buffer.push(CommandKind::BuyTrain(BuyTrain {
            kind: TrainKind::Transport,
        }));
        train_state.place_mode = true;
        train_state.kind = TrainPlaceKind::Transport;
        track_state.anchor = None;
        track_state.suppress_build_click = true;
    }
    // B / X reclaim track tools.
    if keys.just_pressed(KeyCode::KeyB) || keys.just_pressed(KeyCode::KeyX) {
        train_state.place_mode = false;
        track_state.suppress_build_click = false;
    }

    if !train_state.place_mode {
        return;
    }
    // Don't fight demolish clicks.
    if track_state.tool == BuildTool::Demolish {
        return;
    }

    if ui_blocks.0 || click_consumed.0 {
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

    let kind = train_state.kind.to_sim();
    let Some(train_id) = yard.peek_kind(kind) else {
        return;
    };

    // Find station on clicked tile or any station whose track covers this tile.
    let station_id = stations
        .id_at(tile, GROUND_LAYER)
        .or_else(|| {
            stations.iter().find_map(|s| {
                track_for_station(&network, s.tile, s.layer).and_then(|tid| {
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
            // Adjacent to a station tile.
            stations.iter().find_map(|s| {
                let dx = (s.tile.x - tile.x).abs();
                let dy = (s.tile.y - tile.y).abs();
                if dx <= 1 && dy <= 1 {
                    Some(s.id)
                } else {
                    None
                }
            })
        });

    let Some(at_station) = station_id else {
        return;
    };
    let Some(station) = stations.get(at_station) else {
        return;
    };
    if track_for_station(&network, station.tile, station.layer).is_none() {
        return;
    }

    buffer.push(CommandKind::PlaceTrain(PlaceTrain {
        train: train_id,
        at_station,
    }));
}
