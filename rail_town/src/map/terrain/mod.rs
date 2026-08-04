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

//! # Isometric evaluation prototype
//!
//! [`iso`] replaces the four of them at the plugin: it draws one diamond per
//! tile plus cliff faces, from its own atlas, keyed on the same
//! (material, shade, variant) triple [`material`] hands the flat renderer. The
//! top-down pipeline below is left intact and still tested — it is simply not
//! registered on this branch, so switching back is a plugin edit.

// The top-down pipeline, kept whole and still tested but not registered.
#[allow(dead_code)]
pub mod atlas;
#[allow(dead_code)]
pub mod autotile;
#[allow(dead_code)]
pub mod chunk;
pub mod iso;
pub mod material;

#[allow(unused_imports)]
pub use chunk::{rebuild_dirty_terrain, setup_terrain, TerrainDirty};
