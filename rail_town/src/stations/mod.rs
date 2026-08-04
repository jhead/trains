//! Station build tool, catchment ghost, and station / industry sprites.
//!
//! Stations are **built, not given** (`docs/design/04-building-and-tools.md` §6):
//! the Station slot on the menu row arms this tool, the tier row beneath it
//! picks the grade, left-click lays platforms on the line under the cursor, and
//! the Inspector's card upgrades or lifts the stop it describes. `P`, its tier
//! cycle and `U` are accelerators on top of all that, not the way in — for most
//! of this module's life they *were* the way in, and a player who never pressed
//! `P` had no reason to think stations were theirs to place at all.
//!
//! Industries and the opening anchors are still auto-seeded by `rail_sim`; this
//! module only draws those.

mod ghost;
mod preview;
mod tools;
mod visuals;

use bevy::prelude::*;

use ghost::{setup_station_hud, sync_station_ghosts, update_station_hud};
use tools::{apply_confirmed_demolish, speak_station_refusals, station_tool_input};
use visuals::sync_station_industry_sprites;

use crate::inspect::SelectionInputSet;

#[allow(unused_imports)] // available to inspect / overlays
pub use ghost::StationGhost;
#[allow(unused_imports)] // available to inspect / overlays
pub use preview::{preview_station, station_hud_line, station_reason, StationPreview};
pub use tools::{request_demolish, StationToolState};
#[allow(unused_imports)] // available to inspect / overlays
pub use visuals::{tier_sprite_scale, IndustrySprite, NewDemandMarker, StationSprite};

pub struct StationsPlugin;

impl Plugin for StationsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StationToolState>()
            .init_resource::<crate::input::KeyBindings>()
            .add_systems(Startup, setup_station_hud)
            .add_systems(
                Update,
                (
                    station_tool_input
                        .after(SelectionInputSet)
                        .in_set(crate::input::PlayerVerbSet),
                    apply_confirmed_demolish.after(station_tool_input),
                    // After the tool, so this frame's own preview refusal wins
                    // over one the sim raised on an earlier tick.
                    speak_station_refusals.after(station_tool_input),
                    sync_station_ghosts.after(station_tool_input),
                    update_station_hud.after(station_tool_input),
                    sync_station_industry_sprites,
                ),
            );
    }
}
