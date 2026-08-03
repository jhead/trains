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
use rail_sim::TileCoord;

use panel::{
    neighbour_button_clicks, neighbour_button_hover, neighbours_panel_input,
    setup_neighbours_panel, update_neighbours_panel,
};
use yard::{animate_yard_trains, light_yard_windows, sync_border_yards};

/// Border Yard art, the Neighbours panel, and the map-side portal mirror.
pub struct BorderPresentationPlugin;

impl Plugin for BorderPresentationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::input::KeyBindings>()
            .add_systems(Startup, setup_neighbours_panel)
            .add_systems(
            Update,
            (
                seed_border_registry,
                sync_portals_from_registry.after(seed_border_registry),
                sync_border_yards,
                animate_yard_trains.after(sync_border_yards),
                light_yard_windows.after(sync_border_yards),
                neighbours_panel_input.in_set(crate::input::PlayerVerbSet),
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

/// The door the registry wants on each edge, in [`BorderEdge::ALL`] order.
///
/// Comparing this against the last one applied is what keeps the mirror off the
/// per-frame path. See [`sync_portals_from_registry`].
type PortalPlan = [Option<TileCoord>; 4];

fn portal_plan(registry: &BorderRegistry) -> PortalPlan {
    let mut plan: PortalPlan = [None; 4];
    for (slot, edge) in BorderEdge::ALL.into_iter().enumerate() {
        plan[slot] = registry.get(edge).map(|link| link.portal_tile);
    }
    plan
}

/// Mirror open links onto the map's portal records.
///
/// The registry is the source of truth (it is what the save carries), so this
/// only ever writes in one direction.
///
/// # Why this compares a plan instead of trusting `is_changed`
///
/// [`BorderRegistry`] carries the border clock, and `advance_border_trade` bumps
/// that tick every sim tick — so `is_changed()` is true on essentially every
/// frame even in solo play with no border ever opened. Taking `ResMut<MapGrid>`
/// on the strength of it wrote to the map every frame, which marked the map
/// changed, which made `map::terrain` re-composite all sixteen chunks and
/// re-upload sixteen megabytes of texture **per frame**. That single line was
/// two thirds of the frame budget.
///
/// So the gate is the thing that actually matters: *which door is open on each
/// edge*. It changes when a link opens or closes and at no other time, and the
/// grid is only borrowed mutably when it has genuinely moved.
pub fn sync_portals_from_registry(
    registry: Res<BorderRegistry>,
    grid: Option<ResMut<MapGrid>>,
    mut applied: Local<Option<PortalPlan>>,
) {
    let _perf = crate::overlays::perf::scope("sync_portals_from_registry");
    let plan = portal_plan(&registry);
    if *applied == Some(plan) {
        return;
    }
    let Some(mut grid) = grid else {
        // No map yet — try again next frame rather than recording this plan as
        // applied against a grid that was never told.
        return;
    };
    for (slot, edge) in BorderEdge::ALL.into_iter().enumerate() {
        let facing = facing_of(edge);
        match plan[slot] {
            Some(tile) => {
                // Exactly one door per edge, at the tile the line reached.
                grid.close_portals_facing(facing);
                grid.open_portal_at(tile);
            }
            None => {
                grid.close_portals_facing(facing);
            }
        }
    }
    *applied = Some(plan);
}

#[cfg(test)]
mod tests {
    use super::*;

    use rail_map::generate_map;

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

    /// A world with a map and an empty registry, running only the mirror.
    fn mirror_app() -> App {
        let mut app = App::new();
        app.insert_resource(generate_map(24, 24, 42));
        app.insert_resource(BorderRegistry::new(42));
        app.add_systems(Update, sync_portals_from_registry);
        app
    }

    #[test]
    fn a_closed_border_plans_no_doors() {
        let registry = BorderRegistry::new(42);
        assert_eq!(portal_plan(&registry), [None, None, None, None]);
    }

    /// The regression this system's shape exists for.
    ///
    /// `advance_border_trade` bumps the registry's tick every sim tick, so
    /// `Res<BorderRegistry>::is_changed()` was true on essentially every frame
    /// — including solo play with no border ever opened, which is what almost
    /// every session is. Mirroring on the strength of that borrowed `MapGrid`
    /// mutably every frame, which marked the map changed, which made
    /// `map::terrain` re-composite all sixteen chunks and re-upload their
    /// textures: 79.6 ms of a 118 ms frame. The mirror must key on the doors,
    /// not on the clock.
    #[test]
    fn a_ticking_registry_does_not_touch_the_map() {
        let mut app = mirror_app();
        app.update();

        // Watch the map across a run of frames in which the only thing that
        // moves is the border clock — exactly what solo play does forever.
        for _ in 0..8 {
            app.world_mut().clear_trackers();
            app.world_mut().resource_mut::<BorderRegistry>().tick += 1;
            app.update();
            assert!(
                !app.world().resource_ref::<MapGrid>().is_changed(),
                "the portal mirror wrote to the map on a tick-only frame"
            );
        }
    }

    /// Even a *changed* registry must not write when the doors land the same.
    #[test]
    fn re_announcing_the_same_doors_is_not_a_map_edit() {
        let mut app = mirror_app();
        app.update();
        for _ in 0..4 {
            app.world_mut().clear_trackers();
            // A full change announcement, with the plan unmoved.
            app.world_mut()
                .resource_mut::<BorderRegistry>()
                .set_changed();
            app.update();
            assert!(!app.world().resource_ref::<MapGrid>().is_changed());
        }
    }
}
