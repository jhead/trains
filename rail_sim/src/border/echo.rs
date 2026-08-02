//! Echo neighbours — how a single-player town is "still connected to somebody".
//!
//! `12-multiplayer.md` §6 is emphatic that an echo is **not a fallback**: it is
//! the default experience, and a real linked friend is the upgrade. So this
//! module is not a stub with a `TODO` on it — it is the neighbour, and MP-2 adds
//! a second source of the same [`BorderManifest`] beside it.
//!
//! An echo is a pure function of `(map seed, edge, growth)`:
//!
//! - **Stable and reproducible.** The same world always has the same neighbours
//!   with the same names behind the same edges, across saves and reinstalls.
//!   Nothing about an echo is stored that could drift from what regenerates.
//! - **Persistent and named**, with a town name from a curated generator
//!   (§8.1 — there is no free text anywhere in this feature).
//! - **Growing slowly.** `growth` counts [`ECHO_GROWTH_TICKS`] periods since the
//!   link opened; residents, platforms, skyline and supply all follow it. A
//!   returning player finds a neighbour that grew while they were away (§12.4).
//! - **Honestly labelled.** Every manifest it produces carries
//!   [`PresenceSource::Echo`], in the data, so no interface can forget to say so.
//!
//! Growth also drives [`BorderManifest::sequence`], so an echo *publishes*
//! exactly like a remote neighbour would: a new manifest with a higher sequence
//! arrives every so often, and the cache takes it through the same
//! [`BorderManifest::sanitised`] gate MP-2 will use.

use crate::stations::GoodKind;

use super::edge::BorderEdge;
use super::manifest::{
    BorderManifest, HeadlineStat, LinkId, Presence, PresenceSource, Silhouette, StandingOffer,
    StandingRequest, MANIFEST_SCHEMA_VERSION, SILHOUETTE_ROOFS,
};

/// Sim ticks per growth step. A neighbour is a slow, patient thing.
pub const ECHO_GROWTH_TICKS: u64 = 240;

/// Growth steps between one more resident arriving next door.
const RESIDENTS_PER_STEP: u32 = 2;
/// Growth steps before the neighbour opens another platform.
const STEPS_PER_STATION: u32 = 12;
/// Growth steps before the neighbour can supply one more unit per period.
const STEPS_PER_OFFER_UNIT: u32 = 16;
/// Most a neighbour will ever supply per period, however long you play.
const MAX_OFFER_UNITS: u32 = 8;
/// Most platforms a neighbour town will ever claim.
const MAX_ECHO_STATIONS: u32 = 12;

/// Curated first halves of a town name. Never player input.
const NAME_HEADS: &[&str] = &[
    "Ash", "Bram", "Cole", "Dun", "Elm", "Far", "Grey", "Har", "Iron", "Kes", "Lark", "March",
    "Nether", "Old", "Pen", "Quar", "Red", "Stone", "Thorn", "Weir",
];

/// Curated second halves. Chosen so every pairing reads as a place.
const NAME_TAILS: &[&str] = &[
    "combe", "ford", "gate", "moor", "bridge", "wick", "thorpe", "dale", "mere", "stead", "bourne",
    "worth",
];

/// Salts so each trait of a neighbour draws from its own stream.
const SALT_LINK: u64 = 0x11;
const SALT_NAME_HEAD: u64 = 0x21;
const SALT_NAME_TAIL: u64 = 0x22;
const SALT_GOOD: u64 = 0x31;
const SALT_RHYTHM: u64 = 0x41;
const SALT_RESIDENTS: u64 = 0x51;
const SALT_STATIONS: u64 = 0x52;
const SALT_SKYLINE: u64 = 0x61;

/// SplitMix64 finaliser — small, fast and stable across platforms.
///
/// The sim is fixed-step and deterministic, so an echo may not lean on
/// [`std::collections::hash_map::DefaultHasher`], whose output is explicitly not
/// guaranteed between releases.
fn mix(z: u64) -> u64 {
    let mut x = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// One independent value for `(seed, edge, salt)`.
fn stream(seed: u64, edge: BorderEdge, salt: u64) -> u64 {
    mix(seed ^ mix(edge.index() as u64 + 1).wrapping_mul(0x9e37_79b9) ^ mix(salt))
}

fn pick<'a>(pool: &[&'a str], value: u64) -> &'a str {
    pool[(value % pool.len() as u64) as usize]
}

/// Stable link identity for the echo behind `edge` of the map `seed`.
///
/// MP-2 replaces this with the id a friend code agrees on; nothing downstream
/// cares which one it got.
pub fn echo_link_id(seed: u64, edge: BorderEdge) -> LinkId {
    LinkId(stream(seed, edge, SALT_LINK) | 1)
}

/// The neighbour's town name — curated halves, never free text.
pub fn echo_town_name(seed: u64, edge: BorderEdge) -> String {
    let head = pick(NAME_HEADS, stream(seed, edge, SALT_NAME_HEAD));
    let tail = pick(NAME_TAILS, stream(seed, edge, SALT_NAME_TAIL));
    format!("{head}{tail}")
}

/// What this neighbour supplies.
///
/// Two of the four edges supply each commodity, and **opposite edges never
/// supply the same one** — so whatever the map happens to lack, two of its
/// borders can bring it. That keeps §4.3's economic case alive without the echo
/// needing to see your industries, which is what lets it stay a pure function of
/// seed and edge.
///
/// One coin flip decides the whole map, so all four edges are drawn from a
/// single stream rather than from four unrelated hashes that could agree by
/// accident.
pub fn echo_offer_good(seed: u64, edge: BorderEdge) -> GoodKind {
    let flip = stream(seed, BorderEdge::North, SALT_GOOD) & 1 == 1;
    let north_or_west = matches!(edge, BorderEdge::North | BorderEdge::West);
    if north_or_west != flip {
        GoodKind::Lumber
    } else {
        GoodKind::Ore
    }
}

/// What this neighbour wants back — the good they do not supply.
pub fn echo_request_good(seed: u64, edge: BorderEdge) -> GoodKind {
    match echo_offer_good(seed, edge) {
        GoodKind::Lumber => GoodKind::Ore,
        GoodKind::Ore => GoodKind::Lumber,
    }
}

/// The neighbour's trading rhythm, in sim ticks between deliveries.
pub fn echo_period_ticks(seed: u64, edge: BorderEdge) -> u32 {
    90 + (stream(seed, edge, SALT_RHYTHM) % 150) as u32
}

/// Supply after `growth` steps.
pub fn echo_offer(seed: u64, edge: BorderEdge, growth: u32) -> StandingOffer {
    let base = 1 + (stream(seed, edge, SALT_GOOD) % 3) as u32;
    StandingOffer {
        good: echo_offer_good(seed, edge),
        units_per_period: (base + growth / STEPS_PER_OFFER_UNIT).min(MAX_OFFER_UNITS),
        period_ticks: echo_period_ticks(seed, edge),
    }
}

/// What they are asking for, after `growth` steps.
pub fn echo_request(seed: u64, edge: BorderEdge, growth: u32) -> StandingRequest {
    let base = 1 + (stream(seed, edge, SALT_RHYTHM) % 3) as u32;
    StandingRequest {
        good: echo_request_good(seed, edge),
        units_per_period: (base + growth / STEPS_PER_OFFER_UNIT).min(MAX_OFFER_UNITS),
    }
}

/// Their skyline on the horizon, after `growth` steps.
///
/// Roofs rise slowly and unevenly, so a town you have traded with for hours
/// looks like one. It is twelve numbers; it is not a map.
pub fn echo_silhouette(seed: u64, edge: BorderEdge, growth: u32) -> Silhouette {
    let source = stream(seed, edge, SALT_SKYLINE);
    let roofs = (0..SILHOUETTE_ROOFS)
        .map(|i| {
            let nibble = ((source >> ((i % 16) * 4)) & 0xf) as u8;
            // A third of the frontage stays as gaps so the skyline reads as
            // buildings rather than a wall.
            if nibble < 5 {
                0
            } else {
                let base = nibble.min(9);
                let lift = (growth / (4 + i as u32)) as u8;
                base.saturating_add(lift).min(15)
            }
        })
        .collect();
    Silhouette::new(roofs)
}

/// Their headline numbers, after `growth` steps.
pub fn echo_headline(seed: u64, edge: BorderEdge, growth: u32) -> HeadlineStat {
    let residents = 40 + (stream(seed, edge, SALT_RESIDENTS) % 140) as u32;
    let stations = 2 + (stream(seed, edge, SALT_STATIONS) % 3) as u32;
    HeadlineStat {
        residents: residents.saturating_add(growth / RESIDENTS_PER_STEP),
        stations: (stations + growth / STEPS_PER_STATION).min(MAX_ECHO_STATIONS),
    }
}

/// The whole neighbour, as the manifest they would have published.
///
/// This is the one function the sim calls. MP-2 adds a sibling that reads the
/// same type out of a blob store, and everything downstream is untouched.
pub fn echo_manifest(seed: u64, edge: BorderEdge, growth: u32) -> BorderManifest {
    BorderManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        link: echo_link_id(seed, edge),
        // They face us, so their edge is the opposite of ours.
        edge: opposite(edge),
        // Growth *is* the publish counter: an echo puts out a fresh manifest
        // every growth step, exactly as a remote neighbour would.
        sequence: growth as u64,
        departures: Vec::new(),
        offer: echo_offer(seed, edge, growth),
        request: echo_request(seed, edge, growth),
        presence: Presence {
            town_name: echo_town_name(seed, edge),
            headline: echo_headline(seed, edge, growth),
            silhouette: echo_silhouette(seed, edge, growth),
            source: PresenceSource::Echo,
        },
    }
}

/// Growth steps an echo has taken after `ticks` of an open link.
pub fn growth_steps(ticks: u64) -> u32 {
    (ticks / ECHO_GROWTH_TICKS).min(u32::MAX as u64) as u32
}

fn opposite(edge: BorderEdge) -> BorderEdge {
    match edge {
        BorderEdge::North => BorderEdge::South,
        BorderEdge::South => BorderEdge::North,
        BorderEdge::East => BorderEdge::West,
        BorderEdge::West => BorderEdge::East,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_and_edge_always_gives_the_same_neighbour() {
        for seed in [0u64, 42, 7_777, u64::MAX] {
            for edge in BorderEdge::ALL {
                let a = echo_manifest(seed, edge, 3);
                let b = echo_manifest(seed, edge, 3);
                assert_eq!(a, b, "an echo must be reproducible");
            }
        }
    }

    #[test]
    fn each_edge_of_a_map_is_a_different_place() {
        let names: Vec<String> = BorderEdge::ALL
            .iter()
            .map(|e| echo_town_name(42, *e))
            .collect();
        let links: Vec<LinkId> = BorderEdge::ALL
            .iter()
            .map(|e| echo_link_id(42, *e))
            .collect();
        for i in 0..names.len() {
            for j in (i + 1)..names.len() {
                assert_ne!(links[i], links[j], "each edge is its own link");
            }
        }
        assert_eq!(names.len(), 4);
    }

    #[test]
    fn opposite_edges_supply_opposite_goods() {
        // Whatever the map lacks, two of the four borders can bring it.
        for seed in [1u64, 42, 99, 100_003] {
            assert_ne!(
                echo_offer_good(seed, BorderEdge::North),
                echo_offer_good(seed, BorderEdge::East),
                "seed {seed}: a map must not be ringed by one commodity"
            );
            assert_ne!(
                echo_offer_good(seed, BorderEdge::North),
                echo_offer_good(seed, BorderEdge::South)
            );
            assert_ne!(
                echo_offer_good(seed, BorderEdge::East),
                echo_offer_good(seed, BorderEdge::West)
            );
        }
    }

    #[test]
    fn a_neighbour_asks_for_what_it_does_not_supply() {
        for edge in BorderEdge::ALL {
            assert_ne!(echo_offer_good(42, edge), echo_request_good(42, edge));
        }
    }

    #[test]
    fn a_link_id_is_never_zero() {
        // Zero is the "no link" sentinel everywhere else, so it must not be
        // reachable by generation.
        for seed in 0..64u64 {
            for edge in BorderEdge::ALL {
                assert_ne!(echo_link_id(seed, edge), LinkId(0));
            }
        }
    }

    #[test]
    fn neighbours_grow_slowly_and_stop_somewhere() {
        let early = echo_manifest(42, BorderEdge::North, 0);
        let later = echo_manifest(42, BorderEdge::North, 40);
        assert!(later.presence.headline.residents > early.presence.headline.residents);
        assert!(later.presence.headline.stations >= early.presence.headline.stations);
        assert!(later.offer.units_per_period >= early.offer.units_per_period);
        assert!(later.supersedes(&early), "growth publishes a new manifest");

        let forever = echo_manifest(42, BorderEdge::North, u32::MAX / 2);
        assert!(forever.offer.units_per_period <= MAX_OFFER_UNITS);
        assert!(forever.presence.headline.stations <= MAX_ECHO_STATIONS);
        assert!(forever.presence.silhouette.roofs.iter().all(|r| *r <= 15));
    }

    #[test]
    fn a_generated_neighbour_passes_its_own_gate() {
        for seed in [0u64, 42, u64::MAX] {
            for edge in BorderEdge::ALL {
                for growth in [0u32, 1, 500] {
                    let manifest = echo_manifest(seed, edge, growth);
                    let link = manifest.link;
                    assert!(
                        manifest.sanitised(link).is_some(),
                        "an echo must survive the same clamp a stranger's manifest gets"
                    );
                }
            }
        }
    }

    #[test]
    fn an_echo_is_labelled_as_one() {
        let manifest = echo_manifest(42, BorderEdge::West, 5);
        assert!(manifest.is_echo());
        assert_eq!(manifest.presence.source, PresenceSource::Echo);
        assert!(!manifest.town_name().is_empty());
    }

    #[test]
    fn growth_steps_track_the_clock() {
        assert_eq!(growth_steps(0), 0);
        assert_eq!(growth_steps(ECHO_GROWTH_TICKS - 1), 0);
        assert_eq!(growth_steps(ECHO_GROWTH_TICKS), 1);
        assert_eq!(growth_steps(ECHO_GROWTH_TICKS * 9), 9);
    }

    #[test]
    fn names_come_from_the_curated_pools() {
        for seed in 0..200u64 {
            for edge in BorderEdge::ALL {
                let name = echo_town_name(seed, edge);
                assert!(
                    NAME_HEADS.iter().any(|h| name.starts_with(h)),
                    "{name} must start with a curated head"
                );
                assert!(
                    NAME_TAILS.iter().any(|t| name.ends_with(t)),
                    "{name} must end with a curated tail"
                );
            }
        }
    }
}
