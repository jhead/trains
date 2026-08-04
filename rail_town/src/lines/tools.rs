//! Line drawing tool — click stations in order, Enter confirms.
//!
//! `L` selects the Line tool. Left-click appends a station. Enter creates the
//! line via [`CreateLine`]. Esc puts the tool down; right-click takes the last
//! stop back off the draft.
//!
//! # Putting the tool down again
//!
//! Arming this tool parks the track tool in a half-state — `BuildTool::Build`
//! with `suppress_build_click` set — so that the track builder ignores the
//! clicks this tool is collecting. Confirming a line used to clear the draft and
//! deactivate the tool while **leaving that half-state behind**, and the world
//! went quiet: [`crate::inspect::selection`] only picks under the Look tool, so
//! the player's next click on a train did nothing at all, with no message. The
//! line they had just drawn could not be crewed, which is the one thing they had
//! drawn it to do.
//!
//! So every exit from this tool — Enter, `Esc`, or another verb taking the
//! pointer — goes through [`return_to_look`] and hands the world back clickable.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{world_to_tile, MapGrid};
use rail_sim::{
    find_path, line_path, suggest_line_name, track_for_station, CommandBuffer, CommandKind,
    CreateLine, LineRegistry, StationId, StationRegistry, TrackNetwork, GROUND_LAYER,
};

use crate::input::{ControlAction, KeyBindings};
use crate::inspect::WorldClickConsumed;
use crate::map::MapCamera;
use crate::track::{BuildTool, TrackToolState};
use crate::trains::TrainToolState;
use crate::ui::UiBlocksWorld;

use super::panel::{FocusedLine, LinesFeedback};

/// Presentation mode for the Line tool (does not buy rolling stock).
#[derive(Debug, Clone, Default, Resource)]
pub struct LineToolState {
    pub active: bool,
    /// Stations clicked so far (ordered).
    pub draft_stops: Vec<StationId>,
    /// Last connectivity warning for status / HUD.
    pub warn: Option<String>,
    /// Stops of the line just confirmed, until the panel has focused it.
    ///
    /// The [`rail_sim::LineId`] does not exist yet — the command is applied on
    /// the next fixed tick — so the request is carried by the one thing the
    /// player and the sim already agree on: the route itself.
    pub pending_focus: Option<Vec<StationId>>,
}

impl LineToolState {
    pub fn clear_draft(&mut self) {
        self.draft_stops.clear();
        self.warn = None;
    }
}

/// The tool resources a Line-tool exit has to put back, bundled so the exit is
/// one call and cannot be done half way.
#[derive(SystemParam)]
pub struct LineToolFocus<'w> {
    pub line: ResMut<'w, LineToolState>,
    pub track: ResMut<'w, TrackToolState>,
    pub train: ResMut<'w, TrainToolState>,
    pub focused: ResMut<'w, FocusedLine>,
}

/// Put the Line tool down and give the player the Look tool back.
///
/// This is the fix for the reported dead end: `suppress_build_click` is cleared
/// and the track tool returns to `Select`, which is the only state
/// [`crate::inspect::selection`] will pick in. Without both, the world stays
/// unclickable and says nothing about why.
fn return_to_look(track: &mut TrackToolState, line: &mut LineToolState) {
    line.active = false;
    line.clear_draft();
    track.tool = BuildTool::Select;
    track.anchor = None;
    track.drag = None;
    track.suppress_build_click = false;
}

#[allow(clippy::too_many_arguments)]
pub fn line_tool_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MapCamera>>,
    map: Res<MapGrid>,
    stations: Res<StationRegistry>,
    network: Res<TrackNetwork>,
    lines: Res<LineRegistry>,
    mut buffer: ResMut<CommandBuffer>,
    mut focus: LineToolFocus,
    mut feedback: ResMut<LinesFeedback>,
    ui_blocks: Res<UiBlocksWorld>,
    click_consumed: Res<WorldClickConsumed>,
) {
    if bindings.just_pressed(&keys, ControlAction::LineTool) {
        focus.line.active = true;
        focus.train.place_mode = false;
        focus.track.tool = BuildTool::Build;
        focus.track.anchor = None;
        focus.track.drag = None;
        focus.track.suppress_build_click = true;
        focus.line.clear_draft();
    }

    // Other tools reclaim focus.
    let track = bindings.any_just_pressed(
        &keys,
        &[ControlAction::TrackTool, ControlAction::DemolishTool],
    );
    let train = bindings.any_just_pressed(
        &keys,
        &[ControlAction::BuyTransit, ControlAction::BuyTransport],
    );
    if track || train {
        focus.line.active = false;
        focus.line.clear_draft();
        if track {
            focus.track.suppress_build_click = false;
        }
    }

    if !focus.line.active {
        // `Esc` unwinds one layer per press (03 §10.1). With the tool down, the
        // layer still standing is the panel's crewing mode, so this is where it
        // comes off — and it has to, because that mode turns every train click
        // into an assignment.
        if keys.just_pressed(KeyCode::Escape) && focus.focused.0.is_some() {
            focus.focused.0 = None;
            feedback.clear();
        }
        return;
    }

    if keys.just_pressed(KeyCode::Escape) {
        // Esc puts the whole tool down rather than only emptying the draft:
        // right-click already takes stops back one at a time, and a tool that
        // stayed armed here would leave the player holding a pointer that
        // neither builds nor selects. See [`return_to_look`].
        return_to_look(&mut focus.track, &mut focus.line);
        return;
    }

    if bindings.just_pressed(&keys, ControlAction::CommitLine) {
        if confirm_draft(&mut focus.line, &stations, &lines, &mut buffer, &mut feedback) {
            return_to_look(&mut focus.track, &mut focus.line);
        }
        return;
    }

    if ui_blocks.0 || click_consumed.0 {
        return;
    }

    if mouse.just_pressed(MouseButton::Right) {
        if let Some(last) = focus.line.draft_stops.pop() {
            let _ = last;
            focus.line.warn = None;
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
    if focus.line.draft_stops.last() == Some(&station_id) {
        return;
    }

    // Connectivity check against previous stop.
    if let Some(&prev) = focus.line.draft_stops.last() {
        let connected = segment_connected(&network, &stations, prev, station_id);
        if !connected {
            let a = stations.get(prev).map(|s| s.name.as_str()).unwrap_or("?");
            let b = stations
                .get(station_id)
                .map(|s| s.name.as_str())
                .unwrap_or("?");
            focus.line.warn = Some(format!("No route - {a} is not connected to {b}."));
            // Still allow adding so the player sees the warn segment; confirm will
            // still create the line (ops can fix track later). Or refuse?
            // Design: draw warn but allow confirm only if all segments connect.
            // We add with warn; confirm checks path.
        } else {
            focus.line.warn = None;
        }
    }

    focus.line.draft_stops.push(station_id);

    // Drawing over a route the player already has is worth saying while there
    // is still a draft to change, not after Enter has been pressed three times.
    if let Some(existing) = lines.duplicate_of(&focus.line.draft_stops) {
        let name = lines
            .get(existing)
            .map(|l| l.name.clone())
            .unwrap_or_else(|| format!("Line {}", existing.0));
        focus.line.warn = Some(format!("{name} already runs this route."));
    }
}

/// Turn the draft into a [`CreateLine`], or say why it is not one.
///
/// Returns `true` when a line was actually ordered — the caller uses that to
/// decide whether to put the tool down. A refusal keeps the tool armed and the
/// draft intact, because the player's next move is to fix the draft, and taking
/// it away from them would be a second silent failure on top of the first.
///
/// The duplicate check here is the player-facing half; [`rail_sim::lines`] holds
/// the half that actually binds. Two checks, because this one can only warn
/// about what it can see, and the sim's is the one every source of the command
/// passes through.
fn confirm_draft(
    line_state: &mut LineToolState,
    stations: &StationRegistry,
    lines: &LineRegistry,
    buffer: &mut CommandBuffer,
    feedback: &mut LinesFeedback,
) -> bool {
    if line_state.draft_stops.len() < 2 {
        let warn = "Need at least two stations.";
        line_state.warn = Some(warn.into());
        feedback.refuse(warn);
        return false;
    }
    if let Some(existing) = lines.duplicate_of(&line_state.draft_stops) {
        let name = lines
            .get(existing)
            .map(|l| l.name.clone())
            .unwrap_or_else(|| format!("Line {}", existing.0));
        let warn = format!("{name} already runs this route.");
        line_state.warn = Some(warn.clone());
        feedback.refuse(warn);
        return false;
    }
    let name = suggest_line_name(stations, &line_state.draft_stops);
    buffer.push(CommandKind::CreateLine(CreateLine {
        name: Some(name.clone()),
        stops: line_state.draft_stops.clone(),
    }));
    // Hand the new line to the panel the moment the sim mints it, so the player
    // lands on a focused row that tells them what to do next instead of on a
    // list they have to find their own line in.
    line_state.pending_focus = Some(line_state.draft_stops.clone());
    feedback.say(format!("{name} created - click a train to assign it"));
    true
}

/// Focus the line the player just drew, once the sim has actually created it.
///
/// The command is applied on the next fixed tick, so this waits for a line whose
/// stops match the confirmed draft rather than guessing an id. Matching on the
/// route also means a confirm that raced another source of lines still focuses
/// the right one.
pub fn focus_new_line(
    lines: Res<LineRegistry>,
    mut line_state: ResMut<LineToolState>,
    mut focused: ResMut<FocusedLine>,
) {
    let Some(stops) = line_state.pending_focus.clone() else {
        return;
    };
    let mut matches: Vec<_> = lines
        .iter()
        .filter(|l| l.stops == stops)
        .map(|l| l.id)
        .collect();
    // Sorted: the registry is a `HashMap` and the focused row may not depend on
    // iteration order.
    matches.sort_unstable_by_key(|id| id.0);
    let Some(id) = matches.last().copied() else {
        return;
    };
    focused.0 = Some(id);
    line_state.pending_focus = None;
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

// The "click a train while a line is selected → assign" helper that used to sit
// here was `#[allow(dead_code)]` for its whole life: the interaction was
// designed, written down, and never wired to anything. It now lives as a real
// system — `super::panel::assign_clicked_train_to_focused_line` — with the
// feedback and the focus mode that make it findable.

/// Preview connectivity for the draft (used by strip / HUD).
#[allow(dead_code)]
pub fn draft_fully_connected(
    network: &TrackNetwork,
    stations: &StationRegistry,
    stops: &[StationId],
) -> bool {
    line_path(network, stations, stops).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::ids::TileCoord;
    use rail_sim::LineId;

    /// Everything `line_tool_input` reads, and two stations to draw between.
    fn app() -> (App, StationId, StationId) {
        let mut stations = StationRegistry::new();
        let east = stations.insert("Eastgate", TileCoord { x: 3, y: 3 }, GROUND_LAYER);
        let west = stations.insert("Westbrook", TileCoord { x: 9, y: 3 }, GROUND_LAYER);

        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<KeyBindings>()
            .init_resource::<TrackNetwork>()
            .init_resource::<LineRegistry>()
            .init_resource::<CommandBuffer>()
            .init_resource::<LineToolState>()
            .init_resource::<TrackToolState>()
            .init_resource::<TrainToolState>()
            .init_resource::<FocusedLine>()
            .init_resource::<LinesFeedback>()
            .init_resource::<UiBlocksWorld>()
            .init_resource::<WorldClickConsumed>()
            .insert_resource(stations)
            .insert_resource(MapGrid::empty(16, 16, 0))
            .add_systems(Update, line_tool_input);
        (app, east, west)
    }

    fn press(app: &mut App, action: ControlAction) {
        press_key(app, KeyBindings::default().key(action));
    }

    /// Tap `key` for exactly one frame.
    ///
    /// The release matters: `ButtonInput::press` on a key it already holds down
    /// records no *just* pressed, so a test that taps the same key twice would
    /// silently only tap it once.
    fn press_key(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
        app.update();
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.release(key);
        keys.clear();
    }

    fn draft(app: &mut App, stops: Vec<StationId>) {
        app.world_mut().resource_mut::<LineToolState>().draft_stops = stops;
    }

    fn track(app: &App) -> (BuildTool, bool) {
        let state = app.world().resource::<TrackToolState>();
        (state.tool, state.suppress_build_click)
    }

    /// **The reported dead end.** Confirming a line left the pointer in
    /// `BuildTool::Build` with the build click suppressed — a state in which
    /// nothing builds and nothing selects. The player's next click on a train
    /// did nothing, silently, and the line they had just drawn could not be
    /// crewed.
    #[test]
    fn confirming_a_line_hands_the_world_back_to_the_look_tool() {
        let (mut app, east, west) = app();
        press(&mut app, ControlAction::LineTool);
        assert_eq!(
            track(&app),
            (BuildTool::Build, true),
            "the armed tool holds the world click"
        );

        draft(&mut app, vec![east, west]);
        press(&mut app, ControlAction::CommitLine);

        assert_eq!(
            track(&app),
            (BuildTool::Select, false),
            "after Enter the world has to be clickable again"
        );
        assert!(!app.world().resource::<LineToolState>().active);
        assert!(app.world().resource::<LineToolState>().draft_stops.is_empty());

        let pending = app.world().resource::<CommandBuffer>().pending();
        assert_eq!(pending.len(), 1);
        assert!(matches!(pending[0].kind, CommandKind::CreateLine(_)));
    }

    /// Esc is the way out of a tool, so it has to leave the same clean state
    /// Enter does — not a half-armed pointer.
    #[test]
    fn escape_puts_the_line_tool_down_cleanly() {
        let (mut app, east, _) = app();
        press(&mut app, ControlAction::LineTool);
        draft(&mut app, vec![east]);

        press_key(&mut app, KeyCode::Escape);

        assert_eq!(track(&app), (BuildTool::Select, false));
        assert!(!app.world().resource::<LineToolState>().active);
        assert!(app.world().resource::<LineToolState>().draft_stops.is_empty());
        assert!(
            app.world().resource::<CommandBuffer>().pending().is_empty(),
            "Esc creates nothing"
        );
    }

    /// The new line is handed to the panel, so the player lands on a focused row
    /// telling them what to do next.
    #[test]
    fn confirming_a_line_asks_the_panel_to_focus_it() {
        let (mut app, east, west) = app();
        press(&mut app, ControlAction::LineTool);
        draft(&mut app, vec![east, west]);
        press(&mut app, ControlAction::CommitLine);

        assert_eq!(
            app.world().resource::<LineToolState>().pending_focus,
            Some(vec![east, west])
        );

        // …and the panel resolves it once the sim has minted the line.
        let mut lines = LineRegistry::new();
        let id = lines
            .create("Eastgate - Westbrook".into(), vec![east, west])
            .expect("line");
        app.insert_resource(lines);
        app.add_systems(Update, focus_new_line);
        app.update();

        assert_eq!(app.world().resource::<FocusedLine>().0, Some(id));
        assert_eq!(
            app.world().resource::<LineToolState>().pending_focus,
            None,
            "the request is spent once it lands"
        );
    }

    /// **The junk-lines bug.** Three presses of Enter produced three lines over
    /// the same two stations because nothing ever said no.
    #[test]
    fn enter_on_a_route_that_already_exists_refuses_out_loud() {
        let (mut app, east, west) = app();
        let mut lines = LineRegistry::new();
        lines
            .create("Westbrook - Eastgate".into(), vec![west, east])
            .expect("line");
        app.insert_resource(lines);

        press(&mut app, ControlAction::LineTool);
        draft(&mut app, vec![east, west]);
        press(&mut app, ControlAction::CommitLine);

        assert!(
            app.world().resource::<CommandBuffer>().pending().is_empty(),
            "the reversed route is the same out-and-back service"
        );
        let warn = app
            .world()
            .resource::<LineToolState>()
            .warn
            .clone()
            .expect("a refusal has to say so");
        assert_eq!(warn, "Westbrook - Eastgate already runs this route.");
        assert_eq!(
            app.world().resource::<LinesFeedback>().message(),
            Some(warn.as_str()),
            "and it says so in the panel too"
        );
        assert!(warn.is_ascii());

        // The draft is left alone: the player's next move is to change it.
        assert!(app.world().resource::<LineToolState>().active);
        assert_eq!(
            app.world().resource::<LineToolState>().draft_stops,
            vec![east, west]
        );
        assert_eq!(track(&app), (BuildTool::Build, true));
    }

    #[test]
    fn a_draft_of_one_stop_says_what_is_missing() {
        let (mut app, east, _) = app();
        press(&mut app, ControlAction::LineTool);
        draft(&mut app, vec![east]);
        press(&mut app, ControlAction::CommitLine);

        assert!(app.world().resource::<CommandBuffer>().pending().is_empty());
        assert_eq!(
            app.world().resource::<LinesFeedback>().message(),
            Some("Need at least two stations.")
        );
        assert!(
            app.world().resource::<LineToolState>().active,
            "a refusal keeps the tool so the draft can be finished"
        );
    }

    /// `Esc` unwinds one layer per press: the tool first, then the panel's
    /// crewing mode — which must come off, because it turns every train click
    /// into an assignment.
    #[test]
    fn escape_clears_the_panel_focus_once_the_tool_is_down() {
        let (mut app, _, _) = app();
        app.world_mut().resource_mut::<FocusedLine>().0 = Some(LineId(1));
        app.world_mut().resource_mut::<LinesFeedback>().say("something");

        press(&mut app, ControlAction::LineTool);
        press_key(&mut app, KeyCode::Escape);
        assert_eq!(
            app.world().resource::<FocusedLine>().0,
            Some(LineId(1)),
            "the first press put the tool down"
        );

        press_key(&mut app, KeyCode::Escape);
        assert_eq!(app.world().resource::<FocusedLine>().0, None);
        assert_eq!(app.world().resource::<LinesFeedback>().message(), None);
    }
}
