//! Terrain rendering: procedural tile atlas, autotiling, chunked composition.
//!
//! The heightmap is the whole routing puzzle, so it has to be *visible*:
//! "if the player cannot see a ridge, the ridge is not part of the puzzle"
//! ([02 §2.3]). Four pieces do that work:
//!
//! - [`atlas`] paints the tile art procedurally at startup, at the real 32-texel
//!   dimensions and strictly in the binding palette (brief 01 §7).
//! - [`material`] holds the ramps, the height → ramp-step mapping and the
//!   world-anchored hash that picks a tile's flat variant.
//! - [`autotile`] turns tile data into an ordered list of atlas cells: base
//!   variant, material transitions from a 4-bit neighbour mask plus inner
//!   corners, banded cliff faces, and elevation-band contours.
//! - [`chunk`] composites 16 × 16 tiles into one sprite and rebuilds only what
//!   changed (brief 01 §2.5).

pub mod atlas;
pub mod autotile;
pub mod chunk;
pub mod material;

pub use chunk::{rebuild_dirty_terrain, setup_terrain, TerrainDirty};
