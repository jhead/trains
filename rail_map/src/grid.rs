//! [`MapGrid`] — authoritative terrain + portal storage.

use bevy_ecs::prelude::Resource;
use rail_sim::ids::TileCoord;
use serde::{Deserialize, Serialize};

use crate::features::MapFeatures;
use crate::portal::Portal;
use crate::tile::Tile;

/// Default map size for a new sandbox game.
pub const DEFAULT_MAP_WIDTH: u32 = 64;
pub const DEFAULT_MAP_HEIGHT: u32 = 64;
/// Default procedural seed (override via app / future new-game UI).
pub const DEFAULT_MAP_SEED: u64 = 42;

/// Full map: width × height tiles, seed, and closed edge portals.
///
/// # Querying
/// - [`MapGrid::get`] / [`MapGrid::tile`] — tile at [`TileCoord`]
/// - [`Tile::water`] / [`Tile::height`] / [`Tile::is_walkable_for_track`]
/// - [`MapGrid::portals`] / [`MapGrid::portal_at`] — edge portal stubs
/// - [`MapGrid::features`] — what generation *meant*: the opening beat, growth
///   sites, river crossings, ridge passes
///
/// Indexing is row-major: `index = y * width + x` with `0 <= x < width`, `0 <= y < height`.
#[derive(Debug, Clone, Resource, Serialize, Deserialize)]
pub struct MapGrid {
    pub width: u32,
    pub height: u32,
    pub seed: u64,
    tiles: Vec<Tile>,
    portals: Vec<Portal>,
    /// Generator notes. `serde(default)` so a blob written before features
    /// existed still loads — every consumer falls back to measuring.
    #[serde(default)]
    features: MapFeatures,
}

impl MapGrid {
    /// Build an empty (zeroed) grid; prefer [`crate::gen::generate_map`] for gameplay.
    pub fn empty(width: u32, height: u32, seed: u64) -> Self {
        let len = (width as usize)
            .checked_mul(height as usize)
            .expect("map dimensions overflow");
        Self {
            width,
            height,
            seed,
            tiles: vec![
                Tile {
                    height: 0,
                    water: false,
                    kind: crate::tile::TerrainKind::Plains,
                };
                len
            ],
            portals: Vec::new(),
            features: MapFeatures::default(),
        }
    }

    #[inline]
    pub fn contains(&self, coord: TileCoord) -> bool {
        coord.x >= 0
            && coord.y >= 0
            && (coord.x as u32) < self.width
            && (coord.y as u32) < self.height
    }

    #[inline]
    fn index(&self, coord: TileCoord) -> Option<usize> {
        if !self.contains(coord) {
            return None;
        }
        Some(coord.y as usize * self.width as usize + coord.x as usize)
    }

    /// Tile at `coord`, or `None` if out of bounds.
    pub fn get(&self, coord: TileCoord) -> Option<&Tile> {
        self.index(coord).map(|i| &self.tiles[i])
    }

    /// Mutable tile at `coord`, or `None` if out of bounds.
    pub fn get_mut(&mut self, coord: TileCoord) -> Option<&mut Tile> {
        self.index(coord).map(|i| &mut self.tiles[i])
    }

    /// Panicking accessor for in-bounds coords (tests / trusted callers).
    pub fn tile(&self, coord: TileCoord) -> &Tile {
        self.get(coord)
            .unwrap_or_else(|| panic!("tile out of bounds: ({}, {})", coord.x, coord.y))
    }

    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    pub fn tiles_mut(&mut self) -> &mut [Tile] {
        &mut self.tiles
    }

    /// All edge portals (closed in MVP).
    pub fn portals(&self) -> &[Portal] {
        &self.portals
    }

    pub fn portals_mut(&mut self) -> &mut Vec<Portal> {
        &mut self.portals
    }

    /// Portal on a border tile, if any.
    pub fn portal_at(&self, coord: TileCoord) -> Option<&Portal> {
        self.portals.iter().find(|p| p.tile == coord)
    }

    /// What generation meant by this map — see [`MapFeatures`].
    ///
    /// Empty on a hand-built grid or one restored from a blob that predates the
    /// record; [`crate::measure`] falls back to geometry in that case.
    pub fn features(&self) -> &MapFeatures {
        &self.features
    }

    pub fn features_mut(&mut self) -> &mut MapFeatures {
        &mut self.features
    }

    /// Sites anchor placement should seed itself from, best first: the home
    /// town, its near neighbour, then the rest (design 02 §4.1).
    pub fn anchor_hints(&self) -> Vec<TileCoord> {
        self.features.anchor_hints()
    }

    /// Whether this coordinate sits on the map border.
    pub fn is_border(&self, coord: TileCoord) -> bool {
        if !self.contains(coord) {
            return false;
        }
        coord.x == 0
            || coord.y == 0
            || coord.x as u32 == self.width - 1
            || coord.y as u32 == self.height - 1
    }
}
