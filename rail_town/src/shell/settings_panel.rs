//! The Settings panel — four tabs, reachable from the title and from pause.
//!
//! Settings is an overlay rather than a fifth state: the state machine is
//! Title → NewMap → Playing → Paused, and Settings has to be openable from two
//! of those without inventing a transition graph. It remembers which screen
//! opened it and puts the player back there.
//!
//! The panel is rebuilt whenever a value changes. Rebuilding is what keeps the
//! rows honest — every row renders straight from [`Settings`], so a row can never
//! display something the resource does not actually say.

use bevy::prelude::*;

use crate::palette::WARN;
use crate::ui::kit::{micro_font, SPACE_2, SPACE_3};

use super::controls::{is_rebindable, Binding, ControlAction, ControlGroup};
use super::settings::{SettingId, Settings, SettingsTab};
use super::widgets::{
    dim_scrim, screen_root, shell_panel, spawn_button, spawn_note, spawn_panel_title,
    spawn_row_with_note, spawn_rule, spawn_section_label, spawn_tab_strip, MenuAction, MenuCursor,
    MenuItem, MenuList, LAYER_OVERLAY,
};
use super::ShellState;

/// Every row on this panel sits on the overlay layer, so the screen underneath
/// stops taking input the moment Settings opens.
fn row(nav: usize, action: MenuAction) -> MenuItem {
    MenuItem::new(nav, action).on_layer(LAYER_OVERLAY)
}

fn button(action: MenuAction) -> MenuItem {
    MenuItem::mouse_only(action).on_layer(LAYER_OVERLAY)
}

const PANEL_W: f32 = 460.0;
const PANEL_MAX_H: f32 = 85.0; // percent — panels scroll rather than grow (03 §5)

/// Open state for the overlay.
#[derive(Resource, Debug, Clone, Default)]
pub struct SettingsPanel {
    pub open: bool,
    pub tab: SettingsTab,
    /// Screen to return to when the panel closes.
    pub return_to: Option<ShellState>,
    /// Action currently waiting for a key press.
    pub rebinding: Option<ControlAction>,
}

impl SettingsPanel {
    pub fn open_from(&mut self, state: ShellState) {
        self.open = true;
        self.return_to = Some(state);
        self.rebinding = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.rebinding = None;
    }
}

#[derive(Component)]
pub struct SettingsRoot;

/// Spawn / despawn / rebuild, driven purely by resource change detection.
pub fn rebuild_settings_panel(
    mut commands: Commands,
    panel: Res<SettingsPanel>,
    settings: Res<Settings>,
    cursor: Res<MenuCursor>,
    existing: Query<Entity, With<SettingsRoot>>,
) {
    let dirty = panel.is_changed() || settings.is_changed();
    if !dirty && !(panel.open && existing.is_empty()) {
        return;
    }
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    if !panel.open {
        return;
    }

    commands
        .spawn((
            SettingsRoot,
            screen_root(
                "shell::settings",
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
            ),
        ))
        .with_children(|root| {
            root.spawn(dim_scrim());
            root.spawn((
                MenuList {
                    selected: cursor.0,
                    layer: LAYER_OVERLAY,
                },
                shell_panel(Node {
                    width: Val::Px(PANEL_W),
                    max_height: Val::Percent(PANEL_MAX_H),
                    overflow: Overflow::scroll_y(),
                    ..default()
                }),
            ))
            .with_children(|body| {
                spawn_panel_title(body, "Settings");
                spawn_tab_strip(body, panel.tab, LAYER_OVERLAY);

                match panel.tab {
                    SettingsTab::Controls => spawn_controls_tab(body, &settings, &panel),
                    tab => spawn_value_rows(body, &settings, tab),
                }

                spawn_rule(body);
                body.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::FlexEnd,
                    column_gap: Val::Px(SPACE_2),
                    ..default()
                })
                .with_children(|strip| {
                    if panel.tab == SettingsTab::Controls {
                        spawn_button(strip, button(MenuAction::ResetControls), "Reset");
                    }
                    spawn_button(strip, button(MenuAction::CloseSettings), "Back");
                });
                spawn_note(body, "<- -> change   Tab next tab   Esc closes");
            });
        });
}

/// Display / Audio / Gameplay rows: label, value, and an honest note when the
/// setting is stored but nothing consumes it yet.
fn spawn_value_rows(parent: &mut ChildSpawnerCommands, settings: &Settings, tab: SettingsTab) {
    let ids: Vec<SettingId> = SettingId::ALL
        .iter()
        .copied()
        .filter(|id| id.tab() == tab)
        .collect();
    for (index, id) in ids.iter().enumerate() {
        let item = row(index, MenuAction::CycleSetting(*id, 1)).with_steps(
            MenuAction::CycleSetting(*id, -1),
            MenuAction::CycleSetting(*id, 1),
        );
        spawn_row_with_note(
            parent,
            item,
            id.label(),
            &id.value_label(settings),
            id.pending_note(),
        );
    }
}

/// The rebindable list, grouped by context, with conflicts flagged.
fn spawn_controls_tab(
    parent: &mut ChildSpawnerCommands,
    settings: &Settings,
    panel: &SettingsPanel,
) {
    // The hold-to-repeat row is a normal value row and leads the tab.
    spawn_value_rows(parent, settings, SettingsTab::Controls);

    let mut index = SettingId::ALL
        .iter()
        .filter(|id| id.tab() == SettingsTab::Controls)
        .count();

    for group in ControlGroup::ALL {
        let actions: Vec<ControlAction> = ControlAction::ALL
            .iter()
            .copied()
            .filter(|a| a.group() == *group)
            .collect();
        if actions.is_empty() {
            continue;
        }
        spawn_section_label(parent, group.label());
        for action in actions {
            let capturing = panel.rebinding == Some(action);
            let value = if capturing {
                "press a key...".to_string()
            } else {
                settings.controls.key_for(action).label()
            };
            let note = settings.controls.has_conflict(action).then_some("conflict");
            spawn_row_with_note(
                parent,
                row(index, MenuAction::RebindControl(action)),
                action.label(),
                &value,
                note,
            );
            index += 1;
        }
    }

    spawn_rule(parent);
    parent.spawn((
        Text::new(
            "Rebinding is recorded but gameplay still reads its default key — \
             wiring lands with the input map.",
        ),
        micro_font(),
        TextColor(WARN),
        Node {
            max_width: Val::Px(PANEL_W - SPACE_3 * 2.0),
            ..default()
        },
    ));
    spawn_note(
        parent,
        "Pan also accepts the arrow keys. Middle-drag pans, wheel zooms.",
    );
}

/// Capture the next key press into the pending rebind.
pub fn capture_rebind(
    keys: Res<ButtonInput<KeyCode>>,
    mut panel: ResMut<SettingsPanel>,
    mut settings: ResMut<Settings>,
) {
    let Some(action) = panel.rebinding else {
        return;
    };
    // Escape cancels rather than binding: it is the universal unwind.
    if keys.just_pressed(KeyCode::Escape) {
        panel.rebinding = None;
        return;
    }
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let Some(key) = keys
        .get_just_pressed()
        .copied()
        .find(|k| is_rebindable(*k) && *k != KeyCode::Escape)
    else {
        return;
    };
    settings.controls.set(action, Binding { key, ctrl });
    panel.rebinding = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_remembers_where_to_return_to() {
        let mut panel = SettingsPanel::default();
        panel.open_from(ShellState::Paused);
        assert!(panel.open);
        assert_eq!(panel.return_to, Some(ShellState::Paused));

        panel.close();
        assert!(!panel.open);
        assert_eq!(
            panel.return_to,
            Some(ShellState::Paused),
            "the return target survives closing so the caller can still use it"
        );
    }

    #[test]
    fn closing_abandons_a_pending_rebind() {
        let mut panel = SettingsPanel::default();
        panel.open_from(ShellState::Title);
        panel.rebinding = Some(ControlAction::MapView);
        panel.close();
        assert!(panel.rebinding.is_none());
    }

    #[test]
    fn controls_tab_covers_every_action() {
        // Guards against a group being added without a section in the tab.
        for action in ControlAction::ALL {
            assert!(ControlGroup::ALL.contains(&action.group()));
        }
    }
}
