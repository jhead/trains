//! Keyboard buy + click-to-place trains at stations, and selling one back.
//!
//! - `T` — arm **transit** place mode, buying one first only if the yard is empty
//! - `G` — the same for a **transport** (goods) train
//! - Left click on a station tile (or adjacent) — place the oldest unplaced train
//!   of the selected kind (station must have track on/adjacent)
//! - `X` with a train selected — sell it back for its full price, after a confirm
//!
//! # Place before buy
//!
//! These verbs used to push [`CommandKind::BuyTrain`] **every** time they were
//! pressed. A player holding an unplaced train and less than its price in the
//! bank therefore got a failed purchase — which reads as *"I can't place a
//! train, I'm stuck"* — while a free placement click was in fact already armed
//! and a train of theirs was sitting in the yard, invisible.
//!
//! So the yard is asked first: stock the player already owns is placed, and the
//! bank is only touched when there is nothing to place. That makes the verb
//! honest at any balance, and it is what keeps a broke player from being stuck
//! with rolling stock they cannot see.
//!
//! # A click that places nothing says why
//!
//! *"Cannot seem to place a Transport/freight train, it doesn't put anything on
//! the track. It might be spending money and placing it but I cannot see it."*
//!
//! That report is not one bug, it is the absence of an answer. The sprite bank
//! draws freight correctly in both projections
//! (`visuals::a_freight_train_gets_a_sprite_standing_on_its_own_tile_in_either_view`)
//! and the sim spawns it exactly as it spawns a transit. What was missing is
//! that **every** way this verb can decline was a bare `return`: a purchase the
//! bank could not cover, a yard with nothing of that kind in it, and — the one a
//! freight player hits first — a click on the *industry* rather than on a
//! platform. Three different refusals, all of them indistinguishable from a
//! placement that worked and could not be seen.
//!
//! So [`place_at_tile`] returns a [`PlaceRefusal`] rather than `None`, each
//! variant carrying the rule it is enforcing, and the tool speaks it into Town
//! Talk — the feed the buy and the placement already talk in, so the yes and the
//! no arrive in the same place. Freight gets its own sentence, because "click a
//! station" is not useful advice to somebody standing on a sawmill.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{world_to_tile, MapGrid};
use rail_sim::commands::{BuyTrain, PlaceTrain, SellTrain};
use rail_sim::{
    buy_cost, track_for_station, CommandBuffer, CommandKind, ComplaintEntry, ComplaintFeed,
    IndustryRegistry, StationRegistry, StationService, TalkKind, TrainKind, TrainYard,
    TrackNetwork, GROUND_LAYER,
};

use crate::input::{ControlAction, KeyBindings};
use crate::inspect::{Selectable, Selection, WorldClickConsumed};
use crate::lines::LineToolState;
use crate::map::MapCamera;
use crate::track::{BuildTool, TrackToolState};
use crate::ui::format::money_whole;
use crate::ui::{ConfirmAccepted, ConfirmAction, ConfirmDialog, ConfirmPrompt, UiBlocksWorld};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrainPlaceKind {
    #[default]
    Transit,
    Transport,
}

#[derive(Debug, Clone, Default, Resource)]
pub struct TrainToolState {
    /// When true, left-click places a train instead of building track.
    pub place_mode: bool,
    pub kind: TrainPlaceKind,
}

impl TrainPlaceKind {
    fn to_sim(self) -> TrainKind {
        match self {
            Self::Transit => TrainKind::Transit,
            Self::Transport => TrainKind::Transport,
        }
    }
}

/// Arm click-to-place for `kind`, buying one **only if the yard has none**.
///
/// Shared with the menu row ([`crate::ui::toolbar`]), so the pointer and the
/// keyboard cannot disagree about what the verb costs.
pub fn arm_train_place(
    kind: TrainPlaceKind,
    yard: &TrainYard,
    buffer: &mut CommandBuffer,
    train: &mut TrainToolState,
    track: &mut TrackToolState,
    line: &mut LineToolState,
) {
    if yard.peek_kind(kind.to_sim()).is_none() {
        buffer.push(CommandKind::BuyTrain(BuyTrain {
            kind: kind.to_sim(),
        }));
    }
    train.place_mode = true;
    train.kind = kind;
    track.anchor = None;
    track.drag = None;
    track.suppress_build_click = true;
    line.active = false;
    line.clear_draft();
}

/// Why a click did not place a train, in the player's terms.
///
/// Each variant is one rule. [`Self::message`] is the sentence that rule turns
/// into, and it is the only thing the player ever sees of this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceRefusal {
    /// Nothing of that kind is owned. The purchase that would have stocked the
    /// yard failed, and the sim has already said why in its own line — so this
    /// one points at the fix rather than repeating the diagnosis.
    YardEmpty(TrainPlaceKind),
    /// The click landed on or beside an industry with no platform on it. This
    /// is the freight-specific one, and it is the one report A was really about.
    IndustryWithNoPlatform(String),
    /// The click landed on nothing that boards trains at all.
    NotAStop(TrainPlaceKind),
    /// A stop, but one the rails have not reached.
    StopWithNoRails(String),
}

impl PlaceRefusal {
    /// ASCII, whole sentence, names the rule and the way out (03 §3, 04 §4).
    pub fn message(&self) -> String {
        match self {
            Self::YardEmpty(TrainPlaceKind::Transport) => {
                "No goods train in the yard - buy one before placing it".into()
            }
            Self::YardEmpty(TrainPlaceKind::Transit) => {
                "No transit train in the yard - buy one before placing it".into()
            }
            Self::IndustryWithNoPlatform(name) => {
                format!("Freight boards at a goods platform - {name} has none yet")
            }
            Self::NotAStop(TrainPlaceKind::Transport) => {
                "Freight boards at a goods platform - place one against an industry".into()
            }
            Self::NotAStop(TrainPlaceKind::Transit) => {
                "Trains board at a station - click a platform to place one".into()
            }
            Self::StopWithNoRails(name) => {
                format!("{name} has no rails yet - run track to the platform")
            }
        }
    }
}

/// The placement a world click at `tile` should produce, or the rule that
/// stopped it.
///
/// A click counts for a station when it lands on the station tile, on the track
/// that serves it, or on any tile touching it — the same generosity the station
/// tool gives, because a platform is smaller than a finger.
fn place_at_tile(
    tile: rail_sim::TileCoord,
    kind: TrainPlaceKind,
    yard: &TrainYard,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    network: &TrackNetwork,
) -> Result<PlaceTrain, PlaceRefusal> {
    let Some(train) = yard.peek_kind(kind.to_sim()) else {
        return Err(PlaceRefusal::YardEmpty(kind));
    };

    let at_station = stations
        .id_at(tile, GROUND_LAYER)
        .or_else(|| {
            stations.iter().find_map(|s| {
                track_for_station(network, s.tile, s.layer).and_then(|tid| {
                    let piece = network.piece(tid)?;
                    if piece.tile == tile {
                        Some(s.id)
                    } else {
                        None
                    }
                })
            })
        })
        .or_else(|| {
            // Adjacent to a station tile.
            stations.iter().find_map(|s| {
                let dx = (s.tile.x - tile.x).abs();
                let dy = (s.tile.y - tile.y).abs();
                if dx <= 1 && dy <= 1 {
                    Some(s.id)
                } else {
                    None
                }
            })
        });

    let Some(at_station) = at_station else {
        // A freight player aims at the works, not at a platform, because the
        // works is what the train is *for*. Naming the industry they clicked
        // turns a dead click into the next thing to build.
        if let Some(industry) = industries
            .lot_at(tile)
            .or_else(|| industries.abutting(tile))
        {
            return Err(PlaceRefusal::IndustryWithNoPlatform(industry.name.clone()));
        }
        return Err(PlaceRefusal::NotAStop(kind));
    };

    let Some(station) = stations.get(at_station) else {
        return Err(PlaceRefusal::NotAStop(kind));
    };
    if track_for_station(network, station.tile, station.layer).is_none() {
        return Err(PlaceRefusal::StopWithNoRails(station.name.clone()));
    }
    Ok(PlaceTrain { train, at_station })
}

/// The registries a placement click reads, bundled to stay inside Bevy's
/// system-parameter budget.
#[derive(SystemParam)]
pub struct PlaceWorld<'w> {
    stations: Res<'w, StationRegistry>,
    industries: Res<'w, IndustryRegistry>,
    network: Res<'w, TrackNetwork>,
    yard: Res<'w, TrainYard>,
}

/// Push a refusal into Town Talk, where the buy and the placement already speak.
///
/// Consecutive identical lines are dropped: a player clicking the same wrong
/// tile four times has made one mistake, and four copies of the same sentence
/// would push the rest of the feed off the panel.
fn refuse(talk: &mut ComplaintFeed, service: &StationService, refusal: &PlaceRefusal) {
    let line = refusal.message();
    if talk.iter().next().is_some_and(|e| e.peep_name == line) {
        return;
    }
    talk.push(ComplaintEntry {
        kind: TalkKind::Warning,
        peep_name: line,
        station_name: String::new(),
        wait_minutes: 0,
        sim_tick: service.tick,
        peep_id: None,
        station_id: None,
        tile: None,
        count: 1,
    });
}

#[allow(clippy::too_many_arguments)]
pub fn train_tool_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MapCamera>>,
    map: Res<MapGrid>,
    world: PlaceWorld,
    service: Res<StationService>,
    mut talk: ResMut<ComplaintFeed>,
    mut buffer: ResMut<CommandBuffer>,
    mut train_state: ResMut<TrainToolState>,
    mut track_state: ResMut<TrackToolState>,
    mut line_state: ResMut<LineToolState>,
    ui_blocks: Res<UiBlocksWorld>,
    click_consumed: Res<WorldClickConsumed>,
) {
    if bindings.just_pressed(&keys, ControlAction::BuyTransit) {
        arm_train_place(
            TrainPlaceKind::Transit,
            &world.yard,
            &mut buffer,
            &mut train_state,
            &mut track_state,
            &mut line_state,
        );
    }
    if bindings.just_pressed(&keys, ControlAction::BuyTransport) {
        arm_train_place(
            TrainPlaceKind::Transport,
            &world.yard,
            &mut buffer,
            &mut train_state,
            &mut track_state,
            &mut line_state,
        );
    }
    // The track verbs reclaim the pointer.
    if bindings.any_just_pressed(
        &keys,
        &[ControlAction::TrackTool, ControlAction::DemolishTool],
    ) {
        train_state.place_mode = false;
        track_state.suppress_build_click = false;
        line_state.active = false;
        line_state.clear_draft();
    }

    if !train_state.place_mode {
        return;
    }
    // Don't fight demolish clicks.
    if track_state.tool == BuildTool::Demolish {
        return;
    }

    if ui_blocks.0 || click_consumed.0 {
        return;
    }

    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((camera, cam_transform)) = camera_q.single() else {
        return;
    };
    let Ok(pointer) = camera.viewport_to_world_2d(cam_transform, cursor) else {
        return;
    };
    let tile = world_to_tile(pointer.x, pointer.y);
    if !map.contains(tile) {
        return;
    }

    match place_at_tile(
        tile,
        train_state.kind,
        &world.yard,
        &world.stations,
        &world.industries,
        &world.network,
    ) {
        Ok(place) => {
            buffer.push(CommandKind::PlaceTrain(place));
        }
        // The click was a real attempt at a real verb. It gets a real answer.
        Err(refusal) => refuse(&mut talk, &service, &refusal),
    }
}

/// `X` on a selected train asks whether to sell it, naming the price.
///
/// Rolling stock is reversible the way track is (DESIGN.md — *"demolition
/// refunds in full"*), and this is the verb that says so. It reuses the Demolish
/// key and the one confirm dialog rather than inventing either: selling a train
/// is a demolish that happens to be selected rather than pointed at, and 04 §4
/// wants a removal with a consequence to name it before it happens.
pub fn sell_selected_train_input(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    selection: Res<Selection>,
    trains: Query<&rail_sim::Train>,
    mut confirm: ResMut<ConfirmDialog>,
) {
    // A question on screen owns the keyboard until it is answered.
    if confirm.is_open() {
        return;
    }
    if !bindings.just_pressed(&keys, ControlAction::DemolishTool) {
        return;
    }
    let Some(Selectable::Train(id)) = selection.0 else {
        return;
    };
    let Some(train) = trains.iter().find(|t| t.id == id) else {
        return;
    };
    confirm.ask(ConfirmPrompt {
        title: "Sell train".into(),
        body: format!(
            "Sell Train {} for {}? It returns its full price.",
            id.0,
            money_whole(buy_cost(train.kind))
        ),
        confirm: "Sell".into(),
        action: ConfirmAction::SellTrain(id),
    });
}

/// Carry out a sale the player agreed to in the dialog.
///
/// The dialog never touches the sim: it hands the action back and this issues
/// the command, on the tick boundary like every other intent.
pub fn apply_confirmed_sell(
    mut accepted: MessageReader<ConfirmAccepted>,
    mut buffer: ResMut<CommandBuffer>,
    mut selection: ResMut<Selection>,
) {
    for ConfirmAccepted(action) in accepted.read() {
        let ConfirmAction::SellTrain(train) = action else {
            continue;
        };
        buffer.push(CommandKind::SellTrain(SellTrain { train: *train }));
        // The Inspector must not sit open on a train that is on its way out.
        if selection.0 == Some(Selectable::Train(*train)) {
            selection.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::ids::TileCoord;
    use rail_sim::{Money, Train, TrainId, TrainYard, TRANSIT_COST_CENTS};

    /// One east-west line with a stop on it, and a yard holding one transit.
    fn world() -> (TrainYard, StationRegistry, TrackNetwork, rail_sim::StationId) {
        let (yard, stations, network, station, _) = world_with_works();
        (yard, stations, network, station)
    }

    /// The same railway, plus a sawmill sitting off it with no platform.
    ///
    /// The works is the thing a freight player points at, so it has to be in
    /// the fixture that tests where freight clicks land.
    fn world_with_works() -> (
        TrainYard,
        StationRegistry,
        TrackNetwork,
        rail_sim::StationId,
        IndustryRegistry,
    ) {
        let terrain = rail_sim::TrackTerrain::new(16, 16, (0..16 * 16).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut money = Money::new(10_000_000);
        let mut ledger = rail_sim::MoneyLedger::default();
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
        let station = stations.insert("Eastgate", TileCoord { x: 3, y: 4 }, GROUND_LAYER);
        let mut industries = IndustryRegistry::new();
        industries.insert(
            "Marsh Sawmill",
            TileCoord { x: 12, y: 12 },
            Some(rail_sim::GoodKind::Lumber),
            None,
        );
        let mut yard = TrainYard::default();
        yard.buy(TrainKind::Transit);
        (yard, stations, network, station, industries)
    }

    /// The refusal a click at `tile` produces, with the freight yard stocked.
    fn refusal_for(
        tile: TileCoord,
        kind: TrainPlaceKind,
        yard: &TrainYard,
        stations: &StationRegistry,
        industries: &IndustryRegistry,
        network: &TrackNetwork,
    ) -> String {
        place_at_tile(tile, kind, yard, stations, industries, network)
            .expect_err("this click should have been refused")
            .message()
    }

    /// **The soft-lock.** A player with $1,500, a train already in the yard, and
    /// a transit train costing more than that pressed `T` and got a failed
    /// purchase — reading as "I cannot place a train" — when what they actually
    /// had was a free placement waiting for a click.
    #[test]
    fn a_yard_train_is_placed_rather_than_bought_however_broke_the_player_is() {
        let (yard, ..) = world();
        let mut buffer = CommandBuffer::default();
        let mut train = TrainToolState::default();
        let mut track = TrackToolState::default();
        let mut line = LineToolState::default();
        let broke = Money::new(0);
        assert!(broke.cents() < TRANSIT_COST_CENTS);

        arm_train_place(
            TrainPlaceKind::Transit,
            &yard,
            &mut buffer,
            &mut train,
            &mut track,
            &mut line,
        );

        assert!(train.place_mode, "the click is armed");
        assert_eq!(train.kind, TrainPlaceKind::Transit);
        assert!(
            buffer.pending().is_empty(),
            "nothing was bought: the player already owns one, and at $0 the \
             purchase would have failed loudly for no reason"
        );
    }

    #[test]
    fn an_empty_yard_still_buys_one() {
        let mut buffer = CommandBuffer::default();
        let mut train = TrainToolState::default();
        let mut track = TrackToolState::default();
        let mut line = LineToolState::default();

        arm_train_place(
            TrainPlaceKind::Transit,
            &TrainYard::default(),
            &mut buffer,
            &mut train,
            &mut track,
            &mut line,
        );

        let pending = buffer.pending();
        assert_eq!(pending.len(), 1);
        assert!(matches!(
            pending[0].kind,
            CommandKind::BuyTrain(BuyTrain {
                kind: TrainKind::Transit
            })
        ));
        assert!(train.place_mode);
    }

    #[test]
    fn the_yard_is_asked_per_kind_not_per_train() {
        // A transit in the yard must not make the *goods* verb free.
        let (yard, ..) = world();
        let mut buffer = CommandBuffer::default();
        let mut train = TrainToolState::default();
        let mut track = TrackToolState::default();
        let mut line = LineToolState::default();

        arm_train_place(
            TrainPlaceKind::Transport,
            &yard,
            &mut buffer,
            &mut train,
            &mut track,
            &mut line,
        );

        assert!(matches!(
            buffer.pending().first().map(|c| &c.kind),
            Some(CommandKind::BuyTrain(BuyTrain {
                kind: TrainKind::Transport
            }))
        ));
    }

    #[test]
    fn a_click_on_a_station_places_the_train_that_is_already_in_the_yard() {
        let (yard, stations, network, station, industries) = world_with_works();
        let place = place_at_tile(
            TileCoord { x: 3, y: 4 },
            TrainPlaceKind::Transit,
            &yard,
            &stations,
            &industries,
            &network,
        )
        .expect("a station under the click and a train in the yard");
        assert_eq!(place.at_station, station);
        assert_eq!(Some(place.train), yard.peek_kind(TrainKind::Transit));
    }

    #[test]
    fn a_click_next_to_the_platform_still_places() {
        let (yard, stations, network, station, industries) = world_with_works();
        let place = place_at_tile(
            TileCoord { x: 4, y: 5 },
            TrainPlaceKind::Transit,
            &yard,
            &stations,
            &industries,
            &network,
        )
        .expect("a platform is smaller than a finger");
        assert_eq!(place.at_station, station);
    }

    /// **Report A, the one a freight player actually hits.** A goods train is
    /// *for* the works, so that is where the click goes — and there is no
    /// platform there, so nothing happened and nothing was said. The refusal
    /// now names the works and the rule that governs it.
    #[test]
    fn clicking_the_works_with_a_goods_train_names_the_missing_platform() {
        let (mut yard, stations, network, _, industries) = world_with_works();
        yard.buy(TrainKind::Transport);

        assert_eq!(
            refusal_for(
                TileCoord { x: 12, y: 12 },
                TrainPlaceKind::Transport,
                &yard,
                &stations,
                &industries,
                &network,
            ),
            "Freight boards at a goods platform - Marsh Sawmill has none yet"
        );
    }

    /// Off the works and off any platform, the rule is still stated — and it is
    /// stated differently for freight, because "click a station" is not the
    /// advice a goods train needs.
    #[test]
    fn a_click_on_open_ground_says_where_trains_do_board() {
        let (mut yard, stations, network, _, industries) = world_with_works();
        yard.buy(TrainKind::Transport);
        let nowhere = TileCoord { x: 1, y: 14 };

        assert_eq!(
            refusal_for(
                nowhere,
                TrainPlaceKind::Transit,
                &yard,
                &stations,
                &industries,
                &network,
            ),
            "Trains board at a station - click a platform to place one"
        );
        assert_eq!(
            refusal_for(
                nowhere,
                TrainPlaceKind::Transport,
                &yard,
                &stations,
                &industries,
                &network,
            ),
            "Freight boards at a goods platform - place one against an industry"
        );
    }

    #[test]
    fn an_empty_yard_says_the_yard_is_empty_rather_than_nothing_at_all() {
        let (_, stations, network, _, industries) = world_with_works();
        assert_eq!(
            refusal_for(
                TileCoord { x: 3, y: 4 },
                TrainPlaceKind::Transport,
                &TrainYard::default(),
                &stations,
                &industries,
                &network,
            ),
            "No goods train in the yard - buy one before placing it"
        );
    }

    #[test]
    fn a_platform_the_rails_have_not_reached_says_so_by_name() {
        let (yard, mut stations, network, _, industries) = world_with_works();
        stations.insert("Fell End", TileCoord { x: 13, y: 13 }, GROUND_LAYER);
        assert_eq!(
            refusal_for(
                TileCoord { x: 13, y: 13 },
                TrainPlaceKind::Transit,
                &yard,
                &stations,
                &industries,
                &network,
            ),
            "Fell End has no rails yet - run track to the platform"
        );
    }

    /// 03 §3: the shipped font has no glyphs beyond ASCII, and a refusal that
    /// draws as tofu is a refusal the player cannot read.
    #[test]
    fn every_refusal_is_a_readable_sentence() {
        let all = [
            PlaceRefusal::YardEmpty(TrainPlaceKind::Transit),
            PlaceRefusal::YardEmpty(TrainPlaceKind::Transport),
            PlaceRefusal::IndustryWithNoPlatform("Marsh Sawmill".into()),
            PlaceRefusal::NotAStop(TrainPlaceKind::Transit),
            PlaceRefusal::NotAStop(TrainPlaceKind::Transport),
            PlaceRefusal::StopWithNoRails("Fell End".into()),
        ];
        for refusal in all {
            let line = refusal.message();
            assert!(line.is_ascii(), "{refusal:?} draws tofu: {line}");
            assert!(line.contains(" - "), "{refusal:?} states no rule: {line}");
        }
    }

    /// Four clicks on the same wrong tile are one mistake. The feed says it
    /// once, so the rest of Town Talk is still readable.
    #[test]
    fn the_same_refusal_does_not_repeat_itself_down_the_feed() {
        let mut talk = ComplaintFeed::default();
        let service = StationService::default();
        let refusal = PlaceRefusal::NotAStop(TrainPlaceKind::Transport);
        for _ in 0..4 {
            refuse(&mut talk, &service, &refusal);
        }
        assert_eq!(talk.len(), 1);

        // A *different* refusal is different news and still gets through.
        refuse(
            &mut talk,
            &service,
            &PlaceRefusal::YardEmpty(TrainPlaceKind::Transport),
        );
        assert_eq!(talk.len(), 2);
        assert_eq!(
            talk.iter().next().map(|e| e.display_line()),
            Some("No goods train in the yard - buy one before placing it".into())
        );
    }

    #[test]
    fn the_sell_prompt_names_the_train_and_the_money() {
        // The copy is the whole point of the dialog (04 §4): a confirm that
        // does not name the consequence is a speed bump.
        let mut app = App::new();
        app.init_resource::<ConfirmDialog>()
            .init_resource::<Selection>()
            .init_resource::<KeyBindings>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_systems(Update, sell_selected_train_input);

        let train = app
            .world_mut()
            .spawn(Train {
                id: TrainId(3),
                kind: TrainKind::Transit,
            })
            .id();
        let _ = train;
        app.world_mut()
            .resource_mut::<Selection>()
            .set(Selectable::Train(TrainId(3)));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyBindings::default().key(ControlAction::DemolishTool));

        app.update();

        let dialog = app.world().resource::<ConfirmDialog>();
        let prompt = dialog.prompt().expect("the dialog asks first");
        assert_eq!(
            prompt.body,
            "Sell Train 3 for $3,000? It returns its full price."
        );
        assert_eq!(prompt.confirm, "Sell", "the button is the verb");
        assert_eq!(prompt.action, ConfirmAction::SellTrain(TrainId(3)));
        assert!(prompt.body.is_ascii() && prompt.title.is_ascii());
    }

    #[test]
    fn nothing_is_asked_when_the_selection_is_not_a_train() {
        let mut app = App::new();
        app.init_resource::<ConfirmDialog>()
            .init_resource::<Selection>()
            .init_resource::<KeyBindings>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_systems(Update, sell_selected_train_input);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyBindings::default().key(ControlAction::DemolishTool));

        app.update();

        assert!(
            !app.world().resource::<ConfirmDialog>().is_open(),
            "the demolish key with nothing selected is the track tool's, not ours"
        );
    }

    #[test]
    fn saying_yes_buffers_the_sale_and_lets_the_inspector_go() {
        let mut app = App::new();
        app.init_resource::<CommandBuffer>()
            .init_resource::<Selection>()
            .add_message::<ConfirmAccepted>()
            .add_systems(Update, apply_confirmed_sell);
        app.world_mut()
            .resource_mut::<Selection>()
            .set(Selectable::Train(TrainId(3)));

        app.update();
        assert!(app.world().resource::<CommandBuffer>().pending().is_empty());

        app.world_mut()
            .write_message(ConfirmAccepted(ConfirmAction::SellTrain(TrainId(3))));
        app.update();

        let pending = app.world().resource::<CommandBuffer>().pending();
        assert_eq!(pending.len(), 1, "one command per agreement");
        assert!(
            matches!(pending[0].kind, CommandKind::SellTrain(s) if s.train == TrainId(3)),
            "the dialog's yes becomes the command, on the tick boundary"
        );
        assert!(
            app.world().resource::<Selection>().0.is_none(),
            "the Inspector does not sit open on a train that is leaving"
        );
    }

    #[test]
    fn a_station_demolish_agreement_is_not_a_train_sale() {
        let mut app = App::new();
        app.init_resource::<CommandBuffer>()
            .init_resource::<Selection>()
            .add_message::<ConfirmAccepted>()
            .add_systems(Update, apply_confirmed_sell);

        app.world_mut()
            .write_message(ConfirmAccepted(ConfirmAction::DemolishStation(
                rail_sim::StationId(1),
            )));
        app.update();

        assert!(app.world().resource::<CommandBuffer>().pending().is_empty());
    }
}
