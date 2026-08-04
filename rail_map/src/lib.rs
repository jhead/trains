//! Map / terrain library for Rail Town.
//!
//! # Public API (stable for other slices)
//!
//! ## Query a tile
//! ```ignore
//! use rail_map::{generate_map, MapGrid};
//! use rail_sim::ids::TileCoord;
//!
//! let map: MapGrid = generate_map(64, 64, 42);
//! let tile = map.get(TileCoord { x: 10, y: 12 });
//! ```
//!
//! ## Water / height / walkable for track
//! - [`Tile::water`] — deep water (needs bridge later)
//! - [`Tile::height`] — elevation band (`i8`; negative under sea)
//! - [`Tile::kind`] — [`TerrainKind`] band
//! - [`Tile::is_walkable_for_track`] — `true` when track can be laid without a bridge
//!
//! ## World ↔ tile
//! - [`TILE_SIZE`] — ground-plane units per tile edge (32)
//! - [`tile_to_world`] / [`world_to_tile`] / [`map_center_world`]
//!
//! - [`ground_to_world`] — the same for a *sub-tile* point (a peep mid-stride,
//!   a house on its lot)
//!
//! Those follow the live [`Projection`]: top-down squares by default, or
//! 2:1 dimetric diamonds with an elevation lift once [`set_projection`] says so.
//! See [`coords`] for the projection, the lift and [`set_iso_heights`], which
//! installs the height field the lift reads. Use [`top_down_tile_to_world`] and
//! friends where a drawing is *deliberately* a plan view whatever the world is
//! doing — the Map View's schematic plate is the one such caller.
//!
//! ## Portals
//! - [`MapGrid::portals`] — all edge [`Portal`] stubs (`open: false` in MVP)
//! - [`MapGrid::portal_at`] — portal on a border tile, if any
//! - Facing / id types: [`EdgeFacing`], [`PortalId`], [`Layer`]
//!
//! ## Generation
//! - [`generate_map`] — seeded land / water / elevation + edge portals, stock options
//! - [`generate`] — seed plus [`MapGenOptions`], at the size the options name
//! - [`generate_map_with`] — the same, at an explicit size
//! - Option types: [`MapSize`], [`TerrainStyle`], [`WaterStyle`], [`ResourceSpread`]
//!
//! ## What generation meant
//!
//! Design 02 §4 makes anchor placement level design, and level design cannot be
//! inferred back out of a heightmap. So the generator writes its intentions down:
//!
//! - [`MapGrid::features`] — [`MapFeatures`]: the opening beat, growth sites,
//!   river crossings, ridge passes, and a per-tile [`Surface`] class that tells a
//!   river from a bay
//! - [`MapGrid::anchor_hints`] — those sites, best first, ready to seed a sampler
//!
//! ### Handing the opening beat to `rail_sim`
//!
//! `rail_sim` cannot see this crate (the dependency runs the other way), so the
//! hint travels as plain [`TileCoord`](rail_sim::ids::TileCoord)s in a resource
//! the app inserts beside its `TrackTerrain`:
//!
//! ```ignore
//! // rail_town, next to `commands.insert_resource(track_terrain_from(&map))`:
//! commands.insert_resource(rail_sim::AnchorSites(map.anchor_hints()));
//! ```
//!
//! Without it, anchor placement falls back to farthest-point sampling, which
//! design 02 §4.1 names as the worst possible opening.
//!
//! ## Measuring a map
//! - [`measure::composition`] — the four rows of design 02 §2.1
//! - [`measure::river_crossings`] / [`measure::ridge_passes`] — the decisions on offer
//! - [`measure::largest_buildable_region`] — is the mainland one place?

pub mod coords;
mod features;
mod gen;
mod grid;
mod options;
mod portal;
mod tile;

pub mod measure;

pub use coords::{
    clear_iso_heights, ground_to_world, map_center_world, project, project_offset, projection,
    projection_is_iso, set_iso_heights, set_projection, surface_height_of, tile_height, tile_lift,
    tile_to_ground, tile_to_world, tile_to_world_flat, top_down_map_center, top_down_tile_to_world,
    top_down_world_to_tile, unproject, unproject_offset, world_to_tile, Projection, ISO_LIFT,
    ISO_TILE_H, ISO_TILE_W, TILE_SIZE,
};
pub use features::{MapFeatures, RiverCrossing, SiteHint, SiteKind, Surface};
pub use gen::{generate, generate_map, generate_map_with};
pub use grid::{MapGrid, DEFAULT_MAP_HEIGHT, DEFAULT_MAP_SEED, DEFAULT_MAP_WIDTH};
pub use measure::Composition;
pub use options::{
    CompositionTargets, MapGenOptions, MapSize, ResourceSpread, TerrainStyle, WaterStyle,
};
pub use portal::Portal;
pub use tile::{TerrainKind, Tile};

use serde::{Deserialize, Serialize};

/// Vertical / depth layer for track and tiles.
///
/// MVP uses [`Layer::Ground`] only; other variants reserve the seam for
/// tunnels and elevated construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Layer {
    #[default]
    Ground,
    Elevated,
    Underground,
}

/// Facing for edge portals (neighbor handoff later).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeFacing {
    North,
    East,
    South,
    West,
}

/// Stub identity for a map-edge portal. Closed in single-player MVP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PortalId(pub u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_is_default_layer() {
        assert_eq!(Layer::default(), Layer::Ground);
    }
}
