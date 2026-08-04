//! Selection, world picking, outline, and inspector panel (Phase B).

mod cause;
mod hover;
mod outline;
mod panel;
mod pick;
mod selection;

use bevy::prelude::*;

use hover::{
    hover_pick, setup_hover, sync_hover_brackets, update_hover_tooltip, HoverProbe, Hovered,
};
use outline::{setup_selection_outline, sync_selection_outline};
use panel::{
    cause_jump_clicks, paint_station_actions, setup_inspector_panel, station_action_clicks,
    update_inspector_panel,
};
use selection::{
    follow_selection, sample_service_history, selection_click_input, ServiceScoreHistory,
};

pub use pick::Selectable;
pub use selection::{Selection, WorldClickConsumed};

/// Runs before track / train tool press handling so selection can claim a click.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SelectionInputSet;

pub struct InspectPlugin;

impl Plugin for InspectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Selection>()
            .init_resource::<crate::input::KeyBindings>()
            .init_resource::<WorldClickConsumed>()
            .init_resource::<ServiceScoreHistory>()
            .init_resource::<Hovered>()
            .init_resource::<HoverProbe>()
            .configure_sets(Update, SelectionInputSet)
            .add_systems(
                Startup,
                (setup_inspector_panel, setup_selection_outline, setup_hover),
            )
            .add_systems(
                Update,
                selection_click_input
                    .in_set(SelectionInputSet)
                    .in_set(crate::input::PlayerVerbSet),
            )
            .add_systems(
                Update,
                (
                    follow_selection,
                    sample_service_history,
                    sync_selection_outline,
                    // Closing the Inspector is the window manager's job now
                    // (`ui::window` owns the close box, `ui::adapters` turns a
                    // close back into a cleared selection), so this only fills
                    // the rows in.
                    update_inspector_panel,
                    // The station card's own verbs — Upgrade and Demolish live
                    // on the thing they act on (see `panel::UpgradeOffer`).
                    paint_station_actions,
                    // After the fill, so a click acts on the row it saw.
                    cause_jump_clicks.after(update_inspector_panel),
                    station_action_clicks.after(paint_station_actions),
                ),
            )
            // Hover is the middle tier of interrogation (brief 05 §1): pick,
            // then draw the bracket and the chip from what was picked.
            //
            // `hover_pick` gates itself on pointer / camera / window movement,
            // so on a still frame this whole chain costs three early returns.
            .add_systems(
                Update,
                (
                    hover_pick,
                    sync_hover_brackets.after(hover_pick),
                    update_hover_tooltip.after(hover_pick),
                ),
            );
    }
}
