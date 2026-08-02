//! Resolving one tile's terrain data into an ordered list of atlas cells.
//!
//! This is the "art is baked when data changes" half of brief 01 §2.5: nothing
//! here runs per frame. A tile becomes a base variant plus, in draw order, its
//! material transitions, its cliff faces and its band contour.

use rail_map::MapGrid;
use rail_sim::ids::TileCoord;

use super::atlas::{
    base_cell, cliff_cell, cliff_corner_cell, sun_lip_cell, terrace_cell, transition_cell, DIR_S,
    DIR_W, TRANSITION_CORNER,
};
use super::material::{
    elevation_band, material_of, shade_for, surface_height, variant_for, Material,
};

/// Height drop that draws a step face.
///
/// Three bands is `rail_sim`'s "steep hillside, cut-and-fill" at 6× cost. Below
/// it the ground is merely rolling, and a face on every rolling edge reads as
/// fencing rather than as landscape.
pub const STEP_DELTA: i16 = 3;

/// Height drop that draws a full banded cliff face.
///
/// Four bands is `MAX_GRADE`: the last delta track may cross at all. A full
/// face on screen therefore means *this is the edge of what you can build*.
pub const CLIFF_DELTA: i16 = 4;

/// Base + 5 transition pieces + 4 cliff faces + 4 cliff corners + 4 contours
/// + 2 sun lips.
pub const MAX_TILE_LAYERS: usize = 20;

/// Neighbour offsets in atlas direction order: N, E, S, W. North is +Y.
const DIR_OFFSETS: [(i32, i32); 4] = [(0, 1), (1, 0), (0, -1), (-1, 0)];
/// Diagonal offsets in quadrant order: NE, SE, SW, NW. Quadrant `q` lies
/// between directions `q` and `q + 1`.
const QUAD_OFFSETS: [(i32, i32); 4] = [(1, 1), (1, -1), (-1, -1), (-1, 1)];

/// Atlas cells for one tile, bottom layer first.
#[derive(Debug, Clone, Copy)]
pub struct TileDraw {
    cells: [u16; MAX_TILE_LAYERS],
    len: u8,
}

impl TileDraw {
    #[inline]
    fn new() -> Self {
        Self {
            cells: [0; MAX_TILE_LAYERS],
            len: 0,
        }
    }

    #[inline]
    fn push(&mut self, cell: usize) {
        debug_assert!(
            (self.len as usize) < MAX_TILE_LAYERS,
            "tile draw list overflow"
        );
        if (self.len as usize) < MAX_TILE_LAYERS {
            self.cells[self.len as usize] = cell as u16;
            self.len += 1;
        }
    }

    #[inline]
    pub fn layers(&self) -> &[u16] {
        &self.cells[..self.len as usize]
    }
}

#[inline]
fn offset(coord: TileCoord, (dx, dy): (i32, i32)) -> TileCoord {
    TileCoord {
        x: coord.x + dx,
        y: coord.y + dy,
    }
}

#[inline]
fn neighbour_material(map: &MapGrid, coord: TileCoord, delta: (i32, i32)) -> Option<Material> {
    map.get(offset(coord, delta)).map(|t| material_of(t.kind))
}

/// Surface height of a neighbour, or `None` off the map — the map edge is not
/// a cliff.
#[inline]
fn neighbour_surface(map: &MapGrid, coord: TileCoord, delta: (i32, i32)) -> Option<i16> {
    map.get(offset(coord, delta))
        .map(|t| surface_height(t.height, t.water))
}

/// The lowest material adjacent to this tile that outranks it — the one whose
/// lip laps in. Diagonals count, so a staircase boundary still resolves.
fn lowest_higher_neighbour(map: &MapGrid, coord: TileCoord, own: Material) -> Option<Material> {
    DIR_OFFSETS
        .iter()
        .chain(QUAD_OFFSETS.iter())
        .filter_map(|&d| neighbour_material(map, coord, d))
        .filter(|&m| m > own)
        .min()
}

/// Resolve every layer this tile draws.
pub fn resolve_tile(map: &MapGrid, coord: TileCoord) -> TileDraw {
    let mut draw = TileDraw::new();
    let Some(tile) = map.get(coord) else {
        return draw;
    };
    let material = material_of(tile.kind);
    let height = surface_height(tile.height, tile.water);
    let shade = shade_for(tile.kind, tile.height);

    draw.push(base_cell(material, shade, variant_for(coord)));

    // Material transitions. The higher material always laps onto the lower, so
    // each boundary is drawn exactly once, on the low tile.
    if let Some(high) = lowest_higher_neighbour(map, coord, material) {
        // The sea always meets the shore with a sand lip and a foam line, even
        // where the land behind it is rock (brief 01 §6.2.1).
        let boundary = if material == Material::Water {
            0
        } else {
            high.index() - 1
        };
        let mut mask = 0usize;
        for (dir, &delta) in DIR_OFFSETS.iter().enumerate() {
            if neighbour_material(map, coord, delta).is_some_and(|m| m >= high) {
                mask |= 1 << dir;
            }
        }
        if mask != 0 {
            draw.push(transition_cell(boundary, mask));
        }
        for (quadrant, &delta) in QUAD_OFFSETS.iter().enumerate() {
            let flanked = mask & (1 << quadrant) != 0 || mask & (1 << ((quadrant + 1) % 4)) != 0;
            if !flanked && neighbour_material(map, coord, delta).is_some_and(|m| m >= high) {
                draw.push(transition_cell(boundary, TRANSITION_CORNER + quadrant));
            }
        }
    }

    if tile.water {
        return draw;
    }

    // Cliff faces. This is what turns a heightmap into a landscape the player
    // can route around: without them the elevation data may as well not exist.
    let mut faced = [false; 4];
    for (dir, &delta) in DIR_OFFSETS.iter().enumerate() {
        let Some(neighbour) = neighbour_surface(map, coord, delta) else {
            continue;
        };
        let drop = height - neighbour;
        if drop >= CLIFF_DELTA {
            draw.push(cliff_cell(dir, 1));
            faced[dir] = true;
        } else if drop >= STEP_DELTA {
            draw.push(cliff_cell(dir, 0));
            faced[dir] = true;
        }
    }
    // A drop that is diagonal only would otherwise leave a notch in a ridge —
    // but only when the ground is genuinely falling away in that quadrant. A
    // lone pit off one corner is a hole, not a corner, and filling it puts a
    // rock blob in the middle of a field.
    for (quadrant, &delta) in QUAD_OFFSETS.iter().enumerate() {
        let flanks = [quadrant, (quadrant + 1) % 4];
        if flanks.iter().any(|&d| faced[d]) {
            continue;
        }
        let falling = flanks.iter().any(|&d| {
            neighbour_surface(map, coord, DIR_OFFSETS[d]).is_some_and(|h| height - h >= 1)
        });
        if falling
            && neighbour_surface(map, coord, delta).is_some_and(|h| height - h >= CLIFF_DELTA)
        {
            draw.push(cliff_corner_cell(quadrant));
        }
    }

    // Band steps too small to be a face still have to read, or gentle ground is
    // a soft gradient that communicates nothing (brief 02 §2.3). They draw as a
    // pair: a contour shadow at the foot on this tile, and a lit lip on the
    // crest when this tile is the high side of a sunlit edge.
    for (dir, &delta) in DIR_OFFSETS.iter().enumerate() {
        let Some(neighbour) = neighbour_surface(map, coord, delta) else {
            continue;
        };
        let rise = neighbour - height;
        // A face on the tile above needs a shadow at its foot, or it floats.
        if rise >= STEP_DELTA {
            draw.push(terrace_cell(dir));
            continue;
        }
        if !steps_visibly(map, coord, delta, material, tile.height, shade) {
            continue;
        }
        if rise >= 1 {
            draw.push(terrace_cell(dir));
        } else if (1..STEP_DELTA).contains(&-rise) && (dir == DIR_S || dir == DIR_W) {
            draw.push(sun_lip_cell(material, shade, dir));
        }
    }

    draw
}

/// Whether a same-material neighbour sits on a different visible step — a
/// different ramp shade, or a different elevation band. Boundaries between
/// *materials* are already drawn by the transition pass, so they are excluded.
fn steps_visibly(
    map: &MapGrid,
    coord: TileCoord,
    delta: (i32, i32),
    material: Material,
    height: i8,
    shade: usize,
) -> bool {
    let Some(neighbour) = map.get(offset(coord, delta)) else {
        return false;
    };
    if material_of(neighbour.kind) != material {
        return false;
    }
    shade_for(neighbour.kind, neighbour.height) != shade
        || elevation_band(neighbour.height as i16) != elevation_band(height as i16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_map::{generate_map, TerrainKind, Tile, DEFAULT_MAP_SEED};

    use crate::map::terrain::atlas::{
        CLIFF_BASE, CLIFF_CORNER_BASE, SUN_LIP_BASE, TERRACE_BASE, TRANSITION_BASE,
    };

    fn grid(w: u32, h: u32, f: impl Fn(i32, i32) -> Tile) -> MapGrid {
        let mut map = MapGrid::empty(w, h, 1);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                *map.get_mut(TileCoord { x, y }).unwrap() = f(x, y);
            }
        }
        map
    }

    fn land(height: i8, kind: TerrainKind) -> Tile {
        Tile {
            height,
            water: false,
            kind,
        }
    }

    fn sea(height: i8) -> Tile {
        Tile {
            height,
            water: true,
            kind: TerrainKind::Water,
        }
    }

    fn layers(map: &MapGrid, x: i32, y: i32) -> Vec<u16> {
        resolve_tile(map, TileCoord { x, y }).layers().to_vec()
    }

    fn count_in(layers: &[u16], range: std::ops::Range<usize>) -> usize {
        layers
            .iter()
            .filter(|c| range.contains(&(**c as usize)))
            .count()
    }

    #[test]
    fn every_tile_draws_a_base_layer() {
        let map = grid(4, 4, |_, _| land(2, TerrainKind::Plains));
        for y in 0..4 {
            for x in 0..4 {
                let l = layers(&map, x, y);
                assert!(!l.is_empty());
                assert!((l[0] as usize) < TRANSITION_BASE, "base must draw first");
            }
        }
    }

    #[test]
    fn a_flat_field_draws_nothing_but_its_base() {
        let map = grid(5, 5, |_, _| land(2, TerrainKind::Plains));
        assert_eq!(layers(&map, 2, 2).len(), 1);
    }

    #[test]
    fn shoreline_draws_the_sand_and_foam_set_even_against_rock() {
        // Sea in the west, a rock wall in the east: the water tile still gets
        // the coastline piece, not a rock lip.
        let map = grid(3, 1, |x, _| {
            if x == 0 {
                sea(-4)
            } else {
                land(12, TerrainKind::Mountain)
            }
        });
        let l = layers(&map, 0, 0);
        let transitions: Vec<_> = l
            .iter()
            .filter(|c| (TRANSITION_BASE..CLIFF_BASE).contains(&(**c as usize)))
            .collect();
        assert_eq!(transitions.len(), 1);
        let piece = *transitions[0] as usize - TRANSITION_BASE;
        assert!(piece < 20, "water must use boundary 0 (sand + foam)");
    }

    #[test]
    fn a_ridge_draws_a_cliff_face_on_its_high_side() {
        // Low plain in the south, high ground in the north.
        let map = grid(3, 3, |_, y| {
            if y >= 2 {
                land(9, TerrainKind::Hills)
            } else {
                land(3, TerrainKind::Plains)
            }
        });
        let high = layers(&map, 1, 2);
        assert!(
            count_in(&high, CLIFF_BASE..CLIFF_CORNER_BASE) >= 1,
            "high side of a 6-band drop must draw a face"
        );
        let low = layers(&map, 1, 1);
        assert_eq!(
            count_in(&low, CLIFF_BASE..CLIFF_CORNER_BASE),
            0,
            "the low tile is not the one with the face"
        );
    }

    #[test]
    fn a_gentle_slope_draws_no_cliff() {
        let map = grid(3, 3, |_, y| land(2 + y as i8, TerrainKind::Plains));
        for y in 0..3 {
            assert_eq!(
                count_in(&layers(&map, 1, y), CLIFF_BASE..CLIFF_CORNER_BASE),
                0
            );
        }
    }

    #[test]
    fn water_never_draws_a_cliff_from_its_bed() {
        // A shallow beach beside a deep channel is a shoreline, not a cliff.
        let map = grid(3, 1, |x, _| {
            if x == 0 {
                sea(-11)
            } else {
                land(1, TerrainKind::Beach)
            }
        });
        for x in 0..3 {
            assert_eq!(count_in(&layers(&map, x, 0), CLIFF_BASE..TERRACE_BASE), 0);
        }
    }

    #[test]
    fn a_diagonal_drop_fills_its_corner() {
        // A staircase ridge: ground eases off east and falls away hard to the
        // north-east, with no orthogonal drop steep enough to draw a face.
        let map = grid(3, 3, |x, y| match (x, y) {
            (2, 2) => land(2, TerrainKind::Plains),
            (2, _) => land(7, TerrainKind::Hills),
            _ => land(8, TerrainKind::Hills),
        });
        let l = layers(&map, 1, 1);
        assert_eq!(count_in(&l, CLIFF_BASE..CLIFF_CORNER_BASE), 0);
        assert_eq!(count_in(&l, CLIFF_CORNER_BASE..TERRACE_BASE), 1);
    }

    #[test]
    fn a_lone_pit_off_one_corner_is_not_a_cliff_corner() {
        let map = grid(3, 3, |x, y| {
            if x == 2 && y == 2 {
                land(2, TerrainKind::Plains)
            } else {
                land(8, TerrainKind::Hills)
            }
        });
        assert_eq!(
            count_in(&layers(&map, 1, 1), CLIFF_CORNER_BASE..TERRACE_BASE),
            0
        );
    }

    #[test]
    fn a_band_step_draws_a_shadow_below_and_a_lit_lip_above() {
        // A single-band rise to the north, crossing an elevation band boundary.
        let map = grid(3, 3, |_, y| {
            land(if y >= 2 { 3 } else { 2 }, TerrainKind::Plains)
        });
        let low = layers(&map, 1, 1);
        assert_eq!(count_in(&low, TERRACE_BASE..SUN_LIP_BASE), 1);
        assert_eq!(count_in(&low, SUN_LIP_BASE..usize::MAX), 0);

        let high = layers(&map, 1, 2);
        assert_eq!(count_in(&high, TERRACE_BASE..SUN_LIP_BASE), 0);
        assert_eq!(
            count_in(&high, SUN_LIP_BASE..usize::MAX),
            1,
            "the crest of a step must catch the sun"
        );
    }

    #[test]
    fn flat_ground_is_never_lit() {
        let map = grid(3, 3, |_, _| land(4, TerrainKind::Plains));
        assert_eq!(count_in(&layers(&map, 1, 1), TERRACE_BASE..usize::MAX), 0);
    }

    #[test]
    fn only_the_sunlit_edges_get_a_lip() {
        // Ground falling away north and east catches nothing; the shadow line
        // on the neighbour is what draws that step.
        let map = grid(3, 3, |x, y| {
            land(if x == 2 || y == 2 { 2 } else { 4 }, TerrainKind::Plains)
        });
        assert_eq!(count_in(&layers(&map, 1, 1), SUN_LIP_BASE..usize::MAX), 0);

        let map = grid(3, 3, |x, y| {
            land(if x == 0 || y == 0 { 2 } else { 4 }, TerrainKind::Plains)
        });
        assert_eq!(count_in(&layers(&map, 1, 1), SUN_LIP_BASE..usize::MAX), 2);
    }

    #[test]
    fn draw_lists_never_overflow_on_a_real_map() {
        let map = generate_map(64, 64, DEFAULT_MAP_SEED);
        for y in 0..map.height as i32 {
            for x in 0..map.width as i32 {
                let l = resolve_tile(&map, TileCoord { x, y });
                assert!(l.layers().len() <= MAX_TILE_LAYERS);
                assert!(!l.layers().is_empty());
            }
        }
    }

    #[test]
    fn a_real_map_is_legibly_stepped_not_flat_colour() {
        let map = generate_map(64, 64, DEFAULT_MAP_SEED);
        let mut cliffs = 0usize;
        let mut contours = 0usize;
        let mut transitions = 0usize;
        for y in 0..map.height as i32 {
            for x in 0..map.width as i32 {
                let l = resolve_tile(&map, TileCoord { x, y });
                let l = l.layers();
                cliffs += count_in(l, CLIFF_BASE..TERRACE_BASE);
                contours += count_in(l, TERRACE_BASE..usize::MAX);
                transitions += count_in(l, TRANSITION_BASE..CLIFF_BASE);
            }
        }
        let tiles = (map.width * map.height) as usize;
        assert!(
            transitions > tiles / 20,
            "material boundaries barely drawn ({transitions} pieces over {tiles} tiles)"
        );
        assert!(
            cliffs + contours > tiles / 10,
            "elevation is invisible: {cliffs} faces + {contours} contours over {tiles} tiles"
        );
    }
}
