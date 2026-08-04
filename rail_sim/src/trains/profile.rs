//! Distinct Transit vs Transport constraint profiles.
//!
//! Price alone is not a profile — speed, grade tolerance, opex, and dwell
//! differ so the two kinds want different routes across the same terrain.
//! See `docs/design/07-trains-and-lines.md` §3.

use crate::commands::TrainKind;

/// Per-kind movement / cost parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrainProfile {
    /// Ticks to cross a flat straight tile (higher = slower).
    pub base_ticks: u16,
    /// Extra ticks per unit of [`TrackPiece::max_grade`](crate::track::TrackPiece::max_grade).
    pub grade_tick_cost: u16,
    /// Curve units per extra tick (`curve / curve_div`). Lower = more curve drag.
    pub curve_div: u16,
    /// Absolute grade above this is refused (pathfinding + advance).
    pub max_grade: u8,
    /// Operating cost in cents per **real minute**, spread smoothly across
    /// ticks by [`crate::economy::apply_train_opex`]. See
    /// [`crate::economy::opex`] for which minute this is — the crate has two,
    /// 640 apart, and naming the wrong one made every running cost inert.
    ///
    /// Sized against what a train earns rather than what it cost: a transit
    /// working the opening beat grosses around `$1,070` a minute and a
    /// well-shaped line rather more, so `$140` of that is the crew and the
    /// coal. Rolling stock that sits idle on a siding is therefore a slow leak,
    /// which is the point (design 08 §3.3).
    pub opex_cents_per_real_min: i64,
    /// Ticks to wait at a stop after arrival before taking new work.
    pub dwell_ticks: u16,
}

/// Transit: brisk, climbs well, cheap to run, short dwell.
pub const TRANSIT_PROFILE: TrainProfile = TrainProfile {
    base_ticks: 3,
    grade_tick_cost: 1,
    curve_div: 32,
    max_grade: 4, // matches track [`MAX_GRADE`](crate::track::MAX_GRADE)
    opex_cents_per_real_min: 14_000,
    dwell_ticks: 2,
};

/// Transport: slow, poor grade tolerance, expensive, long dwell.
pub const TRANSPORT_PROFILE: TrainProfile = TrainProfile {
    base_ticks: 5,
    grade_tick_cost: 2,
    curve_div: 16,
    max_grade: 1,
    opex_cents_per_real_min: 24_000,
    dwell_ticks: 6,
};

impl TrainProfile {
    pub fn for_kind(kind: TrainKind) -> Self {
        match kind {
            TrainKind::Transit => TRANSIT_PROFILE,
            TrainKind::Transport => TRANSPORT_PROFILE,
        }
    }

    /// Ticks needed to finish the current tile given grade / curve.
    pub fn ticks_for_piece(self, max_grade: u8, curve: u8) -> u16 {
        self.ticks_for_leg(max_grade, curve, 1)
    }

    /// Ticks to cover a leg of `length_sq` tiles-squared at this grade / curve.
    ///
    /// Legs are not all the same length: an orthogonal step is 1 tile, a
    /// diagonal is √2, and a sixteen-direction half-step is √5. Charging every
    /// leg the same time would let a train cover 2.24× the ground per tick on a
    /// shallow run — which would quietly make the longest route the fastest one
    /// and break the design's "shortest, cheapest and fastest are three
    /// different routes".
    ///
    /// Scaling is in integer eighths so the sim stays deterministic; √1 / √2 /
    /// √5 land on 8 / 11 / 18 eighths.
    pub fn ticks_for_leg(self, max_grade: u8, curve: u8, length_sq: u32) -> u16 {
        let grade = (max_grade as u16).saturating_mul(self.grade_tick_cost);
        let turn = if self.curve_div == 0 {
            0
        } else {
            (curve as u16) / self.curve_div
        };
        let flat = self
            .base_ticks
            .saturating_add(grade)
            .saturating_add(turn)
            .max(1);
        let eighths = length_eighths(length_sq);
        (((flat as u32).saturating_mul(eighths) + 4) / 8).max(1) as u16
    }

    /// True when this profile can climb / run a tile of the given grade.
    pub fn tolerates_grade(self, max_grade: u8) -> bool {
        max_grade <= self.max_grade
    }
}

/// Integer-eighths length of a leg from its squared tile length.
///
/// Only three lengths occur on a sixteen-direction square grid, so this is a
/// lookup rather than a square root — exact, branch-cheap, and deterministic
/// across platforms in a way `f32::sqrt` is not guaranteed to be.
fn length_eighths(length_sq: u32) -> u32 {
    match length_sq {
        0 | 1 => 8,  // orthogonal, 1.0
        2 => 11,     // diagonal, 1.414
        5 => 18,     // half-step knight's move, 2.236
        other => {
            // Defensive: round(sqrt(n) * 8) for anything the graph grows later.
            ((other as f64).sqrt() * 8.0).round() as u32
        }
    }
}

#[cfg(test)]
mod leg_tests {
    use super::*;

    #[test]
    fn a_longer_leg_costs_proportionally_more_time() {
        // sqrt(1) : sqrt(2) : sqrt(5)  ->  8 : 11 : 18 eighths.
        assert_eq!(length_eighths(1), 8);
        assert_eq!(length_eighths(2), 11);
        assert_eq!(length_eighths(5), 18);

        let ortho = TRANSIT_PROFILE.ticks_for_leg(0, 0, 1);
        let diag = TRANSIT_PROFILE.ticks_for_leg(0, 0, 2);
        let half = TRANSIT_PROFILE.ticks_for_leg(0, 0, 5);
        assert!(
            half > diag && diag >= ortho,
            "a half-step must not be cheaper than a diagonal: {ortho}/{diag}/{half}"
        );
    }

    #[test]
    fn ground_covered_per_tick_is_roughly_equal_across_leg_kinds() {
        // The whole point: no direction may be a speed exploit. Compare
        // distance-per-tick in eighths, allowing for integer rounding.
        let rate = |length_sq: u32| {
            let ticks = TRANSPORT_PROFILE.ticks_for_leg(0, 0, length_sq) as f64;
            length_eighths(length_sq) as f64 / ticks
        };
        let (o, d, h) = (rate(1), rate(2), rate(5));
        let spread = [o, d, h];
        let max = spread.iter().cloned().fold(f64::MIN, f64::max);
        let min = spread.iter().cloned().fold(f64::MAX, f64::min);
        assert!(
            max / min < 1.25,
            "leg speeds should be within rounding of each other, got {spread:?}"
        );
    }

    #[test]
    fn a_leg_never_costs_zero_ticks() {
        for length_sq in [0, 1, 2, 5, 9] {
            assert!(TRANSIT_PROFILE.ticks_for_leg(0, 0, length_sq) >= 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transit_is_faster_than_transport_on_flat() {
        let t = TRANSIT_PROFILE.ticks_for_piece(0, 0);
        let f = TRANSPORT_PROFILE.ticks_for_piece(0, 0);
        assert!(t < f, "transit {t} should be < transport {f}");
        assert_eq!(t, 3);
        assert_eq!(f, 5);
    }

    #[test]
    fn transport_penalises_grade_harder() {
        let t = TRANSIT_PROFILE.ticks_for_piece(2, 0);
        let f = TRANSPORT_PROFILE.ticks_for_piece(2, 0);
        // Transit: 3 + 2*1 = 5; Transport: 5 + 2*2 = 9
        assert_eq!(t, 5);
        assert_eq!(f, 9);
        assert!(f > t);
    }

    #[test]
    fn freight_refuses_steep_grades_transit_accepts() {
        assert!(TRANSIT_PROFILE.tolerates_grade(4));
        assert!(!TRANSPORT_PROFILE.tolerates_grade(2));
        assert!(TRANSPORT_PROFILE.tolerates_grade(1));
    }

    #[test]
    fn opex_and_dwell_differ() {
        assert!(TRANSPORT_PROFILE.opex_cents_per_real_min > TRANSIT_PROFILE.opex_cents_per_real_min);
        assert!(TRANSPORT_PROFILE.dwell_ticks > TRANSIT_PROFILE.dwell_ticks);
    }
}
