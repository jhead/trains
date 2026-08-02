//! Placement validation: bounds, ground layer, water / bridge span.

use crate::ids::TileCoord;

use super::cost::{GROUND_LAYER, MAX_BRIDGE_SPAN};
use super::network::TrackNetwork;
use super::terrain::TrackTerrain;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementError {
    OutOfBounds,
    /// Autofill anchors are not orthogonal or 45° diagonal.
    NotStraight,
    /// Only ground layer (`0`) is editable in MVP.
    InvalidLayer,
    AlreadyOccupied,
    /// Water crossing wider than [`MAX_BRIDGE_SPAN`] on both axes / path run.
    BridgeTooLong { span: u32 },
    InsufficientFunds,
    /// Demolish target missing.
    UnknownTrack,
}

/// Whether a water tile may accept a bridge (narrow enough on at least one axis).
pub fn water_bridge_allowed(terrain: &TrackTerrain, tile: TileCoord) -> Result<(), PlacementError> {
    if !terrain.is_water(tile) {
        return Ok(());
    }
    let h = terrain.water_span_horizontal(tile);
    let v = terrain.water_span_vertical(tile);
    let span = h.min(v);
    if span > MAX_BRIDGE_SPAN {
        Err(PlacementError::BridgeTooLong { span })
    } else {
        Ok(())
    }
}

/// Contiguous water tiles along a polyline (for autofill bridge checks).
///
/// Rejects if any maximal run of consecutive water tiles on the path exceeds
/// [`MAX_BRIDGE_SPAN`].
pub fn path_bridge_spans_ok(terrain: &TrackTerrain, path: &[TileCoord]) -> Result<(), PlacementError> {
    let mut run = 0u32;
    let mut worst = 0u32;
    for &tile in path {
        if !terrain.contains(tile) {
            return Err(PlacementError::OutOfBounds);
        }
        if terrain.is_water(tile) {
            run += 1;
            worst = worst.max(run);
        } else {
            run = 0;
        }
    }
    if worst > MAX_BRIDGE_SPAN {
        Err(PlacementError::BridgeTooLong { span: worst })
    } else {
        Ok(())
    }
}

pub fn validate_tile_empty(
    network: &TrackNetwork,
    terrain: &TrackTerrain,
    tile: TileCoord,
    layer: u8,
) -> Result<bool, PlacementError> {
    if layer != GROUND_LAYER {
        return Err(PlacementError::InvalidLayer);
    }
    if !terrain.contains(tile) {
        return Err(PlacementError::OutOfBounds);
    }
    if network.id_at(tile, layer).is_some() {
        return Err(PlacementError::AlreadyOccupied);
    }
    let is_bridge = terrain.is_water(tile);
    if is_bridge {
        water_bridge_allowed(terrain, tile)?;
    }
    Ok(is_bridge)
}
