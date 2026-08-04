//! Terrain rendering — two renderers for one heightmap.
//!
//! The heightmap is the whole routing puzzle, so it has to be *visible*:
//! "if the player cannot see a ridge, the ridge is not part of the puzzle"
//! ([02 §2.3]). Which renderer answers that depends on the live projection
//! (`crate::map::projection`); exactly one of them is registered at a time and
//! both atlases are baked at startup, so swapping views costs a re-spawn and
//! never a bake.
//!
//! # From above: [`chunk`]
//!
//! - [`atlas`] paints the tile art procedurally at startup, at the real 32-texel
//!   dimensions and strictly in the binding palette (brief 01 §7).
//! - [`material`] holds the ramps, the height → ramp-step mapping and the
//!   world-anchored hash that picks a tile's flat variant. **Shared** — the
//!   isometric atlas is keyed on the same (material, shade, variant) triple, so
//!   a band step is the same value step in both views.
//! - [`autotile`] turns tile data into an ordered list of atlas cells: base
//!   variant, material transitions from a 4-bit neighbour mask plus inner
//!   corners, banded cliff faces, and elevation-band contours.
//! - [`chunk`] composites 16 × 16 tiles into one sprite and rebuilds only what
//!   changed (brief 01 §2.5).
//!
//! # In isometric: [`iso`]
//!
//! One diamond per tile plus up to two cliff faces, from its own atlas. A
//! diamond grid is not a rectangle of rectangles, so none of the chunk
//! compositor above can be reused — and neither can [`autotile`], which is the
//! one thing the isometric view is measurably poorer for. See [`iso`]'s own
//! docs.

pub mod atlas;
pub mod autotile;
pub mod chunk;
pub mod iso;
pub mod material;
