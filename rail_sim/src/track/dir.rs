//! Sixteen-way tile directions for track links, autofill and junction art.
//!
//! `docs/design/01-art-direction.md` §5.2 makes sixteen binding: eight is the
//! named failure case of the railgen `spritebank` plate (*"the ties skew and pop
//! between facets as the tangent sweeps"*), and sixteen is where the junction
//! plate's two walls are both still open — a 22.5° step clears the ~10° pixel
//! floor below which a frog cannot be drawn, at about six artist-days of
//! turnout art instead of twenty-five.
//!
//! # Layout, and why the compass comes first
//!
//! [`DIR16`] is **not** sorted by angle. Its first eight entries are exactly the
//! old `DIR8` compass steps, in the old order, so every existing caller that
//! holds a direction index — station platform axes, [`opposite_dir`], [`step`] —
//! keeps working unchanged. The eight half-steps occupy `8..16`, in the same
//! clockwise order, each sitting between the two compass steps it straddles.
//!
//! Angular reasoning uses [`clock_index`] instead, which maps a direction onto
//! its position `0..16` on the compass rose (clockwise from north).
//!
//! # The half-steps are knight's moves, and they are not exactly 22.5° apart
//!
//! On a square tile grid the only lattice vectors between N and NE are the
//! knight's moves, so `NNE` is `(1, 2)` — a bearing of 26.57°, not 22.5°. The
//! realised rose therefore steps `26.57°, 18.43°, 18.43°, 26.57°` per quadrant
//! rather than four even 22.5° steps, a worst case 4.07° from the ideal.
//!
//! That is comfortably inside the junction plate's drawable window (roughly 10°
//! to 30°): the tightest step in the rose is 18.43°, still nearly twice the
//! pixel floor. See [`bearing_deg`].

use crate::ids::TileCoord;

/// The eight compass steps (N, NE, E, SE, S, SW, W, NW), clockwise from north.
///
/// Retained as the first half of [`DIR16`] with identical indices. Callers that
/// only ever mean "a tile-adjacent step" — station platform runs, the station
/// track probe — should keep using this.
pub const DIR8: [(i32, i32); 8] = [
    (0, 1),
    (1, 1),
    (1, 0),
    (1, -1),
    (0, -1),
    (-1, -1),
    (-1, 0),
    (-1, 1),
];

/// All sixteen track directions: the eight compass steps, then the eight
/// knight's-move half-steps, both clockwise from north.
///
/// `DIR16[0..8] == DIR8`, index for index.
pub const DIR16: [(i32, i32); 16] = [
    // 0..8 — the compass, tile-adjacent.
    (0, 1),   // 0  N
    (1, 1),   // 1  NE
    (1, 0),   // 2  E
    (1, -1),  // 3  SE
    (0, -1),  // 4  S
    (-1, -1), // 5  SW
    (-1, 0),  // 6  W
    (-1, 1),  // 7  NW
    // 8..16 — the half-steps, each between the two compass steps above it.
    (1, 2),   // 8  NNE
    (2, 1),   // 9  ENE
    (2, -1),  // 10 ESE
    (1, -2),  // 11 SSE
    (-1, -2), // 12 SSW
    (-2, -1), // 13 WSW
    (-2, 1),  // 14 WNW
    (-1, 2),  // 15 NNW
];

/// Number of track directions (brief 01 §5.2).
pub const DIR_COUNT: usize = 16;
/// Index of the first half-step in [`DIR16`].
pub const HALF_STEP_BASE: usize = 8;

/// Bitmask of which of the 16 directions have a linked track neighbour.
///
/// Widened from `u8` to `u16` for the sixteen-direction graph. This type is
/// serialised inside [`TrackPiece`](super::piece::TrackPiece), so the save blob
/// changes shape with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct TrackLinks(pub u16);

impl TrackLinks {
    #[inline]
    pub fn empty() -> Self {
        Self(0)
    }

    #[inline]
    pub fn set(&mut self, dir_index: usize) {
        debug_assert!(dir_index < DIR_COUNT);
        self.0 |= 1 << (dir_index as u16);
    }

    #[inline]
    pub fn clear(&mut self, dir_index: usize) {
        debug_assert!(dir_index < DIR_COUNT);
        self.0 &= !(1 << (dir_index as u16));
    }

    #[inline]
    pub fn has(&self, dir_index: usize) -> bool {
        dir_index < DIR_COUNT && self.0 & (1 << (dir_index as u16)) != 0
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Number of connected directions (0–16).
    pub fn count(self) -> u32 {
        self.0.count_ones()
    }

    /// The set directions, ascending by [`DIR16`] index.
    pub fn dirs(self) -> impl Iterator<Item = usize> {
        (0..DIR_COUNT).filter(move |&i| self.has(i))
    }

    /// True when any half-step (knight's-move) direction is linked.
    pub fn has_half_step(self) -> bool {
        self.0 & 0xFF00 != 0
    }
}

/// Index into [`DIR16`] for stepping from `from` to `to`, if the offset is one
/// of the sixteen. Compass offsets keep their old `DIR8` index.
pub fn dir_index(from: TileCoord, to: TileCoord) -> Option<usize> {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    DIR16.iter().position(|&(x, y)| x == dx && y == dy)
}

/// Opposite direction index.
///
/// Compass in, compass out (`0↔4`, `1↔5`, …, unchanged); half-step in,
/// half-step out (`8↔12`, `9↔13`, …).
#[inline]
pub fn opposite_dir(dir_index: usize) -> usize {
    if dir_index < HALF_STEP_BASE {
        (dir_index + 4) % 8
    } else {
        HALF_STEP_BASE + (dir_index - HALF_STEP_BASE + 4) % 8
    }
}

/// Offset a tile by a [`DIR16`] entry.
#[inline]
pub fn step(coord: TileCoord, dir_index: usize) -> TileCoord {
    let (dx, dy) = DIR16[dir_index];
    TileCoord {
        x: coord.x + dx,
        y: coord.y + dy,
    }
}

/// True when `dir_index` is one of the eight knight's-move half-steps.
#[inline]
pub fn is_half_step(dir_index: usize) -> bool {
    dir_index >= HALF_STEP_BASE
}

/// Position on the sixteen-point compass rose, clockwise from north (`0..16`).
///
/// This is the angular index — use it for anything that reasons about *turns*.
/// [`DIR16`] indices are storage order and are deliberately not angular.
#[inline]
pub fn clock_index(dir_index: usize) -> usize {
    if dir_index < HALF_STEP_BASE {
        dir_index * 2
    } else {
        (dir_index - HALF_STEP_BASE) * 2 + 1
    }
}

/// Inverse of [`clock_index`].
#[inline]
pub fn dir_from_clock(clock: usize) -> usize {
    let c = clock % DIR_COUNT;
    if c % 2 == 0 {
        c / 2
    } else {
        HALF_STEP_BASE + c / 2
    }
}

/// Shortest separation between two directions, in rose steps (`0..=8`).
///
/// `8` means the two directions are exactly opposed — a straight run through.
#[inline]
pub fn clock_separation(a: usize, b: usize) -> usize {
    let ca = clock_index(a);
    let cb = clock_index(b);
    let d = ca.abs_diff(cb);
    d.min(DIR_COUNT - d)
}

/// The two compass directions a half-step sits between, as [`DIR16`] indices.
///
/// `None` for the compass steps themselves. `NNE` straddles `N` and `NE`; `NNW`
/// straddles `NW` and `N`.
#[inline]
pub fn straddled_dirs(dir_index: usize) -> Option<[usize; 2]> {
    if !is_half_step(dir_index) {
        return None;
    }
    let k = dir_index - HALF_STEP_BASE;
    Some([k, (k + 1) % 8])
}

/// The tiles a half-step link passes over, between its two endpoints.
///
/// A knight's move from a tile centre crosses exactly two other tiles, entering
/// and leaving each through an edge midpoint — for `(0,0) → (2,1)` those are
/// `(1,0)` and `(1,1)`, a quarter of the run each. They are precisely the tiles
/// of the two compass steps the half-step straddles, which is why a half-step
/// link and either of its neighbouring compass links can never coexist at a
/// node (see [`turnout_divergence_ok`](super::rules::turnout_divergence_ok)).
///
/// `None` for compass directions, which pass over nothing.
///
/// The set is symmetric: `intermediate_tiles(a, d)` equals
/// `intermediate_tiles(step(a, d), opposite_dir(d))`.
pub fn intermediate_tiles(from: TileCoord, dir_index: usize) -> Option<[TileCoord; 2]> {
    let [a, b] = straddled_dirs(dir_index)?;
    Some([step(from, a), step(from, b)])
}

/// Squared tile length of a direction: `1` orthogonal, `2` diagonal, `5`
/// half-step.
#[inline]
pub fn length_sq(dir_index: usize) -> i32 {
    let (dx, dy) = DIR16[dir_index];
    dx * dx + dy * dy
}

/// True bearing of a direction in degrees, clockwise from north.
///
/// The *realised* geometry, not the nominal 22.5° rose: half-steps come out at
/// 26.57° and 63.43° because they are knight's moves on a square lattice.
pub fn bearing_deg(dir_index: usize) -> f32 {
    let (dx, dy) = DIR16[dir_index];
    let deg = (dx as f32).atan2(dy as f32).to_degrees();
    if deg < 0.0 {
        deg + 360.0
    } else {
        deg
    }
}

/// Smallest angle in degrees between two directions, using true bearings.
pub fn bearing_separation_deg(a: usize, b: usize) -> f32 {
    let d = (bearing_deg(a) - bearing_deg(b)).abs();
    if d > 180.0 {
        360.0 - d
    } else {
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    #[test]
    fn compass_indices_are_unchanged_by_the_widening() {
        assert_eq!(DIR16[..8], DIR8);
        for i in 0..8 {
            assert_eq!(opposite_dir(i), (i + 4) % 8);
            assert_eq!(step(tile(0, 0), i), tile(DIR8[i].0, DIR8[i].1));
        }
        assert_eq!(dir_index(tile(3, 3), tile(3, 4)), Some(0));
        assert_eq!(dir_index(tile(3, 3), tile(4, 3)), Some(2));
    }

    #[test]
    fn every_direction_is_distinct_and_reversible() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..DIR_COUNT {
            assert!(seen.insert(DIR16[i]), "duplicate direction at {i}");
            let opp = opposite_dir(i);
            assert_eq!(DIR16[opp], (-DIR16[i].0, -DIR16[i].1));
            assert_eq!(opposite_dir(opp), i);
            assert_eq!(is_half_step(i), is_half_step(opp));
        }
        assert_eq!(seen.len(), DIR_COUNT);
    }

    #[test]
    fn clock_index_is_a_bijection_onto_the_rose() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..DIR_COUNT {
            let c = clock_index(i);
            assert!(c < DIR_COUNT);
            assert!(seen.insert(c));
            assert_eq!(dir_from_clock(c), i);
        }
        // Clockwise from north: N=0, NNE=1, NE=2, ENE=3, E=4.
        assert_eq!(clock_index(0), 0);
        assert_eq!(clock_index(8), 1);
        assert_eq!(clock_index(1), 2);
        assert_eq!(clock_index(9), 3);
        assert_eq!(clock_index(2), 4);
    }

    #[test]
    fn clock_ordering_matches_true_bearings() {
        let mut by_clock: Vec<usize> = (0..DIR_COUNT).collect();
        by_clock.sort_by_key(|&d| clock_index(d));
        let bearings: Vec<f32> = by_clock.iter().map(|&d| bearing_deg(d)).collect();
        assert!(
            bearings.windows(2).all(|w| w[0] < w[1]),
            "rose order must be monotonic in bearing: {bearings:?}"
        );
    }

    /// The honest geometry: knight's moves are 26.57°/63.43°, not 22.5°/67.5°.
    /// Every adjacent pair still clears the junction plate's ~10° pixel floor.
    #[test]
    fn adjacent_rose_steps_clear_the_pixel_floor() {
        for c in 0..DIR_COUNT {
            let a = dir_from_clock(c);
            let b = dir_from_clock(c + 1);
            let sep = bearing_separation_deg(a, b);
            assert!(
                (18.0..=27.0).contains(&sep),
                "step {c}->{} is {sep}deg, outside the realised rose",
                c + 1
            );
            assert!(sep > 10.0, "step {c} at {sep}deg is below the pixel floor");
        }
        // Worst deviation from an ideal 22.5° rose.
        let worst = (0..DIR_COUNT)
            .map(|c| (bearing_separation_deg(dir_from_clock(c), dir_from_clock(c + 1)) - 22.5).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 4.1, "knight rose deviates by {worst}deg");
    }

    #[test]
    fn half_steps_are_knights_moves_over_two_tiles() {
        for d in HALF_STEP_BASE..DIR_COUNT {
            assert_eq!(length_sq(d), 5);
            let mids = intermediate_tiles(tile(0, 0), d).expect("half-step has intermediates");
            assert_ne!(mids[0], mids[1]);
            let end = step(tile(0, 0), d);
            for m in mids {
                // Each intermediate is tile-adjacent to both endpoints.
                assert!(dir_index(tile(0, 0), m).is_some_and(|i| !is_half_step(i)));
                assert!(dir_index(m, end).is_some_and(|i| !is_half_step(i)));
            }
        }
        assert!(intermediate_tiles(tile(0, 0), 2).is_none());
    }

    #[test]
    fn intermediates_are_the_same_seen_from_either_end() {
        for d in HALF_STEP_BASE..DIR_COUNT {
            let a = tile(4, 4);
            let b = step(a, d);
            let mut from_a = intermediate_tiles(a, d).unwrap();
            let mut from_b = intermediate_tiles(b, opposite_dir(d)).unwrap();
            from_a.sort_by_key(|t| (t.x, t.y));
            from_b.sort_by_key(|t| (t.x, t.y));
            assert_eq!(from_a, from_b, "half-step {d} is asymmetric");
        }
    }

    /// A half-step's intermediates are exactly the tiles of the compass steps it
    /// straddles — the fact the junction rule leans on.
    #[test]
    fn intermediates_are_the_straddled_compass_tiles() {
        for d in HALF_STEP_BASE..DIR_COUNT {
            let [a, b] = straddled_dirs(d).unwrap();
            assert_eq!(clock_separation(d, a), 1);
            assert_eq!(clock_separation(d, b), 1);
            let mids = intermediate_tiles(tile(0, 0), d).unwrap();
            assert!(mids.contains(&step(tile(0, 0), a)));
            assert!(mids.contains(&step(tile(0, 0), b)));
        }
    }

    #[test]
    fn links_hold_all_sixteen_bits() {
        let mut links = TrackLinks::empty();
        assert!(links.is_empty());
        for i in 0..DIR_COUNT {
            links.set(i);
        }
        assert_eq!(links.count(), 16);
        assert_eq!(links.0, u16::MAX);
        assert!(links.has_half_step());
        assert_eq!(links.dirs().count(), 16);
        for i in 0..DIR_COUNT {
            links.clear(i);
        }
        assert!(links.is_empty());

        let mut compass_only = TrackLinks::empty();
        compass_only.set(2);
        compass_only.set(6);
        assert!(!compass_only.has_half_step());
        assert_eq!(compass_only.dirs().collect::<Vec<_>>(), vec![2, 6]);
    }

    #[test]
    fn opposed_directions_separate_by_eight_steps() {
        for i in 0..DIR_COUNT {
            assert_eq!(clock_separation(i, opposite_dir(i)), 8);
            assert_eq!(clock_separation(i, i), 0);
        }
    }
}
