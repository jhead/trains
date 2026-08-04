//! Depth sorting for the isometric view.
//!
//! Top-down needs no depth: layers are enough, and the game's z values say so —
//! terrain 0, track 1, buildings ~1.5, trains 3, feedback 3.5, overlays 4.5.
//! Isometric needs both. A mountain is terrain and must still hide the train
//! behind it, so **the diagonal row wins and the layer only breaks ties inside
//! it**.
//!
//! The row is `x + y`: the projection puts the camera over the map's south-west
//! corner, so `x + y` counts *away* from it and the near row draws last.
//!
//! ```text
//! z = (ROWS - (x + y)) · ROW_Z  +  layer · LAYER_Z
//! ```
//!
//! [`ROWS`] is generous headroom, so a bigger map only moves the band rather
//! than clamping it, and [`LAYER_Z`] is small enough that a whole layer stack
//! fits between two rows. On a 64 × 64 map the band is z 38.6 … 51.2, which
//! stays clear of the two things that live above the world — the time-of-day
//! tint (64) and the Map View plate (200).
//!
//! # Everything else is sorted after the fact
//!
//! [`crate::map::iso_sort::iso_depth_sort`] rewrites z for every world sprite in
//! `PostUpdate`, reading the tile under the sprite back out of the projection.
//! That is one system instead of an edit to all ~20 spawners, and it keeps
//! working for entities whose position is written fresh every frame (trains,
//! peeps, smoke). Terrain sets its own z at spawn and opts out.

use bevy::prelude::*;
use rail_map::world_to_tile;

/// z granted to one diagonal row.
pub const ROW_Z: f32 = 0.1;
/// Rows of headroom before the band would reach zero.
pub const ROWS: f32 = 512.0;
/// z per unit of layer. Six layers have to fit inside one row.
pub const LAYER_Z: f32 = 0.015;

/// Layer for terrain, which is under everything on its own tile.
pub const TERRAIN_LAYER: f32 = 0.0;

/// Below this, a z value is a layer a gameplay system just wrote; above it, a
/// z value is either one of ours or an overlay that lives above the world.
pub const BAND_FLOOR: f32 = 20.0;

/// Sorted z for a diagonal row and a layer.
#[inline]
pub fn depth_z(row: i32, layer: f32) -> f32 {
    (ROWS - row as f32) * ROW_Z + layer.clamp(0.0, 6.0) * LAYER_Z
}

/// Sorted z for a screen-space position, resolving the tile under it.
#[inline]
pub fn depth_z_at(screen: Vec2, layer: f32) -> f32 {
    let tile = world_to_tile(screen.x, screen.y);
    depth_z(tile.x + tile.y, layer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_near_row_draws_in_front_of_the_far_one() {
        assert!(depth_z(0, 0.0) > depth_z(1, 0.0));
        assert!(depth_z(10, 0.0) > depth_z(126, 0.0));
    }

    #[test]
    fn a_row_outranks_every_layer_inside_it() {
        // Terrain one row nearer beats a train one row further back.
        assert!(depth_z(5, TERRAIN_LAYER) > depth_z(6, 4.5));
        // ... and inside one row the layers still stack in order.
        assert!(depth_z(5, 3.0) > depth_z(5, TERRAIN_LAYER));
        assert!(depth_z(5, 4.5) > depth_z(5, 3.0));
    }

    #[test]
    fn the_whole_band_clears_the_overlays_above_the_world() {
        // A 64 x 64 map, its nearest and furthest rows.
        let near = depth_z(0, 6.0);
        let far = depth_z(126, 0.0);
        assert!(far > BAND_FLOOR, "the band dips into fresh layer z: {far}");
        assert!(near < 64.0, "the band collides with the day tint: {near}");
        // Even a 256 x 256 map keeps its feet above the floor.
        assert!(depth_z(510, 0.0) > 0.0);
    }
}
