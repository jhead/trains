//! Pure validity / cost / catchment preview for the station ghost.
//!
//! Siting a station is a real decision made with real information
//! ([04 — Building & Tools] §6): the ring shows how far the stop reaches, and
//! the readout counts the buildings and the unserved anchors already inside it.

use rail_sim::ids::{StationId, TileCoord};
use rail_sim::stations::{
    best_platform_run, validate_station_site, StationPlacementError, StationRegistry, StationTier,
};
use rail_sim::track::{opposite_dir, step};
use rail_sim::{
    DemandSpawner, IndustryRegistry, LineRegistry, Money, TownDensity, TrackNetwork, GROUND_LAYER,
};

/// Density at or above this reads as a standing building.
const BUILDING_DENSITY: f32 = 0.15;

#[derive(Debug, Clone, PartialEq)]
pub struct StationPreview {
    pub tile: TileCoord,
    pub tier: StationTier,
    pub cost_cents: i64,
    pub balance_after_cents: i64,
    pub catchment: i32,
    /// Perimeter of the catchment square — the ring drawn during the drag.
    pub ring: Vec<TileCoord>,
    /// Track tiles the platforms would occupy (empty when the site is illegal).
    pub platforms: Vec<TileCoord>,
    /// Standing buildings inside the catchment.
    pub buildings: u32,
    /// Revealed-but-unconnected anchors inside the catchment.
    pub unserved: u32,
    /// Industry a goods platform here would load (04 §6), if any.
    pub serves: Option<String>,
    pub can_commit: bool,
    pub reject: Option<String>,
}

/// Everything the ghost needs about a candidate site.
#[allow(clippy::too_many_arguments)]
pub fn preview_station(
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    network: &TrackNetwork,
    density: &TownDensity,
    demand: &DemandSpawner,
    money: &Money,
    tile: TileCoord,
    tier: StationTier,
) -> StationPreview {
    let balance = money.cents();
    let cost = tier.build_cents();
    let catchment = tier.catchment();

    let outcome =
        validate_station_site(stations, industries, network, tile, GROUND_LAYER, tier, None);
    let mut reject = outcome
        .as_ref()
        .err()
        .map(|err| station_reason(*err, cost, balance));
    if reject.is_none() && cost > balance {
        reject = Some(station_reason(
            StationPlacementError::InsufficientFunds,
            cost,
            balance,
        ));
    }

    let platforms = if outcome.is_ok() {
        platform_tiles(network, tile, tier)
    } else {
        Vec::new()
    };

    StationPreview {
        tile,
        tier,
        cost_cents: cost,
        balance_after_cents: balance - cost,
        catchment,
        ring: catchment_ring(tile, catchment),
        platforms,
        buildings: buildings_in_catchment(density, tile, catchment),
        unserved: unserved_in_catchment(stations, industries, demand, tile, catchment),
        serves: tier
            .needs_industry()
            .then(|| industries.abutting(tile).map(|i| i.name.clone()))
            .flatten(),
        can_commit: reject.is_none(),
        reject,
    }
}

/// The track tiles a `tier`'s platforms would stand on, in run order.
pub fn platform_tiles(
    network: &TrackNetwork,
    tile: TileCoord,
    tier: StationTier,
) -> Vec<TileCoord> {
    let Some(run) = best_platform_run(network, tile, GROUND_LAYER, tier) else {
        return Vec::new();
    };
    let need = tier.platforms() as usize;
    let mut tiles = vec![tile];

    // Extend along the run, forward first, then back — a stub always grows inward.
    for dir in [run.axis, opposite_dir(run.axis)] {
        let mut cursor = tile;
        while tiles.len() < need {
            cursor = step(cursor, dir);
            if network.id_at(cursor, GROUND_LAYER).is_none() {
                break;
            }
            tiles.push(cursor);
        }
    }
    tiles.sort_by_key(|t| (t.y, t.x));
    tiles
}

/// Perimeter tiles of the catchment square (Chebyshev radius).
pub fn catchment_ring(tile: TileCoord, radius: i32) -> Vec<TileCoord> {
    if radius <= 0 {
        return vec![tile];
    }
    let mut ring = Vec::with_capacity((8 * radius) as usize);
    for d in -radius..=radius {
        ring.push(TileCoord {
            x: tile.x + d,
            y: tile.y - radius,
        });
        ring.push(TileCoord {
            x: tile.x + d,
            y: tile.y + radius,
        });
    }
    for d in (-radius + 1)..radius {
        ring.push(TileCoord {
            x: tile.x - radius,
            y: tile.y + d,
        });
        ring.push(TileCoord {
            x: tile.x + radius,
            y: tile.y + d,
        });
    }
    ring
}

fn buildings_in_catchment(density: &TownDensity, tile: TileCoord, radius: i32) -> u32 {
    let mut count = 0u32;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let t = TileCoord {
                x: tile.x + dx,
                y: tile.y + dy,
            };
            if density.get(t) >= BUILDING_DENSITY {
                count += 1;
            }
        }
    }
    count
}

fn unserved_in_catchment(
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    demand: &DemandSpawner,
    tile: TileCoord,
    radius: i32,
) -> u32 {
    let within = |t: TileCoord| (t.x - tile.x).abs().max((t.y - tile.y).abs()) <= radius;
    let open_stations = stations
        .iter()
        .filter(|s| demand.is_open_station(s.id) && within(s.tile))
        .count();
    let open_industries = industries
        .iter()
        .filter(|i| demand.is_open_industry(i.id) && within(i.tile))
        .count();
    (open_stations + open_industries) as u32
}

/// Plain-language reason for a [`StationPlacementError`], with the numbers.
pub fn station_reason(err: StationPlacementError, cost: i64, balance: i64) -> String {
    match err {
        StationPlacementError::InvalidLayer => "Can't build on that layer".into(),
        StationPlacementError::NoTrack => "Platforms need track - lay the line first".into(),
        StationPlacementError::AlreadyStation => "Station already here".into(),
        StationPlacementError::TooClose { distance, min } => {
            format!("Too close - {distance} tiles, need {min}")
        }
        StationPlacementError::NoPlatformRoom { have, need } => {
            format!("Not enough platform - {have} tiles of line, needs {need}")
        }
        StationPlacementError::NotAStubEnd => {
            "Terminus needs a dead end - the line runs through here".into()
        }
        StationPlacementError::NoIndustryHere => {
            "Goods platforms load an industry - none touches this tile".into()
        }
        StationPlacementError::InsufficientFunds => {
            let short = (cost - balance).max(0);
            format!("Short by {}", format_dollars(short))
        }
        StationPlacementError::UnknownStation => "No station there".into(),
        StationPlacementError::NotUpgradable { from, to } => {
            format!("Can't upgrade {} to {}", from.label(), to.label())
        }
    }
}

/// What lifting `station` would do to the lines that call there.
///
/// `None` when no line calls there — that demolish is unremarkable and needs no
/// confirming. 04 §4: a demolition with a consequence **names the consequence**
/// rather than being refused, so the first line is who loses a call and the
/// second (when there is one) is which line is left with nowhere to run.
pub fn demolish_consequence(lines: &LineRegistry, station: StationId) -> Option<String> {
    let calling = lines.lines_calling_at(station);
    let named: Vec<&str> = calling
        .iter()
        .filter_map(|id| lines.get(*id))
        .map(|line| line.name.as_str())
        .collect();
    let first = *named.first()?;

    let mut text = match named.len() {
        1 => format!("{first} stops here. Demolish and drop the stop?"),
        2 => format!("{first} and 1 other line stop here. Demolish and drop the stop?"),
        n => format!(
            "{first} and {} other lines stop here. Demolish and drop the stop?",
            n - 1
        ),
    };

    // A line whose remaining calls are all the same stop is going nowhere.
    let stranded: Vec<&str> = calling
        .iter()
        .filter_map(|id| lines.get(*id))
        .filter(|line| {
            let mut left = line.stops.iter().filter(|stop| **stop != station);
            match left.next() {
                None => true,
                Some(first) => left.all(|stop| stop == first),
            }
        })
        .map(|line| line.name.as_str())
        .collect();
    match stranded.len() {
        0 => {}
        1 => text.push_str(&format!("\n{} has nowhere left to run.", stranded[0])),
        n => text.push_str(&format!("\n{n} lines have nowhere left to run.")),
    }
    Some(text)
}

/// One-line summary for the tool's cost HUD.
pub fn station_hud_line(preview: &StationPreview) -> String {
    // A goods platform is sited by what it loads, not by who lives nearby.
    let middle = match &preview.serves {
        Some(industry) => format!("Loads {industry}"),
        None => format!(
            "{} buildings  -  {} unserved",
            preview.buildings, preview.unserved
        ),
    };
    format!(
        "{}  {}  -  {} platforms  -  reach {}\n{middle}\nBalance  {}",
        preview.tier.label(),
        format_dollars(preview.cost_cents),
        preview.tier.platforms(),
        preview.catchment,
        format_dollars(preview.balance_after_cents),
    )
}

/// Local copy of the track HUD's formatter (that module is private to `track`).
pub fn format_dollars(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    let dollars = abs / 100;
    let rem = abs % 100;
    format!("{sign}${dollars}.{rem:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::stations::MIN_STATION_SPACING;
    use rail_sim::track::{try_place_track, TrackTerrain};
    use rail_sim::{IndustryTier, MoneyLedger};

    fn line_of(len: i32) -> TrackNetwork {
        let terrain = TrackTerrain::new(32, 32, (0..32 * 32).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut money = Money::new(10_000_000);
        let mut ledger = MoneyLedger::default();
        for i in 0..len {
            try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                TileCoord { x: 4 + i, y: 8 },
                GROUND_LAYER,
            )
            .expect("track");
        }
        network
    }

    fn preview_at(x: i32, tier: StationTier, money: Money) -> StationPreview {
        preview_station(
            &StationRegistry::new(),
            &IndustryRegistry::new(),
            &line_of(8),
            &TownDensity::default(),
            &DemandSpawner::default(),
            &money,
            TileCoord { x, y: 8 },
            tier,
        )
    }

    #[test]
    fn a_legal_site_reports_cost_reach_and_platforms() {
        let p = preview_at(7, StationTier::Interchange, Money::new(1_000_000));
        assert!(p.can_commit, "{:?}", p.reject);
        assert_eq!(p.cost_cents, StationTier::Interchange.build_cents());
        assert_eq!(p.catchment, StationTier::Interchange.catchment());
        assert_eq!(
            p.platforms.len(),
            StationTier::Interchange.platforms() as usize
        );
        // The ring is the perimeter of the catchment square.
        let r = p.catchment;
        assert_eq!(p.ring.len() as i32, 8 * r);
    }

    #[test]
    fn an_illegal_site_says_why_and_draws_no_platforms() {
        let p = preview_at(20, StationTier::Halt, Money::new(1_000_000));
        assert!(!p.can_commit);
        assert_eq!(
            p.reject.as_deref(),
            Some("Platforms need track - lay the line first")
        );
        assert!(p.platforms.is_empty());
    }

    #[test]
    fn short_funds_name_the_shortfall() {
        let money = Money::new(StationTier::Station.build_cents() - 500);
        let p = preview_at(7, StationTier::Station, money);
        assert!(!p.can_commit);
        assert_eq!(p.reject.as_deref(), Some("Short by $5.00"));
    }

    /// 04 §6 last line: the ghost says what the platform would load, and the
    /// refusal says why there is nothing to load.
    #[test]
    fn a_goods_platform_previews_the_industry_it_would_serve() {
        let mut industries = IndustryRegistry::new();
        industries.insert_tier(
            "Pine Sawmill",
            TileCoord { x: 7, y: 6 },
            IndustryTier::Works,
            None,
            None,
        );
        let p = preview_station(
            &StationRegistry::new(),
            &industries,
            &line_of(8),
            &TownDensity::default(),
            &DemandSpawner::default(),
            &Money::new(1_000_000),
            TileCoord { x: 7, y: 8 },
            StationTier::GoodsPlatform,
        );
        assert!(p.can_commit, "{:?}", p.reject);
        assert_eq!(p.serves.as_deref(), Some("Pine Sawmill"));
        assert!(station_hud_line(&p).contains("Loads Pine Sawmill"));

        // Same line, no lot within reach.
        let p = preview_station(
            &StationRegistry::new(),
            &IndustryRegistry::new(),
            &line_of(8),
            &TownDensity::default(),
            &DemandSpawner::default(),
            &Money::new(1_000_000),
            TileCoord { x: 7, y: 8 },
            StationTier::GoodsPlatform,
        );
        assert!(!p.can_commit);
        assert_eq!(
            p.reject.as_deref(),
            Some("Goods platforms load an industry - none touches this tile")
        );
        assert!(p.serves.is_none());
    }

    /// The dialog copy: who loses a call, and who is left going nowhere.
    #[test]
    fn the_demolish_confirm_names_the_lines_that_call_there() {
        let (a, b, c) = (StationId(1), StationId(2), StationId(3));
        let mut lines = LineRegistry::new();

        assert_eq!(
            demolish_consequence(&lines, b),
            None,
            "a stop no line calls at needs no confirming"
        );

        lines.create("Riverside Loop".into(), vec![a, b, c]).unwrap();
        assert_eq!(
            demolish_consequence(&lines, b).as_deref(),
            Some("Riverside Loop stops here. Demolish and drop the stop?")
        );

        lines.create("Quarry Run".into(), vec![a, b, c]).unwrap();
        assert_eq!(
            demolish_consequence(&lines, b).as_deref(),
            Some("Riverside Loop and 1 other line stop here. Demolish and drop the stop?")
        );

        lines.create("Coast Local".into(), vec![b, c]).unwrap();
        assert_eq!(
            demolish_consequence(&lines, b)
                .as_deref()
                .map(|s| s.lines().next().unwrap().to_string())
                .as_deref(),
            Some("Riverside Loop and 2 other lines stop here. Demolish and drop the stop?")
        );
    }

    #[test]
    fn the_demolish_confirm_says_which_line_would_have_nowhere_to_run() {
        let (a, b) = (StationId(1), StationId(2));
        let mut lines = LineRegistry::new();
        lines.create("Riverside Loop".into(), vec![a, b]).unwrap();

        let text = demolish_consequence(&lines, b).expect("a consequence");
        assert_eq!(
            text.lines().nth(1),
            Some("Riverside Loop has nowhere left to run.")
        );

        lines.create("Quarry Run".into(), vec![a, b]).unwrap();
        let text = demolish_consequence(&lines, b).expect("a consequence");
        assert_eq!(text.lines().nth(1), Some("2 lines have nowhere left to run."));

        // A line with somewhere left to go says nothing extra.
        let mut lines = LineRegistry::new();
        lines
            .create("Riverside Loop".into(), vec![a, b, StationId(3)])
            .unwrap();
        let text = demolish_consequence(&lines, b).expect("a consequence");
        assert_eq!(text.lines().count(), 1);
    }

    /// The shipped bitmap font has a small charset (`docs/BURNDOWN.md`).
    #[test]
    fn every_station_string_stays_inside_the_font() {
        let (a, b) = (StationId(1), StationId(2));
        let mut lines = LineRegistry::new();
        lines.create("Riverside Loop".into(), vec![a, b]).unwrap();
        let text = demolish_consequence(&lines, b).expect("a consequence");
        assert!(text.is_ascii(), "{text} would draw as tofu");

        for error in [
            StationPlacementError::NoIndustryHere,
            StationPlacementError::NoTrack,
            StationPlacementError::NotAStubEnd,
            StationPlacementError::UnknownStation,
            StationPlacementError::InsufficientFunds,
            StationPlacementError::AlreadyStation,
            StationPlacementError::InvalidLayer,
            StationPlacementError::TooClose { distance: 2, min: 3 },
            StationPlacementError::NoPlatformRoom { have: 1, need: 2 },
            StationPlacementError::NotUpgradable {
                from: StationTier::Terminus,
                to: StationTier::GoodsPlatform,
            },
        ] {
            let reason = station_reason(error, 100, 0);
            assert!(reason.is_ascii(), "{reason} would draw as tofu");
        }
    }

    #[test]
    fn reasons_carry_the_rule_and_its_number() {
        assert_eq!(
            station_reason(
                StationPlacementError::TooClose {
                    distance: 2,
                    min: MIN_STATION_SPACING,
                },
                0,
                0,
            ),
            format!("Too close - 2 tiles, need {MIN_STATION_SPACING}")
        );
        assert_eq!(
            station_reason(
                StationPlacementError::NoPlatformRoom { have: 2, need: 4 },
                0,
                0
            ),
            "Not enough platform - 2 tiles of line, needs 4"
        );
    }
}
