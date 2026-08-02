//! Immutable terrain snapshot for track placement (avoids rail_sim ↔ rail_map cycle).
//!
//! Populate once at game start from [`rail_map::MapGrid`]; MVP terrain does not change.

use bevy_ecs::prelude::Resource;

use crate::ids::TileCoord;

/// Read-only height / water grid used by track rules.
#[derive(Debug, Clone, Resource)]
pub struct TrackTerrain {
    width: u32,
    height: u32,
    /// Row-major: `y * width + x`.
    water: Vec<bool>,
    heights: Vec<i8>,
}

impl TrackTerrain {
    /// Build from parallel water/height iterators (row-major, length `width * height`).
    pub fn new(
        width: u32,
        height: u32,
        cells: impl IntoIterator<Item = (bool, i8)>,
    ) -> Self {
        let len = (width as usize)
            .checked_mul(height as usize)
            .expect("terrain dimensions overflow");
        let mut water = Vec::with_capacity(len);
        let mut heights = Vec::with_capacity(len);
        for (w, h) in cells {
            water.push(w);
            heights.push(h);
        }
        assert_eq!(water.len(), len, "cell count must equal width * height");
        Self {
            width,
            height,
            water,
            heights,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
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

    #[inline]
    pub fn is_water(&self, coord: TileCoord) -> bool {
        self.index(coord)
            .map(|i| self.water[i])
            .unwrap_or(true)
    }

    #[inline]
    pub fn height_at(&self, coord: TileCoord) -> Option<i8> {
        self.index(coord).map(|i| self.heights[i])
    }

    /// Contiguous water run through `coord` along +X/−X (inclusive). `0` if land.
    pub fn water_span_horizontal(&self, coord: TileCoord) -> u32 {
        if !self.contains(coord) || !self.is_water(coord) {
            return 0;
        }
        let mut count = 1u32;
        let mut x = coord.x - 1;
        while self.contains(TileCoord { x, y: coord.y })
            && self.is_water(TileCoord { x, y: coord.y })
        {
            count += 1;
            x -= 1;
        }
        x = coord.x + 1;
        while self.contains(TileCoord { x, y: coord.y })
            && self.is_water(TileCoord { x, y: coord.y })
        {
            count += 1;
            x += 1;
        }
        count
    }

    /// Contiguous water run through `coord` along +Y/−Y (inclusive). `0` if land.
    pub fn water_span_vertical(&self, coord: TileCoord) -> u32 {
        if !self.contains(coord) || !self.is_water(coord) {
            return 0;
        }
        let mut count = 1u32;
        let mut y = coord.y - 1;
        while self.contains(TileCoord { x: coord.x, y })
            && self.is_water(TileCoord { x: coord.x, y })
        {
            count += 1;
            y -= 1;
        }
        y = coord.y + 1;
        while self.contains(TileCoord { x: coord.x, y })
            && self.is_water(TileCoord { x: coord.x, y })
        {
            count += 1;
            y += 1;
        }
        count
    }
}
