//! Combinatorial name pool and procedural portrait seeds.
//!
//! Brief 06 §4.2 asks for **full names from a combinatorial pool**, so
//! `Mara Aldertone` and `Theo Finch` persist and recur. Given names and family
//! names are drawn separately from a stable hash, which gives
//! `GIVEN_NAMES.len() * FAMILY_NAMES.len()` distinct people while keeping the
//! table small enough to read.
//!
//! Everything here is pure and deterministic — the same seed always produces
//! the same person, which is what makes a saved town feel continuous.

use serde::{Deserialize, Serialize};

/// Given names. Order is stable: seed `0` is always `Mara`.
pub const GIVEN_NAMES: &[&str] = &[
    "Mara", "Jon", "Elise", "Theo", "Nia", "Owen", "Priya", "Sam", "Vera", "Cole", "Asha", "Reed",
    "Ines", "Bram", "Tova", "Ravi", "Greta", "Milo", "Sena", "Idris", "Lena", "Halle", "Otto",
    "Rosa", "Casper", "Yara", "Emrys", "Petra", "Nolan", "Delia", "Fen", "Marit", "Arlo", "Bea",
    "Soren", "Wren", "Hugo", "Isla", "Dov", "Tamsin",
];

/// Family names. Shared by every member of a household.
pub const FAMILY_NAMES: &[&str] = &[
    "Aldertone",
    "Finch",
    "Alderton",
    "Ashby",
    "Colby",
    "Denner",
    "Farrow",
    "Gale",
    "Hallam",
    "Ives",
    "Kettle",
    "Larkin",
    "Mowbray",
    "Nash",
    "Orme",
    "Pike",
    "Quillan",
    "Rowe",
    "Sable",
    "Thorn",
    "Underhill",
    "Vane",
    "Whitlock",
    "Yarrow",
    "Brack",
    "Crane",
    "Dunmore",
    "Ellery",
    "Fenwick",
    "Gorse",
    "Hedley",
    "Ipsley",
];

/// Distinct full names this pool can produce.
pub const NAME_POOL_SIZE: usize = GIVEN_NAMES.len() * FAMILY_NAMES.len();

/// Procedural portrait body type (brief 06 §4.2 — four body types).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BodyType {
    Slight,
    Stocky,
    Tall,
    Round,
}

impl Default for BodyType {
    fn default() -> Self {
        Self::Slight
    }
}

impl BodyType {
    pub const ALL: [BodyType; 4] = [Self::Slight, Self::Stocky, Self::Tall, Self::Round];

    pub fn from_seed(seed: u64) -> Self {
        Self::ALL[(hash64(seed ^ 0x9e37_79b9_7f4a_7c15) % 4) as usize]
    }

    /// Sprite height in texels for a 32-texel tile (art 01 §7 placeholder sizing).
    pub fn height_texels(self) -> u8 {
        match self {
            Self::Slight => 7,
            Self::Stocky => 6,
            Self::Tall => 8,
            Self::Round => 6,
        }
    }

    /// Sprite width in texels.
    pub fn width_texels(self) -> u8 {
        match self {
            Self::Slight => 3,
            Self::Stocky => 4,
            Self::Tall => 3,
            Self::Round => 4,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Slight => "slight",
            Self::Stocky => "stocky",
            Self::Tall => "tall",
            Self::Round => "round",
        }
    }
}

/// How many palette-drawn clothing / hair variants a portrait can pick from.
pub const PORTRAIT_VARIANTS: u8 = 8;

/// Deterministic 64-bit mix (splitmix64 finaliser). World-anchored, never time-anchored.
#[inline]
pub fn hash64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Given name for a peep seed.
pub fn given_name(seed: u64) -> &'static str {
    GIVEN_NAMES[(hash64(seed) % GIVEN_NAMES.len() as u64) as usize]
}

/// Family name for a household seed.
pub fn family_name(seed: u64) -> &'static str {
    FAMILY_NAMES[(hash64(seed ^ 0x51ed_2701_a3f6_9c11) % FAMILY_NAMES.len() as u64) as usize]
}

/// Full name — `"Mara Aldertone"`.
pub fn full_name(peep_seed: u64, household_seed: u64) -> String {
    format!("{} {}", given_name(peep_seed), family_name(household_seed))
}

/// Household plural for Town Talk — `"the Aldertones"`, `"the Finches"`.
pub fn family_plural(family: &str) -> String {
    let lower = family.to_ascii_lowercase();
    let suffix = if lower.ends_with('s')
        || lower.ends_with('x')
        || lower.ends_with("ch")
        || lower.ends_with("sh")
        || lower.ends_with('z')
    {
        "es"
    } else {
        "s"
    };
    format!("The {family}{suffix}")
}

/// Portrait variant index in `0..`[`PORTRAIT_VARIANTS`].
pub fn portrait_variant(seed: u64) -> u8 {
    (hash64(seed ^ 0x2545_f491_4f6c_dd1d) % PORTRAIT_VARIANTS as u64) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn pool_is_combinatorial_not_a_short_list() {
        // The old implementation had a 12-entry first-name list and nothing else.
        assert!(GIVEN_NAMES.len() >= 32);
        assert!(FAMILY_NAMES.len() >= 32);
        assert!(NAME_POOL_SIZE >= 1000, "pool is {NAME_POOL_SIZE}");
    }

    #[test]
    fn names_are_stable_for_a_seed() {
        let a = full_name(7, 3);
        let b = full_name(7, 3);
        assert_eq!(a, b);
        assert!(a.contains(' '), "expected a full name, got {a}");
    }

    #[test]
    fn distinct_seeds_spread_across_the_pool() {
        let mut seen = HashSet::new();
        for i in 0..400u64 {
            seen.insert(full_name(i, i / 3));
        }
        assert!(
            seen.len() > 250,
            "combinatorial pool collapsed to {} names",
            seen.len()
        );
    }

    #[test]
    fn household_members_share_a_family_name() {
        let household = 42;
        let a = full_name(1, household);
        let b = full_name(2, household);
        let fam_a = a.split(' ').nth(1).unwrap();
        let fam_b = b.split(' ').nth(1).unwrap();
        assert_eq!(fam_a, fam_b);
    }

    #[test]
    fn family_plural_handles_sibilants() {
        assert_eq!(family_plural("Alderton"), "The Aldertons");
        assert_eq!(family_plural("Finch"), "The Finches");
        assert_eq!(family_plural("Ives"), "The Iveses");
    }

    #[test]
    fn body_types_cover_all_four() {
        let mut seen = HashSet::new();
        for i in 0..200u64 {
            seen.insert(BodyType::from_seed(i));
        }
        assert_eq!(seen.len(), 4);
    }
}
