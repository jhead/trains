//! Edge portal stubs for future neighbor handoff.

use rail_sim::ids::TileCoord;
use serde::{Deserialize, Serialize};

use crate::{EdgeFacing, Layer, PortalId};

/// A map-edge portal. Closed in single-player MVP — trains turn around / drop cargo later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Portal {
    pub id: PortalId,
    pub facing: EdgeFacing,
    /// Tile on this map that owns the portal.
    pub tile: TileCoord,
    pub layer: Layer,
    /// `false` until a neighbor link is established.
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
}
