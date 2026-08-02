//! What generation *meant*, kept alongside what it produced.
//!
//! A heightmap forgets its own intentions. Once a ridge is a field of `i8` there
//! is no honest way to ask "where were the passes?" — only ways to guess. Design
//! 02 §4 makes anchor placement level design, and level design needs the
//! generator's own answer, not an inference from the output.
//!
//! So the generator records it. [`MapFeatures`] rides on [`crate::MapGrid`] and
//! carries the opening beat (§4.1), the candidate sites the world can grow into
//! (§4.3), where the river can be bridged and how wide the span is (§2.1), and
//! where the ridges let you through (§2.2).
//!
//! Everything here is optional: a grid rebuilt from an older save deserialises
//! with [`MapFeatures::default`] and every consumer falls back to measuring
//! ([`crate::measure`]).

use rail_sim::ids::TileCoord;
use serde::{Deserialize, Serialize};

/// What a tile's water (or lack of it) *is*, which its height cannot say.
///
/// A river that reaches the sea is one connected body of water, so geometry
/// alone cannot separate "the crossing decision" from "the frame". Generation
/// knows, and records it here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Surface {
    #[default]
    Land,
    /// Open sea: edge framing, harbours, bays.
    Sea,
    /// Flowing inland water — the crossing decision.
    River,
    /// Standing inland water in a basin.
    Lake,
}

impl Surface {
    #[inline]
    pub fn is_water(self) -> bool {
        !matches!(self, Self::Land)
    }

    /// Rivers and lakes — the inland-water row of the §2.1 table.
    #[inline]
    pub fn is_inland_water(self) -> bool {
        matches!(self, Self::River | Self::Lake)
    }
}

/// Why the generator likes a tile. Anchor placement reads this to put a thing
/// where its reason makes sense (§4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SiteKind {
    /// Flat, well-connected ground: a settlement belongs here.
    Town,
    /// Sheltered lowland away from rock — a sawmill's forest.
    Forest,
    /// Buildable ground pressed against impassable rock — a quarry.
    Quarry,
    /// Land on a bay, with open sea in reach — a harbour.
    Harbour,
}

/// A place worth putting something, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SiteHint {
    pub tile: TileCoord,
    pub kind: SiteKind,
}

/// Somewhere a river can actually be bridged.
///
/// `span` is the contiguous water run a bridge would have to cover, which is
/// what `rail_sim`'s `MAX_BRIDGE_SPAN` and `bridge_cost_for_span` charge on. A
/// span-1 crossing is routine; a span-3 crossing is 20× base and a commitment
/// (§3.4) — and the whole point is that the map offers both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RiverCrossing {
    /// A water tile in the middle of the crossable run.
    pub tile: TileCoord,
    /// Contiguous water tiles a bridge spans here (1..=`MAX_BRIDGE_SPAN`).
    pub span: u32,
}

/// The generator's notes on the world it just built.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MapFeatures {
    /// Per-tile surface class, row-major (`y * width + x`). Empty when unknown.
    pub surface: Vec<Surface>,
    /// The opening beat's home town: buildable, near map centre (§4.1).
    pub home: Option<TileCoord>,
    /// Its first destination, 8–12 tiles away with a terrain question between.
    pub near: Option<TileCoord>,
    /// Further sites the world can grow into, best first (§4.2, §4.3).
    pub sites: Vec<SiteHint>,
    /// Bridgeable points on the river systems, along the course.
    pub crossings: Vec<RiverCrossing>,
    /// One tile per gap through a ridge — a ridge with two of these is a
    /// decision, with twenty it is a texture (§2.2).
    pub passes: Vec<TileCoord>,
}

impl MapFeatures {
    /// Surface class at a row-major index, or [`Surface::Land`] when generation
    /// left no notes (an older save, a hand-built grid).
    #[inline]
    pub fn surface_at(&self, index: usize) -> Option<Surface> {
        self.surface.get(index).copied()
    }

    /// Whether this record actually describes a grid of `len` tiles.
    #[inline]
    pub fn describes(&self, len: usize) -> bool {
        self.surface.len() == len
    }

    /// Suggested anchor sites, best first: home, its near neighbour, then the
    /// rest. This is the list anchor placement should seed itself from — see the
    /// crate docs for the `rail_sim` side of the handshake.
    pub fn anchor_hints(&self) -> Vec<TileCoord> {
        let mut out = Vec::with_capacity(2 + self.sites.len());
        out.extend(self.home);
        out.extend(self.near);
        for site in &self.sites {
            if !out.contains(&site.tile) {
                out.push(site.tile);
            }
        }
        out
    }

    /// Sites of one kind, in the order generation ranked them.
    pub fn sites_of(&self, kind: SiteKind) -> impl Iterator<Item = TileCoord> + '_ {
        self.sites
            .iter()
            .filter(move |s| s.kind == kind)
            .map(|s| s.tile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_hints_lead_with_the_opening_pair() {
        let features = MapFeatures {
            home: Some(TileCoord { x: 30, y: 32 }),
            near: Some(TileCoord { x: 38, y: 36 }),
            sites: vec![
                SiteHint {
                    tile: TileCoord { x: 38, y: 36 },
                    kind: SiteKind::Town,
                },
                SiteHint {
                    tile: TileCoord { x: 12, y: 50 },
                    kind: SiteKind::Quarry,
                },
            ],
            ..MapFeatures::default()
        };
        let hints = features.anchor_hints();
        assert_eq!(hints[0], TileCoord { x: 30, y: 32 });
        assert_eq!(hints[1], TileCoord { x: 38, y: 36 });
        // The near site is not offered twice just because it is also a site.
        assert_eq!(hints.len(), 3);
    }

    #[test]
    fn empty_features_describe_nothing() {
        let features = MapFeatures::default();
        assert!(!features.describes(16));
        assert_eq!(features.surface_at(0), None);
        assert!(features.anchor_hints().is_empty());
    }
}
