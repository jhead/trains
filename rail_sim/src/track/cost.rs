//! Track construction costs (cents), maintenance, and grade limits.
//!
//! Relative construction costs follow [02 — World & Terrain] §3.1
//! (order-of-magnitude spread). Maintenance is the overextension lever in
//! [08 — Economy & Pressure] §3.1.

use crate::ids::TileCoord;

use super::rules::PlacementError;
use super::terrain::TrackTerrain;

/// Base cost for flat plains / along-contour land track: $10.00 = **1×**.
pub const TRACK_COST_CENTS: i64 = 10_000;

/// Minimum bridge cost (span 1): **8×** base — $80.00.
pub const BRIDGE_COST_CENTS: i64 = TRACK_COST_CENTS * 8;

/// Maximum contiguous water tiles a bridge may span (inclusive).
///
/// A river the player cannot cross at all is a wall, and a wall is not a
/// decision — so every trunk a generator draws is reachable, at a price, and so
/// is the wide water between the narrows. Eight tiles is where a deck stops
/// being a bridge and starts being a causeway; past it the answer is an honest
/// refusal that names the span it measured.
pub const MAX_BRIDGE_SPAN: u32 = 8;

/// Widest span still on the cheap rungs of [`bridge_cost_for_span`] (8–20×).
///
/// Above this a crossing is a *premium* one (30× and up) — buildable, but a
/// different decision entirely. Anything asking "can the player afford to cross
/// here in minute one" means this constant, not [`MAX_BRIDGE_SPAN`]: the map
/// generator authors its narrows against it, so the cheap crossing stays the
/// scarce, interesting one.
pub const CHEAP_BRIDGE_SPAN: u32 = 3;

/// Hard max absolute height delta between adjacent track tiles.
/// Above this, placement is refused ([02] §3.2).
pub const MAX_GRADE: u8 = 4;

/// Land tiles at this height or above are impassable cliff / high peak.
///
/// Generator `TerrainKind::Mountain` starts at height 11; the lower band is
/// still buildable (expensive). True refusal starts here so seeded towns on
/// low mountains remain reachable.
pub const MOUNTAIN_HEIGHT_MIN: i8 = 14;

/// Ground-layer index used by MVP placement commands (`PlaceTrack.layer`).
pub const GROUND_LAYER: u8 = 0;

/// Maintenance **weight** of one ground track tile.
///
/// Not money. [`MAINT_CENTS_PER_WEIGHT_PER_REAL_MIN`](crate::economy::opex::MAINT_CENTS_PER_WEIGHT_PER_REAL_MIN)
/// prices a weight; this says how many units a piece is worth. It was called
/// `TRACK_MAINT_CENTS` and read like a per-tick charge, which is two wrong units
/// in one name.
pub const TRACK_MAINT_WEIGHT: i64 = 1;

/// Maintenance weight of one bridge tile — 4× ground, per [02] §3.1's
/// "bridges and tunnels cost several times more".
pub const BRIDGE_MAINT_WEIGHT: i64 = 4;

/// Optional soft curve refuse — turns sharper than 90° (`curve > 64`).
/// Autofill is straight so this mainly bites at junctions; curves still slow below.
pub const MAX_CURVE: u8 = 64;

/// Bridge construction cost for a water span (tiles), **8–90×** base.
///
/// | Span | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 |
/// | --- | --- | --- | --- | --- | --- | --- | --- | --- |
/// | Per tile | 8× | 14× | 20× | 30× | 42× | 56× | 72× | 90× |
///
/// The first three rungs are the cheap tier ([`CHEAP_BRIDGE_SPAN`]): a ford, a
/// stream, a small river, all of them things a young railway crosses without
/// much thought. The ladder then steepens quadratically, because a deck that
/// stands eight tiles clear of both banks is a different structure and should
/// read as one — the per-tile rate rises *and* there are more tiles to pay it
/// on, so a full eight-span crossing is 720× base: a monument the railway saves
/// up for, not a shortcut it takes.
#[inline]
pub fn bridge_cost_for_span(span: u32) -> i64 {
    let mult = match span {
        0 | 1 => 8,
        2 => 14,
        3 => 20,
        4 => 30,
        5 => 42,
        6 => 56,
        7 => 72,
        _ => 90, // span 8 (and any wider that somehow passed span checks)
    };
    TRACK_COST_CENTS.saturating_mul(mult)
}

/// Local terrain slope: max ortho |Δh| to neighboring land/water cells.
pub fn local_slope(terrain: &TrackTerrain, tile: TileCoord) -> u8 {
    let Some(h) = terrain.height_at(tile) else {
        return 0;
    };
    let mut max_dh = 0u8;
    for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
        let n = TileCoord {
            x: tile.x + dx,
            y: tile.y + dy,
        };
        if let Some(nh) = terrain.height_at(n) {
            let dh = (nh as i16 - h as i16).unsigned_abs() as u8;
            max_dh = max_dh.max(dh);
        }
    }
    max_dh
}

/// Terrain-relative construction cost for one tile (does not check funds / occupancy).
///
/// | Terrain | Relative |
/// | --- | --- |
/// | Flat plains (h≤5, slope≤1) | 1× |
/// | Gentle slope (h≤5, slope 2–3) | 1.5× |
/// | Hills (h≤10, slope≤3) | 3× |
/// | Steep hillside (h≤10, slope≥4) | 6× |
/// | High mountain band (h 11..=13) | 10× |
/// | Bridge by span | 8 / 14 / 20 / 30 / 42 / 56 / 72 / 90× |
/// | Cliff / high peak (h≥14) | refused |
///
/// The slope bands are cut where the generator's elevation bands actually
/// land. [02] §2.3 requires every height step to be a drawn edge, so
/// `rail_map` steps its bands by 3 (a bank) or 4 (`MAX_GRADE`, a cliff face)
/// and never by 1 or 2. A gentle band that stopped at slope 2 could therefore
/// never fire on a generated map: Δ3 is the gentle slope, and Δ4 is the
/// cut-and-fill.
pub fn tile_build_cost(terrain: &TrackTerrain, tile: TileCoord) -> Result<i64, PlacementError> {
    if !terrain.contains(tile) {
        return Err(PlacementError::OutOfBounds);
    }
    if terrain.is_water(tile) {
        // Crossing span = shorter contiguous water run (how far to dry land).
        let span = terrain
            .water_span_horizontal(tile)
            .min(terrain.water_span_vertical(tile));
        return Ok(bridge_cost_for_span(span));
    }
    let height = terrain.height_at(tile).unwrap_or(0);
    if height >= MOUNTAIN_HEIGHT_MIN {
        return Err(PlacementError::TerrainForbidden);
    }
    let slope = local_slope(terrain, tile);
    // milli-multiples of base (1000 = 1×) so 1.5× stays exact.
    let millis = match (height, slope) {
        (h, s) if h <= 5 && s <= 1 => 1_000,
        (h, s) if h <= 5 && s <= 3 => 1_500,
        (h, s) if h <= 10 && s <= 3 => 3_000,
        (h, _) if h <= 10 => 6_000,
        (h, _) if h < MOUNTAIN_HEIGHT_MIN => 10_000, // high mountain band
        _ => return Err(PlacementError::TerrainForbidden),
    };
    Ok((TRACK_COST_CENTS.saturating_mul(millis)) / 1_000)
}

/// Cost for a single tile given whether it needs a bridge (legacy helper).
///
/// Prefer [`tile_build_cost`] — this assumes flat land / span-1 bridge and
/// ignores height/slope.
#[inline]
pub fn tile_cost(is_bridge: bool) -> i64 {
    if is_bridge {
        BRIDGE_COST_CENTS
    } else {
        TRACK_COST_CENTS
    }
}

/// Maintenance weight of one laid piece (not money — see [`TRACK_MAINT_WEIGHT`]).
#[inline]
pub fn piece_maintenance_weight(is_bridge: bool) -> i64 {
    if is_bridge {
        BRIDGE_MAINT_WEIGHT
    } else {
        TRACK_MAINT_WEIGHT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terrain(cells: &[(bool, i8)], w: u32, h: u32) -> TrackTerrain {
        assert_eq!(cells.len(), (w * h) as usize);
        TrackTerrain::new(w, h, cells.iter().copied())
    }

    #[test]
    fn cost_spread_is_order_of_magnitude() {
        // 3×3 flat plains — centre is 1×.
        let flat = terrain(&[(false, 2); 9], 3, 3);
        let flat_c = tile_build_cost(&flat, TileCoord { x: 1, y: 1 }).unwrap();
        assert_eq!(flat_c, TRACK_COST_CENTS);

        // Gentle: plains height stepping a drawn bank (Δ3) up to the hills band.
        let mut gentle_cells = vec![(false, 4i8); 9];
        gentle_cells[1] = (false, 7); // north of centre → Δh=3
        let gentle = terrain(&gentle_cells, 3, 3);
        let gentle_c = tile_build_cost(&gentle, TileCoord { x: 1, y: 1 }).unwrap();
        assert_eq!(gentle_c, TRACK_COST_CENTS * 3 / 2);

        // Hills plateau.
        let hills = terrain(&[(false, 8); 9], 3, 3);
        let hills_c = tile_build_cost(&hills, TileCoord { x: 1, y: 1 }).unwrap();
        assert_eq!(hills_c, TRACK_COST_CENTS * 3);

        // Hills crossing a bank stay at the hills rate.
        let mut banked_cells = vec![(false, 7i8); 9];
        banked_cells[1] = (false, 10); // Δh=3
        let banked = terrain(&banked_cells, 3, 3);
        let banked_c = tile_build_cost(&banked, TileCoord { x: 1, y: 1 }).unwrap();
        assert_eq!(banked_c, TRACK_COST_CENTS * 3);

        // Steep hillside: a full cliff face, Δ4 = MAX_GRADE, is the cut-and-fill.
        let mut steep_cells = vec![(false, 8i8); 9];
        steep_cells[1] = (false, 12); // Δh=4; neighbour is still below the wall
        let steep = terrain(&steep_cells, 3, 3);
        let steep_c = tile_build_cost(&steep, TileCoord { x: 1, y: 1 }).unwrap();
        assert_eq!(steep_c, TRACK_COST_CENTS * 6);

        // The step up out of a shore apron is Δ4 as well, so it stays cut-and-fill.
        let mut shore_cells = vec![(false, 0i8); 9];
        shore_cells[1] = (false, 4); // Δh=4 from band 0 to band 1
        let shore = terrain(&shore_cells, 3, 3);
        let shore_c = tile_build_cost(&shore, TileCoord { x: 1, y: 1 }).unwrap();
        assert_eq!(shore_c, TRACK_COST_CENTS * 6);

        // High mountain band.
        let alpine = terrain(&[(false, 13); 9], 3, 3);
        let alpine_c = tile_build_cost(&alpine, TileCoord { x: 1, y: 1 }).unwrap();
        assert_eq!(alpine_c, TRACK_COST_CENTS * 10);

        // Bridge: 3×3 water pocket → min axis span 3 → 20×.
        let bridge3 = TrackTerrain::new(
            5,
            3,
            (0..3).flat_map(|_y| {
                (0..5).map(move |x| {
                    let water = x >= 1 && x <= 3;
                    (water, if water { -2 } else { 1 })
                })
            }),
        );
        assert_eq!(
            tile_build_cost(&bridge3, TileCoord { x: 2, y: 1 }).unwrap(),
            TRACK_COST_CENTS * 20
        );

        // Span-1 bridge is 8×.
        let bridge1 = TrackTerrain::new(3, 1, [(false, 1), (true, -2), (false, 1)]);
        assert_eq!(
            tile_build_cost(&bridge1, TileCoord { x: 1, y: 0 }).unwrap(),
            TRACK_COST_CENTS * 8
        );

        let cheap = flat_c;
        let pricey = TRACK_COST_CENTS * 20;
        assert!(
            pricey / cheap >= 10,
            "expected ≥10x spread, got {} / {} = {}",
            pricey,
            cheap,
            pricey / cheap
        );
    }

    /// The ladder the player is being asked to weigh: every rung dearer than
    /// the last, and the premium rungs (4–6) dearer than the cheap tier by
    /// enough that a wide crossing is a decision rather than a detail.
    #[test]
    fn the_bridge_ladder_climbs_all_the_way_to_the_span_limit() {
        let rungs: Vec<i64> = (1..=MAX_BRIDGE_SPAN).map(bridge_cost_for_span).collect();
        assert_eq!(
            rungs,
            vec![
                TRACK_COST_CENTS * 8,
                TRACK_COST_CENTS * 14,
                TRACK_COST_CENTS * 20,
                TRACK_COST_CENTS * 30,
                TRACK_COST_CENTS * 42,
                TRACK_COST_CENTS * 56,
                TRACK_COST_CENTS * 72,
                TRACK_COST_CENTS * 90,
            ]
        );
        for pair in rungs.windows(2) {
            assert!(pair[1] > pair[0], "the ladder flattens: {rungs:?}");
        }
        // Span 0 is the degenerate "not really water" case and prices as span 1.
        assert_eq!(bridge_cost_for_span(0), bridge_cost_for_span(1));
        // The cheap tier is the top of the routine rungs, not of the ladder:
        // there are premium rungs above it to climb.
        assert!(
            rungs.len() > CHEAP_BRIDGE_SPAN as usize,
            "the cheap tier swallowed the whole ladder"
        );
        assert!(
            bridge_cost_for_span(CHEAP_BRIDGE_SPAN + 1)
                >= bridge_cost_for_span(CHEAP_BRIDGE_SPAN) * 3 / 2,
            "stepping off the cheap tier should be felt"
        );
    }

    /// A whole eight-span crossing is a monument, not an opening move.
    #[test]
    fn the_widest_crossing_costs_more_than_the_player_starts_with() {
        let whole = bridge_cost_for_span(MAX_BRIDGE_SPAN) * MAX_BRIDGE_SPAN as i64;
        assert_eq!(whole, TRACK_COST_CENTS * 720);
        assert!(
            whole > crate::money::STARTING_CASH_CENTS * 3,
            "{whole} is pocket change against the opening balance"
        );
        // And it is worth several of the cheap crossing it replaces, so
        // scouting for a narrows is never the foolish option.
        let narrows = bridge_cost_for_span(CHEAP_BRIDGE_SPAN) * CHEAP_BRIDGE_SPAN as i64;
        assert!(whole > narrows * 10, "{whole} vs {narrows}");
    }

    /// Span pricing reads the *shorter* axis: how far it is to dry land, not
    /// how long the water is. With every rung priced differently this is no
    /// longer a distinction without a difference.
    #[test]
    fn a_long_narrow_channel_prices_on_the_short_axis() {
        // A 2-wide channel running the full nine rows: cross it, do not swim it.
        let w = 6u32;
        let h = 9u32;
        let channel = TrackTerrain::new(
            w,
            h,
            (0..h).flat_map(move |_y| {
                (0..w).map(move |x| {
                    let water = x == 2 || x == 3;
                    (water, if water { -2 } else { 1 })
                })
            }),
        );
        assert_eq!(
            tile_build_cost(&channel, TileCoord { x: 2, y: 4 }).unwrap(),
            TRACK_COST_CENTS * 14
        );
    }

    #[test]
    fn mountain_refused() {
        let m = terrain(&[(false, 14); 1], 1, 1);
        assert_eq!(
            tile_build_cost(&m, TileCoord { x: 0, y: 0 }),
            Err(PlacementError::TerrainForbidden)
        );
    }
}
