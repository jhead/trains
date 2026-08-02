//! District character — what a place *becomes* depends on what serves it.
//!
//! Brief 06 §2.2: residential near a passenger station, warehouses and yards
//! near a goods facility, commercial frontage along a busy corridor, and rural
//! everywhere else. The last one carries as much weight as the others: unserved
//! ground has to look *deliberately* rural so that a thriving district reads as
//! a consequence rather than as the only finished part of the map.

use rail_sim::{
    GoodKind, IndustryRegistry, StationRegistry, TileCoord, TrackNetwork, GROUND_LAYER,
    GROWTH_RADIUS,
};

use super::building_art::{Family, Roof};

/// Chebyshev reach of a goods facility's works district.
pub const INDUSTRIAL_RADIUS: i32 = 3;
/// Within this many tiles of a station, track frontage turns commercial.
pub const FRONTAGE_RADIUS: i32 = 3;
/// A corridor is a tile with this many track pieces in its 3×3 neighbourhood.
pub const CORRIDOR_PIECES: usize = 2;

/// What a tile's lots grow into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum District {
    /// No station in reach — farms, a lane, scattered cottages.
    #[default]
    Rural,
    /// Housing thickening around a passenger station.
    Residential,
    /// Shop frontage facing a busy line.
    Commercial,
    /// Warehouses, yards and workshops around a goods facility.
    Industrial,
}

impl District {
    /// Silhouette family this district builds from.
    pub fn family(self) -> Family {
        match self {
            Self::Industrial => Family::Works,
            _ => Family::Town,
        }
    }

    /// Tier bias applied to the density-derived growth score.
    ///
    /// Commercial frontage reaches shops and blocks sooner than back streets;
    /// rural land never gets past a cottage.
    pub fn tier_bias(self) -> f32 {
        match self {
            Self::Commercial => 0.7,
            Self::Industrial => 0.25,
            Self::Residential => 0.0,
            Self::Rural => -1.5,
        }
    }

    /// Hard ceiling on tier, regardless of how long a district sits there.
    pub fn tier_cap(self) -> u8 {
        match self {
            Self::Rural => 0,
            _ => 3,
        }
    }

    /// Lots a tile of this district will ever fill.
    pub fn lot_cap(self) -> u8 {
        match self {
            Self::Rural => 1,
            Self::Industrial => 3,
            _ => 4,
        }
    }
}

/// Chebyshev distance in tiles.
#[inline]
fn cheb(a: TileCoord, b: TileCoord) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

/// True when `tile` sits on a run of track rather than beside a lone stub.
pub fn is_corridor(tile: TileCoord, network: &TrackNetwork) -> bool {
    let mut pieces = 0;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let probe = TileCoord {
                x: tile.x + dx,
                y: tile.y + dy,
            };
            if network.at(probe, GROUND_LAYER).is_some() {
                pieces += 1;
            }
        }
    }
    pieces >= CORRIDOR_PIECES
}

/// Resolve a tile's district from what actually serves it.
pub fn classify(
    tile: TileCoord,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    network: &TrackNetwork,
) -> District {
    let station = stations
        .iter()
        .map(|s| cheb(s.tile, tile))
        .min()
        .unwrap_or(i32::MAX);
    let industry = industries
        .iter()
        .map(|i| cheb(i.tile, tile))
        .min()
        .unwrap_or(i32::MAX);

    if industry <= INDUSTRIAL_RADIUS && industry <= station {
        return District::Industrial;
    }
    if station > GROWTH_RADIUS {
        return District::Rural;
    }
    if station <= FRONTAGE_RADIUS && is_corridor(tile, network) {
        return District::Commercial;
    }
    District::Residential
}

/// Which goods the nearest facility handles, for flavour in the works district.
///
/// Kept separate from [`classify`] so the district decision stays cheap; a
/// lumber yard and an ore yard differ by variant, not by silhouette family.
pub fn nearest_good(tile: TileCoord, industries: &IndustryRegistry) -> Option<GoodKind> {
    industries
        .iter()
        .filter(|i| cheb(i.tile, tile) <= INDUSTRIAL_RADIUS)
        .min_by_key(|i| cheb(i.tile, tile))
        .and_then(|i| i.produces.or(i.consumes))
}

/// Roof material for a lot, hashed on its world-anchored lot coordinate.
pub fn roof_for(lot_x: i32, lot_y: i32, district: District) -> Roof {
    // Slate creeps in as a district densifies; tile stays the country default.
    let bias = match district {
        District::Rural => 25,
        District::Residential => 45,
        District::Commercial => 60,
        District::Industrial => 75,
    };
    if super::building_art::world_hash(lot_x, lot_y, 0x2F00) % 100 < bias {
        Roof::Slate
    } else {
        Roof::Tile
    }
}

/// A works building's variant leans on what the facility moves.
pub fn works_variant(lot_x: i32, lot_y: i32, good: Option<GoodKind>) -> u8 {
    let base = (super::building_art::world_hash(lot_x, lot_y, 0x0B0B) % 4) as u8;
    match good {
        Some(GoodKind::Lumber) => base & 0b01,
        Some(GoodKind::Ore) => 2 | (base & 0b01),
        None => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::TrackNetwork;

    fn stations_at(tiles: &[(i32, i32)]) -> StationRegistry {
        let mut reg = StationRegistry::new();
        for (i, (x, y)) in tiles.iter().enumerate() {
            reg.insert(
                format!("S{i}"),
                TileCoord { x: *x, y: *y },
                GROUND_LAYER,
            );
        }
        reg
    }

    fn industries_at(tiles: &[(i32, i32)]) -> IndustryRegistry {
        let mut reg = IndustryRegistry::new();
        for (i, (x, y)) in tiles.iter().enumerate() {
            reg.insert(
                format!("I{i}"),
                TileCoord { x: *x, y: *y },
                Some(GoodKind::Lumber),
                None,
            );
        }
        reg
    }

    #[test]
    fn far_from_everything_is_rural() {
        let stations = stations_at(&[(0, 0)]);
        let industries = IndustryRegistry::new();
        let network = TrackNetwork::new();
        assert_eq!(
            classify(TileCoord { x: 40, y: 40 }, &stations, &industries, &network),
            District::Rural
        );
    }

    #[test]
    fn near_a_passenger_station_is_residential() {
        let stations = stations_at(&[(10, 10)]);
        let industries = IndustryRegistry::new();
        let network = TrackNetwork::new();
        assert_eq!(
            classify(TileCoord { x: 12, y: 10 }, &stations, &industries, &network),
            District::Residential
        );
    }

    #[test]
    fn near_a_goods_facility_is_industrial() {
        let stations = stations_at(&[(10, 10)]);
        let industries = industries_at(&[(12, 12)]);
        let network = TrackNetwork::new();
        assert_eq!(
            classify(TileCoord { x: 13, y: 12 }, &stations, &industries, &network),
            District::Industrial
        );
    }

    #[test]
    fn rural_never_grows_past_a_cottage() {
        assert_eq!(District::Rural.tier_cap(), 0);
        assert_eq!(District::Rural.lot_cap(), 1);
        assert!(District::Rural.tier_bias() < 0.0);
    }

    #[test]
    fn commercial_reaches_shops_sooner_than_back_streets() {
        assert!(District::Commercial.tier_bias() > District::Residential.tier_bias());
    }
}
