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
    /// Signature of the terrain the chunks were last composited from.
    ///
    /// Bevy change detection answers *did anybody write this resource*, which is
    /// not the same question as *did the terrain move*. Any system that borrows
    /// [`MapGrid`] mutably for a non-terrain reason — the border slice's portal
    /// mirror did, once per frame — would otherwise trigger a full-map
    /// re-composite plus a sixteen-megabyte texture re-upload every frame.
    /// Hashing the tiles is cheap next to compositing them, so the flag is the
    /// gate and this is the truth.
    signature: Option<u64>,
}

/// FNV-1a over every tile's drawn properties.
///
/// Only what [`super::autotile::resolve_tile`] reads: elevation, water and
/// material. Portals, features and the seed are deliberately absent — none of
/// them changes a texel, so none of them should cost a rebuild.
fn terrain_signature(map: &MapGrid) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |byte: u8| {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    eat(map.width as u8);
    eat((map.width >> 8) as u8);
    eat(map.height as u8);
    eat((map.height >> 8) as u8);
    for tile in map.tiles() {
        eat(tile.height as u8);
        eat(tile.water as u8);
        eat(tile.kind as u8);
    }
    hash
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
    // These chunks are composited from this terrain, so record what they were
    // built from — the first Update must not redo the startup composite.
    dirty.signature = Some(terrain_signature(map));

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
/// its own: the grid's change tick brings us to look, the terrain signature
/// confirms the tiles really moved, and a grid of a different size rebuilds the
/// chunk entities too. Nothing else has to re-register terrain when the world is
/// swapped.
///
/// The signature is what makes this safe to leave in a hot loop. A write to
/// `MapGrid` for a reason that has nothing to do with terrain must not cost a
/// full re-composite and a full texture re-upload; see [`TerrainDirty`].
pub fn rebuild_dirty_terrain(
    mut commands: Commands,
    map: Res<MapGrid>,
    atlas: Option<Res<TerrainAtlas>>,
    mut dirty: ResMut<TerrainDirty>,
    mut images: ResMut<Assets<Image>>,
    chunks: Query<(Entity, &TerrainChunk, &Sprite)>,
) {
    let _perf = crate::overlays::perf::scope("rebuild_dirty_terrain");
    let mut swapped = false;
    if map.is_changed() {
        let signature = terrain_signature(&map);
        // `is_added` is the startup composite, which already ran; record its
        // signature so the first ordinary frame does not redo it.
        swapped = !map.is_added() && dirty.signature != Some(signature);
        dirty.signature = Some(signature);
        if swapped {
            dirty.mark_all();
        }
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
        use crate::map::terrain::material::{rgba, Material, SHADES};
        // Sampled by ramp rather than by exact colour: a base tile is textured,
        // so which of a material's steps a given texel lands on is a world hash's
        // business. Which *ramp* it is drawn from is the contract.
        let ramp = |m: Material| {
            (0..SHADES)
                .map(|s| {
                    let c = rgba(m.step(s));
                    [c[0], c[1], c[2]]
                })
                .collect::<Vec<_>>()
        };
        let top = texel(width / 2, 4);
        let bottom = texel(width / 2, width - 5);
        assert!(
            ramp(Material::Rock).contains(&top),
            "north must be at the top, in rock: {top:?}"
        );
        assert!(
            ramp(Material::Water).contains(&bottom),
            "south must be at the bottom, in water: {bottom:?}"
        );
    }

    /// A strip of every elevation band, composited exactly as the game does it,
    /// and measured. The colour table having a monotonic ladder in it is not the
    /// same claim as the *screen* having one — texture, transitions, contours and
    /// faces all land on the same tiles — so this pins the whole pipeline.
    #[test]
    fn a_composited_band_strip_climbs_in_luminance() {
        use crate::map::terrain::material::texel_lightness;
        use rail_map::{TerrainKind, Tile};

        // Two tiles per band, west to east, at the heights `rail_map` bands
        // elevation to and the kinds it gives them (`gen.rs`).
        const LADDER: [(TerrainKind, i8); 6] = [
            (TerrainKind::Plains, 0),
            (TerrainKind::Plains, 4),
            (TerrainKind::Hills, 7),
            (TerrainKind::Hills, 10),
            (TerrainKind::Mountain, 13),
            (TerrainKind::Mountain, 16),
        ];
        const BAND_TILES: u32 = 2;
        let tiles_w = LADDER.len() as u32 * BAND_TILES;

        let mut map = MapGrid::empty(tiles_w, 3, 1);
        for y in 0..3i32 {
            for x in 0..tiles_w as i32 {
                let (kind, height) = LADDER[x as usize / BAND_TILES as usize];
                *map.get_mut(TileCoord { x, y }).unwrap() = Tile {
                    height,
                    water: false,
                    kind,
                };
            }
        }

        let a = atlas();
        let data = composite_chunk(&map, &a, 0, 0);
        let width = (tiles_w * CELL) as usize;
        let height = 3 * CELL as usize;
        let texel = |x: usize, y: usize| {
            let o = (y * width + x) * 4;
            [data[o], data[o + 1], data[o + 2], data[o + 3]]
        };

        // Sample the middle of each band only. Faces run at most 9 texels in
        // from an east or west edge and material lips at most 7, so a 12-texel
        // margin leaves nothing but the band's own fill and texture.
        const MARGIN: usize = 12;
        let band_width = (BAND_TILES * CELL) as usize;
        let mean: Vec<f32> = (0..LADDER.len())
            .map(|band| {
                let x0 = band * band_width + MARGIN;
                let x1 = (band + 1) * band_width - MARGIN;
                let mut total = 0.0;
                let mut n = 0.0;
                for y in 0..height {
                    for x in x0..x1 {
                        total += texel_lightness(texel(x, y));
                        n += 1.0;
                    }
                }
                total / n
            })
            .collect();

        for band in 1..5 {
            assert!(
                mean[band] > mean[band - 1],
                "band {band} does not read as higher than band {}: {mean:?}",
                band - 1
            );
        }
        // One rung is tight: `GRASS_M` → `HILL_M` is 0.85 L* because the grass
        // and hill ramps sit on the same three lightness tiers. That rung is
        // also the plains/hills *material* boundary, which brief 01 §6.2.1 draws
        // as an authored transition with its own lip and shadow line — so it is
        // the one step in the ladder that does not have to carry itself on value
        // alone. Every rung inside a material clears a full ramp step.
        for band in 1..5 {
            let same_material = LADDER[band].0 == LADDER[band - 1].0;
            assert!(
                !same_material || mean[band] - mean[band - 1] > 8.0,
                "band {band} steps within its material but barely changes value: {mean:?}"
            );
        }
        assert!(
            mean[5] >= mean[4] - 0.5,
            "the wall must not read as lower than the ground below it: {mean:?}"
        );

        // The wall's own read: its west face carries the brightest terrain
        // texels on the whole strip, and nothing that light is a fill.
        let crest = crate::map::terrain::material::rgba(crate::palette::SNOW);
        let crest_lightness = texel_lightness(crest);
        let mut crest_texels = 0usize;
        for y in 0..height {
            for x in 0..width {
                let px = texel(x, y);
                if px == crest {
                    crest_texels += 1;
                    // The snow is on the wall's face, not out in the field.
                    assert!(
                        x >= 5 * band_width - MARGIN && x < 5 * band_width + MARGIN,
                        "snow at ({x}, {y}) is nowhere near a wall face"
                    );
                } else {
                    assert!(
                        texel_lightness(px) < crest_lightness,
                        "something on the strip outshines the wall's crest at ({x}, {y})"
                    );
                }
            }
        }
        assert!(crest_texels > 0, "the wall was drawn without a lit crest");
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

    /// Writing to `MapGrid` for a reason that is not terrain must cost nothing.
    ///
    /// This is the regression that made the game unplayable. Bevy change
    /// detection answers *did anybody write this resource*, and the border
    /// slice's portal mirror wrote a portal record every frame. Keyed on that
    /// flag alone, this system re-composited all sixteen chunks and re-uploaded
    /// their textures every frame — 79.6 ms of a 118 ms debug frame, and about
    /// 1.7 ms plus the upload in release. Portals, features and the seed do not
    /// change a single texel, so none of them may cost a rebuild.
    #[test]
    fn a_non_terrain_write_to_the_map_does_not_recomposite() {
        let mut app = test_app(32, 32);
        app.update();

        let before: Vec<Handle<Image>> = chunk_sprites(&mut app).into_iter().map(|c| c.1).collect();
        for _ in 0..8 {
            // Touch the grid the way the portal mirror does: a mutable borrow
            // that leaves every tile exactly as it was.
            let mut map = app.world_mut().resource_mut::<MapGrid>();
            map.close_portals_facing(rail_map::EdgeFacing::North);
            app.update();
            assert!(
                !app.world().resource::<TerrainDirty>().any,
                "a portal write dirtied the terrain"
            );
        }
        let after: Vec<Handle<Image>> = chunk_sprites(&mut app).into_iter().map(|c| c.1).collect();
        assert_eq!(before, after, "chunk images must not have been rebuilt");
    }

    /// The other half of the contract: terrain that really moves still rebuilds.
    #[test]
    fn an_edited_tile_still_rebuilds_the_terrain() {
        let mut app = test_app(32, 32);
        app.update();
        assert!(!app.world().resource::<TerrainDirty>().any);

        {
            let mut map = app.world_mut().resource_mut::<MapGrid>();
            let tile = map.get_mut(TileCoord { x: 4, y: 4 }).expect("in bounds");
            tile.height = tile.height.saturating_add(4);
            tile.kind = rail_map::TerrainKind::Mountain;
        }
        app.update();
        // The rebuild ran and cleared itself in the same frame.
        assert!(!app.world().resource::<TerrainDirty>().any);
        assert_eq!(
            app.world().resource::<TerrainDirty>().signature,
            Some(terrain_signature(app.world().resource::<MapGrid>())),
            "the composited art must match the terrain it was built from"
        );
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
