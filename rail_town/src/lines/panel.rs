//! Left-side lines panel — list + simple strip diagram of stops.

use bevy::prelude::*;
use rail_sim::{
    line_colour_rgba, AssignTrainToLine, CommandBuffer, CommandKind, LineId, LineRegistry,
    StationRegistry, Train,
};

use crate::inspect::{Selectable, Selection};
use crate::palette::{BALLAST_L, BALLAST_M, BG1, HI, OUTLINE, RAIL_L};
use crate::ui::kit::{
    body_font, micro_font, text_primary, text_secondary, text_warn, SPACE_2, SPACE_3, STATUS_H,
};

use super::tools::LineToolState;

#[derive(Component)]
pub struct LinesPanelRoot;

#[derive(Component)]
pub struct LinesPanelBody;

#[derive(Component)]
pub struct LineRowButton {
    pub line: LineId,
}

#[derive(Component)]
pub struct AssignTrainButton {
    pub line: LineId,
}

#[derive(Resource, Debug, Default)]
pub(crate) struct LinesPanelCache {
    fingerprint: String,
}

#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct FocusedLine(pub Option<LineId>);

pub fn setup_lines_panel(mut commands: Commands) {
    commands.insert_resource(LinesPanelCache::default());
    commands.insert_resource(FocusedLine::default());
    commands
        .spawn((
            LinesPanelRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(STATUS_H + SPACE_2),
                left: Val::Px(SPACE_3),
                width: Val::Px(260.0),
                max_height: Val::Px(320.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                padding: UiRect::all(Val::Px(SPACE_2)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::ZERO,
                ..default()
            },
            BackgroundColor(BG1),
            BorderColor::all(OUTLINE),
            ZIndex(11),
        ))
        .with_children(|root| {
            root.spawn((
                Text::new("Lines"),
                body_font(),
                text_primary(),
            ));
            root.spawn((
                Text::new("L - click stations - Enter"),
                micro_font(),
                text_secondary(),
            ));
            root.spawn((
                LinesPanelBody,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    width: Val::Percent(100.0),
                    ..default()
                },
            ));
        });
}

pub fn update_lines_panel(
    lines: Res<LineRegistry>,
    stations: Res<StationRegistry>,
    line_tool: Res<LineToolState>,
    focused: Res<FocusedLine>,
    mut cache: ResMut<LinesPanelCache>,
    mut commands: Commands,
    body_q: Query<Entity, With<LinesPanelBody>>,
    children_q: Query<&Children, With<LinesPanelBody>>,
) {
    let mut fp = format!(
        "d:{}:w:{:?}:f:{:?}:",
        line_tool.draft_stops.len(),
        line_tool.warn,
        focused.0
    );
    for line in lines.iter() {
        fp.push_str(&format!(
            "{}:{}:{}:{};",
            line.id.0,
            line.name,
            line.stops.len(),
            line.trains.len()
        ));
    }
    if fp == cache.fingerprint {
        return;
    }
    cache.fingerprint = fp;

    let Ok(body) = body_q.single() else {
        return;
    };
    if let Ok(children) = children_q.get(body) {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    commands.entity(body).with_children(|body| {
        // Draft strip
        if line_tool.active || !line_tool.draft_stops.is_empty() {
            body.spawn((
                Text::new(if line_tool.active {
                    "Draft (Enter to confirm)"
                } else {
                    "Draft"
                }),
                micro_font(),
                text_secondary(),
            ));
            let strip = draft_strip(&stations, &line_tool.draft_stops);
            body.spawn((Text::new(strip), body_font(), text_primary()));
            if let Some(warn) = &line_tool.warn {
                body.spawn((Text::new(warn.clone()), micro_font(), text_warn()));
            }
        }

        let mut sorted: Vec<_> = lines.iter().collect();
        sorted.sort_by_key(|l| l.id.0);

        if sorted.is_empty() && !line_tool.active {
            body.spawn((
                Text::new("No lines yet"),
                micro_font(),
                text_secondary(),
            ));
            return;
        }

        for line in sorted {
            let selected = focused.0 == Some(line.id);
            let rgb = line_colour_rgba(line.colour);
            let colour = Color::srgb(
                rgb[0] as f32 / 255.0,
                rgb[1] as f32 / 255.0,
                rgb[2] as f32 / 255.0,
            );
            let strip = stop_strip(&stations, &line.stops);
            body.spawn((
                Button,
                LineRowButton { line: line.id },
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    padding: UiRect::all(Val::Px(4.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::ZERO,
                    width: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(BG1),
                BorderColor::all(if selected { HI } else { BALLAST_M }),
            ))
            .with_children(|row| {
                row.spawn((
                    Text::new(format!("o {}", line.name)),
                    body_font(),
                    TextColor(colour),
                ));
                row.spawn((Text::new(strip), micro_font(), TextColor(RAIL_L)));
                row.spawn((
                    Text::new(format!("{} train(s)", line.trains.len())),
                    micro_font(),
                    text_secondary(),
                ));
                row.spawn((
                    Button,
                    AssignTrainButton { line: line.id },
                    Node {
                        padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::ZERO,
                        ..default()
                    },
                    BackgroundColor(BG1),
                    BorderColor::all(BALLAST_L),
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("Assign selected train"),
                        micro_font(),
                        text_secondary(),
                    ));
                });
            });
        }
    });
}

pub fn line_row_clicks(
    interactions: Query<(&Interaction, &LineRowButton), (Changed<Interaction>, With<Button>)>,
    mut focused: ResMut<FocusedLine>,
) {
    for (interaction, btn) in &interactions {
        if *interaction == Interaction::Pressed {
            focused.0 = Some(btn.line);
        }
    }
}

pub fn assign_train_clicks(
    interactions: Query<(&Interaction, &AssignTrainButton), (Changed<Interaction>, With<Button>)>,
    selection: Res<Selection>,
    trains: Query<&Train>,
    mut buffer: ResMut<CommandBuffer>,
) {
    for (interaction, btn) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(Selectable::Train(id)) = selection.0 else {
            continue;
        };
        if trains.iter().any(|t| t.id == id) {
            buffer.push(CommandKind::AssignTrainToLine(AssignTrainToLine {
                train: id,
                line: btn.line,
            }));
        }
    }
}

fn stop_strip(stations: &StationRegistry, stops: &[rail_sim::StationId]) -> String {
    if stops.is_empty() {
        return "-".into();
    }
    stops
        .iter()
        .map(|id| {
            stations
                .get(*id)
                .map(|s| s.name.as_str())
                .unwrap_or("?")
                .chars()
                .take(8)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" - ")
}

fn draft_strip(stations: &StationRegistry, stops: &[rail_sim::StationId]) -> String {
    if stops.is_empty() {
        return "(click stations)".into();
    }
    stop_strip(stations, stops)
}
