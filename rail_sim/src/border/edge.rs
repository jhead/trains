//! The four map edges, and which one a tile sits on.
//!
//! A map has four edges and each can host exactly one border link
//! (`12-multiplayer.md` §3.1), so the edge *is* the slot: there is no separate
//! link-slot id to keep in step with it.
//!
//! Orientation matches `rail_map`: tile `(0, 0)` is the south-west corner, so
//! [`BorderEdge::North`] is `y == height - 1` and [`BorderEdge::South`] is
//! `y == 0`. `rail_map::EdgeFacing` converts both ways in `rail_map::portal`.

use serde::{Deserialize, Serialize};

use crate::ids::TileCoord;

/// One of the four map boundaries.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub enum BorderEdge {
    #[default]
    North,
    East,
    South,
    West,
}

impl BorderEdge {
    /// Every edge, in a stable order (north, east, south, west).
    pub const ALL: [BorderEdge; 4] = [Self::North, Self::East, Self::South, Self::West];

    /// Index into [`Self::ALL`] — also the echo seed's edge salt, so it must
    /// stay stable forever.
    pub fn index(self) -> usize {
        match self {
            Self::North => 0,
            Self::East => 1,
            Self::South => 2,
            Self::West => 3,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// Title case, for panel headings.
    pub fn title(self) -> &'static str {
        match self {
            Self::North => "North",
            Self::East => "East",
            Self::South => "South",
            Self::West => "West",
        }
    }

    /// Lower case, for sentences ("the north border").
    pub fn label(self) -> &'static str {
        match self {
            Self::North => "north",
            Self::East => "east",
            Self::South => "south",
            Self::West => "west",
        }
    }

    /// Unit step from the map toward this edge and beyond it.
    ///
    /// The Border Yard is drawn by walking this direction out of the portal
    /// tile, so it is the only place presentation needs to know the geometry.
    pub fn outward(self) -> (i32, i32) {
        match self {
            Self::North => (0, 1),
            Self::East => (1, 0),
            Self::South => (0, -1),
            Self::West => (-1, 0),
        }
    }

    /// Tile `steps` beyond `from` in this edge's outward direction.
    ///
    /// Deliberately returns coordinates *off* the map: the yard is a strip of
    /// world past the boundary, and nothing in it may ever occupy a real tile.
    pub fn beyond(self, from: TileCoord, steps: i32) -> TileCoord {
        let (dx, dy) = self.outward();
        TileCoord {
            x: from.x + dx * steps,
            y: from.y + dy * steps,
        }
    }
}

/// Which edge `tile` lies on, or `None` when it is not on the boundary.
///
/// Corners belong to whichever edge comes first in [`BorderEdge::ALL`], so a
/// corner tile always resolves to exactly one link rather than two.
pub fn edge_for_tile(width: u32, height: u32, tile: TileCoord) -> Option<BorderEdge> {
    if width == 0 || height == 0 {
        return None;
    }
    if tile.x < 0 || tile.y < 0 || tile.x as u32 >= width || tile.y as u32 >= height {
        return None;
    }
    if tile.y as u32 == height - 1 {
        return Some(BorderEdge::North);
    }
    if tile.x as u32 == width - 1 {
        return Some(BorderEdge::East);
    }
    if tile.y == 0 {
        return Some(BorderEdge::South);
    }
    if tile.x == 0 {
        return Some(BorderEdge::West);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_edge_round_trips_its_index() {
        for (i, edge) in BorderEdge::ALL.iter().enumerate() {
            assert_eq!(edge.index(), i);
            assert_eq!(BorderEdge::from_index(i), Some(*edge));
        }
        assert_eq!(BorderEdge::from_index(4), None);
    }

    #[test]
    fn tiles_resolve_to_the_edge_they_touch() {
        let (w, h) = (8u32, 6u32);
        assert_eq!(
            edge_for_tile(w, h, TileCoord { x: 3, y: 5 }),
            Some(BorderEdge::North)
        );
        assert_eq!(
            edge_for_tile(w, h, TileCoord { x: 7, y: 2 }),
            Some(BorderEdge::East)
        );
        assert_eq!(
            edge_for_tile(w, h, TileCoord { x: 3, y: 0 }),
            Some(BorderEdge::South)
        );
        assert_eq!(
            edge_for_tile(w, h, TileCoord { x: 0, y: 2 }),
            Some(BorderEdge::West)
        );
        assert_eq!(edge_for_tile(w, h, TileCoord { x: 3, y: 3 }), None);
        assert_eq!(edge_for_tile(w, h, TileCoord { x: 99, y: 3 }), None);
    }

    #[test]
    fn a_corner_belongs_to_exactly_one_edge() {
        // North-east corner: north wins, so it can never open two links at once.
        assert_eq!(
            edge_for_tile(8, 6, TileCoord { x: 7, y: 5 }),
            Some(BorderEdge::North)
        );
    }

    #[test]
    fn the_yard_lies_outside_the_map() {
        let portal = TileCoord { x: 3, y: 5 };
        assert_eq!(
            BorderEdge::North.beyond(portal, 2),
            TileCoord { x: 3, y: 7 }
        );
        assert_eq!(BorderEdge::West.beyond(portal, 1), TileCoord { x: 2, y: 5 });
    }
}
