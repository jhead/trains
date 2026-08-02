//! Path proposal for drag-to-build / demolish ghosts.
//!
//! The proposal snaps a drag onto one of the sixteen track directions (brief 01
//! §5.2): the eight compass rays, plus the eight knight's-move half-steps that
//! reach two tiles along one axis and one along the other. A half-step run is
//! *sparse* — the tiles it crosses stay bare, because that is the only way the
//! shallow link can exist (see [`rail_sim::straight_line`]).
//!
//! # Stability
//!
//! Brief 04 §2.2: a flickering ghost is unusable. Doubling the ray count
//! doubles the number of boundaries a drag can wobble across, and a wobble here
//! is not one tile changing — it is the whole run swinging onto a different
//! angle.
//!
//! The stabiliser is a **detent**, not a timer: a half-step ray has to beat the
//! best compass ray by a real margin ([`HALF_STEP_DETENT`]) before it wins, and
//! ties resolve to the shorter run on the lower-indexed direction. That makes
//! the proposal a pure, deterministic function of the anchor and the cursor tile
//! with a dead band around every compass ray — so it cannot oscillate for a held
//! cursor, cannot depend on frame rate or on the order systems ran in, and is
//! directly testable. Time-based hysteresis has none of those properties and
//! would need state threaded through a call site this module does not own.

use rail_sim::ids::TileCoord;
use rail_sim::straight_line;
use rail_sim::track::{is_half_step, DIR16, DIR_COUNT};

/// How much better, in squared tiles, a half-step ray must be before it wins.
///
/// This is the dead band that keeps a drag from swinging between a compass
/// angle and the shallow angle beside it. One squared tile is the smallest
/// margin that means anything on a tile grid.
pub const HALF_STEP_DETENT: i32 = 1;

/// How the cursor maps onto a proposed tile run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PathMode {
    /// Snap the endpoint onto the nearest of the sixteen rays (default drag).
    #[default]
    Autofill,
    /// Require the drag to land exactly on one of the sixteen (Shift).
    ExactStraight,
    /// Exactly the cursor tile (Ctrl).
    SingleTile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedPath {
    pub tiles: Vec<TileCoord>,
    /// Endpoint after snapping (equals `to` when no snap applied).
    pub endpoint: TileCoord,
    /// True when [`PathMode::ExactStraight`] and the drag is off every ray.
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
            let endpoint = snap_to_direction(from, to);
            let tiles = straight_line(from, endpoint).unwrap_or_else(|| vec![from]);
            ProposedPath {
                tiles,
                endpoint,
                not_straight: false,
            }
        }
    }
}

/// Snap `to` onto the nearest of the sixteen rays from `from`.
///
/// Half-steps carry [`HALF_STEP_DETENT`]; see the module docs.
pub fn snap_to_direction(from: TileCoord, to: TileCoord) -> TileCoord {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    if dx == 0 && dy == 0 {
        return to;
    }

    let mut best_compass: Option<(i32, TileCoord)> = None;
    let mut best_half: Option<(i32, TileCoord)> = None;

    for dir in 0..DIR_COUNT {
        let Some((error, endpoint)) = ray_candidate(from, dx, dy, dir) else {
            continue;
        };
        let slot = if is_half_step(dir) {
            &mut best_half
        } else {
            &mut best_compass
        };
        // Strict `<` keeps the lowest-indexed direction on a tie, so the choice
        // never depends on iteration incidentals.
        if slot.is_none_or(|(best, _)| error < best) {
            *slot = Some((error, endpoint));
        }
    }

    match (best_compass, best_half) {
        (Some((c_err, c_end)), Some((h_err, h_end))) => {
            if h_err.saturating_add(HALF_STEP_DETENT) <= c_err {
                h_end
            } else {
                c_end
            }
        }
        (Some((_, c_end)), None) => c_end,
        (None, Some((_, h_end))) => h_end,
        (None, None) => to,
    }
}

/// Best endpoint on the ray `from + n * DIR16[dir]` for `n >= 1`, with its
/// squared distance to the cursor.
///
/// The exact projection is rarely a whole number of steps, so both whole steps
/// either side are tried and the nearer wins; a tie takes the shorter run, which
/// is what makes a drag grow rather than jump.
fn ray_candidate(from: TileCoord, dx: i32, dy: i32, dir: usize) -> Option<(i32, TileCoord)> {
    let (sx, sy) = DIR16[dir];
    let len_sq = sx * sx + sy * sy;
    let projection = (dx * sx + dy * sy) as f32 / len_sq as f32;
    if projection < 0.5 {
        // The cursor is behind this ray; it has nothing to offer.
        return None;
    }
    let low = (projection.floor() as i32).max(1);
    let mut best: Option<(i32, TileCoord)> = None;
    for n in [low, low + 1] {
        let end = TileCoord {
            x: from.x + sx * n,
            y: from.y + sy * n,
        };
        let ex = end.x - from.x - dx;
        let ey = end.y - from.y - dy;
        let error = ex * ex + ey * ey;
        if best.is_none_or(|(b, _)| error < b) {
            best = Some((error, end));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    #[test]
    fn snap_keeps_exact_diagonal() {
        let from = tile(2, 2);
        let to = tile(5, 5);
        assert_eq!(snap_to_direction(from, to), to);
        let p = propose_path(from, to, PathMode::Autofill);
        assert_eq!(p.tiles.len(), 4);
        assert!(!p.not_straight);
    }

    #[test]
    fn snap_pulls_off_axis_to_nearest_ray() {
        let from = tile(0, 0);
        // Closer to horizontal than to anything shallower.
        assert_eq!(snap_to_direction(from, tile(5, 1)), tile(5, 0));
        // Closer to the diagonal.
        assert_eq!(snap_to_direction(from, tile(5, 4)), tile(4, 4));
    }

    #[test]
    fn every_ray_snaps_to_itself() {
        let from = tile(8, 8);
        for dir in 0..DIR_COUNT {
            let (sx, sy) = DIR16[dir];
            for n in 1..=3 {
                let on_ray = tile(from.x + sx * n, from.y + sy * n);
                assert_eq!(
                    snap_to_direction(from, on_ray),
                    on_ray,
                    "dir {dir} at {n} steps should be its own snap"
                );
            }
        }
    }

    /// The widening, from the player's side: a shallow drag now gets a shallow
    /// run instead of being rounded up to 45°.
    #[test]
    fn a_shallow_drag_gets_a_half_step_run() {
        let from = tile(0, 0);
        // 26.57° is exactly ENE.
        let p = propose_path(from, tile(6, 3), PathMode::Autofill);
        assert_eq!(p.endpoint, tile(6, 3));
        assert_eq!(
            p.tiles,
            vec![tile(0, 0), tile(2, 1), tile(4, 2), tile(6, 3)],
            "a half-step run is sparse - the crossed tiles stay bare"
        );
    }

    #[test]
    fn exact_straight_accepts_a_half_step_and_still_rejects_the_rest() {
        let from = tile(0, 0);
        let p = propose_path(from, tile(2, 1), PathMode::ExactStraight);
        assert!(!p.not_straight, "a knight's move is now a direction");
        assert_eq!(p.tiles, vec![tile(0, 0), tile(2, 1)]);

        // (3, 1) lies along none of the sixteen.
        let q = propose_path(from, tile(3, 1), PathMode::ExactStraight);
        assert!(q.not_straight);
        assert_eq!(q.tiles, vec![from]);
    }

    #[test]
    fn single_tile_ignores_anchor() {
        let from = tile(0, 0);
        let to = tile(3, 7);
        let p = propose_path(from, to, PathMode::SingleTile);
        assert_eq!(p.tiles, vec![to]);
        assert_eq!(p.endpoint, to);
    }

    /// The detent: near a compass ray the proposal stays on the compass ray, so
    /// the ghost does not swing onto a shallow angle the player did not ask for.
    #[test]
    fn a_near_axis_drag_holds_the_compass_ray() {
        let from = tile(0, 0);
        // 14° from due east — the shallow ray ties on distance but loses the
        // detent, which is the dead band doing its job.
        assert_eq!(snap_to_direction(from, tile(4, 1)), tile(4, 0));
        assert_eq!(snap_to_direction(from, tile(8, 1)), tile(8, 0));
        // Commit properly to the shallow angle and it is offered.
        assert_eq!(snap_to_direction(from, tile(4, 2)), tile(4, 2));
    }

    /// Stability under a drag: sweeping the cursor away from the anchor must
    /// never make the proposed run shorter or swing it off its ray. A pure
    /// function of the tile pair cannot flicker in place; this covers the other
    /// half — flicker as the cursor moves.
    #[test]
    fn dragging_outward_along_a_ray_never_swings_the_proposal() {
        let from = tile(20, 20);
        for dir in 0..DIR_COUNT {
            let (sx, sy) = DIR16[dir];
            let mut last_len = 0usize;
            for n in 1..=8 {
                let cursor = tile(from.x + sx * n, from.y + sy * n);
                let p = propose_path(from, cursor, PathMode::Autofill);
                assert_eq!(p.endpoint, cursor, "dir {dir} at {n} drifted off its ray");
                assert!(
                    p.tiles.len() >= last_len,
                    "dir {dir} shortened from {last_len} at step {n}"
                );
                last_len = p.tiles.len();
            }
        }
    }

    /// Every snap lands on something the sim will actually build.
    #[test]
    fn every_snap_result_is_a_real_run() {
        let from = tile(30, 30);
        for x in -9..=9 {
            for y in -9..=9 {
                let cursor = tile(from.x + x, from.y + y);
                let end = snap_to_direction(from, cursor);
                assert!(
                    straight_line(from, end).is_some(),
                    "snap of ({x},{y}) gave {end:?}, which is not a run"
                );
                let p = propose_path(from, cursor, PathMode::Autofill);
                assert!(!p.tiles.is_empty());
                assert_eq!(p.tiles.first(), Some(&from));
                assert_eq!(p.tiles.last(), Some(&end));
            }
        }
    }

    /// Snapping is idempotent: re-snapping a snapped endpoint is a no-op, so a
    /// proposal can never chase its own tail.
    #[test]
    fn snapping_is_idempotent() {
        let from = tile(30, 30);
        for x in -9..=9 {
            for y in -9..=9 {
                let once = snap_to_direction(from, tile(from.x + x, from.y + y));
                assert_eq!(snap_to_direction(from, once), once);
            }
        }
    }
}
