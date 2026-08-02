//! World ↔ tile conversion helpers.
//!
//! Tile `(0, 0)` is the south-west corner of the map. World space places that
//! tile's **center** at `(TILE_SIZE / 2, TILE_SIZE / 2)`, with +X east and +Y north.

use rail_sim::ids::TileCoord;

/// Side length of one map tile in world units (pixels at 1:1 ortho scale).
pub const TILE_SIZE: f32 = 32.0;

/// World-space center of a tile.
#[inline]
pub fn tile_to_world(coord: TileCoord) -> (f32, f32) {
    (
        (coord.x as f32 + 0.5) * TILE_SIZE,
        (coord.y as f32 + 0.5) * TILE_SIZE,
    )
}

/// Tile containing a world-space point (floor toward −∞).
#[inline]
pub fn world_to_tile(x: f32, y: f32) -> TileCoord {
    TileCoord {
        x: (x / TILE_SIZE).floor() as i32,
        y: (y / TILE_SIZE).floor() as i32,
    }
}

/// World-space center of the whole map (useful for framing the camera).
#[inline]
pub fn map_center_world(width: u32, height: u32) -> (f32, f32) {
    (
        width as f32 * TILE_SIZE * 0.5,
        height as f32 * TILE_SIZE * 0.5,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_tile_center() {
        let c = TileCoord { x: 3, y: 7 };
        let (wx, wy) = tile_to_world(c);
        assert_eq!(world_to_tile(wx, wy), c);
    }

    #[test]
    fn world_edges_map_to_correct_tile() {
        assert_eq!(world_to_tile(0.0, 0.0), TileCoord { x: 0, y: 0 });
        assert_eq!(world_to_tile(31.9, 31.9), TileCoord { x: 0, y: 0 });
        assert_eq!(world_to_tile(32.0, 32.0), TileCoord { x: 1, y: 1 });
    }
}
