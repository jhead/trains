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
use rail_sim::{CommandBuffer, CommandKind, TrainYard};

use crate::lines::LineToolState;
use crate::stations::StationToolState;
use crate::track::{BuildTool, TrackToolState};
use crate::trains::{arm_train_place, TrainPlaceKind, TrainToolState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolbarTool {
    Select,
    Build,
    /// Platforms on the line (04 §6). A mode like Track, with a tier sub-row.
    Station,
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
            Self::Station => "Station",
            Self::Demolish => "Demolish",
            Self::Line => "Line",
            Self::Transit => "Transit",
            Self::Transport => "Transport",
        }
    }
}

/// The four tool resources plus the command buffer, bundled so arming a verb
/// is one call from anywhere in the UI.
#[derive(SystemParam)]
pub struct ToolStates<'w> {
    buffer: ResMut<'w, CommandBuffer>,
    track: ResMut<'w, TrackToolState>,
    train: ResMut<'w, TrainToolState>,
    line: ResMut<'w, LineToolState>,
    station: ResMut<'w, StationToolState>,
    /// The menu row must cost what the key costs — see [`arm_train_place`].
    yard: Res<'w, TrainYard>,
}

impl ToolStates<'_> {
    pub fn arm(&mut self, tool: ToolbarTool) {
        apply_toolbar_tool(
            tool,
            &self.yard,
            &mut self.buffer,
            &mut self.track,
            &mut self.train,
            &mut self.line,
            &mut self.station,
        );
    }

    /// Queue an intent on the same buffer the tools use.
    ///
    /// A system that already takes `ToolStates` holds `ResMut<CommandBuffer>`
    /// inside it, and asking Bevy for a second one is a conflicting access it
    /// refuses at run time (B0002). So the buffer is reached through here
    /// rather than beside it — the Trains window's *Add car* row is the caller
    /// that found this out.
    pub fn queue(&mut self, kind: CommandKind) {
        self.buffer.push(kind);
    }
}

/// Arm one verb, and disarm every other.
///
/// The Station tool is the reason this takes a fifth resource. It arms the
/// pointer exactly as Track does, so a row click that armed Track while the
/// station tool was still up would leave two tools reading the same left click —
/// which is what happened before it had a slot and only the keyboard could
/// reach it.
#[allow(clippy::too_many_arguments)]
pub fn apply_toolbar_tool(
    tool: ToolbarTool,
    yard: &TrainYard,
    buffer: &mut CommandBuffer,
    track: &mut TrackToolState,
    train: &mut TrainToolState,
    line: &mut LineToolState,
    station: &mut StationToolState,
) {
    if tool != ToolbarTool::Station {
        station.disarm();
    }
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
        // A platform is a kind of track (04 §6), so this arms the pointer the
        // same way Track does — and holds the world click, because the station
        // tool answers it rather than the track tool.
        ToolbarTool::Station => {
            station.active = true;
            station.reject = None;
            track.tool = BuildTool::Build;
            track.anchor = None;
            track.drag = None;
            track.suppress_build_click = true;
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
            arm_train_place(TrainPlaceKind::Transit, yard, buffer, train, track, line);
        }
        ToolbarTool::Transport => {
            arm_train_place(TrainPlaceKind::Transport, yard, buffer, train, track, line);
        }
    }
}

/// Which verb the game is currently in, derived from the tool resources.
///
/// The station tool is asked about first: it leaves [`BuildTool::Build`] armed
/// underneath (that is how it holds the pointer), so reading the track tool
/// first would draw Track as the armed verb while a platform ghost is on screen.
pub fn active_tool(
    tool: BuildTool,
    placing: bool,
    kind: Option<TrainPlaceKind>,
    line_active: bool,
    station_active: bool,
) -> ToolbarTool {
    if station_active {
        return ToolbarTool::Station;
    }
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
    use rail_sim::{CommandKind, TrainKind};

    /// Every resource `apply_toolbar_tool` writes, in one place.
    #[derive(Default)]
    struct Tools {
        buffer: CommandBuffer,
        track: TrackToolState,
        train: TrainToolState,
        line: LineToolState,
        station: StationToolState,
    }

    impl Tools {
        fn arm(&mut self, tool: ToolbarTool, yard: &TrainYard) {
            apply_toolbar_tool(
                tool,
                yard,
                &mut self.buffer,
                &mut self.track,
                &mut self.train,
                &mut self.line,
                &mut self.station,
            );
        }

        fn armed(&self) -> ToolbarTool {
            active_tool(
                self.track.tool,
                self.train.place_mode,
                Some(self.train.kind),
                self.line.active,
                self.station.active,
            )
        }
    }

    /// The menu row is the same verb as the key, so it must cost the same:
    /// stock in the yard is placed, not bought twice.
    #[test]
    fn the_menu_row_places_a_yard_train_rather_than_buying_a_second() {
        let mut yard = TrainYard::default();
        yard.buy(TrainKind::Transit);
        let mut tools = Tools::default();

        tools.arm(ToolbarTool::Transit, &yard);

        assert!(tools.train.place_mode);
        assert!(tools.buffer.pending().is_empty(), "no second purchase");

        // An empty yard still buys one.
        tools.arm(ToolbarTool::Transport, &yard);
        assert!(matches!(
            tools.buffer.pending().first().map(|c| &c.kind),
            Some(CommandKind::BuyTrain(_))
        ));
    }

    /// 03 §7: the row is how a player who has read nothing finds a verb. The
    /// Station slot has to *be* the `P` key — arm the tool, and show as armed.
    #[test]
    fn the_station_slot_arms_the_station_tool() {
        let yard = TrainYard::default();
        let mut tools = Tools::default();

        tools.arm(ToolbarTool::Station, &yard);

        assert!(tools.station.active, "the slot arms the platform tool");
        assert_eq!(tools.armed(), ToolbarTool::Station, "and the row says so");
        assert!(
            tools.track.suppress_build_click,
            "the station tool answers the click, not the track tool"
        );
        assert!(!tools.line.active);
        assert!(!tools.train.place_mode);
    }

    /// The bug the slot exists to make impossible: with the tool armed only by
    /// `P`, picking another verb off the row left the station tool up, and two
    /// tools read the same left click.
    #[test]
    fn arming_any_other_verb_puts_the_station_tool_down() {
        let yard = TrainYard::default();
        for tool in [
            ToolbarTool::Select,
            ToolbarTool::Build,
            ToolbarTool::Demolish,
            ToolbarTool::Line,
            ToolbarTool::Transit,
            ToolbarTool::Transport,
        ] {
            let mut tools = Tools::default();
            tools.arm(ToolbarTool::Station, &yard);
            tools.station.reject = Some("Too close - 2 tiles, need 3".into());

            tools.arm(tool, &yard);

            assert!(!tools.station.active, "{tool:?} left the station tool armed");
            assert!(
                tools.station.reject.is_none(),
                "{tool:?} kept a stale refusal on screen"
            );
            assert_ne!(tools.armed(), ToolbarTool::Station);
        }
    }

    #[test]
    fn active_tool_maps_modes() {
        assert_eq!(
            active_tool(BuildTool::Build, false, None, false, false),
            ToolbarTool::Build
        );
        assert_eq!(
            active_tool(BuildTool::Demolish, false, None, false, false),
            ToolbarTool::Demolish
        );
        assert_eq!(
            active_tool(BuildTool::Build, true, Some(TrainPlaceKind::Transit), false, false),
            ToolbarTool::Transit
        );
        assert_eq!(
            active_tool(BuildTool::Build, false, None, true, false),
            ToolbarTool::Line
        );
        // The station tool leaves Build armed underneath, and still reads as
        // Station.
        assert_eq!(
            active_tool(BuildTool::Build, false, None, false, true),
            ToolbarTool::Station
        );
    }

    #[test]
    fn every_verb_says_its_own_name() {
        for tool in [
            ToolbarTool::Select,
            ToolbarTool::Build,
            ToolbarTool::Station,
            ToolbarTool::Demolish,
            ToolbarTool::Line,
            ToolbarTool::Transit,
            ToolbarTool::Transport,
        ] {
            assert!(!tool.label().is_empty());
        }
    }
}
