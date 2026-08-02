//! Place / demolish / upgrade stations against [`StationRegistry`] + [`Money`].
//!
//! A station is a kind of track ([04 — Building & Tools] §6), so every rule here
//! is a rule about the line it sits on: there must be track under the tile, and
//! the tier's platforms must fit in one straight contiguous run through it. A
//! terminus additionally needs that run to dead-end — stub platforms cannot be
//! run through.
//!
//! Validation mirrors [`PlacementError`](crate::track::PlacementError): every
//! rejection names its rule, and where the rule has a number it carries both the
//! value and the limit so the player learns it rather than bouncing off it.

use serde::{Deserialize, Serialize};

use crate::economy::{MoneyCategory, MoneyLedger};
use crate::ids::{LineId, StationId, TileCoord};
use crate::money::Money;
use crate::track::{opposite_dir, step, TrackNetwork, GROUND_LAYER};

use super::registry::{Station, StationRegistry};
use super::service::StationService;
use super::tier::{StationTier, MIN_STATION_SPACING};

/// Build a platform on a piece of line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceStation {
    pub tile: TileCoord,
    /// Reserved for tunnels / elevated; ground-only in MVP.
    pub layer: u8,
    pub tier: StationTier,
    /// Optional override; `None` → [`suggest_station_name`].
    pub name: Option<String>,
}

/// Lift a platform, refunding what was spent on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DemolishStation {
    pub station: StationId,
}

/// Retier a stop in place, keeping its id (and any line that stops there).
///
/// Moving up the Halt → Station → Interchange ladder charges the build-cost
/// difference; moving back down refunds it, which is what makes undo exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpgradeStation {
    pub station: StationId,
    pub to: StationTier,
}

/// Why a station action was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StationPlacementError {
    /// Only ground layer (`0`) is editable in MVP.
    InvalidLayer,
    /// Platforms sit on the line — there is no track under this tile.
    NoTrack,
    /// This tile already has a platform.
    AlreadyStation,
    /// Another stop is inside [`MIN_STATION_SPACING`].
    TooClose { distance: i32, min: i32 },
    /// The straight run through this tile is shorter than the tier's platforms.
    NoPlatformRoom { have: u8, need: u8 },
    /// A terminus needs a stub end; every run through this tile carries on.
    NotAStubEnd,
    InsufficientFunds,
    /// Demolish / upgrade target missing.
    UnknownStation,
    /// The stop is a scheduled call on a line — clear the line first.
    OnLine { line: LineId },
    /// Requested tier is not reachable from the current one.
    NotUpgradable { from: StationTier, to: StationTier },
}

/// A straight contiguous run of track through a candidate station tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformRun {
    /// [`DIR8`](crate::track::DIR8) index of the run's forward direction (`0..4`).
    pub axis: usize,
    /// Track tiles in the run, including the station tile itself.
    pub length: u8,
    /// True when the run dead-ends on at least one side (a terminus stub).
    pub stub: bool,
}

/// Result of a successful place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedStation {
    pub id: StationId,
    pub station: Station,
    /// The run the platforms were laid along.
    pub run: PlatformRun,
}

/// Result of a successful retier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetieredStation {
    pub id: StationId,
    pub from: StationTier,
    pub to: StationTier,
    /// Signed cents moved: positive was charged, negative was refunded.
    pub delta_cents: i64,
}

/// Straight runs of track through `tile`, one per axis (N-S, NE-SW, E-W, SE-NW).
///
/// Empty when `tile` has no track — platforms have nothing to stand on.
pub fn platform_runs(network: &TrackNetwork, tile: TileCoord, layer: u8) -> Vec<PlatformRun> {
    if network.id_at(tile, layer).is_none() {
        return Vec::new();
    }
    (0..4)
        .map(|axis| {
            let forward = walk(network, tile, layer, axis);
            let back = walk(network, tile, layer, opposite_dir(axis));
            PlatformRun {
                axis,
                length: (1 + forward + back).min(u8::MAX as u32) as u8,
                stub: forward == 0 || back == 0,
            }
        })
        .collect()
}

/// Contiguous track tiles from `tile` in one direction, excluding `tile`.
fn walk(network: &TrackNetwork, tile: TileCoord, layer: u8, dir: usize) -> u32 {
    let mut count = 0u32;
    let mut cursor = tile;
    loop {
        cursor = step(cursor, dir);
        if network.id_at(cursor, layer).is_none() {
            return count;
        }
        count += 1;
        // A ring of track would otherwise walk forever.
        if count as usize > network.len() {
            return count;
        }
    }
}

/// Longest run through `tile` that a `tier` could use, if any axis qualifies.
///
/// A terminus only accepts a stub axis; every other tier takes the longest run.
pub fn best_platform_run(
    network: &TrackNetwork,
    tile: TileCoord,
    layer: u8,
    tier: StationTier,
) -> Option<PlatformRun> {
    pick_longest(
        platform_runs(network, tile, layer)
            .into_iter()
            .filter(|run| tier.allows_through_running() || run.stub),
    )
}

/// Longest run through `tile` on any axis, stub or not.
fn longest_run(network: &TrackNetwork, tile: TileCoord, layer: u8) -> Option<PlatformRun> {
    pick_longest(platform_runs(network, tile, layer).into_iter())
}

/// Longest run, breaking ties toward the lower axis index for determinism.
fn pick_longest(runs: impl Iterator<Item = PlatformRun>) -> Option<PlatformRun> {
    runs.max_by_key(|run| (run.length, std::cmp::Reverse(run.axis)))
}

/// Check every rule for putting `tier` on `tile`, ignoring the stop `retier`
/// names (an in-place upgrade is not too close to itself).
///
/// Returns the run the platforms would occupy.
pub fn validate_station_site(
    stations: &StationRegistry,
    network: &TrackNetwork,
    tile: TileCoord,
    layer: u8,
    tier: StationTier,
    retier: Option<StationId>,
) -> Result<PlatformRun, StationPlacementError> {
    if layer != GROUND_LAYER {
        return Err(StationPlacementError::InvalidLayer);
    }
    if network.id_at(tile, layer).is_none() {
        return Err(StationPlacementError::NoTrack);
    }
    match stations.id_at(tile, layer) {
        Some(existing) if Some(existing) != retier => {
            return Err(StationPlacementError::AlreadyStation)
        }
        Some(_) => {}
        None if retier.is_some() => return Err(StationPlacementError::UnknownStation),
        None => {}
    }
    if let Some((_, distance)) = stations.nearest(tile, retier) {
        if distance < MIN_STATION_SPACING {
            return Err(StationPlacementError::TooClose {
                distance,
                min: MIN_STATION_SPACING,
            });
        }
    }

    let need = tier.platforms();
    let usable = best_platform_run(network, tile, layer, tier);
    if let Some(run) = usable {
        if run.length >= need {
            return Ok(run);
        }
    }

    // A terminus that could have fitted on a through run is refused for the
    // stub rule, not for room — say which one actually bit.
    if !tier.allows_through_running() {
        let longest = longest_run(network, tile, layer).map(|r| r.length).unwrap_or(0);
        if longest >= need {
            return Err(StationPlacementError::NotAStubEnd);
        }
    }
    Err(StationPlacementError::NoPlatformRoom {
        have: usable.map(|r| r.length).unwrap_or(0),
        need,
    })
}

/// Try to build one station, debiting [`Money`].
#[allow(clippy::too_many_arguments)]
pub fn try_place_station(
    stations: &mut StationRegistry,
    service: &mut StationService,
    money: &mut Money,
    ledger: &mut MoneyLedger,
    network: &TrackNetwork,
    tile: TileCoord,
    layer: u8,
    tier: StationTier,
    name: Option<String>,
) -> Result<PlacedStation, StationPlacementError> {
    let run = validate_station_site(stations, network, tile, layer, tier, None)?;
    let cost = tier.build_cents();
    ledger
        .try_debit(money, MoneyCategory::Construction, cost)
        .map_err(|_| StationPlacementError::InsufficientFunds)?;

    let name = name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| suggest_station_name(stations, tier));
    let id = stations.insert_tier(name, tile, layer, tier, cost);
    service.ensure(id);
    service.set_tier(id, tier);
    let station = stations.get(id).cloned().expect("just inserted");
    Ok(PlacedStation { id, station, run })
}

/// Lift a station and credit a full refund of `paid_cents`.
///
/// Refuses while a line still calls here — the stop cannot vanish from under a
/// schedule. `line_using` reports the first line containing the stop, if any
/// (see [`LineRegistry::iter`](crate::lines::LineRegistry::iter)).
pub fn try_demolish_station(
    stations: &mut StationRegistry,
    service: &mut StationService,
    money: &mut Money,
    ledger: &mut MoneyLedger,
    station: StationId,
    line_using: impl Fn(StationId) -> Option<LineId>,
) -> Result<Station, StationPlacementError> {
    if stations.get(station).is_none() {
        return Err(StationPlacementError::UnknownStation);
    }
    if let Some(line) = line_using(station) {
        return Err(StationPlacementError::OnLine { line });
    }
    let removed = stations
        .remove(station)
        .ok_or(StationPlacementError::UnknownStation)?;
    ledger.credit(money, MoneyCategory::Construction, removed.paid_cents);
    service.forget(station);
    Ok(removed)
}

/// Retier a stop in place along the Halt → Station → Interchange ladder.
///
/// Moving up debits the build-cost difference; moving down credits it. The
/// terminus is off the ladder in both directions — build one instead.
pub fn try_upgrade_station(
    stations: &mut StationRegistry,
    service: &mut StationService,
    money: &mut Money,
    ledger: &mut MoneyLedger,
    network: &TrackNetwork,
    station: StationId,
    to: StationTier,
) -> Result<RetieredStation, StationPlacementError> {
    let current = stations
        .get(station)
        .ok_or(StationPlacementError::UnknownStation)?;
    let from = current.tier;
    let (tile, layer) = (current.tile, current.layer);

    if from == to || !from.on_ladder_with(to) {
        return Err(StationPlacementError::NotUpgradable { from, to });
    }
    validate_station_site(stations, network, tile, layer, to, Some(station))?;

    let delta = from.retier_cents(to);
    if delta > 0 {
        ledger
            .try_debit(money, MoneyCategory::Construction, delta)
            .map_err(|_| StationPlacementError::InsufficientFunds)?;
    } else if delta < 0 {
        ledger.credit(money, MoneyCategory::Construction, -delta);
    }

    let paid = stations
        .get(station)
        .map(|s| s.paid_cents.saturating_add(delta))
        .unwrap_or(to.build_cents());
    stations.set_tier(station, to, paid);
    service.set_tier(station, to);
    Ok(RetieredStation {
        id: station,
        from,
        to,
        delta_cents: delta,
    })
}

/// Railway-style names for player-built stops, suffixed by tier.
const PLATFORM_NAMES: &[&str] = &[
    "Ashford",
    "Brackwell",
    "Coldharbour",
    "Dunmoor",
    "Elmsworth",
    "Farrow",
    "Greyfell",
    "Hollowgate",
    "Ironbridge",
    "Kestrel",
    "Larkspur",
    "Marchpool",
];

/// First unused name from the pool, suffixed with the tier where it reads
/// naturally ("Ashford Halt", "Brackwell Interchange", plain "Coldharbour").
pub fn suggest_station_name(stations: &StationRegistry, tier: StationTier) -> String {
    let suffix = match tier {
        StationTier::Station => "",
        other => other.label(),
    };
    let decorate = |base: &str| {
        if suffix.is_empty() {
            base.to_string()
        } else {
            format!("{base} {suffix}")
        }
    };

    for base in PLATFORM_NAMES {
        let candidate = decorate(base);
        if !stations.iter().any(|s| s.name == candidate) {
            return candidate;
        }
    }
    // Pool exhausted — number the overflow rather than colliding.
    let mut n = 2usize;
    loop {
        let candidate = format!("{} {n}", decorate(PLATFORM_NAMES[0]));
        if !stations.iter().any(|s| s.name == candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::{try_place_track, TrackTerrain};

    fn land(w: u32, h: u32) -> TrackTerrain {
        TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 0i8)))
    }

    /// Lay a horizontal run of `len` tiles starting at `(x0, y)`.
    fn lay_run(network: &mut TrackNetwork, terrain: &TrackTerrain, x0: i32, y: i32, len: i32) {
        let mut money = Money::new(10_000_000);
        let mut ledger = MoneyLedger::default();
        for i in 0..len {
            try_place_track(
                network,
                &mut money,
                &mut ledger,
                terrain,
                TileCoord { x: x0 + i, y },
                GROUND_LAYER,
            )
            .expect("track");
        }
    }

    fn no_lines(_: StationId) -> Option<LineId> {
        None
    }

    struct World {
        stations: StationRegistry,
        service: StationService,
        money: Money,
        ledger: MoneyLedger,
        network: TrackNetwork,
    }

    fn world(track_len: i32) -> World {
        let terrain = land(32, 32);
        let mut network = TrackNetwork::new();
        lay_run(&mut network, &terrain, 4, 8, track_len);
        World {
            stations: StationRegistry::new(),
            service: StationService::default(),
            money: Money::new(1_000_000),
            ledger: MoneyLedger::default(),
            network,
        }
    }

    fn place(w: &mut World, x: i32, tier: StationTier) -> Result<PlacedStation, StationPlacementError> {
        try_place_station(
            &mut w.stations,
            &mut w.service,
            &mut w.money,
            &mut w.ledger,
            &w.network,
            TileCoord { x, y: 8 },
            GROUND_LAYER,
            tier,
            None,
        )
    }

    #[test]
    fn place_debits_and_registers_the_tier() {
        let mut w = world(8);
        let start = w.money.cents();

        let placed = place(&mut w, 6, StationTier::Station).expect("place");
        assert_eq!(
            w.money.cents(),
            start - StationTier::Station.build_cents()
        );
        assert_eq!(placed.station.tier, StationTier::Station);
        assert_eq!(placed.station.paid_cents, StationTier::Station.build_cents());
        assert_eq!(w.stations.len(), 1);
        assert_eq!(w.service.tier(placed.id), StationTier::Station);
        assert_eq!(placed.station.name, "Ashford");
        // Platforms lie along the line they were built on.
        assert_eq!(placed.run.axis, 2, "east-west run");
        assert!(placed.run.length >= StationTier::Station.platforms());
    }

    #[test]
    fn stations_must_stand_on_track() {
        let mut w = world(8);
        let err = try_place_station(
            &mut w.stations,
            &mut w.service,
            &mut w.money,
            &mut w.ledger,
            &w.network,
            TileCoord { x: 20, y: 20 },
            GROUND_LAYER,
            StationTier::Halt,
            None,
        )
        .unwrap_err();
        assert_eq!(err, StationPlacementError::NoTrack);
        assert_eq!(w.money.cents(), 1_000_000, "refused build must not charge");
    }

    #[test]
    fn only_the_ground_layer_takes_platforms() {
        let mut w = world(8);
        let err = try_place_station(
            &mut w.stations,
            &mut w.service,
            &mut w.money,
            &mut w.ledger,
            &w.network,
            TileCoord { x: 6, y: 8 },
            1,
            StationTier::Halt,
            None,
        )
        .unwrap_err();
        assert_eq!(err, StationPlacementError::InvalidLayer);
    }

    #[test]
    fn a_second_platform_too_near_names_the_distance() {
        let mut w = world(12);
        place(&mut w, 6, StationTier::Halt).expect("first");

        let err = place(&mut w, 8, StationTier::Halt).unwrap_err();
        assert_eq!(
            err,
            StationPlacementError::TooClose {
                distance: 2,
                min: MIN_STATION_SPACING,
            }
        );
        // Exactly the limit away is fine.
        place(&mut w, 9, StationTier::Halt).expect("spaced");
        assert_eq!(w.stations.len(), 2);
    }

    #[test]
    fn same_tile_twice_is_already_a_station() {
        let mut w = world(8);
        place(&mut w, 6, StationTier::Halt).expect("first");
        assert_eq!(
            place(&mut w, 6, StationTier::Halt).unwrap_err(),
            StationPlacementError::AlreadyStation
        );
    }

    #[test]
    fn platforms_must_fit_the_run_and_name_both_numbers() {
        // Two tiles of track: a halt fits, an interchange does not.
        let mut w = world(2);
        assert_eq!(
            place(&mut w, 5, StationTier::Interchange).unwrap_err(),
            StationPlacementError::NoPlatformRoom {
                have: 2,
                need: StationTier::Interchange.platforms(),
            }
        );
        place(&mut w, 5, StationTier::Halt).expect("halt fits");
    }

    #[test]
    fn a_terminus_needs_a_dead_end() {
        let mut w = world(9);
        // Mid-run: the line carries on both ways, so no stub exists.
        assert_eq!(
            place(&mut w, 8, StationTier::Terminus).unwrap_err(),
            StationPlacementError::NotAStubEnd
        );
        // The buffer stop at the end of the run takes one.
        let placed = place(&mut w, 12, StationTier::Terminus).expect("terminus at the end");
        assert!(placed.run.stub);
        assert_eq!(placed.station.name, "Ashford Terminus");
    }

    #[test]
    fn short_funds_refuse_without_charging() {
        let mut w = world(8);
        w.money = Money::new(StationTier::Station.build_cents() - 1);
        assert_eq!(
            place(&mut w, 6, StationTier::Station).unwrap_err(),
            StationPlacementError::InsufficientFunds
        );
        assert_eq!(w.money.cents(), StationTier::Station.build_cents() - 1);
        assert!(w.stations.is_empty());
    }

    #[test]
    fn upgrade_in_place_keeps_the_id_and_charges_the_difference() {
        let mut w = world(8);
        let placed = place(&mut w, 6, StationTier::Halt).expect("halt");
        let after_place = w.money.cents();

        let retier = try_upgrade_station(
            &mut w.stations,
            &mut w.service,
            &mut w.money,
            &mut w.ledger,
            &w.network,
            placed.id,
            StationTier::Station,
        )
        .expect("upgrade");

        assert_eq!(retier.id, placed.id, "upgrading keeps the stop's identity");
        assert_eq!(retier.from, StationTier::Halt);
        assert_eq!(retier.to, StationTier::Station);
        assert_eq!(
            retier.delta_cents,
            StationTier::Station.build_cents() - StationTier::Halt.build_cents()
        );
        assert_eq!(w.money.cents(), after_place - retier.delta_cents);
        let station = w.stations.get(placed.id).expect("still there");
        assert_eq!(station.tier, StationTier::Station);
        assert_eq!(station.paid_cents, StationTier::Station.build_cents());
        assert_eq!(w.service.tier(placed.id), StationTier::Station);
    }

    #[test]
    fn upgrade_is_refused_when_the_platforms_would_not_fit() {
        // Three tiles of track: a station fits, an interchange never will.
        let mut w = world(3);
        let placed = place(&mut w, 5, StationTier::Halt).expect("halt");
        let before = w.money.cents();
        assert_eq!(
            try_upgrade_station(
                &mut w.stations,
                &mut w.service,
                &mut w.money,
                &mut w.ledger,
                &w.network,
                placed.id,
                StationTier::Interchange,
            )
            .unwrap_err(),
            StationPlacementError::NoPlatformRoom {
                have: 3,
                need: StationTier::Interchange.platforms(),
            }
        );
        assert_eq!(w.money.cents(), before);
        assert_eq!(
            w.stations.get(placed.id).expect("unchanged").tier,
            StationTier::Halt
        );
    }

    #[test]
    fn the_terminus_is_off_the_upgrade_ladder() {
        let mut w = world(9);
        let placed = place(&mut w, 12, StationTier::Terminus).expect("terminus");
        assert_eq!(
            try_upgrade_station(
                &mut w.stations,
                &mut w.service,
                &mut w.money,
                &mut w.ledger,
                &w.network,
                placed.id,
                StationTier::Interchange,
            )
            .unwrap_err(),
            StationPlacementError::NotUpgradable {
                from: StationTier::Terminus,
                to: StationTier::Interchange,
            }
        );
    }

    #[test]
    fn retier_round_trip_returns_the_balance_exactly() {
        let mut w = world(8);
        let placed = place(&mut w, 6, StationTier::Halt).expect("halt");
        let before = w.money.cents();

        for to in [StationTier::Interchange, StationTier::Halt] {
            try_upgrade_station(
                &mut w.stations,
                &mut w.service,
                &mut w.money,
                &mut w.ledger,
                &w.network,
                placed.id,
                to,
            )
            .expect("retier");
        }

        assert_eq!(w.money.cents(), before, "undoing an upgrade must be exact");
        assert_eq!(
            w.stations.get(placed.id).expect("still there").paid_cents,
            StationTier::Halt.build_cents()
        );
    }

    #[test]
    fn demolish_refunds_in_full_and_forgets_the_score() {
        let mut w = world(8);
        let before = w.money.cents();
        let placed = place(&mut w, 6, StationTier::Interchange).expect("interchange");
        w.service.record_arrival(placed.id);

        let removed = try_demolish_station(
            &mut w.stations,
            &mut w.service,
            &mut w.money,
            &mut w.ledger,
            placed.id,
            no_lines,
        )
        .expect("demolish");

        assert_eq!(removed.tier, StationTier::Interchange);
        assert_eq!(w.money.cents(), before, "demolish refunds in full");
        assert!(w.stations.is_empty());
        assert_eq!(w.service.score(placed.id).deliveries, 0);
    }

    #[test]
    fn a_stop_on_a_line_cannot_be_lifted_from_under_it() {
        let mut w = world(8);
        let placed = place(&mut w, 6, StationTier::Halt).expect("halt");
        let err = try_demolish_station(
            &mut w.stations,
            &mut w.service,
            &mut w.money,
            &mut w.ledger,
            placed.id,
            |_| Some(LineId(3)),
        )
        .unwrap_err();
        assert_eq!(err, StationPlacementError::OnLine { line: LineId(3) });
        assert_eq!(w.stations.len(), 1, "refused demolish must not remove");
    }

    #[test]
    fn demolishing_nothing_is_an_unknown_station() {
        let mut w = world(4);
        assert_eq!(
            try_demolish_station(
                &mut w.stations,
                &mut w.service,
                &mut w.money,
                &mut w.ledger,
                StationId(99),
                no_lines,
            )
            .unwrap_err(),
            StationPlacementError::UnknownStation
        );
    }

    #[test]
    fn suggested_names_do_not_collide() {
        let mut w = world(24);
        let a = place(&mut w, 5, StationTier::Halt).expect("a");
        let b = place(&mut w, 10, StationTier::Halt).expect("b");
        assert_eq!(a.station.name, "Ashford Halt");
        assert_eq!(b.station.name, "Brackwell Halt");
        // A different tier reuses the base name with its own suffix.
        let c = place(&mut w, 15, StationTier::Station).expect("c");
        assert_eq!(c.station.name, "Ashford");
    }
}
