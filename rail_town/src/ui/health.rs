//! Network health — the readout the player consults constantly.
//!
//! # Why this is not a notification
//!
//! Brief 05 §7 says an alert must be **actionable**, and that anything else
//! belongs in Town Talk. "Westbrook service low (0)" on the opening frame is
//! neither: a station that has never had a train call at it is not a
//! degradation, it is a station waiting for its first train, and the player
//! already knows because they just built it.
//!
//! Two things follow, and both are implemented here:
//!
//! 1. **Health is a permanent readout, not an event.** The strip under the
//!    status row shows the network's score, the counts that mean something is
//!    wrong right now (blocked trains, unserved demand, stations still waiting),
//!    and the **worst stations first** as small meters. It is the third and last
//!    permanently-visible thing on screen, next to money and time, because it is
//!    the third thing the player needs to know at all times.
//! 2. **A never-served station never alerts.** [`alert_is_actionable`] filters
//!    the service-low alert for any station with zero deliveries, so the alert
//!    board only ever says "this got worse", never "this has not started yet".
//!    A station in that state reads `awaiting service` in the strip instead —
//!    a neutral fact in the place the player is already looking.
//!
//! # Cost
//!
//! The model is rebuilt on a 250 ms timer rather than every frame, and the strip
//! is only repainted when the rendered signature actually changes.

use std::time::Duration;

use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;
use rail_map::tile_to_world;
use rail_sim::{
    Alert, AlertBoard, AlertKey, AlertKind, DemandSpawner, MoneyLedger, StationId,
    StationRegistry, StationService, TileCoord, TileOccupancy, TrainLocation,
};

use crate::inspect::{Selectable, Selection};
use crate::map::CameraFocusRequest;
use crate::palette::{BALLAST_L, BG1, OUTLINE, RAIL_L, WARN};
use crate::ui::format::{abbreviate, money_rate};
use crate::ui::kit::{
    chrome_button_node, control_border, meter_fill, micro_font, spawn_meter, text_accent,
    text_secondary, HEALTH_H, SPACE_1, SPACE_2,
};
use crate::ui::window::{WindowId, WindowManager};

/// How often the model is rebuilt. Fast enough to feel live, slow enough that
/// it never shows up in a frame budget.
const REFRESH: Duration = Duration::from_millis(250);

/// Station meters shown on the always-visible strip. The rest live in the
/// Network window, which is what the `Network` button opens.
const STRIP_METERS: usize = 4;

/// Width of a meter on the strip.
const STRIP_METER_W: f32 = 28.0;

/// Width of a meter in the Network window.
const WINDOW_METER_W: f32 = 64.0;

/// Longest station name the strip will draw before abbreviating.
const STRIP_NAME_CHARS: usize = 9;

/// One station's contribution to network health.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StationHealth {
    pub id: StationId,
    pub name: String,
    pub tile: TileCoord,
    pub score: u8,
    pub waiting: u32,
    /// `false` until a train has actually called here.
    pub served: bool,
}

impl StationHealth {
    /// What the row says in words. Colour never carries the state alone (03 §4).
    pub fn state_label(&self) -> &'static str {
        if !self.served {
            "awaiting service"
        } else if self.score < 34 {
            "poor"
        } else if self.score < 67 {
            "fair"
        } else {
            "good"
        }
    }

    fn readout(&self) -> String {
        if self.served {
            format!("{}", self.score)
        } else {
            "-".into()
        }
    }
}

/// The whole health picture, rebuilt on a timer.
#[derive(Resource, Debug, Default)]
pub struct NetworkHealth {
    /// Worst first; stations still awaiting their first train sort last, because
    /// they are not a problem, they are a to-do.
    pub stations: Vec<StationHealth>,
    /// Mean score across served stations. `None` when nothing has run yet.
    pub score: Option<u32>,
    pub blocked: u32,
    pub parked: u32,
    /// Settlements and industries the network does not reach.
    pub unserved: u32,
    /// Stations that exist but have never been served.
    pub awaiting: u32,
    pub net_rate_cents_per_min: i64,
    /// Everything the strip draws, flattened, so repaints can be skipped.
    signature: String,
}

impl NetworkHealth {
    /// Headline number for the strip.
    pub fn score_label(&self) -> String {
        match self.score {
            Some(score) => format!("{score}"),
            None => "-".into(),
        }
    }

    /// The one-line summary of what is wrong, in words.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.blocked > 0 {
            parts.push(format!("{} blocked", self.blocked));
        }
        if self.parked > 0 {
            parts.push(format!("{} parked", self.parked));
        }
        if self.unserved > 0 {
            parts.push(format!("{} unserved", self.unserved));
        }
        if self.awaiting > 0 {
            parts.push(format!("{} awaiting", self.awaiting));
        }
        if parts.is_empty() {
            if self.stations.is_empty() {
                "no stations yet".into()
            } else {
                "all running".into()
            }
        } else {
            parts.join("  ")
        }
    }

    /// `true` when something here needs the player, as opposed to being merely
    /// unfinished.
    pub fn needs_attention(&self) -> bool {
        self.blocked > 0 || self.parked > 0 || self.score.is_some_and(|s| s < 34)
    }
}

/// Rebuild the model. Throttled — see [`REFRESH`].
#[allow(clippy::too_many_arguments)]
pub fn refresh_network_health(
    stations: Res<StationRegistry>,
    service: Res<StationService>,
    demand: Res<DemandSpawner>,
    occupancy: Res<TileOccupancy>,
    ledger: Res<MoneyLedger>,
    trains: Query<&TrainLocation>,
    mut health: ResMut<NetworkHealth>,
) {
    let mut rows: Vec<StationHealth> = stations
        .iter()
        .map(|station| {
            let score = service.score(station.id);
            StationHealth {
                id: station.id,
                name: station.name.clone(),
                tile: station.tile,
                score: score.score,
                waiting: score.total_waiting(),
                served: score.deliveries > 0,
            }
        })
        .collect();

    // Worst first, but a station that has never been served is not "worst" —
    // it has no reading at all, so it sorts after everything that does.
    rows.sort_by(|a, b| {
        b.served
            .cmp(&a.served)
            .then(a.score.cmp(&b.score))
            .then(b.waiting.cmp(&a.waiting))
            .then(a.id.0.cmp(&b.id.0))
    });

    let served: Vec<u32> = rows
        .iter()
        .filter(|r| r.served)
        .map(|r| r.score as u32)
        .collect();
    let score = if served.is_empty() {
        None
    } else {
        Some(served.iter().sum::<u32>() / served.len() as u32)
    };

    let parked = trains.iter().filter(|loc| loc.parked).count() as u32;

    health.awaiting = rows.iter().filter(|r| !r.served).count() as u32;
    health.stations = rows;
    health.score = score;
    health.blocked = occupancy.blocked_by.len() as u32;
    health.parked = parked;
    health.unserved = demand.open.len() as u32;
    health.net_rate_cents_per_min = ledger.net_rate_cents_per_min();
}

/// `true` when an alert is worth putting in front of the player.
///
/// Brief 05 §7. The one thing filtered here is a service-low alert for a station
/// that has never been served — see the module docs.
pub fn alert_is_actionable(alert: &Alert, service: &StationService) -> bool {
    match alert.key {
        AlertKey::StationService(id) => service.score(id).deliveries > 0,
        _ => true,
    }
}

/// Count of alerts the player should actually see.
pub fn actionable_alert_count(board: &AlertBoard, service: &StationService) -> usize {
    board
        .iter()
        .filter(|a| alert_is_actionable(a, service))
        .count()
}

/// How loudly the alert bell should read.
///
/// An opportunity and a failure are both "alerts", but they are not the same
/// news. A new settlement asking to be connected is the game inviting the player
/// somewhere; a parked train is the railway falling over. Painting eight
/// invitations in `warn` on the opening frame is the same mistake as the old
/// "service low (0)" — it makes an ordinary state look like an emergency.
///
/// Never colour alone (03 §4): the bell always carries its count as well.
pub fn alerts_are_bad_news(board: &AlertBoard, service: &StationService) -> bool {
    board
        .iter()
        .filter(|a| alert_is_actionable(a, service))
        .any(|a| !matches!(a.kind, AlertKind::NewDemand))
}

// ─ The strip ───────────────────────────────────────────────

#[derive(Component)]
pub struct HealthStripRoot;

/// A clickable station meter on the strip or in the window.
#[derive(Component, Debug, Clone, Copy)]
pub struct HealthChip {
    pub station: StationId,
    pub tile: TileCoord,
}

/// The chip that opens the Network window.
#[derive(Component)]
pub struct NetworkChip;

#[derive(Component)]
pub struct NetworkWindowRoot;

#[derive(Component)]
pub struct NetworkWindowBody;

/// Spawn the health row into the top chrome.
///
/// It is the third permanently-visible thing on screen, and the last: 03 §1's
/// rule is that nothing is permanent unless it is permanently relevant, and
/// whether the railway is working qualifies.
pub fn spawn_health_row(parent: &mut ChildSpawnerCommands) {
    parent.spawn((
        HealthStripRoot,
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(HEALTH_H),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(SPACE_2),
            padding: UiRect::axes(Val::Px(SPACE_2), Val::Px(0.0)),
            overflow: Overflow::clip(),
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(BG1),
    ));
}

/// Rebuild the strip when — and only when — its rendered content changes.
pub fn rebuild_health_strip(
    mut commands: Commands,
    mut health: ResMut<NetworkHealth>,
    root_q: Query<Entity, With<HealthStripRoot>>,
    children_q: Query<&Children, With<HealthStripRoot>>,
) {
    let signature = strip_signature(&health);
    if signature == health.signature {
        return;
    }
    health.signature = signature;

    let Ok(root) = root_q.single() else {
        return;
    };
    if let Ok(children) = children_q.get(root) {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    let score_label = health.score_label();
    let summary = health.summary();
    let attention = health.needs_attention();
    let meter_percent = health.score.unwrap_or(0);
    let chips: Vec<StationHealth> = health.stations.iter().take(STRIP_METERS).cloned().collect();

    commands.entity(root).with_children(|strip| {
        // Network summary — clicking it opens the full list.
        let (node, bg, border) = chrome_button_node(SPACE_1, 0.0);
        strip
            .spawn((
                Button,
                NetworkChip,
                Node {
                    column_gap: Val::Px(SPACE_1),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    ..node
                },
                bg,
                border,
            ))
            .with_children(|chip| {
                chip.spawn((Text::new("NET"), micro_font(), text_secondary()));
                chip.spawn((Text::new(score_label), micro_font(), text_accent()));
                spawn_meter(
                    chip,
                    STRIP_METER_W,
                    meter_percent,
                    meter_fill(meter_percent),
                );
            });

        strip.spawn((
            Text::new(summary),
            micro_font(),
            TextColor(if attention { WARN } else { BALLAST_L }),
        ));

        // Worst first. The net rate is deliberately *not* repeated here — it is
        // already one row up in the status strip, and the same number twice in
        // adjacent rows reads as two different numbers at a glance.
        for row in &chips {
            spawn_station_chip(strip, row, STRIP_METER_W, STRIP_NAME_CHARS);
        }
    });
}

fn spawn_station_chip(
    parent: &mut ChildSpawnerCommands,
    row: &StationHealth,
    meter_w: f32,
    name_chars: usize,
) {
    let (node, bg, border) = chrome_button_node(SPACE_1, 0.0);
    parent
        .spawn((
            Button,
            HealthChip {
                station: row.id,
                tile: row.tile,
            },
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
        .with_children(|chip| {
            chip.spawn((
                Text::new(abbreviate(&row.name, name_chars)),
                micro_font(),
                TextColor(RAIL_L),
            ));
            chip.spawn((Text::new(row.readout()), micro_font(), text_secondary()));
            if row.served {
                spawn_meter(
                    chip,
                    meter_w,
                    row.score as u32,
                    meter_fill(row.score as u32),
                );
            } else {
                chip.spawn((
                    Text::new(row.state_label()),
                    micro_font(),
                    text_secondary(),
                ));
            }
        });
}

fn strip_signature(health: &NetworkHealth) -> String {
    let mut out = format!("{}|{}", health.score_label(), health.summary());
    for row in health.stations.iter().take(STRIP_METERS) {
        out.push('#');
        out.push_str(&row.name);
        out.push(':');
        out.push_str(&row.readout());
        out.push(':');
        out.push_str(row.state_label());
    }
    out
}

pub fn health_chip_hover(
    mut chips: Query<
        (&Interaction, &mut BorderColor),
        (Changed<Interaction>, Or<(With<HealthChip>, With<NetworkChip>)>),
    >,
) {
    for (interaction, mut border) in &mut chips {
        let hovered = matches!(interaction, Interaction::Hovered | Interaction::Pressed);
        *border = control_border(false, hovered);
    }
}

/// Clicking a station meter selects it and flies there.
pub fn health_chip_clicks(
    interactions: Query<(&Interaction, &HealthChip), (Changed<Interaction>, With<Button>)>,
    mut selection: ResMut<Selection>,
    mut focus: ResMut<CameraFocusRequest>,
) {
    for (interaction, chip) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        selection.set(Selectable::Station(chip.station));
        let (x, y) = tile_to_world(chip.tile);
        focus.0 = Some(Vec2::new(x, y));
    }
}

pub fn network_chip_clicks(
    interactions: Query<&Interaction, (Changed<Interaction>, With<NetworkChip>)>,
    mut manager: ResMut<WindowManager>,
) {
    for interaction in &interactions {
        if *interaction == Interaction::Pressed {
            manager.toggle(WindowId::Network);
        }
    }
}

// ─ The Network window ──────────────────────────────────────

/// Last painted body, so the window only rebuilds when its list changes.
#[derive(Resource, Debug, Default)]
pub struct NetworkWindowCache {
    signature: String,
}

pub fn setup_network_window(mut commands: Commands) {
    commands.init_resource::<NetworkWindowCache>();
    commands
        .spawn((
            NetworkWindowRoot,
            super::window::window_root(WindowId::Network, 236.0),
        ))
        .with_children(|panel| {
            panel.spawn((
                NetworkWindowBody,
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(SPACE_1),
                    padding: UiRect::all(Val::Px(SPACE_1)),
                    ..default()
                },
            ));
        });
}

pub fn rebuild_network_window(
    mut commands: Commands,
    manager: Res<WindowManager>,
    health: Res<NetworkHealth>,
    mut cache: ResMut<NetworkWindowCache>,
    body_q: Query<Entity, With<NetworkWindowBody>>,
    children_q: Query<&Children, With<NetworkWindowBody>>,
) {
    if !manager.is_open(WindowId::Network) {
        return;
    }
    let signature = window_signature(&health);
    if signature == cache.signature {
        return;
    }
    cache.signature = signature;

    let Ok(body) = body_q.single() else {
        return;
    };
    if let Ok(children) = children_q.get(body) {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    let summary = format!(
        "Network {}  -  {}",
        health.score_label(),
        health.summary()
    );
    let rate = format!("Net {}", money_rate(health.net_rate_cents_per_min));
    let rows: Vec<StationHealth> = health.stations.clone();

    commands.entity(body).with_children(|panel| {
        panel.spawn((Text::new(summary), micro_font(), text_accent()));
        panel.spawn((Text::new(rate), micro_font(), text_secondary()));
        super::kit::spawn_rule(panel);
        if rows.is_empty() {
            panel.spawn((
                Text::new("No stations yet. Place one to start a network."),
                micro_font(),
                text_secondary(),
            ));
            return;
        }
        for row in &rows {
            panel
                .spawn(Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(1.0),
                    ..default()
                })
                .with_children(|group| {
                    spawn_station_chip(group, row, WINDOW_METER_W, 18);
                    group.spawn((
                        Text::new(format!(
                            "{}  -  {} waiting",
                            row.state_label(),
                            row.waiting
                        )),
                        micro_font(),
                        text_secondary(),
                    ));
                });
        }
    });
}

fn window_signature(health: &NetworkHealth) -> String {
    let mut out = format!(
        "{}|{}|{}",
        health.score_label(),
        health.summary(),
        money_rate(health.net_rate_cents_per_min)
    );
    for row in &health.stations {
        out.push('#');
        out.push_str(&row.name);
        out.push(':');
        out.push_str(&row.readout());
        out.push(':');
        out.push_str(&row.waiting.to_string());
        out.push(':');
        out.push_str(row.state_label());
    }
    out
}

/// Run condition for the throttled model rebuild.
pub fn health_refresh_due() -> impl FnMut(Res<Time>) -> bool + Clone {
    on_timer(REFRESH)
}

// Keep the palette import honest for the audit.
#[allow(dead_code)]
fn _palette_parity() -> [Color; 2] {
    [OUTLINE, RAIL_L]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::{AlertFocus, AlertKind};

    fn station(id: u64, name: &str, score: u8, served: bool) -> StationHealth {
        StationHealth {
            id: StationId(id),
            name: name.into(),
            tile: TileCoord { x: 0, y: 0 },
            score,
            waiting: 0,
            served,
        }
    }

    #[test]
    fn a_brand_new_station_reads_as_waiting_not_as_failing() {
        // The playtest note: "Westbrook service low (0)" on the opening frame.
        let fresh = station(1, "Westbrook", 0, false);
        assert_eq!(fresh.state_label(), "awaiting service");
        assert_eq!(fresh.readout(), "-", "a station with no arrivals has no score");
    }

    #[test]
    fn a_never_served_station_does_not_raise_an_alert() {
        let mut service = StationService::default();
        let id = StationId(1);
        let alert = Alert {
            id: 1,
            kind: AlertKind::StationServiceLow,
            message: "Westbrook service low (0)".into(),
            focus: AlertFocus::Station(id),
            key: AlertKey::StationService(id),
        };
        assert!(
            !alert_is_actionable(&alert, &service),
            "an unserved station is not a degradation"
        );
        // Once a train has actually called there, a falling score is real news.
        service.record_arrival(id);
        assert!(alert_is_actionable(&alert, &service));
    }

    #[test]
    fn other_alerts_are_never_filtered() {
        let service = StationService::default();
        let alert = Alert {
            id: 2,
            kind: AlertKind::CashLow,
            message: "Cash low".into(),
            focus: AlertFocus::None,
            key: AlertKey::CashLow,
        };
        assert!(alert_is_actionable(&alert, &service));
    }

    #[test]
    fn the_worst_served_station_leads_and_unserved_ones_trail() {
        let mut health = NetworkHealth::default();
        health.stations = vec![
            station(1, "Good", 90, true),
            station(2, "New", 0, false),
            station(3, "Bad", 12, true),
        ];
        health.stations.sort_by(|a, b| {
            b.served
                .cmp(&a.served)
                .then(a.score.cmp(&b.score))
                .then(b.waiting.cmp(&a.waiting))
                .then(a.id.0.cmp(&b.id.0))
        });
        let names: Vec<&str> = health.stations.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Bad", "Good", "New"]);
    }

    #[test]
    fn an_empty_network_says_so_rather_than_scoring_zero() {
        let health = NetworkHealth::default();
        assert_eq!(health.score_label(), "-");
        assert_eq!(health.summary(), "no stations yet");
        assert!(!health.needs_attention());
    }

    #[test]
    fn the_summary_names_every_problem_in_words() {
        let mut health = NetworkHealth::default();
        health.stations = vec![station(1, "A", 80, true)];
        health.blocked = 2;
        health.unserved = 1;
        let summary = health.summary();
        assert!(summary.contains("2 blocked"), "{summary}");
        assert!(summary.contains("1 unserved"), "{summary}");
        assert!(health.needs_attention());

        health.blocked = 0;
        health.unserved = 0;
        assert_eq!(health.summary(), "all running");
    }

    #[test]
    fn awaiting_stations_are_listed_but_do_not_count_as_trouble() {
        let mut health = NetworkHealth::default();
        health.stations = vec![station(1, "New", 0, false)];
        health.awaiting = 1;
        assert!(health.summary().contains("1 awaiting"));
        assert!(
            !health.needs_attention(),
            "an unfinished network is not a failing one"
        );
    }

    #[test]
    fn the_strip_signature_moves_with_the_readings() {
        let mut health = NetworkHealth::default();
        health.stations = vec![station(1, "A", 40, true)];
        health.score = Some(40);
        let before = strip_signature(&health);
        health.stations[0].score = 41;
        assert_ne!(strip_signature(&health), before);
    }
}
