//! Map / terrain library for Rail Town.
//!
//! Slice 0: empty stub. Slice 1 adds `MapGrid`, height/water generation,
//! and edge `Portal` stubs.

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
