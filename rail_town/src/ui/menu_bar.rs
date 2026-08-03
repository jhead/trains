//! The top chrome: menu row, status strip, health strip.
//!
//! Binding standard: [`docs/design/03-ui-system.md`](../../../docs/design/03-ui-system.md) §5.
//!
//! # The split
//!
//! The menu row carries two groups, and the split is deliberate:
//!
//! * **Verbs, on the left.** Look / Track / Demolish / Line / Transit /
//!   Transport are *modes*, not windows. They arm the pointer, so they must be
//!   one click with no intermediate step, and they must show which one is armed.
//!   03 §7's rule survives the rework intact: a player who has read nothing can
//!   find every verb in the game on this bar.
//! * **Windows, on the right.** Network / Town Talk / Ledger / Alerts / Goals /
//!   Neighbours are *readings*. Each button opens a window the player can move,
//!   stack and close. Nothing in this group changes what a click on the world
//!   does, which is exactly why it is a separate group.
//!
//! Two more sit at the end: Map View and Overlay change how the world is drawn
//! rather than what a click does, and Settings hands off to the shell.
//!
//! The bottom toolbar is gone. Two permanent bars competing for the player's
//! eye is one too many, and the playtest asked for the top row to be the
//! organising idea.
//!
//! # Map View
//!
//! Map View is owned by `map::map_view`, which listens for the Map View
//! binding. Rather than reach into another module's state — and desynchronise
//! its saved camera — the button posts a synthetic key press through
//! [`SyntheticKeys`], which is injected in `PreUpdate` and released the
//! following frame. It is a small bridge, and it is honest: the button and the
//! key take exactly the same path.
//!
//! # Keys on the bar
//!
//! Every slot carries its shortcut beside its name (03 §7), and that shortcut
//! is read out of [`KeyBindings`] rather than typed in beside the label. So a
//! rebind is visible where the player looks for the verb, not only in Settings —
//! and a bar that says `L` while the game listens for something else is not a
//! state this file can reach.

use bevy::prelude::*;

use crate::input::{ControlAction, KeyBindings};
use crate::lines::LineToolState;
use crate::map::MapViewState;
use crate::overlays::{ActiveOverlay, OverlayKind};
use crate::palette::{BG1, OUTLINE};
use crate::shell::{MenuAction, MenuActivated};
use crate::track::TrackToolState;
use crate::trains::TrainToolState;
use crate::ui::health::spawn_health_row;
use crate::ui::kit::{
    chrome_button_node, control_border, micro_font, text_primary, text_secondary, MENU_H, SPACE_1,
    SPACE_2,
};
use crate::ui::status_strip::spawn_status_row;
use crate::ui::toolbar::{active_tool, ToolbarTool};
use crate::ui::window::{WindowId, WindowManager};

/// Root of the whole top block. One node, laid out in flow, so the rows can
/// change height without anything else having to know.
#[derive(Component)]
pub struct TopChromeRoot;

#[derive(Component)]
pub struct MenuRowRoot;

/// What a menu-row button does.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuButton {
    /// Arms a build verb.
    Tool(ToolbarTool),
    /// Toggles a window.
    Window(WindowId),
    MapView,
    Overlay,
    Settings,
}

/// Keys the UI posts on the player's behalf, so a button and its hotkey take
/// the same code path. See the module docs.
#[derive(Resource, Debug, Default)]
pub struct SyntheticKeys {
    queued: Vec<KeyCode>,
    held: Vec<KeyCode>,
}

impl SyntheticKeys {
    pub fn press(&mut self, key: KeyCode) {
        self.queued.push(key);
    }
}

/// Window buttons, in row order, with the action whose key also opens them.
///
/// The flag says whether [`window_hotkeys`] owns that key. Neighbours is
/// `false`: the panel's own module listens for it, and toggling in both places
/// would cancel itself out.
const WINDOW_SLOTS: &[(WindowId, ControlAction, bool)] = &[
    (WindowId::Network, ControlAction::WindowNetwork, true),
    (WindowId::TownTalk, ControlAction::WindowTownTalk, true),
    (WindowId::Ledger, ControlAction::Ledger, true),
    (WindowId::Alerts, ControlAction::WindowAlerts, true),
    (WindowId::Goals, ControlAction::WindowGoals, true),
    (WindowId::Neighbours, ControlAction::WindowNeighbours, false),
];

/// Build verbs, in row order, with the action that also arms them. The label
/// comes from [`ToolbarTool::label`] so the bar and the status readout cannot
/// drift, and the key comes from [`KeyBindings`] for the same reason.
const TOOL_SLOTS: &[(ToolbarTool, ControlAction)] = &[
    (ToolbarTool::Select, ControlAction::LookTool),
    (ToolbarTool::Build, ControlAction::TrackTool),
    (ToolbarTool::Demolish, ControlAction::DemolishTool),
    (ToolbarTool::Line, ControlAction::LineTool),
    (ToolbarTool::Transit, ControlAction::BuyTransit),
    (ToolbarTool::Transport, ControlAction::BuyTransport),
];

/// The shortcut text beside a slot's name. Repainted by
/// [`refresh_menu_key_labels`] whenever the bindings change.
#[derive(Component, Debug, Clone, Copy)]
pub struct MenuKeyLabel(ControlAction);

pub fn setup_top_chrome(
    mut commands: Commands,
    money: Res<rail_sim::Money>,
    bindings: Res<KeyBindings>,
) {
    let starting_cents = money.cents();
    let bindings = bindings.clone();
    commands
        .spawn((
            TopChromeRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                border: UiRect {
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    top: Val::Px(0.0),
                    bottom: Val::Px(1.0),
                },
                border_radius: BorderRadius::ZERO,
                ..default()
            },
            BackgroundColor(BG1),
            BorderColor::all(OUTLINE),
            ZIndex(10),
        ))
        .with_children(|chrome| {
            spawn_menu_row(chrome, &bindings);
            spawn_status_row(chrome, starting_cents);
            spawn_health_row(chrome);
        });
}

fn spawn_menu_row(parent: &mut ChildSpawnerCommands, bindings: &KeyBindings) {
    parent
        .spawn((
            MenuRowRoot,
            Node {
                width: Val::Percent(100.0),
                min_height: Val::Px(MENU_H),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                // Wrapping is what keeps every verb reachable on a narrow
                // window instead of pushing the last of them off the edge.
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(SPACE_1),
                row_gap: Val::Px(1.0),
                padding: UiRect::axes(Val::Px(SPACE_2), Val::Px(2.0)),
                ..default()
            },
        ))
        .with_children(|row| {
            for (tool, action) in TOOL_SLOTS {
                let button = MenuButton::Tool(*tool);
                spawn_menu_button(row, button, tool.label(), Some(*action), bindings);
            }
            spawn_divider(row);
            for (id, action, _) in WINDOW_SLOTS {
                let button = MenuButton::Window(*id);
                spawn_menu_button(row, button, id.title(), Some(*action), bindings);
            }
            spawn_menu_button(
                row,
                MenuButton::MapView,
                "Map",
                Some(ControlAction::MapView),
                bindings,
            );
            spawn_menu_button(
                row,
                MenuButton::Overlay,
                "Overlay",
                Some(ControlAction::CycleOverlay),
                bindings,
            );
            spawn_divider(row);
            // Settings has no shortcut: it hands off to the shell, and `Esc`
            // already owns the way out.
            spawn_menu_button(row, MenuButton::Settings, "Settings", None, bindings);
        });
}

fn spawn_divider(parent: &mut ChildSpawnerCommands) {
    parent.spawn((
        Node {
            width: Val::Px(1.0),
            height: Val::Px(MENU_H - 8.0),
            margin: UiRect::horizontal(Val::Px(SPACE_1)),
            ..default()
        },
        BackgroundColor(crate::palette::BALLAST_D),
    ));
}

fn spawn_menu_button(
    parent: &mut ChildSpawnerCommands,
    button: MenuButton,
    label: &str,
    action: Option<ControlAction>,
    bindings: &KeyBindings,
) {
    let (node, bg, border) = chrome_button_node(SPACE_1, 1.0);
    parent
        .spawn((
            Button,
            button,
            Node {
                column_gap: Val::Px(SPACE_1),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                flex_shrink: 0.0,
                ..node
            },
            bg,
            border,
        ))
        .with_children(|slot| {
            slot.spawn((Text::new(label.to_string()), micro_font(), text_primary()));
            if let Some(action) = action {
                slot.spawn((
                    MenuKeyLabel(action),
                    Text::new(bindings.label(action)),
                    micro_font(),
                    text_secondary(),
                ));
            }
        });
}

/// Repaint the shortcut beside each slot when the player rebinds one.
///
/// Gated on the resource's own change detection, so a normal frame does not
/// touch a single node.
pub fn refresh_menu_key_labels(
    bindings: Res<KeyBindings>,
    mut labels: Query<(&MenuKeyLabel, &mut Text)>,
) {
    if !bindings.is_changed() {
        return;
    }
    for (slot, mut text) in &mut labels {
        let wanted = bindings.label(slot.0);
        if text.0 != wanted {
            text.0 = wanted;
        }
    }
}

/// Paint selection and hover on the menu row.
///
/// Runs every frame but touches nothing unless a button's wanted border differs
/// from the one it already has.
#[allow(clippy::too_many_arguments)]
pub fn update_menu_row(
    track: Res<TrackToolState>,
    train: Option<Res<TrainToolState>>,
    line: Option<Res<LineToolState>>,
    manager: Res<WindowManager>,
    map_view: Res<MapViewState>,
    overlay: Res<ActiveOverlay>,
    mut buttons: Query<(&MenuButton, &Interaction, &mut BorderColor)>,
) {
    let placing = train.as_ref().is_some_and(|t| t.place_mode);
    let place_kind = train.as_ref().map(|t| t.kind);
    let line_active = line.as_ref().is_some_and(|l| l.active);
    let armed = active_tool(track.tool, placing, place_kind, line_active);

    for (button, interaction, mut border) in &mut buttons {
        let selected = match button {
            MenuButton::Tool(tool) => *tool == armed,
            MenuButton::Window(id) => manager.is_open(*id),
            MenuButton::MapView => map_view.active,
            MenuButton::Overlay => overlay.0 != OverlayKind::None,
            MenuButton::Settings => false,
        };
        let hovered = matches!(interaction, Interaction::Hovered | Interaction::Pressed);
        let wanted = control_border(selected, hovered);
        if border.top != wanted.top {
            *border = wanted;
        }
    }
}

/// Answer a menu-row click.
#[allow(clippy::too_many_arguments)]
pub fn menu_row_clicks(
    interactions: Query<(&Interaction, &MenuButton), (Changed<Interaction>, With<Button>)>,
    mut manager: ResMut<WindowManager>,
    mut overlay: ResMut<ActiveOverlay>,
    mut synthetic: ResMut<SyntheticKeys>,
    bindings: Res<KeyBindings>,
    mut settings: MessageWriter<MenuActivated>,
    mut tools: crate::ui::toolbar::ToolStates,
) {
    for (interaction, button) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match button {
            MenuButton::Tool(tool) => tools.arm(*tool),
            MenuButton::Window(id) => manager.toggle(*id),
            MenuButton::MapView => synthetic.press(bindings.key(ControlAction::MapView)),
            MenuButton::Overlay => overlay.0 = overlay.0.next(),
            MenuButton::Settings => {
                settings.write(MenuActivated(MenuAction::OpenSettings));
            }
        }
    }
}

/// Keyboard shortcuts for the window group.
///
/// [`KeyBindings::just_pressed`] requires an exact modifier match, so `Ctrl+Y`
/// stays redo and nothing here steals a chord from another owner.
pub fn window_hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    mut manager: ResMut<WindowManager>,
) {
    for (id, action, owns_key) in WINDOW_SLOTS {
        if !owns_key {
            continue;
        }
        if bindings.just_pressed(&keys, *action) {
            manager.toggle(*id);
        }
    }
}

/// Post queued synthetic presses, and release the previous frame's.
///
/// `PreUpdate`, after the real input has been gathered, so the systems that
/// read `just_pressed` in `Update` cannot tell the difference.
pub fn inject_synthetic_keys(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut synthetic: ResMut<SyntheticKeys>,
) {
    if synthetic.held.is_empty() && synthetic.queued.is_empty() {
        return;
    }
    for key in std::mem::take(&mut synthetic.held) {
        keys.release(key);
    }
    let queued = std::mem::take(&mut synthetic.queued);
    for key in queued {
        keys.press(key);
        synthetic.held.push(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Binding;
    use std::collections::HashSet;

    #[test]
    fn every_verb_in_the_game_is_on_the_bar() {
        // 03 §7's acceptance bar, restated for the menu row.
        let listed: HashSet<ToolbarTool> = TOOL_SLOTS.iter().map(|(t, _)| *t).collect();
        for tool in [
            ToolbarTool::Select,
            ToolbarTool::Build,
            ToolbarTool::Demolish,
            ToolbarTool::Line,
            ToolbarTool::Transit,
            ToolbarTool::Transport,
        ] {
            assert!(listed.contains(&tool), "{tool:?} is unreachable by mouse");
        }
    }

    #[test]
    fn every_window_except_the_inspector_has_a_button() {
        // The Inspector opens by selecting something in the world, so it is the
        // one window without a button.
        let listed: HashSet<WindowId> = WINDOW_SLOTS.iter().map(|(id, _, _)| *id).collect();
        for id in WindowId::ALL {
            if *id == WindowId::Inspector {
                assert!(!listed.contains(id));
                continue;
            }
            assert!(listed.contains(id), "{id:?} has no way in");
        }
    }

    #[test]
    fn no_window_hotkey_collides_with_a_gameplay_verb() {
        // 03 §10.2: "Window keys avoid every key a gameplay verb already owns."
        // Both halves now come out of one table, so the check is a lookup rather
        // than a hand-kept list that could go stale — which is exactly how the
        // Ledger and the Line tool ended up sharing `L`.
        let bindings = KeyBindings::default();
        let window_actions: HashSet<ControlAction> =
            WINDOW_SLOTS.iter().map(|(_, a, _)| *a).collect();
        for (id, action, _) in WINDOW_SLOTS {
            let key = bindings.binding(*action);
            for other in ControlAction::ALL {
                if window_actions.contains(other) {
                    continue;
                }
                assert_ne!(
                    bindings.binding(*other),
                    key,
                    "{id:?} steals {} from {:?}",
                    key.label(),
                    other
                );
            }
        }
    }

    #[test]
    fn the_ledger_button_answers_to_k() {
        // The `L` clash, at the surface the player reads it from.
        let bindings = KeyBindings::default();
        let (_, action, owns) = WINDOW_SLOTS
            .iter()
            .find(|(id, _, _)| *id == WindowId::Ledger)
            .expect("the Ledger has a button");
        assert_eq!(bindings.key(*action), KeyCode::KeyK);
        assert!(owns, "the menu row owns the Ledger key");
        assert_eq!(bindings.label(*action), "K", "and the bar draws it");
    }

    #[test]
    fn the_bar_draws_the_key_the_game_listens_for() {
        // Rebinding the track tool must move the label on the bar as well as
        // the behaviour: a row reading `B` while the game answers `J` is worse
        // than no label at all.
        let mut bindings = KeyBindings::default();
        assert_eq!(bindings.label(ControlAction::TrackTool), "B");
        bindings.set(ControlAction::TrackTool, Binding::key(KeyCode::KeyJ));
        assert_eq!(bindings.label(ControlAction::TrackTool), "J");
    }

    #[test]
    fn only_one_owner_toggles_the_neighbours_window() {
        // Both the menu row and `border::panel` can see the key. If both acted
        // the window would open and close in the same frame and never appear.
        let owned: Vec<WindowId> = WINDOW_SLOTS
            .iter()
            .filter(|(_, _, owns)| *owns)
            .map(|(id, _, _)| *id)
            .collect();
        assert!(!owned.contains(&WindowId::Neighbours));
        assert_eq!(owned.len(), WINDOW_SLOTS.len() - 1);
    }

    #[test]
    fn a_synthetic_press_is_released_the_next_frame() {
        let mut keys = ButtonInput::<KeyCode>::default();
        let mut synth = SyntheticKeys::default();
        synth.press(KeyCode::KeyM);

        // Frame 1: the press lands.
        for key in std::mem::take(&mut synth.held) {
            keys.release(key);
        }
        for key in std::mem::take(&mut synth.queued) {
            keys.press(key);
            synth.held.push(key);
        }
        assert!(keys.just_pressed(KeyCode::KeyM));

        // Frame 2: the engine clears `just_pressed`, and we let go.
        keys.clear();
        for key in std::mem::take(&mut synth.held) {
            keys.release(key);
        }
        assert!(!keys.pressed(KeyCode::KeyM));
        assert!(synth.held.is_empty());
    }
}
