//! The Border Manifest — the only thing that ever crosses a border.
//!
//! `12-multiplayer.md` §4.1: not world state, not chunks. A small, versioned,
//! plain-data record keyed by stable ids, carrying
//!
//! | Field | Meaning |
//! | --- | --- |
//! | [`BorderManifest::schema_version`] + [`BorderManifest::link`] | routing and compatibility |
//! | [`BorderManifest::departures`] | what left through this portal, and when |
//! | [`BorderManifest::offer`] | what will be supplied, per period |
//! | [`BorderManifest::request`] | what is wanted back |
//! | [`BorderManifest::presence`] | town name, a headline stat, the border silhouette |
//! | [`BorderManifest::sequence`] | ordering and idempotent replay |
//!
//! # What is deliberately *not* here
//!
//! There is no [`TileCoord`](crate::ids::TileCoord), no terrain, no track, no
//! station, no peep — nothing that could reconstruct a map. That is a privacy
//! property first (§8.1) and the reason exchange stays cheap enough to run over
//! trivial infrastructure second. [`tests::a_full_manifest_is_kilobytes`] holds
//! the size line; keep new fields on the right side of it.
//!
//! # MP-2 without changes
//!
//! Nothing in this module knows where a manifest came from. MP-1 fills one in
//! from a locally generated [`echo`](super::echo) neighbour; MP-2 will fetch the
//! same bytes from a blob store keyed by [`LinkId`] (`rail_net::ManifestStore`)
//! and hand them to the same [`BorderManifest::sanitised`] gate. The sim cannot
//! tell the difference, which is the whole point.
//!
//! # Threat model
//!
//! Because a neighbour can only ever add to your world (§2.2), a hostile
//! manifest's blast radius is "you receive goods you did not expect". So the
//! defence is three lines long: reject unknown schema versions, clamp every
//! quantity to a sane bound, and cap the string and vector lengths.
//! [`BorderManifest::sanitised`] is that gate and it never fails in a way that
//! stops trade — it returns a clamped manifest or [`None`], and [`None`] means
//! "keep using the cache", never "wait".

use serde::{Deserialize, Serialize};

use crate::stations::GoodKind;

use super::edge::BorderEdge;

/// Manifest wire format. Bump on any change to the shape below.
///
/// A manifest carrying an unknown version is refused by
/// [`BorderManifest::sanitised`] and the cached one keeps supplying — §5's
/// "version mismatch → reject the manifest, fall back to cache. Trade
/// continues."
pub const MANIFEST_SCHEMA_VERSION: u16 = 1;

/// Longest town name accepted from a manifest.
pub const MAX_TOWN_NAME_LEN: usize = 24;
/// Roof heights in a border silhouette — a skyline, not a map.
pub const SILHOUETTE_ROOFS: usize = 12;
/// Departures kept in a manifest; older ones are dropped.
pub const MAX_DEPARTURES: usize = 16;
/// Hard ceiling on any published quantity, whoever sent it.
pub const MAX_UNITS: u32 = 64;
/// Shortest trading period a manifest may claim, in sim ticks.
pub const MIN_PERIOD_TICKS: u32 = 60;
/// Longest trading period a manifest may claim, in sim ticks.
pub const MAX_PERIOD_TICKS: u32 = 4_096;

/// Stable identity of one border pairing.
///
/// Derived from the map seed and the edge for an echo (see
/// [`super::echo::echo_link_id`]) so it is reproducible; MP-2 friend codes will
/// supply their own and everything downstream is unchanged.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct LinkId(pub u64);

/// Where a published presence came from — and it is always said out loud.
///
/// §6: "An echo is always honestly labelled in the interface. Not deceptive,
/// just present." The label lives in the data so no UI can forget to draw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PresenceSource {
    /// Generated deterministically from a map seed and an edge.
    #[default]
    Echo,
    /// A real player's published border data (MP-2).
    Linked,
}

impl PresenceSource {
    pub fn is_echo(self) -> bool {
        matches!(self, Self::Echo)
    }

    /// Short badge for the Neighbours panel.
    pub fn label(self) -> &'static str {
        match self {
            Self::Echo => "echo",
            Self::Linked => "linked",
        }
    }
}

/// One train that left through the portal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Departure {
    /// Border tick it left on (see [`super::BorderRegistry::tick`]).
    pub tick: u64,
    /// What it carried, if anything. Empty stock still counts as a crossing.
    pub good: Option<GoodKind>,
    pub units: u32,
}

/// What a side will supply through this link, per period.
///
/// This is the field that makes §5 work: it is cached locally, and return
/// trains are generated from the cache on your own tick with no network
/// involved. An offer that is months stale still supplies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandingOffer {
    pub good: GoodKind,
    pub units_per_period: u32,
    /// Sim ticks between deliveries — the trading rhythm.
    pub period_ticks: u32,
}

impl Default for StandingOffer {
    fn default() -> Self {
        Self {
            good: GoodKind::Ore,
            units_per_period: 1,
            period_ticks: MIN_PERIOD_TICKS,
        }
    }
}

impl StandingOffer {
    fn clamped(self) -> Self {
        Self {
            good: self.good,
            units_per_period: self.units_per_period.clamp(0, MAX_UNITS),
            period_ticks: self.period_ticks.clamp(MIN_PERIOD_TICKS, MAX_PERIOD_TICKS),
        }
    }
}

/// What a side would like to receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandingRequest {
    pub good: GoodKind,
    pub units_per_period: u32,
}

impl Default for StandingRequest {
    fn default() -> Self {
        Self {
            good: GoodKind::Lumber,
            units_per_period: 1,
        }
    }
}

impl StandingRequest {
    fn clamped(self) -> Self {
        Self {
            good: self.good,
            units_per_period: self.units_per_period.clamp(0, MAX_UNITS),
        }
    }
}

/// The far town's skyline, as roof heights only.
///
/// Twelve small numbers is enough to read as a place on the horizon and is not
/// enough to be anybody's map — §3.2's "enough to read as a place, not enough
/// to be their map", made literal.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Silhouette {
    /// Roof heights left to right, `0..=15`. Zero is a gap.
    pub roofs: Vec<u8>,
}

impl Silhouette {
    pub fn new(roofs: Vec<u8>) -> Self {
        Self { roofs }.clamped()
    }

    fn clamped(mut self) -> Self {
        self.roofs.truncate(SILHOUETTE_ROOFS);
        for roof in &mut self.roofs {
            *roof = (*roof).min(15);
        }
        self
    }

    /// Tallest roof, for scaling the yard art.
    pub fn tallest(&self) -> u8 {
        self.roofs.iter().copied().max().unwrap_or(0)
    }
}

/// A headline stat or two — enough for the panel to say how they are doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HeadlineStat {
    pub residents: u32,
    pub stations: u32,
}

/// Everything a neighbour publishes about themselves.
///
/// Nothing appears in your Border Yard that is not in here, which sidesteps the
/// privacy question entirely (§3.2).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Presence {
    /// From a curated generator or a filtered list — never free text (§8.1).
    pub town_name: String,
    pub headline: HeadlineStat,
    pub silhouette: Silhouette,
    pub source: PresenceSource,
}

impl Presence {
    fn clamped(mut self) -> Self {
        if self.town_name.chars().count() > MAX_TOWN_NAME_LEN {
            self.town_name = self.town_name.chars().take(MAX_TOWN_NAME_LEN).collect();
        }
        self.silhouette = self.silhouette.clamped();
        self.headline.residents = self.headline.residents.min(1_000_000);
        self.headline.stations = self.headline.stations.min(1_000);
        self
    }
}

/// One side of a border link, as published.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BorderManifest {
    pub schema_version: u16,
    pub link: LinkId,
    /// The *sender's* edge. Kept for routing and for the panel's "their west,
    /// your east" phrasing; it says nothing about map contents.
    pub edge: BorderEdge,
    /// Monotonic per link. Ordering and idempotent replay.
    pub sequence: u64,
    pub departures: Vec<Departure>,
    pub offer: StandingOffer,
    pub request: StandingRequest,
    pub presence: Presence,
}

impl Default for BorderManifest {
    fn default() -> Self {
        Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            link: LinkId(0),
            edge: BorderEdge::North,
            sequence: 0,
            departures: Vec::new(),
            offer: StandingOffer::default(),
            request: StandingRequest::default(),
            presence: Presence::default(),
        }
    }
}

impl BorderManifest {
    /// Accept a manifest from anywhere, clamped to sane bounds, or refuse it.
    ///
    /// [`None`] means "this one is not usable" — an unknown schema version or a
    /// link that is not the one we asked for. The caller's answer to [`None`] is
    /// always to keep using the cached neighbour, never to wait (§2.1).
    pub fn sanitised(self, expect_link: LinkId) -> Option<Self> {
        if self.schema_version != MANIFEST_SCHEMA_VERSION {
            return None;
        }
        if self.link != expect_link {
            return None;
        }
        let mut clamped = self;
        clamped.offer = clamped.offer.clamped();
        clamped.request = clamped.request.clamped();
        clamped.presence = clamped.presence.clamped();
        for departure in &mut clamped.departures {
            departure.units = departure.units.min(MAX_UNITS);
        }
        // Newest departures are the interesting ones.
        if clamped.departures.len() > MAX_DEPARTURES {
            let drop = clamped.departures.len() - MAX_DEPARTURES;
            clamped.departures.drain(0..drop);
        }
        Some(clamped)
    }

    /// Record a crossing, keeping the list bounded.
    pub fn push_departure(&mut self, departure: Departure) {
        self.departures.push(departure);
        while self.departures.len() > MAX_DEPARTURES {
            self.departures.remove(0);
        }
    }

    /// Whether this manifest supersedes `other` for the same link.
    ///
    /// Replay of an older or equal sequence is a no-op, so a blob store that
    /// hands the same bytes back twice costs nothing.
    pub fn supersedes(&self, other: &Self) -> bool {
        self.link == other.link && self.sequence > other.sequence
    }

    pub fn is_echo(&self) -> bool {
        self.presence.source.is_echo()
    }

    pub fn town_name(&self) -> &str {
        &self.presence.town_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maximal() -> BorderManifest {
        BorderManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            link: LinkId(u64::MAX),
            edge: BorderEdge::West,
            sequence: u64::MAX,
            departures: (0..MAX_DEPARTURES)
                .map(|i| Departure {
                    tick: u64::MAX - i as u64,
                    good: Some(GoodKind::Lumber),
                    units: MAX_UNITS,
                })
                .collect(),
            offer: StandingOffer {
                good: GoodKind::Ore,
                units_per_period: MAX_UNITS,
                period_ticks: MAX_PERIOD_TICKS,
            },
            request: StandingRequest {
                good: GoodKind::Lumber,
                units_per_period: MAX_UNITS,
            },
            presence: Presence {
                town_name: "M".repeat(MAX_TOWN_NAME_LEN),
                headline: HeadlineStat {
                    residents: 1_000_000,
                    stations: 1_000,
                },
                silhouette: Silhouette::new(vec![15; SILHOUETTE_ROOFS]),
                source: PresenceSource::Echo,
            },
        }
    }

    /// §4.1: kilobytes, not megabytes. If this ever fails, something that can
    /// reconstruct a map has probably been added to the manifest.
    #[test]
    fn a_full_manifest_is_kilobytes() {
        let bytes = bincode::serde::encode_to_vec(maximal(), bincode::config::standard())
            .expect("manifest encodes");
        assert!(
            bytes.len() < 1_024,
            "a maximal manifest must stay under a kilobyte, got {}",
            bytes.len()
        );
    }

    #[test]
    fn a_manifest_round_trips_through_bytes() {
        let manifest = maximal();
        let bytes = bincode::serde::encode_to_vec(&manifest, bincode::config::standard())
            .expect("encode");
        let (back, _): (BorderManifest, usize) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
                .expect("decode");
        assert_eq!(back, manifest);
    }

    #[test]
    fn an_unknown_schema_version_is_refused() {
        let mut manifest = maximal();
        manifest.schema_version = MANIFEST_SCHEMA_VERSION + 1;
        assert_eq!(manifest.clone().sanitised(LinkId(u64::MAX)), None);
        // …and the right version for the wrong link is refused too.
        assert_eq!(maximal().sanitised(LinkId(7)), None);
    }

    #[test]
    fn absurd_quantities_are_clamped_not_rejected() {
        let mut manifest = maximal();
        manifest.offer.units_per_period = 9_999_999;
        manifest.offer.period_ticks = 1;
        manifest.request.units_per_period = 9_999_999;
        manifest.presence.town_name = "Z".repeat(400);
        manifest.presence.silhouette.roofs = vec![200; 400];
        manifest.departures.push(Departure {
            tick: 1,
            good: None,
            units: u32::MAX,
        });

        let clean = manifest.sanitised(LinkId(u64::MAX)).expect("still usable");
        assert_eq!(clean.offer.units_per_period, MAX_UNITS);
        assert_eq!(clean.offer.period_ticks, MIN_PERIOD_TICKS);
        assert_eq!(clean.request.units_per_period, MAX_UNITS);
        assert_eq!(clean.presence.town_name.chars().count(), MAX_TOWN_NAME_LEN);
        assert_eq!(clean.presence.silhouette.roofs.len(), SILHOUETTE_ROOFS);
        assert!(clean.presence.silhouette.roofs.iter().all(|r| *r <= 15));
        assert_eq!(clean.departures.len(), MAX_DEPARTURES);
        assert!(clean.departures.iter().all(|d| d.units <= MAX_UNITS));
    }

    #[test]
    fn replaying_an_old_sequence_is_a_no_op() {
        let mut old = maximal();
        old.sequence = 4;
        let mut new = maximal();
        new.sequence = 5;
        assert!(new.supersedes(&old));
        assert!(!old.supersedes(&new));
        assert!(!new.supersedes(&new), "same sequence is idempotent");
    }

    #[test]
    fn departures_stay_bounded() {
        let mut manifest = BorderManifest::default();
        for tick in 0..(MAX_DEPARTURES as u64 * 3) {
            manifest.push_departure(Departure {
                tick,
                good: Some(GoodKind::Ore),
                units: 1,
            });
        }
        assert_eq!(manifest.departures.len(), MAX_DEPARTURES);
        assert_eq!(
            manifest.departures.last().map(|d| d.tick),
            Some(MAX_DEPARTURES as u64 * 3 - 1),
            "the newest crossing survives"
        );
    }

    #[test]
    fn an_echo_says_so_in_the_data() {
        let manifest = maximal();
        assert!(manifest.is_echo());
        assert_eq!(manifest.presence.source.label(), "echo");
        assert_eq!(PresenceSource::Linked.label(), "linked");
        assert!(!PresenceSource::Linked.is_echo());
    }
}
