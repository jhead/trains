//! Station tool — build platforms on the line, upgrade them in place.
//!
//! A station is a kind of track, so this tool behaves like the track tools:
//! hover shows a live ghost with the catchment ring and its cost, left click
//! commits, and every rejection names its rule.
//!
//! - **The Station slot on the menu row**, or `P` — arm the tool. `P` again
//!   cycles Halt → Station → Interchange → Terminus → Goods Platform, and the
//!   tier row under the slot picks one directly, with its price.
//! - Left click — build the selected tier on the track under the cursor
//! - `U` — upgrade the station under the cursor to the next tier up. The
//!   Inspector's card offers the same thing with the price on it.
//! - Right click — lift the station under the cursor (full refund). When lines
//!   call there it asks first, naming them (04 §4).
//! - `Esc` — leave the tool; `B` / `X` / `T` / `G` / `L` reclaim their own, and
//!   so does any other slot on the row (`ui::toolbar::apply_toolbar_tool`).

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{world_to_tile, MapGrid};
use rail_sim::ids::TileCoord;
use rail_sim::stations::{
    push_station_command, DemolishStation, PlaceStation, StationCommand, StationPlacementError,
    StationRegistry, StationTier, UpgradeStation,
};
use rail_sim::{
    CommandBuffer, DemandSpawner, IndustryRegistry, LineRegistry, Money, StationEdit, StationId,
    TownDensity, TrackNetwork, TrackTerrain, GROUND_LAYER,
};

use crate::input::{ControlAction, KeyBindings};
use crate::inspect::WorldClickConsumed;
use crate::lines::LineToolState;
use crate::map::MapCamera;
use crate::track::{BuildTool, TrackToolState};
use crate::trains::TrainToolState;
use crate::ui::{ConfirmAccepted, ConfirmAction, ConfirmDialog, ConfirmPrompt, UiBlocksWorld};

use super::preview::{demolish_consequence, preview_station, station_reason, StationPreview};

/// Tier order the `P` key cycles through.
const TIER_CYCLE: [StationTier; 5] = [
    StationTier::Halt,
    StationTier::Station,
    StationTier::Interchange,
    StationTier::Terminus,
    StationTier::GoodsPlatform,
];

#[derive(Debug, Clone, Default, Resource)]
pub struct StationToolState {
    /// When true, left-click builds a platform instead of track.
    pub active: bool,
    pub tier: StationTier,
    pub hover_tile: Option<TileCoord>,
    pub preview: Option<StationPreview>,
    /// Sticky rejection message (survives past the click that caused it).
    pub reject: Option<String>,
}

impl StationToolState {
    /// Advance to the next tier in the `P` cycle.
    pub fn cycle_tier(&mut self) {
        let i = TIER_CYCLE
            .iter()
            .position(|t| *t == self.tier)
            .unwrap_or(0);
        self.tier = TIER_CYCLE[(i + 1) % TIER_CYCLE.len()];
    }

    /// Put the tool down: no ghost, no sticky refusal, no armed pointer.
    ///
    /// Public because the menu row disarms it whenever another verb is armed —
    /// see `ui::toolbar::apply_toolbar_tool`.
    pub fn disarm(&mut self) {
        self.active = false;
        self.preview = None;
        self.reject = None;
    }
}

/// Everything the preview reads about the world (grouped to stay inside
/// Bevy's system-parameter budget).
#[derive(SystemParam)]
pub struct SiteWorld<'w> {
    stations: Res<'w, StationRegistry>,
    industries: Res<'w, IndustryRegistry>,
    network: Res<'w, TrackNetwork>,
    density: Res<'w, TownDensity>,
    demand: Res<'w, DemandSpawner>,
    money: Res<'w, Money>,
    lines: Res<'w, LineRegistry>,
    /// Only so a refusal over open water can say *water* rather than *no track*.
    /// `Option` because the terrain arrives with the map, one frame behind boot.
    terrain: Option<Res<'w, TrackTerrain>>,
}

/// The other tools' focus state, so `P` can take the pointer cleanly.
#[derive(SystemParam)]
pub struct ToolFocus<'w> {
    track: ResMut<'w, TrackToolState>,
    train: ResMut<'w, TrainToolState>,
    line: ResMut<'w, LineToolState>,
    ui_blocks: Res<'w, UiBlocksWorld>,
    click_consumed: Res<'w, WorldClickConsumed>,
}

/// Cursor tile under the map camera, if on-map.
fn cursor_tile(
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

#[allow(clippy::too_many_arguments)]
pub fn station_tool_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MapCamera>>,
    map: Res<MapGrid>,
    world: SiteWorld,
    mut buffer: ResMut<CommandBuffer>,
    mut state: ResMut<StationToolState>,
    mut focus: ToolFocus,
    mut confirm: ResMut<ConfirmDialog>,
) {
    // A question on screen owns the keyboard and the pointer until it is
    // answered — the tool must not build under its own dialog.
    if confirm.is_open() {
        return;
    }
    if bindings.just_pressed(&keys, ControlAction::PlaceStation) {
        if state.active {
            state.cycle_tier();
        } else {
            state.active = true;
            focus.train.place_mode = false;
            focus.line.active = false;
            focus.line.clear_draft();
            focus.track.tool = BuildTool::Build;
            focus.track.anchor = None;
            focus.track.drag = None;
        }
        state.reject = None;
        // Hold the world click for us while the tool is up.
        focus.track.suppress_build_click = true;
    }

    // Other tools reclaim focus (they each clear `suppress_build_click` themselves).
    if bindings.any_just_pressed(
        &keys,
        &[
            ControlAction::TrackTool,
            ControlAction::DemolishTool,
            ControlAction::BuyTransit,
            ControlAction::BuyTransport,
            ControlAction::LineTool,
        ],
    ) {
        state.disarm();
    }

    if !state.active {
        return;
    }

    if keys.just_pressed(KeyCode::Escape) {
        state.disarm();
        focus.track.suppress_build_click = false;
        return;
    }

    let hover = cursor_tile(&windows, &camera_q, &map);
    state.hover_tile = hover;

    let tier = state.tier;
    let terrain = world.terrain.as_deref();
    state.preview = hover.map(|tile| {
        preview_station(
            &world.stations,
            &world.industries,
            &world.network,
            &world.density,
            &world.demand,
            &world.money,
            terrain,
            tile,
            tier,
        )
    });
    // A stale reason must not outlive the site that caused it.
    if state.preview.as_ref().is_some_and(|p| p.can_commit) {
        state.reject = None;
    }

    // Upgrade works off the hovered stop whether or not the pointer is free.
    if bindings.just_pressed(&keys, ControlAction::UpgradeStation) {
        upgrade_under_cursor(&mut state, &world.stations, &mut buffer);
        return;
    }

    if focus.ui_blocks.0 || focus.click_consumed.0 {
        return;
    }

    if mouse.just_pressed(MouseButton::Left) {
        commit_place(&mut state, &mut buffer);
    } else if mouse.just_pressed(MouseButton::Right) {
        demolish_under_cursor(
            &mut state,
            &world.stations,
            &world.lines,
            &mut buffer,
            &mut confirm,
        );
    }
}

/// Carry out a demolish the player agreed to in the dialog.
///
/// The dialog never touches the sim: it hands back the action and the tool that
/// raised it issues the command, on the tick boundary like every other intent.
pub fn apply_confirmed_demolish(
    mut accepted: MessageReader<ConfirmAccepted>,
    mut buffer: ResMut<CommandBuffer>,
    mut state: ResMut<StationToolState>,
) {
    for ConfirmAccepted(action) in accepted.read() {
        // One dialog serves every confirmable action, so each tool answers only
        // for its own — a train sale is somebody else's yes.
        let ConfirmAction::DemolishStation(station) = action else {
            continue;
        };
        state.reject = None;
        push_station_command(
            &mut buffer,
            StationCommand::Demolish(DemolishStation { station: *station }),
        );
    }
}

fn commit_place(state: &mut StationToolState, buffer: &mut CommandBuffer) {
    let Some(preview) = state.preview.clone() else {
        return;
    };
    if let Some(reason) = preview.reject.clone() {
        state.reject = Some(reason);
        return;
    }
    state.reject = None;
    push_station_command(
        buffer,
        StationCommand::Place(PlaceStation::new(
            preview.tile,
            GROUND_LAYER,
            preview.tier,
            None,
        )),
    );
}

fn upgrade_under_cursor(
    state: &mut StationToolState,
    stations: &StationRegistry,
    buffer: &mut CommandBuffer,
) {
    let Some(tile) = state.hover_tile else {
        return;
    };
    let Some(station) = stations.at(tile, GROUND_LAYER) else {
        state.reject = Some(station_reason(StationPlacementError::UnknownStation, 0, 0));
        return;
    };
    let Some(to) = station.tier.next_upgrade() else {
        state.reject = Some(station_reason(
            StationPlacementError::NotUpgradable {
                from: station.tier,
                to: station.tier,
            },
            0,
            0,
        ));
        return;
    };
    state.reject = None;
    push_station_command(
        buffer,
        StationCommand::Upgrade(UpgradeStation {
            station: station.id,
            to,
        }),
    );
}

fn demolish_under_cursor(
    state: &mut StationToolState,
    stations: &StationRegistry,
    lines: &LineRegistry,
    buffer: &mut CommandBuffer,
    confirm: &mut ConfirmDialog,
) {
    let Some(tile) = state.hover_tile else {
        return;
    };
    let Some(station) = stations.at(tile, GROUND_LAYER) else {
        state.reject = Some(station_reason(StationPlacementError::UnknownStation, 0, 0));
        return;
    };
    state.reject = None;
    request_demolish(station.id, lines, buffer, confirm);
}

/// Lift `station`, asking first when lifting it costs somebody a call.
///
/// 04 §4: a demolish with a consequence **asks and names it**; everything else
/// is a plain reversible lift and goes straight through. Shared with the
/// Inspector's Demolish button so the right-click and the panel cannot disagree
/// about which lifts are worth a question.
pub fn request_demolish(
    station: StationId,
    lines: &LineRegistry,
    buffer: &mut CommandBuffer,
    confirm: &mut ConfirmDialog,
) {
    if let Some(body) = demolish_consequence(lines, station) {
        confirm.ask(ConfirmPrompt {
            title: "Demolish station".into(),
            body,
            confirm: "Demolish".into(),
            action: ConfirmAction::DemolishStation(station),
        });
        return;
    }
    push_station_command(buffer, StationCommand::Demolish(DemolishStation { station }));
}

/// Give a refusal the sim made a voice at the cursor.
///
/// The ghost speaks for everything the preview can see, but the preview is not
/// the last word — the sim re-validates every command, and a refusal that only
/// happens there used to vanish silently: [`StationEdit::Failed`] had no reader
/// in this crate at all. 04 §3 is explicit that silent rejection is the worst
/// thing an interface can do, so the reason lands on the same sticky line the
/// ghost uses.
pub fn speak_station_refusals(
    mut edits: MessageReader<StationEdit>,
    mut state: ResMut<StationToolState>,
) {
    for edit in edits.read() {
        match edit {
            StationEdit::Failed { error, .. } => {
                // The price that was refused is not in the message — a retier
                // charges a difference, not a build cost — so the funds line
                // falls back to its priceless wording rather than inventing a
                // number. Every other reason carries its own.
                state.reject = Some(station_reason(*error, 0, 0));
            }
            // Something landed: whatever was refused before is history.
            _ => state.reject = None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::{CommandKind, StationId};

    #[test]
    fn saying_yes_in_the_dialog_buffers_the_demolish_command() {
        let mut app = App::new();
        app.init_resource::<CommandBuffer>()
            .init_resource::<StationToolState>()
            .add_message::<ConfirmAccepted>()
            .add_systems(Update, apply_confirmed_demolish);

        // Nothing agreed to yet: the sim hears nothing.
        app.update();
        assert!(app.world().resource::<CommandBuffer>().pending().is_empty());

        app.world_mut()
            .write_message(ConfirmAccepted(ConfirmAction::DemolishStation(StationId(7))));
        app.update();

        let pending = app.world().resource::<CommandBuffer>().pending();
        assert_eq!(pending.len(), 1, "one command per agreement");
        assert!(
            matches!(pending[0].kind, CommandKind::DemolishStation(d) if d.station == StationId(7)),
            "the dialog's yes becomes the command, on the tick boundary"
        );
    }

    /// 04 §3: silent rejection is the worst thing an interface can do. The
    /// ghost speaks for what the preview can see, but the sim gets the last
    /// word on every command — and until now nothing in this crate read
    /// [`StationEdit::Failed`], so an upgrade the sim refused said nothing at
    /// all.
    #[test]
    fn a_refusal_from_the_sim_still_reaches_the_cursor() {
        let mut app = App::new();
        app.init_resource::<StationToolState>()
            .add_message::<StationEdit>()
            .add_systems(Update, speak_station_refusals);

        app.world_mut().write_message(StationEdit::Failed {
            error: StationPlacementError::NoPlatformRoom { have: 3, need: 4 },
            tile: Some(TileCoord { x: 6, y: 8 }),
        });
        app.update();
        assert_eq!(
            app.world().resource::<StationToolState>().reject.as_deref(),
            Some("Not enough platform - 3 tiles of line, needs 4"),
            "the rule and both its numbers"
        );

        // Something landing clears the last refusal rather than leaving it up.
        app.world_mut().write_message(StationEdit::Retiered {
            id: StationId(1),
            tile: TileCoord { x: 6, y: 8 },
            from: StationTier::Halt,
            to: StationTier::Station,
        });
        app.update();
        assert!(app.world().resource::<StationToolState>().reject.is_none());

        // A refused spend has no price to quote here, and says so rather than
        // claiming the player is short by nothing.
        app.world_mut().write_message(StationEdit::Failed {
            error: StationPlacementError::InsufficientFunds,
            tile: None,
        });
        app.update();
        assert_eq!(
            app.world().resource::<StationToolState>().reject.as_deref(),
            Some("Not enough money for that")
        );
    }

    #[test]
    fn p_cycles_every_tier_and_returns_to_the_start() {
        let mut state = StationToolState::default();
        assert_eq!(state.tier, StationTier::Station, "default is the workhorse");

        let mut seen = vec![state.tier];
        for _ in 0..TIER_CYCLE.len() {
            state.cycle_tier();
            seen.push(state.tier);
        }
        assert_eq!(seen.first(), seen.last(), "the cycle closes");
        for tier in StationTier::ALL {
            assert!(seen.contains(&tier), "{} is reachable", tier.label());
        }
    }
}
