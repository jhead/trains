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
//! Map View is owned by `map::map_view`, which listens for `M`. Rather than
//! reach into another module's state — and desynchronise its saved camera — the
//! button posts a synthetic key press through [`SyntheticKeys`], which is
//! injected in `PreUpdate` and released the following frame. It is a small
//! bridge, and it is honest: the button and the key take exactly the same path.

use bevy::prelude::*;

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

/// Window buttons, in row order, with the key that also opens them.
const WINDOW_SLOTS: &[(WindowId, &str, Option<KeyCode>)] = &[
    (WindowId::Network, "H", Some(KeyCode::KeyH)),
    (WindowId::TownTalk, "Y", Some(KeyCode::KeyY)),
    (WindowId::Ledger, "K", Some(KeyCode::KeyK)),
    (WindowId::Alerts, "C", Some(KeyCode::KeyC)),
    (WindowId::Goals, "O", Some(KeyCode::KeyO)),
    // `N` is owned by the Neighbours panel itself; the button mirrors it.
    (WindowId::Neighbours, "N", None),
];

/// Build verbs, in row order, with the key that also arms them. The label comes
/// from [`ToolbarTool::label`] so the bar and the status readout cannot drift.
const TOOL_SLOTS: &[(ToolbarTool, &str)] = &[
    (ToolbarTool::Select, "V"),
    (ToolbarTool::Build, "B"),
    (ToolbarTool::Demolish, "X"),
    (ToolbarTool::Line, "L"),
    (ToolbarTool::Transit, "T"),
    (ToolbarTool::Transport, "G"),
];

pub fn setup_top_chrome(mut commands: Commands, money: Res<rail_sim::Money>) {
    let starting_cents = money.cents();
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
            spawn_menu_row(chrome);
            spawn_status_row(chrome, starting_cents);
            spawn_health_row(chrome);
        });
}

fn spawn_menu_row(parent: &mut ChildSpawnerCommands) {
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
            for (tool, key) in TOOL_SLOTS {
                spawn_menu_button(row, MenuButton::Tool(*tool), tool.label(), key);
            }
            spawn_divider(row);
            for (id, key, _) in WINDOW_SLOTS {
                spawn_menu_button(row, MenuButton::Window(*id), id.title(), key);
            }
            spawn_menu_button(row, MenuButton::MapView, "Map", "M");
            spawn_menu_button(row, MenuButton::Overlay, "Overlay", "Tab");
            spawn_divider(row);
            spawn_menu_button(row, MenuButton::Settings, "Settings", "");
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
    key: &str,
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
            if !key.is_empty() {
                slot.spawn((Text::new(key.to_string()), micro_font(), text_secondary()));
            }
        });
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
            MenuButton::MapView => synthetic.press(KeyCode::KeyM),
            MenuButton::Overlay => overlay.0 = overlay.0.next(),
            MenuButton::Settings => {
                settings.write(MenuActivated(MenuAction::OpenSettings));
            }
        }
    }
}

/// Keyboard shortcuts for the window group.
///
/// Modifier-held presses are ignored so `Ctrl+Y` stays redo and nothing here
/// steals a chord from another owner.
pub fn window_hotkeys(keys: Res<ButtonInput<KeyCode>>, mut manager: ResMut<WindowManager>) {
    if keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight)
        || keys.pressed(KeyCode::SuperLeft)
        || keys.pressed(KeyCode::SuperRight)
    {
        return;
    }
    for (id, _, key) in WINDOW_SLOTS {
        let Some(key) = key else { continue };
        if keys.just_pressed(*key) {
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
        // B/X/L/T/G/V arm tools, WASD pans, M is Map View, N is Neighbours,
        // F follows, Z zooms, P places a station, U upgrades one.
        let taken = [
            KeyCode::KeyB,
            KeyCode::KeyX,
            KeyCode::KeyL,
            KeyCode::KeyT,
            KeyCode::KeyG,
            KeyCode::KeyV,
            KeyCode::KeyW,
            KeyCode::KeyA,
            KeyCode::KeyS,
            KeyCode::KeyD,
            KeyCode::KeyM,
            KeyCode::KeyN,
            KeyCode::KeyF,
            KeyCode::KeyZ,
            KeyCode::KeyP,
            KeyCode::KeyU,
        ];
        for (id, _, key) in WINDOW_SLOTS {
            let Some(key) = key else { continue };
            assert!(!taken.contains(key), "{id:?} steals {key:?}");
        }
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
