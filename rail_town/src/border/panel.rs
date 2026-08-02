//! The Neighbours panel — your four edges, and what flows each way.
//!
//! `docs/design/12-multiplayer.md` §9. One row per edge showing its link (echo
//! or real), the partner's town name, what flows each way, the relationship's
//! maturity, and a link / unlink action. Toggled with `N`.
//!
//! Everything the player can do to a border happens here, and every one of those
//! actions is an ordinary [`CommandKind`](rail_sim::CommandKind) pushed into the
//! [`CommandBuffer`] — the same path a station or a length of track takes. There
//! is no border-shaped side door into the simulation.
//!
//! An echo is labelled as an echo, in as many words, because the data says so
//! (`PresenceSource`) and §6 asks for it out loud: *"An echo is always honestly
//! labelled in the interface. Not deceptive, just present."*

use bevy::prelude::*;
use rail_sim::border::{
    push_border_command, railhead_on_edge, AssignTrainToBorder, BorderCommand, BorderEdge,
    BorderLink, BorderRegistry, BorderRun, CloseBorder, OpenBorder, BORDER_PORTAL_COST_CENTS,
};
use rail_sim::commands::TrainKind;
use rail_sim::{CommandBuffer, TileCoord, TrackNetwork, TrackTerrain, Train, TrainId, GROUND_LAYER};

use crate::palette::{BALLAST_L, BG1, HI, OUTLINE, RAIL_L};
use crate::ui::kit::{
    body_font, micro_font, panel_node, text_accent, text_primary, text_secondary, FONT_MICRO,
    SPACE_1, SPACE_2, SPACE_3, STATUS_H,
};

#[derive(Resource, Debug, Default)]
pub struct NeighboursPanelState {
    pub open: bool,
}

#[derive(Component)]
pub struct NeighboursPanelRoot;

#[derive(Component, Debug, Clone, Copy)]
pub struct NeighbourRowText {
    pub edge: BorderEdge,
}

/// What a row's button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeighbourAction {
    /// Pay for the portal where the line already reaches the edge.
    Open,
    /// Put a free goods train on this border run.
    Send,
    /// Sever the link. Never destructive.
    Close,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct NeighbourButton {
    pub edge: BorderEdge,
    pub action: NeighbourAction,
}

/// Cached row strings so the panel only rewrites text that changed.
#[derive(Resource, Debug, Default)]
pub(crate) struct NeighboursUiCache {
    rows: [String; 4],
}

pub fn setup_neighbours_panel(mut commands: Commands) {
    commands.init_resource::<NeighboursPanelState>();
    commands.insert_resource(NeighboursUiCache::default());

    let (node, bg, border) = panel_node(Node {
        position_type: PositionType::Absolute,
        top: Val::Px(STATUS_H + SPACE_2),
        right: Val::Px(SPACE_3),
        width: Val::Px(320.0),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(SPACE_2),
        padding: UiRect::all(Val::Px(SPACE_2)),
        display: Display::None,
        ..default()
    });

    commands
        .spawn((NeighboursPanelRoot, node, bg, border, ZIndex(12)))
        .with_children(|panel| {
            panel
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    width: Val::Percent(100.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((Text::new("Neighbours"), body_font(), text_accent()));
                    row.spawn((Text::new("N"), micro_font(), text_secondary()));
                });

            for edge in BorderEdge::ALL {
                panel
                    .spawn(Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(SPACE_1),
                        width: Val::Percent(100.0),
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn((
                            NeighbourRowText { edge },
                            Text::new(String::new()),
                            micro_font(),
                            text_primary(),
                        ));
                        row.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(SPACE_1),
                            ..default()
                        })
                        .with_children(|actions| {
                            for (action, label) in [
                                (NeighbourAction::Open, "Open portal"),
                                (NeighbourAction::Send, "Send a train"),
                                (NeighbourAction::Close, "Unlink"),
                            ] {
                                spawn_action(actions, edge, action, label);
                            }
                        });
                    });
            }

            panel.spawn((
                Text::new("Trade runs offline. Echo neighbours need no connection."),
                micro_font(),
                text_secondary(),
            ));
        });
}

fn spawn_action(
    parent: &mut ChildSpawnerCommands,
    edge: BorderEdge,
    action: NeighbourAction,
    label: &str,
) {
    parent
        .spawn((
            Button,
            NeighbourButton { edge, action },
            Node {
                padding: UiRect::axes(Val::Px(SPACE_1), Val::Px(2.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::ZERO,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                display: Display::None,
                ..default()
            },
            BackgroundColor(BG1),
            BorderColor::all(OUTLINE),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(label.to_string()),
                TextFont::from_font_size(FONT_MICRO),
                TextColor(RAIL_L),
            ));
        });
}

pub fn neighbours_panel_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<NeighboursPanelState>,
) {
    if keys.just_pressed(KeyCode::Escape) && state.open {
        state.open = false;
        return;
    }
    if keys.just_pressed(KeyCode::KeyN) {
        state.open = !state.open;
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_neighbours_panel(
    state: Res<NeighboursPanelState>,
    registry: Res<BorderRegistry>,
    network: Res<TrackNetwork>,
    terrain: Option<Res<TrackTerrain>>,
    mut cache: ResMut<NeighboursUiCache>,
    mut root_q: Query<&mut Node, (With<NeighboursPanelRoot>, Without<NeighbourButton>)>,
    mut text_q: Query<(&NeighbourRowText, &mut Text)>,
    mut button_q: Query<(&NeighbourButton, &mut Node), Without<NeighboursPanelRoot>>,
) {
    if let Ok(mut node) = root_q.single_mut() {
        node.display = if state.open {
            Display::Flex
        } else {
            Display::None
        };
    }
    if !state.open {
        return;
    }

    for edge in BorderEdge::ALL {
        let railhead = terrain
            .as_deref()
            .and_then(|t| railhead_on_edge(&network, t, edge, GROUND_LAYER));
        let line = edge_line(edge, registry.get(edge), railhead);
        let slot = edge.index();
        if cache.rows[slot] != line {
            cache.rows[slot] = line.clone();
            for (row, mut text) in &mut text_q {
                if row.edge == edge {
                    *text = Text::new(line.clone());
                }
            }
        }

        let open = registry.is_open(edge);
        for (button, mut node) in &mut button_q {
            if button.edge != edge {
                continue;
            }
            let show = match button.action {
                NeighbourAction::Open => !open && railhead.is_some(),
                NeighbourAction::Send | NeighbourAction::Close => open,
            };
            node.display = if show { Display::Flex } else { Display::None };
        }
    }
}

pub fn neighbour_button_hover(
    mut q: Query<(&Interaction, &mut BorderColor), With<NeighbourButton>>,
) {
    for (interaction, mut border) in &mut q {
        *border = match interaction {
            Interaction::Hovered => BorderColor::all(BALLAST_L),
            Interaction::Pressed => BorderColor::all(HI),
            Interaction::None => BorderColor::all(OUTLINE),
        };
    }
}

/// Turn a click into a command in the buffer. Nothing here touches the sim.
#[allow(clippy::too_many_arguments)]
pub fn neighbour_button_clicks(
    mut buffer: ResMut<CommandBuffer>,
    registry: Res<BorderRegistry>,
    network: Res<TrackNetwork>,
    terrain: Option<Res<TrackTerrain>>,
    trains: Query<(&Train, Option<&BorderRun>)>,
    buttons: Query<(&Interaction, &NeighbourButton), Changed<Interaction>>,
) {
    for (interaction, button) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match button.action {
            NeighbourAction::Open => {
                let Some(terrain) = terrain.as_deref() else {
                    continue;
                };
                let Some(tile) = railhead_on_edge(&network, terrain, button.edge, GROUND_LAYER)
                else {
                    continue;
                };
                push_border_command(
                    &mut buffer,
                    BorderCommand::Open(OpenBorder {
                        tile,
                        layer: GROUND_LAYER,
                        edge: button.edge,
                    }),
                );
            }
            NeighbourAction::Send => {
                if !registry.is_open(button.edge) {
                    continue;
                }
                let Some(train) = free_train_for_border(&trains) else {
                    continue;
                };
                push_border_command(
                    &mut buffer,
                    BorderCommand::Assign(AssignTrainToBorder {
                        train,
                        edge: Some(button.edge),
                    }),
                );
            }
            NeighbourAction::Close => {
                push_border_command(
                    &mut buffer,
                    BorderCommand::Close(CloseBorder { edge: button.edge }),
                );
            }
        }
    }
}

/// Lowest-numbered train not already on a border run, goods stock preferred.
fn free_train_for_border(trains: &Query<(&Train, Option<&BorderRun>)>) -> Option<TrainId> {
    let mut free: Vec<(TrainKind, TrainId)> = trains
        .iter()
        .filter(|(_, run)| run.is_none())
        .map(|(train, _)| (train.kind, train.id))
        .collect();
    free.sort_by_key(|(kind, id)| (!matches!(kind, TrainKind::Transport), id.0));
    free.first().map(|(_, id)| *id)
}

/// One row of the panel, as text.
///
/// Pure over its inputs so the wording is testable without a world.
fn edge_line(
    edge: BorderEdge,
    link: Option<&BorderLink>,
    railhead: Option<TileCoord>,
) -> String {
    let Some(link) = link else {
        return match railhead {
            Some(tile) => format!(
                "{} - closed\n  a line reaches ({}, {}) - {} to open",
                edge.title(),
                tile.x,
                tile.y,
                dollars(BORDER_PORTAL_COST_CENTS)
            ),
            None => format!(
                "{} - closed\n  run a line to the {} edge",
                edge.title(),
                edge.label()
            ),
        };
    };

    let badge = if link.is_echo() { "echo" } else { "linked" };
    let offer = link.their_offer();
    let out = link.our_offer();
    let headline = link.neighbour.presence.headline;
    let waiting = link.transit.len();
    let out_line = if waiting == 0 {
        String::new()
    } else if waiting == 1 {
        " - 1 train out".to_string()
    } else {
        format!(" - {waiting} trains out")
    };

    format!(
        "{} - {} ({})\n  they send {} x{} - they want {}\n  you send {} - {}% mature - {} people{}",
        edge.title(),
        link.town_name(),
        badge,
        offer.good.label(),
        offer.units_per_period,
        link.their_request().good.label(),
        out.good.label(),
        link.maturity(),
        headline.residents,
        out_line,
    )
}

fn dollars(cents: i64) -> String {
    let abs = cents.unsigned_abs();
    let d = abs / 100;
    let r = abs % 100;
    if r == 0 {
        format!("${d}")
    } else {
        format!("${d}.{r:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::border::BorderLink;

    fn link() -> BorderLink {
        BorderLink::opened(
            BorderEdge::East,
            TileCoord { x: 15, y: 8 },
            GROUND_LAYER,
            42,
            0,
            BORDER_PORTAL_COST_CENTS,
        )
    }

    #[test]
    fn a_closed_edge_says_what_to_do_next() {
        let line = edge_line(BorderEdge::North, None, None);
        assert!(line.contains("North - closed"));
        assert!(line.contains("run a line to the north edge"));

        let ready = edge_line(BorderEdge::North, None, Some(TileCoord { x: 3, y: 63 }));
        assert!(ready.contains("(3, 63)"));
        assert!(ready.contains("$1500"), "the price is on the button: {ready}");
    }

    #[test]
    fn an_open_edge_names_the_town_and_says_it_is_an_echo() {
        let link = link();
        let line = edge_line(BorderEdge::East, Some(&link), None);
        assert!(line.contains(link.town_name()));
        assert!(line.contains("(echo)"), "an echo is labelled: {line}");
        assert!(line.contains("they send"));
        assert!(line.contains("they want"));
        assert!(line.contains("you send"));
        assert!(line.contains("0% mature"));
        assert!(!line.contains("train out"), "nothing is out yet");
    }

    #[test]
    fn trains_in_transit_are_counted_not_awaited() {
        let mut link = link();
        link.transit.push(rail_sim::border::TransitTrain {
            train: TrainId(1),
            kind: TrainKind::Transport,
            sent_tick: 0,
            due_tick: 180,
            home: rail_sim::TrackId(1),
            carried: None,
            units: 1,
        });
        let line = edge_line(BorderEdge::East, Some(&link), None);
        assert!(line.contains("1 train out"));
        // Nothing anywhere in the row invites the player to wait.
        assert!(!line.to_lowercase().contains("waiting"));
        assert!(!line.to_lowercase().contains("pending"));
    }

    #[test]
    fn money_reads_as_money() {
        assert_eq!(dollars(150_000), "$1500");
        assert_eq!(dollars(1_234), "$12.34");
    }
}
