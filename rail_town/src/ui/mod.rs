//! The interface: window system, top chrome, and the windows themselves.
//!
//! Binding standard: [`docs/design/03-ui-system.md`](../../../docs/design/03-ui-system.md).
//!
//! # Shape
//!
//! Rail Town's interface is a **management-sim window system**, not a fixed HUD.
//! Three things are permanently on screen — the menu row, the status strip and
//! the network health strip, all in one block along the top ([`menu_bar`]).
//! Everything else is a window the player opened ([`window`]).
//!
//! | Module | Job |
//! | --- | --- |
//! | [`kit`] | Metrics, type, colour roles, meters, button chrome |
//! | [`format`] | Money, rate, clock and date readouts |
//! | [`window`] | Open / close / drag / stack / `Esc` |
//! | [`menu_bar`] | The top block, and the verb and window button groups |
//! | [`status_strip`] | Money, rate, date and time, speed, alert bell |
//! | [`health`] | Network health — the permanent readout, and its window |
//! | [`toolbar`] | The build-verb model behind the menu row |
//! | [`town_talk`], [`ledger`], [`alerts`] | Windows drawn here |
//! | [`adapters`] | Windows drawn elsewhere (Goals, Neighbours, Inspector) |
//!
//! Sound lives in the `audio` module, not here.
//!
//! # Cost
//!
//! Nothing in here iterates the world every frame. Every window paints from a
//! signature and skips the write when it has not moved; a closed window returns
//! on its first line; the health model rebuilds on a 250 ms timer; and window
//! layout is gated on [`window::WindowManager`] change detection.

mod adapters;
mod alerts;
mod format;
mod health;
pub(crate) mod kit;
mod ledger;
mod menu_bar;
mod status_strip;
mod toolbar;
mod town_talk;
mod undo;
mod window;

use bevy::input::InputSystems;
use bevy::prelude::*;

use adapters::{introduce_goals_window, sync_inspector_window, GoalsIntroduced, InspectorLink};
use alerts::{
    alert_dismiss_all_clicks, alert_row_clicks, setup_alerts_ui, update_alert_row_hover,
    update_alerts_ui,
};
use health::{
    health_chip_clicks, health_chip_hover, health_refresh_due, network_chip_clicks,
    rebuild_health_strip, rebuild_network_window, refresh_network_health, setup_network_window,
    NetworkHealth,
};
use kit::pointer_blocks_world;
use ledger::{setup_ledger_ui, update_ledger_panel};
use menu_bar::{
    inject_synthetic_keys, menu_row_clicks, setup_top_chrome, update_menu_row, window_hotkeys,
    SyntheticKeys,
};
use status_strip::{
    advance_calendar, alert_bell_clicks, speed_button_clicks, update_speed_buttons,
    update_status_strip, GameCalendar, StatusStripCache,
};
use town_talk::{refresh_town_talk_rows, setup_town_talk_ui, town_talk_clicks};
use undo::undo_redo_input;
use window::{
    apply_window_layout, close_top_window_on_escape, dress_new_windows, drag_windows,
    raise_clicked_window, update_window_chrome, window_close_clicks,
};

pub use window::{UiWindow, WindowEscSet, WindowId, WindowManager};

/// True while the pointer is over a UI button (world clicks should ignore).
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct UiBlocksWorld(pub bool);

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiBlocksWorld>()
            .insert_resource(WindowManager::new())
            .init_resource::<NetworkHealth>()
            .init_resource::<GameCalendar>()
            .init_resource::<StatusStripCache>()
            .init_resource::<SyntheticKeys>()
            .init_resource::<GoalsIntroduced>()
            .init_resource::<InspectorLink>()
            // `Esc` must reach a window before it reaches the pause menu, and a
            // synthetic key must land before anything reads `just_pressed`.
            // Both belong in `PreUpdate`, after real input has been gathered.
            // `ShellPlugin` orders its own `Esc` handling after this set.
            .configure_sets(PreUpdate, WindowEscSet.after(InputSystems))
            .add_systems(
                PreUpdate,
                (inject_synthetic_keys, close_top_window_on_escape)
                    .chain()
                    .in_set(WindowEscSet),
            )
            .add_systems(
                Startup,
                (
                    setup_top_chrome,
                    setup_network_window,
                    setup_town_talk_ui,
                    setup_ledger_ui,
                    setup_alerts_ui,
                ),
            )
            // Window plumbing.
            .add_systems(
                Update,
                (
                    dress_new_windows,
                    window_close_clicks,
                    raise_clicked_window,
                    drag_windows,
                    sync_inspector_window,
                    introduce_goals_window,
                    apply_window_layout,
                    update_window_chrome,
                )
                    .chain(),
            )
            // Top chrome.
            .add_systems(
                Update,
                (
                    sync_ui_blocks_world,
                    advance_calendar,
                    update_status_strip.after(advance_calendar),
                    update_speed_buttons,
                    speed_button_clicks,
                    alert_bell_clicks,
                    update_menu_row,
                    menu_row_clicks,
                    window_hotkeys,
                ),
            )
            // Network health: model on a timer, strip and window from signatures.
            .add_systems(
                Update,
                (
                    refresh_network_health.run_if(health_refresh_due()),
                    rebuild_health_strip.after(refresh_network_health),
                    rebuild_network_window.after(refresh_network_health),
                    health_chip_hover,
                    health_chip_clicks,
                    network_chip_clicks,
                ),
            )
            // Windows drawn here.
            .add_systems(
                Update,
                (
                    refresh_town_talk_rows,
                    town_talk_clicks,
                    update_ledger_panel,
                    update_alerts_ui,
                    alert_row_clicks,
                    alert_dismiss_all_clicks,
                    update_alert_row_hover,
                    undo_redo_input,
                ),
            );
    }
}

fn sync_ui_blocks_world(
    interactions: Query<&Interaction, Or<(With<Button>, With<kit::WorldClickBlocker>)>>,
    mut blocks: ResMut<UiBlocksWorld>,
) {
    blocks.0 = pointer_blocks_world(&interactions);
}
