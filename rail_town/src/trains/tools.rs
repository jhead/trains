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

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{world_to_tile, MapGrid};
use rail_sim::commands::{BuyTrain, PlaceTrain, SellTrain};
use rail_sim::{
    buy_cost, track_for_station, CommandBuffer, CommandKind, StationRegistry, TrainKind, TrainYard,
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

/// The placement a world click at `tile` should produce, if any.
///
/// A click counts for a station when it lands on the station tile, on the track
/// that serves it, or on any tile touching it — the same generosity the station
/// tool gives, because a platform is smaller than a finger. Placement needs a
/// train of that kind in the yard and rails at the stop; without either, the
/// click is simply not a placement.
fn place_at_tile(
    tile: rail_sim::TileCoord,
    kind: TrainKind,
    yard: &TrainYard,
    stations: &StationRegistry,
    network: &TrackNetwork,
) -> Option<PlaceTrain> {
    let train = yard.peek_kind(kind)?;

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
        })?;

    let station = stations.get(at_station)?;
    track_for_station(network, station.tile, station.layer)?;
    Some(PlaceTrain { train, at_station })
}

#[allow(clippy::too_many_arguments)]
pub fn train_tool_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MapCamera>>,
    map: Res<MapGrid>,
    stations: Res<StationRegistry>,
    network: Res<TrackNetwork>,
    yard: Res<TrainYard>,
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
            &yard,
            &mut buffer,
            &mut train_state,
            &mut track_state,
            &mut line_state,
        );
    }
    if bindings.just_pressed(&keys, ControlAction::BuyTransport) {
        arm_train_place(
            TrainPlaceKind::Transport,
            &yard,
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
    let Ok(world) = camera.viewport_to_world_2d(cam_transform, cursor) else {
        return;
    };
    let tile = world_to_tile(world.x, world.y);
    if !map.contains(tile) {
        return;
    }

    if let Some(place) = place_at_tile(
        tile,
        train_state.kind.to_sim(),
        &yard,
        &stations,
        &network,
    ) {
        buffer.push(CommandKind::PlaceTrain(place));
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
        let mut yard = TrainYard::default();
        yard.buy(TrainKind::Transit);
        (yard, stations, network, station)
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
        let (yard, stations, network, station) = world();
        let place = place_at_tile(
            TileCoord { x: 3, y: 4 },
            TrainKind::Transit,
            &yard,
            &stations,
            &network,
        )
        .expect("a station under the click and a train in the yard");
        assert_eq!(place.at_station, station);
        assert_eq!(Some(place.train), yard.peek_kind(TrainKind::Transit));
    }

    #[test]
    fn a_click_next_to_the_platform_still_places() {
        let (yard, stations, network, station) = world();
        let place = place_at_tile(
            TileCoord { x: 4, y: 5 },
            TrainKind::Transit,
            &yard,
            &stations,
            &network,
        )
        .expect("a platform is smaller than a finger");
        assert_eq!(place.at_station, station);
    }

    #[test]
    fn a_click_on_open_ground_places_nothing() {
        let (yard, stations, network, _) = world();
        assert!(place_at_tile(
            TileCoord { x: 12, y: 12 },
            TrainKind::Transit,
            &yard,
            &stations,
            &network,
        )
        .is_none());
    }

    #[test]
    fn an_empty_yard_places_nothing() {
        let (_, stations, network, _) = world();
        assert!(place_at_tile(
            TileCoord { x: 3, y: 4 },
            TrainKind::Transit,
            &TrainYard::default(),
            &stations,
            &network,
        )
        .is_none());
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
