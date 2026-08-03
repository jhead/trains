//! Station tiers — the four platform grades the player can build.
//!
//! A station is a kind of track ([04 — Building & Tools] §6): it is placed on a
//! piece of line as a platform, so a tier is mostly a statement about how much
//! railway the stop occupies. Platforms drive everything else — a wider stop
//! reaches further, turns trains around faster, and holds more people.
//!
//! | Tier | Platforms | Catchment | Dwell | Capacity |
//! | --- | --- | --- | --- | --- |
//! | Halt | 1 short | 3 | 1.5× | 6 |
//! | Station | 2 | 5 | 1× | 14 |
//! | Interchange | 4 | 8 | 0.6× | 32 |
//! | Terminus | 3 stub | 6 | 0.9× | 24 |
//! | Goods platform | 2 | 1 | 1.4× | 4 |
//!
//! §6's last line — "freight facilities work the same way: a goods platform
//! placed against an industry" — is the fifth row. It is a platform like any
//! other, built with the same tool on the same line, and it carries one extra
//! rule: it only stands where it touches an industry lot
//! ([`Industry::abuts`](super::industry::Industry::abuts)). Its catchment is
//! almost nothing because it serves a works, not a town.
//!
//! [`StationTier::Station`] is the default so auto-seeded anchors keep the
//! pre-tier catchment ([`crate::town::GROWTH_RADIUS`] = 5).

use serde::{Deserialize, Serialize};

use crate::ids::TileCoord;

use super::registry::{Station, StationRegistry};

/// Halt — one short platform: $400.00 = 4x a flat track tile.
pub const HALT_COST_CENTS: i64 = 40_000;

/// Station — two platforms, the workhorse: $1,200.00.
pub const STATION_COST_CENTS: i64 = 120_000;

/// Interchange — four platforms where lines meet: $4,000.00.
pub const INTERCHANGE_COST_CENTS: i64 = 400_000;

/// Terminus — three stub platforms, end of line: $2,600.00.
pub const TERMINUS_COST_CENTS: i64 = 260_000;

/// Goods platform — two loading faces against an industry: $900.00.
pub const GOODS_PLATFORM_COST_CENTS: i64 = 90_000;

/// Minimum Chebyshev tiles between two stations — a platform every tile is not
/// a railway, it is a tram.
pub const MIN_STATION_SPACING: i32 = 3;

/// Per-tier parameters. Mirrors [`TrainProfile`](crate::trains::TrainProfile):
/// price alone is not a tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StationTierSpec {
    /// Contiguous straight track tiles the stop needs (its platforms).
    pub platforms: u8,
    /// Chebyshev radius (tiles) the stop draws town growth and demand from.
    pub catchment: i32,
    /// Dwell as a percentage of the train profile's own
    /// [`dwell_ticks`](crate::trains::TrainProfile::dwell_ticks). Lower = brisker.
    pub dwell_percent: u16,
    /// Waiting peeps the platform holds before crowding drags the score down.
    pub capacity: u32,
    /// Construction cost in cents.
    pub build_cents: i64,
    /// Operating cost in cents per **real minute**, spread smoothly across
    /// ticks by [`crate::economy::apply_track_maintenance`]. See
    /// [`crate::economy::opex`] on which minute this is and why it matters.
    ///
    /// Design 08 §3.3: *"an interchange nobody uses is a genuine liability"*.
    /// An interchange on a branch nobody rides costs `$80` a minute to keep
    /// the lights on — its build price again every fifty minutes, forever.
    pub maint_cents_per_real_min: i64,
    /// Score added per arrival (bigger stops turn service into reputation faster).
    pub arrival_gain: u8,
    /// `false` for stub ends — a terminus cannot be run through.
    pub through_running: bool,
}

/// Cheap, slow to board, serves a small catchment.
pub const HALT_SPEC: StationTierSpec = StationTierSpec {
    platforms: 1,
    catchment: 3,
    dwell_percent: 150,
    capacity: 6,
    build_cents: HALT_COST_CENTS,
    maint_cents_per_real_min: 1_000,
    arrival_gain: 6,
    through_running: true,
};

/// The workhorse — two platforms, middling everything.
pub const STATION_SPEC: StationTierSpec = StationTierSpec {
    platforms: 2,
    catchment: 5, // matches the pre-tier GROWTH_RADIUS
    dwell_percent: 100,
    capacity: 14,
    build_cents: STATION_COST_CENTS,
    maint_cents_per_real_min: 3_000,
    arrival_gain: 8,
    through_running: true,
};

/// Expensive, fast turnaround, wide catchment, lines can meet.
pub const INTERCHANGE_SPEC: StationTierSpec = StationTierSpec {
    platforms: 4,
    catchment: 8,
    dwell_percent: 60,
    capacity: 32,
    build_cents: INTERCHANGE_COST_CENTS,
    maint_cents_per_real_min: 8_000,
    arrival_gain: 12,
    through_running: true,
};

/// End-of-line, high capacity, no through running.
pub const TERMINUS_SPEC: StationTierSpec = StationTierSpec {
    platforms: 3,
    catchment: 6,
    dwell_percent: 90,
    capacity: 24,
    build_cents: TERMINUS_COST_CENTS,
    maint_cents_per_real_min: 5_500,
    arrival_gain: 10,
    through_running: false,
};

/// Freight facility — two loading faces, built against an industry lot.
///
/// Long dwell (loading, not boarding), almost no catchment: it serves the works
/// it stands against, and a town does not grow around a coal drop.
pub const GOODS_PLATFORM_SPEC: StationTierSpec = StationTierSpec {
    platforms: 2,
    catchment: 1,
    dwell_percent: 140,
    capacity: 4,
    build_cents: GOODS_PLATFORM_COST_CENTS,
    // Between a Halt and a Station, on the same real-minute scale: a loading
    // face is cheap to keep, but it is still staff and hardstanding.
    maint_cents_per_real_min: 2_500,
    arrival_gain: 6,
    through_running: true,
};

/// Platform grade of a stop.
///
/// Halt → Station → Interchange is the in-place upgrade ladder. Terminus and
/// GoodsPlatform are siblings, not rungs: each is a different shape of railway,
/// so they are built rather than upgraded into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StationTier {
    Halt,
    #[default]
    Station,
    Interchange,
    Terminus,
    /// 04 §6 last line — freight, placed against an industry.
    ///
    /// Deliberately last: the discriminant order is the save format's, and a
    /// new variant in the middle would re-read every stored tier as another one.
    GoodsPlatform,
}

impl StationTier {
    pub const ALL: [StationTier; 5] = [
        Self::Halt,
        Self::Station,
        Self::Interchange,
        Self::Terminus,
        Self::GoodsPlatform,
    ];

    pub fn spec(self) -> StationTierSpec {
        match self {
            Self::Halt => HALT_SPEC,
            Self::Station => STATION_SPEC,
            Self::Interchange => INTERCHANGE_SPEC,
            Self::Terminus => TERMINUS_SPEC,
            Self::GoodsPlatform => GOODS_PLATFORM_SPEC,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Halt => "Halt",
            Self::Station => "Station",
            Self::Interchange => "Interchange",
            Self::Terminus => "Terminus",
            Self::GoodsPlatform => "Goods Platform",
        }
    }

    /// `true` for the freight tier, which must stand against an industry lot.
    #[inline]
    pub fn needs_industry(self) -> bool {
        matches!(self, Self::GoodsPlatform)
    }

    #[inline]
    pub fn platforms(self) -> u8 {
        self.spec().platforms
    }

    #[inline]
    pub fn catchment(self) -> i32 {
        self.spec().catchment
    }

    #[inline]
    pub fn capacity(self) -> u32 {
        self.spec().capacity
    }

    #[inline]
    pub fn build_cents(self) -> i64 {
        self.spec().build_cents
    }

    #[inline]
    pub fn maint_cents_per_real_min(self) -> i64 {
        self.spec().maint_cents_per_real_min
    }

    #[inline]
    pub fn arrival_gain(self) -> u8 {
        self.spec().arrival_gain
    }

    /// `false` only for [`StationTier::Terminus`] (stub platforms).
    #[inline]
    pub fn allows_through_running(self) -> bool {
        self.spec().through_running
    }

    /// Dwell ticks for a train whose profile dwells `base` ticks here.
    ///
    /// Never returns `0` — a train always stops long enough to be seen stopping.
    pub fn dwell_ticks(self, base: u16) -> u16 {
        let scaled =
            (u32::from(base) * u32::from(self.spec().dwell_percent) + 50) / 100;
        (scaled.min(u32::from(u16::MAX)) as u16).max(1)
    }

    /// Position on the upgrade ladder, or `None` for the off-ladder tiers.
    pub fn rank(self) -> Option<u8> {
        match self {
            Self::Halt => Some(0),
            Self::Station => Some(1),
            Self::Interchange => Some(2),
            Self::Terminus | Self::GoodsPlatform => None,
        }
    }

    /// Next rung up, if any.
    pub fn next_upgrade(self) -> Option<StationTier> {
        match self {
            Self::Halt => Some(Self::Station),
            Self::Station => Some(Self::Interchange),
            Self::Interchange | Self::Terminus | Self::GoodsPlatform => None,
        }
    }

    /// Both tiers sit on the Halt → Station → Interchange ladder.
    pub fn on_ladder_with(self, other: StationTier) -> bool {
        self.rank().is_some() && other.rank().is_some()
    }

    /// Signed cents to move a stop from `self` to `to`: positive is a charge,
    /// negative is a refund.
    ///
    /// The delta is the plain build-cost difference, so an upgrade followed by
    /// its undo returns the balance exactly.
    #[inline]
    pub fn retier_cents(self, to: StationTier) -> i64 {
        to.build_cents().saturating_sub(self.build_cents())
    }
}

/// Catchment influence of one station at `tile` (`0.0..=1.0`).
///
/// `influence = (score / 100) * (1 - dist / (catchment + 1))` with Chebyshev
/// distance, so a wider tier both reaches further *and* falls off more gently.
pub fn catchment_influence(station: &Station, score: u8, tile: TileCoord) -> f32 {
    let radius = station.tier.catchment();
    let dx = (station.tile.x - tile.x).abs();
    let dy = (station.tile.y - tile.y).abs();
    let dist = dx.max(dy);
    if radius <= 0 || dist > radius {
        return 0.0;
    }
    let quality = score as f32 / 100.0;
    let falloff = 1.0 - (dist as f32) / ((radius + 1) as f32);
    (quality * falloff).clamp(0.0, 1.0)
}

/// Widest catchment currently standing — how far growth rings must be scanned.
pub fn max_catchment(stations: &StationRegistry) -> i32 {
    stations
        .iter()
        .map(|s| s.tier.catchment())
        .max()
        .unwrap_or(0)
}

/// Sum per-tier station maintenance for the current registry.
pub fn station_maintenance_total(stations: &StationRegistry) -> i64 {
    stations
        .iter()
        .map(|s| s.tier.maint_cents_per_real_min())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::GROUND_LAYER;

    #[test]
    fn tiers_spread_cost_catchment_and_capacity() {
        // Cost rises with platforms; the terminus is priced between the two.
        assert!(HALT_COST_CENTS < STATION_COST_CENTS);
        assert!(STATION_COST_CENTS < TERMINUS_COST_CENTS);
        assert!(TERMINUS_COST_CENTS < INTERCHANGE_COST_CENTS);

        assert_eq!(StationTier::Halt.platforms(), 1);
        assert_eq!(StationTier::Station.platforms(), 2);
        assert_eq!(StationTier::Terminus.platforms(), 3);
        assert_eq!(StationTier::Interchange.platforms(), 4);

        assert!(StationTier::Interchange.catchment() > StationTier::Station.catchment());
        assert!(StationTier::Station.catchment() > StationTier::Halt.catchment());
        assert!(StationTier::Interchange.capacity() > StationTier::Halt.capacity());
    }

    #[test]
    fn station_tier_is_the_default_so_seeded_anchors_keep_growth_radius() {
        assert_eq!(StationTier::default(), StationTier::Station);
        assert_eq!(
            StationTier::Station.catchment(),
            crate::town::GROWTH_RADIUS,
            "default tier must preserve pre-tier growth reach"
        );
    }

    #[test]
    fn interchange_turns_trains_faster_than_a_halt() {
        // Transit dwells 2 ticks; transport 6.
        let transit = crate::trains::TRANSIT_PROFILE.dwell_ticks;
        let transport = crate::trains::TRANSPORT_PROFILE.dwell_ticks;

        assert!(
            StationTier::Interchange.dwell_ticks(transport)
                < StationTier::Station.dwell_ticks(transport)
        );
        assert!(
            StationTier::Halt.dwell_ticks(transport)
                > StationTier::Station.dwell_ticks(transport)
        );
        assert_eq!(StationTier::Station.dwell_ticks(transport), transport);
        // Never instant, however brisk the tier.
        assert!(StationTier::Interchange.dwell_ticks(transit) >= 1);
        assert!(StationTier::Interchange.dwell_ticks(0) >= 1);
    }

    #[test]
    fn only_the_terminus_refuses_through_running() {
        for tier in StationTier::ALL {
            assert_eq!(
                tier.allows_through_running(),
                tier != StationTier::Terminus,
                "{} through-running",
                tier.label()
            );
        }
    }

    #[test]
    fn upgrade_ladder_skips_the_terminus() {
        assert_eq!(StationTier::Halt.next_upgrade(), Some(StationTier::Station));
        assert_eq!(
            StationTier::Station.next_upgrade(),
            Some(StationTier::Interchange)
        );
        assert_eq!(StationTier::Interchange.next_upgrade(), None);
        assert_eq!(StationTier::Terminus.next_upgrade(), None);
        assert_eq!(StationTier::GoodsPlatform.next_upgrade(), None);
        assert!(!StationTier::Station.on_ladder_with(StationTier::Terminus));
        assert!(
            !StationTier::Station.on_ladder_with(StationTier::GoodsPlatform),
            "a passenger stop is not a freight facility with more platforms"
        );
        assert!(StationTier::Halt.on_ladder_with(StationTier::Interchange));
    }

    /// 04 §6 last line: freight is a platform tier of its own.
    #[test]
    fn the_goods_platform_is_freight_shaped() {
        let goods = StationTier::GoodsPlatform;
        assert!(goods.needs_industry(), "it is placed against an industry");
        assert!(
            StationTier::ALL
                .iter()
                .filter(|t| t.needs_industry())
                .count()
                == 1,
            "no passenger tier asks for an industry"
        );
        // It serves a works, not a town, so it draws almost no catchment.
        assert!(goods.catchment() < StationTier::Halt.catchment());
        assert!(goods.capacity() < StationTier::Halt.capacity());
        // Loading is slower than boarding (07 §3: freight dwell scales with load).
        let transport = crate::trains::TRANSPORT_PROFILE.dwell_ticks;
        assert!(goods.dwell_ticks(transport) > StationTier::Station.dwell_ticks(transport));
        // Priced between a halt and the workhorse: cheap enough to serve a
        // sawmill on spec, dear enough to be a decision.
        assert!(goods.build_cents() > HALT_COST_CENTS);
        assert!(goods.build_cents() < STATION_COST_CENTS);
    }

    #[test]
    fn retier_delta_is_symmetric_so_undo_is_exact() {
        let up = StationTier::Halt.retier_cents(StationTier::Interchange);
        let down = StationTier::Interchange.retier_cents(StationTier::Halt);
        assert_eq!(up, INTERCHANGE_COST_CENTS - HALT_COST_CENTS);
        assert_eq!(up + down, 0);
    }

    #[test]
    fn catchment_influence_widens_with_tier() {
        let mut reg = StationRegistry::new();
        let id = reg.insert_tier(
            "Ashford Halt",
            TileCoord { x: 10, y: 10 },
            GROUND_LAYER,
            StationTier::Halt,
            HALT_COST_CENTS,
        );
        let far = TileCoord { x: 16, y: 10 }; // 6 tiles out

        let halt = reg.get(id).expect("station");
        assert_eq!(
            catchment_influence(halt, 100, far),
            0.0,
            "a halt must not reach 6 tiles"
        );

        reg.set_tier(id, StationTier::Interchange, INTERCHANGE_COST_CENTS);
        let interchange = reg.get(id).expect("station");
        assert!(
            catchment_influence(interchange, 100, far) > 0.0,
            "an interchange must reach 6 tiles"
        );

        // Nearer tiles still favour the wider tier.
        let near = TileCoord { x: 12, y: 10 };
        let wide = catchment_influence(interchange, 100, near);
        reg.set_tier(id, StationTier::Halt, HALT_COST_CENTS);
        let narrow = catchment_influence(reg.get(id).expect("station"), 100, near);
        assert!(wide > narrow, "interchange {wide} should beat halt {narrow}");
    }

    #[test]
    fn maintenance_scales_with_the_standing_tiers() {
        let mut reg = StationRegistry::new();
        reg.insert_tier(
            "Ashford Halt",
            TileCoord { x: 1, y: 1 },
            GROUND_LAYER,
            StationTier::Halt,
            HALT_COST_CENTS,
        );
        assert_eq!(station_maintenance_total(&reg), HALT_SPEC.maint_cents_per_real_min);
        assert_eq!(max_catchment(&reg), HALT_SPEC.catchment);

        reg.insert_tier(
            "Brackwell Interchange",
            TileCoord { x: 9, y: 9 },
            GROUND_LAYER,
            StationTier::Interchange,
            INTERCHANGE_COST_CENTS,
        );
        assert_eq!(
            station_maintenance_total(&reg),
            HALT_SPEC.maint_cents_per_real_min + INTERCHANGE_SPEC.maint_cents_per_real_min
        );
        assert_eq!(max_catchment(&reg), INTERCHANGE_SPEC.catchment);
    }
}
