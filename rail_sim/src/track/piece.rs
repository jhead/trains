//! Track piece data and light grade / curve constraints for later train speed.

use bevy_ecs::prelude::Component;
use serde::{Deserialize, Serialize};

use crate::ids::{TileCoord, TrackId};

use super::dir::{clock_separation, TrackLinks};

/// Whether the piece sits on land or spans water.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackKind {
    Ground,
    Bridge,
}

/// One laid track tile (sim state; presentation mirrors this).
#[derive(Debug, Clone, PartialEq, Eq, Component, Serialize, Deserialize)]
pub struct TrackPiece {
    pub id: TrackId,
    pub tile: TileCoord,
    /// MVP: always [`super::cost::GROUND_LAYER`].
    pub layer: u8,
    pub kind: TrackKind,
    /// Terrain height cached at placement (for grade).
    pub height: i8,
    /// Which of the 16 directions currently have a linked neighbour
    /// ([`DIR16`](super::dir::DIR16)). Half-step links reach a tile two along
    /// one axis and one along the other.
    pub links: TrackLinks,
    /// Max absolute height delta to a linked neighbor (0 = flat).
    /// Trains later: slow when this exceeds a threshold.
    pub max_grade: u8,
    /// Turn sharpness at this node (0 = straight / dead-end, higher = sharper).
    /// Derived from link directions; trains later: slow on high values.
    pub curve: u8,
    /// Cents charged when placed — full refund on demolish.
    pub paid_cents: i64,
}

impl TrackPiece {
    pub fn is_bridge(&self) -> bool {
        self.kind == TrackKind::Bridge
    }
}

/// Curve penalty from the set of link direction indices (0–15).
///
/// Measured on the sixteen-point rose ([`clock_separation`]), so a compass-only
/// node scores exactly what it scored on the eight-direction graph: a
/// separation of one rose step is 16, and the old 45°/90°/135° pairs still come
/// out at 32/64/96. Half-step links land on the odd multiples in between, which
/// is the whole point of the widening — a shallow divergence now reads as
/// shallow to the train profiles instead of rounding to the nearest 45°.
///
/// Dead-end / isolated → 0. Two exactly opposed dirs → 0 (straight through).
pub fn curve_from_link_dirs(dirs: &[usize]) -> u8 {
    if dirs.len() < 2 {
        return 0;
    }
    let mut sharpest = 0u8;
    for i in 0..dirs.len() {
        for j in (i + 1)..dirs.len() {
            let sep = clock_separation(dirs[i], dirs[j]);
            // 8 steps apart means opposite (straight through); treat as no turn.
            let turn = if sep == 8 { 0 } else { sep as u8 };
            sharpest = sharpest.max(turn);
        }
    }
    sharpest.saturating_mul(16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::dir::{clock_index, dir_from_clock, DIR_COUNT};

    /// The eight-direction scores this replaced, reproduced exactly.
    fn legacy_curve_8(dirs: &[usize]) -> u8 {
        if dirs.len() < 2 {
            return 0;
        }
        let mut sharpest = 0u8;
        for i in 0..dirs.len() {
            for j in (i + 1)..dirs.len() {
                let mut d = (dirs[i] as i8 - dirs[j] as i8).unsigned_abs();
                if d > 4 {
                    d = 8 - d;
                }
                let turn = if d == 4 { 0 } else { d };
                sharpest = sharpest.max(turn);
            }
        }
        sharpest.saturating_mul(32)
    }

    #[test]
    fn compass_only_nodes_score_exactly_as_before() {
        for a in 0..8usize {
            for b in 0..8usize {
                for c in 0..8usize {
                    let dirs = [a, b, c];
                    assert_eq!(
                        curve_from_link_dirs(&dirs),
                        legacy_curve_8(&dirs),
                        "compass set {dirs:?} changed score"
                    );
                }
            }
        }
    }

    #[test]
    fn a_lone_or_straight_piece_has_no_curve() {
        assert_eq!(curve_from_link_dirs(&[]), 0);
        assert_eq!(curve_from_link_dirs(&[2]), 0);
        for d in 0..DIR_COUNT {
            assert_eq!(
                curve_from_link_dirs(&[d, super::super::dir::opposite_dir(d)]),
                0,
                "opposed pair through {d} should read straight"
            );
        }
    }

    /// A half-step divergence is gentler than the 45° it used to round to.
    #[test]
    fn half_steps_score_between_the_compass_values() {
        // N + NNE — one rose step apart.
        let shallow = curve_from_link_dirs(&[0, 8]);
        // N + NE — two rose steps apart, the old minimum.
        let old_min = curve_from_link_dirs(&[0, 1]);
        assert_eq!(shallow, 16);
        assert_eq!(old_min, 32);
        assert!(shallow < old_min);

        // Every separation maps to a distinct, monotonic score.
        let scores: Vec<u8> = (1..8)
            .map(|s| curve_from_link_dirs(&[dir_from_clock(0), dir_from_clock(s)]))
            .collect();
        assert_eq!(scores, vec![16, 32, 48, 64, 80, 96, 112]);
        assert_eq!(clock_index(dir_from_clock(3)), 3);
    }
}
