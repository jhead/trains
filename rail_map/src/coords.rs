//! World ↔ tile conversion helpers, in either of the game's two projections.
//!
//! Tile `(0, 0)` is the south-west corner of the map. Everything in the game
//! that ever asks where a tile is comes through [`tile_to_world`], and
//! everything that asks which tile a point is over comes through
//! [`world_to_tile`]. That is the whole seam: swap what those two functions
//! compute and track, stations, trains, peeps, decals, smoke, alerts, audio
//! panning and the cursor all move with them, with no change at any call site.
//!
//! # The two projections
//!
//! [`Projection::TopDown`] is the shipping one. A tile is a 32 × 32 square,
//! world `(x + 0.5, y + 0.5) · TILE_SIZE`, +X east and +Y north, and the screen
//! is the ground plane seen from directly above.
//!
//! [`Projection::Iso`] is classic 2:1 dimetric. Take the same ground plane
//! `(gx, gy)`:
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
//! A tile is therefore a 64 × 32 diamond. Bevy's +Y is up, so the lift **adds**.
//! +X runs up-and-right, +Y runs up-and-left: the camera sits over the map's
//! south-west corner, so the near corner of a tile is its south-west one and the
//! two faces a cliff shows are its **south** and **west** ones.
//!
//! # Both globals are the same trade
//!
//! The live projection ([`set_projection`]) and the height field the iso lift
//! reads ([`set_iso_heights`]) are process-global. [`tile_to_world`] takes a
//! [`TileCoord`] and nothing else — every one of its ~40 call sites across the
//! presentation crate assumes that, and threading a mode flag and a `&MapGrid`
//! through all of them to pick a branch and look up one `i8` would be most of a
//! diff and none of the behaviour. Unset, the projection is top-down and every
//! tile sits at zero, which is what sim-side and generator tests get.
//!
//! Neither global is sim state. Nothing in `rail_sim` reads either one, no save
//! records them, and flipping the projection mid-simulation is invisible to
//! every tick that follows — `rail_town`'s `map::projection` has the test that
//! pins that.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::RwLock;

use rail_sim::ids::TileCoord;

use crate::grid::MapGrid;

/// Side length of one map tile in *ground-plane* units.
pub const TILE_SIZE: f32 = 32.0;

/// Screen width of one tile diamond, in [`Projection::Iso`].
pub const ISO_TILE_W: f32 = TILE_SIZE * 2.0;
/// Screen height of one tile diamond, in [`Projection::Iso`].
pub const ISO_TILE_H: f32 = TILE_SIZE;

/// Screen pixels one unit of terrain height lifts a tile.
///
/// `rail_map` steps elevation in bands 3 or 4 units apart, so a band step is
/// 12–16 px — between three eighths and half a tile's screen height, which is a
/// terrace you can see from across the map without a mountain leaving the
/// screen. The tallest band on a default map (height 16) stands 64 px, two
/// tiles.
pub const ISO_LIFT: f32 = 4.0;

// ── Which projection is live ───────────────────────────────────────────────

/// How the game draws the ground plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Projection {
    /// Axis-aligned square tiles seen from directly above. The shipping view.
    #[default]
    TopDown,
    /// 2:1 dimetric diamonds with an elevation lift.
    Iso,
}

impl Projection {
    /// The other one. A view mode with two members has exactly one flip.
    #[inline]
    pub fn flipped(self) -> Self {
        match self {
            Self::TopDown => Self::Iso,
            Self::Iso => Self::TopDown,
        }
    }

    /// Stable name for settings files and log lines. ASCII, player-facing.
    #[inline]
    pub fn label(self) -> &'static str {
        match self {
            Self::TopDown => "Top-down",
            Self::Iso => "Isometric",
        }
    }

    #[inline]
    fn from_bits(bits: u8) -> Self {
        if bits == 1 {
            Self::Iso
        } else {
            Self::TopDown
        }
    }

    #[inline]
    fn bits(self) -> u8 {
        match self {
            Self::TopDown => 0,
            Self::Iso => 1,
        }
    }
}

static PROJECTION: AtomicU8 = AtomicU8::new(0);

/// The projection every conversion below is currently using.
#[inline]
pub fn projection() -> Projection {
    Projection::from_bits(PROJECTION.load(Ordering::Relaxed))
}

/// Make `projection` the live one. Returns the one it replaced.
///
/// Presentation-side only — see the module docs. `rail_town`'s
/// `map::projection` owns the flip and everything that has to follow it.
pub fn set_projection(projection: Projection) -> Projection {
    Projection::from_bits(PROJECTION.swap(projection.bits(), Ordering::Relaxed))
}

/// `true` while the world is drawn in 2:1 dimetric.
#[inline]
pub fn projection_is_iso() -> bool {
    projection() == Projection::Iso
}

// ── The iso projection itself ──────────────────────────────────────────────

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

/// A displacement in world space, from a displacement on the ground plane.
///
/// The projection is linear away from the lift, so a *difference* between two
/// positions carries across it exactly — which is what lets the camera keep the
/// fraction of a tile it was standing off centre when the view flips, instead
/// of snapping to the nearest tile centre and drifting half a tile every time.
#[inline]
pub fn project_offset(gx: f32, gy: f32) -> (f32, f32) {
    match projection() {
        Projection::TopDown => (gx, gy),
        Projection::Iso => project(gx, gy),
    }
}

/// The inverse of [`project_offset`]: a world displacement back onto the ground.
#[inline]
pub fn unproject_offset(sx: f32, sy: f32) -> (f32, f32) {
    match projection() {
        Projection::TopDown => (sx, sy),
        Projection::Iso => unproject(sx, sy),
    }
}

/// Ground-plane centre of a tile, before projection.
#[inline]
pub fn tile_to_ground(coord: TileCoord) -> (f32, f32) {
    (
        (coord.x as f32 + 0.5) * TILE_SIZE,
        (coord.y as f32 + 0.5) * TILE_SIZE,
    )
}

// ── The top-down layout, whatever the live projection is ───────────────────
//
// A purpose-built plan drawing — the Map View's schematic plate — is laid out
// in tile order at a fixed scale and is not a picture of the world, so it wants
// these three regardless of how the world itself is being drawn. Everything
// else should use the projection-following versions below.

/// Top-down world centre of a tile, ignoring the live projection.
#[inline]
pub fn top_down_tile_to_world(coord: TileCoord) -> (f32, f32) {
    tile_to_ground(coord)
}

/// Tile containing a top-down point, ignoring the live projection.
#[inline]
pub fn top_down_world_to_tile(x: f32, y: f32) -> TileCoord {
    TileCoord {
        x: (x / TILE_SIZE).floor() as i32,
        y: (y / TILE_SIZE).floor() as i32,
    }
}

/// Top-down centre of the whole map, ignoring the live projection.
#[inline]
pub fn top_down_map_center(width: u32, height: u32) -> (f32, f32) {
    (
        width as f32 * TILE_SIZE * 0.5,
        height as f32 * TILE_SIZE * 0.5,
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

/// Install `map`'s heights as the lift the iso projection uses.
///
/// Called whenever a map is installed or edited. Cheap — one `i8` per tile.
/// Harmless in [`Projection::TopDown`], which never reads the field.
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

/// Screen pixels a tile is lifted by its own elevation, in [`Projection::Iso`].
///
/// Deliberately answers the same in either projection: it is a question about
/// the iso terrain build, not about where anything is drawn right now.
#[inline]
pub fn tile_lift(coord: TileCoord) -> f32 {
    tile_height(coord) as f32 * ISO_LIFT
}

// ── The contract the rest of the game uses ─────────────────────────────────

/// World-space centre of a tile, in the live projection.
#[inline]
pub fn tile_to_world(coord: TileCoord) -> (f32, f32) {
    match projection() {
        Projection::TopDown => top_down_tile_to_world(coord),
        Projection::Iso => {
            let (gx, gy) = tile_to_ground(coord);
            let (sx, sy) = project(gx, gy);
            (sx, sy + tile_lift(coord))
        }
    }
}

/// World-space centre of a tile **on the ground plane**, ignoring its elevation.
///
/// This is what iso depth sorting keys on: a lifted mountain must not sort in
/// front of the tile that is genuinely nearer the camera. In top-down it is
/// [`tile_to_world`], because nothing is lifted there.
#[inline]
pub fn tile_to_world_flat(coord: TileCoord) -> (f32, f32) {
    match projection() {
        Projection::TopDown => top_down_tile_to_world(coord),
        Projection::Iso => {
            let (gx, gy) = tile_to_ground(coord);
            project(gx, gy)
        }
    }
}

/// Tile under a world-space point, in the live projection.
///
/// Top-down floors toward −∞ and is exact. Iso makes it a search rather than an
/// inverse: a lifted tile covers the ground behind it, so the answer is the
/// *topmost* tile whose diamond the point falls in. Lift levels are tried high
/// to low, and a higher lift always resolves to a smaller `x + y` — a tile
/// nearer the camera, drawn later — so the first level that explains the point
/// is the one the player can see.
///
/// With no height field installed the iso branch collapses to the flat inverse.
pub fn world_to_tile(sx: f32, sy: f32) -> TileCoord {
    if projection() == Projection::TopDown {
        return top_down_world_to_tile(sx, sy);
    }

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

/// World-space centre of the whole map, in the live projection (useful for
/// framing the camera).
pub fn map_center_world(width: u32, height: u32) -> (f32, f32) {
    let (cx, cy) = top_down_map_center(width, height);
    match projection() {
        Projection::TopDown => (cx, cy),
        Projection::Iso => project(cx, cy),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tile::TerrainKind;

    /// Serialises every test that touches a process-global — the projection and
    /// the height field both are.
    static GLOBALS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Holds the globals for one test and puts them back afterwards, so a
    /// panicking test cannot leave the process in the other projection.
    struct Guard {
        _lock: std::sync::MutexGuard<'static, ()>,
        restore: Projection,
    }

    impl Guard {
        fn with(projection: Projection) -> Self {
            let lock = GLOBALS.lock().unwrap_or_else(|e| e.into_inner());
            let restore = set_projection(projection);
            clear_iso_heights();
            Self {
                _lock: lock,
                restore,
            }
        }

        fn iso() -> Self {
            Self::with(Projection::Iso)
        }

        fn top_down() -> Self {
            Self::with(Projection::TopDown)
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            clear_iso_heights();
            set_projection(self.restore);
        }
    }

    fn flat_map(w: u32, h: u32) -> MapGrid {
        MapGrid::empty(w, h, 1)
    }

    // ── Top-down: the shipping projection, unchanged ───────────────────────

    #[test]
    fn top_down_is_the_default() {
        assert_eq!(Projection::default(), Projection::TopDown);
        assert_eq!(Projection::TopDown.flipped(), Projection::Iso);
        assert_eq!(Projection::Iso.flipped(), Projection::TopDown);
    }

    #[test]
    fn a_top_down_tile_is_a_thirty_two_unit_square() {
        let _g = Guard::top_down();
        let c = TileCoord { x: 3, y: 7 };
        assert_eq!(tile_to_world(c), (3.5 * TILE_SIZE, 7.5 * TILE_SIZE));
        assert_eq!(tile_to_world_flat(c), tile_to_world(c));
        assert_eq!(world_to_tile(0.0, 0.0), TileCoord { x: 0, y: 0 });
        assert_eq!(world_to_tile(31.9, 31.9), TileCoord { x: 0, y: 0 });
        assert_eq!(world_to_tile(32.0, 32.0), TileCoord { x: 1, y: 1 });
        assert_eq!(map_center_world(64, 64), (1024.0, 1024.0));
    }

    /// Elevation must not reach the top-down view: a height field is installed
    /// whatever the projection, and lifting a square tile would move it off its
    /// own grid cell.
    #[test]
    fn top_down_ignores_the_height_field() {
        let _g = Guard::top_down();
        let mut map = flat_map(16, 16);
        let peak = TileCoord { x: 8, y: 8 };
        map.get_mut(peak).unwrap().height = 12;
        set_iso_heights(&map);

        assert_eq!(tile_to_world(peak), (8.5 * TILE_SIZE, 8.5 * TILE_SIZE));
        let (wx, wy) = tile_to_world(peak);
        assert_eq!(world_to_tile(wx, wy), peak);
    }

    #[test]
    fn top_down_round_trips_every_tile_centre() {
        let _g = Guard::top_down();
        for y in -3..40 {
            for x in -3..40 {
                let c = TileCoord { x, y };
                let (wx, wy) = tile_to_world(c);
                assert_eq!(world_to_tile(wx, wy), c, "centre of {c:?} did not invert");
            }
        }
    }

    // ── Iso ────────────────────────────────────────────────────────────────

    #[test]
    fn an_iso_tile_is_a_two_to_one_diamond() {
        let _g = Guard::iso();
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
        let _g = Guard::iso();
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
        let _g = Guard::iso();
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
        let _g = Guard::iso();
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
    }

    #[test]
    fn water_sits_at_its_surface_not_its_bed() {
        let _g = Guard::iso();
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
    }

    /// Depth sorting keys on the *flat* position: a mountain must not sort in
    /// front of the tile in front of it just because it is tall.
    #[test]
    fn the_sort_key_ignores_elevation() {
        let _g = Guard::iso();
        let mut map = flat_map(8, 8);
        map.get_mut(TileCoord { x: 4, y: 4 }).unwrap().height = 16;
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
    }

    /// `x + y`: the diagonal row a tile sits on, counting away from the camera.
    fn sort_depth(c: TileCoord) -> i32 {
        c.x + c.y
    }

    #[test]
    fn map_centre_is_the_middle_of_the_projected_map() {
        let _g = Guard::iso();
        let (cx, cy) = map_center_world(64, 64);
        assert_eq!(cx, 0.0, "a square map is centred on the screen x axis");
        let south = tile_to_world(TileCoord { x: 0, y: 0 }).1;
        let north = tile_to_world(TileCoord { x: 63, y: 63 }).1;
        assert!((cy - (south + north) * 0.5).abs() < 1.0);
    }

    // ── The flip ───────────────────────────────────────────────────────────

    /// The plan helpers are the schematic plate's, and a plate is a drawing in
    /// tile order — it must read the same whichever way the world is drawn.
    #[test]
    fn the_top_down_layout_helpers_ignore_the_live_projection() {
        let c = TileCoord { x: 6, y: 2 };
        let mut seen = Vec::new();
        for mode in [Projection::TopDown, Projection::Iso] {
            let _g = Guard::with(mode);
            let mut map = flat_map(8, 8);
            map.get_mut(c).unwrap().height = 9;
            set_iso_heights(&map);
            seen.push((
                top_down_tile_to_world(c),
                top_down_map_center(8, 8),
                top_down_world_to_tile(200.0, 80.0),
            ));
        }
        assert_eq!(seen[0], seen[1]);
        assert_eq!(seen[0].0, (6.5 * TILE_SIZE, 2.5 * TILE_SIZE));
    }

    /// Two flips are the identity — for the projection itself and therefore for
    /// every coordinate that reads it.
    #[test]
    fn two_flips_land_back_where_they_started() {
        let _g = Guard::top_down();
        let mut map = flat_map(12, 12);
        map.get_mut(TileCoord { x: 5, y: 5 }).unwrap().height = 10;
        set_iso_heights(&map);

        let sample: Vec<(f32, f32)> = (0..12)
            .flat_map(|y| (0..12).map(move |x| TileCoord { x, y }))
            .map(tile_to_world)
            .collect();

        set_projection(Projection::Iso);
        let iso: Vec<(f32, f32)> = (0..12)
            .flat_map(|y| (0..12).map(move |x| TileCoord { x, y }))
            .map(tile_to_world)
            .collect();
        assert_ne!(sample, iso, "the flip has to actually change something");

        set_projection(Projection::TopDown);
        let back: Vec<(f32, f32)> = (0..12)
            .flat_map(|y| (0..12).map(move |x| TileCoord { x, y }))
            .map(tile_to_world)
            .collect();
        assert_eq!(sample, back, "flipping twice moved the world");
    }

    /// Picking has to round-trip in whichever projection is live, over real
    /// terrain rather than a flat plain.
    #[test]
    fn picking_round_trips_in_both_projections() {
        for mode in [Projection::TopDown, Projection::Iso] {
            let _g = Guard::with(mode);
            let mut map = flat_map(24, 24);
            for y in 0..24i32 {
                for x in 0..24i32 {
                    // A ramp plus a step, so no two rows share a lift.
                    map.get_mut(TileCoord { x, y }).unwrap().height =
                        ((x + y) % 5) as i8 + if y > 12 { 4 } else { 0 };
                }
            }
            set_iso_heights(&map);
            for y in 0..24i32 {
                for x in 0..24i32 {
                    let c = TileCoord { x, y };
                    let (wx, wy) = tile_to_world(c);
                    assert_eq!(
                        world_to_tile(wx, wy),
                        c,
                        "{c:?} did not round-trip in {}",
                        mode.label()
                    );
                }
            }
        }
    }

    /// The camera's sub-tile offset has to survive a flip, so the offset
    /// helpers must be exact inverses in either projection.
    #[test]
    fn an_offset_round_trips_through_either_projection() {
        for mode in [Projection::TopDown, Projection::Iso] {
            let _g = Guard::with(mode);
            for &(gx, gy) in &[(0.0, 0.0), (-16.0, -16.0), (7.5, -3.25), (31.0, 12.0)] {
                let (sx, sy) = project_offset(gx, gy);
                let (bx, by) = unproject_offset(sx, sy);
                assert!((bx - gx).abs() < 1e-3 && (by - gy).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn set_projection_reports_what_it_replaced() {
        let _g = Guard::top_down();
        assert_eq!(set_projection(Projection::Iso), Projection::TopDown);
        assert!(projection_is_iso());
        assert_eq!(set_projection(Projection::TopDown), Projection::Iso);
        assert!(!projection_is_iso());
    }

    #[test]
    fn the_labels_are_ascii() {
        for mode in [Projection::TopDown, Projection::Iso] {
            assert!(mode.label().is_ascii(), "{mode:?} has a tofu label");
        }
    }
}
