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
    try_place_path(network, money, ledger, terrain, &path, layer)
}

/// Lay an arbitrary polyline whose consecutive tiles each step along one of
/// the sixteen directions — the smart-route commit (design 04 §2.2).
///
/// Atomic exactly like [`try_autofill_track`]: the whole path is validated,
/// costed and debited before any piece lands, so a mid-route failure charges
/// nothing and places nothing. Tiles that already carry track are skipped, and
/// a half-step leg anywhere along the path demands its crossed tiles clear,
/// leg by leg, since a bent path has no single run direction.
pub fn try_place_path(
    network: &mut TrackNetwork,
    money: &mut Money,
    ledger: &mut MoneyLedger,
    terrain: &TrackTerrain,
    path: &[TileCoord],
    layer: u8,
) -> Result<Vec<PlacedTrack>, PlacementError> {
    // Every consecutive pair must be one DIR16 step; anything else is not a
    // shape the graph can link. A half-step leg needs its crossed tiles clear
    // of existing track *and* of the rest of this same path — a straight run
    // cannot cross itself, but a bent one can, and placement would sever the
    // link it just paid for.
    let on_path: std::collections::HashSet<(i32, i32)> =
        path.iter().map(|t| (t.x, t.y)).collect();
    for pair in path.windows(2) {
        let dir = dir_index(pair[0], pair[1]).ok_or(PlacementError::NotStraight)?;
        if is_half_step(dir) {
            half_step_run_clear(network, pair[0], layer, dir)?;
            for crossed in super::dir::intermediate_tiles(pair[0], dir)
                .into_iter()
                .flatten()
            {
                if on_path.contains(&(crossed.x, crossed.y)) {
                    return Err(PlacementError::HalfStepBlocked { tile: crossed });
                }
            }
        }
    }

    // Validate path bridge runs and grades as a whole.
    path_bridge_spans_ok(terrain, path)?;
    path_grades_ok(terrain, path)?;

    let mut to_place: Vec<TileCoord> = Vec::new();
    for &tile in path {
        if network.id_at(tile, layer).is_some() {
            continue;
        }
        // Per-tile checks (bounds / layer / water / grade to existing / terrain).
        let _is_bridge = validate_tile_empty(network, terrain, tile, layer)?;
        to_place.push(tile);
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
