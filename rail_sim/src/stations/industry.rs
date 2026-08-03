//! Industries that produce / consume goods for transport trains.
//!
//! An industry stands on a **lot**, not on a tile ([04 — Building & Tools] §6:
//! "a goods platform placed against an industry"). The lot is what a
//! [`StationTier::GoodsPlatform`](super::tier::StationTier::GoodsPlatform) has
//! to touch, so its size is the one number that makes the freight rule mean
//! something: a quarry complex can be reached from a longer stretch of line
//! than a one-tile yard.

use std::collections::HashMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::ids::TileCoord;

/// Stable industry id (save / commands / jobs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IndustryId(pub u64);

/// Cargo types moved by transport trains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GoodKind {
    Lumber,
    Ore,
}

impl GoodKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Lumber => "lumber",
            Self::Ore => "ore",
        }
    }
}

/// Ground an industry stands on, as a Chebyshev radius from its tile.
///
/// | Tier | Lot | Character |
/// | --- | --- | --- |
/// | Yard | 1x1 | A siding and a shed — one tile of ground |
/// | Works | 3x3 | The workhorse: mills, sawmills, foundries |
/// | Complex | 5x5 | Quarries and harbours; a platform can meet it anywhere along its edge |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum IndustryTier {
    Yard,
    #[default]
    Works,
    Complex,
}

impl IndustryTier {
    pub const ALL: [IndustryTier; 3] = [Self::Yard, Self::Works, Self::Complex];

    /// Chebyshev radius of the lot around [`Industry::tile`].
    #[inline]
    pub fn lot_radius(self) -> i32 {
        match self {
            Self::Yard => 0,
            Self::Works => 1,
            Self::Complex => 2,
        }
    }

    /// Side of the square lot in tiles (`2 * radius + 1`).
    #[inline]
    pub fn lot_side(self) -> i32 {
        self.lot_radius() * 2 + 1
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Yard => "Yard",
            Self::Works => "Works",
            Self::Complex => "Complex",
        }
    }
}

/// A produce-and/or-consume site on land.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Industry {
    pub id: IndustryId,
    pub name: String,
    /// Centre of the lot.
    pub tile: TileCoord,
    /// How much ground it stands on — see [`IndustryTier`].
    pub tier: IndustryTier,
    pub produces: Option<GoodKind>,
    pub consumes: Option<GoodKind>,
}

impl Industry {
    /// `true` when `tile` is inside the lot.
    pub fn lot_contains(&self, tile: TileCoord) -> bool {
        self.lot_distance(tile) <= self.tier.lot_radius()
    }

    /// `true` when `tile` is on the lot or in the ring of tiles around it —
    /// which is what "placed **against** an industry" means for a platform.
    pub fn abuts(&self, tile: TileCoord) -> bool {
        self.lot_distance(tile) <= self.tier.lot_radius() + 1
    }

    /// Chebyshev tiles from the lot centre.
    fn lot_distance(&self, tile: TileCoord) -> i32 {
        (self.tile.x - tile.x).abs().max((self.tile.y - tile.y).abs())
    }
}

/// Registry of industries.
#[derive(Debug, Clone, Default, Resource)]
pub struct IndustryRegistry {
    industries: HashMap<IndustryId, Industry>,
    by_tile: HashMap<(i32, i32), IndustryId>,
    next_id: u64,
}

impl IndustryRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.industries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.industries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Industry> {
        self.industries.values()
    }

    pub fn get(&self, id: IndustryId) -> Option<&Industry> {
        self.industries.get(&id)
    }

    pub fn at(&self, tile: TileCoord) -> Option<&Industry> {
        let id = *self.by_tile.get(&(tile.x, tile.y))?;
        self.industries.get(&id)
    }

    /// Insert a default-tier ([`IndustryTier::Works`]) site.
    pub fn insert(
        &mut self,
        name: impl Into<String>,
        tile: TileCoord,
        produces: Option<GoodKind>,
        consumes: Option<GoodKind>,
    ) -> IndustryId {
        self.insert_tier(name, tile, IndustryTier::default(), produces, consumes)
    }

    /// Insert a site standing on a `tier`-sized lot; panics if the tile is taken.
    pub fn insert_tier(
        &mut self,
        name: impl Into<String>,
        tile: TileCoord,
        tier: IndustryTier,
        produces: Option<GoodKind>,
        consumes: Option<GoodKind>,
    ) -> IndustryId {
        self.next_id = self.next_id.saturating_add(1);
        let id = IndustryId(self.next_id);
        let key = (tile.x, tile.y);
        assert!(
            !self.by_tile.contains_key(&key),
            "industry tile already occupied"
        );
        self.by_tile.insert(key, id);
        self.industries.insert(
            id,
            Industry {
                id,
                name: name.into(),
                tile,
                tier,
                produces,
                consumes,
            },
        );
        id
    }

    /// The industry whose lot `tile` stands on, if any.
    ///
    /// Ties (overlapping lots) break to the lowest [`IndustryId`] so the answer
    /// never depends on hash order.
    pub fn lot_at(&self, tile: TileCoord) -> Option<&Industry> {
        self.industries
            .values()
            .filter(|i| i.lot_contains(tile))
            .min_by_key(|i| i.id.0)
    }

    /// The industry a platform on `tile` would be built against, if any.
    ///
    /// On the lot or in the ring around it — see [`Industry::abuts`].
    pub fn abutting(&self, tile: TileCoord) -> Option<&Industry> {
        self.industries
            .values()
            .filter(|i| i.abuts(tile))
            .min_by_key(|i| i.id.0)
    }

    /// First industry that produces `good`, if any.
    pub fn producer_of(&self, good: GoodKind) -> Option<&Industry> {
        self.industries
            .values()
            .find(|i| i.produces == Some(good))
    }

    /// First industry that consumes `good`, if any.
    pub fn consumer_of(&self, good: GoodKind) -> Option<&Industry> {
        self.industries
            .values()
            .find(|i| i.consumes == Some(good))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lot_grows_with_the_tier() {
        assert_eq!(IndustryTier::Yard.lot_side(), 1);
        assert_eq!(IndustryTier::Works.lot_side(), 3);
        assert_eq!(IndustryTier::Complex.lot_side(), 5);
        assert!(IndustryTier::Complex.lot_radius() > IndustryTier::Works.lot_radius());
        assert_eq!(IndustryTier::default(), IndustryTier::Works);
    }

    #[test]
    fn a_platform_abuts_the_lot_edge_but_not_the_tile_beyond_it() {
        let mut reg = IndustryRegistry::new();
        let id = reg.insert_tier(
            "Pine Sawmill",
            TileCoord { x: 10, y: 10 },
            IndustryTier::Works,
            Some(GoodKind::Lumber),
            None,
        );
        let mill = reg.get(id).expect("industry");

        // 3x3 lot: inside at 1, the ring at 2, nothing at 3.
        assert!(mill.lot_contains(TileCoord { x: 11, y: 11 }));
        assert!(!mill.lot_contains(TileCoord { x: 12, y: 10 }));
        assert!(mill.abuts(TileCoord { x: 12, y: 10 }));
        assert!(!mill.abuts(TileCoord { x: 13, y: 10 }));

        assert_eq!(reg.abutting(TileCoord { x: 12, y: 10 }).map(|i| i.id), Some(id));
        assert_eq!(reg.abutting(TileCoord { x: 13, y: 10 }), None);
        assert_eq!(reg.lot_at(TileCoord { x: 10, y: 11 }).map(|i| i.id), Some(id));
    }

    #[test]
    fn a_yard_is_reached_only_from_the_tiles_touching_it() {
        let mut reg = IndustryRegistry::new();
        let id = reg.insert_tier(
            "Cedar Yard",
            TileCoord { x: 4, y: 4 },
            IndustryTier::Yard,
            Some(GoodKind::Lumber),
            None,
        );
        let yard = reg.get(id).expect("industry");
        assert!(yard.abuts(TileCoord { x: 5, y: 5 }), "diagonals touch");
        assert!(!yard.abuts(TileCoord { x: 6, y: 4 }));
    }
}
