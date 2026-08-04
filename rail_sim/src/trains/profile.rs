//! Distinct Transit vs Transport constraint profiles.
//!
//! Price alone is not a profile — speed, grade tolerance, opex, and dwell
//! differ so the two kinds want different routes across the same terrain.
//! See `docs/design/07-trains-and-lines.md` §3.
//!
//! # How fast a train is
//!
//! Binding standard: [`docs/design/17-time-and-pacing.md`](../../../docs/design/17-time-and-pacing.md) §4.
//!
//! Speed is stated in **ticks to cross a flat straight tile**, and a tick is
//! 1/64 of a real second at 1x. Transit's `base_ticks` is therefore two numbers
//! at once and both are load-bearing:
//!
//! | | value |
//! | --- | --- |
//! | Sim time per tile | **one sim-minute** (`6` ticks x 10 sim-seconds) |
//! | Real time per tile | 0.094 s — **10.7 tiles a real second** at 1x |
//!
//! It used to be `3`, which is 21.3 tiles a real second: a transit crossed a
//! standard 64-tile map in three seconds and ran the opening line's twenty-tile
//! round trip 57 times a minute. The owner's report was *"insanely fast"*, and
//! the sim agreed — half a sim-minute a tile is an express doing about 70 km/h
//! between stops a few hundred metres apart.
//!
//! **A tile a minute is the model, and the ceiling on slowing further is the
//! peep on the platform.** A peep walks at
//! [`WALK_TICKS_PER_TILE`](crate::peeps::WALK_TICKS_PER_TILE) = 24 ticks a
//! tile, so transit is exactly four times walking pace. Taking trains much
//! below that would put a railway on a par with going on foot, on screen and in
//! the fiction both. Going slower than a tile a minute means slowing the walk
//! first, and that is a peep-model change, not a train one.

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
    /// Extra ticks per tile for **each car beyond the first**.
    ///
    /// The consist penalty, and one of the two seams 07 §3 left for it. A car is
    /// weight on the drawbar: more of it is slower, and freight — which has the
    /// worse power-to-weight of the two — pays double what transit pays.
    pub car_tick_cost: u16,
    /// Extra dwell ticks for **each car beyond the first**.
    ///
    /// The other seam. Loading a second carriage takes a second carriage's
    /// worth of time, and the platform grade scales the whole figure
    /// ([`StationTier::dwell_ticks`](crate::stations::StationTier::dwell_ticks))
    /// — which is what makes a long consist want a better station.
    pub car_dwell_ticks: u16,
    /// Longest consist this kind will run, in cars. `1` is a single car.
    ///
    /// Transit stops at three because three is the deepest a single pair's
    /// queue ever gets ([`MAX_PENDING_PER_PAIR`](crate::economy::MAX_PENDING_PER_PAIR)),
    /// so a fourth carriage could never be filled by anything the board offers.
    /// Transport stops at one for a reason that is about the world rather than
    /// about the train — see [`TRANSPORT_PROFILE`].
    pub max_cars: u8,
}

/// Transit: brisk, climbs well, cheap to run, short dwell.
///
/// One sim-minute a tile — see the module docs for why that number and not a
/// smaller one. Everything else here is the old profile at the same halved
/// pace, so grade, curve and dwell cost the same *share* of a journey as
/// before: only the clock moved.
pub const TRANSIT_PROFILE: TrainProfile = TrainProfile {
    base_ticks: 6,
    grade_tick_cost: 2,
    curve_div: 16,
    max_grade: 4, // matches track [`MAX_GRADE`](crate::track::MAX_GRADE)
    opex_cents_per_real_min: 14_000,
    dwell_ticks: 4,
    // A sixth slower per carriage, and half again as long at the platform. Both
    // are deliberately small enough that a *filled* car wins and large enough
    // that an *empty* one is a mistake the player can feel: a two-car transit
    // running one load banks 6/7 of what a single car would.
    car_tick_cost: 1,
    car_dwell_ticks: 2,
    max_cars: 3,
};

/// Transport: slow, poor grade tolerance, expensive, long dwell.
///
/// # Why freight runs one wagon
///
/// [`max_cars`](TrainProfile::max_cars) is `1`, and that is a statement about
/// the *world*, not about the locomotive. A car is worth having when there is a
/// queue for it to take, and a passenger queue is real: peep routines produce
/// more departures for a pair than one carriage can lift, and
/// [`drain_peep_demand`](crate::economy::drain_peep_demand) now keeps them
/// instead of dropping them. Industries have no equivalent — an
/// [`Industry`](crate::stations::Industry) produces and consumes a good with no
/// stockpile behind it, so the board carries exactly one working per
/// producer→consumer pair however long the train takes to come back. A second
/// wagon would be a wagon that is always empty and always slowing the train
/// down, sold at a price the player could never earn back.
///
/// The seam is filled in and the number is `1`. Give an industry a stock level
/// and this becomes a two-line change: raise it, and let goods jobs stack the
/// way passenger jobs do. See `docs/design/07-trains-and-lines.md` §3.
pub const TRANSPORT_PROFILE: TrainProfile = TrainProfile {
    base_ticks: 10,
    grade_tick_cost: 4,
    curve_div: 8,
    max_grade: 1,
    opex_cents_per_real_min: 24_000,
    dwell_ticks: 12,
    // Twice transit's drag per car and three times the loading time: freight is
    // the kind that would be worst at running long, if it ran long.
    car_tick_cost: 2,
    car_dwell_ticks: 6,
    max_cars: 1,
};

impl TrainProfile {
    pub fn for_kind(kind: TrainKind) -> Self {
        match kind {
            TrainKind::Transit => TRANSIT_PROFILE,
            TrainKind::Transport => TRANSPORT_PROFILE,
        }
    }

    /// This profile as a consist of `cars` runs it.
    ///
    /// The whole consist model is here: **every car past the first costs time
    /// on the road and time at the platform, and buys one more load.** Applying
    /// it as a modified profile rather than as a parameter on every call means
    /// pathfinding, movement, dwell and the presentation's interpolation all
    /// read the same slowed-down train without any of them having to know that
    /// consists exist.
    ///
    /// `cars` of `0` is read as `1`: a train is never less than a car, and a
    /// zero would otherwise make an empty consist the fastest thing on the map.
    pub fn for_consist(self, cars: u8) -> Self {
        let extra = u16::from(cars.max(1).saturating_sub(1));
        Self {
            base_ticks: self
                .base_ticks
                .saturating_add(extra.saturating_mul(self.car_tick_cost)),
            dwell_ticks: self
                .dwell_ticks
                .saturating_add(extra.saturating_mul(self.car_dwell_ticks)),
            ..self
        }
    }

    /// Cars this kind will run at most, never below one.
    #[inline]
    pub fn cap_cars(self, cars: u8) -> u8 {
        cars.clamp(1, self.max_cars.max(1))
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
        assert_eq!(t, 6);
        assert_eq!(f, 10);
    }

    #[test]
    fn transport_penalises_grade_harder() {
        let t = TRANSIT_PROFILE.ticks_for_piece(2, 0);
        let f = TRANSPORT_PROFILE.ticks_for_piece(2, 0);
        // Transit: 6 + 2*2 = 10; Transport: 10 + 2*4 = 18
        assert_eq!(t, 10);
        assert_eq!(f, 18);
        assert!(f > t);
    }

    /// **The speed claim.** Brief 17 §4: a transit covers one tile per
    /// sim-minute, which is 10.7 tiles a real second at 1x.
    #[test]
    fn a_transit_covers_one_tile_per_sim_minute() {
        use crate::economy::TICKS_PER_SIM_MINUTE;
        assert_eq!(
            i64::from(TRANSIT_PROFILE.ticks_for_piece(0, 0)),
            TICKS_PER_SIM_MINUTE,
            "brief 17 states a tile a sim-minute; the profile has to be that"
        );

        // Ten tiles is the opening beat's separation (design 02 §4.1).
        let ten_tiles = u32::from(TRANSIT_PROFILE.ticks_for_piece(0, 0)) * 10;
        assert_eq!(ten_tiles, 60, "ten tiles is ten sim-minutes");
        // …and 0.94 real seconds of watching, at 64 Hz.
        assert!(
            (0.9..=1.0).contains(&(f64::from(ten_tiles) / 64.0)),
            "a ten-tile journey should take about a real second at 1x, got {}",
            f64::from(ten_tiles) / 64.0
        );
    }

    /// The floor on slowing trains: a railway has to beat walking, visibly.
    #[test]
    fn a_train_is_four_times_walking_pace() {
        let walk = crate::peeps::WALK_TICKS_PER_TILE;
        let transit = u32::from(TRANSIT_PROFILE.ticks_for_piece(0, 0));
        let transport = u32::from(TRANSPORT_PROFILE.ticks_for_piece(0, 0));
        assert_eq!(walk / transit, 4, "transit should be four times the walk");
        assert!(
            transport * 2 < walk,
            "even freight has to comfortably beat going on foot: {transport} \
             against {walk}"
        );
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

    /// **The consist rule, on the profile that owns it.** A car costs road time
    /// and platform time, in that order of magnitude — and a single car is
    /// exactly the train the game had before consists existed.
    #[test]
    fn a_car_costs_road_time_and_platform_time() {
        let one = TRANSIT_PROFILE.for_consist(1);
        assert_eq!(one, TRANSIT_PROFILE, "one car is the profile itself");
        // Zero cars cannot be faster than one — the clamp is what stops a
        // default-constructed consist outrunning the railway.
        assert_eq!(TRANSIT_PROFILE.for_consist(0), TRANSIT_PROFILE);

        let two = TRANSIT_PROFILE.for_consist(2);
        let three = TRANSIT_PROFILE.for_consist(3);
        assert_eq!(two.ticks_for_piece(0, 0), 7, "6 + 1 tick a tile");
        assert_eq!(three.ticks_for_piece(0, 0), 8);
        assert_eq!(two.dwell_ticks, 6, "4 + 2 ticks at the platform");
        assert_eq!(three.dwell_ticks, 8);

        // Freight drags harder per car, which is the profile difference and not
        // an accident of the numbers.
        let heavy = TRANSPORT_PROFILE.for_consist(2);
        assert_eq!(heavy.ticks_for_piece(0, 0), 12, "10 + 2 ticks a tile");
        assert!(
            heavy.base_ticks - TRANSPORT_PROFILE.base_ticks
                > two.base_ticks - TRANSIT_PROFILE.base_ticks
        );

        // Grade and curve are untouched: a car is weight, not a worse driver.
        assert_eq!(two.grade_tick_cost, TRANSIT_PROFILE.grade_tick_cost);
        assert_eq!(two.max_grade, TRANSIT_PROFILE.max_grade);
        assert_eq!(two.opex_cents_per_real_min, TRANSIT_PROFILE.opex_cents_per_real_min);
    }

    /// The cap is per kind, and freight's is one — see [`TRANSPORT_PROFILE`].
    #[test]
    fn the_consist_cap_is_a_property_of_the_kind() {
        assert_eq!(TRANSIT_PROFILE.max_cars, 3);
        assert_eq!(TRANSPORT_PROFILE.max_cars, 1);
        assert_eq!(TRANSIT_PROFILE.cap_cars(9), 3);
        assert_eq!(TRANSIT_PROFILE.cap_cars(0), 1);
        assert_eq!(TRANSPORT_PROFILE.cap_cars(3), 1);
        // Transit's cap and the board's queue depth are the same number on
        // purpose: a fourth carriage could never be filled.
        assert_eq!(
            usize::from(TRANSIT_PROFILE.max_cars),
            crate::economy::MAX_PENDING_PER_PAIR
        );
    }

    /// A longer consist is slower over the same ground, on every leg shape.
    #[test]
    fn a_longer_consist_is_never_quicker() {
        for (grade, curve, length_sq) in [(0, 0, 1), (1, 40, 2), (2, 90, 5)] {
            let mut previous = 0;
            for cars in 1..=3u8 {
                let ticks = TRANSIT_PROFILE.for_consist(cars).ticks_for_leg(grade, curve, length_sq);
                assert!(
                    ticks >= previous,
                    "{cars} cars ran a ({grade},{curve},{length_sq}) leg in {ticks} \
                     against {previous} for one fewer"
                );
                previous = ticks;
            }
        }
    }
}
