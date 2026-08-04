//! The Inspector — identity, type, headline, body (brief 05 §3).
//!
//! A window, not a fixed panel: it carries [`UiWindow`] and everything about
//! where it sits, how it stacks, its title bar and its close box belongs to
//! `ui::window`. It is the one window nothing on the menu row opens — selecting
//! something in the world is what opens it, and `ui::adapters` keeps the two in
//! step in both directions, so `Esc` and the close box clear the selection
//! rather than leaving a panel describing something no longer selected.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use rail_sim::stations::validate_station_site;
use rail_sim::{
    buy_cost, commands::TrainKind, push_station_command, CommandBuffer, HouseholdRegistry,
    IndustryRegistry, Journey, JourneyMemory, LineRegistry, Money, Mood, Peep, Routine,
    StationCommand, StationId, StationRegistry, StationService, StationTier, TileOccupancy,
    TrackNetwork, Train, TrainCargo, TrainLocation, UpgradeStation, WaitingAtStation,
};

use crate::palette::{BALLAST_D, OK};
use crate::stations::{request_demolish, station_reason};
use crate::ui::format::money_whole;
use crate::ui::kit::{
    body_font, chrome_button_node, control_border, display_font, micro_font, panel_node,
    text_accent, text_disabled, text_primary, text_secondary, text_warn, WorldClickBlocker,
    FONT_BODY, SPACE_1,
};
use crate::ui::{ConfirmDialog, UiWindow, WindowId};

use super::cause::{peep_mood_line, station_cause_line, StationCauseInput};
use super::pick::Selectable;
use super::selection::{Selection, ServiceScoreHistory, LONG_WAIT_MINUTES};

/// Brief 05 §3: 280 texels, right side, clear of the centre of the world.
/// `ui::window` reads the same number for the Inspector's default corner.
pub const INSPECTOR_W: f32 = 280.0;

#[derive(Component)]
pub struct InspectorRoot;

/// Holds the rows, so the window's title bar stays put while they scroll.
#[derive(Component)]
struct InspectorBody;

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

/// The row of verbs at the foot of the card. Only a station fills it in.
#[derive(Component)]
pub(crate) struct InspectorActionRow;

#[derive(Component)]
pub(crate) struct UpgradeStationButton;

#[derive(Component)]
pub(crate) struct UpgradeStationLabel;

#[derive(Component)]
pub(crate) struct DemolishStationButton;

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
    cause_jump: Query<'w, 's, &'static mut CauseJump>,
}

/// What the Inspector can offer to do to the selected stop.
///
/// 03 §7 keeps the menu row for **modes** — verbs that arm the pointer. Upgrade
/// arms nothing: it applies in place, to one named stop, at a price that only
/// means anything beside the tier that stop already is. So it lives here, on the
/// card that describes that stop, and `U` stays as its accelerator.
///
/// The offer is computed, not assumed: 04 §3's last line asks a tool to signal
/// an impossible action *before* the click, so a platform that will not fit says
/// so where the button would be rather than refusing after the fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpgradeOffer {
    /// The rung being offered, when there is one at all.
    pub to: Option<StationTier>,
    /// What the button says.
    pub label: String,
    /// What the upgrade buys, or why it cannot be had.
    pub note: String,
    /// False when the label is a reason rather than an offer.
    pub ready: bool,
}

/// Price, fit and funds for the next rung above `station`.
pub(crate) fn upgrade_offer(
    station: StationId,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    network: &TrackNetwork,
    money: &Money,
) -> UpgradeOffer {
    let blocked = |note: String| UpgradeOffer {
        to: None,
        label: "Upgrade".into(),
        note,
        ready: false,
    };
    let Some(current) = stations.get(station) else {
        return blocked("No station there".into());
    };
    let Some(to) = current.tier.next_upgrade() else {
        // The terminus and the goods platform are siblings of the ladder, not
        // rungs on it — the card says so rather than offering a dead button.
        return blocked(match current.tier {
            StationTier::Interchange => "Interchange is the top of the ladder".into(),
            other => format!("{} is built, not upgraded into", other.label()),
        });
    };
    let price = current.tier.retier_cents(to);
    if let Err(err) = validate_station_site(
        stations,
        industries,
        network,
        current.tile,
        current.layer,
        to,
        Some(station),
    ) {
        return blocked(station_reason(err, price, money.cents()));
    }
    if price > money.cents() {
        return blocked(station_reason(
            rail_sim::StationPlacementError::InsufficientFunds,
            price,
            money.cents(),
        ));
    }
    UpgradeOffer {
        label: format!("Upgrade to {}   {}", to.label(), money_whole(price)),
        // What the money buys, in the two numbers the tier table moves.
        note: format!(
            "{} platforms, reach {}, holds {}",
            to.platforms(),
            to.catchment(),
            to.capacity()
        ),
        to: Some(to),
        ready: true,
    }
}

/// Walk one hop up a blocked train's queue: clicking the cause row selects
/// whatever it names (brief 07 §4.2 — "offers to select that"). Selection
/// drives the panel, so the next click walks the next hop.
pub(crate) fn cause_jump_clicks(
    rows: Query<(&Interaction, &CauseJump), Changed<Interaction>>,
    mut selection: ResMut<Selection>,
) {
    for (interaction, jump) in &rows {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if let Some(target) = jump.0 {
            selection.set(target);
        }
    }
}

/// The Inspector's window root.
///
/// Position, stacking, the title bar and the close box are all the window
/// manager's (`ui::window`), so nothing about placement lives here — the only
/// job left is to be the right width and to start hidden, because a window that
/// showed itself before the player selected anything would be an empty panel on
/// screen from the first frame.
fn inspector_window() -> impl Bundle {
    let (node, bg, border) = panel_node(Node {
        position_type: PositionType::Absolute,
        width: Val::Px(INSPECTOR_W),
        max_height: Val::Percent(70.0),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(SPACE_1),
        padding: UiRect::all(Val::Px(SPACE_1)),
        display: Display::None,
        ..default()
    });
    (
        InspectorRoot,
        UiWindow::new(WindowId::Inspector),
        WorldClickBlocker,
        Interaction::default(),
        node,
        bg,
        border,
    )
}

/// A 1-texel rule between sections.
fn spawn_divider(parent: &mut ChildSpawnerCommands) {
    parent.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(1.0),
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(BALLAST_D),
    ));
}

pub fn setup_inspector_panel(mut commands: Commands) {
    commands.insert_resource(InspectorCache::default());

    commands
        .spawn(inspector_window())
        .with_children(|root| {
            // The rows live one level down so `dress_new_windows` can put its
            // title bar above them, and so overflow scrolls the content rather
            // than the bar the player drags the window by.
            root.spawn((
                InspectorBody,
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(SPACE_1),
                    padding: UiRect::all(Val::Px(SPACE_1)),
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
            ))
            .with_children(|body| {
                // Identity. The title bar says "Inspector"; this says which one.
                body.spawn((
                    InspectorNameText,
                    Text::new(""),
                    display_font(),
                    text_primary(),
                ));
                body.spawn((
                    InspectorTypeText,
                    Text::new(""),
                    micro_font(),
                    text_secondary(),
                ));

                spawn_divider(body);

                body.spawn((
                    InspectorHeadlineText,
                    Text::new(""),
                    body_font(),
                    text_accent(),
                ));
                body.spawn((
                    InspectorTrendText,
                    Text::new(""),
                    micro_font(),
                    text_secondary(),
                ));
                body.spawn((
                    InspectorCauseText,
                    // The one row that can be a verb: clicking a blocked
                    // train's cause selects its blocker, so the chain in
                    // brief 07 §4.2 is walked a click at a time.
                    CauseJump(None),
                    Interaction::default(),
                    Text::new(""),
                    body_font(),
                    text_primary(),
                ));

                spawn_divider(body);

                body.spawn((
                    InspectorBodyText,
                    Text::new(""),
                    TextFont::from_font_size(FONT_BODY),
                    text_primary(),
                ));

                spawn_station_actions(body);
            });
        });
}

/// The station card's two verbs, spawned once and hidden until a stop is
/// selected. Upgrade carries its own label (the price moves with the tier);
/// Demolish never changes, because what it costs is a question the confirm
/// dialog asks with the consequence attached.
fn spawn_station_actions(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            InspectorActionRow,
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(SPACE_1),
                display: Display::None,
                ..default()
            },
        ))
        .with_children(|row| {
            let (node, bg, border) = chrome_button_node(SPACE_1, 2.0);
            row.spawn((
                Button,
                UpgradeStationButton,
                Node {
                    width: Val::Percent(100.0),
                    ..node
                },
                bg,
                border,
            ))
            .with_children(|b| {
                b.spawn((
                    UpgradeStationLabel,
                    Text::new(""),
                    micro_font(),
                    text_primary(),
                ));
            });

            let (node, bg, border) = chrome_button_node(SPACE_1, 2.0);
            row.spawn((
                Button,
                DemolishStationButton,
                Node {
                    width: Val::Percent(100.0),
                    ..node
                },
                bg,
                border,
            ))
            .with_children(|b| {
                // 03 §8.2: destructive controls are labelled, not painted red.
                b.spawn((
                    Text::new("Demolish - refunds in full"),
                    micro_font(),
                    text_warn(),
                ));
            });
        });
}

/// Everything the Peep card needs, in one read.
///
/// Brief 06 §4.2 gives a peep a routine, a journey and a memory, and 05 §3.3
/// spends all three on this panel. They are `Option` because a peep restored
/// from an old save may arrive without one (`rail_sim::save::snapshot` stores
/// them individually) — a missing component drops its line rather than the card.
type PeepQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static Peep,
        &'static WaitingAtStation,
        Option<&'static Routine>,
        Option<&'static Journey>,
        Option<&'static JourneyMemory>,
    ),
>;

#[allow(clippy::too_many_arguments)]
pub fn update_inspector_panel(
    selection: Res<Selection>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    service: Res<StationService>,
    history: Res<ServiceScoreHistory>,
    network: Res<TrackNetwork>,
    occupancy: Res<TileOccupancy>,
    lines: Res<LineRegistry>,
    households: Res<HouseholdRegistry>,
    peeps: PeepQuery,
    trains: Query<(&Train, &TrainLocation, &TrainCargo)>,
    mut cache: ResMut<InspectorCache>,
    mut ui: InspectorUi,
) {
    // Showing and hiding is the window manager's, driven off `Selection` by
    // `ui::adapters::sync_inspector_window`. Two owners of one `display` would
    // fight every frame, so this system only ever writes the contents.
    let Some(sel) = selection.0 else {
        if !cache.fingerprint.is_empty() {
            cache.fingerprint.clear();
        }
        return;
    };

    let view = build_view(
        sel,
        &stations,
        &industries,
        &service,
        &history,
        &network,
        &occupancy,
        &lines,
        &households,
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
    if let Ok(mut jump) = ui.cause_jump.single_mut() {
        jump.0 = view.cause_jump;
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
    /// What clicking the cause row selects — the blocker behind a blocked
    /// train, and nothing for every other view.
    cause_jump: Option<Selectable>,
    body: String,
}

/// The cause row's click target (see [`InspectorView::cause_jump`]).
#[derive(Component)]
pub(crate) struct CauseJump(pub(crate) Option<Selectable>);

#[allow(clippy::too_many_arguments)]
fn build_view(
    sel: Selectable,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    service: &StationService,
    history: &ServiceScoreHistory,
    network: &TrackNetwork,
    occupancy: &TileOccupancy,
    lines: &LineRegistry,
    households: &HouseholdRegistry,
    peeps: &PeepQuery,
    trains: &Query<(&Train, &TrainLocation, &TrainCargo)>,
) -> InspectorView {
    match sel {
        Selectable::Station(id) => {
            // A stop the player just demolished — from this very card — is gone,
            // and the honest card says so. It used to fall back to "Station 7"
            // with every number at zero, which reads as a real stop nobody has
            // ever used. Every other kind here already answers this way.
            let Some(station) = stations.get(id) else {
                return missing("Station", format!("Station {}", id.0));
            };
            let name = station.name.clone();
            let score = service.score(id);
            let delta = history.delta(id);
            let spark = history.sparkline(id);
            let long_wait_secs = LONG_WAIT_MINUTES * 60;
            let long_wait_count = peeps
                .iter()
                .filter(|(_, w, _, journey, _)| {
                    // Only a peep actually on the platform is waiting; the
                    // component is on everybody (see [`PeepQuery`]).
                    journey.map_or(true, |j| j.stage.is_waiting())
                        && w.station == id
                        && w.wait_secs >= long_wait_secs
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
            // The tier is the fact every other number on this card hangs off,
            // and the card used to hard-code "Tier 1" for all five of them.
            let tier = station.tier;
            let calling = lines.lines_calling_at(id).len();
            let served = match calling {
                0 => "No line calls here".to_string(),
                1 => "1 line calls here".to_string(),
                n => format!("{n} lines call here"),
            };
            let body = format!(
                "{served}\nCatchment: {} tiles\nPlatforms: {}  -  holds {}\nWaiting jobs: {}\n\
                 Deliveries: {}\nTile: {}, {}",
                tier.catchment(),
                tier.platforms(),
                tier.capacity(),
                score.waiting_passengers,
                score.deliveries,
                station.tile.x,
                station.tile.y,
            );
            InspectorView {
                fingerprint: format!(
                    "st:{}:{}:{}:{}:{}:{}:{calling}",
                    id.0,
                    score.score,
                    delta,
                    score.waiting_passengers,
                    cause,
                    tier.label(),
                ),
                name,
                type_line: format!("Station - {}", tier.label()),
                headline: format!("Service      {}/100", score.score),
                trend: spark,
                cause,
                cause_tone: tone,
                cause_jump: None,
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
            // Name the immediate blocker and, when the queue is more than one
            // deep, where it leads — brief 07 §4.2 wants the chain traceable
            // in seconds, and each click on the cause row walks one hop.
            let blocker = occupancy.blocked_by.get(&train.id).copied();
            let chain_head = rail_sim::blocked_chain_head(&occupancy, train.id);
            let blocker_line = match (blocker, chain_head) {
                (Some(b), Some(h)) if h != b => {
                    format!("Blocked by Train {} - queue heads at Train {}", b.0, h.0)
                }
                (Some(b), _) => format!("Blocked by Train {} - click to select", b.0),
                (None, _) => job.clone(),
            };
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
                cause_jump: blocker.map(Selectable::Train),
                body: format!(
                    "{line_note}\nCargo / job\n{job}\n\nPath step {}/{}\n{}\nX sells it back \
                     for {}",
                    loc.path_index + 1,
                    loc.path.len().max(1),
                    if status == "Blocked" {
                        blocker_line
                    } else {
                        String::new()
                    },
                    // The verb is only findable if something says it exists, and
                    // this is the panel a player is already looking at when they
                    // want the train gone.
                    money_whole(buy_cost(train.kind))
                ),
            }
        }
        Selectable::Peep(id) => {
            let Some((peep, waiting, routine, journey, memory)) =
                peeps.iter().find(|(p, ..)| p.id == id)
            else {
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
            let home_place = routine
                .and_then(|r| stations.get(r.home_station))
                .map(|s| s.name.as_str())
                .unwrap_or(station_name.as_str());
            InspectorView {
                fingerprint: format!(
                    "pp:{}:{}:{}:{}:{}:{}",
                    id.0,
                    mood_label,
                    waiting.wait_secs,
                    station_name,
                    journey.map(|j| format!("{:?}/{:?}", j.stage, j.leg)).unwrap_or_default(),
                    memory
                        .map(|m| format!("{}/{}", m.lifetime_journeys, m.bad_streak))
                        .unwrap_or_default(),
                ),
                name: peep.name.clone(),
                type_line: "Peep - Resident".into(),
                headline: format!("Mood      {mood_label}"),
                trend: String::new(),
                cause: cause.clone(),
                cause_tone: tone,
                cause_jump: None,
                body: peep_body(
                    peep,
                    routine,
                    journey,
                    memory,
                    households,
                    stations,
                    home_place,
                    service.tick,
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
            // The lot is what a goods platform has to touch (04 §6), so the
            // card states it rather than leaving the player to guess.
            let lot = ind.tier.lot_side();
            InspectorView {
                fingerprint: format!("in:{}:{}:{}", id.0, produces, consumes),
                name: ind.name.clone(),
                type_line: format!("Industry - {}", ind.tier.label()),
                headline: format!("Produces      {produces}"),
                trend: String::new(),
                cause: format!("Consumes {consumes}"),
                cause_tone: CauseTone::Neutral,
                cause_jump: None,
                body: format!(
                    "Produces: {produces}\nConsumes: {consumes}\nLot: {lot} by {lot} tiles\nTile: {}, {}",
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
                cause_jump: None,
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

/// Show the station verbs, and keep the Upgrade label honest.
///
/// A separate system from [`update_inspector_panel`] on purpose: that one owns
/// every `&mut Text` on the card through one mutually-exclusive query set, and a
/// seventh row would have to be threaded through all six exclusion lists to
/// avoid a borrow panic. Painting the verbs here keeps that set closed.
#[allow(clippy::too_many_arguments)]
pub fn paint_station_actions(
    selection: Res<Selection>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    network: Res<TrackNetwork>,
    money: Res<Money>,
    mut row: Query<&mut Node, With<InspectorActionRow>>,
    mut label: Query<(&mut Text, &mut TextColor), With<UpgradeStationLabel>>,
    mut buttons: Query<(&Interaction, &mut BorderColor), With<UpgradeStationButton>>,
) {
    let Ok(mut node) = row.single_mut() else {
        return;
    };
    let station = match selection.0 {
        Some(Selectable::Station(id)) if stations.get(id).is_some() => id,
        _ => {
            if node.display != Display::None {
                node.display = Display::None;
            }
            return;
        }
    };
    if node.display != Display::Flex {
        node.display = Display::Flex;
    }

    let offer = upgrade_offer(station, &stations, &industries, &network, &money);
    // Two lines: what the button does, and what it buys or why it cannot.
    let wanted = format!("{}\n{}", offer.label, offer.note);
    if let Ok((mut text, mut colour)) = label.single_mut() {
        if text.as_str() != wanted {
            *text = Text::new(wanted);
        }
        let tone = if offer.ready {
            text_primary()
        } else {
            // 03 §8.2: a disabled control drops its label rather than shouting.
            text_disabled()
        };
        if colour.0 != tone.0 {
            *colour = tone;
        }
    }
    for (interaction, mut border) in &mut buttons {
        let hovered =
            offer.ready && matches!(interaction, Interaction::Hovered | Interaction::Pressed);
        let want = control_border(false, hovered);
        if border.top != want.top {
            *border = want;
        }
    }
}

/// Answer the station card's verbs.
///
/// Upgrade only fires when the offer is real, so a stop whose platforms will not
/// fit says so on the button instead of pushing a command the sim will refuse.
/// Demolish goes through the same path as a right-click with the station tool,
/// confirm dialog and all, so the two can never disagree about consequences.
#[allow(clippy::too_many_arguments)]
pub fn station_action_clicks(
    upgrades: Query<&Interaction, (Changed<Interaction>, With<UpgradeStationButton>)>,
    demolishes: Query<&Interaction, (Changed<Interaction>, With<DemolishStationButton>)>,
    selection: Res<Selection>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    network: Res<TrackNetwork>,
    lines: Res<LineRegistry>,
    money: Res<Money>,
    mut buffer: ResMut<CommandBuffer>,
    mut confirm: ResMut<ConfirmDialog>,
) {
    let Some(Selectable::Station(station)) = selection.0 else {
        return;
    };
    if upgrades.iter().any(|i| *i == Interaction::Pressed) {
        let offer = upgrade_offer(station, &stations, &industries, &network, &money);
        if let Some(to) = offer.to {
            push_station_command(
                &mut buffer,
                StationCommand::Upgrade(UpgradeStation { station, to }),
            );
        }
    }
    if demolishes.iter().any(|i| *i == Interaction::Pressed) {
        request_demolish(station, &lines, &mut buffer, &mut confirm);
    }
}

/// How many remembered legs the Peep card lists.
const RECENT_TRIPS_SHOWN: usize = 4;

/// The Peep card's body — a person, not a stat block (brief 05 §3.3).
///
/// Every sentence here already existed in `rail_sim` and had no caller:
/// [`Journey::describe`], [`Peep::tenure_line`], [`Routine::describe`],
/// [`JourneyMemory::verdict_line`] and [`JourneyMemory::tolerance_line`]. The
/// panel names the stations and puts the lines in order; the wording belongs to
/// the slice that owns the state, so what the card says can never drift from
/// what caused it. The old body — station, wait, home tile — was three numbers
/// about somebody the design promises is *knowable*.
#[allow(clippy::too_many_arguments)]
fn peep_body(
    peep: &Peep,
    routine: Option<&Routine>,
    journey: Option<&Journey>,
    memory: Option<&JourneyMemory>,
    households: &HouseholdRegistry,
    stations: &StationRegistry,
    home_place: &str,
    tick: u64,
) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(6);

    // Where they are going right now.
    if let Some(journey) = journey {
        let from = station_name_of(stations, journey.from_station);
        let to = station_name_of(stations, journey.to_station);
        lines.push(journey.describe(&from, &to));
    }

    // The line that makes it land.
    lines.push(peep.tenure_line(home_place, tick));

    // Role, and the time they habitually travel.
    if let Some(routine) = routine {
        lines.push(routine.describe(home_place));
    }

    // Who they live with. The id is the authority; the home tile is the
    // fallback, because a peep whose household went missing still lives
    // somewhere and the family name is the point.
    if let Some(household) = households
        .get(peep.household)
        .or_else(|| households.iter().find(|h| h.home == peep.home))
    {
        let size = household.members.len().max(1);
        lines.push(format!("{} - a household of {size}.", household.plural()));
    }

    // What they remember, and what it has bought them.
    if let Some(memory) = memory {
        lines.push(memory.verdict_line(RECENT_TRIPS_SHOWN));
        lines.push(memory.tolerance_line());
    }

    lines.join("\n")
}

fn station_name_of(stations: &StationRegistry, id: rail_sim::StationId) -> String {
    stations
        .get(id)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| format!("Station {}", id.0))
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
        cause_jump: None,
        body: String::new(),
    }
}

fn format_cents(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    format!("{sign}${}.{:02}", abs / 100, abs % 100)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::MinimalPlugins;

    fn spawned() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_systems(Startup, setup_inspector_panel);
        app.update();
        app
    }

    /// Brief 07 §4.2 — the cause row is a click target: pressing it selects
    /// whatever the view offered, and a row with nothing to offer changes
    /// nothing.
    #[test]
    fn clicking_the_cause_row_selects_the_offered_blocker() {
        let mut app = spawned();
        app.init_resource::<Selection>();
        app.add_systems(Update, cause_jump_clicks);

        let row = app
            .world_mut()
            .query_filtered::<Entity, With<CauseJump>>()
            .single(app.world())
            .expect("the cause row exists");

        // Nothing offered: a press is inert.
        *app.world_mut().entity_mut(row).get_mut::<Interaction>().unwrap() =
            Interaction::Pressed;
        app.update();
        assert_eq!(app.world().resource::<Selection>().0, None);

        // A blocked train's view offers its blocker.
        app.world_mut().entity_mut(row).get_mut::<CauseJump>().unwrap().0 =
            Some(Selectable::Train(rail_sim::ids::TrainId(7)));
        *app.world_mut().entity_mut(row).get_mut::<Interaction>().unwrap() =
            Interaction::None;
        app.update();
        *app.world_mut().entity_mut(row).get_mut::<Interaction>().unwrap() =
            Interaction::Pressed;
        app.update();
        assert_eq!(
            app.world().resource::<Selection>().0,
            Some(Selectable::Train(rail_sim::ids::TrainId(7)))
        );
    }

    /// The Inspector is a window like every other panel, so the manager can
    /// place it, stack it, drag it and close it. If this ever comes back
    /// `None`, `ui::adapters` is syncing a slot that nothing renders.
    #[test]
    fn the_inspector_is_a_window() {
        let mut app = spawned();
        let mut q = app
            .world_mut()
            .query_filtered::<&UiWindow, With<InspectorRoot>>();
        let window = q.single(app.world()).expect("one inspector root");
        assert_eq!(window.id, WindowId::Inspector);
    }

    /// Nothing is selected at boot, so nothing should be on screen. The window
    /// manager opens it when `Selection` fills in.
    #[test]
    fn the_inspector_starts_hidden_and_is_only_as_wide_as_the_brief_says() {
        let mut app = spawned();
        let mut q = app.world_mut().query_filtered::<&Node, With<InspectorRoot>>();
        let node = q.single(app.world()).expect("one inspector root");
        assert_eq!(node.display, Display::None);
        assert_eq!(node.width, Val::Px(INSPECTOR_W));
    }

    /// Every row the update system writes has to exist, or a panel silently
    /// stops reporting one of its lines.
    #[test]
    fn every_row_the_update_system_writes_exists() {
        let mut app = spawned();
        let world = app.world_mut();
        assert_eq!(world.query::<&InspectorNameText>().iter(world).count(), 1);
        assert_eq!(world.query::<&InspectorTypeText>().iter(world).count(), 1);
        assert_eq!(
            world.query::<&InspectorHeadlineText>().iter(world).count(),
            1
        );
        assert_eq!(world.query::<&InspectorTrendText>().iter(world).count(), 1);
        assert_eq!(world.query::<&InspectorCauseText>().iter(world).count(), 1);
        assert_eq!(world.query::<&InspectorBodyText>().iter(world).count(), 1);
        assert_eq!(world.query::<&InspectorBody>().iter(world).count(), 1);
        // The station verbs live on the card that describes the stop.
        assert_eq!(world.query::<&InspectorActionRow>().iter(world).count(), 1);
        assert_eq!(
            world.query::<&UpgradeStationButton>().iter(world).count(),
            1
        );
        assert_eq!(
            world.query::<&DemolishStationButton>().iter(world).count(),
            1
        );
    }

    /// Flat land with a straight east-west run of `len` tiles from `(4, 8)`.
    fn a_line(len: i32) -> TrackNetwork {
        let terrain = rail_sim::TrackTerrain::new(32, 32, (0..32 * 32).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut money = Money::new(10_000_000);
        let mut ledger = rail_sim::MoneyLedger::default();
        for i in 0..len {
            rail_sim::track::try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                TileCoord { x: 4 + i, y: 8 },
                GROUND_LAYER,
            )
            .expect("track");
        }
        network
    }

    fn a_stop(tier: StationTier) -> (StationRegistry, StationId) {
        let mut stations = StationRegistry::new();
        let id = stations.insert_tier(
            "Ashford",
            TileCoord { x: 6, y: 8 },
            GROUND_LAYER,
            tier,
            tier.build_cents(),
        );
        (stations, id)
    }

    /// The card's Upgrade row names the rung, its price, and what the money
    /// buys — the three things a player needs before spending it.
    #[test]
    fn the_upgrade_offer_names_the_rung_its_price_and_what_it_buys() {
        let (stations, id) = a_stop(StationTier::Halt);
        let offer = upgrade_offer(
            id,
            &stations,
            &IndustryRegistry::new(),
            &a_line(8),
            &Money::new(1_000_000),
        );

        assert!(offer.ready);
        assert_eq!(offer.to, Some(StationTier::Station));
        let price = StationTier::Halt.retier_cents(StationTier::Station);
        assert!(offer.label.contains("Station"), "{}", offer.label);
        assert!(offer.label.contains(&money_whole(price)), "{}", offer.label);
        assert!(
            offer.note.contains(&StationTier::Station.catchment().to_string()),
            "{} should say what the reach becomes",
            offer.note
        );
        assert!(offer.label.is_ascii() && offer.note.is_ascii());
    }

    /// 04 §3's last line — prevention beats rejection. An upgrade that cannot
    /// fit says so where the button is, instead of being refused after the fact
    /// by a sim message nothing used to read.
    #[test]
    fn an_upgrade_that_will_not_fit_says_so_instead_of_offering() {
        // Three tiles of line: a Station fits, an Interchange never will.
        let (stations, id) = a_stop(StationTier::Station);
        let offer = upgrade_offer(
            id,
            &stations,
            &IndustryRegistry::new(),
            &a_line(3),
            &Money::new(10_000_000),
        );
        assert!(!offer.ready);
        assert_eq!(offer.to, None, "a blocked offer sends no command");
        assert_eq!(
            offer.note,
            format!(
                "Not enough platform - 3 tiles of line, needs {}",
                StationTier::Interchange.platforms()
            )
        );

        // Money is a reason too, and it names the shortfall.
        let (stations, id) = a_stop(StationTier::Halt);
        let price = StationTier::Halt.retier_cents(StationTier::Station);
        let offer = upgrade_offer(
            id,
            &stations,
            &IndustryRegistry::new(),
            &a_line(8),
            &Money::new(price - 500),
        );
        assert!(!offer.ready);
        assert_eq!(offer.note, "Short by $5.00");
    }

    /// The two tiers off the ladder are built, not upgraded into — and the card
    /// says which, rather than showing a button that does nothing.
    #[test]
    fn the_off_ladder_tiers_explain_themselves() {
        for (tier, expected) in [
            (
                StationTier::Interchange,
                "Interchange is the top of the ladder",
            ),
            (
                StationTier::Terminus,
                "Terminus is built, not upgraded into",
            ),
            (
                StationTier::GoodsPlatform,
                "Goods Platform is built, not upgraded into",
            ),
        ] {
            let (stations, id) = a_stop(tier);
            let offer = upgrade_offer(
                id,
                &stations,
                &IndustryRegistry::new(),
                &a_line(8),
                &Money::new(10_000_000),
            );
            assert!(!offer.ready);
            assert_eq!(offer.note, expected);
        }
    }

    /// The card is where a player reads what a stop *is*. It used to say
    /// "Station - Tier 1" for all five tiers.
    #[test]
    fn the_station_card_reads_its_own_tier_catchment_and_calls() {
        let (stations, id) = a_stop(StationTier::Interchange);
        let mut lines = LineRegistry::new();
        lines
            .create("Riverside Loop".into(), vec![id, StationId(99)])
            .expect("a line");

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let world = app.world_mut();
        let mut peeps = world.query::<(
            &Peep,
            &WaitingAtStation,
            Option<&Routine>,
            Option<&Journey>,
            Option<&JourneyMemory>,
        )>();
        let mut trains = world.query::<(&Train, &TrainLocation, &TrainCargo)>();

        let view = build_view(
            Selectable::Station(id),
            &stations,
            &IndustryRegistry::new(),
            &StationService::default(),
            &ServiceScoreHistory::default(),
            &a_line(8),
            &TileOccupancy::default(),
            &lines,
            &HouseholdRegistry::default(),
            &peeps.query(world),
            &trains.query(world),
        );

        assert_eq!(view.type_line, "Station - Interchange");
        assert_eq!(view.name, "Ashford");
        assert!(
            view.body.contains(&format!(
                "Catchment: {} tiles",
                StationTier::Interchange.catchment()
            )),
            "{}",
            view.body
        );
        assert!(view.body.contains("1 line calls here"), "{}", view.body);
        assert!(view.body.is_ascii());

        // And a stop that has just been lifted — very possibly from this card —
        // says so instead of reading as a real stop nobody uses.
        let gone = build_view(
            Selectable::Station(StationId(404)),
            &stations,
            &IndustryRegistry::new(),
            &StationService::default(),
            &ServiceScoreHistory::default(),
            &a_line(8),
            &TileOccupancy::default(),
            &lines,
            &HouseholdRegistry::default(),
            &peeps.query(world),
            &trains.query(world),
        );
        assert_eq!(gone.cause, "No longer in the world.");
    }

    /// The card's verbs are the same commands the tool issues — an upgrade goes
    /// straight to the buffer, and a demolish that costs somebody a call goes
    /// through the confirm first (04 §4).
    #[test]
    fn the_card_verbs_issue_the_commands_they_name() {
        let (stations, id) = a_stop(StationTier::Halt);
        let mut lines = LineRegistry::new();
        lines
            .create("Riverside Loop".into(), vec![id, StationId(99)])
            .expect("a line");

        let mut app = spawned();
        app.insert_resource(Selection(Some(Selectable::Station(id))))
            .insert_resource(stations)
            .insert_resource(lines)
            .insert_resource(a_line(8))
            .init_resource::<IndustryRegistry>()
            .insert_resource(Money::new(10_000_000))
            .init_resource::<CommandBuffer>()
            .init_resource::<ConfirmDialog>()
            .add_systems(Update, station_action_clicks);

        let press = |app: &mut App, entity: Entity| {
            app.world_mut().entity_mut(entity).insert(Interaction::None);
            app.update();
            app.world_mut()
                .entity_mut(entity)
                .insert(Interaction::Pressed);
            app.update();
        };

        let upgrade = app
            .world_mut()
            .query_filtered::<Entity, With<UpgradeStationButton>>()
            .single(app.world())
            .expect("an upgrade button");
        press(&mut app, upgrade);
        let pending = app.world().resource::<CommandBuffer>().pending().to_vec();
        assert!(
            matches!(
                pending.first().map(|c| &c.kind),
                Some(rail_sim::CommandKind::UpgradeStation(u))
                    if u.station == id && u.to == StationTier::Station
            ),
            "{pending:?}"
        );

        let demolish = app
            .world_mut()
            .query_filtered::<Entity, With<DemolishStationButton>>()
            .single(app.world())
            .expect("a demolish button");
        press(&mut app, demolish);
        let dialog = app.world().resource::<ConfirmDialog>();
        assert!(dialog.is_open(), "a stop a line calls at asks first");
        assert_eq!(
            app.world().resource::<CommandBuffer>().pending().len(),
            1,
            "and nothing is demolished until the player says yes"
        );
    }

    use rail_sim::{
        JourneyOutcome, JourneyRecord, JourneyStage, PeepId, StationId, TileCoord, GROUND_LAYER,
        TICKS_PER_DAY,
    };

    /// A named resident with a home, a family, a routine and a history.
    fn a_resident() -> (
        Peep,
        Routine,
        Journey,
        JourneyMemory,
        HouseholdRegistry,
        StationRegistry,
    ) {
        let mut stations = StationRegistry::new();
        let east = stations.insert("Eastgate", TileCoord { x: 4, y: 4 }, GROUND_LAYER);
        let mill = stations.insert("Millhaven", TileCoord { x: 20, y: 4 }, GROUND_LAYER);

        let home = TileCoord { x: 5, y: 5 };
        let mut households = HouseholdRegistry::new();
        let household = households.insert(home, east, 0);
        households.add_member(household, PeepId(1));
        households.add_member(household, PeepId(2));

        let peep = Peep::new(PeepId(1), "Mara Aldertone", home, household, 0);
        let routine = Routine::from_seed(3, home, east, TileCoord { x: 21, y: 5 }, mill);
        let mut journey = Journey::new(&routine);
        journey.set_stage(JourneyStage::WaitingOnPlatform);
        let mut memory = JourneyMemory::default();
        memory.record(JourneyRecord {
            from: east,
            to: mill,
            wait_secs: 60,
            total_secs: 240,
            outcome: JourneyOutcome::Good,
            ended_tick: 0,
        });
        (peep, routine, journey, memory, households, stations)
    }

    /// Brief 05 §3.3 — the panel that earns the design's emotional hook. The
    /// card used to be station / wait / home tile, with `Journey::describe`,
    /// `Peep::tenure_line` and the whole memory slice sitting unread.
    #[test]
    fn the_peep_card_reads_as_a_person() {
        let (peep, routine, journey, memory, households, stations) = a_resident();
        let body = peep_body(
            &peep,
            Some(&routine),
            Some(&journey),
            Some(&memory),
            &households,
            &stations,
            "Eastgate",
            TICKS_PER_DAY * 14,
        );
        assert!(body.is_ascii(), "the bitmap font has no other glyphs: {body}");
        for expected in [
            "Waiting at Eastgate for the Millhaven train.",
            "Mara has lived in Eastgate for 14 days.",
            "leaves Eastgate about",
            "a household of 2.",
            "Recent trips: good.",
        ] {
            assert!(body.contains(expected), "card is missing {expected:?}:\n{body}");
        }
    }

    /// A peep restored from an older save can arrive with only the components
    /// that snapshot stored. The card degrades a line at a time, never vanishes.
    #[test]
    fn a_peep_missing_its_routine_still_gets_a_card() {
        let (peep, _, _, _, _, stations) = a_resident();
        let body = peep_body(
            &peep,
            None,
            None,
            None,
            &HouseholdRegistry::new(),
            &stations,
            "Eastgate",
            0,
        );
        assert_eq!(body, "Mara has just moved to Eastgate.");
    }

    #[test]
    fn a_station_name_falls_back_rather_than_going_blank() {
        let stations = StationRegistry::new();
        assert_eq!(station_name_of(&stations, StationId(7)), "Station 7");
    }

    #[test]
    fn money_reads_as_money() {
        assert_eq!(format_cents(0), "$0.00");
        assert_eq!(format_cents(1234), "$12.34");
        assert_eq!(format_cents(-509), "-$5.09");
    }
}
