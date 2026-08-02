//! Chunked terrain rendering — 16 × 16 tiles composite to one drawn sprite.
//!
//! Brief 01 §2.5: art is baked when data changes, never per frame, and the unit
//! of rebuild is a chunk. Nothing here runs per tile per frame; the whole map is
//! a handful of quads once startup is done.
//!
//! Texels land on world units 1:1 and chunk origins are whole tiles, so the
//! terrain stays contiguous and integer-aligned under the camera's texel snap.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rail_map::{MapGrid, TILE_SIZE};
use rail_sim::ids::TileCoord;

use super::atlas::{TerrainAtlas, CELL, TRANSITION_BASE};
use super::autotile::resolve_tile;

/// Tiles per chunk edge (brief 01 §2.5).
pub const CHUNK_TILES: u32 = 16;
/// Terrain is the bottom layer band (brief 01 §6.1).
pub const TERRAIN_Z: f32 = 0.0;

/// One composited terrain chunk.
#[derive(Component, Debug, Clone, Copy)]
pub struct TerrainChunk {
    pub cx: u32,
    pub cy: u32,
}

/// Chunks whose composited art no longer matches the map data.
#[derive(Resource, Default)]
pub struct TerrainDirty {
    dirty: Vec<bool>,
    cols: u32,
    rows: u32,
    any: bool,
}

impl TerrainDirty {
    fn resize(&mut self, cols: u32, rows: u32) {
        self.cols = cols;
        self.rows = rows;
        self.dirty = vec![false; (cols * rows) as usize];
        self.any = false;
    }

    pub fn mark_all(&mut self) {
        self.dirty.fill(true);
        self.any = !self.dirty.is_empty();
    }

    /// Mark the chunks affected by one tile changing.
    ///
    /// Autotiling reads a tile's diagonals, so an edit on a chunk seam dirties
    /// its neighbours too.
    #[allow(dead_code)] // Entry point for terraforming / bridges in later slices.
    pub fn mark_tile(&mut self, coord: TileCoord) {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let x = coord.x + dx;
                let y = coord.y + dy;
                if x < 0 || y < 0 {
                    continue;
                }
                let cx = x as u32 / CHUNK_TILES;
                let cy = y as u32 / CHUNK_TILES;
                if cx < self.cols && cy < self.rows {
                    self.dirty[(cy * self.cols + cx) as usize] = true;
                    self.any = true;
                }
            }
        }
    }

    #[inline]
    fn is_dirty(&self, cx: u32, cy: u32) -> bool {
        self.dirty
            .get((cy * self.cols + cx) as usize)
            .copied()
            .unwrap_or(false)
    }

    fn clear(&mut self) {
        self.dirty.fill(false);
        self.any = false;
    }
}

/// Tiles this chunk actually covers — the last row / column may be short.
#[inline]
fn chunk_extent(map: &MapGrid, cx: u32, cy: u32) -> (u32, u32) {
    (
        (map.width - cx * CHUNK_TILES).min(CHUNK_TILES),
        (map.height - cy * CHUNK_TILES).min(CHUNK_TILES),
    )
}

/// World-space centre of a chunk. Always a whole number of texels.
#[inline]
fn chunk_center(map: &MapGrid, cx: u32, cy: u32) -> (f32, f32) {
    let (w, h) = chunk_extent(map, cx, cy);
    (
        (cx * CHUNK_TILES) as f32 * TILE_SIZE + w as f32 * TILE_SIZE * 0.5,
        (cy * CHUNK_TILES) as f32 * TILE_SIZE + h as f32 * TILE_SIZE * 0.5,
    )
}

/// Composite every tile of a chunk into `data`.
///
/// Row 0 of the image is the chunk's **north** edge, so tile rows are written
/// bottom-up. Layers stamp in resolve order; the base is opaque and copies
/// wholesale, overlays test alpha per texel (the art is never partly
/// transparent, so there is nothing to blend).
fn composite(map: &MapGrid, atlas: &TerrainAtlas, cx: u32, cy: u32, data: &mut [u8]) {
    let (tiles_w, tiles_h) = chunk_extent(map, cx, cy);
    let stride = (tiles_w * CELL) as usize * 4;
    let row_bytes = CELL as usize * 4;

    for ty in 0..tiles_h {
        let oy = ((tiles_h - 1 - ty) * CELL) as usize;
        for tx in 0..tiles_w {
            let ox = (tx * CELL) as usize * 4;
            let coord = TileCoord {
                x: (cx * CHUNK_TILES + tx) as i32,
                y: (cy * CHUNK_TILES + ty) as i32,
            };
            let draw = resolve_tile(map, coord);
            for &cell in draw.layers() {
                let cell = cell as usize;
                let opaque = cell < TRANSITION_BASE;
                for row in 0..CELL {
                    let src = atlas.cell_row(cell, row);
                    let start = (oy + row as usize) * stride + ox;
                    let dst = &mut data[start..start + row_bytes];
                    if opaque {
                        dst.copy_from_slice(src);
                    } else {
                        for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                            if s[3] != 0 {
                                d.copy_from_slice(s);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn chunk_image(map: &MapGrid, atlas: &TerrainAtlas, cx: u32, cy: u32) -> Image {
    let (tiles_w, tiles_h) = chunk_extent(map, cx, cy);
    let mut image = Image::new_fill(
        Extent3d {
            width: tiles_w * CELL,
            height: tiles_h * CELL,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    // One texel is one screen pixel times a whole number — never filtered,
    // never mipmapped (brief 01 §2.1).
    image.sampler = ImageSampler::nearest();
    if let Some(data) = image.data.as_mut() {
        composite(map, atlas, cx, cy, data);
    }
    image
}

/// Composite every chunk of `map` and spawn one sprite each.
fn spawn_chunks(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    map: &MapGrid,
    atlas: &TerrainAtlas,
    dirty: &mut TerrainDirty,
) -> (u32, u32) {
    let cols = map.width.div_ceil(CHUNK_TILES);
    let rows = map.height.div_ceil(CHUNK_TILES);
    dirty.resize(cols, rows);

    for cy in 0..rows {
        for cx in 0..cols {
            let handle = images.add(chunk_image(map, atlas, cx, cy));
            let (wx, wy) = chunk_center(map, cx, cy);
            commands.spawn((
                TerrainChunk { cx, cy },
                Sprite::from_image(handle),
                Transform::from_xyz(wx, wy, TERRAIN_Z),
            ));
        }
    }
    (cols, rows)
}

/// Paint the atlas, then build the chunk grid for the starting map.
pub fn setup_terrain(
    mut commands: Commands,
    map: Res<MapGrid>,
    mut images: ResMut<Assets<Image>>,
    mut dirty: ResMut<TerrainDirty>,
) {
    let started = Instant::now();
    let atlas = TerrainAtlas::build();
    let atlas_done = Instant::now();

    let (cols, rows) = spawn_chunks(&mut commands, &mut images, &map, &atlas, &mut dirty);

    info!(
        "terrain: atlas {} texels in {:?}, {}x{} chunks composited in {:?}",
        atlas.texel_count(),
        atlas_done - started,
        cols,
        rows,
        atlas_done.elapsed(),
    );
    commands.insert_resource(atlas);
}

/// Re-composite chunks whose tiles changed. Idle when nothing has.
///
/// Replacing the whole [`MapGrid`] — a new game, a reloaded save — is enough on
/// its own: the grid's change tick rebuilds every chunk, and a grid of a
/// different size rebuilds the chunk entities too. Nothing else has to
/// re-register terrain when the world is swapped.
pub fn rebuild_dirty_terrain(
    mut commands: Commands,
    map: Res<MapGrid>,
    atlas: Option<Res<TerrainAtlas>>,
    mut dirty: ResMut<TerrainDirty>,
    mut images: ResMut<Assets<Image>>,
    chunks: Query<(Entity, &TerrainChunk, &Sprite)>,
) {
    let swapped = map.is_changed() && !map.is_added();
    if swapped {
        dirty.mark_all();
    }
    if !dirty.any {
        return;
    }
    let Some(atlas) = atlas else {
        return;
    };

    // A different map size means a different chunk grid: drop it and rebuild.
    let resized = map.width.div_ceil(CHUNK_TILES) != dirty.cols
        || map.height.div_ceil(CHUNK_TILES) != dirty.rows;
    if swapped && resized {
        for (entity, _, sprite) in &chunks {
            images.remove(&sprite.image);
            commands.entity(entity).despawn();
        }
        spawn_chunks(&mut commands, &mut images, &map, &atlas, &mut dirty);
        dirty.clear();
        return;
    }

    for (_, chunk, sprite) in &chunks {
        if !dirty.is_dirty(chunk.cx, chunk.cy) {
            continue;
        }
        let Some(image) = images.get_mut(&sprite.image) else {
            continue;
        };
        let Some(data) = image.data.as_mut() else {
            continue;
        };
        composite(&map, &atlas, chunk.cx, chunk.cy, data);
    }
    dirty.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_map::{generate_map, DEFAULT_MAP_SEED};

    fn atlas() -> TerrainAtlas {
        TerrainAtlas::build()
    }

    fn composite_chunk(map: &MapGrid, atlas: &TerrainAtlas, cx: u32, cy: u32) -> Vec<u8> {
        let (w, h) = chunk_extent(map, cx, cy);
        let mut data = vec![0u8; (w * CELL) as usize * (h * CELL) as usize * 4];
        composite(map, atlas, cx, cy, &mut data);
        data
    }

    #[test]
    fn chunks_tile_the_map_without_gaps_or_overlap() {
        let map = generate_map(64, 64, DEFAULT_MAP_SEED);
        let cols = map.width.div_ceil(CHUNK_TILES);
        let rows = map.height.div_ceil(CHUNK_TILES);
        assert_eq!((cols, rows), (4, 4));

        let mut covered = 0u32;
        for cy in 0..rows {
            for cx in 0..cols {
                let (w, h) = chunk_extent(&map, cx, cy);
                covered += w * h;
                // West edge of this chunk butts the east edge of the last one.
                let (center_x, _) = chunk_center(&map, cx, cy);
                let west = center_x - w as f32 * TILE_SIZE * 0.5;
                assert_eq!(west, (cx * CHUNK_TILES) as f32 * TILE_SIZE);
            }
        }
        assert_eq!(covered, map.width * map.height);
    }

    #[test]
    fn chunk_origins_are_whole_texels() {
        let map = generate_map(50, 34, 9);
        let cols = map.width.div_ceil(CHUNK_TILES);
        let rows = map.height.div_ceil(CHUNK_TILES);
        for cy in 0..rows {
            for cx in 0..cols {
                let (x, y) = chunk_center(&map, cx, cy);
                assert_eq!(x.fract(), 0.0, "chunk centre off the texel grid");
                assert_eq!(y.fract(), 0.0, "chunk centre off the texel grid");
            }
        }
    }

    #[test]
    fn ragged_map_sizes_still_cover_every_tile() {
        let map = generate_map(37, 21, 3);
        let mut covered = 0u32;
        for cy in 0..map.height.div_ceil(CHUNK_TILES) {
            for cx in 0..map.width.div_ceil(CHUNK_TILES) {
                let (w, h) = chunk_extent(&map, cx, cy);
                covered += w * h;
            }
        }
        assert_eq!(covered, map.width * map.height);
    }

    #[test]
    fn composited_terrain_is_contiguous() {
        // Every texel of every chunk is opaque: no gaps, no seams, no grid.
        let map = generate_map(64, 64, DEFAULT_MAP_SEED);
        let a = atlas();
        for cy in 0..4 {
            for cx in 0..4 {
                let data = composite_chunk(&map, &a, cx, cy);
                assert!(
                    data.chunks_exact(4).all(|p| p[3] == 255),
                    "chunk ({cx}, {cy}) left a hole in the terrain"
                );
            }
        }
    }

    #[test]
    fn north_is_the_top_of_the_chunk_image() {
        // Sea in the south, mountain in the north: the image must not be flipped.
        let mut map = MapGrid::empty(16, 16, 1);
        for y in 0..16i32 {
            for x in 0..16i32 {
                let tile = map.get_mut(TileCoord { x, y }).unwrap();
                if y < 8 {
                    tile.water = true;
                    tile.height = -6;
                    tile.kind = rail_map::TerrainKind::Water;
                } else {
                    tile.water = false;
                    tile.height = 13;
                    tile.kind = rail_map::TerrainKind::Mountain;
                }
            }
        }
        let a = atlas();
        let data = composite_chunk(&map, &a, 0, 0);
        let width = (16 * CELL) as usize;
        let texel = |x: usize, y: usize| {
            let o = (y * width + x) * 4;
            [data[o], data[o + 1], data[o + 2]]
        };
        let top = texel(width / 2, 4);
        let bottom = texel(width / 2, width - 5);
        let rock = crate::map::terrain::material::rgba(crate::palette::ROCK_D);
        let water = crate::map::terrain::material::rgba(crate::palette::WATER_M);
        assert_eq!(top, [rock[0], rock[1], rock[2]], "north must be at the top");
        assert_eq!(
            bottom,
            [water[0], water[1], water[2]],
            "south must be at the bottom"
        );
    }

    #[test]
    fn one_draw_per_chunk_not_per_tile() {
        // 4096 tiles used to be 4096 sprites; the whole map is now 16 quads.
        let map = generate_map(64, 64, DEFAULT_MAP_SEED);
        let sprites = map.width.div_ceil(CHUNK_TILES) * map.height.div_ceil(CHUNK_TILES);
        assert_eq!(sprites, 16);
        assert!(sprites < map.width * map.height / 100);
    }

    fn test_app(width: u32, height: u32) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<Assets<Image>>();
        app.init_resource::<TerrainDirty>();
        app.insert_resource(generate_map(width, height, DEFAULT_MAP_SEED));
        app.add_systems(Startup, setup_terrain);
        app.add_systems(Update, rebuild_dirty_terrain);
        app
    }

    fn chunk_sprites(app: &mut App) -> Vec<(TerrainChunk, Handle<Image>)> {
        app.world_mut()
            .query::<(&TerrainChunk, &Sprite)>()
            .iter(app.world())
            .map(|(c, s)| (*c, s.image.clone()))
            .collect()
    }

    #[test]
    fn swapping_the_map_rebuilds_the_chunk_grid() {
        let mut app = test_app(32, 32);
        app.update();
        assert_eq!(chunk_sprites(&mut app).len(), 4);

        // A same-size map re-composites in place: same entities, new art.
        let before = chunk_sprites(&mut app);
        app.insert_resource(generate_map(32, 32, 99));
        app.update();
        let after = chunk_sprites(&mut app);
        assert_eq!(after.len(), 4);
        assert_eq!(before[0].1, after[0].1, "same size should reuse its images");

        // A different size rebuilds the grid itself.
        app.insert_resource(generate_map(48, 48, 7));
        app.update();
        let resized = chunk_sprites(&mut app);
        assert_eq!(resized.len(), 9);

        let images = app.world().resource::<Assets<Image>>();
        for (chunk, handle) in &resized {
            let image = images.get(handle).expect("chunk image");
            let (w, h) = chunk_extent(app.world().resource::<MapGrid>(), chunk.cx, chunk.cy);
            assert_eq!(image.width(), w * CELL);
            assert_eq!(image.height(), h * CELL);
        }
    }

    #[test]
    fn an_unchanged_map_does_no_work() {
        let mut app = test_app(32, 32);
        app.update();
        app.update();
        assert!(!app.world().resource::<TerrainDirty>().any);
    }

    #[test]
    fn dirty_marking_covers_the_seam() {
        let mut dirty = TerrainDirty::default();
        dirty.resize(4, 4);
        // A tile on the western seam of chunk 1 dirties chunk 0 as well.
        dirty.mark_tile(TileCoord { x: 16, y: 20 });
        assert!(dirty.is_dirty(1, 1));
        assert!(dirty.is_dirty(0, 1));
        assert!(!dirty.is_dirty(3, 3));

        dirty.clear();
        assert!(!dirty.any);
        dirty.mark_all();
        assert!(dirty.is_dirty(3, 3));
    }

    #[test]
    fn full_map_composite_is_a_startup_cost_not_a_frame_cost() {
        let map = generate_map(64, 64, DEFAULT_MAP_SEED);
        let a = atlas();
        let started = Instant::now();
        for cy in 0..4 {
            for cx in 0..4 {
                let _ = composite_chunk(&map, &a, cx, cy);
            }
        }
        let elapsed = started.elapsed();
        assert!(
            elapsed.as_millis() < 2_000,
            "compositing the whole map took {elapsed:?}"
        );
    }
}
