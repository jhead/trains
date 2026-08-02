//! Edge portals — the map boundary, and the door in it.
//!
//! [`crate::gen::generate_map`] lays one **closed** portal on every border tile,
//! so the boundary is uniform and solo play sees nothing at all. Opening one is
//! a construction project in the simulation (`rail_sim::border`); this module
//! holds the map-side half: which tile, which way it faces, whether it is open,
//! and where the Border Yard is drawn.
//!
//! # Ownership
//!
//! The portal record here is geometry and presentation. The *relationship* — the
//! neighbour, the cached offer, the trains in transit — lives in
//! `rail_sim::border::BorderRegistry`, which is what the save carries. A portal
//! being open mirrors that and is refreshed from it, never the other way round.
//! Two consequences worth keeping: regenerating a map cannot lose a link, and
//! nothing in `rail_map` needs to know what a manifest is.

use rail_sim::ids::TileCoord;
use serde::{Deserialize, Serialize};

use crate::{EdgeFacing, Layer, MapGrid, PortalId};

/// A map-edge portal. Closed until a border link is opened behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Portal {
    pub id: PortalId,
    pub facing: EdgeFacing,
    /// Tile on this map that owns the portal.
    pub tile: TileCoord,
    pub layer: Layer,
    /// `false` until a neighbour link is established.
    pub open: bool,
}

impl Portal {
    pub fn closed(id: PortalId, facing: EdgeFacing, tile: TileCoord) -> Self {
        Self {
            id,
            facing,
            tile,
            layer: Layer::Ground,
            open: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Open or close the door. Idempotent; returns `true` if it changed.
    pub fn set_open(&mut self, open: bool) -> bool {
        let changed = self.open != open;
        self.open = open;
        changed
    }

    /// Tile `steps` beyond the boundary, in the direction this portal faces.
    ///
    /// Deliberately off the map. The Border Yard (`12-multiplayer.md` §3.2) is a
    /// strip of world past the edge, and nothing drawn in it may ever land on a
    /// real tile — constraint §2.2 expressed as geometry.
    pub fn yard_tile(&self, steps: i32) -> TileCoord {
        let (dx, dy) = self.facing.outward();
        TileCoord {
            x: self.tile.x + dx * steps,
            y: self.tile.y + dy * steps,
        }
    }
}

impl EdgeFacing {
    /// Every facing, in index order.
    pub const ALL: [EdgeFacing; 4] = [Self::North, Self::East, Self::South, Self::West];

    /// Stable index — north, east, south, west.
    ///
    /// Matches `rail_sim::border::BorderEdge::index`, which is how the two enums
    /// convert without either crate depending on the other's ordering.
    pub fn index(self) -> usize {
        match self {
            Self::North => 0,
            Self::East => 1,
            Self::South => 2,
            Self::West => 3,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Some(match index {
            0 => Self::North,
            1 => Self::East,
            2 => Self::South,
            3 => Self::West,
            _ => return None,
        })
    }

    /// Unit step from the map out through this edge.
    ///
    /// Tile `(0, 0)` is the south-west corner (see [`crate::coords`]), so north
    /// is `+y`.
    pub fn outward(self) -> (i32, i32) {
        match self {
            Self::North => (0, 1),
            Self::East => (1, 0),
            Self::South => (0, -1),
            Self::West => (-1, 0),
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::North => "North",
            Self::East => "East",
            Self::South => "South",
            Self::West => "West",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::North => "north",
            Self::East => "east",
            Self::South => "south",
            Self::West => "west",
        }
    }

    /// The facing a neighbour on the far side would have.
    pub fn opposite(self) -> Self {
        match self {
            Self::North => Self::South,
            Self::South => Self::North,
            Self::East => Self::West,
            Self::West => Self::East,
        }
    }
}

/// Door controls on the grid itself.
///
/// Inherent methods rather than free functions so they ride along with
/// [`MapGrid`], which is already exported — opening a border needs no new name
/// in `rail_map`'s public root.
impl MapGrid {
    /// Open the portal on `tile`, if there is one. Returns `true` if it changed.
    ///
    /// Called from presentation when `rail_sim::border` reports a link opening,
    /// so the map's own record follows the simulation rather than leading it.
    pub fn open_portal_at(&mut self, tile: TileCoord) -> bool {
        match self.portals_mut().iter_mut().find(|p| p.tile == tile) {
            Some(portal) => portal.set_open(true),
            None => false,
        }
    }

    /// Close every portal on one edge. Returns how many changed.
    ///
    /// Severing a link closes the door and nothing else — the track stays, the
    /// tile stays, and the boundary looks exactly as it did in solo play.
    pub fn close_portals_facing(&mut self, facing: EdgeFacing) -> usize {
        let mut closed = 0;
        for portal in self.portals_mut().iter_mut() {
            if portal.facing == facing && portal.set_open(false) {
                closed += 1;
            }
        }
        closed
    }

    /// The open portal on `facing`, if a link was built there.
    pub fn open_portal_facing(&self, facing: EdgeFacing) -> Option<&Portal> {
        self.portals().iter().find(|p| p.facing == facing && p.open)
    }

    /// Every open portal, in the order they were generated.
    pub fn open_portals(&self) -> impl Iterator<Item = &Portal> {
        self.portals().iter().filter(|p| p.open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate_map;

    #[test]
    fn every_border_tile_starts_closed() {
        let grid = generate_map(16, 12, 42);
        assert!(!grid.portals().is_empty());
        assert!(
            grid.portals().iter().all(|p| !p.is_open()),
            "solo play must see a uniform, shut boundary"
        );
        for portal in grid.portals() {
            assert!(grid.is_border(portal.tile));
        }
    }

    #[test]
    fn opening_and_closing_a_door_leaves_the_map_alone() {
        let mut grid = generate_map(16, 12, 42);
        let tiles_before = grid.tiles().to_vec();
        let count_before = grid.portals().len();

        let tile = grid
            .portals()
            .iter()
            .find(|p| p.facing == EdgeFacing::East)
            .expect("an east portal")
            .tile;

        assert!(grid.open_portal_at(tile));
        assert!(!grid.open_portal_at(tile), "idempotent");
        assert_eq!(
            grid.open_portal_facing(EdgeFacing::East).map(|p| p.tile),
            Some(tile)
        );
        assert_eq!(grid.open_portals().count(), 1);

        assert_eq!(grid.close_portals_facing(EdgeFacing::East), 1);
        assert!(grid.open_portal_facing(EdgeFacing::East).is_none());
        assert_eq!(grid.open_portals().count(), 0);

        assert_eq!(grid.tiles(), tiles_before.as_slice(), "no tile changed");
        assert_eq!(grid.portals().len(), count_before);
    }

    #[test]
    fn opening_a_tile_with_no_portal_does_nothing() {
        let mut grid = generate_map(16, 12, 42);
        assert!(!grid.open_portal_at(TileCoord { x: 8, y: 6 }));
    }

    #[test]
    fn the_yard_is_outside_the_map() {
        let grid = generate_map(16, 12, 42);
        for portal in grid.portals() {
            for steps in 1..4 {
                assert!(
                    !grid.contains(portal.yard_tile(steps)),
                    "{:?} at ({}, {}) would draw the yard on a real tile",
                    portal.facing,
                    portal.tile.x,
                    portal.tile.y
                );
            }
        }
    }

    #[test]
    fn facings_round_trip_their_index_and_invert() {
        for (i, facing) in EdgeFacing::ALL.iter().enumerate() {
            assert_eq!(facing.index(), i);
            assert_eq!(EdgeFacing::from_index(i), Some(*facing));
            assert_eq!(facing.opposite().opposite(), *facing);
            let (dx, dy) = facing.outward();
            let (ox, oy) = facing.opposite().outward();
            assert_eq!((dx + ox, dy + oy), (0, 0));
        }
        assert_eq!(EdgeFacing::from_index(9), None);
    }
}
