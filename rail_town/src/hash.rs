//! World-anchored hashing (brief 01 §2.4) — the crate's single implementation.
//!
//! Every procedural choice in the presentation — which water tile glints,
//! where a window sits, which flat variant a tile draws, when a chimney puffs —
//! hashes on **integer world coordinates** plus a constant salt. Never on
//! screen position, never on time.
//!
//! The `downsample` plate's finding is blunt: screen-anchored noise *boils*
//! across the whole surface under scroll, while world-anchored noise is fixed
//! to the ground and simply scrolls with it. Time-seeded noise is the same
//! failure in the temporal axis — it re-rolls the world every frame.
//!
//! Hashes here pick a *phase*, and phase is then advanced by a shared clock.
//! That is what stops four hundred water tiles pulsing in unison.

/// Deterministic 32-bit hash of a world coordinate and a salt.
///
/// The y term is **added**, not xored, into the x term before the finalising
/// rounds — adding breaks the x==y symmetry that shows up as a diagonal moiré
/// across adjacent tiles, which is the visible failure mode this exists to
/// avoid.
#[inline]
pub(crate) fn world_hash(x: i32, y: i32, salt: u32) -> u32 {
    let mut h = (x as u32)
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add((y as u32).wrapping_mul(0x85EB_CA6B))
        ^ salt.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 13;
    h = h.wrapping_mul(0x27D4_EB2F);
    h ^ (h >> 16)
}

/// [`world_hash`] mapped to `0.0..1.0`.
#[inline]
pub(crate) fn hash_unit(x: i32, y: i32, salt: u32) -> f32 {
    world_hash(x, y, salt) as f32 / u32::MAX as f32
}

/// A per-tile phase offset in seconds, spread across one `period`.
#[inline]
pub(crate) fn hash_phase(x: i32, y: i32, salt: u32, period: f32) -> f32 {
    hash_unit(x, y, salt) * period
}

/// An integer offset in `-half..=half`, for scattering a decal inside its tile.
#[inline]
pub(crate) fn hash_offset(x: i32, y: i32, salt: u32, half: i32) -> i32 {
    let span = (half * 2 + 1) as u32;
    (world_hash(x, y, salt) % span) as i32 - half
}

/// Which frame of a `frames`-long loop is showing at `secs`, given `phase`.
///
/// The phase comes from the world hash, so two tiles side by side sit at
/// different points in the same loop and the surface never pulses as one.
#[inline]
pub(crate) fn frame_at(secs: f32, phase: f32, period: f32, frames: u32) -> u32 {
    debug_assert!(period > 0.0 && frames > 0);
    let cycle = ((secs + phase) / period).rem_euclid(1.0);
    ((cycle * frames as f32) as u32).min(frames - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic_per_coordinate() {
        assert_eq!(world_hash(12, 7, 99), world_hash(12, 7, 99));
        assert_ne!(world_hash(12, 7, 99), world_hash(12, 7, 100));
        assert_ne!(world_hash(12, 7, 99), world_hash(7, 12, 99));
    }

    #[test]
    fn neighbours_decorrelate() {
        // Adjacent tiles must not land on the same value; a run of matches is
        // what a visible moiré looks like.
        let mut matches = 0;
        for y in 0..64 {
            for x in 0..64 {
                if world_hash(x, y, 1) == world_hash(x + 1, y, 1) {
                    matches += 1;
                }
            }
        }
        assert_eq!(matches, 0);
    }

    #[test]
    fn diagonal_is_not_symmetric() {
        // The adding mix exists so that reflecting across x==y changes the
        // value; a symmetric hash draws its noise mirrored about the diagonal.
        let mut matches = 0;
        for y in 0..64i32 {
            for x in 0..64i32 {
                if x != y && world_hash(x, y, 9) == world_hash(y, x, 9) {
                    matches += 1;
                }
            }
        }
        assert_eq!(matches, 0);
    }

    #[test]
    fn unit_is_uniform_enough_for_gating() {
        let mut sum = 0.0;
        let mut n = 0.0;
        for y in 0..64 {
            for x in 0..64 {
                let v = hash_unit(x, y, 7);
                assert!((0.0..=1.0).contains(&v));
                sum += v;
                n += 1.0;
            }
        }
        let mean = sum / n;
        assert!((mean - 0.5).abs() < 0.03, "hash mean drifted: {mean}");
    }

    #[test]
    fn offsets_stay_inside_the_tile() {
        for y in 0..32 {
            for x in 0..32 {
                let o = hash_offset(x, y, 3, 6);
                assert!((-6..=6).contains(&o));
            }
        }
    }

    #[test]
    fn frame_index_covers_every_frame_and_stays_in_range() {
        let mut seen = [false; 3];
        for step in 0..240 {
            let f = frame_at(step as f32 * 0.01, 0.0, 2.4, 3);
            assert!(f < 3);
            seen[f as usize] = true;
        }
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn phase_spreads_tiles_across_the_loop() {
        // Two neighbours at the same instant should usually be on different
        // frames — that is the whole point of hashing the phase.
        let period = 1.2;
        let mut differing = 0;
        for x in 0..64 {
            let a = frame_at(0.0, hash_phase(x, 0, 5, period), period, 2);
            let b = frame_at(0.0, hash_phase(x + 1, 0, 5, period), period, 2);
            if a != b {
                differing += 1;
            }
        }
        assert!(differing > 16, "phases barely differ: {differing}/64");
    }
}
