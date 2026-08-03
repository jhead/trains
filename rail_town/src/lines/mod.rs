//! Player line tools and strip panel.

mod panel;
mod tools;

use bevy::prelude::*;

use panel::{
    assign_train_clicks, line_row_clicks, setup_lines_panel, update_lines_panel, FocusedLine,
};
use tools::line_tool_input;

use crate::inspect::SelectionInputSet;

pub use tools::LineToolState;

pub struct LinesPlugin;

impl Plugin for LinesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LineToolState>()
            .init_resource::<crate::input::KeyBindings>()
            .init_resource::<FocusedLine>()
            .add_systems(Startup, setup_lines_panel)
            .add_systems(
                Update,
                (
                    line_tool_input.after(SelectionInputSet),
                    update_lines_panel,
                    line_row_clicks,
                    assign_train_clicks,
                ),
            );
    }
}
