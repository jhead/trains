//! Station build tool, catchment ghost, and station / industry sprites.
//!
//! Stations are **built, not given** (`docs/design/04-building-and-tools.md` §6):
//! `P` selects the tool and cycles the tier, left-click lays platforms on the
//! line under the cursor, `U` upgrades in place. Industries and the opening
//! anchors are still auto-seeded by `rail_sim`; this module only draws those.

mod ghost;
mod preview;
mod tools;
mod visuals;

use bevy::prelude::*;

use ghost::{setup_station_hud, sync_station_ghosts, update_station_hud};
use tools::station_tool_input;
use visuals::sync_station_industry_sprites;

use crate::inspect::SelectionInputSet;

#[allow(unused_imports)] // available to inspect / overlays
pub use ghost::StationGhost;
#[allow(unused_imports)] // available to inspect / overlays
pub use preview::{preview_station, station_hud_line, station_reason, StationPreview};
pub use tools::StationToolState;
#[allow(unused_imports)] // available to inspect / overlays
pub use visuals::{tier_sprite_scale, IndustrySprite, NewDemandMarker, StationSprite};

pub struct StationsPlugin;

impl Plugin for StationsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StationToolState>()
            .add_systems(Startup, setup_station_hud)
            .add_systems(
                Update,
                (
                    station_tool_input.after(SelectionInputSet),
                    sync_station_ghosts.after(station_tool_input),
                    update_station_hud.after(station_tool_input),
                    sync_station_industry_sprites,
                ),
            );
    }
}
