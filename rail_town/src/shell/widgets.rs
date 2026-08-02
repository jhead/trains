//! Shell menu widgets, built on the in-game kit rather than beside it.
//!
//! Everything here composes [`crate::ui::kit`] — same spacing scale, same type
//! roles, same palette, square corners. A menu row is the design's list row
//! ([`03 §8.6`](../../../docs/design/03-ui-system.md)): 24 tall, a 2-texel `hi`
//! left border when selected, hover raised to `ballastD`. There is no second UI
//! style in this module and there must not be one.
//!
//! Every screen drives the same three shared systems: [`menu_keyboard_nav`],
//! [`menu_pointer`] and [`paint_menu_items`]. A screen only has to spawn
//! [`MenuItem`]s under a [`MenuList`] and read [`MenuActivated`].

use bevy::prelude::*;

use crate::palette::{BALLAST_D, BALLAST_M, BG0, BG1, HI, OUTLINE};
use crate::ui::kit::{
    body_font, display_font, micro_font, panel_node, text_accent, text_disabled, text_primary,
    text_secondary, WorldClickBlocker, SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_6,
};

use super::controls::ControlAction;
use super::map_options::OptionField;
use super::settings::{SettingId, SettingsTab};

/// Marks a shell-owned UI root, so world-HUD gating can tell ours from theirs.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct ShellUi;

/// Row height for menu items (design 03 §8.6).
pub const ROW_H: f32 = SPACE_6;
/// Selection marker width (design: 2-texel `hi` left border).
pub const SELECT_BORDER: f32 = 2.0;
/// Shell UI sits above every in-game panel (game chrome tops out at 20).
pub const SHELL_Z: i32 = 100;

/// Everything a shell menu row can ask for. One enum keeps dispatch in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    // Title
    Continue,
    NewMap,
    Load,
    OpenSettings,
    Quit,
    // Pause
    Resume,
    Save,
    QuitToTitle,
    // New Map
    Begin,
    Back,
    RerollSeed,
    CycleMapOption(OptionField, i32),
    // Settings
    SelectTab(SettingsTab),
    CycleSetting(SettingId, i32),
    RebindControl(ControlAction),
    ResetControls,
    CloseSettings,
    /// Row exists for layout / reference only and does nothing when activated.
    Inert,
}

/// A row was activated, by key or by click.
#[derive(Message, Debug, Clone, Copy)]
pub struct MenuActivated(pub MenuAction);

/// Base menu layer — the screen currently in [`super::ShellState`].
pub const LAYER_SCREEN: u8 = 0;
/// Overlay layer — the Settings panel, which sits over whichever screen opened
/// it. Only the highest layer on screen takes input.
pub const LAYER_OVERLAY: u8 = 1;

/// Root of one navigable list. One per screen, plus one per open overlay.
#[derive(Component, Debug, Default)]
pub struct MenuList {
    pub selected: usize,
    /// Highest layer present wins input; everything below it goes quiet.
    pub layer: u8,
}

/// The selected row, mirrored outside the entity.
///
/// Screens that rebuild themselves (New Map on every option change, Settings on
/// every value change) would otherwise throw the selection away on each keypress.
/// The list seeds itself from this on spawn and writes back to it every frame.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct MenuCursor(pub usize);

/// Keep [`MenuCursor`] in step with the list that currently has input.
pub fn sync_menu_cursor(lists: Query<&MenuList>, mut cursor: ResMut<MenuCursor>) {
    let Some(selected) = active_list(lists.iter()).map(|list| list.selected) else {
        return;
    };
    if cursor.0 != selected {
        cursor.0 = selected;
    }
}

/// The topmost list, or `None` when no menu is on screen.
fn active_list<'a>(lists: impl Iterator<Item = &'a MenuList>) -> Option<&'a MenuList> {
    lists.max_by_key(|list| list.layer)
}

/// One row. `nav` is `None` for mouse-only rows such as tab buttons.
#[derive(Component, Debug, Clone, Copy)]
pub struct MenuItem {
    pub nav: Option<usize>,
    pub action: MenuAction,
    /// Left arrow on this row (option rows step their value).
    pub left: Option<MenuAction>,
    /// Right arrow on this row.
    pub right: Option<MenuAction>,
    pub enabled: bool,
    /// Must match the row's owning [`MenuList`]. See [`LAYER_OVERLAY`].
    pub layer: u8,
}

impl MenuItem {
    pub fn new(nav: usize, action: MenuAction) -> Self {
        Self {
            nav: Some(nav),
            action,
            left: None,
            right: None,
            enabled: true,
            layer: LAYER_SCREEN,
        }
    }

    /// A row the mouse can click but the keyboard skips.
    pub fn mouse_only(action: MenuAction) -> Self {
        Self {
            nav: None,
            ..Self::new(0, action)
        }
    }

    pub fn with_steps(mut self, left: MenuAction, right: MenuAction) -> Self {
        self.left = Some(left);
        self.right = Some(right);
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn on_layer(mut self, layer: u8) -> Self {
        self.layer = layer;
        self
    }
}

/// Label inside a row that should take the selection colour.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct MenuItemLabel;

/// Full-screen shell root: transparent, click-blocking, above the game HUD.
///
/// Transparent matters — design §2 wants the live world visible behind the menu.
/// [`WorldClickBlocker`] keeps build tools from firing through it: while a shell
/// screen is up the pointer is always over blocking chrome, so `UiBlocksWorld`
/// stays set and no click reaches the world.
///
/// `layout` supplies flex direction / alignment; position and size are fixed.
pub fn screen_root(name: impl Into<std::borrow::Cow<'static, str>>, layout: Node) -> impl Bundle {
    (
        Name::new(name),
        ShellUi,
        WorldClickBlocker,
        Interaction::default(),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(0.0),
            left: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..layout
        },
        BackgroundColor(Color::NONE),
        ZIndex(SHELL_Z),
    )
}

/// Opaque scrim that dims what is behind it to 50% (design 03 §5, 09 §4).
pub fn dim_scrim() -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(0.0),
            left: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(BG0.with_alpha(0.5)),
    )
}

/// Standard shell panel: kit chrome, 12 inset, column layout.
pub fn shell_panel(extra: Node) -> impl Bundle {
    let (node, bg, border) = panel_node(Node {
        flex_direction: FlexDirection::Column,
        padding: UiRect::all(Val::Px(SPACE_3)),
        row_gap: Val::Px(SPACE_2),
        ..extra
    });
    (node, bg, border)
}

/// Panel title in Display type with the 1-texel `ballastD` rule under it.
pub fn spawn_panel_title(parent: &mut ChildSpawnerCommands, title: &str) {
    parent.spawn((Text::new(title.to_string()), display_font(), text_primary()));
    spawn_rule(parent);
}

/// 1-texel `ballastD` divider.
pub fn spawn_rule(parent: &mut ChildSpawnerCommands) {
    parent.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(1.0),
            ..default()
        },
        BackgroundColor(BALLAST_D),
    ));
}

/// Small-caps-ish section label in Micro type.
pub fn spawn_section_label(parent: &mut ChildSpawnerCommands, label: &str) {
    parent.spawn((
        Text::new(label.to_uppercase()),
        micro_font(),
        text_secondary(),
    ));
}

/// A quiet one-line note (footers, hints, "not wired yet").
pub fn spawn_note(parent: &mut ChildSpawnerCommands, text: &str) {
    parent.spawn((Text::new(text.to_string()), micro_font(), text_secondary()));
}

/// Menu row: label on the left, optional value on the right.
///
/// Returns nothing — screens address rows through [`MenuItem`] queries.
pub fn spawn_row(parent: &mut ChildSpawnerCommands, item: MenuItem, label: &str, value: &str) {
    spawn_row_with_note(parent, item, label, value, None);
}

/// Menu row with a trailing Micro note (used for "not wired yet" and conflicts).
pub fn spawn_row_with_note(
    parent: &mut ChildSpawnerCommands,
    item: MenuItem,
    label: &str,
    value: &str,
    note: Option<&str>,
) {
    parent
        .spawn((
            Button,
            item,
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(ROW_H),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(SPACE_2),
                padding: UiRect::horizontal(Val::Px(SPACE_2)),
                // The selection marker lives in the left border, so reserve it
                // on every row and the labels never shift when selection moves.
                border: UiRect::left(Val::Px(SELECT_BORDER)),
                border_radius: BorderRadius::ZERO,
                ..default()
            },
            BackgroundColor(BG1),
            BorderColor::all(Color::NONE),
        ))
        .with_children(|row| {
            row.spawn((
                MenuItemLabel,
                Text::new(label.to_string()),
                body_font(),
                text_primary(),
            ));
            if !value.is_empty() {
                row.spawn((
                    Text::new(value.to_string()),
                    body_font(),
                    text_secondary(),
                    Node {
                        margin: UiRect::left(Val::Auto),
                        ..default()
                    },
                ));
            }
            if let Some(note) = note {
                row.spawn((
                    Text::new(note.to_string()),
                    micro_font(),
                    text_disabled(),
                    Node {
                        margin: UiRect::left(if value.is_empty() {
                            Val::Auto
                        } else {
                            Val::Px(SPACE_2)
                        }),
                        ..default()
                    },
                ));
            }
        });
}

/// Design 03 §8.2 button: 24 tall, 12 horizontal inset, raised edge.
pub fn spawn_button(parent: &mut ChildSpawnerCommands, item: MenuItem, label: &str) {
    parent
        .spawn((
            Button,
            item,
            Node {
                height: Val::Px(ROW_H),
                padding: UiRect::horizontal(Val::Px(SPACE_3)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::ZERO,
                ..default()
            },
            BackgroundColor(BG1),
            BorderColor::all(BALLAST_M),
        ))
        .with_children(|b| {
            b.spawn((
                MenuItemLabel,
                Text::new(label.to_string()),
                body_font(),
                text_primary(),
            ));
        });
}

/// Horizontal strip of tab buttons.
pub fn spawn_tab_strip(parent: &mut ChildSpawnerCommands, active: SettingsTab, layer: u8) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(SPACE_1),
            ..default()
        })
        .with_children(|strip| {
            for tab in SettingsTab::ALL {
                strip
                    .spawn((
                        Button,
                        MenuItem::mouse_only(MenuAction::SelectTab(*tab)).on_layer(layer),
                        Node {
                            height: Val::Px(ROW_H),
                            padding: UiRect::horizontal(Val::Px(SPACE_3)),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            border: UiRect::bottom(Val::Px(SELECT_BORDER)),
                            border_radius: BorderRadius::ZERO,
                            ..default()
                        },
                        BackgroundColor(if *tab == active { BG1 } else { BG0 }),
                        BorderColor::all(if *tab == active { HI } else { OUTLINE }),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Text::new(tab.label()),
                            body_font(),
                            if *tab == active {
                                text_accent()
                            } else {
                                text_secondary()
                            },
                        ));
                    });
            }
        });
}

/// Bottom-right stamp: build version and the seed on screen (design §8).
pub fn spawn_corner_stamp(parent: &mut ChildSpawnerCommands, text: &str) {
    parent.spawn((
        Text::new(text.to_string()),
        micro_font(),
        text_secondary(),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(SPACE_4),
            bottom: Val::Px(SPACE_3),
            ..default()
        },
    ));
}

/// Up / Down move the selection, Enter activates, Left / Right step a value.
///
/// Runs in `PreUpdate` ahead of world-input suppression, so the shell reads keys
/// the game never sees while a menu is open.
pub fn menu_keyboard_nav(
    keys: Res<ButtonInput<KeyCode>>,
    mut lists: Query<&mut MenuList>,
    items: Query<&MenuItem>,
    mut activated: MessageWriter<MenuActivated>,
) {
    let Some(layer) = lists.iter().map(|list| list.layer).max() else {
        return;
    };
    let Some(mut list) = lists.iter_mut().find(|list| list.layer == layer) else {
        return;
    };
    let mut navigable: Vec<(usize, MenuItem)> = items
        .iter()
        .filter(|item| item.layer == layer)
        .filter_map(|item| item.nav.map(|nav| (nav, *item)))
        .collect();
    if navigable.is_empty() {
        return;
    }
    navigable.sort_by_key(|(nav, _)| *nav);

    let step = if keys.just_pressed(KeyCode::ArrowDown) {
        1
    } else if keys.just_pressed(KeyCode::ArrowUp) {
        -1
    } else {
        0
    };
    if step != 0 {
        list.selected = advance_selection(&navigable, list.selected, step);
    }

    let Some((_, current)) = navigable.iter().find(|(nav, _)| *nav == list.selected) else {
        // Selection landed on a row that no longer exists; snap to the first.
        list.selected = navigable[0].0;
        return;
    };
    if !current.enabled {
        return;
    }

    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter) {
        activated.write(MenuActivated(current.action));
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        if let Some(action) = current.left {
            activated.write(MenuActivated(action));
        }
    }
    if keys.just_pressed(KeyCode::ArrowRight) {
        if let Some(action) = current.right {
            activated.write(MenuActivated(action));
        }
    }
}

/// Next selectable row in `step` direction, wrapping, skipping disabled rows.
fn advance_selection(navigable: &[(usize, MenuItem)], selected: usize, step: i32) -> usize {
    let start = navigable
        .iter()
        .position(|(nav, _)| *nav == selected)
        .unwrap_or(0) as i32;
    let len = navigable.len() as i32;
    for offset in 1..=len {
        let index = (start + step * offset).rem_euclid(len) as usize;
        if navigable[index].1.enabled {
            return navigable[index].0;
        }
    }
    selected
}

/// Hover moves the selection; a press activates. Mouse is primary (03 §10.1).
pub fn menu_pointer(
    mut lists: Query<&mut MenuList>,
    items: Query<(&Interaction, &MenuItem), Changed<Interaction>>,
    mut activated: MessageWriter<MenuActivated>,
) {
    let Some(layer) = lists.iter().map(|list| list.layer).max() else {
        return;
    };
    for (interaction, item) in &items {
        // A screen underneath an open overlay is scenery, not a control.
        if !item.enabled || item.layer != layer {
            continue;
        }
        match interaction {
            Interaction::Hovered => {
                if let (Some(mut list), Some(nav)) =
                    (lists.iter_mut().find(|list| list.layer == layer), item.nav)
                {
                    list.selected = nav;
                }
            }
            Interaction::Pressed => {
                activated.write(MenuActivated(item.action));
            }
            Interaction::None => {}
        }
    }
}

/// Selection and hover painting. Colour never carries state alone: the selected
/// row also gains a `hi` left border, and disabled rows lose their label colour.
pub fn paint_menu_items(
    lists: Query<&MenuList>,
    mut items: Query<(
        &MenuItem,
        &Interaction,
        &mut BackgroundColor,
        &mut BorderColor,
        &Children,
    )>,
    mut labels: Query<&mut TextColor, With<MenuItemLabel>>,
) {
    let layer = lists.iter().map(|list| list.layer).max().unwrap_or(0);
    let selected = active_list(lists.iter())
        .map(|list| list.selected)
        .unwrap_or(usize::MAX);

    for (item, interaction, mut bg, mut border, children) in &mut items {
        // Tab buttons paint themselves on rebuild; leave their borders alone.
        if item.nav.is_none() {
            continue;
        }
        let active = item.layer == layer;
        let is_selected = active && item.nav == Some(selected);
        let hovered = active && matches!(interaction, Interaction::Hovered | Interaction::Pressed);

        // Hover raises the fill one step (03 §8.6); everything else sits on bg1.
        let fill = if hovered && item.enabled {
            BALLAST_D
        } else {
            BG1
        };
        if bg.0 != fill {
            bg.0 = fill;
        }

        let edge = if is_selected { HI } else { Color::NONE };
        *border = BorderColor::all(edge);

        for child in children.iter() {
            if let Ok(mut colour) = labels.get_mut(child) {
                *colour = if !item.enabled {
                    text_disabled()
                } else if is_selected {
                    text_accent()
                } else {
                    text_primary()
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(flags: &[bool]) -> Vec<(usize, MenuItem)> {
        flags
            .iter()
            .enumerate()
            .map(|(i, enabled)| {
                let item = MenuItem::new(i, MenuAction::Inert);
                (i, if *enabled { item } else { item.disabled() })
            })
            .collect()
    }

    #[test]
    fn selection_wraps_in_both_directions() {
        let list = rows(&[true, true, true]);
        assert_eq!(advance_selection(&list, 0, 1), 1);
        assert_eq!(advance_selection(&list, 2, 1), 0);
        assert_eq!(advance_selection(&list, 0, -1), 2);
    }

    #[test]
    fn selection_skips_disabled_rows() {
        let list = rows(&[true, false, false, true]);
        assert_eq!(advance_selection(&list, 0, 1), 3);
        assert_eq!(advance_selection(&list, 3, 1), 0);
        assert_eq!(advance_selection(&list, 0, -1), 3);
    }

    #[test]
    fn a_list_of_one_stays_put() {
        let list = rows(&[true]);
        assert_eq!(advance_selection(&list, 0, 1), 0);
        assert_eq!(advance_selection(&list, 0, -1), 0);
    }

    #[test]
    fn an_all_disabled_list_never_moves() {
        let list = rows(&[false, false]);
        assert_eq!(advance_selection(&list, 0, 1), 0);
    }

    #[test]
    fn an_overlay_takes_input_from_the_screen_underneath() {
        let screen = MenuList {
            selected: 3,
            layer: LAYER_SCREEN,
        };
        let overlay = MenuList {
            selected: 1,
            layer: LAYER_OVERLAY,
        };
        let lists = [screen, overlay];
        assert_eq!(
            active_list(lists.iter()).map(|l| l.selected),
            Some(1),
            "the settings panel, not the title menu behind it, owns the cursor"
        );
    }

    #[test]
    fn with_no_overlay_the_screen_owns_input() {
        let lists = [MenuList {
            selected: 2,
            layer: LAYER_SCREEN,
        }];
        assert_eq!(active_list(lists.iter()).map(|l| l.selected), Some(2));
        assert_eq!(active_list([].iter()).map(|l| l.selected), None);
    }

    #[test]
    fn rows_default_to_the_screen_layer() {
        assert_eq!(MenuItem::new(0, MenuAction::Inert).layer, LAYER_SCREEN);
        assert_eq!(
            MenuItem::mouse_only(MenuAction::Inert)
                .on_layer(LAYER_OVERLAY)
                .layer,
            LAYER_OVERLAY
        );
    }

    #[test]
    fn option_rows_carry_both_step_directions() {
        let item = MenuItem::new(0, MenuAction::RerollSeed).with_steps(
            MenuAction::CycleMapOption(OptionField::Size, -1),
            MenuAction::CycleMapOption(OptionField::Size, 1),
        );
        assert_eq!(
            item.left,
            Some(MenuAction::CycleMapOption(OptionField::Size, -1))
        );
        assert_eq!(
            item.right,
            Some(MenuAction::CycleMapOption(OptionField::Size, 1))
        );
    }
}
