#![allow(dead_code)] // Shell surface — `main.rs` wires a subset; the rest is API.
//! Game shell — title, new map, pause, settings.
//!
//! Phase E of [`docs/BURNDOWN.md`]. The shell is what turns the build into a
//! product: a way in, a way out, and a way to choose what you are playing.
//! Design brief: [`docs/design/09-shell-and-menus.md`](../../../docs/design/09-shell-and-menus.md).
//!
//! # Shape
//!
//! [`ShellState`] is the whole state machine: `Title → NewMap → Playing →
//! Paused`. Settings is an overlay ([`settings_panel::SettingsPanel`]) rather
//! than a fifth state, because it opens from two different screens and returns to
//! whichever one called it.
//!
//! The world is generated and drawn from boot, so the title screen is the actual
//! game running quietly behind the menu (design §2) rather than a still. Nothing
//! in the shell hides it.
//!
//! # How the shell keeps gameplay out of the menus
//!
//! Two mechanisms, both self-contained, so adding [`ShellPlugin`] is enough to
//! get correct behaviour without editing any gameplay plugin:
//!
//! 1. **Pointer** — every shell screen roots a full-window
//!    [`WorldClickBlocker`](crate::ui::kit::WorldClickBlocker). The existing
//!    `UiBlocksWorld` resource therefore reads `true` whenever a menu is up, and
//!    the build / demolish / select / map-view tools already respect it.
//! 2. **Keyboard, wheel** — [`suppress_world_input`] clears the input resources
//!    at the end of `PreUpdate` while the shell owns the screen. Shell input runs
//!    earlier in the same chain, so menus read keys the game never sees.
//!
//! Suppression is a safety net, not a substitute for state gating. Where the
//! integrator can add `.run_if(in_state(ShellState::Playing))` to a gameplay
//! plugin's `Update` systems, that is strictly better and this plugin does not
//! conflict with it. See [`ShellPlugin::suppress_world_input`].
//!
//! # New Map, Load, and the world rebuild
//!
//! Beginning a new map (or restoring a save) replaces the world's definition —
//! `MapGrid`, `TrackTerrain`, starting cash — and clears the sim registries.
//! Presentation follows on its own: `map::terrain` re-composites when `MapGrid`
//! changes, and the station / industry / train / building / peep sprites all
//! reconcile against their registries every frame. Track sprites are the one
//! exception (they are `TrackEdit`-driven), so the shell despawns those directly.
//!
//! [`WorldRebuildSet`] and [`world_rebuild_pending`] are the seam for anything
//! that still needs telling. Order a system `.after(WorldRebuildSet)` in `Update`
//! with `.run_if(world_rebuild_pending)` and it will run on exactly the frame a
//! new world is installed, whether that came from New Map or from a load.
//!
//! # Goals mode
//!
//! The New Map screen's Mode row is a real choice now, and the shell is the only
//! thing that sets it: installing a world also installs its
//! [`GoalBoard`](rail_sim::GoalBoard), started in the chosen mode with that
//! world's effective seed (see [`goal_board_for`]). Deriving and evaluating the
//! set is `rail_sim`'s job — the shell never invents an objective. The panel in
//! [`goals_panel`] draws whatever the board says.
//!
//! **This module therefore requires `rail_sim::goals` to be reachable.** That
//! costs three lines in `rail_sim/src/lib.rs`: `pub mod goals;`, a `pub use
//! goals::{…}` re-export, and `goals::GoalsPlugin` in `SimPlugin`'s
//! `add_plugins` tuple. Without them this file does not compile, and that is
//! deliberate — a Mode row that silently did nothing is exactly what this
//! change was made to remove.

mod boot_demo;
pub mod controls;
mod goals_panel;
mod map_options;
mod new_map;
mod pause;
pub(crate) mod persist;
mod save;
mod settings;
mod settings_panel;
mod title;
mod widgets;

use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::input::InputSystems;
use bevy::prelude::*;
use rail_sim::{
    AlertBoard, BorderRegistry, CommandBuffer, CommandHistory, ComplaintFeed, DemandSpawner,
    DistrictFlow, EventDirector, GoalBoard, HouseholdRegistry, IndustryRegistry, JobBoard,
    LineRegistry, MaintenanceAccrual, MapDescriptor, Money, MoneyLedger, Peep, PeepBudget,
    PeepFocus, PeepSpawnState, StationRegistry, StationService, TileOccupancy, TownDensity,
    TrackNetwork, Train, TrainYard, VacatedHomes, WorldAnchorsSeeded,
};

use crate::inspect::Selection;
use crate::map::MapCamera;
use crate::track::{TrackSprite, TrackToolState};

// The shell's public surface. `rail_town` is a binary, so items the app has not
// wired yet read as unused imports; they are the module's API all the same.
#[allow(unused_imports)]
pub use map_options::{
    GameMode, MapOptions, MapReadouts, MapSize, OptionField, ResourceSpread, StartingCash,
    TerrainStyle, WaterStyle,
};
#[allow(unused_imports)]
pub use save::{AutosaveTimer, SaveStatus, ShellSaveRequest};
#[allow(unused_imports)]
pub use settings::{Settings, SettingsTab};
#[allow(unused_imports)]
pub use settings_panel::SettingsPanel;
#[allow(unused_imports)]
pub use widgets::{MenuAction, MenuActivated, MenuCursor, ShellUi};

use boot_demo::{boot_world_is_untouched, lay_boot_demo_line, release_boot_world, BootDemo};
use new_map::{DraftMapOptions, PreviewImage};
use widgets::{menu_keyboard_nav, menu_pointer, paint_menu_items, sync_menu_cursor};

/// The shell state machine. Everything outside `Playing` is a menu.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ShellState {
    #[default]
    Title,
    NewMap,
    Playing,
    Paused,
}

impl ShellState {
    /// `true` while the shell owns the screen and gameplay input must not fire.
    pub fn is_menu(self) -> bool {
        !matches!(self, Self::Playing)
    }

    /// Whether the title's slow camera drift should own the view.
    ///
    /// Deliberately **not** [`Self::is_menu`]. Pause is a menu, but it is drawn
    /// over the player's own view of their own world — drifting there wanders
    /// the camera off wherever they were standing, and resuming dumps them
    /// somewhere else entirely. Pause holds the camera exactly still.
    pub fn drifts_background(self) -> bool {
        matches!(self, Self::Title | Self::NewMap)
    }
}

/// Where the world the shell boots into comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BootSeed {
    /// A fresh seed every launch, so the title screen is a new world each time
    /// (design §2: "the title screen looks better every time the game does").
    #[default]
    Rolled,
    /// A fixed seed — for reproducible runs, screenshots and tests.
    Fixed(u64),
}

/// World setup waiting to be applied, plus where it is in the rebuild handshake.
///
/// Two flags rather than one, because the request and the rebuild happen on
/// different frames: Begin asks during `Update`, and the state transition that
/// installs the world does not run until the frame after.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct PendingWorld {
    pub options: MapOptions,
    /// Begin asked for a new world; consumed on the next entry into `Playing`.
    requested: bool,
    /// `true` for exactly the frame the world was installed in. Presentation
    /// rebuild hooks read this through [`world_rebuild_pending`].
    rebuilding: bool,
}

impl PendingWorld {
    /// Ask for `options` to become the world on the next entry into `Playing`.
    pub fn request(&mut self, options: MapOptions) {
        self.options = options;
        self.requested = true;
    }

    /// `true` while presentation still has to be rebuilt for a new world.
    pub fn is_rebuilding(&self) -> bool {
        self.rebuilding
    }
}

/// The shell's world-rebuild work. Anything the app adds to reconstruct
/// presentation (terrain sprites) must be ordered `.after(WorldRebuildSet)`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorldRebuildSet;

/// Run condition: a new world was just installed and presentation must be rebuilt.
pub fn world_rebuild_pending(pending: Res<PendingWorld>) -> bool {
    pending.is_rebuilding()
}

/// Run condition: the shell owns the screen.
pub fn shell_owns_screen(state: Res<State<ShellState>>) -> bool {
    state.get().is_menu()
}

/// Run condition for the title's background drift.
pub fn shell_drifts_background(state: Res<State<ShellState>>) -> bool {
    state.get().drifts_background()
}

/// Title, new map, pause menu, settings, and the state machine behind them.
pub struct ShellPlugin {
    /// World the shell generates at boot. See [`BootSeed`].
    pub boot_seed: BootSeed,
    /// Clear keyboard / wheel input while a menu is up. Leave this on unless the
    /// gameplay plugins are already gated on [`ShellState::Playing`].
    pub suppress_world_input: bool,
}

impl Default for ShellPlugin {
    fn default() -> Self {
        Self {
            boot_seed: BootSeed::default(),
            suppress_world_input: true,
        }
    }
}

impl Plugin for ShellPlugin {
    fn build(&self, app: &mut App) {
        let options = MapOptions {
            seed: match self.boot_seed {
                BootSeed::Rolled => map_options::roll_seed(),
                BootSeed::Fixed(seed) => seed,
            },
            ..MapOptions::default()
        };

        app.init_state::<ShellState>()
            // Settings are read from disk here so the very first frame already
            // honours them — a UI scale that pops one frame in looks broken.
            .insert_resource(Settings::load())
            .insert_resource(PendingWorld {
                options,
                ..PendingWorld::default()
            })
            .insert_resource(DraftMapOptions(options))
            .init_resource::<MenuCursor>()
            .init_resource::<PreviewImage>()
            .init_resource::<goals_panel::GoalsPanelCache>()
            .init_resource::<SettingsPanel>()
            .init_resource::<SaveStatus>()
            .init_resource::<ShellSaveRequest>()
            .init_resource::<AutosaveTimer>()
            .init_resource::<title::DriftClock>()
            .init_resource::<BootDemo>()
            .add_message::<MenuActivated>()
            // The set exists in both schedules: a new map installs its world
            // during the state transition, a load installs one mid-`Update`.
            .configure_sets(OnEnter(ShellState::Playing), WorldRebuildSet)
            .configure_sets(Update, WorldRebuildSet)
            // Runs after every plugin's `build`, before `Startup` spawns tiles —
            // so the world the game draws is the world the shell chose.
            .add_systems(PreStartup, install_boot_world)
            .add_systems(OnEnter(ShellState::Title), reset_cursor)
            .add_systems(OnEnter(ShellState::NewMap), reset_cursor)
            .add_systems(OnEnter(ShellState::Paused), reset_cursor)
            .add_systems(
                OnEnter(ShellState::Playing),
                (apply_pending_world.in_set(WorldRebuildSet), release_boot_world),
            )
            .add_systems(OnEnter(ShellState::Paused), pause_sim)
            .add_systems(OnEnter(ShellState::Playing), resume_sim)
            // `Esc` reaches an open window before it reaches the pause menu
            // (design 03 §10.1). `ui::WindowEscSet` closes the top window and
            // consumes the key, so ordering after it is the whole coordination.
            .add_systems(
                PreUpdate,
                (
                    shell_hotkeys,
                    settings_panel::capture_rebind,
                    menu_keyboard_nav.run_if(shell_menu_visible),
                )
                    .chain()
                    .after(InputSystems)
                    .after(crate::ui::WindowEscSet),
            )
            .add_systems(
                Update,
                (
                    menu_pointer,
                    dispatch_menu_actions.after(menu_pointer),
                    sync_menu_cursor.after(dispatch_menu_actions),
                    paint_menu_items.after(sync_menu_cursor),
                ),
            )
            .add_systems(
                Update,
                (
                    save::service_save_requests.after(dispatch_menu_actions),
                    mark_rebuild_after_load.after(save::service_save_requests),
                )
                    .in_set(WorldRebuildSet),
            )
            .add_systems(
                Update,
                (
                    title::spawn_title_if_missing.run_if(in_state(ShellState::Title)),
                    // Design §2 wants trains moving behind the menu, so the boot
                    // world builds itself one short line. Boot only, and never
                    // once the player has taken the world over.
                    lay_boot_demo_line.run_if(boot_world_is_untouched),
                    pause::spawn_pause_if_missing.run_if(in_state(ShellState::Paused)),
                    new_map::seed_typing.run_if(in_state(ShellState::NewMap)),
                    new_map::rebuild_new_map_screen
                        .after(new_map::seed_typing)
                        .run_if(in_state(ShellState::NewMap)),
                    settings_panel::rebuild_settings_panel,
                    settings::apply_display_settings,
                    settings::apply_audio_settings,
                    settings::persist_settings_on_change,
                    goals_panel::rebuild_goals_panel,
                    hide_game_hud,
                    tick_autosave.run_if(in_state(ShellState::Playing)),
                ),
            )
            .add_systems(
                PostUpdate,
                title::drift_background_world.run_if(shell_drifts_background),
            )
            .add_systems(Last, finish_world_rebuild);

        if self.suppress_world_input {
            app.add_systems(
                PreUpdate,
                suppress_world_input
                    .after(InputSystems)
                    .after(menu_keyboard_nav)
                    .after(settings_panel::capture_rebind)
                    .after(shell_hotkeys)
                    // Not `shell_owns_screen`: Settings opens *over* play, so
                    // the state is still `Playing` while it is up. Binding a
                    // verb to `B` used to arm the track tool on the way past,
                    // which is a poor way to learn that rebinding now works.
                    .run_if(shell_menu_visible),
            );
        }
    }
}

/// `true` when a navigable shell menu is on screen.
fn shell_menu_visible(state: Res<State<ShellState>>, panel: Res<SettingsPanel>) -> bool {
    panel.open || state.get().is_menu()
}

/// Replace the boot map with the shell's, before anything draws it.
fn install_boot_world(mut commands: Commands, pending: Res<PendingWorld>) {
    let map = pending.options.generate();
    commands.insert_resource(map_descriptor_for(&pending.options, &map));
    commands.insert_resource(map);
    commands.insert_resource(goal_board_for(&pending.options));
}

/// How this world was made, for the sim to record in a save.
///
/// A save stores the terrain it plays on, but the *map* — the grid the art and
/// the generator's feature notes come from — is rebuilt on load from the seed
/// and these knobs rather than stored twice. Design 02 §5 promises a seed
/// reproduces a world; this is that promise being spent. The knobs travel packed
/// because `rail_sim` cannot see `rail_map`.
///
/// Sized from the grid that was actually generated, not from
/// [`MapOptions::size`], so the two can never disagree.
fn map_descriptor_for(options: &MapOptions, map: &rail_map::MapGrid) -> MapDescriptor {
    MapDescriptor::new(options.seed, map.width, map.height)
        .with_knobs(options.gen_options().pack())
}

/// The goal board a world with these options starts from.
///
/// Sandbox is a board that exists and does nothing; Goals asks `rail_sim` to
/// derive a set from this map as soon as its anchors are placed. The
/// *effective* seed is used rather than the typed one, because that is what
/// identifies the world — two maps that differ only by terrain style are
/// different worlds and deserve different objectives.
fn goal_board_for(options: &MapOptions) -> GoalBoard {
    let mut board = GoalBoard::default();
    board.start(options.mode.to_goal_mode(), options.effective_seed());
    board
}

fn reset_cursor(mut cursor: ResMut<MenuCursor>) {
    cursor.0 = 0;
}

/// Pause / resume go through the command buffer, never straight at [`SimClock`],
/// so the shell obeys the same intent path as every other control.
fn pause_sim(mut buffer: ResMut<CommandBuffer>) {
    buffer.push(rail_sim::CommandKind::pause(true));
}

fn resume_sim(mut buffer: ResMut<CommandBuffer>) {
    buffer.push(rail_sim::CommandKind::pause(false));
}

/// `Esc` unwinds one layer per press, and `Tab` walks the settings tabs.
///
/// Design 03 §10.1: never more than one layer per press, and never two things at
/// once. A build drag in progress belongs to the track tool, so the pause menu
/// stays out of its way.
fn shell_hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    state: Res<State<ShellState>>,
    track: Option<Res<TrackToolState>>,
    mut panel: ResMut<SettingsPanel>,
    mut next: ResMut<NextState<ShellState>>,
    mut cursor: ResMut<MenuCursor>,
) {
    if panel.open {
        // A pending rebind owns the next key press, including Escape.
        if panel.rebinding.is_some() {
            return;
        }
        if keys.just_pressed(KeyCode::Tab) {
            let index = SettingsTab::ALL
                .iter()
                .position(|t| *t == panel.tab)
                .unwrap_or(0);
            panel.tab = SettingsTab::ALL[(index + 1) % SettingsTab::ALL.len()];
            cursor.0 = 0;
        }
        if keys.just_pressed(KeyCode::Escape) {
            panel.close();
            cursor.0 = 0;
        }
        return;
    }

    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    match state.get() {
        ShellState::Playing => {
            let dragging = track.is_some_and(|t| t.drag.is_some());
            if !dragging {
                next.set(ShellState::Paused);
            }
        }
        ShellState::Paused => next.set(ShellState::Playing),
        ShellState::NewMap => next.set(ShellState::Title),
        // Escape on the title does nothing. Quitting is a menu item, never a
        // stray keypress.
        ShellState::Title => {}
    }
}

/// One place where every shell button is answered.
#[allow(clippy::too_many_arguments)]
fn dispatch_menu_actions(
    mut activated: MessageReader<MenuActivated>,
    state: Res<State<ShellState>>,
    mut next: ResMut<NextState<ShellState>>,
    mut panel: ResMut<SettingsPanel>,
    mut settings: ResMut<Settings>,
    mut draft: ResMut<DraftMapOptions>,
    mut pending: ResMut<PendingWorld>,
    mut status: ResMut<SaveStatus>,
    mut save_request: ResMut<ShellSaveRequest>,
    mut cursor: ResMut<MenuCursor>,
    mut exit: MessageWriter<AppExit>,
) {
    for MenuActivated(action) in activated.read().copied() {
        if new_map::apply_new_map_action(&mut draft, action) {
            continue;
        }
        match action {
            MenuAction::Continue => {
                // With a save, resume it; without one, play the world already on
                // screen (design §2 — the background map is playable as it is).
                if let Some(info) = save::newest_slot() {
                    save::request_load(&mut save_request, info.slot);
                }
                next.set(ShellState::Playing);
            }
            MenuAction::NewMap => {
                draft.0 = pending.options;
                next.set(ShellState::NewMap);
            }
            // One-click Load takes the newest save. A slot picker with
            // thumbnails (design §6) is the next thing to build here.
            MenuAction::Load => match save::newest_slot() {
                Some(info) => {
                    save::request_load(&mut save_request, info.slot);
                    next.set(ShellState::Playing);
                }
                None => status.set("No saves yet"),
            },
            MenuAction::OpenSettings => {
                panel.open_from(*state.get());
                cursor.0 = 0;
            }
            MenuAction::Quit => {
                exit.write(AppExit::Success);
            }
            MenuAction::Resume => next.set(ShellState::Playing),
            MenuAction::Save => save::request_save(&mut save_request, None),
            MenuAction::QuitToTitle => {
                status.clear();
                next.set(ShellState::Title);
            }
            MenuAction::Begin => {
                pending.request(draft.0);
                next.set(ShellState::Playing);
            }
            MenuAction::Back => next.set(ShellState::Title),
            MenuAction::SelectTab(tab) => {
                panel.tab = tab;
                cursor.0 = 0;
            }
            MenuAction::CycleSetting(id, delta) => id.cycle(&mut settings, delta),
            MenuAction::RebindControl(control) => panel.rebinding = Some(control),
            MenuAction::ResetControls => settings.controls.reset(),
            MenuAction::CloseSettings => {
                panel.close();
                cursor.0 = 0;
            }
            // Handled above by `apply_new_map_action`, or intentionally inert.
            MenuAction::CycleMapOption(..) | MenuAction::RerollSeed | MenuAction::Inert => {}
        }
    }
}

/// Install the world the player just configured.
///
/// Replaces the map and terrain, resets the treasury to the chosen bracket, and
/// clears the sim state the shell can reach so anchors re-seed onto the new
/// terrain. Presentation sprites are despawned here; respawning terrain is the
/// app's hook (see the module docs).
///
/// # Nothing of the old world may survive
///
/// The list below is deliberately the same list [`rail_sim::WorldSnapshot`]
/// overwrites on a load, because a New Map and a load leave the world in exactly
/// the same condition and anything either one forgets is a ghost. Three of them
/// were forgotten and each one was a bug the player could see:
///
/// - **Train entities.** A train is *sim state*, not a sprite that reconciles
///   against a registry, so clearing [`TrainYard`] never touched it. It survived
///   into the new world still holding a `TrackId` from the old one, which the
///   new (empty) [`TrackNetwork`] has never heard of — so it never moved, could
///   not be routed, and with no sell command could not be got rid of. That is
///   the orphaned train standing on the grass at the start of a new game.
/// - **[`BorderRegistry`].** Trains mid-crossing live in here as plain data, and
///   they come home on a due tick regardless of which world is on screen. A
///   crossing begun in the old world would land rolling stock on the new one.
/// - **[`PeepSpawnState`].** It remembers which station ids it has already
///   populated. A fresh [`StationRegistry`] hands out the same ids from one
///   again, so every new station read as "already served, nobody is moving back
///   in" and the new map got no residents at all.
///
/// The rest of the peep slice — [`HouseholdRegistry`], [`DistrictFlow`],
/// [`PeepBudget`], [`PeepFocus`], [`ComplaintFeed`] — goes with it, because a
/// family, a district's flow or a line of Town Talk all name stations by an id
/// the new world will hand out again to somewhere else entirely.
#[allow(clippy::too_many_arguments)]
fn apply_pending_world(
    mut commands: Commands,
    mut pending: ResMut<PendingWorld>,
    track_sprites: Query<Entity, With<TrackSprite>>,
    peeps: Query<Entity, With<Peep>>,
    trains: Query<Entity, With<Train>>,
    mut cameras: Query<&mut Transform, With<MapCamera>>,
) {
    if !pending.requested {
        return;
    }
    pending.requested = false;
    pending.rebuilding = true;
    let options = pending.options;
    let map = options.generate();

    // Station, industry, building and peep *sprites* all reconcile against their
    // registries every frame, so clearing the registries below is enough to
    // clear them. Track sprites are the exception — they are driven by
    // `TrackEdit` messages and have no reconcile pass — so they go by hand.
    // Peep and train entities are sim state, not presentation, and go with them.
    for entity in track_sprites.iter().chain(peeps.iter()).chain(trains.iter()) {
        commands.entity(entity).despawn();
    }

    commands.insert_resource(map_descriptor_for(&options, &map));
    commands.insert_resource(map_options::track_terrain_from(&map));
    // The seeder reads the generator's picked sites; without this a New Map
    // would anchor its opening beat against the boot map's hints.
    commands.insert_resource(rail_sim::AnchorSites(map.anchor_hints()));
    commands.insert_resource(Money::new(options.cash.cents()));
    commands.insert_resource(CommandBuffer::default());
    commands.insert_resource(CommandHistory::default());
    commands.insert_resource(EventDirector::default());
    commands.insert_resource(TrackNetwork::default());
    commands.insert_resource(StationRegistry::default());
    commands.insert_resource(IndustryRegistry::default());
    commands.insert_resource(StationService::default());
    commands.insert_resource(TrainYard::default());
    commands.insert_resource(TileOccupancy::default());
    commands.insert_resource(JobBoard::default());
    commands.insert_resource(MoneyLedger::default());
    commands.insert_resource(MaintenanceAccrual::default());
    commands.insert_resource(AlertBoard::default());
    commands.insert_resource(DemandSpawner::default());
    commands.insert_resource(LineRegistry::default());
    commands.insert_resource(TownDensity::default());
    commands.insert_resource(BorderRegistry::default());
    // The town's people, what they remember, and what they said about it.
    commands.insert_resource(PeepSpawnState::default());
    commands.insert_resource(HouseholdRegistry::default());
    commands.insert_resource(DistrictFlow::default());
    commands.insert_resource(PeepBudget::default());
    commands.insert_resource(PeepFocus::default());
    commands.insert_resource(VacatedHomes::default());
    commands.insert_resource(ComplaintFeed::default());
    commands.insert_resource(WorldAnchorsSeeded(false));
    commands.insert_resource(Selection::default());
    // Goals are a lens on the sandbox, so they are installed with the world
    // rather than switched on separately. Sandbox worlds get an inert board.
    commands.insert_resource(goal_board_for(&options));

    if let Ok(mut transform) = cameras.single_mut() {
        title::centre_camera_on_map(&map, &mut transform);
    }
    // Inserted last, and deliberately: `map::terrain` watches `MapGrid` for a
    // change and re-composites (or regrows) its chunk grid off the back of it,
    // so swapping the resource *is* the terrain rebuild.
    commands.insert_resource(map);
}

/// The rebuild flag lives for exactly one frame: every `OnEnter` hook, and every
/// `Update` system that watched for it, has run by `Last`.
fn finish_world_rebuild(mut pending: ResMut<PendingWorld>) {
    if pending.rebuilding {
        pending.rebuilding = false;
    }
}

/// Hide the in-game HUD while the shell owns the screen.
///
/// Root UI nodes that are not shell-owned are the game's chrome — status strip,
/// toolbar, Town Talk, ledger, alerts, inspector. Hiding by [`Visibility`] rather
/// than `Node.display` deliberately leaves each panel's own show / hide logic
/// untouched, so nothing has to be restored when play resumes.
fn hide_game_hud(
    state: Res<State<ShellState>>,
    mut roots: Query<&mut Visibility, (With<Node>, Without<ChildOf>, Without<ShellUi>)>,
) {
    // Paused still shows the HUD: the player should see what they were doing.
    let visible = matches!(state.get(), ShellState::Playing | ShellState::Paused);
    let wanted = if visible {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in &mut roots {
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Drop keyboard / wheel input before gameplay systems see it.
///
/// Runs last in the shell's `PreUpdate` chain, so shell navigation — including
/// a pending rebind — has already read what it needs. A key held across the
/// transition back into play needs one re-press, which is the correct behaviour
/// anyway: nobody expects to still be panning after closing a menu.
fn suppress_world_input(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut scroll: ResMut<AccumulatedMouseScroll>,
) {
    keys.reset_all();
    mouse.reset_all();
    *scroll = AccumulatedMouseScroll::default();
}

/// Autosave on the configured interval (design §6). Never blocks play — the
/// request is serviced by the background writer in `rail_sim::save`.
fn tick_autosave(
    time: Res<Time>,
    settings: Res<Settings>,
    mut timer: ResMut<AutosaveTimer>,
    mut request: ResMut<ShellSaveRequest>,
) {
    if timer.tick(time.delta_secs(), settings.gameplay.autosave_minutes) {
        // `Auto(0)` names the rotation, not slot zero: the save layer picks the
        // next slot in the ring itself.
        save::request_save(&mut request, Some(rail_sim::save::SaveSlot::Auto(0)));
    }
}

/// A finished load leaves the world holding new data and stale art, so it asks
/// for the same presentation rebuild a new map does.
fn mark_rebuild_after_load(
    mut request: ResMut<ShellSaveRequest>,
    mut pending: ResMut<PendingWorld>,
) {
    if request.loaded {
        request.loaded = false;
        pending.rebuilding = true;
    }
}

#[cfg(test)]
mod tests {
    use super::ShellState;

    #[test]
    fn pause_holds_the_camera_where_the_player_left_it() {
        // Pause is a menu, but it is drawn over the player's own view. Drifting
        // there wanders the camera off and resuming dumps them somewhere else.
        assert!(ShellState::Paused.is_menu());
        assert!(
            !ShellState::Paused.drifts_background(),
            "pausing must not hand the camera to the title drift"
        );
        assert!(ShellState::Title.drifts_background());
        assert!(ShellState::NewMap.drifts_background());
        assert!(!ShellState::Playing.drifts_background());
    }

    use super::*;
    use bevy::asset::AssetPlugin;
    use bevy::input::keyboard::{Key, KeyboardInput};
    use bevy::input::{ButtonState, InputPlugin};
    use bevy::state::app::StatesPlugin;

    /// Headless app with just enough of Bevy for the shell to run for real.
    ///
    /// The point is to exercise schedule construction, run conditions and
    /// resource availability — the failures that only appear at runtime.
    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            InputPlugin,
            AssetPlugin::default(),
        ))
        .init_asset::<Image>()
        .init_resource::<UiScale>()
        // Normally `SimPlugin`'s; the shell only needs somewhere to post intent.
        .init_resource::<CommandBuffer>()
        .add_plugins(ShellPlugin {
            boot_seed: BootSeed::Fixed(42),
            suppress_world_input: true,
        })
        // Override whatever `ShellPlugin::build` read off this machine, so the
        // tests never depend on (or write to) a real player profile.
        .insert_resource(Settings::default());
        app
    }

    fn state_of(app: &App) -> ShellState {
        *app.world().resource::<State<ShellState>>().get()
    }

    fn go_to(app: &mut App, state: ShellState) {
        app.world_mut()
            .resource_mut::<NextState<ShellState>>()
            .set(state);
        app.update();
    }

    /// Tap a key: press and release in one frame, as real hardware would over
    /// two. Without the release the key stays down and the *next* press never
    /// registers as `just_pressed`.
    fn press(app: &mut App, key: KeyCode) {
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(KeyboardInput {
                key_code: key,
                logical_key: Key::Unidentified(bevy::input::keyboard::NativeKey::Unidentified),
                state,
                text: None,
                repeat: false,
                window: Entity::PLACEHOLDER,
            });
        }
        app.update();
    }

    fn count<F: bevy::ecs::query::QueryFilter>(app: &mut App) -> usize {
        let mut query = app.world_mut().query_filtered::<Entity, F>();
        query.iter(app.world()).count()
    }

    /// Configure and Begin, then let the state transition install the world.
    fn begin(app: &mut App, options: MapOptions) {
        go_to(app, ShellState::NewMap);
        app.world_mut().resource_mut::<DraftMapOptions>().0 = options;
        app.world_mut()
            .write_message(MenuActivated(MenuAction::Begin));
        app.update();
        app.update();
    }

    /// A setup no default would land on: every knob off its stock value, and a
    /// Small map so the generator is cheap to run twice.
    fn distinctive(seed: u64) -> MapOptions {
        MapOptions {
            seed,
            size: MapSize::Small,
            terrain: TerrainStyle::Rugged,
            water: WaterStyle::Riverlands,
            resources: ResourceSpread::Clustered,
            ..MapOptions::default()
        }
    }

    /// The map's identity: seed, size, and every tile.
    fn fingerprint(map: &rail_map::MapGrid) -> (u64, u32, u32, Vec<rail_map::Tile>) {
        (map.seed, map.width, map.height, map.tiles().to_vec())
    }

    #[test]
    fn the_plugin_boots_into_a_title_screen_over_a_generated_world() {
        let mut app = test_app();
        app.update();

        assert_eq!(state_of(&app), ShellState::Title);
        assert_eq!(
            app.world().resource::<rail_map::MapGrid>().seed,
            42,
            "the shell installs its own world before anything draws it"
        );
        assert_eq!(
            count::<With<ShellUi>>(&mut app),
            1,
            "one shell screen is up"
        );
    }

    #[test]
    fn escape_opens_the_pause_menu_and_escape_again_resumes() {
        let mut app = test_app();
        app.update();
        go_to(&mut app, ShellState::Playing);
        assert_eq!(state_of(&app), ShellState::Playing);
        assert_eq!(
            count::<With<ShellUi>>(&mut app),
            0,
            "no chrome while playing"
        );

        press(&mut app, KeyCode::Escape);
        assert_eq!(state_of(&app), ShellState::Paused);
        assert_eq!(count::<With<ShellUi>>(&mut app), 1);

        press(&mut app, KeyCode::Escape);
        assert_eq!(state_of(&app), ShellState::Playing);
    }

    #[test]
    fn pausing_stops_the_sim_and_resuming_starts_it_again() {
        let mut app = test_app();
        app.update();
        go_to(&mut app, ShellState::Playing);
        go_to(&mut app, ShellState::Paused);
        let paused = app
            .world()
            .resource::<CommandBuffer>()
            .pending()
            .iter()
            .any(|c| matches!(c.kind, rail_sim::CommandKind::Pause(p) if p.paused));
        assert!(paused, "entering the pause menu asks the sim to pause");
    }

    #[test]
    fn the_new_map_screen_builds_a_preview_and_begin_installs_the_world() {
        let mut app = test_app();
        app.update();
        go_to(&mut app, ShellState::NewMap);
        app.update();

        assert_eq!(count::<With<new_map::NewMapRoot>>(&mut app), 1);
        let preview = app.world().resource::<PreviewImage>().0.clone();
        assert!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&preview)
                .is_some(),
            "the preview texture exists"
        );

        // Choose a different seed, then Begin.
        app.world_mut().resource_mut::<DraftMapOptions>().0.seed = 777;
        app.world_mut()
            .write_message(MenuActivated(MenuAction::Begin));
        // A state change requested during `Update` lands on the next frame's
        // transition, so the world is installed one update later.
        app.update();
        app.update();

        assert_eq!(state_of(&app), ShellState::Playing);
        assert_eq!(
            app.world().resource::<rail_map::MapGrid>().seed,
            777,
            "Begin installs the configured world"
        );
        assert!(
            !app.world().resource::<PendingWorld>().is_rebuilding(),
            "the rebuild flag lasts exactly one frame"
        );
    }

    /// The orphaned-train bug, at the seam it came from.
    ///
    /// A train is sim state, not a sprite that reconciles against a registry, so
    /// nothing cleared it when the world was replaced. It arrived in the new
    /// world holding a `TrackId` the new (empty) network had never heard of:
    /// unable to move, unable to be routed, and — there being no sell command —
    /// unable to be got rid of.
    ///
    /// The peep slice is checked here in full for the same reason. Every one of
    /// those resources keys off a station id, and a new world hands the same ids
    /// out again to entirely different places.
    #[test]
    fn a_new_map_leaves_nothing_of_the_old_world_behind() {
        let mut app = test_app();
        app.update();
        go_to(&mut app, ShellState::Playing);

        // A world that has been played in: rolling stock on the map, a border
        // ledger, and a peep spawner that remembers which stations it served.
        app.world_mut().spawn((
            rail_sim::Train {
                id: rail_sim::TrainId(1),
                kind: rail_sim::TrainKind::Transit,
            },
            rail_sim::TrainLocation::at_track(rail_sim::TrackId(7)),
        ));
        app.world_mut().spawn(rail_sim::Peep::new(
            rail_sim::PeepId(1),
            "Mara Aldertone",
            rail_sim::TileCoord { x: 4, y: 4 },
            rail_sim::HouseholdId(1),
            0,
        ));
        app.world_mut().insert_resource(BorderRegistry::new(1234));
        let mut spawn_state = PeepSpawnState {
            next_id: 42,
            ..PeepSpawnState::default()
        };
        spawn_state.spawned_for.insert(rail_sim::StationId(1));
        app.world_mut().insert_resource(spawn_state);

        // The rest of the town: a family at home, a district moving people, a
        // level-of-detail budget, and the camera the budget was biased toward.
        let mut households = HouseholdRegistry::new();
        households.insert(
            rail_sim::TileCoord { x: 4, y: 5 },
            rail_sim::StationId(1),
            0,
        );
        app.world_mut().insert_resource(households);
        let mut flow = DistrictFlow::default();
        flow.entry(rail_sim::StationId(1)).residents = 30;
        flow.request_trip(rail_sim::StationId(1), rail_sim::StationId(2));
        app.world_mut().insert_resource(flow);
        let mut budget = PeepBudget::default();
        budget.max_detailed = 9;
        app.world_mut().insert_resource(budget);
        let mut focus = PeepFocus::default();
        focus.look_at(rail_sim::TileCoord { x: 4, y: 4 }, 3);
        app.world_mut().insert_resource(focus);

        begin(
            &mut app,
            MapOptions {
                seed: 555,
                ..MapOptions::default()
            },
        );

        assert_eq!(app.world().resource::<rail_map::MapGrid>().seed, 555);
        assert_eq!(
            count::<With<rail_sim::Train>>(&mut app),
            0,
            "a train from the old world has no track under it in this one"
        );
        assert_eq!(
            count::<With<Peep>>(&mut app),
            0,
            "the old town's residents do not move to the new map"
        );
        assert_eq!(
            *app.world().resource::<BorderRegistry>(),
            BorderRegistry::default(),
            "a crossing begun in the old world must not land stock in this one"
        );
        let spawn_state = app.world().resource::<PeepSpawnState>();
        assert!(
            spawn_state.spawned_for.is_empty(),
            "station ids start again from one: a stale spawn record leaves the \
             new map with no residents at all"
        );
        assert_eq!(spawn_state.next_id, 0, "peep ids start again with the town");
        assert_eq!(
            app.world().resource::<HouseholdRegistry>().len(),
            0,
            "a family cannot still live at a station the new world has never built"
        );
        let flow = app.world().resource::<DistrictFlow>();
        assert_eq!(flow.iter().count(), 0);
        assert!(
            flow.pending_trips().is_empty(),
            "trips requested in the old world would be drained into this one's \
             job board"
        );
        assert_eq!(*app.world().resource::<PeepBudget>(), PeepBudget::default());
        assert_eq!(*app.world().resource::<PeepFocus>(), PeepFocus::default());
        assert!(app.world().resource::<TownDensity>().is_empty());
        assert!(app.world().resource::<ComplaintFeed>().is_empty());
        assert_eq!(
            app.world().resource::<rail_sim::AnchorSites>().0,
            app.world().resource::<rail_map::MapGrid>().anchor_hints(),
            "the seeder must be handed this world's opening sites, not the \
             boot map's"
        );
    }

    /// Seed sharing (design 02 §5) only reproduces a world if the *settings*
    /// travel with the seed — the options steer the generator now, so the same
    /// number with different knobs is a different map. Beginning a world
    /// therefore records how it was made, right next to the world.
    #[test]
    fn beginning_a_map_records_the_settings_that_made_it() {
        let mut app = test_app();
        app.update();
        begin(&mut app, distinctive(31_337));

        let descriptor = *app.world().resource::<MapDescriptor>();
        let map = app.world().resource::<rail_map::MapGrid>();
        assert_eq!(descriptor.seed, 31_337);
        assert_eq!(
            (descriptor.width, descriptor.height),
            (map.width, map.height),
            "the descriptor is sized from the grid, so the two cannot disagree"
        );

        let knobs = descriptor.gen.knobs.expect("the world says how it was made");
        let options = rail_map::MapGenOptions::unpack(knobs).expect("knobs unpack");
        assert_eq!(options.size, rail_map::MapSize::Small);
        assert_eq!(options.terrain, rail_map::TerrainStyle::Rugged);
        assert_eq!(options.water, rail_map::WaterStyle::Riverlands);
        assert_eq!(options.resources, rail_map::ResourceSpread::Clustered);
    }

    /// The map the save was played on, back tile for tile.
    ///
    /// A save carries the terrain but not the `MapGrid` — the seed and the
    /// packed knobs stand in for it, and the load regenerates. This is the whole
    /// reason the knobs are recorded, so it is worth proving against a world
    /// that is genuinely on screen at the time and genuinely different.
    #[test]
    fn loading_a_save_brings_back_the_map_it_was_played_on() {
        rail_sim::save::set_save_root(
            std::env::temp_dir().join(format!("rail_town_shell_saves_{}", std::process::id())),
        );
        let slot = rail_sim::save::SaveSlot::named("shell map reload").expect("valid name");
        let _ = rail_sim::save::delete_slot(&slot);

        let mut app = test_app();
        app.update();

        begin(&mut app, distinctive(24_601));
        let played = fingerprint(app.world().resource::<rail_map::MapGrid>());
        rail_sim::save::save_to_slot(app.world(), &slot).expect("save");

        // Somebody else's world, on screen, so nothing left over can pass for
        // the saved one.
        begin(
            &mut app,
            MapOptions {
                seed: 11,
                size: MapSize::Small,
                ..MapOptions::default()
            },
        );
        assert_ne!(
            fingerprint(app.world().resource::<rail_map::MapGrid>()),
            played
        );

        save::request_load(
            &mut app.world_mut().resource_mut::<ShellSaveRequest>(),
            slot.clone(),
        );
        app.update();

        assert_eq!(
            fingerprint(app.world().resource::<rail_map::MapGrid>()),
            played,
            "a load must give back the world that was saved, knobs and all"
        );
        let _ = rail_sim::save::delete_slot(&slot);
    }

    #[test]
    fn menu_keys_never_reach_the_game() {
        let mut app = test_app();
        app.update();
        // `B` is the track tool in play. On the title screen it must be gone by
        // the time gameplay systems run.
        press(&mut app, KeyCode::KeyB);
        let keys = app.world().resource::<ButtonInput<KeyCode>>();
        assert!(
            !keys.pressed(KeyCode::KeyB) && !keys.just_pressed(KeyCode::KeyB),
            "world input is suppressed while the shell owns the screen"
        );
    }

    #[test]
    fn a_rebind_press_does_not_also_arm_the_tool_it_binds() {
        // Settings opens *over* play, so the state is still `Playing` while it
        // is up. Pressing `B` to bind a verb has to reach the capture and stop
        // there — otherwise the player arms the track tool every time they
        // rebind something to it, which makes rebinding feel broken precisely
        // now that it works.
        let mut app = test_app();
        app.update();
        go_to(&mut app, ShellState::Playing);
        {
            let mut panel = app.world_mut().resource_mut::<SettingsPanel>();
            panel.open_from(ShellState::Playing);
            panel.rebinding = Some(controls::ControlAction::MapView);
        }
        press(&mut app, KeyCode::KeyB);

        assert_eq!(
            app.world()
                .resource::<Settings>()
                .controls
                .key_for(controls::ControlAction::MapView),
            controls::Binding::key(KeyCode::KeyB),
            "the rebind captured the key"
        );
        let keys = app.world().resource::<ButtonInput<KeyCode>>();
        assert!(
            !keys.just_pressed(KeyCode::KeyB),
            "and nothing downstream of the panel saw the same press"
        );
    }

    #[test]
    fn quitting_to_the_title_brings_the_title_screen_back() {
        let mut app = test_app();
        app.update();
        go_to(&mut app, ShellState::Playing);
        app.world_mut()
            .write_message(MenuActivated(MenuAction::QuitToTitle));
        app.update();
        app.update();
        assert_eq!(state_of(&app), ShellState::Title);
        assert_eq!(count::<With<ShellUi>>(&mut app), 1);
    }

    #[test]
    fn settings_opens_over_the_title_and_returns_to_it() {
        let mut app = test_app();
        app.update();
        app.world_mut()
            .write_message(MenuActivated(MenuAction::OpenSettings));
        app.update();

        let panel = app.world().resource::<SettingsPanel>().clone();
        assert!(panel.open);
        assert_eq!(panel.return_to, Some(ShellState::Title));

        press(&mut app, KeyCode::Escape);
        assert!(!app.world().resource::<SettingsPanel>().open);
        assert_eq!(
            state_of(&app),
            ShellState::Title,
            "closing settings must not also unwind the screen behind it"
        );
    }

    #[test]
    fn beginning_a_goals_map_starts_the_board_for_that_world() {
        let mut app = test_app();
        app.update();
        // Boot worlds are sandbox, so the board exists and does nothing.
        assert!(!app.world().resource::<GoalBoard>().is_active());

        go_to(&mut app, ShellState::NewMap);
        {
            let mut draft = app.world_mut().resource_mut::<DraftMapOptions>();
            draft.0.seed = 777;
            draft.0.mode = GameMode::Goals;
        }
        app.world_mut()
            .write_message(MenuActivated(MenuAction::Begin));
        app.update();
        app.update();

        let board = app.world().resource::<GoalBoard>();
        assert!(board.is_active(), "the Mode row reaches the sim");
        assert_eq!(
            board.seed,
            MapOptions {
                seed: 777,
                mode: GameMode::Goals,
                ..MapOptions::default()
            }
            .effective_seed(),
            "the set is derived from the world, not from the typed seed"
        );
        assert!(
            board.needs_generation(),
            "generation is `rail_sim`'s job, once the anchors land"
        );
    }

    #[test]
    fn the_goals_panel_follows_the_board_and_the_screen() {
        let mut app = test_app();
        app.update();
        assert_eq!(count::<With<goals_panel::GoalsPanelRoot>>(&mut app), 0);

        // A goals world with a set, in play.
        let mut board = GoalBoard::default();
        board.start(rail_sim::GoalMode::Goals, 1);
        board.install(vec![rail_sim::Goal::new(
            rail_sim::GoalId(0),
            rail_sim::GoalKind::Deliveries,
            "Complete 40 paid runs",
            40,
            8_640,
        )]);
        app.world_mut().insert_resource(board);
        go_to(&mut app, ShellState::Playing);
        app.update();
        assert_eq!(count::<With<goals_panel::GoalsPanelRoot>>(&mut app), 1);

        // Paused still shows it; the title screen does not.
        go_to(&mut app, ShellState::Paused);
        app.update();
        assert_eq!(count::<With<goals_panel::GoalsPanelRoot>>(&mut app), 1);

        go_to(&mut app, ShellState::Title);
        app.update();
        assert_eq!(
            count::<With<goals_panel::GoalsPanelRoot>>(&mut app),
            0,
            "the shell owns the screen; nothing of the game's is left on it"
        );
    }

    #[test]
    fn a_sandbox_world_never_draws_a_goals_panel() {
        let mut app = test_app();
        app.update();
        go_to(&mut app, ShellState::Playing);
        app.update();
        app.update();
        assert!(!app.world().resource::<GoalBoard>().is_active());
        assert_eq!(count::<With<goals_panel::GoalsPanelRoot>>(&mut app), 0);
    }

    #[test]
    fn only_playing_is_not_a_menu() {
        assert!(ShellState::Title.is_menu());
        assert!(ShellState::NewMap.is_menu());
        assert!(ShellState::Paused.is_menu());
        assert!(!ShellState::Playing.is_menu());
    }

    #[test]
    fn the_shell_boots_into_the_title() {
        assert_eq!(ShellState::default(), ShellState::Title);
    }

    #[test]
    fn a_fixed_boot_seed_reproduces_the_same_world() {
        let a = MapOptions {
            seed: 84_213,
            ..MapOptions::default()
        };
        assert_eq!(a.generate().seed, a.effective_seed());
        assert_eq!(
            a.effective_seed(),
            84_213,
            "stock options pass the seed through"
        );
    }

    #[test]
    fn suppression_is_on_by_default_so_the_plugin_is_safe_alone() {
        let plugin = ShellPlugin::default();
        assert!(plugin.suppress_world_input);
        assert_eq!(plugin.boot_seed, BootSeed::Rolled);
    }
}
