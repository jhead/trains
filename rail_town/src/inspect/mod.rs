//! Selection, world picking, outline, and inspector panel (Phase B).

mod cause;
mod hover;
mod outline;
mod panel;
mod pick;
mod selection;

use bevy::prelude::*;

use hover::{hover_pick, setup_hover, sync_hover_brackets, update_hover_tooltip, Hovered};
use outline::{setup_selection_outline, sync_selection_outline};
use panel::{inspector_close_clicks, setup_inspector_panel, update_inspector_panel};
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
            .init_resource::<WorldClickConsumed>()
            .init_resource::<ServiceScoreHistory>()
            .init_resource::<Hovered>()
            .configure_sets(Update, SelectionInputSet)
            .add_systems(
                Startup,
                (setup_inspector_panel, setup_selection_outline, setup_hover),
            )
            .add_systems(Update, selection_click_input.in_set(SelectionInputSet))
            .add_systems(
                Update,
                (
                    follow_selection,
                    sample_service_history,
                    sync_selection_outline,
                    update_inspector_panel,
                    inspector_close_clicks,
                ),
            )
            // Hover is the middle tier of interrogation (brief 05 §1): pick,
            // then draw the bracket and the tooltip from what was picked.
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
