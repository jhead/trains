//! Trains window — every train the player owns, and what to do with each one.
//!
//! Opens from the menu row or `R`.
//!
//! # Why this exists
//!
//! *"It might be spending money and placing it but I cannot see it."*
//!
//! Until now the only way to find a train was to spot it on the map, and stock
//! in [`TrainYard`] is not on the map at all — it is a `Vec` in a resource with
//! no sprite, no marker and no readout. A player who bought a goods train and
//! then mis-clicked the placement owned something they had no way of finding,
//! and the only evidence it existed was a Town Talk line that had already
//! scrolled away.
//!
//! So the yard is the point of this window, not an afterthought in it. Unplaced
//! stock sorts to the **top** of the list, above everything running, and its row
//! carries the button that finishes the job.
//!
//! # Model first
//!
//! [`train_rows`] is a pure function of the sim's registries and returns the
//! rows as data. Everything the player reads — the status sentence, the cargo
//! line, which buttons a row offers — is decided there and tested there, with no
//! `World` and no UI. The systems below only draw it and turn clicks into the
//! commands the rest of the game already understands: [`Selection`] plus
//! [`CameraFocusRequest`] to locate, [`arm_train_place`] to place, and the one
//! [`ConfirmDialog`] to sell.

use bevy::prelude::*;
use rail_map::tile_to_world;
use rail_sim::{
    buy_cost, IndustryRegistry, LineRegistry, StationRegistry, TrackNetwork, Train, TrainCargo,
    TrainId, TrainKind, TrainLocation, TrainYard, GROUND_LAYER,
};

use crate::inspect::{Selectable, Selection};
use crate::map::CameraFocusRequest;
use crate::palette::{BG1, OUTLINE};
use crate::ui::format::money_whole;
use crate::ui::kit::{
    chrome_button_node, control_border, micro_font, text_accent, text_primary, text_secondary,
    SPACE_1,
};
use crate::ui::toolbar::{ToolStates, ToolbarTool};
use crate::ui::window::{window_root, WindowId, WindowManager};
use crate::ui::{ConfirmAction, ConfirmDialog, ConfirmPrompt};

/// Where a train is, in one sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrainStanding {
    /// Bought, never placed. Invisible on the map — the reason for this window.
    InYard,
    /// Placed, but the track under it is gone. Nothing will move it again.
    Stranded,
    /// Crewed to a player line.
    OnLine(String),
    /// Running free, standing at a stop it can be named by.
    AtStop(String),
    /// Running free, somewhere between stops.
    Running,
}

impl TrainStanding {
    /// The status column. ASCII, and it says what to do when there is something
    /// to do (03 §3, 04 §4).
    pub fn label(&self) -> String {
        match self {
            Self::InYard => "in yard - not placed yet".into(),
            Self::Stranded => "stranded - no track under it".into(),
            Self::OnLine(name) => format!("on line {name}"),
            Self::AtStop(name) => format!("in service at {name}"),
            Self::Running => "in service".into(),
        }
    }

    /// `true` when the train exists only in the yard, so there is nowhere to fly.
    pub fn is_unplaced(&self) -> bool {
        matches!(self, Self::InYard)
    }
}

/// One row of the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainRow {
    pub id: TrainId,
    pub kind: TrainKind,
    pub standing: TrainStanding,
    /// What it is carrying, or `None` when it is empty or still in the yard.
    pub cargo: Option<String>,
    /// Where "locate" should fly to. `None` for yard stock.
    pub tile: Option<rail_sim::TileCoord>,
}

impl TrainRow {
    /// The row's own name — matching the Inspector's and Town Talk's wording, so
    /// a player can carry a train's identity between the three.
    pub fn title(&self) -> String {
        format!("{} train {}", kind_label(self.kind), self.id.0)
    }

    /// Title and status, as the row draws them.
    pub fn headline(&self) -> String {
        format!("{} - {}", self.title(), self.standing.label())
    }
}

fn kind_label(kind: TrainKind) -> &'static str {
    match kind {
        TrainKind::Transit => "Transit",
        TrainKind::Transport => "Transport",
    }
}

/// Everything the window shows, built from the sim and nothing else.
///
/// Yard stock first, then the map, each group by id — so a row does not move
/// under the player's cursor because a train crossed a tile.
pub fn train_rows(
    yard: &TrainYard,
    placed: &[(&Train, &TrainLocation, &TrainCargo)],
    lines: &LineRegistry,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    network: &TrackNetwork,
) -> Vec<TrainRow> {
    let mut rows: Vec<TrainRow> = yard
        .unplaced()
        .iter()
        .map(|(id, kind)| TrainRow {
            id: *id,
            kind: *kind,
            standing: TrainStanding::InYard,
            cargo: None,
            tile: None,
        })
        .collect();
    rows.sort_by_key(|r| r.id.0);

    let mut running: Vec<TrainRow> = placed
        .iter()
        .map(|(train, loc, cargo)| {
            let tile = network.piece(loc.track).map(|p| p.tile);
            // Stranded is checked *before* the line, deliberately: a crewed
            // train whose rails were lifted is still going nowhere, and "on
            // line Coast" would be the reassuring half of the truth. `movement`
            // cannot advance a train whose piece is gone, so this is terminal
            // until the player sells it or relays the track.
            let standing = match (tile, lines.line_for_train(train.id)) {
                (None, _) => TrainStanding::Stranded,
                (Some(_), Some(line)) => TrainStanding::OnLine(line.name.clone()),
                (Some(tile), None) => match station_at(stations, tile) {
                    Some(name) => TrainStanding::AtStop(name),
                    None => TrainStanding::Running,
                },
            };
            TrainRow {
                id: train.id,
                kind: train.kind,
                standing,
                cargo: cargo_label(cargo, stations, industries),
                tile,
            }
        })
        .collect();
    running.sort_by_key(|r| r.id.0);

    rows.append(&mut running);
    rows
}

/// A stop on or beside `tile`, by name — the same reach a placement click has.
fn station_at(stations: &StationRegistry, tile: rail_sim::TileCoord) -> Option<String> {
    if let Some(id) = stations.id_at(tile, GROUND_LAYER) {
        return stations.get(id).map(|s| s.name.clone());
    }
    stations
        .iter()
        .find(|s| (s.tile.x - tile.x).abs() <= 1 && (s.tile.y - tile.y).abs() <= 1)
        .map(|s| s.name.clone())
}

/// What a train is carrying, in the Inspector's words.
fn cargo_label(
    cargo: &TrainCargo,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
) -> Option<String> {
    match cargo {
        TrainCargo::Empty => None,
        TrainCargo::Passengers { from, to } => {
            let a = stations.get(*from).map(|s| s.name.as_str()).unwrap_or("?");
            let b = stations.get(*to).map(|s| s.name.as_str()).unwrap_or("?");
            Some(format!("Passengers {a} -> {b}"))
        }
        TrainCargo::Goods { kind, from, to } => {
            let a = industries.get(*from).map(|i| i.name.as_str()).unwrap_or("?");
            let b = industries.get(*to).map(|i| i.name.as_str()).unwrap_or("?");
            Some(format!("{} {a} -> {b}", kind.label()))
        }
    }
}

// ─ The window ──────────────────────────────────────────────

#[derive(Component)]
pub struct TrainsPanelRoot;

#[derive(Component)]
pub struct TrainsListBody;

/// A per-row verb. One component for all three, so one click system serves them.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrainRowButton {
    pub train: TrainId,
    pub kind: TrainKind,
    pub action: TrainRowAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainRowAction {
    /// Select it and fly the camera to it.
    Locate,
    /// Arm the place verb for this train's kind.
    Place,
    /// Ask, then sell.
    Sell,
}

impl TrainRowAction {
    fn label(self) -> &'static str {
        match self {
            Self::Locate => "Find",
            Self::Place => "Place",
            Self::Sell => "Sell",
        }
    }
}

/// What the list drew last time, so an unchanged frame costs a string compare.
///
/// `Option`, not `String`: an empty railway's fingerprint is the empty string,
/// so a `String::default()` cache reads as "already drawn" on the very first
/// pass and the *"No trains yet"* line never appears. A window that opens blank
/// is indistinguishable from one that is broken, which is the same category of
/// bug this whole window exists to answer.
#[derive(Resource, Debug, Default)]
pub(crate) struct TrainsUiCache {
    fingerprint: Option<String>,
}

pub fn setup_trains_ui(mut commands: Commands) {
    commands.insert_resource(TrainsUiCache::default());
    commands
        .spawn((TrainsPanelRoot, window_root(WindowId::Trains, 296.0)))
        .with_children(|panel| {
            panel.spawn((
                TrainsListBody,
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(SPACE_1),
                    ..default()
                },
            ));
        });
}

/// Everything the list reads out of the sim, bundled for the parameter budget.
#[derive(bevy::ecs::system::SystemParam)]
pub struct TrainsWorld<'w, 's> {
    yard: Res<'w, TrainYard>,
    lines: Res<'w, LineRegistry>,
    stations: Res<'w, StationRegistry>,
    industries: Res<'w, IndustryRegistry>,
    network: Res<'w, TrackNetwork>,
    trains: Query<'w, 's, (&'static Train, &'static TrainLocation, &'static TrainCargo)>,
}

/// Repaint the list when — and only when — it has actually changed.
pub fn update_trains_ui(
    manager: Res<WindowManager>,
    world: TrainsWorld,
    mut cache: ResMut<TrainsUiCache>,
    mut commands: Commands,
    body_q: Query<Entity, With<TrainsListBody>>,
    children_q: Query<&Children, With<TrainsListBody>>,
) {
    if !manager.is_open(WindowId::Trains) {
        return;
    }
    let placed: Vec<(&Train, &TrainLocation, &TrainCargo)> = world.trains.iter().collect();
    let rows = train_rows(
        &world.yard,
        &placed,
        &world.lines,
        &world.stations,
        &world.industries,
        &world.network,
    );

    let fingerprint = rows
        .iter()
        .map(|r| format!("{}:{}", r.headline(), r.cargo.clone().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("|");
    if cache.fingerprint.as_deref() == Some(fingerprint.as_str()) {
        return;
    }
    cache.fingerprint = Some(fingerprint);

    let Ok(body) = body_q.single() else {
        return;
    };
    if let Ok(children) = children_q.get(body) {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    commands.entity(body).with_children(|list| {
        if rows.is_empty() {
            list.spawn((
                Text::new("No trains yet. Press T for a transit, G for a goods train."),
                micro_font(),
                text_secondary(),
            ));
            return;
        }
        for row in &rows {
            spawn_row(list, row);
        }
    });
}

fn spawn_row(list: &mut ChildSpawnerCommands, row: &TrainRow) {
    list.spawn((
        Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(1.0),
            padding: UiRect::axes(Val::Px(SPACE_1), Val::Px(1.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::ZERO,
            ..default()
        },
        BackgroundColor(BG1),
        BorderColor::all(OUTLINE),
    ))
    .with_children(|card| {
        // Yard stock reads in the accent, because it is the row with something
        // owed on it. Never colour alone (03 §4) — the status says so in words.
        let tone = if row.standing.is_unplaced() {
            text_accent()
        } else {
            text_primary()
        };
        card.spawn((Text::new(row.headline()), micro_font(), tone));
        if let Some(cargo) = &row.cargo {
            card.spawn((Text::new(cargo.clone()), micro_font(), text_secondary()));
        }
        card.spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(SPACE_1),
            ..default()
        })
        .with_children(|verbs| {
            for action in row_actions(row) {
                let (node, bg, border) = chrome_button_node(SPACE_1, 0.0);
                verbs
                    .spawn((
                        Button,
                        TrainRowButton {
                            train: row.id,
                            kind: row.kind,
                            action,
                        },
                        node,
                        bg,
                        border,
                    ))
                    .with_children(|b| {
                        b.spawn((Text::new(action.label()), micro_font(), text_secondary()));
                    });
            }
        });
    });
}

/// The verbs a row offers.
///
/// A train in the yard cannot be flown to, because it is nowhere; a train on the
/// map cannot be placed, because it already is. Selling is always available —
/// rolling stock is reversible wherever it stands (DESIGN.md).
pub fn row_actions(row: &TrainRow) -> Vec<TrainRowAction> {
    if row.standing.is_unplaced() {
        vec![TrainRowAction::Place, TrainRowAction::Sell]
    } else {
        vec![TrainRowAction::Locate, TrainRowAction::Sell]
    }
}

/// Turn a row button into the verb the rest of the game already has.
pub fn train_row_clicks(
    interactions: Query<(&Interaction, &TrainRowButton), (Changed<Interaction>, With<Button>)>,
    trains: Query<(&Train, &TrainLocation)>,
    network: Res<TrackNetwork>,
    // `ToolStates` already holds `TrainToolState` mutably; asking for it a
    // second time here is a conflicting access Bevy refuses at system init.
    mut tools: ToolStates,
    mut selection: ResMut<Selection>,
    mut focus: ResMut<CameraFocusRequest>,
    mut confirm: ResMut<ConfirmDialog>,
) {
    for (interaction, button) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match button.action {
            TrainRowAction::Locate => {
                selection.set(Selectable::Train(button.train));
                let tile = trains
                    .iter()
                    .find(|(t, _)| t.id == button.train)
                    .and_then(|(_, loc)| network.piece(loc.track))
                    .map(|piece| piece.tile);
                if let Some(tile) = tile {
                    let (wx, wy) = tile_to_world(tile);
                    focus.0 = Some(Vec2::new(wx, wy));
                }
            }
            // The same verb the `T` / `G` keys arm, through the same call — so
            // the window cannot cost something different from the key. The yard
            // already holds this train, so `arm_train_place` buys nothing, and
            // it is the call that sets `TrainToolState::kind` for the click.
            TrainRowAction::Place => tools.arm(match button.kind {
                TrainKind::Transit => ToolbarTool::Transit,
                TrainKind::Transport => ToolbarTool::Transport,
            }),
            // Straight into the one confirm dialog, with the same sentence `X`
            // on a selected train produces.
            TrainRowAction::Sell => {
                if confirm.is_open() {
                    continue;
                }
                confirm.ask(ConfirmPrompt {
                    title: "Sell train".into(),
                    body: format!(
                        "Sell Train {} for {}? It returns its full price.",
                        button.train.0,
                        money_whole(buy_cost(button.kind))
                    ),
                    confirm: "Sell".into(),
                    action: ConfirmAction::SellTrain(button.train),
                });
            }
        }
    }
}

pub fn update_train_row_hover(
    mut q: Query<(&Interaction, &mut BorderColor), (Changed<Interaction>, With<TrainRowButton>)>,
) {
    for (interaction, mut border) in &mut q {
        let hovered = matches!(interaction, Interaction::Hovered | Interaction::Pressed);
        *border = control_border(false, hovered);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::ids::{StationId, TileCoord};
    use rail_sim::{GoodKind, Money, MoneyLedger, TrackTerrain};

    /// A short line with one stop, plus a works, plus a yard.
    fn world() -> (
        TrackNetwork,
        StationRegistry,
        IndustryRegistry,
        LineRegistry,
        Vec<StationId>,
    ) {
        let terrain = TrackTerrain::new(16, 16, (0..16 * 16).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut money = Money::new(10_000_000);
        let mut ledger = MoneyLedger::default();
        for x in 2..=8 {
            rail_sim::track::try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                TileCoord { x, y: 4 },
                GROUND_LAYER,
            )
            .expect("track");
        }
        let mut stations = StationRegistry::new();
        let east = stations.insert("Eastgate", TileCoord { x: 3, y: 4 }, GROUND_LAYER);
        let west = stations.insert("Westbrook", TileCoord { x: 8, y: 4 }, GROUND_LAYER);
        let mut industries = IndustryRegistry::new();
        industries.insert(
            "Marsh Sawmill",
            TileCoord { x: 12, y: 12 },
            Some(GoodKind::Lumber),
            None,
        );
        (
            network,
            stations,
            industries,
            LineRegistry::new(),
            vec![east, west],
        )
    }

    fn track_at(network: &TrackNetwork, x: i32) -> rail_sim::TrackId {
        network
            .id_at(TileCoord { x, y: 4 }, GROUND_LAYER)
            .expect("track")
    }

    /// **The report, restated as a list.** A goods train the player paid for and
    /// never managed to place has to be *on screen somewhere*, above everything
    /// that is already working, with the button that finishes the job.
    #[test]
    fn yard_stock_is_listed_first_and_offers_the_place_verb() {
        let (network, stations, industries, lines, _) = world();
        let mut yard = TrainYard::default();
        let freight = yard.buy(TrainKind::Transport);

        let running = Train {
            id: TrainId(9),
            kind: TrainKind::Transit,
        };
        let loc = TrainLocation::at_track(track_at(&network, 6));
        let cargo = TrainCargo::Empty;

        let rows = train_rows(
            &yard,
            &[(&running, &loc, &cargo)],
            &lines,
            &stations,
            &industries,
            &network,
        );

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, freight, "unplaced stock sorts above the map");
        assert_eq!(
            rows[0].headline(),
            "Transport train 1 - in yard - not placed yet"
        );
        assert_eq!(
            row_actions(&rows[0]),
            vec![TrainRowAction::Place, TrainRowAction::Sell],
            "there is nowhere to fly to, and something to finish"
        );
        assert!(rows[0].tile.is_none());

        assert_eq!(rows[1].id, TrainId(9));
        assert_eq!(
            row_actions(&rows[1]),
            vec![TrainRowAction::Locate, TrainRowAction::Sell]
        );
    }

    #[test]
    fn a_placed_train_reads_its_stop_its_line_and_its_load() {
        let (network, stations, industries, mut lines, stops) = world();
        let yard = TrainYard::default();

        // Standing at the platform, free-roam, empty.
        let idle = Train {
            id: TrainId(1),
            kind: TrainKind::Transit,
        };
        let at_stop = TrainLocation::at_track(track_at(&network, 3));
        let empty = TrainCargo::Empty;

        // Out on the line with a load.
        let hauling = Train {
            id: TrainId(2),
            kind: TrainKind::Transport,
        };
        let mid = TrainLocation::at_track(track_at(&network, 6));
        let goods = TrainCargo::Goods {
            kind: GoodKind::Lumber,
            from: rail_sim::IndustryId(1),
            to: rail_sim::IndustryId(1),
        };

        let line = lines.create("Coast".into(), stops).expect("a line");
        assert!(lines.assign_train(line, TrainId(2)));

        let rows = train_rows(
            &yard,
            &[(&idle, &at_stop, &empty), (&hauling, &mid, &goods)],
            &lines,
            &stations,
            &industries,
            &network,
        );

        assert_eq!(rows[0].headline(), "Transit train 1 - in service at Eastgate");
        assert_eq!(rows[0].cargo, None, "an empty train has no cargo line");
        assert_eq!(rows[1].headline(), "Transport train 2 - on line Coast");
        assert_eq!(
            rows[1].cargo.as_deref(),
            Some("lumber Marsh Sawmill -> Marsh Sawmill")
        );
        // Both are findable.
        assert_eq!(rows[0].tile, Some(TileCoord { x: 3, y: 4 }));
        assert_eq!(rows[1].tile, Some(TileCoord { x: 6, y: 4 }));
    }

    /// Track lifted out from under a train leaves it somewhere `movement` will
    /// never advance it from. The list says so rather than calling it "running".
    #[test]
    fn a_train_whose_track_is_gone_reads_as_stranded() {
        let (network, stations, industries, lines, _) = world();
        let train = Train {
            id: TrainId(4),
            kind: TrainKind::Transport,
        };
        let nowhere = TrainLocation::at_track(rail_sim::TrackId(9_999));
        let cargo = TrainCargo::Empty;

        let rows = train_rows(
            &TrainYard::default(),
            &[(&train, &nowhere, &cargo)],
            &lines,
            &stations,
            &industries,
            &network,
        );
        assert_eq!(rows[0].standing, TrainStanding::Stranded);
        assert_eq!(
            rows[0].headline(),
            "Transport train 4 - stranded - no track under it"
        );
        assert_eq!(rows[0].tile, None);
    }

    #[test]
    fn an_empty_railway_lists_nothing_at_all() {
        let (network, stations, industries, lines, _) = world();
        assert!(train_rows(
            &TrainYard::default(),
            &[],
            &lines,
            &stations,
            &industries,
            &network,
        )
        .is_empty());
    }

    // ─ The buttons, in a real schedule ─────────────────────

    use crate::lines::LineToolState;
    use crate::stations::StationToolState;
    use crate::track::TrackToolState;
    use crate::trains::{TrainPlaceKind, TrainToolState};
    use rail_sim::{CommandBuffer, CommandKind};

    /// Everything `train_row_clicks` reads, plus a pressed button.
    fn clicked(action: TrainRowAction, kind: TrainKind, network: TrackNetwork) -> App {
        let mut app = App::new();
        app.init_resource::<CommandBuffer>()
            .init_resource::<TrackToolState>()
            .init_resource::<TrainToolState>()
            .init_resource::<LineToolState>()
            .init_resource::<StationToolState>()
            .init_resource::<TrainYard>()
            .init_resource::<Selection>()
            .init_resource::<CameraFocusRequest>()
            .init_resource::<ConfirmDialog>()
            .insert_resource(network)
            .add_systems(Update, train_row_clicks);
        app.world_mut().spawn((
            Button,
            Interaction::Pressed,
            TrainRowButton {
                train: TrainId(2),
                kind,
                action,
            },
        ));
        app
    }

    /// **The button the whole window is for.** Placing from the row has to arm
    /// exactly the verb the `G` key arms — same call, same cost — or the window
    /// becomes a second way to buy a train the player already owns.
    #[test]
    fn the_place_button_arms_the_same_verb_the_key_does_and_buys_nothing() {
        let (network, ..) = world();
        let mut app = clicked(TrainRowAction::Place, TrainKind::Transport, network);
        // The train is already stock, which is the case that must not re-buy.
        app.world_mut()
            .resource_mut::<TrainYard>()
            .buy(TrainKind::Transport);
        app.update();

        let tool = app.world().resource::<TrainToolState>();
        assert!(tool.place_mode, "the next world click places");
        assert_eq!(tool.kind, TrainPlaceKind::Transport);
        assert!(
            app.world().resource::<CommandBuffer>().pending().is_empty(),
            "the yard already holds it - nothing may be bought"
        );

        // With an empty yard the same button buys, exactly as the key does.
        let (network, ..) = world();
        let mut app = clicked(TrainRowAction::Place, TrainKind::Transport, network);
        app.update();
        assert!(matches!(
            app.world().resource::<CommandBuffer>().pending().first().map(|c| &c.kind),
            Some(CommandKind::BuyTrain(_))
        ));
    }

    #[test]
    fn the_find_button_selects_the_train_and_flies_the_camera_to_it() {
        let (network, ..) = world();
        let tile = TileCoord { x: 6, y: 4 };
        let track = network.id_at(tile, GROUND_LAYER).expect("track");
        let mut app = clicked(TrainRowAction::Locate, TrainKind::Transit, network);
        app.world_mut().spawn((
            Train {
                id: TrainId(2),
                kind: TrainKind::Transit,
            },
            TrainLocation::at_track(track),
        ));
        app.update();

        assert_eq!(
            app.world().resource::<Selection>().0,
            Some(Selectable::Train(TrainId(2))),
            "the Inspector follows the row"
        );
        let (wx, wy) = tile_to_world(tile);
        assert_eq!(
            app.world().resource::<CameraFocusRequest>().0,
            Some(Vec2::new(wx, wy))
        );
    }

    /// Selling goes through the one confirm dialog, with the same sentence the
    /// `X` key produces — 04 §4: a removal with a consequence names it first.
    #[test]
    fn the_sell_button_asks_before_it_sells() {
        let (network, ..) = world();
        let mut app = clicked(TrainRowAction::Sell, TrainKind::Transport, network);
        app.update();

        assert!(
            app.world().resource::<CommandBuffer>().pending().is_empty(),
            "nothing is sold until the player says yes"
        );
        let dialog = app.world().resource::<ConfirmDialog>();
        let prompt = dialog.prompt().expect("the dialog asks first");
        assert_eq!(
            prompt.body,
            "Sell Train 2 for $4,500? It returns its full price."
        );
        assert_eq!(prompt.action, ConfirmAction::SellTrain(TrainId(2)));
        assert!(prompt.body.is_ascii());
    }

    /// 03 §3 — the shipped font has no glyphs beyond ASCII.
    #[test]
    fn every_string_the_window_draws_is_ascii() {
        let (network, stations, industries, lines, _) = world();
        let mut yard = TrainYard::default();
        yard.buy(TrainKind::Transport);
        let train = Train {
            id: TrainId(2),
            kind: TrainKind::Transit,
        };
        let loc = TrainLocation::at_track(track_at(&network, 3));
        let cargo = TrainCargo::Passengers {
            from: rail_sim::StationId(1),
            to: rail_sim::StationId(1),
        };
        let rows = train_rows(
            &yard,
            &[(&train, &loc, &cargo)],
            &lines,
            &stations,
            &industries,
            &network,
        );
        for row in &rows {
            assert!(row.headline().is_ascii(), "{}", row.headline());
            if let Some(cargo) = &row.cargo {
                assert!(cargo.is_ascii(), "{cargo}");
            }
            for action in row_actions(row) {
                assert!(action.label().is_ascii());
            }
        }
    }
}
