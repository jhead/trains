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

/// Density at which each successive lot is taken up.
pub const LOT_UP: [f32; 4] = [0.10, 0.32, 0.56, 0.80];
/// Density at which each lot is given up again — the gap is deliberate
/// hysteresis so a lot never flickers on a jittering density value.
pub const LOT_DOWN: [f32; 4] = [0.06, 0.26, 0.48, 0.72];

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
/// Includes the small hashed offset that keeps a street from looking gridded.
/// Both components are whole texels — the pixel contract admits nothing else.
pub fn lot_base(tile: TileCoord, slot: u8) -> (i32, i32) {
    let (lx, ly) = lot_coord(tile, slot);
    let origin_x = tile.x * TILE_TEXELS + (slot & 1) as i32 * LOT_TEXELS;
    let origin_y = tile.y * TILE_TEXELS + (slot >> 1) as i32 * LOT_TEXELS;
    let jitter_x = (world_hash(lx, ly, 0x00FF) % 5) as i32 - 2;
    let jitter_y = (world_hash(lx, ly, 0xFF00) % 4) as i32;
    (
        origin_x + LOT_TEXELS / 2 + jitter_x,
        origin_y + 1 + jitter_y,
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

/// Rural props scattered on land the railway has not reached.
///
/// Returns the prop index within [`super::building_art::FRAME_RURAL`], or
/// `None` for open ground — countryside needs air as much as it needs objects.
pub fn rural_prop(tile: TileCoord) -> Option<usize> {
    match world_hash(tile.x, tile.y, 0xC0DE) % 100 {
        0..=9 => Some(0),   // ploughed field
        10..=16 => Some(1), // haystack
        17..=25 => Some(2), // hedgerow
        26..=30 => Some(3), // lane and gate
        31..=36 => Some(4), // dry-stone wall
        37..=44 => Some(5), // lone tree
        _ => None,
    }
}

/// Which lot a rural prop sits on, so props keep the same 16-texel rhythm.
pub fn rural_slot(tile: TileCoord) -> u8 {
    (world_hash(tile.x, tile.y, 0xBEEF) % 4) as u8
}

/// A rural block occasionally holds a lone farmstead rather than a prop.
pub fn rural_farmstead(tile: TileCoord) -> Option<BuildingKind> {
    let h = world_hash(tile.x, tile.y, 0xFA47) % 100;
    if h >= 7 {
        return None;
    }
    let (lx, ly) = lot_coord(tile, rural_slot(tile));
    Some(BuildingKind {
        // A farm is a working building, and it should read as one.
        family: if h < 4 { Family::Town } else { Family::Works },
        tier: 0,
        variant: (world_hash(lx, ly, 0x1234) % 4) as u8,
        roof: if h.is_multiple_of(2) { Roof::Tile } else { Roof::Slate },
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
            "only {} distinct buildings across a street — that reads as wallpaper",
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
    fn countryside_is_populated_but_not_crowded() {
        let mut props = 0;
        for x in 0..40 {
            for y in 0..40 {
                if rural_prop(tile(x, y)).is_some() {
                    props += 1;
                }
            }
        }
        let ratio = props as f32 / 1600.0;
        assert!(
            (0.30..0.55).contains(&ratio),
            "rural coverage {ratio} should read as countryside, not as empty or as forest"
        );
    }
}
