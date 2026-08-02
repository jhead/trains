//! Eight-way tile directions for track links and autofill.

use crate::ids::TileCoord;

/// Cardinal + ordinal neighbors (N, NE, E, SE, S, SW, W, NW).
pub const DIR8: [(i32, i32); 8] = [
    (0, 1),
    (1, 1),
    (1, 0),
    (1, -1),
    (0, -1),
    (-1, -1),
    (-1, 0),
    (-1, 1),
];

/// Bitmask of which of the 8 directions have a linked track neighbor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct TrackLinks(pub u8);

impl TrackLinks {
    #[inline]
    pub fn empty() -> Self {
        Self(0)
    }

    #[inline]
    pub fn set(&mut self, dir_index: usize) {
        self.0 |= 1 << (dir_index as u8);
    }

    #[inline]
    pub fn clear(&mut self, dir_index: usize) {
        self.0 &= !(1 << (dir_index as u8));
    }

    #[inline]
    pub fn has(&self, dir_index: usize) -> bool {
        self.0 & (1 << (dir_index as u8)) != 0
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Number of connected directions (0–8).
    pub fn count(self) -> u32 {
        self.0.count_ones()
    }
}

/// Index into [`DIR8`] for stepping from `from` to an adjacent `to`, if any.
pub fn dir_index(from: TileCoord, to: TileCoord) -> Option<usize> {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    DIR8.iter().position(|&(x, y)| x == dx && y == dy)
}

/// Opposite direction index (0↔4, 1↔5, …).
#[inline]
pub fn opposite_dir(dir_index: usize) -> usize {
    (dir_index + 4) % 8
}

/// Offset a tile by a [`DIR8`] entry.
#[inline]
pub fn step(coord: TileCoord, dir_index: usize) -> TileCoord {
    let (dx, dy) = DIR8[dir_index];
    TileCoord {
        x: coord.x + dx,
        y: coord.y + dy,
    }
}
