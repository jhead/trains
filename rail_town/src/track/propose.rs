//! Path proposal for drag-to-build / demolish ghosts.
//!
//! Phase A: orthogonal + 45° only. Smart A* routing is Phase C.

use rail_sim::ids::TileCoord;
use rail_sim::straight_line;

/// How the cursor maps onto a proposed tile run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PathMode {
    /// Snap endpoint onto nearest ortho / 45° ray (default drag).
    #[default]
    Autofill,
    /// Require an exact ortho / 45° segment (Shift).
    ExactStraight,
    /// Exactly the cursor tile (Ctrl).
    SingleTile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedPath {
    pub tiles: Vec<TileCoord>,
    /// Endpoint after snapping (equals `to` when no snap applied).
    pub endpoint: TileCoord,
    /// True when [`PathMode::ExactStraight`] and the drag is off-axis.
    pub not_straight: bool,
}

/// Propose tiles from `from` toward `to` under `mode`.
pub fn propose_path(from: TileCoord, to: TileCoord, mode: PathMode) -> ProposedPath {
    match mode {
        PathMode::SingleTile => ProposedPath {
            tiles: vec![to],
            endpoint: to,
            not_straight: false,
        },
        PathMode::ExactStraight => match straight_line(from, to) {
            Some(tiles) => ProposedPath {
                endpoint: to,
                tiles,
                not_straight: false,
            },
            None => ProposedPath {
                tiles: vec![from],
                endpoint: to,
                not_straight: true,
            },
        },
        PathMode::Autofill => {
            let endpoint = snap_to_ortho45(from, to);
            let tiles = straight_line(from, endpoint).unwrap_or_else(|| vec![from]);
            ProposedPath {
                tiles,
                endpoint,
                not_straight: false,
            }
        }
    }
}

/// Snap `to` onto the nearest orthogonal or 45° ray from `from`.
pub fn snap_to_ortho45(from: TileCoord, to: TileCoord) -> TileCoord {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    if dx == 0 && dy == 0 {
        return to;
    }
    let sx = dx.signum();
    let sy = dy.signum();
    let adx = dx.unsigned_abs() as i32;
    let ady = dy.unsigned_abs() as i32;

    let candidates = [
        TileCoord {
            x: from.x + dx,
            y: from.y,
        },
        TileCoord {
            x: from.x,
            y: from.y + dy,
        },
        TileCoord {
            x: from.x + sx * adx.min(ady),
            y: from.y + sy * adx.min(ady),
        },
    ];

    candidates
        .into_iter()
        .min_by_key(|c| {
            let ex = c.x - to.x;
            let ey = c.y - to.y;
            ex * ex + ey * ey
        })
        .expect("three candidates")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_keeps_exact_diagonal() {
        let from = TileCoord { x: 2, y: 2 };
        let to = TileCoord { x: 5, y: 5 };
        assert_eq!(snap_to_ortho45(from, to), to);
        let p = propose_path(from, to, PathMode::Autofill);
        assert_eq!(p.tiles.len(), 4);
        assert!(!p.not_straight);
    }

    #[test]
    fn snap_pulls_off_axis_to_nearest_ray() {
        let from = TileCoord { x: 0, y: 0 };
        // Closer to horizontal than diagonal.
        let to = TileCoord { x: 5, y: 1 };
        assert_eq!(snap_to_ortho45(from, to), TileCoord { x: 5, y: 0 });
        // Closer to diagonal.
        let to2 = TileCoord { x: 5, y: 4 };
        assert_eq!(snap_to_ortho45(from, to2), TileCoord { x: 4, y: 4 });
    }

    #[test]
    fn exact_straight_flags_knight_move() {
        let from = TileCoord { x: 0, y: 0 };
        let to = TileCoord { x: 2, y: 1 };
        let p = propose_path(from, to, PathMode::ExactStraight);
        assert!(p.not_straight);
        assert_eq!(p.tiles, vec![from]);
    }

    #[test]
    fn single_tile_ignores_anchor() {
        let from = TileCoord { x: 0, y: 0 };
        let to = TileCoord { x: 3, y: 7 };
        let p = propose_path(from, to, PathMode::SingleTile);
        assert_eq!(p.tiles, vec![to]);
        assert_eq!(p.endpoint, to);
    }
}
