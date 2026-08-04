//! Track that climbs: the ramp geometry for the isometric view.
//!
//! Design brief `15-isometric-track.md`. Two tiles a height apart carry a
//! railway, and a railway may not have a step in it. This module owns the
//! geometry that makes the drawn run continuous, and [`super::visuals`] owns the
//! painting that walks it.
//!
//! # The one fact everything rests on
//!
//! `rail_map`'s isometric projection is **affine in height**:
//!
//! ```text
//! sx = gx - gy
//! sy = (gx + gy) / 2 + h * ISO_LIFT
//! ```
//!
//! It is linear in all three of `gx`, `gy` and `h`, so the projection of a
//! straight line in `(gx, gy, h)` is a straight line on screen. A ramp between
//! two tile centres is therefore not a curve to approximate — it is the straight
//! screen segment from `tile_to_world(a)` to `tile_to_world(b)`, and drawing it
//! right is drawing a straight line.
//!
//! Each of the two pieces draws its own half and they meet at the link midpoint.
//! [`half_link_screen`] shows that midpoint is always a whole number of texels
//! from both centres, so neither half rounds it and neither half can round it
//! differently. The joint is exact by construction.
//!
//! # And the one that fixes the flat
//!
//! A cell bakes on its own and starts its sleeper ladder at its own tile centre.
//! A diagonal link is 45.25 texels and a half-step link is 71.55 — neither a
//! whole number of 4-texel sleepers — so the pitch measured along a real run
//! runs 4, 4, 4 and then 6 or 7 at the boundary. [`sleeper_pitch`] fits the
//! pitch to the link instead of to the tile, which is the whole fix: both halves
//! compute the same ladder from opposite ends.

use bevy::math::Vec2;
use rail_map::ISO_LIFT;
use rail_sim::ids::TileCoord;
use rail_sim::track::{intermediate_tiles, step, DIR16, DIR_COUNT};

/// Texels per tile edge — the pixel contract's source tile size (01 §2.1).
const TEXELS_PER_TILE: f32 = 32.0;

/// Nominal sleeper pitch along the run (01 §5.3). What [`sleeper_pitch`] fits
/// a whole number of into each link.
pub const NOMINAL_SLEEPER_PITCH: f32 = 4.0;

// ── The height delta of every leg ──────────────────────────────────────────

/// The height delta from a piece to each of its sixteen possible neighbours.
///
/// Part of the bake key, so it is `Hash + Eq` and its default — every leg level
/// — is the key flat track and the whole top-down view use. A cell with the
/// default grades is byte-for-byte the cell that was baked before this module
/// existed.
///
/// Stored as a plain `i8` per direction rather than packed: a leg onto **water**
/// is exempt from the grade rules entirely (the projection reads water at its
/// surface, not its bed), so the real range is wider than `MAX_GRADE` and
/// nothing here clamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LegGrades([i8; DIR_COUNT]);

impl LegGrades {
    /// Every leg level. Top-down always, and isometric wherever the ground is.
    pub const LEVEL: Self = Self([0; DIR_COUNT]);

    /// The deltas around `tile`, for the legs in `links`, read through
    /// `height_of`.
    ///
    /// The height source has to be the one `tile_to_world` lifts by —
    /// [`rail_map::tile_height`], not `TrackPiece::height`, which is raw terrain
    /// and disagrees with the projection over water. Reading the field the
    /// projection reads is what makes the two ends of a leg land on the *same*
    /// texel rather than nearly the same one.
    pub fn around(tile: TileCoord, links: u16, height_of: impl Fn(TileCoord) -> i8) -> Self {
        let mut grades = [0i8; DIR_COUNT];
        let here = height_of(tile);
        for (dir, slot) in grades.iter_mut().enumerate() {
            if links & (1 << dir) == 0 {
                continue;
            }
            *slot = height_of(step(tile, dir)).saturating_sub(here);
        }
        Self(grades)
    }

    /// The deltas a piece has in the live projection: real in isometric, level
    /// from above, where nothing is lifted and a ramp would be a lie.
    pub fn for_projection(tile: TileCoord, links: u16) -> Self {
        if rail_map::projection_is_iso() {
            Self::around(tile, links, rail_map::tile_height)
        } else {
            Self::LEVEL
        }
    }

    /// Height delta along one leg. Zero for a leg that is not linked.
    #[inline]
    pub fn at(&self, dir: usize) -> i8 {
        self.0.get(dir).copied().unwrap_or(0)
    }
}

// ── The ramp ───────────────────────────────────────────────────────────────

/// Ground-plane run of one link, in texels: 32 orthogonal, 45.25 diagonal,
/// 71.55 half-step.
#[inline]
pub fn link_run(dir: usize) -> f32 {
    let (dx, dy) = DIR16[dir];
    Vec2::new(dx as f32, dy as f32).length() * TEXELS_PER_TILE
}

/// Half a link — the share of the run this piece draws.
#[inline]
pub fn leg_reach(dir: usize) -> f32 {
    link_run(dir) * 0.5
}

/// Screen offset from a tile centre to the midpoint of one of its links,
/// including the half of the height step this leg climbs.
///
/// **Always integral.** `DIR16` steps are whole tiles and `grade` is a whole
/// number of height units, so:
///
/// ```text
/// (16 * (dx - dy),  8 * (dx + dy) + 2 * grade)
/// ```
///
/// is a whole number of texels for every one of the sixteen. That is why the
/// joint can be asserted as equality: neither half has anything to round.
#[inline]
pub fn half_link_screen(dir: usize, grade: i8) -> Vec2 {
    let (dx, dy) = DIR16[dir];
    Vec2::new(
        TEXELS_PER_TILE * 0.5 * (dx - dy) as f32,
        TEXELS_PER_TILE * 0.25 * (dx + dy) as f32 + ISO_LIFT * 0.5 * grade as f32,
    )
}

/// How far a leg has climbed at `t` texels along its ground run.
///
/// Linear, which is the whole of §2.1 of the brief: the projection is affine in
/// height, so a constant gradient in the world is a constant slope on screen.
/// Zero at the tile centre, half the step at the midpoint.
#[inline]
pub fn lift_at(dir: usize, grade: i8, t: f32) -> f32 {
    if grade == 0 {
        return 0.0;
    }
    let reach = leg_reach(dir);
    (t / reach) * ISO_LIFT * 0.5 * grade as f32
}

/// Walks one leg of a cell: ground offsets in, cell texels out.
///
/// Every painter goes through this, so the bed, the sleepers, the rail bodies,
/// the railheads and the polish layer all ramp together and stay in register.
#[derive(Debug, Clone, Copy)]
pub struct LegWalk {
    /// Projected unit vector along the run.
    pub along: Vec2,
    /// Projected ground-plane perpendicular. Deliberately not re-normalised —
    /// the foreshortening is what makes a sleeper lie flat.
    pub across: Vec2,
    /// How far this piece draws, in texels of ground run.
    pub reach: f32,
    /// Height units this leg climbs over the whole link.
    pub grade: i8,
    dir: usize,
    projection: rail_map::Projection,
}

impl LegWalk {
    /// The walk for one leg, in the projection the cell is being baked for.
    pub fn new(dir: usize, grade: i8, projection: rail_map::Projection) -> Self {
        let (dx, dy) = DIR16[dir];
        let v = Vec2::new(dx as f32, dy as f32).normalize();
        let perp = Vec2::new(-v.y, v.x);
        let (along, across) = match projection {
            rail_map::Projection::TopDown => (v, perp),
            rail_map::Projection::Iso => (project(v), project(perp)),
        };
        Self {
            along,
            across,
            reach: leg_reach(dir),
            // From above nothing is lifted, so nothing climbs.
            grade: match projection {
                rail_map::Projection::TopDown => 0,
                rail_map::Projection::Iso => grade,
            },
            dir,
            projection,
        }
    }

    /// The same leg cut short — the stub an unlinked piece draws so a lone tile
    /// still reads as track. It reaches nowhere, so it climbs nothing.
    pub fn stub(mut self, reach: f32) -> Self {
        self.reach = reach;
        self.grade = 0;
        self
    }

    /// True while this leg runs level.
    #[inline]
    pub fn is_level(&self) -> bool {
        self.grade == 0
    }

    /// The screen point `t` along the run and `s` across it, ramp included.
    #[inline]
    pub fn at(&self, t: f32, s: f32) -> Vec2 {
        self.along_point(t) + self.across * s
    }

    /// How far up the run `t` is, ramp and all.
    ///
    /// Isometric walks *toward the midpoint* rather than along a unit vector
    /// and then adding a lift. The two are the same expression — the half-link
    /// carries both the run and the climb — but this one is exact at both ends:
    /// [`half_link_screen`] is an integral vector, so `t = 0` lands on the tile
    /// centre and `t = reach` lands on the joint with no float drift for the
    /// rounding to swallow.
    ///
    /// Top-down keeps the unit-vector walk it has always used. Its cells are
    /// pinned byte-for-byte, and there is no joint to be exact about: nothing
    /// is lifted and a diagonal midpoint is off the texel lattice either way.
    #[inline]
    fn along_point(&self, t: f32) -> Vec2 {
        match self.projection {
            rail_map::Projection::TopDown => self.along * t,
            // Always the *link's* reach, never a stub's shortened one, or a
            // stub would run the whole half-link in the distance it has.
            rail_map::Projection::Iso => {
                half_link_screen(self.dir, self.grade) * (t / leg_reach(self.dir))
            }
        }
    }

    /// The same point without the ramp — where the ground under it sits, and so
    /// the floor an embankment skirt fills down to.
    #[inline]
    pub fn flat_at(&self, t: f32, s: f32) -> Vec2 {
        self.along * t + self.across * s
    }

    /// Texels this leg stands above its own tile's surface at `t`. Negative
    /// where it runs below, which is a cutting and draws no skirt.
    #[inline]
    pub fn lift(&self, t: f32) -> f32 {
        lift_at(self.dir, self.grade, t)
    }

    /// Where the rail's shadow texel goes, relative to its head.
    ///
    /// One whole texel across the run **in screen space**, on the side away from
    /// the light. One ground unit across a leg projects to 1.118 screen texels
    /// on a 2:1 staircase, so a flank measured on the ground aliases into
    /// speckle; a flank measured in screen texels is a line. See brief 15 §3.4.
    ///
    /// The side is chosen from the *unsnapped* perpendicular and then snapped to
    /// the eight-neighbourhood, so the offset is always exactly one step, never
    /// rounds back onto the head it belongs to, and — because a leg and its
    /// opposite have exactly negated runs — comes out the same from both ends of
    /// a link. A shadow that swapped flanks at a boundary would be a jog.
    pub fn rail_shadow_offset(&self) -> (i32, i32) {
        let run = self.along.normalize_or_zero();
        // Perpendicular to the run, turned to whichever side the light is
        // travelling toward. Isometric lights from the upper left (see
        // `terrain::iso::paint_top`), so the light travels down and to the
        // right and the shadow falls that way.
        let mut n = Vec2::new(run.y, -run.x);
        let away = n.x - n.y;
        if away < 0.0 || (away == 0.0 && n.x < 0.0) {
            n = -n;
        }
        (unit_step(n.x), unit_step(n.y))
    }

    /// The `t` values the sleepers of this leg sit at, near end first.
    ///
    /// Pitched to the link rather than to the tile, so the ladder is even
    /// through the boundary — see [`sleeper_pitch`].
    pub fn sleepers(&self) -> impl Iterator<Item = f32> + '_ {
        let pitch = sleeper_pitch(self.dir, self.projection);
        let count = (self.reach / pitch + 1e-4).floor() as u32;
        (0..=count).map(move |k| k as f32 * pitch)
    }

    /// The `t` values to sample when walking the length of this leg, no coarser
    /// than `step` apart.
    ///
    /// Isometric divides the run into a whole number of samples so the **last
    /// one lands exactly on `reach`** — the link midpoint, where the far piece's
    /// own leg arrives. Walking a fixed 0.5 instead stops up to 0.28 texels
    /// short on a half-step leg, which is the difference between a joint that is
    /// provably closed and one that happens to round shut.
    ///
    /// Top-down keeps the fixed stride it has always walked. Its cells are
    /// pinned byte-for-byte, and from above there is no joint to close: a
    /// diagonal link's midpoint is not on the texel lattice there either way.
    pub fn run_samples(&self, step: f32) -> impl Iterator<Item = f32> {
        let (count, stride) = match self.projection {
            // The fixed stride the flat view has always walked. Every sample it
            // took was a whole multiple of `step`, so counting them out is the
            // same walk to the bit.
            rail_map::Projection::TopDown => ((self.reach / step).floor(), step),
            rail_map::Projection::Iso => {
                let n = (self.reach / step).ceil().max(1.0);
                (n, self.reach / n)
            }
        };
        (0..=count as u32).map(move |k| k as f32 * stride)
    }
}

/// Round toward the nearest of −1, 0, 1.
#[inline]
fn unit_step(v: f32) -> i32 {
    if v > 0.5 {
        1
    } else if v < -0.5 {
        -1
    } else {
        0
    }
}

/// Ground-plane vector to screen-plane vector. The projection is linear, so it
/// applies to a direction exactly as it applies to a point.
#[inline]
fn project(v: Vec2) -> Vec2 {
    let (x, y) = rail_map::project(v.x, v.y);
    Vec2::new(x, y)
}

/// Sleeper pitch along a leg: a whole number of sleepers per **link**.
///
/// From above this is the brief's flat 4 and nothing moves. In isometric it is
/// the link's run divided by the nearest whole number of 4-texel sleepers:
///
/// | Leg | Link run | Sleepers | Pitch |
/// | --- | --- | --- | --- |
/// | Orthogonal | 32.00 | 8 | 4.000 |
/// | Diagonal | 45.25 | 11 | 4.114 |
/// | Half-step | 71.55 | 18 | 3.975 |
///
/// Both halves of a link compute the same ladder from opposite ends, so the
/// pitch is even across the joint instead of stuttering to 6 or 7 texels there.
/// Never more than 3% off the brief's 4 — under half a texel over a whole link,
/// and far under the cost of the stutter it removes.
///
/// Top-down carries the identical latent stutter and is deliberately untouched:
/// it is the shipping view, and its cells are pinned byte-for-byte.
pub fn sleeper_pitch(dir: usize, projection: rail_map::Projection) -> f32 {
    if projection == rail_map::Projection::TopDown {
        return NOMINAL_SLEEPER_PITCH;
    }
    let run = link_run(dir);
    let count = (run / NOMINAL_SLEEPER_PITCH).round().max(1.0);
    run / count
}

// ── What the ghost would link to ───────────────────────────────────────────

/// The link mask a piece at `tile` would have, given what is occupied.
///
/// Mirrors `TrackNetwork::links_for`, which is not callable for track that does
/// not exist yet: the build ghost has to draw the piece the player is *about* to
/// place, including its links to the rest of the route they are dragging. The
/// rule is the network's own — a compass step always links, and a half-step
/// links only while both tiles it crosses are clear of track — evaluated
/// against an occupancy that counts the proposed route as already built.
///
/// `a_ghost_agrees_with_the_network_it_predicts` pins it against the real thing.
pub fn links_for_occupancy(tile: TileCoord, occupied: impl Fn(TileCoord) -> bool) -> u16 {
    let mut links = 0u16;
    for dir in 0..DIR_COUNT {
        if !occupied(step(tile, dir)) {
            continue;
        }
        let clear = match intermediate_tiles(tile, dir) {
            None => true,
            Some(mids) => !mids.iter().copied().any(&occupied),
        };
        if clear {
            links |= 1 << dir;
        }
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_map::{tile_to_world, MapGrid, Projection};
    use rail_sim::track::{is_half_step, opposite_dir};
    use rail_sim::{TrackNetwork, GROUND_LAYER};

    fn iso() -> crate::map::tests::ProjectionGuard {
        crate::map::tests::ProjectionGuard::new(Projection::Iso)
    }

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    /// The reason the joint can be asserted as equality rather than as a
    /// tolerance: there is nothing to round.
    #[test]
    fn every_link_midpoint_lands_on_a_whole_texel() {
        for dir in 0..DIR_COUNT {
            for grade in -8i8..=8 {
                let m = half_link_screen(dir, grade);
                assert_eq!(m.x, m.x.round(), "dir {dir} grade {grade} x {m:?}");
                assert_eq!(m.y, m.y.round(), "dir {dir} grade {grade} y {m:?}");
            }
        }
    }

    /// The midpoint this module computes has to be the midpoint the *projection*
    /// computes, or the drawn run does not end on the tile centres.
    #[test]
    fn the_half_link_is_half_of_what_the_projection_says() {
        let _iso = iso();
        let mut map = MapGrid::empty(24, 24, 1);
        // A hillside: heights that step in both axes, so no two legs share one.
        for y in 0..24i32 {
            for x in 0..24i32 {
                map.get_mut(tile(x, y)).unwrap().height = ((x * 2 + y) % 5) as i8;
            }
        }
        crate::map::projection::set_iso_heights(&map);

        let a = tile(11, 11);
        for dir in 0..DIR_COUNT {
            let b = step(a, dir);
            let grade = rail_map::tile_height(b) - rail_map::tile_height(a);
            let (ax, ay) = tile_to_world(a);
            let (bx, by) = tile_to_world(b);
            let half = half_link_screen(dir, grade);
            assert_eq!(
                (ax + half.x, ay + half.y),
                ((ax + bx) * 0.5, (ay + by) * 0.5),
                "dir {dir} grade {grade}: the half-link is not the midpoint"
            );
            // And the far end draws its half back onto the very same point.
            let back = half_link_screen(opposite_dir(dir), -grade);
            assert_eq!((bx + back.x, by + back.y), (ax + half.x, ay + half.y));
        }
        crate::map::projection::clear_iso_heights();
    }

    /// The ramp is the straight segment: at the end of the leg it has climbed
    /// exactly half the step, and it got there linearly.
    #[test]
    fn the_ramp_is_linear_and_meets_the_midpoint() {
        for dir in 0..DIR_COUNT {
            for grade in [-4i8, -1, 0, 1, 4] {
                let walk = LegWalk::new(dir, grade, Projection::Iso);
                assert_eq!(walk.lift(0.0), 0.0, "a leg starts on its own tile");
                assert_eq!(
                    walk.lift(walk.reach),
                    ISO_LIFT * 0.5 * grade as f32,
                    "dir {dir} grade {grade} does not reach half the step"
                );
                // Linear: the midpoint of the leg is half of the leg's climb.
                let mid = walk.lift(walk.reach * 0.5);
                assert!((mid - walk.lift(walk.reach) * 0.5).abs() < 1e-4);
                // The walk's own end point agrees with the closed form.
                let end = walk.at(walk.reach, 0.0);
                let want = half_link_screen(dir, grade);
                assert!(
                    (end - want).length() < 1e-3,
                    "dir {dir} grade {grade}: walk ended at {end:?}, wanted {want:?}"
                );
            }
        }
    }

    /// Nothing lifts from above, so nothing may ramp there either.
    #[test]
    fn top_down_never_ramps() {
        for dir in 0..DIR_COUNT {
            for grade in [-4i8, -1, 1, 4] {
                let walk = LegWalk::new(dir, grade, Projection::TopDown);
                assert_eq!(walk.grade, 0);
                assert_eq!(walk.lift(walk.reach), 0.0);
                assert_eq!(walk.at(7.0, 3.0), walk.flat_at(7.0, 3.0));
            }
        }
    }

    /// The stutter this fixes, stated as arithmetic: a whole number of sleepers
    /// per link, so both ends lay the same ladder.
    #[test]
    fn the_sleeper_ladder_is_even_across_a_boundary() {
        for dir in 0..DIR_COUNT {
            let pitch = sleeper_pitch(dir, Projection::Iso);
            let run = link_run(dir);
            let count = run / pitch;
            assert!(
                (count - count.round()).abs() < 1e-4,
                "dir {dir}: {count} sleepers per link is not whole"
            );
            // Never far from the brief's 4.
            assert!(
                (pitch - NOMINAL_SLEEPER_PITCH).abs() / NOMINAL_SLEEPER_PITCH < 0.04,
                "dir {dir} pitch {pitch} strays too far from 4"
            );

            // Lay both halves of a real link and measure every gap.
            let near = LegWalk::new(dir, 0, Projection::Iso);
            let far = LegWalk::new(opposite_dir(dir), 0, Projection::Iso);
            let mut along: Vec<f32> = near.sleepers().collect();
            along.extend(far.sleepers().map(|t| run - t));
            along.sort_by(|a, b| a.partial_cmp(b).unwrap());
            along.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
            for w in along.windows(2) {
                assert!(
                    (w[1] - w[0] - pitch).abs() < 1e-3,
                    "dir {dir}: sleepers at {:?} and {:?} are {} apart, not {pitch}",
                    w[0],
                    w[1],
                    w[1] - w[0]
                );
            }
            // The run really is covered end to end.
            assert_eq!(along.first().copied(), Some(0.0));
            assert!((along.last().copied().unwrap() - run).abs() < 1e-3);
        }
    }

    /// The shipping view's ladder does not move.
    #[test]
    fn top_down_keeps_the_briefs_flat_pitch() {
        for dir in 0..DIR_COUNT {
            assert_eq!(sleeper_pitch(dir, Projection::TopDown), NOMINAL_SLEEPER_PITCH);
        }
    }

    /// One texel, always: never back onto the head it shadows, never into the
    /// light's own corner, and — the clause that matters at a joint — the same
    /// side seen from either end, so a rail's shadow cannot swap flanks at a
    /// tile boundary.
    #[test]
    fn the_rail_shadow_is_one_screen_texel_away() {
        for dir in 0..DIR_COUNT {
            let walk = LegWalk::new(dir, 0, Projection::Iso);
            let (ox, oy) = walk.rail_shadow_offset();
            assert!(
                (ox, oy) != (0, 0),
                "dir {dir} shadows itself instead of its flank"
            );
            assert!(ox.abs() <= 1 && oy.abs() <= 1, "dir {dir} offset ({ox},{oy})");
            // Never up and to the left, which is where the light is.
            assert!(
                ox > 0 || (ox == 0 && oy < 0),
                "dir {dir} shadows into the light: ({ox},{oy})"
            );
            assert_eq!(
                (ox, oy),
                LegWalk::new(opposite_dir(dir), 0, Projection::Iso).rail_shadow_offset(),
                "dir {dir} and its opposite shadow onto different flanks"
            );
            // Across the run, not along it — a shadow that ran with the rail
            // would land on the next sample's head and vanish.
            let run = walk.along.normalize();
            let offset = Vec2::new(ox as f32, oy as f32).normalize();
            assert!(
                run.dot(offset).abs() < 0.5,
                "dir {dir} shadows along its own run"
            );
        }
    }

    /// Isometric closes the joint by construction: the last sample of a leg is
    /// the midpoint, not almost the midpoint.
    #[test]
    fn an_isometric_leg_is_walked_all_the_way_to_its_midpoint() {
        for dir in 0..DIR_COUNT {
            let walk = LegWalk::new(dir, 0, Projection::Iso);
            let samples: Vec<f32> = walk.run_samples(0.5).collect();
            assert_eq!(samples.first().copied(), Some(0.0));
            assert_eq!(
                samples.last().copied(),
                Some(walk.reach),
                "dir {dir} stops short of its own midpoint"
            );
            for w in samples.windows(2) {
                assert!(w[1] - w[0] <= 0.5 + 1e-6, "dir {dir} walked too coarsely");
            }

            // From above the stride is the fixed one, unchanged.
            let flat = LegWalk::new(dir, 0, Projection::TopDown);
            let flat_samples: Vec<f32> = flat.run_samples(0.5).collect();
            let want: Vec<f32> = std::iter::successors(Some(0.0f32), |t| Some(t + 0.5))
                .take_while(|t| *t <= flat.reach)
                .collect();
            assert_eq!(flat_samples, want, "dir {dir} moved the top-down stride");
        }
    }

    /// A lone piece's stub reaches nowhere, so it may not ramp toward a
    /// neighbour it has not got.
    #[test]
    fn a_stub_never_climbs() {
        let walk = LegWalk::new(2, 3, Projection::Iso).stub(16.0);
        assert!(walk.is_level());
        assert_eq!(walk.reach, 16.0);
        assert_eq!(walk.lift(16.0), 0.0);
    }

    #[test]
    fn grades_are_read_from_the_field_the_projection_lifts_by() {
        let _iso = iso();
        let mut map = MapGrid::empty(8, 8, 1);
        map.get_mut(tile(4, 4)).unwrap().height = 2;
        map.get_mut(tile(5, 4)).unwrap().height = 5;
        {
            // Water reads as its surface, not its bed — and the grades have to
            // agree with that, or a bridge does not meet its bank.
            let wet = map.get_mut(tile(4, 5)).unwrap();
            wet.water = true;
            wet.height = -7;
        }
        crate::map::projection::set_iso_heights(&map);

        let links = (1 << 2) | (1 << 0); // east and north
        let grades = LegGrades::for_projection(tile(4, 4), links);
        assert_eq!(grades.at(2), 3, "east climbs 5 - 2");
        assert_eq!(grades.at(0), -2, "north drops onto water at surface 0");
        assert_eq!(grades.at(6), 0, "an unlinked leg is level");
        assert_ne!(grades, LegGrades::LEVEL);
        crate::map::projection::clear_iso_heights();
    }

    #[test]
    fn top_down_grades_are_always_level() {
        let _flat = crate::map::tests::ProjectionGuard::new(Projection::TopDown);
        let mut map = MapGrid::empty(8, 8, 1);
        map.get_mut(tile(5, 4)).unwrap().height = 9;
        crate::map::projection::set_iso_heights(&map);
        let grades = LegGrades::for_projection(tile(4, 4), u16::MAX);
        assert_eq!(grades, LegGrades::LEVEL);
        assert_eq!(grades, LegGrades::LEVEL);
        crate::map::projection::clear_iso_heights();
    }

    /// The ghost's link prediction has to be the network's rule, not a
    /// lookalike — so with nothing proposed it must agree exactly.
    #[test]
    fn a_ghost_agrees_with_the_network_it_predicts() {
        let mut network = TrackNetwork::new();
        let mut money = rail_sim::Money::new(10_000_000);
        let mut ledger = rail_sim::MoneyLedger::default();
        let terrain = rail_sim::TrackTerrain::new(16, 16, (0..16 * 16).map(|_| (false, 0i8)));
        // A run, a diagonal and a lone piece two away, so half-steps both link
        // and get blocked.
        for t in [
            tile(4, 4),
            tile(5, 4),
            tile(6, 4),
            tile(6, 5),
            tile(8, 5),
            tile(3, 6),
        ] {
            rail_sim::track::try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                t,
                GROUND_LAYER,
            )
            .expect("placed");
        }
        for y in 3..8 {
            for x in 3..10 {
                let t = tile(x, y);
                let mine = links_for_occupancy(t, |c| network.id_at(c, GROUND_LAYER).is_some());
                let theirs = network.links_for(t, GROUND_LAYER).0;
                assert_eq!(mine, theirs, "links disagree at {t:?}");
            }
        }
    }

    /// A tile on the proposed route links to the rest of the route, which is
    /// the whole point: the ghost draws the run the player is dragging.
    #[test]
    fn a_ghost_links_along_the_route_it_is_proposing() {
        let network = TrackNetwork::new();
        let route = [tile(2, 2), tile(3, 2), tile(4, 2)];
        let occupied = |c: TileCoord| {
            route.contains(&c) || network.id_at(c, GROUND_LAYER).is_some()
        };
        // The middle of the run is a straight through-piece.
        assert_eq!(
            links_for_occupancy(tile(3, 2), occupied),
            (1 << 2) | (1 << 6),
            "the middle of a proposed run should link both ways"
        );
        // The far end links back one way only.
        assert_eq!(links_for_occupancy(tile(4, 2), occupied), 1 << 6);
        // A half-step is refused while the tiles it crosses are on the route.
        let bent = [tile(0, 0), tile(2, 1)];
        let bent_occupied = |c: TileCoord| bent.contains(&c);
        assert_eq!(links_for_occupancy(tile(0, 0), bent_occupied), 1 << 9);
        let blocked = [tile(0, 0), tile(2, 1), tile(1, 0)];
        let blocked_occupied = |c: TileCoord| blocked.contains(&c);
        assert_eq!(
            links_for_occupancy(tile(0, 0), blocked_occupied) & (1 << 9),
            0,
            "a half-step over occupied ground must not link"
        );
    }

    /// Ghost/placed parity, at the level where the two can actually drift: the
    /// links the ghost predicts for a route have to be the links the network
    /// hands those pieces once the route is committed.
    ///
    /// Everything downstream of the mask — variant, bridge, grades, the bake —
    /// is one shared key builder, so if the masks agree the pictures agree.
    #[test]
    fn what_the_ghost_predicts_is_what_the_route_becomes() {
        let mut network = TrackNetwork::new();
        let mut money = rail_sim::Money::new(10_000_000);
        let mut ledger = rail_sim::MoneyLedger::default();
        let terrain = rail_sim::TrackTerrain::new(16, 16, (0..16 * 16).map(|_| (false, 0i8)));

        // Something already on the ground for the route to join onto.
        rail_sim::track::try_place_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            tile(2, 6),
            GROUND_LAYER,
        )
        .expect("anchor placed");

        // An S-curve: straight, a diagonal, then a knight's move away.
        let route = [
            tile(2, 5),
            tile(3, 5),
            tile(4, 4),
            tile(5, 3),
            tile(7, 2),
        ];
        let on_route: Vec<TileCoord> = route.to_vec();
        let predicted: Vec<u16> = route
            .iter()
            .map(|&t| {
                links_for_occupancy(t, |c| {
                    on_route.contains(&c) || network.id_at(c, GROUND_LAYER).is_some()
                })
            })
            .collect();

        rail_sim::track::try_place_path(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            &route,
            GROUND_LAYER,
        )
        .expect("route placed");

        for (&t, &want) in route.iter().zip(&predicted) {
            let piece = network.at(t, GROUND_LAYER).expect("route piece");
            assert_eq!(
                piece.links.0, want,
                "the ghost at {t:?} drew links the placed piece does not have"
            );
        }
    }

    #[test]
    fn the_reaches_are_the_ones_the_cross_section_was_sized_for() {
        assert_eq!(leg_reach(2), 16.0);
        assert!((leg_reach(1) - 22.627).abs() < 0.01);
        assert!((leg_reach(9) - 35.777).abs() < 0.01);
        for dir in 0..DIR_COUNT {
            assert_eq!(is_half_step(dir), leg_reach(dir) > TEXELS_PER_TILE);
            assert_eq!(link_run(dir), leg_reach(dir) * 2.0);
        }
    }
}
