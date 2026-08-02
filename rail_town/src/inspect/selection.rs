//! Selection state, click-to-select input, and follow-camera.

use std::collections::{HashMap, VecDeque};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{world_to_tile, MapGrid, TILE_SIZE};
use rail_sim::{
    IndustryRegistry, Peep, StationId, StationRegistry, TileOccupancy, TrackNetwork, Train,
    TrainLocation, GROUND_LAYER,
};

use crate::map::{CameraFocusRequest, MapCamera, MapViewState};
use crate::stations::{IndustrySprite, StationSprite};
use crate::track::{TrackSprite, TrackToolState};
use crate::trains::{TrainSprite, TrainToolState};
use crate::town::PeepSprite;
use crate::ui::UiBlocksWorld;

use super::pick::{point_hits_sprite, resolve_pick, Selectable};

/// Current single selection (Phase B — no multi-select yet).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Selection(pub Option<Selectable>);

impl Selection {
    pub fn clear(&mut self) {
        self.0 = None;
    }

    pub fn set(&mut self, s: Selectable) {
        self.0 = Some(s);
    }
}

/// When true, track / train tools should ignore this frame's left press.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct WorldClickConsumed(pub bool);

/// Recent service scores for sparkline / cause delta (per station).
#[derive(Resource, Debug, Default)]
pub struct ServiceScoreHistory {
    samples: HashMap<StationId, VecDeque<u8>>,
}

const HISTORY_LEN: usize = 24;
/// Wait minutes that count as "long" in the station cause line.
pub const LONG_WAIT_MINUTES: u32 = 8;

impl ServiceScoreHistory {
    pub fn push(&mut self, id: StationId, score: u8) {
        let q = self.samples.entry(id).or_default();
        if q.back().copied() == Some(score) && q.len() >= 2 {
            if q.len() >= HISTORY_LEN {
                q.pop_front();
                q.push_back(score);
            }
            return;
        }
        q.push_back(score);
        while q.len() > HISTORY_LEN {
            q.pop_front();
        }
    }

    pub fn delta(&self, id: StationId) -> i16 {
        let Some(q) = self.samples.get(&id) else {
            return 0;
        };
        let Some(&last) = q.back() else {
            return 0;
        };
        let older = q
            .iter()
            .rev()
            .nth(8)
            .or_else(|| q.front())
            .copied()
            .unwrap_or(last);
        last as i16 - older as i16
    }

    pub fn sparkline(&self, id: StationId) -> String {
        let Some(q) = self.samples.get(&id) else {
            return "·".into();
        };
        const BARS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        q.iter()
            .map(|&s| {
                let idx = ((s as usize) * (BARS.len() - 1)) / 100;
                BARS[idx.min(BARS.len() - 1)]
            })
            .collect()
    }
}

/// Bundled sprite queries so click input stays under Bevy's system-param limit.
#[derive(SystemParam)]
pub struct WorldPickSprites<'w, 's> {
    peeps: Query<'w, 's, (&'static PeepSprite, &'static Transform, &'static Sprite)>,
    trains: Query<'w, 's, (&'static TrainSprite, &'static Transform, &'static Sprite)>,
    stations: Query<'w, 's, (&'static StationSprite, &'static Transform, &'static Sprite)>,
    industries: Query<'w, 's, (&'static IndustrySprite, &'static Transform, &'static Sprite)>,
    tracks: Query<'w, 's, (&'static TrackSprite, &'static Transform, &'static Sprite)>,
}

pub fn sample_service_history(
    service: Res<rail_sim::StationService>,
    mut history: ResMut<ServiceScoreHistory>,
) {
    for (id, score) in &service.scores {
        history.push(*id, score.score);
    }
}

pub fn selection_click_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MapCamera>>,
    map: Res<MapGrid>,
    map_view: Res<MapViewState>,
    ui_blocks: Res<UiBlocksWorld>,
    train_tool: Res<TrainToolState>,
    track_tool: Res<TrackToolState>,
    network: Res<TrackNetwork>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    sprites: WorldPickSprites,
    mut selection: ResMut<Selection>,
    mut consumed: ResMut<WorldClickConsumed>,
) {
    // Reset each frame before tools read it (SelectionInputSet runs first).
    consumed.0 = false;

    if keys.just_pressed(KeyCode::Escape) {
        selection.clear();
    }

    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    if ui_blocks.0 {
        return;
    }
    // Map View owns left-click (fly-to); don't pick undersampled sprites.
    if map_view.active {
        return;
    }
    if train_tool.place_mode {
        return;
    }
    if track_tool.drag.is_some() {
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

    let hit = pick_world(world, tile, &network, &stations, &industries, &sprites);

    match hit {
        Some(s) => {
            selection.set(s);
            consumed.0 = true;
        }
        None => {
            selection.clear();
        }
    }
}

fn pick_world(
    world: Vec2,
    tile: rail_sim::TileCoord,
    network: &TrackNetwork,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    sprites: &WorldPickSprites,
) -> Option<Selectable> {
    let mut candidates = Vec::new();

    for (sprite, tf, spr) in sprites.peeps.iter() {
        let size = spr.custom_size.unwrap_or(Vec2::splat(TILE_SIZE * 0.28));
        if point_hits_sprite(world, tf.translation.truncate(), size) {
            candidates.push(Selectable::Peep(sprite.id));
        }
    }
    for (sprite, tf, spr) in sprites.trains.iter() {
        let size = spr
            .custom_size
            .unwrap_or(Vec2::new(TILE_SIZE * 0.55, TILE_SIZE * 0.22));
        let pad = Vec2::splat(4.0);
        if point_hits_sprite(world, tf.translation.truncate(), size + pad) {
            candidates.push(Selectable::Train(sprite.id));
        }
    }
    for (sprite, tf, spr) in sprites.stations.iter() {
        let size = spr.custom_size.unwrap_or(Vec2::splat(TILE_SIZE * 0.55));
        if point_hits_sprite(world, tf.translation.truncate(), size) {
            candidates.push(Selectable::Station(sprite.id));
        }
    }
    if let Some(st) = stations.at(tile, GROUND_LAYER) {
        candidates.push(Selectable::Station(st.id));
    }
    for (sprite, tf, spr) in sprites.industries.iter() {
        let size = spr.custom_size.unwrap_or(Vec2::splat(TILE_SIZE * 0.5));
        if point_hits_sprite(world, tf.translation.truncate(), size) {
            candidates.push(Selectable::Industry(sprite.id));
        }
    }
    if let Some(ind) = industries.at(tile) {
        candidates.push(Selectable::Industry(ind.id));
    }
    for (sprite, tf, spr) in sprites.tracks.iter() {
        let size = spr
            .custom_size
            .unwrap_or(Vec2::new(TILE_SIZE * 0.7, TILE_SIZE * 0.35));
        let pad = Vec2::new(0.0, 6.0);
        if point_hits_sprite(world, tf.translation.truncate(), size + pad) {
            candidates.push(Selectable::Track(sprite.id));
        }
    }
    if let Some(id) = network.id_at(tile, GROUND_LAYER) {
        candidates.push(Selectable::Track(id));
    }

    resolve_pick(&candidates)
}

/// `F` requests a texel-snapped camera cut to the selection.
pub fn follow_selection(
    keys: Res<ButtonInput<KeyCode>>,
    selection: Res<Selection>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    network: Res<TrackNetwork>,
    peeps: Query<&Peep>,
    trains: Query<(&Train, &TrainLocation)>,
    train_sprites: Query<(&TrainSprite, &Transform)>,
    peep_sprites: Query<(&PeepSprite, &Transform)>,
    mut focus: ResMut<CameraFocusRequest>,
) {
    if !keys.just_pressed(KeyCode::KeyF) {
        return;
    }
    let Some(sel) = selection.0 else {
        return;
    };

    let target = match sel {
        Selectable::Station(id) => stations.get(id).map(|s| {
            let (x, y) = rail_map::tile_to_world(s.tile);
            Vec2::new(x, y)
        }),
        Selectable::Industry(id) => industries.get(id).map(|i| {
            let (x, y) = rail_map::tile_to_world(i.tile);
            Vec2::new(x, y)
        }),
        Selectable::Track(id) => network.piece(id).map(|p| {
            let (x, y) = rail_map::tile_to_world(p.tile);
            Vec2::new(x, y)
        }),
        Selectable::Train(id) => train_sprites
            .iter()
            .find(|(s, _)| s.id == id)
            .map(|(_, tf)| tf.translation.truncate())
            .or_else(|| {
                trains.iter().find(|(t, _)| t.id == id).and_then(|(_, loc)| {
                    network.piece(loc.track).map(|p| {
                        let (x, y) = rail_map::tile_to_world(p.tile);
                        Vec2::new(x, y)
                    })
                })
            }),
        Selectable::Peep(id) => peep_sprites
            .iter()
            .find(|(s, _)| s.id == id)
            .map(|(_, tf)| tf.translation.truncate())
            .or_else(|| {
                peeps.iter().find(|p| p.id == id).map(|p| {
                    let (x, y) = rail_map::tile_to_world(p.home);
                    Vec2::new(x, y)
                })
            }),
    };

    if let Some(t) = target {
        focus.0 = Some(t);
    }
}

/// True when a train is waiting on an occupied next tile.
#[allow(dead_code)]
pub fn train_is_blocked(
    loc: &TrainLocation,
    occupancy: &TileOccupancy,
    train_id: rail_sim::TrainId,
) -> bool {
    if loc.parked || loc.at_destination() {
        return false;
    }
    let Some(&next) = loc.path.get(loc.path_index.saturating_add(1)) else {
        return false;
    };
    match occupancy.by_track.get(&next) {
        Some(other) if *other != train_id => true,
        _ => false,
    }
}
