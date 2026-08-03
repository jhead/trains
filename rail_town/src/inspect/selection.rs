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

use crate::input::{ControlAction, KeyBindings};
use crate::map::{CameraFocusRequest, MapCamera, MapViewState};
use crate::stations::{IndustrySprite, StationSprite};
use crate::track::{BuildTool, TrackSprite, TrackToolState};
use crate::trains::{TrainSprite, TrainToolState};
use crate::town::PeepSprite;
use crate::ui::UiBlocksWorld;

use super::pick::{better_pick, point_hits_sprite, Selectable};

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

    /// Trend as text, until the real 24x8 sparkline widget lands (03 §8.5).
    ///
    /// ASCII only. The shipped font is Bevy's default at integer sizes and has
    /// no block-drawing glyphs, so `U+2581..U+2588` rendered as tofu boxes in
    /// the Inspector's trend row — a row of empty rectangles reads as a bug,
    /// not as a trend. This ramp says the same thing in glyphs that exist.
    pub fn sparkline(&self, id: StationId) -> String {
        let Some(q) = self.samples.get(&id) else {
            return "-".into();
        };
        const BARS: &[char] = &['.', ':', '-', '=', '+', '*', '#', '@'];
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

/// Fallback footprint for each sprite kind, when it carries no `custom_size`.
///
/// One table, read by both the hit test and the hover bracket, so the shape the
/// player points at is the shape they see framed.
fn default_size(sel: Selectable) -> Vec2 {
    match sel {
        Selectable::Peep(_) => Vec2::splat(TILE_SIZE * 0.28),
        Selectable::Train(_) => Vec2::new(TILE_SIZE * 0.55, TILE_SIZE * 0.22),
        Selectable::Station(_) => Vec2::splat(TILE_SIZE * 0.55),
        Selectable::Industry(_) => Vec2::splat(TILE_SIZE * 0.5),
        Selectable::Track(_) => Vec2::new(TILE_SIZE * 0.7, TILE_SIZE * 0.35),
    }
}

impl WorldPickSprites<'_, '_> {
    /// Where `sel`'s sprite actually is, in world texels.
    ///
    /// [`super::hover`] frames the hovered object with this rather than with the
    /// tile it stands on: peeps and trains move continuously and are a fraction
    /// of a tile across, so a tile-sized bracket around one reads as a selection
    /// of the ground, not of the thing.
    pub(super) fn rect_of(&self, sel: Selectable) -> Option<Rect> {
        let found = match sel {
            Selectable::Peep(id) => self
                .peeps
                .iter()
                .find(|(s, _, _)| s.id == id)
                .map(|(_, tf, spr)| (tf, spr)),
            Selectable::Train(id) => self
                .trains
                .iter()
                .find(|(s, _, _)| s.id == id)
                .map(|(_, tf, spr)| (tf, spr)),
            Selectable::Station(id) => self
                .stations
                .iter()
                .find(|(s, _, _)| s.id == id)
                .map(|(_, tf, spr)| (tf, spr)),
            Selectable::Industry(id) => self
                .industries
                .iter()
                .find(|(s, _, _)| s.id == id)
                .map(|(_, tf, spr)| (tf, spr)),
            Selectable::Track(id) => self
                .tracks
                .iter()
                .find(|(s, _, _)| s.id == id)
                .map(|(_, tf, spr)| (tf, spr)),
        };
        let (tf, spr) = found?;
        let size = spr.custom_size.unwrap_or_else(|| default_size(sel));
        Some(Rect::from_center_size(tf.translation.truncate(), size))
    }
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
    // Only the Look tool selects. An armed tool owns the world press —
    // otherwise clicking to demolish also selected the piece and popped the
    // Inspector open on a track that was about to stop existing.
    if track_tool.tool != BuildTool::Select || track_tool.drag.is_some() {
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

/// Shared with [`super::hover`] so the hover tier picks exactly what a click
/// would pick, rather than growing a second, subtly different hit test.
///
/// Hover calls this whenever the pointer moves, so it keeps the running best
/// rather than collecting candidates and sorting them: the answer is the same
/// (first-wins on a priority tie, as [`resolve_pick`]) with no allocation.
pub(super) fn pick_world(
    world: Vec2,
    tile: rail_sim::TileCoord,
    network: &TrackNetwork,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    sprites: &WorldPickSprites,
) -> Option<Selectable> {
    let mut best: Option<Selectable> = None;
    let mut offer = |candidate: Selectable| best = better_pick(best, candidate);

    for (sprite, tf, spr) in sprites.peeps.iter() {
        let sel = Selectable::Peep(sprite.id);
        let size = spr.custom_size.unwrap_or_else(|| default_size(sel));
        if point_hits_sprite(world, tf.translation.truncate(), size) {
            offer(sel);
        }
    }
    for (sprite, tf, spr) in sprites.trains.iter() {
        let sel = Selectable::Train(sprite.id);
        let size = spr.custom_size.unwrap_or_else(|| default_size(sel));
        let pad = Vec2::splat(4.0);
        if point_hits_sprite(world, tf.translation.truncate(), size + pad) {
            offer(sel);
        }
    }
    for (sprite, tf, spr) in sprites.stations.iter() {
        let sel = Selectable::Station(sprite.id);
        let size = spr.custom_size.unwrap_or_else(|| default_size(sel));
        if point_hits_sprite(world, tf.translation.truncate(), size) {
            offer(sel);
        }
    }
    if let Some(st) = stations.at(tile, GROUND_LAYER) {
        offer(Selectable::Station(st.id));
    }
    for (sprite, tf, spr) in sprites.industries.iter() {
        let sel = Selectable::Industry(sprite.id);
        let size = spr.custom_size.unwrap_or_else(|| default_size(sel));
        if point_hits_sprite(world, tf.translation.truncate(), size) {
            offer(sel);
        }
    }
    if let Some(ind) = industries.at(tile) {
        offer(Selectable::Industry(ind.id));
    }
    for (sprite, tf, spr) in sprites.tracks.iter() {
        let sel = Selectable::Track(sprite.id);
        let size = spr.custom_size.unwrap_or_else(|| default_size(sel));
        let pad = Vec2::new(0.0, 6.0);
        if point_hits_sprite(world, tf.translation.truncate(), size + pad) {
            offer(sel);
        }
    }
    if let Some(id) = network.id_at(tile, GROUND_LAYER) {
        offer(Selectable::Track(id));
    }

    best
}

/// `F` requests a texel-snapped camera cut to the selection.
pub fn follow_selection(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
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
    if !bindings.just_pressed(&keys, ControlAction::FollowSelection) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trend_ramp_is_ascii_and_rises() {
        let mut history = ServiceScoreHistory::default();
        let id = StationId(1);
        for score in [0, 14, 28, 42, 57, 71, 85, 100] {
            history.push(id, score);
        }
        let line = history.sparkline(id);
        assert!(
            line.is_ascii(),
            "the shipped font draws non-ASCII as tofu: {line:?}"
        );
        // Monotonic input has to produce a monotonic ramp, or it is decoration.
        let ranks: Vec<usize> = line
            .chars()
            .map(|c| ".:-=+*#@".find(c).expect("ramp glyph"))
            .collect();
        assert!(
            ranks.windows(2).all(|w| w[0] <= w[1]),
            "ramp {line:?} does not rise with the score"
        );
        assert_eq!(ranks.first(), Some(&0));
        assert_eq!(ranks.last(), Some(&7));
    }

    #[test]
    fn an_unknown_station_has_no_trend() {
        let history = ServiceScoreHistory::default();
        assert_eq!(history.sparkline(StationId(9)), "-");
        assert_eq!(history.delta(StationId(9)), 0);
    }
}
