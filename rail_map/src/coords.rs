//! World ↔ tile conversion helpers — **isometric evaluation prototype**.
//!
//! Tile `(0, 0)` is the south-west corner of the map. On `main` this module maps
//! a tile to an axis-aligned square: world `(x + 0.5, y + 0.5) * TILE_SIZE`, +X
//! east and +Y north, and the screen is the ground plane seen from directly
//! above.
//!
//! On this branch "world space" **is** iso screen space. Bevy's 2D world already
//! is screen space up to the camera, so reprojecting here reprojects everything
//! that ever asked where a tile is — track, stations, trains, peeps, decals,
//! alerts, audio panning and the cursor — without any of them knowing.
//!
//! # The projection
//!
//! Classic 2:1 dimetric. Take the old axis-aligned ground plane `(gx, gy)`:
//!
//! ```text
//! sx = gx - gy
//! sy = (gx + gy) / 2 + height · ISO_LIFT
//! ```
//!
//! It is *linear* in `(gx, gy)`, which is the property the rest of the game
//! leans on without asking: anything that interpolates between two tile centres
//! — a train between two track pieces, a peep walking a lot — interpolates in
//! projected space and lands exactly where projecting the interpolation would
//! have put it. Only the height term is non-linear, and it is a per-tile lookup.
//!
//! A tile is therefore a 64 × 32 diamond. Bevy's +Y is up, so the lift **adds**
//! (the brief's `- h·lift` assumes a y-down screen). +X runs up-and-right, +Y
//! runs up-and-left: the camera sits over the map's south-west corner, so the
//! near corner of a tile is its south-west one and the two faces a cliff shows
//! are its **south** and **west** ones.
//!
//! # The height field is a global, and that is the prototype tax
//!
//! [`tile_to_world`] takes a [`TileCoord`] and nothing else — every one of its
//! ~40 call sites across the presentation crate assumes that. Threading a
//! `&MapGrid` through all of them to look up one `i8` is most of the diff and
//! none of the evaluation, so the lift comes from a process-global set once when
//! a map is installed ([`set_iso_heights`]). Unset, every tile sits at zero and
//! the projection is flat — which is what sim-side and generator tests get.

use std::sync::RwLock;

use rail_sim::ids::TileCoord;

use crate::grid::MapGrid;

/// Side length of one map tile in *ground-plane* units.
pub const TILE_SIZE: f32 = 32.0;

/// Screen width of one tile diamond.
pub const ISO_TILE_W: f32 = TILE_SIZE * 2.0;
/// Screen height of one tile diamond.
pub const ISO_TILE_H: f32 = TILE_SIZE;

/// Screen pixels one unit of terrain height lifts a tile.
///
/// `rail_map` bands elevation every 3 units, so a band step is 12 px — three
/// eighths of a tile's screen height, which is a terrace you can see from across
/// the map without a mountain leaving the screen. The tallest wall on a default
/// map (height 18) stands 72 px, a little over two tiles.
pub const ISO_LIFT: f32 = 4.0;

// ── The projection itself ──────────────────────────────────────────────────

/// Ground plane → iso screen. Pure, no height.
#[inline]
pub fn project(gx: f32, gy: f32) -> (f32, f32) {
    (gx - gy, (gx + gy) * 0.5)
}

/// Iso screen → ground plane. Exact inverse of [`project`].
#[inline]
pub fn unproject(sx: f32, sy: f32) -> (f32, f32) {
    (sy + sx * 0.5, sy - sx * 0.5)
}

/// Ground-plane centre of a tile, before projection.
#[inline]
pub fn tile_to_ground(coord: TileCoord) -> (f32, f32) {
    (
        (coord.x as f32 + 0.5) * TILE_SIZE,
        (coord.y as f32 + 0.5) * TILE_SIZE,
    )
}

// ── The height field ───────────────────────────────────────────────────────

/// Per-tile surface heights, in map order, for the lift term.
#[derive(Default)]
struct HeightField {
    width: i32,
    height: i32,
    /// Surface height per tile: the tile's own height, or 0 where it is water —
    /// water reads as its surface, not its bed, so a river is a flat ribbon
    /// rather than a nine-band canyon.
    surface: Vec<i8>,
    lo: i8,
    hi: i8,
}

impl HeightField {
    #[inline]
    fn at(&self, coord: TileCoord) -> Option<i8> {
        if coord.x < 0 || coord.y < 0 || coord.x >= self.width || coord.y >= self.height {
            return None;
        }
        self.surface
            .get((coord.y * self.width + coord.x) as usize)
            .copied()
    }
}

static HEIGHTS: RwLock<Option<HeightField>> = RwLock::new(None);

/// Surface height a tile presents: water is its surface, land is its height.
#[inline]
pub fn surface_height_of(tile: &crate::tile::Tile) -> i8 {
    if tile.water {
        0
    } else {
        tile.height
    }
}

/// Install `map`'s heights as the lift the projection uses.
///
/// Called whenever a map is installed or edited. Cheap — one `i8` per tile.
pub fn set_iso_heights(map: &MapGrid) {
    let surface: Vec<i8> = map.tiles().iter().map(surface_height_of).collect();
    let lo = surface.iter().copied().min().unwrap_or(0);
    let hi = surface.iter().copied().max().unwrap_or(0);
    let field = HeightField {
        width: map.width as i32,
        height: map.height as i32,
        surface,
        lo,
        hi,
    };
    if let Ok(mut slot) = HEIGHTS.write() {
        *slot = Some(field);
    }
}

/// Drop the height field: every tile falls back to sea level.
pub fn clear_iso_heights() {
    if let Ok(mut slot) = HEIGHTS.write() {
        *slot = None;
    }
}

/// Surface height at `coord`, or 0 off-map / with no field installed.
#[inline]
pub fn tile_height(coord: TileCoord) -> i8 {
    HEIGHTS
        .read()
        .ok()
        .and_then(|f| f.as_ref().and_then(|f| f.at(coord)))
        .unwrap_or(0)
}

/// Screen pixels a tile is lifted by its own elevation.
#[inline]
pub fn tile_lift(coord: TileCoord) -> f32 {
    tile_height(coord) as f32 * ISO_LIFT
}

// ── The contract the rest of the game uses ─────────────────────────────────

/// Screen-space centre of a tile's diamond, elevation included.
#[inline]
pub fn tile_to_world(coord: TileCoord) -> (f32, f32) {
    let (gx, gy) = tile_to_ground(coord);
    let (sx, sy) = project(gx, gy);
    (sx, sy + tile_lift(coord))
}

/// Screen-space centre of a tile's diamond **on the ground plane**, ignoring its
/// elevation. This is what depth sorting keys on: a lifted mountain must not
/// sort in front of the tile that is genuinely nearer the camera.
#[inline]
pub fn tile_to_world_flat(coord: TileCoord) -> (f32, f32) {
    let (gx, gy) = tile_to_ground(coord);
    project(gx, gy)
}

/// Tile under a screen-space point.
///
/// Elevation makes this a search rather than an inverse: a lifted tile covers
/// the ground behind it, so the answer is the *topmost* tile whose diamond the
/// point falls in. Lift levels are tried high to low, and a higher lift always
/// resolves to a smaller `x + y` — a tile nearer the camera, drawn later — so
/// the first level that explains the point is the one the player can see.
///
/// With no height field installed this collapses to the flat inverse.
pub fn world_to_tile(sx: f32, sy: f32) -> TileCoord {
    let flat = |sy: f32| {
        let (gx, gy) = unproject(sx, sy);
        TileCoord {
            x: (gx / TILE_SIZE).floor() as i32,
            y: (gy / TILE_SIZE).floor() as i32,
        }
    };

    if let Ok(guard) = HEIGHTS.read() {
        if let Some(field) = guard.as_ref() {
            for level in (field.lo..=field.hi).rev() {
                let candidate = flat(sy - level as f32 * ISO_LIFT);
                if field.at(candidate) == Some(level) {
                    return candidate;
                }
            }
            // Off the map (or in a gap the levels do not explain): answer on the
            // ground plane so the caller still gets a sensible out-of-bounds
            // coordinate to reject.
            return flat(sy);
        }
    }
    flat(sy)
}

/// Screen-space centre of the whole map (useful for framing the camera).
pub fn map_center_world(width: u32, height: u32) -> (f32, f32) {
    project(
        width as f32 * TILE_SIZE * 0.5,
        height as f32 * TILE_SIZE * 0.5,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tile::TerrainKind;

    /// Serialises the tests that install a height field: it is process-global.
    static FIELD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn flat_map(w: u32, h: u32) -> MapGrid {
        MapGrid::empty(w, h, 1)
    }

    #[test]
    fn a_tile_is_a_two_to_one_diamond() {
        let _g = FIELD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_iso_heights();
        let origin = tile_to_world(TileCoord { x: 0, y: 0 });
        let east = tile_to_world(TileCoord { x: 1, y: 0 });
        let north = tile_to_world(TileCoord { x: 0, y: 1 });

        // One step east is half a diamond right and half a diamond up; one step
        // north is the mirror. That is the 2:1 dimetric grid.
        assert_eq!(east.0 - origin.0, ISO_TILE_W * 0.5);
        assert_eq!(east.1 - origin.1, ISO_TILE_H * 0.5);
        assert_eq!(north.0 - origin.0, -ISO_TILE_W * 0.5);
        assert_eq!(north.1 - origin.1, ISO_TILE_H * 0.5);
    }

    #[test]
    fn project_and_unproject_round_trip() {
        for &(gx, gy) in &[(0.0, 0.0), (17.5, -3.25), (2048.0, 96.0), (-40.0, 512.0)] {
            let (sx, sy) = project(gx, gy);
            let (bx, by) = unproject(sx, sy);
            assert!((bx - gx).abs() < 1e-3, "{gx} -> {bx}");
            assert!((by - gy).abs() < 1e-3, "{gy} -> {by}");
        }
    }

    #[test]
    fn flat_ground_round_trips_tile_centres() {
        let _g = FIELD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_iso_heights();
        for y in -3..40 {
            for x in -3..40 {
                let c = TileCoord { x, y };
                let (sx, sy) = tile_to_world(c);
                assert_eq!(world_to_tile(sx, sy), c, "centre of {c:?} did not invert");
            }
        }
    }

    /// Every point inside a diamond has to answer with that diamond, or the
    /// player cannot lay track by pointing at the ground.
    #[test]
    fn every_point_in_a_diamond_picks_its_own_tile() {
        let _g = FIELD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_iso_heights();
        let c = TileCoord { x: 5, y: 9 };
        let (cx, cy) = tile_to_world(c);
        // Walk the diamond's interior: |dx| / 2 + |dy| < 16 keeps clear of the
        // edges, where a half-texel either way is genuinely the next tile.
        let mut checked = 0;
        for dy in -15..=15 {
            for dx in -31..=31 {
                if (dx as f32).abs() * 0.5 + (dy as f32).abs() >= 15.0 {
                    continue;
                }
                assert_eq!(
                    world_to_tile(cx + dx as f32, cy + dy as f32),
                    c,
                    "({dx}, {dy}) off the centre of {c:?} picked the wrong tile"
                );
                checked += 1;
            }
        }
        assert!(checked > 400, "the diamond sweep barely covered anything");
    }

    #[test]
    fn height_lifts_a_tile_and_picking_follows_it() {
        let _g = FIELD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut map = flat_map(16, 16);
        let peak = TileCoord { x: 8, y: 8 };
        map.get_mut(peak).unwrap().height = 12;
        map.get_mut(peak).unwrap().kind = TerrainKind::Mountain;
        set_iso_heights(&map);

        let flat_pos = tile_to_world_flat(peak);
        let lifted = tile_to_world(peak);
        assert_eq!(lifted.0, flat_pos.0, "lift must not move a tile sideways");
        assert_eq!(lifted.1 - flat_pos.1, 12.0 * ISO_LIFT);

        // The cursor over the lifted diamond finds the peak, not the ground
        // behind it.
        assert_eq!(world_to_tile(lifted.0, lifted.1), peak);
        // ... and the neighbours, which are still at sea level, still answer.
        for n in [
            TileCoord { x: 7, y: 8 },
            TileCoord { x: 9, y: 8 },
            TileCoord { x: 8, y: 7 },
        ] {
            let (nx, ny) = tile_to_world(n);
            assert_eq!(world_to_tile(nx, ny), n, "{n:?} was swallowed by the peak");
        }
        clear_iso_heights();
    }

    #[test]
    fn water_sits_at_its_surface_not_its_bed() {
        let _g = FIELD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut map = flat_map(8, 8);
        let wet = TileCoord { x: 2, y: 2 };
        {
            let t = map.get_mut(wet).unwrap();
            t.water = true;
            t.height = -6;
            t.kind = TerrainKind::Water;
        }
        set_iso_heights(&map);
        assert_eq!(tile_lift(wet), 0.0);
        assert_eq!(tile_to_world(wet), tile_to_world_flat(wet));
        clear_iso_heights();
    }

    /// Depth sorting keys on the *flat* position: a mountain must not sort in
    /// front of the tile in front of it just because it is tall.
    #[test]
    fn the_sort_key_ignores_elevation() {
        let _g = FIELD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut map = flat_map(8, 8);
        map.get_mut(TileCoord { x: 4, y: 4 }).unwrap().height = 18;
        set_iso_heights(&map);

        let near = TileCoord { x: 4, y: 3 }; // one step south — nearer the camera
        let far = TileCoord { x: 4, y: 4 }; // the peak, behind it
        assert!(
            sort_depth(near) < sort_depth(far),
            "the southern tile must sort in front of the peak behind it"
        );
        // The flat screen row is what carries that, and it is unmoved by height.
        assert!(tile_to_world_flat(near).1 < tile_to_world_flat(far).1);
        assert!(
            tile_to_world(far).1 > tile_to_world(near).1 + ISO_TILE_H,
            "the peak should still tower on screen"
        );
        clear_iso_heights();
    }

    /// `x + y`: the diagonal row a tile sits on, counting away from the camera.
    fn sort_depth(c: TileCoord) -> i32 {
        c.x + c.y
    }

    #[test]
    fn map_centre_is_the_middle_of_the_projected_map() {
        let _g = FIELD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_iso_heights();
        let (cx, cy) = map_center_world(64, 64);
        assert_eq!(cx, 0.0, "a square map is centred on the screen x axis");
        let south = tile_to_world(TileCoord { x: 0, y: 0 }).1;
        let north = tile_to_world(TileCoord { x: 63, y: 63 }).1;
        assert!((cy - (south + north) * 0.5).abs() < 1.0);
    }
}
