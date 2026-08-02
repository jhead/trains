//! Place / demolish / autofill mutations against [`TrackNetwork`] + [`Money`].

use crate::economy::{MoneyCategory, MoneyLedger};
use crate::ids::{TileCoord, TrackId};
use crate::money::Money;

use super::cost::tile_cost;
use super::network::TrackNetwork;
use super::piece::{TrackKind, TrackPiece};
use super::rules::{path_bridge_spans_ok, validate_tile_empty, PlacementError};
use super::terrain::TrackTerrain;

/// Straight line between anchors for autofill (orthogonal or 45° diagonal).
///
/// Returns `None` if the segment is not axis-aligned or equal-step diagonal.
pub fn straight_line(from: TileCoord, to: TileCoord) -> Option<Vec<TileCoord>> {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    if dx == 0 && dy == 0 {
        return Some(vec![from]);
    }
    let adx = dx.unsigned_abs();
    let ady = dy.unsigned_abs();
    let steps = adx.max(ady);
    if adx != 0 && ady != 0 && adx != ady {
        return None;
    }
    let step_x = dx.signum();
    let step_y = dy.signum();
    let mut out = Vec::with_capacity(steps as usize + 1);
    for i in 0..=steps {
        out.push(TileCoord {
            x: from.x + step_x * i as i32,
            y: from.y + step_y * i as i32,
        });
    }
    Some(out)
}

/// Result of a successful place (single tile).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedTrack {
    pub id: TrackId,
    pub piece: TrackPiece,
}

/// Try to place one track tile, debiting [`Money`].
pub fn try_place_track(
    network: &mut TrackNetwork,
    money: &mut Money,
    ledger: &mut MoneyLedger,
    terrain: &TrackTerrain,
    tile: TileCoord,
    layer: u8,
) -> Result<PlacedTrack, PlacementError> {
    let is_bridge = validate_tile_empty(network, terrain, tile, layer)?;
    let cost = tile_cost(is_bridge);
    ledger
        .try_debit(money, MoneyCategory::Construction, cost)
        .map_err(|_| PlacementError::InsufficientFunds)?;

    let height = terrain.height_at(tile).unwrap_or(0);
    let id = network.alloc_id();
    let piece = TrackPiece {
        id,
        tile,
        layer,
        kind: if is_bridge {
            TrackKind::Bridge
        } else {
            TrackKind::Ground
        },
        height,
        links: super::dir::TrackLinks::empty(),
        max_grade: 0,
        curve: 0,
        paid_cents: cost,
    };
    network.insert_piece(piece.clone());
    let piece = network.piece(id).cloned().unwrap_or(piece);
    Ok(PlacedTrack { id, piece })
}

/// Demolish by id and credit a full refund of `paid_cents`.
pub fn try_demolish(
    network: &mut TrackNetwork,
    money: &mut Money,
    ledger: &mut MoneyLedger,
    track: TrackId,
) -> Result<TrackPiece, PlacementError> {
    let piece = network
        .remove_piece(track)
        .ok_or(PlacementError::UnknownTrack)?;
    ledger.credit(money, MoneyCategory::Construction, piece.paid_cents);
    Ok(piece)
}

/// Auto-fill a straight run between anchors (inclusive). Skips tiles that
/// already have track; fails the whole command if any *new* tile is illegal
/// or funds cannot cover the sum of new tiles.
pub fn try_autofill_track(
    network: &mut TrackNetwork,
    money: &mut Money,
    ledger: &mut MoneyLedger,
    terrain: &TrackTerrain,
    from: TileCoord,
    to: TileCoord,
    layer: u8,
) -> Result<Vec<PlacedTrack>, PlacementError> {
    let path = straight_line(from, to).ok_or(PlacementError::NotStraight)?;

    // Validate path bridge runs as a whole (consecutive water on the line).
    path_bridge_spans_ok(terrain, &path)?;

    let mut to_place: Vec<(TileCoord, bool)> = Vec::new();
    for tile in &path {
        if network.id_at(*tile, layer).is_some() {
            continue;
        }
        // Per-tile checks (bounds / layer / local water width).
        let is_bridge = validate_tile_empty(network, terrain, *tile, layer)?;
        to_place.push((*tile, is_bridge));
    }

    let total: i64 = to_place.iter().map(|(_, b)| tile_cost(*b)).sum();
    ledger
        .try_debit(money, MoneyCategory::Construction, total)
        .map_err(|_| PlacementError::InsufficientFunds)?;

    let mut placed = Vec::with_capacity(to_place.len());
    for (tile, is_bridge) in to_place {
        let height = terrain.height_at(tile).unwrap_or(0);
        let cost = tile_cost(is_bridge);
        let id = network.alloc_id();
        let piece = TrackPiece {
            id,
            tile,
            layer,
            kind: if is_bridge {
                TrackKind::Bridge
            } else {
                TrackKind::Ground
            },
            height,
            links: super::dir::TrackLinks::empty(),
            max_grade: 0,
            curve: 0,
            paid_cents: cost,
        };
        network.insert_piece(piece);
        let piece = network.piece(id).cloned().expect("just inserted");
        placed.push(PlacedTrack { id, piece });
    }
    Ok(placed)
}
