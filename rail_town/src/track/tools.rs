//! Drag-to-build / right-drag demolish → sim [`CommandBuffer`].
//!
//! ## Build
//! Press → drag → release. Live ghost every frame. The default drag proposes
//! the smart route (brief 04 §2.2 — cheapest legal path, weighted straight);
//! Shift snaps to one of the sixteen directions, terrain be damned; Ctrl
//! places a single tile; Alt holds the contour. After a successful commit the
//! endpoint stays as the continuous-build anchor.
//!
//! ## Demolish
//! Left-click / left-drag with the Demolish tool, or right-drag from either
//! tool, refunds along the snapped path. Esc clears the build anchor.
//!
//! ## A click is not a zero-length drag
//!
//! Demolish captures its origin tile on press and follows the pointer to the
//! release. A pointer never holds perfectly still, so treating "still on the
//! press tile" as the test for a click means a click near a tile boundary can
//! commit a two-tile run and take the neighbour with it. The gesture is
//! separated by *distance travelled* instead ([`DRAG_SLOP_TEXELS`]): until the
//! pointer has left a small disc around the press point the run is exactly the
//! press tile, whatever tile the cursor has since wandered into.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{world_to_tile, MapGrid};
use rail_sim::commands::{AutoFillPath, AutoFillTrack, Demolish, PlaceTrack};
use rail_sim::ids::TileCoord;
use rail_sim::{
    CommandBuffer, CommandKind, Money, TrackNetwork, TrackTerrain, GROUND_LAYER,
};

use crate::input::{ControlAction, KeyBindings};
use crate::map::MapCamera;
use crate::ui::UiBlocksWorld;

use super::feedback::{push_reject, BuildFeedback};
use super::preview::{preview_build, preview_demolish, BuildPreview, DemolishPreview};
use super::propose::PathMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Resource)]
pub enum BuildTool {
    /// Look around without building anything.
    ///
    /// The default, deliberately. With only Build and Demolish the player is
    /// permanently armed: every click on the world lays track, and there is no
    /// way to just inspect. `Esc` always comes back here.
    #[default]
    Select,
    Build,
    Demolish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragKind {
    Build,
    Demolish,
}

/// How far the pointer may travel from the press point and still read as a
/// click, in world texels.
///
/// A fifth of a tile: far enough to swallow the shake of a click and any
/// sub-tile jitter in the pointer, nowhere near far enough to reach a
/// neighbouring tile centre. Measured in *world* texels rather than screen
/// pixels so the gesture means the same thing at every zoom rung — a click is a
/// click on the ground, not on the screen.
pub const DRAG_SLOP_TEXELS: f32 = 6.0;

#[derive(Debug, Clone, Default, Resource)]
pub struct TrackToolState {
    pub tool: BuildTool,
    /// Continuous-build / in-progress anchor.
    pub anchor: Option<TileCoord>,
    pub drag: Option<DragKind>,
    pub drag_origin: Option<TileCoord>,
    /// World position of the press that started the drag, for [`DRAG_SLOP_TEXELS`].
    pub drag_press_world: Option<Vec2>,
    /// Which button started the drag; only its release commits the run.
    pub drag_button: Option<MouseButton>,
    /// True once the pointer has travelled past the slop — the gesture is a
    /// drag, not a click, and stays one for the rest of the press.
    pub drag_moved: bool,
    pub hover_tile: Option<TileCoord>,
    pub path_mode: PathMode,
    pub build_preview: Option<BuildPreview>,
    pub demolish_preview: Option<DemolishPreview>,
    /// When true (train place mode), ignore build/demolish pointer input.
    pub suppress_build_click: bool,
    /// Last frame's accepted smart proposal — the shape hold of brief 04
    /// §2.2. Fed back into the search so an equal-cost alternative cannot
    /// flicker the ghost; cleared whenever there is no smart preview.
    pub smart_hold: Option<Vec<TileCoord>>,
}

impl TrackToolState {
    /// Forget any in-progress press. Does not touch the tool or the anchor.
    ///
    /// Public because arming a tool from anywhere else — the toolbar, Map View,
    /// the train and line tools — has to drop the *whole* press, not just
    /// `drag`: a half-cleared press leaves an origin and a press point from a
    /// gesture the player has already abandoned.
    pub fn clear_drag(&mut self) {
        self.drag = None;
        self.drag_origin = None;
        self.drag_press_world = None;
        self.drag_moved = false;
        self.drag_button = None;
    }

    /// Start a press at `tile`, remembering where on the ground it landed.
    fn begin_drag(
        &mut self,
        kind: DragKind,
        tile: TileCoord,
        world: Option<Vec2>,
        button: MouseButton,
    ) {
        self.drag = Some(kind);
        self.drag_origin = Some(tile);
        self.drag_press_world = world;
        self.drag_moved = false;
        self.drag_button = Some(button);
    }
}

/// Cursor position in world texels under the map camera.
fn cursor_world(
    windows: &Query<&Window, With<PrimaryWindow>>,
    camera_q: &Query<(&Camera, &GlobalTransform), With<MapCamera>>,
) -> Option<Vec2> {
    let window = windows.single().ok()?;
    let cursor = window.cursor_position()?;
    let (camera, cam_transform) = camera_q.single().ok()?;
    camera.viewport_to_world_2d(cam_transform, cursor).ok()
}

/// Whether a press that started at `press` has travelled far enough to be a drag.
#[inline]
pub fn press_became_drag(press: Vec2, now: Vec2) -> bool {
    press.distance_squared(now) > DRAG_SLOP_TEXELS * DRAG_SLOP_TEXELS
}

/// The far end of a demolish gesture that pressed on `origin`.
///
/// Until the press has travelled past the slop (`moved`) the run is the press
/// tile and nothing else, so a click that ends a texel over a tile boundary
/// still demolishes the tile the player was pointing at rather than reaching
/// into its neighbour. Once it is a real drag the run follows the pointer, and
/// a pointer that has left the map holds the last tile it had.
#[inline]
pub fn demolish_tip(origin: TileCoord, hover: Option<TileCoord>, moved: bool) -> TileCoord {
    if moved {
        hover.unwrap_or(origin)
    } else {
        origin
    }
}

/// Remember a smart proposal's shape for next frame's tie-breaking, or drop
/// the hold when the mode is pure or the route was refused.
fn update_smart_hold(state: &mut TrackToolState, preview: &BuildPreview) {
    if state.path_mode.is_smart() && preview.reject.is_none() {
        state.smart_hold = Some(preview.tiles.iter().map(|g| g.tile).collect());
    } else if !state.path_mode.is_smart() {
        state.smart_hold = None;
    }
}

/// Brief 04 §2.2's modifier table: none = smart, Shift = straight (ray snap,
/// terrain be damned), Ctrl = single tile, Alt = contour lock. Modifiers are
/// deliberately literal — they are chords, not rebindable verbs.
fn path_mode_from_keys(keys: &ButtonInput<KeyCode>) -> PathMode {
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let alt = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    if ctrl {
        PathMode::SingleTile
    } else if shift {
        PathMode::Autofill
    } else if alt {
        PathMode::ContourLock
    } else {
        PathMode::Smart
    }
}

pub fn track_tool_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
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
    if bindings.just_pressed(&keys, ControlAction::TrackTool) {
        state.tool = BuildTool::Build;
        state.suppress_build_click = false;
    }
    if bindings.just_pressed(&keys, ControlAction::DemolishTool) {
        state.tool = BuildTool::Demolish;
        state.suppress_build_click = false;
        state.clear_drag();
    }
    let had_pending =
        state.anchor.is_some() || state.drag.is_some() || state.drag_origin.is_some();
    if bindings.just_pressed(&keys, ControlAction::LookTool) {
        state.tool = BuildTool::Select;
        state.suppress_build_click = false;
        state.clear_drag();
    }
    if keys.just_pressed(KeyCode::Escape) {
        state.anchor = None;
        state.clear_drag();
        state.build_preview = None;
        state.demolish_preview = None;
        // Esc unwinds one layer at a time (brief 03 §10.1): first cancel a
        // pending drag or anchor, and only disarm the tool once there is
        // nothing left to cancel. Without the second step the player has no way
        // out of build mode at all.
        if !had_pending {
            state.tool = BuildTool::Select;
        }
    }

    let world = cursor_world(&windows, &camera_q);
    let hover = world
        .map(|w| world_to_tile(w.x, w.y))
        .filter(|&tile| map.contains(tile));
    state.hover_tile = hover;
    state.path_mode = path_mode_from_keys(&keys);

    // Select is a look-around mode: no ghost, no click-to-build.
    if state.tool == BuildTool::Select {
        state.anchor = None;
        state.clear_drag();
        state.build_preview = None;
        state.demolish_preview = None;
        return;
    }

    if state.suppress_build_click {
        state.clear_drag();
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

    // An armed tool owns the world left-press.
    //
    // This used to defer to `WorldClickConsumed`, and that is what made a
    // demolish click a no-op: the inspector's picker claims *every* press that
    // lands on a tile carrying track (it falls back to `TrackNetwork::id_at`
    // when no sprite is hit), so the one tile the player was pointing at was the
    // one tile Demolish could never touch. Tools that genuinely take the pointer
    // away — station and train placement, line editing, Map View — say so
    // through `suppress_build_click`, which is handled above; selection does not
    // need a second, silent veto on top of it.
    if mouse.just_pressed(MouseButton::Left) {
        if let Some(tile) = hover {
            match state.tool {
                // Handled by the early return above; clicks belong to selection.
                BuildTool::Select => {}
                BuildTool::Build => {
                    let origin = if state.path_mode == PathMode::SingleTile {
                        tile
                    } else {
                        state.anchor.unwrap_or(tile)
                    };
                    state.anchor = Some(origin);
                    state.begin_drag(DragKind::Build, origin, world, MouseButton::Left);
                }
                BuildTool::Demolish => {
                    state.begin_drag(DragKind::Demolish, tile, world, MouseButton::Left);
                }
            }
        }
    }

    if mouse.just_pressed(MouseButton::Right) {
        if let Some(tile) = hover {
            state.begin_drag(DragKind::Demolish, tile, world, MouseButton::Right);
            state.build_preview = None;
        }
    }

    if let Some(kind) = state.drag {
        if let (Some(press), Some(now)) = (state.drag_press_world, world) {
            if press_became_drag(press, now) {
                state.drag_moved = true;
            }
        }
        let origin = state.drag_origin.or(state.anchor);
        // Build keeps following the cursor: its origin is the run's anchor,
        // which is routinely nowhere near the press.
        let tip = match kind {
            DragKind::Demolish => origin.map(|o| demolish_tip(o, hover, state.drag_moved)),
            DragKind::Build => hover.or(origin),
        };
        if let (Some(from), Some(to)) = (origin, tip) {
            match kind {
                DragKind::Build => {
                    state.demolish_preview = None;
                    let preview = preview_build(
                        &network,
                        &terrain,
                        &money,
                        from,
                        to,
                        state.path_mode,
                        state.smart_hold.as_deref(),
                    );
                    update_smart_hold(&mut state, &preview);
                    state.build_preview = Some(preview);
                }
                DragKind::Demolish => {
                    state.build_preview = None;
                    state.demolish_preview = Some(preview_demolish(&network, &money, from, to));
                }
            }
        }
    } else if state.tool == BuildTool::Build {
        if let (Some(from), Some(to)) = (state.anchor, hover) {
            let preview = preview_build(
                &network,
                &terrain,
                &money,
                from,
                to,
                state.path_mode,
                state.smart_hold.as_deref(),
            );
            update_smart_hold(&mut state, &preview);
            state.build_preview = Some(preview);
            state.demolish_preview = None;
        } else {
            state.build_preview = None;
            state.demolish_preview = None;
            state.smart_hold = None;
        }
    } else {
        state.build_preview = None;
        state.demolish_preview = None;
        state.smart_hold = None;
    }

    let left_up = mouse.just_released(MouseButton::Left);
    let right_up = mouse.just_released(MouseButton::Right);
    // A demolish drag is committed by releasing the button that started it.
    // Accepting either release let the player press one button, release the
    // other, and fire a run they never gestured.
    let demolish_release = match state.drag_button {
        Some(MouseButton::Right) => right_up,
        _ => left_up,
    };

    if left_up && state.drag == Some(DragKind::Build) {
        commit_build(&mut state, &mut buffer, &mut feedback, &network);
        state.clear_drag();
    } else if demolish_release && state.drag == Some(DragKind::Demolish) {
        commit_demolish(&mut state, &mut buffer, &mut feedback, &network);
        state.clear_drag();
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
        PathMode::Smart | PathMode::ContourLock => {
            // The proposal already holds the routed tiles; commit exactly what
            // the ghost showed, as one atomic command and one undo entry.
            let tiles: Vec<TileCoord> = preview.tiles.iter().map(|g| g.tile).collect();
            if tiles.len() <= 1 {
                if network.id_at(to, GROUND_LAYER).is_none() {
                    buffer.push(CommandKind::PlaceTrack(PlaceTrack {
                        tile: to,
                        layer: GROUND_LAYER,
                    }));
                }
            } else {
                buffer.push(CommandKind::AutoFillPath(AutoFillPath {
                    tiles,
                    layer: GROUND_LAYER,
                }));
            }
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

#[cfg(test)]
mod tests {
    /// A demolish run is committed by the button that started it.
    ///
    /// Accepting either release let the player press one button, release the
    /// other, and fire a run they never gestured.
    #[test]
    fn only_the_button_that_started_the_drag_commits_it() {
        let mut state = TrackToolState::default();
        state.begin_drag(
            DragKind::Demolish,
            tile(1, 1),
            Some(Vec2::splat(48.0)),
            MouseButton::Right,
        );
        assert_eq!(state.drag_button, Some(MouseButton::Right));

        state.clear_drag();
        assert_eq!(state.drag_button, None, "clearing a press forgets its button");
    }

    use super::*;
    use rail_sim::track::{try_demolish, try_place_track};
    use rail_sim::MoneyLedger;

    use super::super::preview::preview_demolish;

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    struct Yard {
        network: TrackNetwork,
        terrain: TrackTerrain,
        money: Money,
        ledger: MoneyLedger,
    }

    impl Yard {
        fn new() -> Self {
            Self {
                network: TrackNetwork::new(),
                terrain: TrackTerrain::new(16, 16, (0..16 * 16).map(|_| (false, 0i8))),
                money: Money::new(50_000_000),
                ledger: MoneyLedger::default(),
            }
        }

        fn lay(&mut self, x: i32, y: i32) {
            try_place_track(
                &mut self.network,
                &mut self.money,
                &mut self.ledger,
                &self.terrain,
                tile(x, y),
                GROUND_LAYER,
            )
            .expect("place");
        }

        /// One click: press on `origin`, release with the pointer over `hover`
        /// without ever leaving the slop. Returns the tiles actually removed.
        fn click_demolish(&mut self, origin: TileCoord, hover: Option<TileCoord>) -> Vec<TileCoord> {
            self.gesture(origin, hover, false)
        }

        /// A press that travelled: the run follows the pointer.
        fn drag_demolish(&mut self, origin: TileCoord, hover: TileCoord) -> Vec<TileCoord> {
            self.gesture(origin, Some(hover), true)
        }

        fn gesture(
            &mut self,
            origin: TileCoord,
            hover: Option<TileCoord>,
            moved: bool,
        ) -> Vec<TileCoord> {
            let tip = demolish_tip(origin, hover, moved);
            let mut state = TrackToolState {
                tool: BuildTool::Demolish,
                drag: Some(DragKind::Demolish),
                drag_origin: Some(origin),
                drag_moved: moved,
                hover_tile: hover,
                ..Default::default()
            };
            state.demolish_preview =
                Some(preview_demolish(&self.network, &self.money, origin, tip));

            let mut buffer = CommandBuffer::new();
            let mut feedback = BuildFeedback::default();
            commit_demolish(&mut state, &mut buffer, &mut feedback, &self.network);

            let mut removed = Vec::new();
            for command in buffer.drain() {
                let CommandKind::Demolish(d) = command.kind else {
                    panic!("demolish must only ever emit Demolish commands");
                };
                let piece = try_demolish(
                    &mut self.network,
                    &mut self.money,
                    &mut self.ledger,
                    d.track,
                )
                .expect("the id the tool emitted must still be in the network");
                removed.push(piece.tile);
            }
            removed
        }
    }

    /// The bug, in one line: a click on a tile with track takes *that* tile.
    #[test]
    fn a_single_click_demolishes_exactly_the_tile_under_it() {
        let mut yard = Yard::new();
        for x in 3..=7 {
            yard.lay(x, 5);
        }
        assert_eq!(yard.click_demolish(tile(5, 5), Some(tile(5, 5))), [tile(5, 5)]);
        assert!(yard.network.id_at(tile(5, 5), GROUND_LAYER).is_none());
        // Both neighbours survive: the click reached sideways for nothing.
        assert!(yard.network.id_at(tile(4, 5), GROUND_LAYER).is_some());
        assert!(yard.network.id_at(tile(6, 5), GROUND_LAYER).is_some());
        assert_eq!(yard.network.len(), 4);
    }

    /// The reported symptom: the last piece of a line has to go too, and the
    /// network has to end up genuinely empty.
    #[test]
    fn clicking_the_last_piece_of_a_line_empties_the_network() {
        let mut yard = Yard::new();
        for x in 3..=6 {
            yard.lay(x, 5);
        }
        for x in (3..=6).rev() {
            let target = tile(x, 5);
            assert_eq!(
                yard.click_demolish(target, Some(target)),
                [target],
                "clicking {target:?} must remove {target:?}"
            );
        }
        assert!(yard.network.is_empty(), "the line must demolish down to nothing");
    }

    /// A one-piece network is the degenerate case of the same thing: no
    /// neighbours to relink, nothing for the run to snap onto.
    #[test]
    fn a_one_piece_network_clicks_away_to_empty() {
        let mut yard = Yard::new();
        yard.lay(8, 8);
        assert_eq!(yard.network.len(), 1);
        assert_eq!(yard.click_demolish(tile(8, 8), Some(tile(8, 8))), [tile(8, 8)]);
        assert!(yard.network.is_empty());
    }

    /// The hazard the slop exists for: the pointer crosses a tile boundary
    /// between press and release. Without it the run snaps out to the neighbour
    /// and takes a tile the player never pointed at.
    #[test]
    fn a_click_that_wobbles_into_the_next_tile_still_takes_only_its_own() {
        for drift in [tile(6, 5), tile(4, 5), tile(5, 6), tile(5, 4)] {
            let mut yard = Yard::new();
            for x in 3..=7 {
                yard.lay(x, 5);
            }
            yard.lay(5, 4);
            yard.lay(5, 6);
            assert_eq!(
                yard.click_demolish(tile(5, 5), Some(drift)),
                [tile(5, 5)],
                "a click that drifted toward {drift:?} reached into it"
            );
            assert!(yard.network.id_at(drift, GROUND_LAYER).is_some());
        }
    }

    /// The slop only silences the wobble; a gesture that really travelled still
    /// demolishes the run it drew.
    #[test]
    fn a_real_drag_still_demolishes_the_whole_run() {
        let mut yard = Yard::new();
        for x in 3..=7 {
            yard.lay(x, 5);
        }
        let mut removed = yard.drag_demolish(tile(3, 5), tile(6, 5));
        removed.sort_by_key(|t| t.x);
        assert_eq!(
            removed,
            vec![tile(3, 5), tile(4, 5), tile(5, 5), tile(6, 5)]
        );
        assert_eq!(yard.network.len(), 1);
    }

    #[test]
    fn the_slop_is_a_disc_around_the_press_point() {
        let press = Vec2::new(100.0, 100.0);
        assert!(!press_became_drag(press, press));
        assert!(!press_became_drag(press, press + Vec2::new(DRAG_SLOP_TEXELS - 0.5, 0.0)));
        assert!(press_became_drag(press, press + Vec2::new(DRAG_SLOP_TEXELS + 0.5, 0.0)));
        // Smaller than a tile in every direction, so the slop can never swallow
        // a deliberate one-tile drag.
        assert!(DRAG_SLOP_TEXELS < rail_map::TILE_SIZE * 0.5);
        for dir in [Vec2::Y, -Vec2::Y, -Vec2::X, Vec2::ONE.normalize()] {
            assert!(press_became_drag(press, press + dir * rail_map::TILE_SIZE));
        }
    }

    #[test]
    fn demolish_tip_holds_the_press_tile_until_the_gesture_travels() {
        let origin = tile(2, 2);
        assert_eq!(demolish_tip(origin, Some(tile(9, 9)), false), origin);
        assert_eq!(demolish_tip(origin, Some(tile(9, 9)), true), tile(9, 9));
        // Pointer off the map mid-drag: hold what we had rather than jumping.
        assert_eq!(demolish_tip(origin, None, true), origin);
    }

    /// Switching tools has to leave no half-captured press behind.
    #[test]
    fn clearing_a_drag_forgets_every_part_of_it() {
        let mut state = TrackToolState::default();
        state.begin_drag(
            DragKind::Demolish,
            tile(1, 1),
            Some(Vec2::splat(48.0)),
            MouseButton::Left,
        );
        state.drag_moved = true;
        state.clear_drag();
        assert!(state.drag.is_none());
        assert!(state.drag_origin.is_none());
        assert!(state.drag_press_world.is_none());
        assert!(!state.drag_moved);
    }
}
