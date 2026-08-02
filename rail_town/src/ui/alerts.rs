//! Alerts window — non-modal, clickable, dismissible.
//!
//! Alerts were the primary way the player found out the railway was unwell,
//! which is why the very first frame of a new game shouted "Westbrook service
//! low (0)" at someone who had done nothing wrong. Health is now a permanent
//! readout ([`super::health`]), and this window is what it always should have
//! been: a list of things that changed for the worse and can be acted on.
//!
//! Brief 05 §7's rule is enforced by [`alert_is_actionable`]: an alert about a
//! station that has never been served is not shown here and is not counted on
//! the status strip's bell.

use bevy::prelude::*;
use rail_map::tile_to_world;
use rail_sim::{AlertBoard, AlertFocus, StationRegistry, StationService};

use crate::inspect::{Selectable, Selection};
use crate::palette::{BG1, OUTLINE};
use crate::ui::health::alert_is_actionable;
use crate::ui::kit::{
    chrome_button_node, control_border, micro_font, text_primary, text_secondary, text_warn,
    SPACE_1,
};
use crate::ui::window::{window_root, WindowId, WindowManager};

#[derive(Component)]
pub struct AlertStripRoot;

#[derive(Component)]
pub struct AlertListBody;

#[derive(Component)]
pub struct AlertRow {
    pub alert_id: u64,
}

#[derive(Component)]
pub struct AlertDismissAllButton;

#[derive(Resource, Debug, Default)]
pub(crate) struct AlertUiCache {
    fingerprint: String,
}

pub fn setup_alerts_ui(mut commands: Commands) {
    commands.insert_resource(AlertUiCache::default());
    commands
        .spawn((AlertStripRoot, window_root(WindowId::Alerts, 300.0)))
        .with_children(|panel| {
            panel.spawn((
                AlertListBody,
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(SPACE_1),
                    ..default()
                },
            ));
        });
}

pub fn update_alerts_ui(
    manager: Res<WindowManager>,
    board: Res<AlertBoard>,
    service: Res<StationService>,
    mut cache: ResMut<AlertUiCache>,
    mut commands: Commands,
    body_q: Query<Entity, With<AlertListBody>>,
    children_q: Query<&Children, With<AlertListBody>>,
) {
    if !manager.is_open(WindowId::Alerts) {
        return;
    }
    let visible: Vec<(u64, String)> = board
        .iter()
        .filter(|a| alert_is_actionable(a, &service))
        .map(|a| (a.id, a.message.clone()))
        .collect();

    let fingerprint = visible
        .iter()
        .map(|(id, message)| format!("{id}:{message}"))
        .collect::<Vec<_>>()
        .join("|");
    if fingerprint == cache.fingerprint {
        return;
    }
    cache.fingerprint = fingerprint;

    let Ok(body) = body_q.single() else {
        return;
    };
    if let Ok(children) = children_q.get(body) {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    commands.entity(body).with_children(|list| {
        if visible.is_empty() {
            list.spawn((
                Text::new("Nothing needs you right now."),
                micro_font(),
                text_secondary(),
            ));
            return;
        }
        for (id, message) in &visible {
            list.spawn((
                Button,
                AlertRow { alert_id: *id },
                Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(SPACE_1),
                    padding: UiRect::axes(Val::Px(SPACE_1), Val::Px(1.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::ZERO,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(BG1),
                BorderColor::all(OUTLINE),
            ))
            .with_children(|row| {
                row.spawn((Text::new("!"), micro_font(), text_warn()));
                row.spawn((Text::new(message.clone()), micro_font(), text_primary()));
            });
        }
        let (node, bg, border) = chrome_button_node(SPACE_1, 1.0);
        list.spawn((Button, AlertDismissAllButton, node, bg, border))
            .with_children(|b| {
                b.spawn((Text::new("Dismiss all"), micro_font(), text_secondary()));
            });
    });
}

pub fn alert_row_clicks(
    interactions: Query<(&Interaction, &AlertRow), (Changed<Interaction>, With<Button>)>,
    board: Res<AlertBoard>,
    stations: Res<StationRegistry>,
    mut focus: ResMut<crate::map::CameraFocusRequest>,
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
    mut q: Query<
        (&Interaction, &mut BorderColor),
        (Changed<Interaction>, Or<(With<AlertRow>, With<AlertDismissAllButton>)>),
    >,
) {
    for (interaction, mut border) in &mut q {
        let hovered = matches!(interaction, Interaction::Hovered | Interaction::Pressed);
        *border = control_border(false, hovered);
    }
}

fn focus_tile(focus: AlertFocus, stations: &StationRegistry) -> Option<rail_sim::TileCoord> {
    match focus {
        AlertFocus::Tile(t) => Some(t),
        AlertFocus::Station(id) => stations.get(id).map(|s| s.tile),
        AlertFocus::Train(_) => None, // tile already preferred when parked
        AlertFocus::None => None,
    }
}
