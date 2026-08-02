//! Placement validation: bounds, ground layer, water / bridge span, grade, terrain.

use crate::ids::TileCoord;

use super::cost::{
    local_slope, tile_build_cost, GROUND_LAYER, MAX_BRIDGE_SPAN, MAX_GRADE, MOUNTAIN_HEIGHT_MIN,
};
use super::dir::{step, DIR8};
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
    /// Absolute height delta to a neighbour exceeds [`MAX_GRADE`].
    GradeTooSteep { grade: u8 },
    /// Cliff / mountain band — not buildable.
    TerrainForbidden,
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

/// Refuse a path when any consecutive pair exceeds [`MAX_GRADE`].
pub fn path_grades_ok(terrain: &TrackTerrain, path: &[TileCoord]) -> Result<(), PlacementError> {
    for w in path.windows(2) {
        let a = w[0];
        let b = w[1];
        if !terrain.contains(a) || !terrain.contains(b) {
            return Err(PlacementError::OutOfBounds);
        }
        // Water height is a flood tag, not a climb — skip grade on any water leg.
        if terrain.is_water(a) || terrain.is_water(b) {
            continue;
        }
        let ha = terrain.height_at(a).unwrap_or(0);
        let hb = terrain.height_at(b).unwrap_or(0);
        let grade = (ha as i16 - hb as i16).unsigned_abs() as u8;
        if grade > MAX_GRADE {
            return Err(PlacementError::GradeTooSteep { grade });
        }
    }
    Ok(())
}

/// Grade from `tile` to any existing adjacent track on the same layer.
pub fn grade_to_neighbors_ok(
    network: &TrackNetwork,
    terrain: &TrackTerrain,
    tile: TileCoord,
    layer: u8,
) -> Result<(), PlacementError> {
    if terrain.is_water(tile) {
        return Ok(());
    }
    let our_h = terrain.height_at(tile).unwrap_or(0);
    for (i, _) in DIR8.iter().enumerate() {
        let n = step(tile, i);
        if network.id_at(n, layer).is_none() {
            continue;
        }
        if terrain.is_water(n) {
            continue;
        }
        let nh = terrain.height_at(n).unwrap_or(0);
        let grade = (our_h as i16 - nh as i16).unsigned_abs() as u8;
        if grade > MAX_GRADE {
            return Err(PlacementError::GradeTooSteep { grade });
        }
    }
    Ok(())
}

fn land_buildable(terrain: &TrackTerrain, tile: TileCoord) -> Result<(), PlacementError> {
    let height = terrain.height_at(tile).unwrap_or(0);
    if height >= MOUNTAIN_HEIGHT_MIN {
        return Err(PlacementError::TerrainForbidden);
    }
    // Extremely steep local relief (cliff face) even below mountain band.
    let slope = local_slope(terrain, tile);
    if slope > MAX_GRADE + 1 {
        return Err(PlacementError::GradeTooSteep { grade: slope });
    }
    Ok(())
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
    } else {
        land_buildable(terrain, tile)?;
    }
    grade_to_neighbors_ok(network, terrain, tile, layer)?;
    // Ensure cost table accepts the tile (mountain / etc.).
    let _ = tile_build_cost(terrain, tile)?;
    Ok(is_bridge)
}
