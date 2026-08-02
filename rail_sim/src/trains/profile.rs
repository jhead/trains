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
    /// Operating cost per Advance tick while unparked (cents).
    pub opex_cents: i64,
    /// Ticks to wait at a stop after arrival before taking new work.
    pub dwell_ticks: u16,
}

/// Transit: brisk, climbs well, cheap to run, short dwell.
pub const TRANSIT_PROFILE: TrainProfile = TrainProfile {
    base_ticks: 3,
    grade_tick_cost: 1,
    curve_div: 32,
    max_grade: 4, // matches track [`MAX_GRADE`](crate::track::MAX_GRADE)
    opex_cents: 8,
    dwell_ticks: 2,
};

/// Transport: slow, poor grade tolerance, expensive, long dwell.
pub const TRANSPORT_PROFILE: TrainProfile = TrainProfile {
    base_ticks: 5,
    grade_tick_cost: 2,
    curve_div: 16,
    max_grade: 1,
    opex_cents: 16,
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
        let grade = (max_grade as u16).saturating_mul(self.grade_tick_cost);
        let turn = if self.curve_div == 0 {
            0
        } else {
            (curve as u16) / self.curve_div
        };
        self.base_ticks
            .saturating_add(grade)
            .saturating_add(turn)
            .max(1)
    }

    /// True when this profile can climb / run a tile of the given grade.
    pub fn tolerates_grade(self, max_grade: u8) -> bool {
        max_grade <= self.max_grade
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
        assert!(TRANSPORT_PROFILE.opex_cents > TRANSIT_PROFILE.opex_cents);
        assert!(TRANSPORT_PROFILE.dwell_ticks > TRANSIT_PROFILE.dwell_ticks);
    }
}
