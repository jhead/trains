//! Bottom-centre toolbar — discoverable tool buttons with hotkey labels.

use bevy::prelude::*;
use rail_sim::commands::BuyTrain;
use rail_sim::{CommandBuffer, CommandKind, TrainKind};

use crate::palette::{BALLAST_L, BALLAST_M, BG1, HI, OUTLINE, RAIL_L};
use crate::track::{BuildTool, TrackToolState};
use crate::trains::{TrainPlaceKind, TrainToolState};
use crate::ui::kit::{micro_font, text_secondary, SPACE_2, TOOL_SLOT};

#[derive(Component)]
pub struct ToolbarRoot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarTool {
    Build,
    Demolish,
    Transit,
    Transport,
}

#[derive(Component)]
pub struct ToolbarButton {
    pub tool: ToolbarTool,
}

pub fn setup_toolbar(mut commands: Commands) {
    let slots = [
        (ToolbarTool::Build, "Track", "B"),
        (ToolbarTool::Demolish, "Demolish", "X"),
        (ToolbarTool::Transit, "Transit", "T"),
        (ToolbarTool::Transport, "Transport", "G"),
    ];

    commands
        .spawn((
            ToolbarRoot,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(SPACE_2),
                left: Val::Percent(50.0),
                // Centre via translate — Bevy UI uses margin auto as alternative.
                margin: UiRect::left(Val::Px(-(TOOL_SLOT * slots.len() as f32) / 2.0)),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(2.0),
                height: Val::Px(TOOL_SLOT),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::ZERO,
                padding: UiRect::all(Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(BG1),
            BorderColor::all(OUTLINE),
            ZIndex(10),
        ))
        .with_children(|bar| {
            for (tool, name, key) in slots {
                bar.spawn((
                    Button,
                    ToolbarButton { tool },
                    Node {
                        width: Val::Px(TOOL_SLOT - 4.0),
                        height: Val::Px(TOOL_SLOT - 4.0),
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(2.0),
                        border: UiRect::all(Val::Px(2.0)),
                        border_radius: BorderRadius::ZERO,
                        ..default()
                    },
                    BackgroundColor(BG1),
                    BorderColor::all(BALLAST_M),
                ))
                .with_children(|slot| {
                    slot.spawn((
                        Text::new(name),
                        TextFont::from_font_size(10.0),
                        TextColor(RAIL_L),
                    ));
                    slot.spawn((Text::new(key), micro_font(), text_secondary()));
                });
            }
        });
}

pub fn update_toolbar_visuals(
    track: Res<TrackToolState>,
    train: Option<Res<TrainToolState>>,
    mut q: Query<(&ToolbarButton, &Interaction, &mut BorderColor, &Children), With<Button>>,
    mut child_colors: Query<&mut TextColor>,
) {
    let placing = train.as_ref().is_some_and(|t| t.place_mode);
    let place_kind = train.as_ref().map(|t| t.kind);
    let active = active_tool(track.tool, placing, place_kind);

    for (btn, interaction, mut border, children) in &mut q {
        let selected = btn.tool == active;
        *border = if selected {
            BorderColor::all(HI)
        } else if matches!(interaction, Interaction::Hovered) {
            BorderColor::all(BALLAST_L)
        } else {
            BorderColor::all(BALLAST_M)
        };
        for child in children.iter() {
            if let Ok(mut c) = child_colors.get_mut(child) {
                // Name stays primary; key stays secondary unless selected.
                let _ = &mut c;
            }
        }
        let _ = selected;
    }
}

pub fn toolbar_button_clicks(
    interactions: Query<(&Interaction, &ToolbarButton), (Changed<Interaction>, With<Button>)>,
    mut buffer: ResMut<CommandBuffer>,
    mut track: ResMut<TrackToolState>,
    mut train: ResMut<TrainToolState>,
) {
    for (interaction, btn) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        apply_toolbar_tool(btn.tool, &mut buffer, &mut track, &mut train);
    }
}

fn apply_toolbar_tool(
    tool: ToolbarTool,
    buffer: &mut CommandBuffer,
    track: &mut TrackToolState,
    train: &mut TrainToolState,
) {
    match tool {
        ToolbarTool::Build => {
            track.tool = BuildTool::Build;
            track.anchor = None;
            track.drag = None;
            track.suppress_build_click = false;
            train.place_mode = false;
        }
        ToolbarTool::Demolish => {
            track.tool = BuildTool::Demolish;
            track.anchor = None;
            track.drag = None;
            track.suppress_build_click = false;
            train.place_mode = false;
        }
        ToolbarTool::Transit => {
            buffer.push(CommandKind::BuyTrain(BuyTrain {
                kind: TrainKind::Transit,
            }));
            train.place_mode = true;
            train.kind = TrainPlaceKind::Transit;
            track.anchor = None;
            track.drag = None;
            track.suppress_build_click = true;
        }
        ToolbarTool::Transport => {
            buffer.push(CommandKind::BuyTrain(BuyTrain {
                kind: TrainKind::Transport,
            }));
            train.place_mode = true;
            train.kind = TrainPlaceKind::Transport;
            track.anchor = None;
            track.drag = None;
            track.suppress_build_click = true;
        }
    }
}

fn active_tool(
    tool: BuildTool,
    placing: bool,
    kind: Option<TrainPlaceKind>,
) -> ToolbarTool {
    if placing {
        return match kind.unwrap_or_default() {
            TrainPlaceKind::Transit => ToolbarTool::Transit,
            TrainPlaceKind::Transport => ToolbarTool::Transport,
        };
    }
    match tool {
        BuildTool::Build => ToolbarTool::Build,
        BuildTool::Demolish => ToolbarTool::Demolish,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_tool_maps_modes() {
        assert_eq!(
            active_tool(BuildTool::Build, false, None),
            ToolbarTool::Build
        );
        assert_eq!(
            active_tool(BuildTool::Demolish, false, None),
            ToolbarTool::Demolish
        );
        assert_eq!(
            active_tool(BuildTool::Build, true, Some(TrainPlaceKind::Transit)),
            ToolbarTool::Transit
        );
    }
}
