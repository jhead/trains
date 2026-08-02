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
//! - [`TILE_SIZE`] — world units per tile edge (32)
//! - [`tile_to_world`] / [`world_to_tile`] / [`map_center_world`]
//!
//! ## Portals
//! - [`MapGrid::portals`] — all edge [`Portal`] stubs (`open: false` in MVP)
//! - [`MapGrid::portal_at`] — portal on a border tile, if any
//! - Facing / id types: [`EdgeFacing`], [`PortalId`], [`Layer`]
//!
//! ## Generation
//! - [`generate_map`] — seeded procedural land / water / elevation + edge portals

mod coords;
mod gen;
mod grid;
mod portal;
mod tile;

pub use coords::{map_center_world, tile_to_world, world_to_tile, TILE_SIZE};
pub use gen::generate_map;
pub use grid::{
    MapGrid, DEFAULT_MAP_HEIGHT, DEFAULT_MAP_SEED, DEFAULT_MAP_WIDTH,
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
