//! A working railway behind the title screen.
//!
//! Design [`09 §2`](../../../docs/design/09-shell-and-menus.md): the background
//! is *"the game running quietly — trains moving on a small pre-built network"*.
//! What actually shipped was a landscape with two or three station markers on it
//! and no rail at all, which sells the game as a terrain generator. The whole
//! first impression of a railway game is a train crossing the frame.
//!
//! # What it does
//!
//! Once, at boot, it lays one short line between the two nearest seeded stations
//! and puts a single transit train on it. The train then does what any free-roam
//! transit train does — the point is that it *moves*.
//!
//! Everything goes through [`CommandBuffer`]. Not a shortcut: track placement
//! validates grades, bridge spans, terrain and funds, and a background world
//! that bypassed that would be the one world in the game built by different
//! rules. It also means the demo cannot lay anything the player could not.
//!
//! A pathological map — two anchors with a mountain between them, an
//! unbridgeable strait — simply gets no demo. The title screen is still a
//! landscape, which is what it was before, and nothing about it is worth an
//! error.
//!
//! # It is a demo, not a head start
//!
//! The boot world is pre-player: nothing that happens on the title screen is the
//! player's, and none of it may follow them into a game. [`release_boot_world`]
//! is the seam — the **first** entry into `Playing`, whether that came from
//! Begin, Load or Continue, resets the treasury to the world's configured
//! bracket, empties the ledger and the undo stack, and zeroes goal progress.
//!
//! That is the cheaper half of the two options the audit offered. Crediting the
//! demo's cost back through the ledger would leave the construction category
//! non-zero, and it would do nothing at all about the other half: a transit
//! train running for as long as the player leaves the menu up banks fares, and
//! that income would consume `onboarding::payout`'s once-per-world first-payout
//! moment and dirty goal progress before the first frame of play. Resetting is
//! one place, and it is *provably* everything. A load lands during `Update`,
//! after this `OnEnter` reset, so a restored save still wins.
//!
//! The reset latches, so resuming from Pause — also an entry into `Playing` —
//! never touches the player's money.

use bevy::prelude::*;
use rail_sim::commands::{AutoFillTrack, BuyTrain, PlaceTrain, TrainKind};
use rail_sim::{
    find_path, track_for_station, CommandBuffer, CommandHistory, CommandKind, GoalBoard,
    GoalStatus, Money, MoneyLedger, Station, StationRegistry, TileCoord, TrackNetwork,
    TrackTerrain, TrainYard, GROUND_LAYER,
};

use super::{PendingWorld, ShellState};

/// How many candidate pairs the demo will try before giving up.
///
/// The nearest pair is almost always the right answer; the retries exist for
/// the map where it happens to be across a ravine. Three is enough to be robust
/// and small enough that a hopeless map stops asking.
const MAX_ATTEMPTS: u8 = 3;

/// Frames to wait for a pushed command to reach the network.
///
/// Commands drain on `FixedUpdate`, which does not run on every `Update`, so
/// this is a generous count of *frames* rather than a tick budget.
const SETTLE_FRAMES: u8 = 30;

/// Where the title screen's demo railway has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Stage {
    /// Waiting for the world: terrain, anchors, an empty network.
    #[default]
    Waiting,
    /// Autofill commands pushed; waiting to see whether they landed.
    Laying,
    /// The line is up and a train has been bought; waiting for the yard.
    Buying,
    /// Nothing left to do — laid, or given up on.
    Done,
}

/// The title screen's demo line, and whether the world is still the demo's.
#[derive(Resource, Debug, Default)]
pub struct BootDemo {
    stage: Stage,
    /// Candidate station pairs, nearest first, still to try.
    candidates: Vec<(TileCoord, TileCoord)>,
    attempts: u8,
    frames: u8,
    /// Set once the player takes the world over. Latches, so Pause → Playing
    /// never resets anything.
    released: bool,
}

impl BootDemo {
    /// `true` once a real session owns the world.
    pub fn is_released(&self) -> bool {
        self.released
    }
}

/// Lay the demo line, then put a train on it. Boot only, title only.
///
/// Every sim resource is optional. The shell is buildable without `SimPlugin` —
/// its own tests run that way — and a decorative background must never be the
/// thing that makes the menu refuse to come up.
pub fn lay_boot_demo_line(
    mut demo: ResMut<BootDemo>,
    stations: Option<Res<StationRegistry>>,
    network: Option<Res<TrackNetwork>>,
    terrain: Option<Res<TrackTerrain>>,
    yard: Option<Res<TrainYard>>,
    mut buffer: ResMut<CommandBuffer>,
) {
    if demo.released || demo.stage == Stage::Done {
        return;
    }
    // No terrain means no world yet; nothing can be validated against it.
    let (Some(stations), Some(network), Some(yard), true) = (
        stations.as_deref(),
        network.as_deref(),
        yard.as_deref(),
        terrain.is_some(),
    ) else {
        return;
    };

    match demo.stage {
        Stage::Waiting => {
            if stations.len() < 2 {
                return; // Anchors have not been seeded yet.
            }
            if !network.is_empty() {
                // Somebody else built here. Not our world to decorate.
                demo.stage = Stage::Done;
                return;
            }
            if demo.candidates.is_empty() {
                demo.candidates = closest_pairs(stations);
            }
            let Some((from, to)) = demo.candidates.first().copied() else {
                demo.stage = Stage::Done;
                return;
            };
            demo.candidates.remove(0);
            demo.attempts += 1;
            for (a, b) in elbow(from, to) {
                buffer.push(CommandKind::AutoFillTrack(AutoFillTrack {
                    from: a,
                    to: b,
                    layer: GROUND_LAYER,
                }));
            }
            demo.frames = 0;
            demo.stage = Stage::Laying;
        }

        Stage::Laying => {
            demo.frames = demo.frames.saturating_add(1);
            if stations_are_linked(stations, network) {
                buffer.push(CommandKind::BuyTrain(BuyTrain {
                    kind: TrainKind::Transit,
                }));
                demo.frames = 0;
                demo.stage = Stage::Buying;
                return;
            }
            if demo.frames < SETTLE_FRAMES {
                return;
            }
            // That pair could not be joined. Try the next, or leave the title
            // screen as the landscape it was.
            demo.stage = if demo.attempts < MAX_ATTEMPTS && !demo.candidates.is_empty() {
                Stage::Waiting
            } else {
                Stage::Done
            };
        }

        Stage::Buying => {
            demo.frames = demo.frames.saturating_add(1);
            if let Some(train) = yard.peek_kind(TrainKind::Transit) {
                let at_station = linked_station(stations, network)
                    .or_else(|| stations.iter().map(|s| s.id).next());
                if let Some(at_station) = at_station {
                    buffer.push(CommandKind::PlaceTrain(PlaceTrain { train, at_station }));
                }
                demo.stage = Stage::Done;
            } else if demo.frames >= SETTLE_FRAMES {
                // The purchase never landed (no funds, no yard). Leave the line.
                demo.stage = Stage::Done;
            }
        }

        Stage::Done => {}
    }
}

/// The world stops being a demo the first time a real session starts.
///
/// Runs on `OnEnter(Playing)`, which is also the transition out of Pause — hence
/// the latch. See the module docs for why this resets rather than refunds.
pub fn release_boot_world(
    mut demo: ResMut<BootDemo>,
    pending: Res<PendingWorld>,
    mut goals: Option<ResMut<GoalBoard>>,
    mut commands: Commands,
) {
    if demo.released {
        return;
    }
    demo.released = true;
    demo.stage = Stage::Done;
    commands.insert_resource(Money::new(pending.options.cash.cents()));
    commands.insert_resource(MoneyLedger::default());
    // The demo's track is on the undo stack; the player never built it.
    commands.insert_resource(CommandHistory::default());
    // Nothing the demo train delivered counts toward the player's objectives.
    // Progress is zeroed **in place** rather than by installing a fresh board:
    // the set may already have been derived against this world's anchors, and
    // throwing it away would hand the player a different set of objectives from
    // the one the world was generated for.
    if let Some(board) = goals.as_deref_mut() {
        for goal in board.iter_mut() {
            goal.current = 0;
            goal.status = GoalStatus::Active;
            goal.resolved_tick = 0;
            goal.warned = false;
        }
    }
}

/// Station pairs, nearest first.
///
/// Sorted on `(distance, ids)` so the pair a given world picks never depends on
/// the registry's `HashMap` order — a title screen that laid its line somewhere
/// different on every launch of the same seed would be a bug, not a feature.
fn closest_pairs(stations: &StationRegistry) -> Vec<(TileCoord, TileCoord)> {
    let mut all: Vec<&Station> = stations.iter().collect();
    all.sort_by_key(|s| s.id.0);
    let mut pairs: Vec<(i32, u64, u64, TileCoord, TileCoord)> = Vec::new();
    for (i, a) in all.iter().enumerate() {
        for b in all.iter().skip(i + 1) {
            pairs.push((chebyshev(a.tile, b.tile), a.id.0, b.id.0, a.tile, b.tile));
        }
    }
    pairs.sort_by_key(|(d, ai, bi, _, _)| (*d, *ai, *bi));
    pairs.into_iter().map(|(_, _, _, a, b)| (a, b)).collect()
}

fn chebyshev(a: TileCoord, b: TileCoord) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

/// Two straight runs joining `from` to `to`: a 45° leg, then an orthogonal one.
///
/// Autofill only lays along one of the sixteen directions, so an arbitrary pair
/// of anchors needs a corner. Diagonal-first is the shape the game's own track
/// art is built for and the shape a player would draw: it keeps the run short
/// and puts the single bend in the middle rather than at a station throat.
///
/// Zero-length legs are dropped, so two anchors that already line up get one
/// straight run and no corner at all.
fn elbow(from: TileCoord, to: TileCoord) -> Vec<(TileCoord, TileCoord)> {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let diagonal = dx.abs().min(dy.abs());
    let corner = TileCoord {
        x: from.x + dx.signum() * diagonal,
        y: from.y + dy.signum() * diagonal,
    };
    [(from, corner), (corner, to)]
        .into_iter()
        .filter(|(a, b)| a != b)
        .collect()
}

/// A station whose platform has track that reaches another station's.
fn linked_station(
    stations: &StationRegistry,
    network: &TrackNetwork,
) -> Option<rail_sim::StationId> {
    let mut all: Vec<&Station> = stations.iter().collect();
    all.sort_by_key(|s| s.id.0);
    for (i, a) in all.iter().enumerate() {
        for b in all.iter().skip(i + 1) {
            let (Some(from), Some(to)) = (
                track_for_station(network, a.tile, a.layer),
                track_for_station(network, b.tile, b.layer),
            ) else {
                continue;
            };
            if find_path(network, from, to).is_some() {
                return Some(a.id);
            }
        }
    }
    None
}

fn stations_are_linked(stations: &StationRegistry, network: &TrackNetwork) -> bool {
    linked_station(stations, network).is_some()
}

/// Run condition: the demo only ever builds on the world it booted into.
pub fn boot_world_is_untouched(demo: Res<BootDemo>, state: Res<State<ShellState>>) -> bool {
    !demo.released && *state.get() == ShellState::Title
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    #[test]
    fn the_elbow_is_two_runs_the_autofill_can_actually_lay() {
        use rail_sim::straight_line;

        for (from, to) in [
            (tile(4, 4), tile(12, 9)),
            (tile(12, 9), tile(4, 4)),
            (tile(4, 4), tile(4, 11)),
            (tile(4, 4), tile(11, 4)),
            (tile(9, 9), tile(2, 2)),
        ] {
            let legs = elbow(from, to);
            assert!(!legs.is_empty(), "{from:?} -> {to:?} produced no run");
            for (a, b) in &legs {
                assert!(
                    straight_line(*a, *b).is_some(),
                    "{a:?} -> {b:?} is off all sixteen directions"
                );
            }
            // The legs join up, end to end, from one anchor to the other.
            assert_eq!(legs.first().unwrap().0, from);
            assert_eq!(legs.last().unwrap().1, to);
            for pair in legs.windows(2) {
                assert_eq!(pair[0].1, pair[1].0, "the legs do not meet");
            }
        }
    }

    #[test]
    fn two_anchors_in_line_need_no_corner() {
        assert_eq!(elbow(tile(2, 2), tile(9, 2)).len(), 1);
        assert_eq!(elbow(tile(2, 2), tile(2, 9)).len(), 1);
        assert_eq!(elbow(tile(2, 2), tile(9, 9)).len(), 1, "a clean diagonal");
        assert!(elbow(tile(2, 2), tile(2, 2)).is_empty());
    }

    /// The whole feature against a real (headless) shell and sim.
    ///
    /// Design 09 §2 is a statement about what the player *sees* at boot, so the
    /// only test worth having drives the real plugins and looks at the world.
    mod app {
        use super::*;
        use crate::shell::{
            map_options::MapOptions, BootSeed, DraftMapOptions, MenuAction, MenuActivated, Settings,
            ShellPlugin, StartingCash,
        };
        use bevy::asset::AssetPlugin;
        use bevy::state::app::StatesPlugin;
        use rail_sim::{SimPlugin, Train};

        /// The shell, the sim, and a flat, dry world for the anchors to sit on.
        ///
        /// Terrain is generated flat rather than copied from `MapGrid` because
        /// this test is about the demo running, not about which seeds happen to
        /// have a mountain in the way — the pathological map is covered by
        /// `elbow` and by the attempt budget.
        fn boot_app() -> App {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                StatesPlugin,
                bevy::input::InputPlugin,
                AssetPlugin::default(),
            ))
            .init_asset::<Image>()
            .init_resource::<UiScale>()
            .add_plugins(SimPlugin)
            .add_plugins(ShellPlugin {
                boot_seed: BootSeed::Fixed(42),
                suppress_world_input: false,
            })
            .insert_resource(Settings::default())
            .add_systems(bevy::app::Startup, install_flat_terrain);
            app
        }

        fn install_flat_terrain(mut commands: Commands, map: Res<rail_map::MapGrid>) {
            let cells = (0..map.width * map.height).map(|_| (false, 0i8));
            commands.insert_resource(TrackTerrain::new(map.width, map.height, cells));
            commands.insert_resource(rail_sim::AnchorSites(map.anchor_hints()));
        }

        /// One frame, plus the fixed tick the command buffer drains on.
        fn step(app: &mut App, frames: usize) {
            for _ in 0..frames {
                app.update();
                app.world_mut().run_schedule(bevy::app::FixedUpdate);
            }
        }

        fn trains(app: &mut App) -> usize {
            app.world_mut().query::<&Train>().iter(app.world()).count()
        }

        #[test]
        fn booting_to_the_title_leaves_a_line_with_a_train_on_it() {
            let mut app = boot_app();
            step(&mut app, 40);

            assert_eq!(
                *app.world().resource::<State<ShellState>>().get(),
                ShellState::Title
            );
            let stations = app.world().resource::<StationRegistry>();
            let network = app.world().resource::<TrackNetwork>();
            assert!(stations.len() >= 2, "the world seeded no anchors to join");
            assert!(!network.is_empty(), "the title screen has no railway on it");
            assert!(
                linked_station(stations, network).is_some(),
                "the demo line does not actually join two stations"
            );
            assert_eq!(trains(&mut app), 1, "design 09 §2 wants a train moving");
        }

        #[test]
        fn beginning_a_new_map_starts_trackless_with_the_players_own_money() {
            let mut app = boot_app();
            step(&mut app, 40);
            assert!(!app.world().resource::<TrackNetwork>().is_empty());

            // The demo spent real money through the real command path.
            let options = MapOptions {
                seed: 777,
                cash: StartingCash::Generous,
                ..MapOptions::default()
            };
            app.world_mut()
                .resource_mut::<NextState<ShellState>>()
                .set(ShellState::NewMap);
            step(&mut app, 1);
            app.world_mut().resource_mut::<DraftMapOptions>().0 = options;
            app.world_mut()
                .write_message(MenuActivated(MenuAction::Begin));
            step(&mut app, 3);

            assert_eq!(
                *app.world().resource::<State<ShellState>>().get(),
                ShellState::Playing
            );
            assert!(
                app.world().resource::<TrackNetwork>().is_empty(),
                "the player's world inherited the title screen's railway"
            );
            assert_eq!(trains(&mut app), 0, "and its rolling stock");
            // The player's world starts billing its own seeded stations from
            // the first tick, so the balance is the configured cash minus
            // exactly what the ledger says was kept up — and not one cent of
            // construction, which is what would mean the demo charged them.
            let ledger = app.world().resource::<MoneyLedger>();
            assert_eq!(
                ledger.total(rail_sim::MoneyCategory::Construction),
                0,
                "the demo charged the player for track they never laid"
            );
            assert_eq!(
                app.world().resource::<Money>().cents(),
                options.cash.cents() + ledger.session_income() - ledger.session_expense(),
                "the balance moved by something the ledger never saw"
            );
            assert_eq!(
                ledger.session_income(),
                0,
                "the demo train's fares would have consumed the first payout"
            );
            let board = app.world().resource::<GoalBoard>();
            assert!(
                board.iter().all(|g| g.current == 0),
                "the demo dirtied goal progress"
            );
            assert!(
                app.world().resource::<BootDemo>().is_released(),
                "the world is the player's now"
            );
        }

        #[test]
        fn resuming_from_pause_never_resets_the_players_money() {
            // `release_boot_world` runs on `OnEnter(Playing)`, and so does
            // leaving the pause menu. The latch is the whole safety.
            let mut app = boot_app();
            step(&mut app, 40);
            app.world_mut()
                .resource_mut::<NextState<ShellState>>()
                .set(ShellState::Playing);
            step(&mut app, 2);

            let spent = 12_345;
            app.world_mut().insert_resource(Money::new(spent));
            app.world_mut()
                .resource_mut::<NextState<ShellState>>()
                .set(ShellState::Paused);
            step(&mut app, 2);
            app.world_mut()
                .resource_mut::<NextState<ShellState>>()
                .set(ShellState::Playing);
            step(&mut app, 2);

            // Opex nibbles a cent or two while the sim runs; what must not
            // happen is the balance jumping back to a starting bracket.
            let now = app.world().resource::<Money>().cents();
            assert!(
                (now - spent).abs() < 100,
                "unpausing refilled the treasury: {spent} -> {now}"
            );
        }
    }

    #[test]
    fn the_pair_is_the_nearest_one_and_never_depends_on_iteration_order() {
        let mut stations = StationRegistry::new();
        stations.insert("Far", tile(40, 40), GROUND_LAYER);
        stations.insert("Home", tile(10, 10), GROUND_LAYER);
        stations.insert("Near", tile(16, 12), GROUND_LAYER);

        let first = closest_pairs(&stations);
        assert_eq!(first.first().copied(), Some((tile(10, 10), tile(16, 12))));
        assert_eq!(first.len(), 3, "every pair is a candidate");
        for _ in 0..16 {
            assert_eq!(closest_pairs(&stations), first, "ordering is not stable");
        }
    }
}
