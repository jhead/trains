//! Player line tools and strip panel.

mod panel;
mod tools;

use bevy::prelude::*;

use panel::{
    apply_confirmed_remove_line, assign_clicked_train_to_focused_line, assign_train_clicks,
    line_row_clicks, remove_line_clicks, setup_lines_panel, tick_lines_feedback,
    update_lines_panel,
};
use tools::{focus_new_line, line_tool_input};

use crate::inspect::SelectionInputSet;

pub use panel::{FocusedLine, LinesFeedback};
pub use tools::LineToolState;

pub struct LinesPlugin;

impl Plugin for LinesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LineToolState>()
            .init_resource::<crate::input::KeyBindings>()
            .init_resource::<FocusedLine>()
            .init_resource::<LinesFeedback>()
            .add_systems(Startup, setup_lines_panel)
            .add_systems(
                Update,
                (
                    line_tool_input
                        .after(SelectionInputSet)
                        .in_set(crate::input::PlayerVerbSet),
                    // Reads the pick the selection pass just made, so it has to
                    // run after it — this is the click-to-assign interaction.
                    assign_clicked_train_to_focused_line
                        .after(SelectionInputSet)
                        .in_set(crate::input::PlayerVerbSet),
                    focus_new_line,
                    tick_lines_feedback,
                    update_lines_panel,
                    line_row_clicks,
                    assign_train_clicks,
                    remove_line_clicks,
                    apply_confirmed_remove_line,
                ),
            );
    }
}
