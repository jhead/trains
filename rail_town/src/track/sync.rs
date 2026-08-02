//! Copy [`MapGrid`] into [`TrackTerrain`] once (MVP terrain is immutable).

use bevy::prelude::*;
use rail_map::MapGrid;
use rail_sim::ids::TileCoord;
use rail_sim::TrackTerrain;

pub fn sync_track_terrain_from_map(mut commands: Commands, map: Res<MapGrid>) {
    let mut cells = Vec::with_capacity((map.width as usize) * (map.height as usize));
    for y in 0..map.height {
        for x in 0..map.width {
            let tile = map.tile(TileCoord {
                x: x as i32,
                y: y as i32,
            });
            cells.push((tile.water, tile.height));
        }
    }
    commands.insert_resource(TrackTerrain::new(map.width, map.height, cells));
}
