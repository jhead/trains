//! New Map — options on the left, a live schematic preview on the right.
//!
//! Design [`09 §3`](../../../docs/design/09-shell-and-menus.md): one screen, no
//! wizard, and the preview is the point. Every option change regenerates the map
//! and re-measures the readouts, so rolling the dice until something looks
//! interesting is a legitimate way to start a game.
//!
//! The screen is rebuilt whole whenever [`MapOptions`] changes. That is cheap at
//! menu scale and keeps the row values, the preview and the readouts impossible
//! to get out of step with each other.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::palette::{BG0, OUTLINE};
use crate::ui::kit::{
    body_font, micro_font, text_accent, text_secondary, SPACE_2, SPACE_3, SPACE_4,
};

use super::map_options::{
    roll_seed, schematic_rgba, MapOptions, MapReadouts, OptionField, PREVIEW_BOX, SEED_MAX,
};
use super::widgets::{
    screen_root, shell_panel, spawn_button, spawn_note, spawn_panel_title, spawn_row_with_note,
    spawn_rule, spawn_section_label, MenuAction, MenuCursor, MenuItem, MenuList, LAYER_SCREEN,
};
use super::ShellState;

/// Options column width. Wide enough for `Resources  Scattered` on one line.
const OPTIONS_W: f32 = 320.0;

/// Nav index of the first non-option row (Back), after the seven option rows.
const BACK_ROW: usize = OptionField::ALL.len();
const BEGIN_ROW: usize = BACK_ROW + 1;

/// The setup being edited. Committed to [`super::PendingWorld`] by Begin.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DraftMapOptions(pub MapOptions);

/// Preview texture, kept alive between rebuilds by holding its handle.
#[derive(Resource, Debug, Default)]
pub struct PreviewImage(pub Handle<Image>);

/// Rebuild the screen whenever the draft changes (and once on entry).
pub fn rebuild_new_map_screen(
    mut commands: Commands,
    draft: Res<DraftMapOptions>,
    cursor: Res<MenuCursor>,
    mut images: ResMut<Assets<Image>>,
    mut preview: ResMut<PreviewImage>,
    existing: Query<Entity, With<NewMapRoot>>,
) {
    if !draft.is_changed() && !existing.is_empty() {
        return;
    }
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    let options = draft.0;
    let map = options.generate();
    let readouts = MapReadouts::measure(&map);

    let mut image = Image::new(
        Extent3d {
            width: map.width,
            height: map.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        schematic_rgba(&map),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    // Nearest sampling and a whole-number scale: the preview is pixel art too.
    image.sampler = ImageSampler::nearest();
    preview.0 = images.add(image);

    let preview_px = (map.width * options.size.preview_scale()) as f32;

    commands
        .spawn((
            NewMapRoot,
            screen_root(
                "shell::new_map",
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    column_gap: Val::Px(SPACE_4),
                    ..default()
                },
            ),
            DespawnOnExit(ShellState::NewMap),
        ))
        .with_children(|root| {
            root.spawn((
                MenuList {
                    selected: cursor.0,
                    layer: LAYER_SCREEN,
                },
                shell_panel(Node {
                    width: Val::Px(OPTIONS_W),
                    ..default()
                }),
            ))
            .with_children(|panel| {
                spawn_panel_title(panel, "New Map");
                for (index, field) in OptionField::ALL.iter().enumerate() {
                    let item = MenuItem::new(index, option_activate_action(*field)).with_steps(
                        MenuAction::CycleMapOption(*field, -1),
                        MenuAction::CycleMapOption(*field, 1),
                    );
                    spawn_row_with_note(
                        panel,
                        item,
                        field.label(),
                        &field.value_label(&options),
                        field.pending_note(),
                    );
                }
                spawn_rule(panel);
                panel
                    .spawn(Node {
                        flex_direction: FlexDirection::Row,
                        justify_content: JustifyContent::FlexEnd,
                        column_gap: Val::Px(SPACE_2),
                        ..default()
                    })
                    .with_children(|row| {
                        spawn_button(row, MenuItem::new(BACK_ROW, MenuAction::Back), "Back");
                        spawn_button(row, MenuItem::new(BEGIN_ROW, MenuAction::Begin), "Begin");
                    });
                spawn_note(panel, "<- -> change   Enter roll seed   digits type a seed");
            });

            root.spawn(shell_panel(Node {
                align_items: AlignItems::Center,
                ..default()
            }))
            .with_children(|panel| {
                panel.spawn((
                    ImageNode::new(preview.0.clone()),
                    Node {
                        width: Val::Px(preview_px),
                        height: Val::Px(preview_px),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::ZERO,
                        ..default()
                    },
                    BorderColor::all(OUTLINE),
                    BackgroundColor(BG0),
                ));
                spawn_section_label(panel, "This map");
                spawn_readouts(panel, &readouts);
                panel.spawn((
                    Text::new(format!("share  {}", options.share_code())),
                    micro_font(),
                    text_accent(),
                ));
            });
        });
}

/// Marker so the rebuild can find and drop the previous screen.
#[derive(Component)]
pub struct NewMapRoot;

/// Enter on the Seed row rolls a new one; on any other row it steps forward.
fn option_activate_action(field: OptionField) -> MenuAction {
    match field {
        OptionField::Seed => MenuAction::RerollSeed,
        other => MenuAction::CycleMapOption(other, 1),
    }
}

fn spawn_readouts(parent: &mut ChildSpawnerCommands, readouts: &MapReadouts) {
    let lines = [
        format!("land {}%", readouts.land_pct),
        format!("towns {}", readouts.towns),
        format!("rivers {}", readouts.rivers),
        format!("passes {}", readouts.passes),
        format!("mainland {}%", readouts.mainland_pct),
    ];
    parent
        .spawn(Node {
            width: Val::Px(PREVIEW_BOX),
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            column_gap: Val::Px(SPACE_3),
            row_gap: Val::Px(SPACE_2),
            ..default()
        })
        .with_children(|row| {
            for line in lines {
                row.spawn((Text::new(line), body_font(), text_secondary()));
            }
        });
}

/// Type digits straight into the seed while the Seed row is selected.
///
/// Seeds are meant to be shared and re-entered (design 02 §5); a field you can
/// only nudge one step at a time is not really shareable.
pub fn seed_typing(
    mut input: MessageReader<KeyboardInput>,
    cursor: Res<MenuCursor>,
    mut draft: ResMut<DraftMapOptions>,
) {
    if cursor.0 != OptionField::Seed.index_in_rows() {
        input.clear();
        return;
    }
    for event in input.read() {
        if !event.state.is_pressed() {
            continue;
        }
        match &event.logical_key {
            Key::Character(text) => {
                for ch in text.chars().filter(char::is_ascii_digit) {
                    let digit = ch as u64 - '0' as u64;
                    let next = draft.0.seed * 10 + digit;
                    // Silently ignore overflow rather than wrapping to nonsense.
                    if next <= SEED_MAX {
                        draft.0.seed = next;
                    }
                }
            }
            Key::Backspace => draft.0.seed /= 10,
            _ => {}
        }
    }
}

impl OptionField {
    /// Row index of this option on the New Map screen.
    pub fn index_in_rows(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }
}

/// Apply one menu action to the draft. Returns `true` when it changed something.
pub fn apply_new_map_action(draft: &mut DraftMapOptions, action: MenuAction) -> bool {
    match action {
        MenuAction::CycleMapOption(field, delta) => {
            field.cycle(&mut draft.0, delta);
            true
        }
        MenuAction::RerollSeed => {
            draft.0.seed = roll_seed();
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::map_options::{MapSize, TerrainStyle};

    #[test]
    fn rolling_the_dice_changes_the_seed_and_nothing_else() {
        let mut draft = DraftMapOptions(MapOptions {
            size: MapSize::Large,
            terrain: TerrainStyle::Rugged,
            ..MapOptions::default()
        });
        let before = draft.0;
        assert!(apply_new_map_action(&mut draft, MenuAction::RerollSeed));
        assert_eq!(draft.0.size, before.size);
        assert_eq!(draft.0.terrain, before.terrain);
    }

    #[test]
    fn stepping_an_option_reports_a_change() {
        let mut draft = DraftMapOptions::default();
        assert!(apply_new_map_action(
            &mut draft,
            MenuAction::CycleMapOption(OptionField::Size, 1)
        ));
        assert_ne!(draft.0.size, MapOptions::default().size);
    }

    #[test]
    fn unrelated_actions_leave_the_draft_alone() {
        let mut draft = DraftMapOptions::default();
        assert!(!apply_new_map_action(&mut draft, MenuAction::Begin));
        assert_eq!(draft.0, MapOptions::default());
    }

    #[test]
    fn option_rows_come_before_the_buttons() {
        assert_eq!(BACK_ROW, OptionField::ALL.len());
        assert_eq!(BEGIN_ROW, BACK_ROW + 1);
        assert_eq!(OptionField::Seed.index_in_rows(), 0);
    }
}
