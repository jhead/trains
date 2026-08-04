//! Lot geometry and the density → occupancy/tier mapping.
//!
//! Brief 06 §2: *a tile is not a building.* A tile is a **block** holding up to
//! four **lots**, and density is how many of those lots are occupied and how
//! tall they have grown. That resolution is the whole reason a district can
//! read as a place instead of as a bar chart.
//!
//! Everything here is a pure function of integer world coordinates and the
//! sim's density value, so the same lot always resolves to the same building
//! (pixel contract §2.4 — world-anchored, never screen, never time).

use rail_sim::{GoodKind, TileCoord};

use super::building_art::{world_hash, BuildingKind, Family, Roof};
use super::districts::{roof_for, works_variant, District};

/// Texels along one tile edge — must match [`rail_map::TILE_SIZE`].
pub const TILE_TEXELS: i32 = 32;
/// Texels along one lot edge; four lots tile a block 2 × 2.
pub const LOT_TEXELS: i32 = TILE_TEXELS / 2;
/// Lots in a block.
pub const LOTS_PER_TILE: u8 = 4;
/// Texels one lot of a row is set back from the other, so overlapping
/// neighbours have a definite front and back.
pub const LOT_SETBACK: i32 = 5;

/// Density at which each successive lot is taken up.
///
/// The first step is deliberately above the noise floor: a tile that the sim
/// barely touches is open ground, not a lone house in a field. Combined with
/// the steep distance falloff in `rail_sim`, that is what gives a town an
/// outskirt and then a hard edge.
pub const LOT_UP: [f32; 4] = [0.14, 0.32, 0.56, 0.80];
/// Density at which each lot is given up again — the gap is deliberate
/// hysteresis so a lot never flickers on a jittering density value.
pub const LOT_DOWN: [f32; 4] = [0.09, 0.26, 0.48, 0.72];

/// How many lots this block should hold at `d`, given how many it holds now.
pub fn lots_wanted(d: f32, district: District, current: u8) -> u8 {
    let mut n = 0u8;
    for i in 0..LOTS_PER_TILE as usize {
        let threshold = if current > i as u8 {
            LOT_DOWN[i]
        } else {
            LOT_UP[i]
        };
        if d >= threshold {
            n = i as u8 + 1;
        } else {
            break;
        }
    }
    n.min(district.lot_cap())
}

/// Order in which a block's lots are taken up — hashed so streets fill
/// unevenly rather than always from the same corner.
pub fn fill_order(tile: TileCoord) -> [u8; 4] {
    let mut order = [0u8, 1, 2, 3];
    let h = world_hash(tile.x, tile.y, 0x0F17);
    // Fisher–Yates over four entries, driven by one hash.
    for i in (1..4usize).rev() {
        let j = ((h >> (i * 5)) % (i as u32 + 1)) as usize;
        order.swap(i, j);
    }
    order
}

/// Position of `slot` in this block's fill order (`0` = built first).
pub fn fill_position(tile: TileCoord, slot: u8) -> u8 {
    fill_order(tile)
        .iter()
        .position(|s| *s == slot)
        .unwrap_or(0) as u8
}

/// World-anchored lot coordinate — the hash key for everything about this lot.
pub fn lot_coord(tile: TileCoord, slot: u8) -> (i32, i32) {
    (
        tile.x * 2 + (slot & 1) as i32,
        tile.y * 2 + (slot >> 1) as i32,
    )
}

/// Integer world position of a lot's building base (bottom-centre of its sprite).
///
/// Includes the small hashed offset that keeps a street from looking gridded,
/// plus a **stagger**: one of the two lots in a row is set back from the other.
/// Buildings are wider than their lot now, so two of them side by side overlap;
/// the stagger gives that overlap an unambiguous depth order (further south
/// draws in front, brief 01 §6.1) and reads as an organic street rather than as
/// two sprites fighting over the same row.
///
/// Both components are whole texels — the pixel contract admits nothing else.
pub fn lot_base(tile: TileCoord, slot: u8) -> (i32, i32) {
    let (lx, ly) = lot_coord(tile, slot);
    let origin_x = tile.x * TILE_TEXELS + (slot & 1) as i32 * LOT_TEXELS;
    let origin_y = tile.y * TILE_TEXELS + (slot >> 1) as i32 * LOT_TEXELS;
    let jitter_x = (world_hash(lx, ly, 0x00FF) % 5) as i32 - 2;
    let jitter_y = (world_hash(lx, ly, 0xFF00) % 3) as i32;
    // Which side of the row sits back is hashed per block, so a street does not
    // develop a single repeating sawtooth.
    let lean = (world_hash(tile.x, tile.y, 0x77E1) % 2) as u8;
    let setback = if slot & 1 == lean { LOT_SETBACK } else { 0 };
    (
        origin_x + LOT_TEXELS / 2 + jitter_x,
        origin_y + 1 + jitter_y + setback,
    )
}

/// Hashed orientation. A mirror, never a rotation — resampled pixel art is mush
/// (pixel contract §2.2), but a horizontal flip is exact.
pub fn lot_flip(tile: TileCoord, slot: u8) -> bool {
    let (lx, ly) = lot_coord(tile, slot);
    world_hash(lx, ly, 0x5F1D).is_multiple_of(2)
}

/// Growth score for one lot: density, district character, and where this lot
/// sits in the block's build order, plus a hashed nudge.
pub fn growth_score(d: f32, tile: TileCoord, slot: u8, district: District) -> f32 {
    let (lx, ly) = lot_coord(tile, slot);
    let jitter = (world_hash(lx, ly, 0x7A11) % 100) as f32 / 100.0 * 0.7 - 0.35;
    let order = fill_position(tile, slot) as f32;
    d * 4.0 + district.tier_bias() - order * 0.45 + jitter
}

/// Tier `0..=3` from a growth score, capped by what the district supports.
pub fn tier_from_score(score: f32, district: District) -> u8 {
    let tier = if score < 1.0 {
        0
    } else if score < 2.0 {
        1
    } else if score < 3.0 {
        2
    } else {
        3
    };
    tier.min(district.tier_cap())
}

/// Resolve the building a lot should hold at this density.
pub fn plan_kind(
    tile: TileCoord,
    slot: u8,
    d: f32,
    district: District,
    good: Option<GoodKind>,
) -> BuildingKind {
    let (lx, ly) = lot_coord(tile, slot);
    let tier = tier_from_score(growth_score(d, tile, slot, district), district);
    match district.family() {
        Family::Works => BuildingKind {
            family: Family::Works,
            tier,
            variant: works_variant(lx, ly, good),
            roof: roof_for(lx, ly, district),
        },
        Family::Town => BuildingKind {
            family: Family::Town,
            tier,
            variant: (world_hash(lx, ly, 0x1234) % 4) as u8,
            roof: roof_for(lx, ly, district),
        },
    }
}

// ─ Countryside ─────────────────────────────────────────
//
// Unserved land must look *deliberately* rural (brief 06 §2.2) — and that means
// mostly **empty**. A prop on every other tile is not countryside, it is
// confetti: it carpets the map, it competes with the town for attention, and it
// destroys the contrast that makes a served district read as a consequence.
//
// So the countryside is built out of a few farms with their own fields around
// them, and open land everywhere else. One coarse cell holds at most one farm.

/// Side of the coarse cell that holds at most one farm (tiles).
pub const FARM_CELL: i32 = 8;
/// Chance in 100 that a coarse cell has a farm at all.
const FARM_CELL_CHANCE: u32 = 55;
/// Chance in 100 that an otherwise-open tile carries a lone landmark.
const LANDMARK_CHANCE: u32 = 1;

/// The farm anchor for the coarse cell containing `tile`, when it has one.
///
/// Pure function of the cell's integer coordinates, so the countryside is the
/// same every run and does not move when the camera does.
pub fn farm_anchor(tile: TileCoord) -> Option<TileCoord> {
    let cx = tile.x.div_euclid(FARM_CELL);
    let cy = tile.y.div_euclid(FARM_CELL);
    let h = world_hash(cx, cy, 0xFA47);
    if h % 100 >= FARM_CELL_CHANCE {
        return None;
    }
    let span = (FARM_CELL - 2).max(1) as u32;
    Some(TileCoord {
        x: cx * FARM_CELL + 1 + ((h >> 8) % span) as i32,
        y: cy * FARM_CELL + 1 + ((h >> 16) % span) as i32,
    })
}

/// Chebyshev distance in tiles.
#[inline]
fn cheb(a: TileCoord, b: TileCoord) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

/// Rural props on land the railway has not reached.
///
/// Returns the prop index within [`super::building_art::FRAME_RURAL`], or
/// `None` for open ground — which is most of the map, on purpose. Fields and
/// walls only appear next to a farm, so the countryside reads as worked land
/// around a farmstead rather than as objects sprinkled over terrain. Away from
/// any farm a tile very occasionally carries a single landmark.
pub fn rural_prop(tile: TileCoord) -> Option<usize> {
    if let Some(anchor) = farm_anchor(tile) {
        if anchor == tile {
            // The farmstead itself stands here.
            return None;
        }
        if cheb(anchor, tile) <= 1 {
            // Roughly half of a farm's eight neighbours are worked ground.
            return match world_hash(tile.x, tile.y, 0xC0DE) % 100 {
                0..=17 => Some(0),  // ploughed field
                18..=27 => Some(1), // haystack
                28..=35 => Some(2), // hedgerow
                36..=40 => Some(3), // lane and gate
                41..=45 => Some(4), // dry-stone wall
                _ => None,
            };
        }
    }
    // Open country: a lone tree every great while, and nothing else.
    if world_hash(tile.x, tile.y, 0x7EE5) % 100 < LANDMARK_CHANCE {
        return Some(5);
    }
    None
}

/// Which lot a rural prop sits on, so props keep the same 16-texel rhythm.
pub fn rural_slot(tile: TileCoord) -> u8 {
    (world_hash(tile.x, tile.y, 0xBEEF) % 4) as u8
}

/// The farmstead standing on this tile, when it is a farm's anchor.
pub fn rural_farmstead(tile: TileCoord) -> Option<BuildingKind> {
    if farm_anchor(tile) != Some(tile) {
        return None;
    }
    let h = world_hash(tile.x, tile.y, 0x0FA5) % 100;
    let (lx, ly) = lot_coord(tile, rural_slot(tile));
    Some(BuildingKind {
        // A farm is a working building, and it should read as one.
        family: if h < 55 { Family::Town } else { Family::Works },
        tier: 0,
        variant: (world_hash(lx, ly, 0x1234) % 4) as u8,
        roof: if h.is_multiple_of(2) {
            Roof::Tile
        } else {
            Roof::Slate
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_map::TILE_SIZE;

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    #[test]
    fn lot_grid_matches_the_map_tile_size() {
        assert_eq!(TILE_TEXELS as f32, TILE_SIZE);
        assert_eq!(LOT_TEXELS * 2, TILE_TEXELS);
    }

    /// **The pacing claim, end to end.** Brief 17 §5.
    ///
    /// The sim owns a *rate* and this module owns the *thresholds*, and neither
    /// half means anything on its own: a growth rate is only "over a few days"
    /// if the density it produces crosses the numbers that put a building on a
    /// lot. This is the one place both halves are in scope at once.
    ///
    /// The block modelled is the heart of a fully served town — target density
    /// `1.0` — which is the fastest any block in the game grows. Everything
    /// further out is slower, because `town_falloff` caps its target.
    #[test]
    fn a_block_fills_over_days_and_the_first_house_lands_promptly() {
        use rail_sim::{GROWTH_APPROACH_RATE, GROWTH_PASSES_PER_DAY};

        /// Sim days a block at target `1.0` needs to reach `d`, at the sim's
        /// own rate — the closed form of "close `GROWTH_APPROACH_RATE` of the gap,
        /// `GROWTH_PASSES_PER_DAY` times a day".
        fn days_to(d: f32) -> f32 {
            let per_day = (1.0 - GROWTH_APPROACH_RATE).powi(GROWTH_PASSES_PER_DAY as i32);
            (1.0 - d).ln() / per_day.ln()
        }

        let days: Vec<f32> = LOT_UP.iter().map(|d| days_to(*d)).collect();

        // A stake goes in inside the first sim day, so the player can connect
        // the building to the line they just built (brief 06 §1)…
        assert!(
            (0.3..=1.0).contains(&days[0]),
            "the first lot should be claimed inside the first sim day, not {} \
             days in",
            days[0]
        );
        // …and the block is not finished for the better part of a working week,
        // which is the owner's "over a few days".
        assert!(
            (4.0..=8.0).contains(&days[3]),
            "a full block should take several sim days, not {}",
            days[3]
        );
        // Monotone, and no two lots land on the same day — a block that gains
        // three lots at once is a town that appears rather than grows.
        for pair in days.windows(2) {
            assert!(
                pair[1] > pair[0] + 0.4,
                "lots must arrive separately: {pair:?}"
            );
        }

        // In the minutes the player is actually sitting there (brief 17 §1: a
        // sim day is 2.25 real minutes at 1x).
        let real_minutes = |d: f32| d * 2.25;
        assert!(
            (0.7..=2.5).contains(&real_minutes(days[0])),
            "first house at {} real minutes",
            real_minutes(days[0])
        );
        assert!(
            (9.0..=18.0).contains(&real_minutes(days[3])),
            "full block at {} real minutes",
            real_minutes(days[3])
        );
    }

    #[test]
    fn density_takes_up_lots_one_at_a_time() {
        let d = District::Residential;
        assert_eq!(lots_wanted(0.0, d, 0), 0);
        assert_eq!(lots_wanted(0.2, d, 0), 1);
        assert_eq!(lots_wanted(0.4, d, 0), 2);
        assert_eq!(lots_wanted(0.6, d, 0), 3);
        assert_eq!(lots_wanted(0.95, d, 0), 4);
    }

    #[test]
    fn occupancy_has_hysteresis_so_lots_do_not_flicker() {
        let d = District::Residential;
        // A block already holding three lots keeps them just below the up-step.
        assert_eq!(lots_wanted(0.54, d, 3), 3);
        // A block holding two does not gain a third at the same density.
        assert_eq!(lots_wanted(0.54, d, 2), 2);
    }

    #[test]
    fn rural_blocks_never_hold_more_than_a_single_cottage() {
        assert_eq!(lots_wanted(1.0, District::Rural, 4), 1);
        let kind = plan_kind(tile(3, 4), 0, 1.0, District::Rural, None);
        assert_eq!(kind.tier, 0);
    }

    #[test]
    fn lots_of_a_block_sit_on_a_two_by_two_grid() {
        let t = tile(2, 3);
        let mut seen = Vec::new();
        for slot in 0..4u8 {
            let (x, y) = lot_base(t, slot);
            // Base stays inside the block plus the hashed nudge.
            assert!(x >= t.x * TILE_TEXELS - 2 && x <= (t.x + 1) * TILE_TEXELS + 2);
            assert!(y >= t.y * TILE_TEXELS && y <= (t.y + 1) * TILE_TEXELS);
            seen.push((x, y));
        }
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 4, "lots must not stack on one another");
    }

    #[test]
    fn planning_is_world_anchored_and_repeatable() {
        let a = plan_kind(tile(9, 4), 2, 0.7, District::Residential, None);
        let b = plan_kind(tile(9, 4), 2, 0.7, District::Residential, None);
        assert_eq!(a, b);
        let other = plan_kind(tile(10, 4), 2, 0.7, District::Residential, None);
        assert!(a != other || a.tier == other.tier);
    }

    #[test]
    fn a_street_is_not_one_repeated_house() {
        let mut kinds = std::collections::HashSet::new();
        for x in 0..8 {
            for slot in 0..4u8 {
                kinds.insert(plan_kind(tile(x, 5), slot, 0.6, District::Residential, None));
            }
        }
        assert!(
            kinds.len() >= 8,
            "only {} distinct buildings across a street - that reads as wallpaper",
            kinds.len()
        );
    }

    #[test]
    fn denser_blocks_grow_taller() {
        let low = plan_kind(tile(4, 4), 0, 0.15, District::Residential, None).tier;
        let high = plan_kind(tile(4, 4), 0, 0.95, District::Residential, None).tier;
        assert!(high > low, "density must show as height ({high} > {low})");
    }

    #[test]
    fn commercial_frontage_outgrows_the_back_street() {
        let mut commercial_higher = 0;
        for x in 0..16 {
            let back = plan_kind(tile(x, 2), 0, 0.7, District::Residential, None).tier;
            let front = plan_kind(tile(x, 2), 0, 0.7, District::Commercial, None).tier;
            if front > back {
                commercial_higher += 1;
            }
            assert!(front >= back);
        }
        assert!(commercial_higher > 0);
    }

    #[test]
    fn goods_districts_build_works_not_houses() {
        let kind = plan_kind(tile(6, 6), 1, 0.8, District::Industrial, Some(GoodKind::Ore));
        assert_eq!(kind.family, Family::Works);
    }

    #[test]
    fn fill_order_is_a_permutation() {
        for x in 0..24 {
            for y in 0..24 {
                let mut order = fill_order(tile(x, y));
                order.sort_unstable();
                assert_eq!(order, [0, 1, 2, 3]);
            }
        }
    }

    #[test]
    fn the_countryside_is_mostly_open_land() {
        let mut objects = 0;
        for x in 0..40 {
            for y in 0..40 {
                let t = tile(x, y);
                if rural_prop(t).is_some() || rural_farmstead(t).is_some() {
                    objects += 1;
                }
            }
        }
        let ratio = objects as f32 / 1600.0;
        assert!(
            (0.02..0.10).contains(&ratio),
            "rural coverage {ratio} - open country is the point; a prop on every \
             other tile carpets the map and nothing reads as a place"
        );
    }

    #[test]
    fn a_farm_is_a_cluster_not_a_scatter() {
        // Every prop belongs to a farm, or is one of the rare lone landmarks.
        let mut clustered = 0;
        let mut loose = 0;
        for x in 0..48 {
            for y in 0..48 {
                let t = tile(x, y);
                let Some(prop) = rural_prop(t) else { continue };
                match farm_anchor(t) {
                    Some(a) if (a.x - t.x).abs().max((a.y - t.y).abs()) <= 1 => clustered += 1,
                    _ => {
                        assert_eq!(prop, 5, "only the lone landmark stands away from a farm");
                        loose += 1;
                    }
                }
            }
        }
        assert!(clustered > 0, "farms must actually have fields around them");
        assert!(
            loose < clustered,
            "{loose} loose props against {clustered} clustered ones is a scatter"
        );
    }

    #[test]
    fn farms_are_spaced_out_and_never_double_up() {
        let mut farms = Vec::new();
        for x in 0..64 {
            for y in 0..64 {
                let t = tile(x, y);
                if rural_farmstead(t).is_some() {
                    assert_eq!(farm_anchor(t), Some(t));
                    farms.push(t);
                }
            }
        }
        assert!(!farms.is_empty(), "a map with no farms is unfinished, not rural");
        // One per coarse cell, so a 64x64 map holds tens of farms, not hundreds.
        assert!(
            farms.len() <= (64 / FARM_CELL * (64 / FARM_CELL)) as usize,
            "{} farms on a 64x64 map is a suburb",
            farms.len()
        );
        for a in &farms {
            for b in &farms {
                if a != b {
                    let d = (a.x - b.x).abs().max((a.y - b.y).abs());
                    assert!(d >= 2, "farms at {a:?} and {b:?} are on top of each other");
                }
            }
        }
    }

    #[test]
    fn one_lot_of_a_row_is_set_back_from_the_other() {
        // Overlapping neighbours need a definite front and back to sort by.
        for x in 0..16 {
            for y in 0..16 {
                let t = tile(x, y);
                let west = lot_base(t, 0).1;
                let east = lot_base(t, 1).1;
                assert_ne!(west, east, "block {t:?} draws two lots at the same depth");
            }
        }
    }
}
