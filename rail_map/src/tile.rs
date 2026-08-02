//! Per-tile terrain data.

use serde::{Deserialize, Serialize};

/// Coarse terrain band derived from height / water during generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TerrainKind {
    Water,
    Beach,
    #[default]
    Plains,
    Hills,
    Mountain,
}

/// One cell of the map grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tile {
    /// Elevation band. Negative values are below sea level (typically water).
    pub height: i8,
    pub water: bool,
    pub kind: TerrainKind,
}

impl Tile {
    /// Track may be laid without a bridge when the tile is not water.
    ///
    /// Deep-water bridges (Slice 2) will relax this for short spans.
    #[inline]
    pub fn is_walkable_for_track(&self) -> bool {
        !self.water
    }
}
