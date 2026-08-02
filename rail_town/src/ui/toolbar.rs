//! The build verbs — the model behind the menu row's left-hand group.
//!
//! This used to be a bottom-centre toolbar. The bar is gone (03 §5) but its job
//! is not: it still owns the mapping between "the player asked for Track" and
//! the three tool resources that actually arm the pointer, and it still owns the
//! reverse mapping used to show which verb is armed.
//!
//! Keeping that here rather than in [`super::menu_bar`] means the row is pure
//! presentation, and this stays unit-testable without a `World`.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use rail_sim::commands::BuyTrain;
use rail_sim::{CommandBuffer, CommandKind, TrainKind};

use crate::lines::LineToolState;
use crate::track::{BuildTool, TrackToolState};
use crate::trains::{TrainPlaceKind, TrainToolState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolbarTool {
    Select,
    Build,
    Demolish,
    Line,
    Transit,
    Transport,
}

impl ToolbarTool {
    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Look",
            Self::Build => "Track",
            Self::Demolish => "Demolish",
            Self::Line => "Line",
            Self::Transit => "Transit",
            Self::Transport => "Transport",
        }
    }
}

/// The three tool resources plus the command buffer, bundled so arming a verb
/// is one call from anywhere in the UI.
#[derive(SystemParam)]
pub struct ToolStates<'w> {
    buffer: ResMut<'w, CommandBuffer>,
    track: ResMut<'w, TrackToolState>,
    train: ResMut<'w, TrainToolState>,
    line: ResMut<'w, LineToolState>,
}

impl ToolStates<'_> {
    pub fn arm(&mut self, tool: ToolbarTool) {
        apply_toolbar_tool(
            tool,
            &mut self.buffer,
            &mut self.track,
            &mut self.train,
            &mut self.line,
        );
    }
}

pub fn apply_toolbar_tool(
    tool: ToolbarTool,
    buffer: &mut CommandBuffer,
    track: &mut TrackToolState,
    train: &mut TrainToolState,
    line: &mut LineToolState,
) {
    match tool {
        ToolbarTool::Select => {
            // Disarm everything — this is the "put the tools down" slot.
            track.tool = BuildTool::Select;
            track.anchor = None;
            track.drag = None;
            track.suppress_build_click = false;
            train.place_mode = false;
            line.active = false;
            line.clear_draft();
        }
        ToolbarTool::Build => {
            track.tool = BuildTool::Build;
            track.anchor = None;
            track.drag = None;
            track.suppress_build_click = false;
            train.place_mode = false;
            line.active = false;
            line.clear_draft();
        }
        ToolbarTool::Demolish => {
            track.tool = BuildTool::Demolish;
            track.anchor = None;
            track.drag = None;
            track.suppress_build_click = false;
            train.place_mode = false;
            line.active = false;
            line.clear_draft();
        }
        ToolbarTool::Line => {
            line.active = true;
            line.clear_draft();
            train.place_mode = false;
            track.tool = BuildTool::Build;
            track.anchor = None;
            track.drag = None;
            track.suppress_build_click = true;
        }
        ToolbarTool::Transit => {
            buffer.push(CommandKind::BuyTrain(BuyTrain {
                kind: TrainKind::Transit,
            }));
            train.place_mode = true;
            train.kind = TrainPlaceKind::Transit;
            track.anchor = None;
            track.drag = None;
            track.suppress_build_click = true;
            line.active = false;
            line.clear_draft();
        }
        ToolbarTool::Transport => {
            buffer.push(CommandKind::BuyTrain(BuyTrain {
                kind: TrainKind::Transport,
            }));
            train.place_mode = true;
            train.kind = TrainPlaceKind::Transport;
            track.anchor = None;
            track.drag = None;
            track.suppress_build_click = true;
            line.active = false;
            line.clear_draft();
        }
    }
}

/// Which verb the game is currently in, derived from the tool resources.
pub fn active_tool(
    tool: BuildTool,
    placing: bool,
    kind: Option<TrainPlaceKind>,
    line_active: bool,
) -> ToolbarTool {
    if line_active {
        return ToolbarTool::Line;
    }
    if placing {
        return match kind.unwrap_or_default() {
            TrainPlaceKind::Transit => ToolbarTool::Transit,
            TrainPlaceKind::Transport => ToolbarTool::Transport,
        };
    }
    match tool {
        BuildTool::Select => ToolbarTool::Select,
        BuildTool::Build => ToolbarTool::Build,
        BuildTool::Demolish => ToolbarTool::Demolish,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_tool_maps_modes() {
        assert_eq!(
            active_tool(BuildTool::Build, false, None, false),
            ToolbarTool::Build
        );
        assert_eq!(
            active_tool(BuildTool::Demolish, false, None, false),
            ToolbarTool::Demolish
        );
        assert_eq!(
            active_tool(BuildTool::Build, true, Some(TrainPlaceKind::Transit), false),
            ToolbarTool::Transit
        );
        assert_eq!(
            active_tool(BuildTool::Build, false, None, true),
            ToolbarTool::Line
        );
    }

    #[test]
    fn every_verb_says_its_own_name() {
        for tool in [
            ToolbarTool::Select,
            ToolbarTool::Build,
            ToolbarTool::Demolish,
            ToolbarTool::Line,
            ToolbarTool::Transit,
            ToolbarTool::Transport,
        ] {
            assert!(!tool.label().is_empty());
        }
    }
}
