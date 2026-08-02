//! Place / demolish / autofill mutations against [`TrackNetwork`] + [`Money`].

use crate::economy::{MoneyCategory, MoneyLedger};
use crate::ids::{TileCoord, TrackId};
use crate::money::Money;

use super::cost::tile_build_cost;
use super::dir::{dir_index, is_half_step, DIR16, DIR_COUNT};
use super::network::TrackNetwork;
use super::piece::{TrackKind, TrackPiece};
use super::rules::{
    half_step_run_clear, path_bridge_spans_ok, path_grades_ok, validate_tile_empty, PlacementError,
};
use super::terrain::TrackTerrain;

/// Straight run between anchors for autofill, along any of the sixteen
/// directions.
///
/// Orthogonal and 45° runs are contiguous, exactly as before. A **half-step run
/// is sparse**: its tiles are two apart on one axis and one on the other, and
/// the tiles in between deliberately stay empty, because a half-step link only
/// exists while the tiles it crosses are free of track (see
/// [`TrackNetwork`](super::network::TrackNetwork) module docs). Laying the gaps
/// in would turn a shallow run into a staircase of 45° corners.
///
/// Returns `None` if the segment does not lie along one of the sixteen.
pub fn straight_line(from: TileCoord, to: TileCoord) -> Option<Vec<TileCoord>> {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    if dx == 0 && dy == 0 {
        return Some(vec![from]);
    }
    let (dir, steps) = run_direction(dx, dy)?;
    let (sx, sy) = DIR16[dir];
    let mut out = Vec::with_capacity(steps as usize + 1);
    for i in 0..=steps {
        out.push(TileCoord {
            x: from.x + sx * i as i32,
            y: from.y + sy * i as i32,
        });
    }
    Some(out)
}

/// The [`DIR16`] direction a `(dx, dy)` offset runs along, and how many steps.
///
/// Compass directions are tried first so an offset that is both — there are
/// none, but the ordering keeps the eight-direction results bit-identical —
/// resolves the old way.
pub fn run_direction(dx: i32, dy: i32) -> Option<(usize, u32)> {
    if dx == 0 && dy == 0 {
        return None;
    }
    for dir in 0..DIR_COUNT {
        let (sx, sy) = DIR16[dir];
        // n * (sx, sy) == (dx, dy) for some positive integer n.
        let n = if sx != 0 { dx / sx } else { dy / sy };
        if n > 0 && sx * n == dx && sy * n == dy {
            return Some((dir, n as u32));
        }
    }
    None
}

/// The direction an autofill run travels, from its first two tiles.
fn path_direction(path: &[TileCoord]) -> Option<usize> {
    let (&a, &b) = (path.first()?, path.get(1)?);
    dir_index(a, b)
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
    let cost = tile_build_cost(terrain, tile)?;
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

    // Validate path bridge runs and grades as a whole.
    path_bridge_spans_ok(terrain, &path)?;
    path_grades_ok(terrain, &path)?;

    // A half-step run only connects while the tiles it crosses stay clear, so
    // refuse the whole command rather than charging for a run that would land as
    // disconnected stubs.
    if let Some(dir) = path_direction(&path).filter(|&d| is_half_step(d)) {
        for leg_start in path.iter().take(path.len().saturating_sub(1)) {
            half_step_run_clear(network, *leg_start, layer, dir)?;
        }
    }

    let mut to_place: Vec<TileCoord> = Vec::new();
    for tile in &path {
        if network.id_at(*tile, layer).is_some() {
            continue;
        }
        // Per-tile checks (bounds / layer / water / grade to existing / terrain).
        let _is_bridge = validate_tile_empty(network, terrain, *tile, layer)?;
        to_place.push(*tile);
    }

    let mut costs = Vec::with_capacity(to_place.len());
    let mut total = 0i64;
    for &tile in &to_place {
        let cost = tile_build_cost(terrain, tile)?;
        total = total.saturating_add(cost);
        costs.push(cost);
    }
    ledger
        .try_debit(money, MoneyCategory::Construction, total)
        .map_err(|_| PlacementError::InsufficientFunds)?;

    let mut placed = Vec::with_capacity(to_place.len());
    for (tile, cost) in to_place.into_iter().zip(costs) {
        let is_bridge = terrain.is_water(tile);
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
        network.insert_piece(piece);
        let piece = network.piece(id).cloned().expect("just inserted");
        placed.push(PlacedTrack { id, piece });
    }
    Ok(placed)
}
