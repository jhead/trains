//! Border presentation — the Border Yard, the Neighbours panel, and the door.
//!
//! `docs/design/12-multiplayer.md` §3.2 and §9. Three jobs, and no fourth:
//!
//! - [`yard`] draws the strip of world beyond each open edge.
//! - [`panel`] lists the four edges and turns clicks into ordinary commands.
//! - [`sync_portals_from_registry`] mirrors the simulation's open links onto
//!   [`rail_map::MapGrid`]'s portal records, so the map's door follows the sim
//!   rather than leading it.
//!
//! Border events do **not** get a notification system of their own: they go into
//! Town Talk, which `rail_sim::border` already writes to (§9, "Town Talk carries
//! the border"). Nothing in this module needs to know that happened.
//!
//! The chrome is the existing pixel kit (`crate::ui::kit`) and the existing
//! palette (`crate::palette`). There is no second UI style here and there must
//! never be one.

mod panel;
mod yard;

use bevy::prelude::*;
use rail_map::{EdgeFacing, MapGrid};
use rail_sim::border::{BorderEdge, BorderRegistry};

use panel::{
    neighbour_button_clicks, neighbour_button_hover, neighbours_panel_input,
    setup_neighbours_panel, update_neighbours_panel,
};
use yard::{animate_yard_trains, light_yard_windows, sync_border_yards};

/// Border Yard art, the Neighbours panel, and the map-side portal mirror.
pub struct BorderPresentationPlugin;

impl Plugin for BorderPresentationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_neighbours_panel).add_systems(
            Update,
            (
                seed_border_registry,
                sync_portals_from_registry.after(seed_border_registry),
                sync_border_yards,
                animate_yard_trains.after(sync_border_yards),
                light_yard_windows.after(sync_border_yards),
                neighbours_panel_input,
                update_neighbours_panel.after(neighbours_panel_input),
                neighbour_button_hover,
                neighbour_button_clicks,
            ),
        );
    }
}

/// `rail_sim`'s edge, as `rail_map`'s facing.
///
/// The two enums are declared in crates that cannot see each other and are kept
/// in step by their index — asserted in both directions by
/// [`tests::the_two_edge_enums_agree`].
pub fn facing_of(edge: BorderEdge) -> EdgeFacing {
    EdgeFacing::from_index(edge.index()).unwrap_or(EdgeFacing::North)
}

/// `rail_map`'s facing, as `rail_sim`'s edge.
///
/// The inverse half of the pair. Unused today — presentation only ever
/// translates one way — but it is what a portal-click tool and MP-2's pairing
/// screen will both need, and keeping both halves next to each other is what
/// makes [`tests::the_two_edge_enums_agree`] able to check the round trip.
#[allow(dead_code)]
pub fn edge_of(facing: EdgeFacing) -> BorderEdge {
    BorderEdge::from_index(facing.index()).unwrap_or(BorderEdge::North)
}

/// Tell the border registry which world it is in.
///
/// Echo neighbours are a pure function of `(map seed, edge)`, so a registry that
/// never learned the seed would give every map on earth the same four towns. The
/// seed lives on [`MapGrid`]; the registry is what the save carries, so a world
/// that already has links keeps the seed those links were generated from and a
/// freshly generated map picks up its own.
pub fn seed_border_registry(grid: Option<Res<MapGrid>>, mut registry: ResMut<BorderRegistry>) {
    let Some(grid) = grid else {
        return;
    };
    // Only write when it would change something: `ResMut` is change-detected and
    // the portal mirror keys off that.
    if registry.seed == grid.seed || !registry.is_empty() {
        return;
    }
    registry.seed = grid.seed;
}

/// Mirror open links onto the map's portal records.
///
/// The registry is the source of truth (it is what the save carries), so this
/// only ever writes in one direction. It is cheap: four edges, and it early-outs
/// unless the registry changed.
pub fn sync_portals_from_registry(registry: Res<BorderRegistry>, grid: Option<ResMut<MapGrid>>) {
    if !registry.is_changed() {
        return;
    }
    let Some(mut grid) = grid else {
        return;
    };
    for edge in BorderEdge::ALL {
        let facing = facing_of(edge);
        match registry.get(edge) {
            Some(link) => {
                let tile = link.portal_tile;
                // Exactly one door per edge, at the tile the line reached.
                grid.close_portals_facing(facing);
                grid.open_portal_at(tile);
            }
            None => {
                grid.close_portals_facing(facing);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_edge_enums_agree() {
        for edge in BorderEdge::ALL {
            assert_eq!(edge_of(facing_of(edge)), edge);
            assert_eq!(facing_of(edge).index(), edge.index());
            assert_eq!(facing_of(edge).outward(), edge.outward());
            assert_eq!(facing_of(edge).label(), edge.label());
        }
        for facing in EdgeFacing::ALL {
            assert_eq!(facing_of(edge_of(facing)), facing);
        }
    }
}
