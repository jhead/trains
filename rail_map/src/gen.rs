//! Seeded procedural terrain generation.

use rail_sim::ids::TileCoord;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::grid::MapGrid;
use crate::portal::Portal;
use crate::tile::{TerrainKind, Tile};
use crate::{EdgeFacing, PortalId};

/// Generate a deterministic map: same `(width, height, seed)` → identical tiles & portals.
pub fn generate_map(width: u32, height: u32, seed: u64) -> MapGrid {
    assert!(width >= 2 && height >= 2, "map must be at least 2×2");

    let mut rng = StdRng::seed_from_u64(seed);
    let mut grid = MapGrid::empty(width, height, seed);

    // Multi-octave value noise → elevation field, then classify.
    let mut heights = vec![0.0f32; (width * height) as usize];
    fill_elevation(&mut heights, width, height, &mut rng);
    classify_tiles(&mut grid, &heights);
    place_edge_portals(&mut grid);

    grid
}

fn fill_elevation(out: &mut [f32], width: u32, height: u32, rng: &mut StdRng) {
    let w = width as usize;
    let h = height as usize;

    // Coarse noise grids at several scales, bilinearly upsampled and summed.
    let octaves: &[(u32, f32)] = &[(8, 1.0), (16, 0.45), (32, 0.2)];
    let mut accum = vec![0.0f32; w * h];

    for &(cells, weight) in octaves {
        let gw = cells.max(2);
        let gh = cells.max(2);
        let mut coarse = vec![0.0f32; (gw * gh) as usize];
        for v in &mut coarse {
            *v = rng.gen_range(-1.0..1.0);
        }
        for y in 0..h {
            for x in 0..w {
                let fx = (x as f32 / (w - 1).max(1) as f32) * (gw - 1) as f32;
                let fy = (y as f32 / (h - 1).max(1) as f32) * (gh - 1) as f32;
                accum[y * w + x] += sample_bilinear(&coarse, gw, gh, fx, fy) * weight;
            }
        }
    }

    // Mild coast bias: pull map edges down so shorelines appear without drowning
    // the interior (MVP needs one large playable landmass for track).
    for y in 0..h {
        for x in 0..w {
            let nx = (x as f32 / (w - 1).max(1) as f32) * 2.0 - 1.0;
            let ny = (y as f32 / (h - 1).max(1) as f32) * 2.0 - 1.0;
            let edge = (nx.abs().max(ny.abs())).powf(2.2);
            accum[y * w + x] -= edge * 0.28;
        }
    }

    // Soft blur for contiguous landmasses.
    for _ in 0..3 {
        blur_inplace(&mut accum, w, h);
    }

    out.copy_from_slice(&accum);
}

fn sample_bilinear(grid: &[f32], gw: u32, gh: u32, fx: f32, fy: f32) -> f32 {
    let x0 = fx.floor() as i32;
    let y0 = fy.floor() as i32;
    let x1 = (x0 + 1).min(gw as i32 - 1);
    let y1 = (y0 + 1).min(gh as i32 - 1);
    let x0 = x0.clamp(0, gw as i32 - 1);
    let y0 = y0.clamp(0, gh as i32 - 1);
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;
    let i = |x: i32, y: i32| grid[y as usize * gw as usize + x as usize];
    let a = i(x0, y0) * (1.0 - tx) + i(x1, y0) * tx;
    let b = i(x0, y1) * (1.0 - tx) + i(x1, y1) * tx;
    a * (1.0 - ty) + b * ty
}

fn blur_inplace(buf: &mut [f32], w: usize, h: usize) {
    let src = buf.to_vec();
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0.0;
            let mut n = 0.0;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let xx = x as i32 + dx;
                    let yy = y as i32 + dy;
                    if xx >= 0 && yy >= 0 && xx < w as i32 && yy < h as i32 {
                        sum += src[yy as usize * w + xx as usize];
                        n += 1.0;
                    }
                }
            }
            buf[y * w + x] = sum / n;
        }
    }
}

fn classify_tiles(grid: &mut MapGrid, heights: &[f32]) {
    let w = grid.width as usize;
    let h = grid.height as usize;
    // Sea level below the field mean so seed 42 (and typical seeds) keep a
    // contiguous interior continent players can rail across.
    let sea = -0.22f32;

    for y in 0..h {
        for x in 0..w {
            let v = heights[y * w + x];
            let water = v < sea;
            // Map continuous height to i8 bands roughly [-8, 12].
            let height = if water {
                ((v - sea) * 20.0).round().clamp(-12.0, -1.0) as i8
            } else {
                ((v - sea) * 18.0).round().clamp(0.0, 16.0) as i8
            };
            let kind = if water {
                TerrainKind::Water
            } else if height <= 1 {
                TerrainKind::Beach
            } else if height <= 5 {
                TerrainKind::Plains
            } else if height <= 10 {
                TerrainKind::Hills
            } else {
                TerrainKind::Mountain
            };
            let tile = Tile {
                height,
                water,
                kind,
            };
            *grid
                .get_mut(TileCoord {
                    x: x as i32,
                    y: y as i32,
                })
                .expect("in-bounds") = tile;
        }
    }
}

/// One closed portal per border tile, facing outward.
fn place_edge_portals(grid: &mut MapGrid) {
    let w = grid.width;
    let h = grid.height;
    let mut next_id = 1u64;
    let mut portals = Vec::new();

    // North edge (y = h-1), facing North — skip corners handled once each.
    for x in 0..w {
        let tile = TileCoord {
            x: x as i32,
            y: (h - 1) as i32,
        };
        portals.push(Portal::closed(PortalId(next_id), EdgeFacing::North, tile));
        next_id += 1;
    }
    // South edge (y = 0)
    for x in 0..w {
        let tile = TileCoord { x: x as i32, y: 0 };
        portals.push(Portal::closed(PortalId(next_id), EdgeFacing::South, tile));
        next_id += 1;
    }
    // West edge (x = 0), excluding corners already covered
    for y in 1..h - 1 {
        let tile = TileCoord { x: 0, y: y as i32 };
        portals.push(Portal::closed(PortalId(next_id), EdgeFacing::West, tile));
        next_id += 1;
    }
    // East edge (x = w-1), excluding corners
    for y in 1..h - 1 {
        let tile = TileCoord {
            x: (w - 1) as i32,
            y: y as i32,
        };
        portals.push(Portal::closed(PortalId(next_id), EdgeFacing::East, tile));
        next_id += 1;
    }

    *grid.portals_mut() = portals;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::{DEFAULT_MAP_HEIGHT, DEFAULT_MAP_SEED, DEFAULT_MAP_WIDTH};

    #[test]
    fn same_seed_same_heights() {
        let a = generate_map(32, 32, 12345);
        let b = generate_map(32, 32, 12345);
        assert_eq!(a.width, b.width);
        assert_eq!(a.height, b.height);
        assert_eq!(a.seed, b.seed);
        for y in 0..32i32 {
            for x in 0..32i32 {
                let c = TileCoord { x, y };
                assert_eq!(a.tile(c).height, b.tile(c).height);
                assert_eq!(a.tile(c).water, b.tile(c).water);
                assert_eq!(a.tile(c).kind, b.tile(c).kind);
            }
        }
    }

    #[test]
    fn different_seed_changes_map() {
        let a = generate_map(24, 24, 1);
        let b = generate_map(24, 24, 2);
        let differs = (0..24i32)
            .flat_map(|y| (0..24i32).map(move |x| TileCoord { x, y }))
            .any(|c| a.tile(c).height != b.tile(c).height);
        assert!(
            differs,
            "expected different seeds to produce different heights"
        );
    }

    #[test]
    fn portals_present_on_all_edges() {
        let map = generate_map(DEFAULT_MAP_WIDTH, DEFAULT_MAP_HEIGHT, DEFAULT_MAP_SEED);
        let expected = (map.width * 2 + (map.height.saturating_sub(2)) * 2) as usize;
        assert_eq!(map.portals().len(), expected);
        assert!(map.portals().iter().all(|p| !p.open));

        // Every border tile has at least one portal.
        for x in 0..map.width as i32 {
            assert!(map.portal_at(TileCoord { x, y: 0 }).is_some());
            assert!(map
                .portal_at(TileCoord {
                    x,
                    y: (map.height - 1) as i32,
                })
                .is_some());
        }
        for y in 0..map.height as i32 {
            assert!(map.portal_at(TileCoord { x: 0, y }).is_some());
            assert!(map
                .portal_at(TileCoord {
                    x: (map.width - 1) as i32,
                    y,
                })
                .is_some());
        }
    }

    #[test]
    fn map_has_land_and_water() {
        let map = generate_map(48, 48, 7);
        let water = map.tiles().iter().filter(|t| t.water).count();
        let land = map.tiles().len() - water;
        assert!(water > 0, "expected some water");
        assert!(land > 0, "expected some land");
    }

    #[test]
    fn default_map_has_large_connected_landmass() {
        let map = generate_map(DEFAULT_MAP_WIDTH, DEFAULT_MAP_HEIGHT, DEFAULT_MAP_SEED);
        let land = map.tiles().iter().filter(|t| !t.water).count();
        let water = map.tiles().len() - land;
        assert!(
            land > water / 2,
            "expected substantial land for MVP play (land={land} water={water})"
        );

        // Largest 4-connected land component should fit spaced stations.
        let mut seen = vec![false; map.tiles().len()];
        let w = map.width as i32;
        let h = map.height as i32;
        let idx = |x: i32, y: i32| (y * w + x) as usize;
        let mut best = 0usize;
        for y in 0..h {
            for x in 0..w {
                let start = idx(x, y);
                if seen[start] || map.tile(TileCoord { x, y }).water {
                    continue;
                }
                let mut stack = vec![(x, y)];
                seen[start] = true;
                let mut size = 0usize;
                while let Some((cx, cy)) = stack.pop() {
                    size += 1;
                    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let nx = cx + dx;
                        let ny = cy + dy;
                        if nx < 0 || ny < 0 || nx >= w || ny >= h {
                            continue;
                        }
                        let ni = idx(nx, ny);
                        if seen[ni] || map.tile(TileCoord { x: nx, y: ny }).water {
                            continue;
                        }
                        seen[ni] = true;
                        stack.push((nx, ny));
                    }
                }
                best = best.max(size);
            }
        }
        assert!(
            best >= 200,
            "largest landmass too small for track loop (best={best})"
        );
    }
}
