//! Right-side inspector panel (280px) — identity, type, headline, body.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use rail_sim::{
    commands::TrainKind, IndustryRegistry, LineRegistry, Mood, Peep, StationRegistry, StationService,
    TileOccupancy, TrackNetwork, Train, TrainCargo, TrainLocation, WaitingAtStation,
};

use crate::palette::{BALLAST_D, BG1, OK, OUTLINE};
use crate::ui::kit::{
    body_font, display_font, micro_font, panel_node, text_accent, text_primary, text_secondary,
    text_warn, WorldClickBlocker, FONT_BODY, SPACE_2, SPACE_3,
};

use super::cause::{peep_mood_line, station_cause_line, StationCauseInput};
use super::pick::Selectable;
use super::selection::{Selection, ServiceScoreHistory, LONG_WAIT_MINUTES};

pub const INSPECTOR_W: f32 = 280.0;

#[derive(Component)]
pub struct InspectorRoot;

#[derive(Component)]
pub struct InspectorCloseButton;

#[derive(Component)]
struct InspectorNameText;

#[derive(Component)]
struct InspectorTypeText;

#[derive(Component)]
struct InspectorHeadlineText;

#[derive(Component)]
struct InspectorTrendText;

#[derive(Component)]
struct InspectorCauseText;

#[derive(Component)]
struct InspectorBodyText;

#[derive(Resource, Debug, Default)]
pub(crate) struct InspectorCache {
    fingerprint: String,
}

/// Every row of the panel is a separate `&mut Text` query, so each one must
/// exclude *all* of the others.
///
/// The markers are mutually exclusive by construction — a row carries exactly
/// one — but the borrow checker cannot know that, and a partial `Without` set
/// compiles fine and then panics on the first frame the system runs. Spelling
/// the exclusions out in full is what keeps that from happening.
macro_rules! inspector_row {
    ($lw:lifetime, $ls:lifetime, $marker:ty $(, $other:ty)* $(,)?) => {
        Query<$lw, $ls, &'static mut Text, (With<$marker> $(, Without<$other>)*)>
    };
}

#[derive(SystemParam)]
pub(crate) struct InspectorUi<'w, 's> {
    root: Query<'w, 's, &'static mut Node, With<InspectorRoot>>,
    name: inspector_row!('w, 's, InspectorNameText, InspectorTypeText, InspectorHeadlineText,
        InspectorTrendText, InspectorCauseText, InspectorBodyText),
    type_line: inspector_row!('w, 's, InspectorTypeText, InspectorNameText, InspectorHeadlineText,
        InspectorTrendText, InspectorCauseText, InspectorBodyText),
    headline: inspector_row!('w, 's, InspectorHeadlineText, InspectorNameText, InspectorTypeText,
        InspectorTrendText, InspectorCauseText, InspectorBodyText),
    trend: inspector_row!('w, 's, InspectorTrendText, InspectorNameText, InspectorTypeText,
        InspectorHeadlineText, InspectorCauseText, InspectorBodyText),
    cause: inspector_row!('w, 's, InspectorCauseText, InspectorNameText, InspectorTypeText,
        InspectorHeadlineText, InspectorTrendText, InspectorBodyText),
    body: inspector_row!('w, 's, InspectorBodyText, InspectorNameText, InspectorTypeText,
        InspectorHeadlineText, InspectorTrendText, InspectorCauseText),
    cause_color: Query<'w, 's, &'static mut TextColor, With<InspectorCauseText>>,
}

pub fn setup_inspector_panel(mut commands: Commands) {
    commands.insert_resource(InspectorCache::default());

    let (node, bg, border) = panel_node(Node {
        position_type: PositionType::Absolute,
        top: Val::Px(28.0),
        right: Val::Px(SPACE_3),
        width: Val::Px(INSPECTOR_W),
        max_height: Val::Percent(85.0),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(SPACE_2),
        padding: UiRect::all(Val::Px(SPACE_3)),
        display: Display::None,
        overflow: Overflow::scroll_y(),
        ..default()
    });

    commands
        .spawn((
            InspectorRoot,
            WorldClickBlocker,
            Interaction::default(),
            node,
            bg,
            border,
            ZIndex(20),
        ))
        .with_children(|root| {
            // Identity row
            root.spawn(Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    InspectorNameText,
                    Text::new(""),
                    display_font(),
                    text_primary(),
                ));
                row.spawn((
                    Button,
                    InspectorCloseButton,
                    Node {
                        width: Val::Px(24.0),
                        height: Val::Px(24.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::ZERO,
                        ..default()
                    },
                    BackgroundColor(BG1),
                    BorderColor::all(OUTLINE),
                ))
                .with_children(|btn| {
                    btn.spawn((Text::new("x"), body_font(), text_secondary()));
                });
            });

            root.spawn((
                InspectorTypeText,
                Text::new(""),
                micro_font(),
                text_secondary(),
            ));

            // Divider
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(1.0),
                    ..default()
                },
                BackgroundColor(BALLAST_D),
            ));

            root.spawn((
                InspectorHeadlineText,
                Text::new(""),
                body_font(),
                text_accent(),
            ));
            root.spawn((
                InspectorTrendText,
                Text::new(""),
                micro_font(),
                text_secondary(),
            ));
            root.spawn((
                InspectorCauseText,
                Text::new(""),
                body_font(),
                text_primary(),
            ));

            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(1.0),
                    ..default()
                },
                BackgroundColor(BALLAST_D),
            ));

            root.spawn((
                InspectorBodyText,
                Text::new(""),
                TextFont::from_font_size(FONT_BODY),
                text_primary(),
            ));
        });
}

pub fn inspector_close_clicks(
    interactions: Query<&Interaction, (Changed<Interaction>, With<InspectorCloseButton>)>,
    mut selection: ResMut<Selection>,
) {
    for interaction in &interactions {
        if *interaction == Interaction::Pressed {
            selection.clear();
        }
    }
}

pub fn update_inspector_panel(
    selection: Res<Selection>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    service: Res<StationService>,
    history: Res<ServiceScoreHistory>,
    network: Res<TrackNetwork>,
    occupancy: Res<TileOccupancy>,
    lines: Res<LineRegistry>,
    peeps: Query<(&Peep, &WaitingAtStation)>,
    trains: Query<(&Train, &TrainLocation, &TrainCargo)>,
    mut cache: ResMut<InspectorCache>,
    mut ui: InspectorUi,
) {
    let Ok(mut root) = ui.root.single_mut() else {
        return;
    };

    let Some(sel) = selection.0 else {
        root.display = Display::None;
        cache.fingerprint.clear();
        return;
    };

    root.display = Display::Flex;

    let view = build_view(
        sel,
        &stations,
        &industries,
        &service,
        &history,
        &network,
        &occupancy,
        &lines,
        &peeps,
        &trains,
    );

    if view.fingerprint == cache.fingerprint {
        return;
    }
    cache.fingerprint = view.fingerprint;

    if let Ok(mut t) = ui.name.single_mut() {
        *t = Text::new(view.name);
    }
    if let Ok(mut t) = ui.type_line.single_mut() {
        *t = Text::new(view.type_line);
    }
    if let Ok(mut t) = ui.headline.single_mut() {
        *t = Text::new(view.headline);
    }
    if let Ok(mut t) = ui.trend.single_mut() {
        *t = Text::new(view.trend);
    }
    if let Ok(mut t) = ui.cause.single_mut() {
        *t = Text::new(view.cause);
    }
    if let Ok(mut t) = ui.body.single_mut() {
        *t = Text::new(view.body);
    }
    if let Ok(mut c) = ui.cause_color.single_mut() {
        *c = match view.cause_tone {
            CauseTone::Warn => text_warn(),
            CauseTone::Ok => TextColor(OK),
            CauseTone::Neutral => text_primary(),
            CauseTone::Accent => text_accent(),
        };
    }
}

#[derive(Clone, Copy)]
enum CauseTone {
    Warn,
    Ok,
    Neutral,
    Accent,
}

struct InspectorView {
    fingerprint: String,
    name: String,
    type_line: String,
    headline: String,
    trend: String,
    cause: String,
    cause_tone: CauseTone,
    body: String,
}

fn build_view(
    sel: Selectable,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    service: &StationService,
    history: &ServiceScoreHistory,
    network: &TrackNetwork,
    occupancy: &TileOccupancy,
    lines: &LineRegistry,
    peeps: &Query<(&Peep, &WaitingAtStation)>,
    trains: &Query<(&Train, &TrainLocation, &TrainCargo)>,
) -> InspectorView {
    match sel {
        Selectable::Station(id) => {
            let name = stations
                .get(id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| format!("Station {}", id.0));
            let score = service.score(id);
            let delta = history.delta(id);
            let spark = history.sparkline(id);
            let long_wait_secs = LONG_WAIT_MINUTES * 60;
            let long_wait_count = peeps
                .iter()
                .filter(|(p, w)| {
                    let _ = p;
                    w.station == id && w.wait_secs >= long_wait_secs
                })
                .count() as u32;
            let cause = station_cause_line(StationCauseInput {
                score: score.score,
                score_delta: delta,
                waiting_passengers: score.waiting_passengers,
                long_wait_count,
                long_wait_minutes: LONG_WAIT_MINUTES,
            });
            let tone = if cause.starts_with("Falling") {
                CauseTone::Warn
            } else if cause.starts_with("Rising") {
                CauseTone::Ok
            } else {
                CauseTone::Neutral
            };
            let body = format!(
                "Waiting jobs: {}\nDeliveries: {}\nTile: {}, {}",
                score.waiting_passengers,
                score.deliveries,
                stations.get(id).map(|s| s.tile.x).unwrap_or(0),
                stations.get(id).map(|s| s.tile.y).unwrap_or(0),
            );
            InspectorView {
                fingerprint: format!(
                    "st:{}:{}:{}:{}:{}",
                    id.0, score.score, delta, score.waiting_passengers, cause
                ),
                name,
                type_line: "Station - Tier 1".into(),
                headline: format!("Service      {}/100", score.score),
                trend: spark,
                cause,
                cause_tone: tone,
                body,
            }
        }
        Selectable::Train(id) => {
            let Some((train, loc, cargo)) = trains.iter().find(|(t, _, _)| t.id == id) else {
                return missing("Train", format!("Train {}", id.0));
            };
            let kind = match train.kind {
                TrainKind::Transit => "Transit",
                TrainKind::Transport => "Transport",
            };
            let status = if loc.parked {
                "Parked"
            } else if let Some(blocker) = occupancy.blocked_by.get(&train.id) {
                let _ = blocker;
                "Blocked"
            } else if loc.dwell_remaining > 0 {
                "Dwelling"
            } else if loc.at_destination() {
                "Idle"
            } else {
                "Running"
            };
            let job = match cargo {
                TrainCargo::Empty => "Empty - seeking work".into(),
                TrainCargo::Passengers { from, to } => {
                    let a = stations.get(*from).map(|s| s.name.as_str()).unwrap_or("?");
                    let b = stations.get(*to).map(|s| s.name.as_str()).unwrap_or("?");
                    format!("Passengers {a} -> {b}")
                }
                TrainCargo::Goods { kind, from, to } => {
                    let a = industries.get(*from).map(|i| i.name.as_str()).unwrap_or("?");
                    let b = industries.get(*to).map(|i| i.name.as_str()).unwrap_or("?");
                    format!("{} {a} -> {b}", kind.label())
                }
            };
            let blocker_line = occupancy
                .blocked_by
                .get(&train.id)
                .map(|b| format!("Blocked by Train {}", b.0))
                .unwrap_or_else(|| job.clone());
            let tone = match status {
                "Blocked" | "Parked" => CauseTone::Warn,
                "Running" => CauseTone::Ok,
                _ => CauseTone::Neutral,
            };
            let line_note = lines
                .line_for_train(id)
                .map(|l| format!("Line: {}", l.name))
                .unwrap_or_else(|| "Line: (free-roam)".into());
            InspectorView {
                fingerprint: format!(
                    "tr:{}:{}:{}:{}:{:?}",
                    id.0,
                    status,
                    job,
                    loc.path_index,
                    occupancy.blocked_by.get(&train.id)
                ),
                name: format!("Train {}", id.0),
                type_line: format!("Train - {kind}"),
                headline: format!("Status      {status}"),
                trend: String::new(),
                cause: blocker_line.clone(),
                cause_tone: tone,
                body: format!(
                    "{line_note}\nCargo / job\n{job}\n\nPath step {}/{}\n{}",
                    loc.path_index + 1,
                    loc.path.len().max(1),
                    if status == "Blocked" {
                        blocker_line
                    } else {
                        String::new()
                    }
                ),
            }
        }
        Selectable::Peep(id) => {
            let Some((peep, waiting)) = peeps.iter().find(|(p, _)| p.id == id) else {
                return missing("Peep", format!("Peep {}", id.0));
            };
            let station_name = stations
                .get(waiting.station)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| format!("Station {}", waiting.station.0));
            let mood_label = match peep.mood {
                Mood::Content => "Content",
                Mood::Uneasy => "Uneasy",
                Mood::Frustrated => "Frustrated",
            };
            let cause = peep_mood_line(peep.mood, waiting.wait_secs, &station_name);
            let tone = match peep.mood {
                Mood::Frustrated => CauseTone::Warn,
                Mood::Uneasy => CauseTone::Accent,
                Mood::Content => CauseTone::Ok,
            };
            InspectorView {
                fingerprint: format!(
                    "pp:{}:{}:{}:{}",
                    id.0, mood_label, waiting.wait_secs, station_name
                ),
                name: peep.name.clone(),
                type_line: "Peep - Resident".into(),
                headline: format!("Mood      {mood_label}"),
                trend: String::new(),
                cause: cause.clone(),
                cause_tone: tone,
                body: format!(
                    "At station: {station_name}\nWait: {} min\nHome tile: {}, {}",
                    waiting.wait_secs / 60,
                    peep.home.x,
                    peep.home.y
                ),
            }
        }
        Selectable::Industry(id) => {
            let Some(ind) = industries.get(id) else {
                return missing("Industry", format!("Industry {}", id.0));
            };
            let produces = ind
                .produces
                .map(|g| g.label())
                .unwrap_or("-");
            let consumes = ind
                .consumes
                .map(|g| g.label())
                .unwrap_or("-");
            InspectorView {
                fingerprint: format!("in:{}:{}:{}", id.0, produces, consumes),
                name: ind.name.clone(),
                type_line: "Industry".into(),
                headline: format!("Produces      {produces}"),
                trend: String::new(),
                cause: format!("Consumes {consumes}"),
                cause_tone: CauseTone::Neutral,
                body: format!(
                    "Produces: {produces}\nConsumes: {consumes}\nTile: {}, {}",
                    ind.tile.x, ind.tile.y
                ),
            }
        }
        Selectable::Track(id) => {
            let Some(piece) = network.piece(id) else {
                return missing("Track", format!("Track {}", id.0));
            };
            let kind = if piece.is_bridge() { "Bridge" } else { "Ground" };
            let cost = format_cents(piece.paid_cents);
            InspectorView {
                fingerprint: format!(
                    "tk:{}:{}:{}:{}",
                    id.0, piece.paid_cents, piece.max_grade, piece.curve
                ),
                name: format!("Track {}", id.0),
                type_line: format!("Track - {kind}"),
                headline: format!("Cost paid      {cost}"),
                trend: String::new(),
                cause: format!("Grade {} - Curve {}", piece.max_grade, piece.curve),
                cause_tone: CauseTone::Neutral,
                body: format!(
                    "Paid: {cost}\nGrade: {}\nCurve: {}\nTile: {}, {} - layer {}",
                    piece.max_grade,
                    piece.curve,
                    piece.tile.x,
                    piece.tile.y,
                    piece.layer
                ),
            }
        }
    }
}

fn missing(kind: &str, name: String) -> InspectorView {
    InspectorView {
        fingerprint: format!("missing:{kind}:{}", name),
        name,
        type_line: kind.into(),
        headline: "-".into(),
        trend: String::new(),
        cause: "No longer in the world.".into(),
        cause_tone: CauseTone::Warn,
        body: String::new(),
    }
}

fn format_cents(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    format!("{sign}${}.{:02}", abs / 100, abs % 100)
}
