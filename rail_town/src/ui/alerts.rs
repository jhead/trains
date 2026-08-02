//! Top-right alert strip — non-modal, clickable, dismissible.

use bevy::prelude::*;
use rail_map::tile_to_world;
use rail_sim::{AlertBoard, AlertFocus, StationRegistry};

use crate::inspect::{Selectable, Selection};
use crate::map::CameraFocusRequest;
use crate::palette::{BG1, HI, OUTLINE, WARN};
use crate::ui::kit::{
    body_font, micro_font, text_primary, text_secondary, text_warn, SPACE_2, SPACE_3, STATUS_H,
};

const MAX_VISIBLE: usize = 3;

#[derive(Component)]
pub struct AlertStripRoot;

#[derive(Component)]
pub struct AlertRow {
    pub alert_id: u64,
}

#[derive(Component)]
pub struct AlertDismissAllButton;

#[derive(Component)]
pub struct AlertCountText;

#[derive(Resource, Debug, Default)]
pub(crate) struct AlertUiCache {
    fingerprint: String,
}

pub fn setup_alerts_ui(mut commands: Commands) {
    commands.insert_resource(AlertUiCache::default());
    commands.spawn((
        AlertStripRoot,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(STATUS_H + SPACE_2),
            right: Val::Px(SPACE_3),
            width: Val::Px(320.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(4.0),
            align_items: AlignItems::Stretch,
            ..default()
        },
        ZIndex(11),
    ));
}

pub fn update_alerts_ui(
    board: Res<AlertBoard>,
    mut cache: ResMut<AlertUiCache>,
    mut commands: Commands,
    root_q: Query<Entity, With<AlertStripRoot>>,
    children_q: Query<&Children, With<AlertStripRoot>>,
) {
    let fingerprint = board
        .iter()
        .map(|a| format!("{}:{}", a.id, a.message))
        .collect::<Vec<_>>()
        .join("|");
    if fingerprint == cache.fingerprint {
        return;
    }
    cache.fingerprint = fingerprint;

    let Ok(root) = root_q.single() else {
        return;
    };

    if let Ok(children) = children_q.get(root) {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    let total = board.len();
    if total == 0 {
        return;
    }

    commands.entity(root).with_children(|strip| {
        let visible: Vec<_> = board.iter().take(MAX_VISIBLE).collect();
        for alert in &visible {
            strip
                .spawn((
                    Button,
                    AlertRow {
                        alert_id: alert.id,
                    },
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(SPACE_2),
                        padding: UiRect::axes(Val::Px(SPACE_2), Val::Px(4.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::ZERO,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(BG1),
                    BorderColor::all(OUTLINE),
                ))
                .with_children(|row| {
                    row.spawn((Text::new("!"), body_font(), text_warn()));
                    row.spawn((
                        Text::new(alert.message.clone()),
                        micro_font(),
                        text_primary(),
                    ));
                });
        }
        if total > MAX_VISIBLE {
            strip.spawn((
                AlertCountText,
                Text::new(format!("+{} more", total - MAX_VISIBLE)),
                micro_font(),
                text_secondary(),
            ));
        }
        strip
            .spawn((
                Button,
                AlertDismissAllButton,
                Node {
                    padding: UiRect::axes(Val::Px(SPACE_2), Val::Px(2.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::ZERO,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(BG1),
                BorderColor::all(OUTLINE),
            ))
            .with_children(|b| {
                b.spawn((Text::new("Dismiss all"), micro_font(), text_secondary()));
            });
    });
}

pub fn alert_row_clicks(
    interactions: Query<(&Interaction, &AlertRow), (Changed<Interaction>, With<Button>)>,
    board: Res<AlertBoard>,
    stations: Res<StationRegistry>,
    mut focus: ResMut<CameraFocusRequest>,
    mut selection: ResMut<Selection>,
) {
    for (interaction, row) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(alert) = board.iter().find(|a| a.id == row.alert_id) else {
            continue;
        };
        match alert.focus {
            AlertFocus::Station(id) => selection.set(Selectable::Station(id)),
            AlertFocus::Train(id) => selection.set(Selectable::Train(id)),
            _ => {}
        }
        if let Some(tile) = focus_tile(alert.focus, &stations) {
            let (wx, wy) = tile_to_world(tile);
            focus.0 = Some(Vec2::new(wx, wy));
        }
    }
}

pub fn alert_dismiss_all_clicks(
    interactions: Query<&Interaction, (Changed<Interaction>, With<AlertDismissAllButton>)>,
    mut board: ResMut<AlertBoard>,
) {
    for interaction in &interactions {
        if *interaction == Interaction::Pressed {
            board.dismiss_all();
        }
    }
}

pub fn update_alert_row_hover(
    mut q: Query<(&Interaction, &mut BorderColor), (With<AlertRow>, With<Button>)>,
) {
    for (interaction, mut border) in &mut q {
        *border = if matches!(*interaction, Interaction::Hovered | Interaction::Pressed) {
            BorderColor::all(HI)
        } else {
            BorderColor::all(OUTLINE)
        };
    }
}

fn focus_tile(
    focus: AlertFocus,
    stations: &StationRegistry,
) -> Option<rail_sim::TileCoord> {
    match focus {
        AlertFocus::Tile(t) => Some(t),
        AlertFocus::Station(id) => stations.get(id).map(|s| s.tile),
        AlertFocus::Train(_) => None, // tile already preferred when parked
        AlertFocus::None => None,
    }
}

// Keep WARN referenced for kit parity.
#[allow(dead_code)]
fn _warn() -> Color {
    WARN
}
