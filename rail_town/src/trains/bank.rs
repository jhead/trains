//! The train facing bank — baked at the **realised** bearings of the track rose.
//!
//! # Why not thirty-two even steps
//!
//! Brief 01 §5.2 asks for "a thirty-two entry sprite bank for the rail runs",
//! and the obvious reading of that is 32 entries at 11.25°. That reading is
//! wrong, and §5.2 says so itself two paragraphs later:
//!
//! > A 32-entry bank spaced at 11.25° does not contain 26.57° — the nearest
//! > entry is 4.07° away, well above the `spritebank` plate's 2.81° target.
//! > Baking an even bank would reintroduce exactly the facet-popping that plate
//! > exists to warn about. Bake at the sixteen realised bearings plus
//! > interpolants between them.
//!
//! A square lattice cannot give an even rose. The half-steps are knight's
//! moves, so the realised tangents are `atan2` of the actual
//! [`DIR16`](rail_sim::track::DIR16) vectors — 0° · 26.57° · 45° · 63.43° · 90°
//! and so on round the compass, stepping `26.57 · 18.43 · 18.43 · 26.57` per
//! quadrant. Those sixteen are the *only* headings a train on this graph ever
//! actually holds.
//!
//! So the bank is thirty-two entries and none of them is a rounding of
//! anything:
//!
//! - **Even entries** are the sixteen realised bearings. Bank error on a
//!   straight run is therefore **exactly zero**, not 2.81°.
//! - **Odd entries** are the midpoints between adjacent realised bearings. A
//!   train crossing a node where the route turns one rose step sweeps
//!   `dir → mid → next` instead of snapping, so a curve reads as a curve.
//!
//! # Direction is a different sprite, never a rotation
//!
//! §2.2 is a contract, and this module is where it is easiest to break: a
//! facing is *exactly* the thing a `Quat` would be convenient for. Nothing here
//! rotates, mirrors or stretches — every entry is painted from scratch at its
//! own bearing, and picking a facing is picking a handle. The transforms this
//! module's art rides on carry identity rotation and unit scale.
//!
//! The previous stand-in expressed facing as an axis-aligned rectangle
//! elongated along travel plus `flip_x`, which collapsed the sixteen directions
//! onto four appearances — NE and SE drew identically — and was a transform
//! besides.
//!
//! # No noise
//!
//! §2.4 anchors procedural variation to integer world coordinates. A train is
//! the one thing in the world that is *not* at a world coordinate for long, so
//! it carries no procedural variation at all: the bank is a pure function of
//! kind and bearing, and two bakes of the same entry are byte-identical.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rail_sim::commands::TrainKind;
use rail_sim::track::{bearing_deg, clock_index, dir_from_clock, DIR_COUNT};

use crate::palette::{
    OUTLINE, PLASTER_L, ROOF_SLATE_D, ROOF_SLATE_L, ROOF_SLATE_M, ROOF_TILE_D, ROOF_TILE_L,
    ROOF_TILE_M, WIN_DARK, WOOD_M,
};

/// Entries in the bank: the sixteen realised bearings, interleaved with the
/// sixteen midpoints between them.
pub const BANK_ENTRIES: usize = DIR_COUNT * 2;

/// Cell edge in texels — one tile, which fits the longest body on the diagonal.
const CELL: u32 = 32;
const CENTER: i32 = (CELL / 2) as i32;

/// Body length along travel, in texels (0.55 of a tile, brief 01 §7's "real
/// dimensions" placeholder rule).
const BODY_LEN: f32 = 18.0;
/// Body width across the rail, in texels.
const BODY_WID: f32 = 8.0;
/// A trailing car is shorter than the engine that pulls it, so a consist reads
/// as *one engine and its train* rather than as a row of identical blocks.
const CAR_LEN: f32 = 13.0;

/// Which vehicle in a consist a cell draws.
///
/// 07 §5 wants a train to be *"composed of a locomotive and cars, with the
/// length visible in the world"*. Length alone is not composition: three
/// identical bodies in a line read as three trains queueing, which is a state
/// this game genuinely has and must not be confused with. So the leading
/// vehicle keeps its headlamp and its full length, and a car is a shorter body
/// with no lamp — the same hue, so the pair still reads as one train of one
/// kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainPart {
    /// The engine, at the head of the consist.
    Loco,
    /// A trailing carriage or wagon.
    Car,
}

/// Sub-texel step when walking the body, so a body at 26.57° leaves no holes.
const WALK_STEP: f32 = 0.5;

/// How far into a leg the facing is still easing out of the previous heading,
/// and how early it starts easing into the next. A quarter each end leaves the
/// middle half of every tile on the leg's own realised bearing.
pub const CURVE_EASE: f32 = 0.25;

/// Bearing of a bank entry, in degrees clockwise from north.
///
/// Even entries are realised tangents; odd entries bisect the realised step
/// they sit inside. Never a multiple of the 11.25° that an even 32-rose would
/// use, except where a realised bearing happens to land on one.
pub fn bank_bearing_deg(entry: usize) -> f32 {
    let entry = entry % BANK_ENTRIES;
    let clock = entry / 2;
    let a = bearing_deg(dir_from_clock(clock));
    if entry % 2 == 0 {
        return a;
    }
    let mut b = bearing_deg(dir_from_clock(clock + 1));
    // The last step wraps past north; unwrap it before bisecting or the
    // midpoint of 333.43° and 0° would come out due south.
    if b <= a {
        b += 360.0;
    }
    ((a + b) * 0.5).rem_euclid(360.0)
}

/// Bank entry for a direction the train is actually running along.
#[inline]
pub fn entry_for_dir(dir: usize) -> usize {
    clock_index(dir) * 2
}

/// Bank entry midway between two directions, when they are one rose step apart.
///
/// `None` for anything wider: a turn of two steps or more is not a sweep the
/// bank can interpolate, and a train taking one should show the leg's own
/// bearing rather than a heading it never holds.
#[inline]
pub fn entry_between(a: usize, b: usize) -> Option<usize> {
    let (ca, cb) = (clock_index(a), clock_index(b));
    match (cb + DIR_COUNT - ca) % DIR_COUNT {
        1 => Some(ca * 2 + 1),
        d if d == DIR_COUNT - 1 => Some(cb * 2 + 1),
        _ => None,
    }
}

/// Which bank entry a train shows, given the leg it is on, the legs either side
/// of it, and how far along the leg it has travelled.
///
/// The tangent of a polyline at the middle of a segment is the segment's own
/// direction, and at a node it is the average of the two segments meeting
/// there. That is exactly the realised bearing in the middle of a tile and the
/// midpoint entry at its ends, which is why the bank holds both.
pub fn facing_entry(previous: Option<usize>, dir: usize, next: Option<usize>, t: f32) -> usize {
    if t < CURVE_EASE {
        if let Some(entry) = previous.and_then(|p| entry_between(p, dir)) {
            return entry;
        }
    } else if t > 1.0 - CURVE_EASE {
        if let Some(entry) = next.and_then(|n| entry_between(dir, n)) {
            return entry;
        }
    }
    entry_for_dir(dir)
}

// ── Baking ─────────────────────────────────────────────────────────────────

/// Baked facings, kept for the life of the session and shared by every train.
///
/// Content-addressed on `(kind, entry)`, so a hundred trains on one bearing
/// cost one cell.
#[derive(Default)]
pub struct TrainBank {
    /// Keyed on `(kind index, part index, entry)`. `TrainKind` lives in
    /// `rail_sim` and is not `Hash`, and presentation does not get to change sim
    /// types.
    cache: HashMap<(u8, u8, usize), Handle<Image>>,
}

/// Cache key half for a kind.
#[inline]
fn kind_index(kind: TrainKind) -> u8 {
    match kind {
        TrainKind::Transit => 0,
        TrainKind::Transport => 1,
    }
}

#[inline]
fn part_index(part: TrainPart) -> u8 {
    match part {
        TrainPart::Loco => 0,
        TrainPart::Car => 1,
    }
}

impl TrainBank {
    /// The leading vehicle's cell — what a single-car train has always drawn.
    pub fn get(
        &mut self,
        images: &mut Assets<Image>,
        kind: TrainKind,
        entry: usize,
    ) -> Handle<Image> {
        self.get_part(images, kind, TrainPart::Loco, entry)
    }

    pub fn get_part(
        &mut self,
        images: &mut Assets<Image>,
        kind: TrainKind,
        part: TrainPart,
        entry: usize,
    ) -> Handle<Image> {
        let entry = entry % BANK_ENTRIES;
        let key = (kind_index(kind), part_index(part), entry);
        if let Some(handle) = self.cache.get(&key) {
            return handle.clone();
        }
        let handle = images.add(cell_image(paint_facing(kind, part, entry)));
        self.cache.insert(key, handle.clone());
        handle
    }

    /// Cells baked so far — the vocabulary actually in use.
    #[cfg(test)]
    pub fn baked(&self) -> usize {
        self.cache.len()
    }
}

fn rgba(color: Color) -> [u8; 4] {
    let s = color.to_srgba();
    [
        (s.red * 255.0).round() as u8,
        (s.green * 255.0).round() as u8,
        (s.blue * 255.0).round() as u8,
        255,
    ]
}

/// One facing cell under construction: RGBA texels, cell-centred, y up.
struct Canvas {
    px: Vec<u8>,
}

impl Canvas {
    fn new() -> Self {
        Self {
            px: vec![0u8; (CELL * CELL) as usize * 4],
        }
    }

    fn put(&mut self, x: i32, y: i32, color: [u8; 4]) {
        let ix = CENTER + x;
        let iy = CENTER - y;
        if ix < 0 || iy < 0 || ix >= CELL as i32 || iy >= CELL as i32 {
            return;
        }
        let o = ((iy as u32 * CELL + ix as u32) * 4) as usize;
        self.px[o..o + 4].copy_from_slice(&color);
    }

    #[cfg(test)]
    fn at(&self, x: i32, y: i32) -> [u8; 4] {
        let ix = CENTER + x;
        let iy = CENTER - y;
        if ix < 0 || iy < 0 || ix >= CELL as i32 || iy >= CELL as i32 {
            return [0; 4];
        }
        let o = ((iy as u32 * CELL + ix as u32) * 4) as usize;
        [self.px[o], self.px[o + 1], self.px[o + 2], self.px[o + 3]]
    }
}

fn cell_image(canvas: Canvas) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: CELL,
            height: CELL,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        canvas.px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    // One texel is one screen pixel times a whole number (brief 01 §2.1).
    image.sampler = ImageSampler::nearest();
    image
}

/// The three-step ramp a kind is drawn in.
///
/// Cool slate for the passenger train, warm tile for the goods train — the
/// widest hue separation the world palette offers without touching the
/// diagnostic accents, so acceptance-bar item 4 ("tell a passenger train from a
/// goods train") is answered by hue before it is answered by shape.
fn ramp(kind: TrainKind) -> [Color; 3] {
    match kind {
        TrainKind::Transit => [ROOF_SLATE_D, ROOF_SLATE_M, ROOF_SLATE_L],
        TrainKind::Transport => [ROOF_TILE_D, ROOF_TILE_M, ROOF_TILE_L],
    }
}

/// Paint one facing: the body walked along the entry's bearing, in its own
/// axes, one texel at a time.
fn paint_facing(kind: TrainKind, part: TrainPart, entry: usize) -> Canvas {
    let mut canvas = Canvas::new();
    let theta = bank_bearing_deg(entry).to_radians();
    // Bearings run clockwise from north, so north is +y and east is +x.
    let along = Vec2::new(theta.sin(), theta.cos());
    let across = Vec2::new(along.y, -along.x);

    let [dark, mid, light] = ramp(kind);
    let leading = part == TrainPart::Loco;
    let half_len = if leading { BODY_LEN } else { CAR_LEN } * 0.5;
    let half_wid = BODY_WID * 0.5;

    let mut t = -half_len;
    while t <= half_len {
        let mut s = -half_wid;
        while s <= half_wid {
            let color = if t.abs() > half_len - 1.0 || s.abs() > half_wid - 1.0 {
                // The one outline colour in the game (brief 01 §3.1).
                OUTLINE
            } else if leading && t > half_len - 3.0 {
                // Headlamp glass at the leading end, so a facing reads which way
                // round it is and not merely which axis it is on. Only the
                // engine carries one — a lamp on every car would read as a line
                // of separate trains.
                PLASTER_L
            } else if leading && t > half_len - 6.0 {
                dark
            } else if s.abs() <= 1.0 {
                // The roof line, and the only light step on the body.
                light
            } else if s.abs() <= 2.0 {
                mid
            } else {
                match kind {
                    // Carriage windows along the flanks.
                    TrainKind::Transit => WIN_DARK,
                    // A timber load, ribbed along the run.
                    TrainKind::Transport => {
                        if (t + half_len) as i32 % 4 == 0 {
                            dark
                        } else {
                            WOOD_M
                        }
                    }
                }
            };
            let p = along * t + across * s;
            canvas.put(p.x.round() as i32, p.y.round() as i32, rgba(color));
            s += WALK_STEP;
        }
        t += WALK_STEP;
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::track::{clock_separation, DIR16};

    fn realised_bearings() -> Vec<f32> {
        (0..DIR_COUNT)
            .map(|d| {
                let (dx, dy) = DIR16[d];
                let deg = (dx as f32).atan2(dy as f32).to_degrees();
                if deg < 0.0 {
                    deg + 360.0
                } else {
                    deg
                }
            })
            .collect()
    }

    fn sorted(mut v: Vec<f32>) -> Vec<f32> {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v
    }

    /// The debt item, asserted: the bank's realised half **is** the `atan2` set
    /// of the sixteen lattice directions, exactly, with no rounding anywhere.
    #[test]
    fn the_bank_holds_the_realised_bearings_exactly() {
        let baked = sorted((0..BANK_ENTRIES).step_by(2).map(bank_bearing_deg).collect());
        let realised = sorted(realised_bearings());
        assert_eq!(baked.len(), DIR_COUNT);
        for (a, b) in baked.iter().zip(realised.iter()) {
            assert_eq!(a, b, "bank bearing {a} is not a realised tangent");
        }
        // Every direction can name its own entry, and gets its own bearing back.
        for dir in 0..DIR_COUNT {
            let (dx, dy) = DIR16[dir];
            let want = (dx as f32).atan2(dy as f32).to_degrees().rem_euclid(360.0);
            assert!(
                (bank_bearing_deg(entry_for_dir(dir)) - want).abs() < 1e-3,
                "dir {dir} maps to the wrong entry"
            );
        }
    }

    /// The failure the debt item names: an even 32-rose misses the knight's
    /// moves by 4.07°, well past the `spritebank` plate's 2.81° target. The
    /// realised bank misses them by nothing.
    #[test]
    fn an_even_rose_would_miss_the_knights_moves() {
        let even: Vec<f32> = (0..32).map(|i| i as f32 * 11.25).collect();
        let worst = realised_bearings()
            .into_iter()
            .map(|want| {
                even.iter()
                    .map(|e| (e - want).abs().min(360.0 - (e - want).abs()))
                    .fold(f32::MAX, f32::min)
            })
            .fold(0.0f32, f32::max);
        assert!(
            (worst - 4.07).abs() < 0.01,
            "an even bank is {worst}deg out, not 4.07"
        );

        // ... and the bank we bake is exact.
        let baked: Vec<f32> = (0..BANK_ENTRIES).map(bank_bearing_deg).collect();
        let ours = realised_bearings()
            .into_iter()
            .map(|want| {
                baked
                    .iter()
                    .map(|e| (e - want).abs().min(360.0 - (e - want).abs()))
                    .fold(f32::MAX, f32::min)
            })
            .fold(0.0f32, f32::max);
        assert!(ours < 1e-3, "the bank is {ours}deg off its own rose");

        // The banks are genuinely different sets, not the same list reordered.
        let mismatched = baked
            .iter()
            .filter(|b| even.iter().all(|e| (*e - **b).abs() > 1e-3))
            .count();
        assert!(
            mismatched >= 24,
            "only {mismatched} of 32 entries differ from an even rose"
        );
    }

    /// The odd entries bisect the realised step they sit in, so the sweep
    /// through a curve is even.
    #[test]
    fn midpoints_bisect_the_realised_steps() {
        for clock in 0..DIR_COUNT {
            let a = bank_bearing_deg(clock * 2);
            let mid = bank_bearing_deg(clock * 2 + 1);
            let b = bank_bearing_deg((clock * 2 + 2) % BANK_ENTRIES);
            let sweep = |from: f32, to: f32| (to - from).rem_euclid(360.0);
            let first = sweep(a, mid);
            let second = sweep(mid, b);
            assert!(
                (first - second).abs() < 1e-2,
                "step {clock}: {first} then {second} is not a bisection"
            );
            // And the realised rose's own steps, halved: 26.57 -> 13.28.
            assert!(
                (9.2..=13.3).contains(&first),
                "step {clock} sweeps {first}deg"
            );
        }
    }

    /// Thirty-two distinct headings, none repeated.
    #[test]
    fn the_bank_is_thirty_two_distinct_bearings() {
        let bearings: Vec<f32> = (0..BANK_ENTRIES).map(bank_bearing_deg).collect();
        assert_eq!(bearings.len(), 32);
        for a in 0..BANK_ENTRIES {
            for b in (a + 1)..BANK_ENTRIES {
                assert!(
                    (bearings[a] - bearings[b]).abs() > 1e-3,
                    "entries {a} and {b} are the same bearing"
                );
            }
            assert!((0.0..360.0).contains(&bearings[a]));
        }
    }

    /// A midpoint exists exactly for the pairs a train can actually sweep
    /// through — one rose step — and for no others.
    #[test]
    fn midpoints_exist_only_between_adjacent_directions() {
        for a in 0..DIR_COUNT {
            for b in 0..DIR_COUNT {
                let entry = entry_between(a, b);
                assert_eq!(
                    entry.is_some(),
                    clock_separation(a, b) == 1,
                    "{a} -> {b} separation {} gave {entry:?}",
                    clock_separation(a, b)
                );
                if let Some(entry) = entry {
                    assert_eq!(entry % 2, 1, "a midpoint must be an odd entry");
                    // Symmetric: the sweep is the same heading either way round.
                    assert_eq!(entry_between(b, a), Some(entry));
                    // And it really does lie half way between the two, the
                    // short way round — including across the wrap at north.
                    let want = {
                        let (x, y) = (bearing_deg(a), bearing_deg(b));
                        let mut delta = (y - x).rem_euclid(360.0);
                        if delta > 180.0 {
                            delta -= 360.0;
                        }
                        (x + delta * 0.5).rem_euclid(360.0)
                    };
                    assert!(
                        (bank_bearing_deg(entry) - want).abs() < 1e-2,
                        "{a} -> {b}: entry {entry} is {}, wanted {want}",
                        bank_bearing_deg(entry)
                    );
                }
            }
        }
    }

    /// A straight run holds one facing the whole way; a one-step curve sweeps
    /// through the midpoint instead of snapping at the node.
    #[test]
    fn a_curve_sweeps_and_a_straight_does_not() {
        // East all the way through: nothing to interpolate.
        for t in [0.0, 0.1, 0.5, 0.9, 1.0] {
            assert_eq!(facing_entry(Some(2), 2, Some(2), t), entry_for_dir(2));
        }

        // N -> NNE: one rose step, so the exit of the leg eases.
        let mid = entry_between(0, 8).expect("N and NNE are adjacent");
        assert_eq!(facing_entry(None, 0, Some(8), 0.5), entry_for_dir(0));
        assert_eq!(facing_entry(None, 0, Some(8), 0.95), mid);
        // ... and the next leg eases out of the same midpoint.
        assert_eq!(facing_entry(Some(0), 8, None, 0.05), mid);
        assert_eq!(facing_entry(Some(0), 8, None, 0.5), entry_for_dir(8));

        // A two-step turn is not a sweep the bank can draw, so it shows the
        // leg's own bearing rather than a heading the train never holds.
        assert_eq!(facing_entry(None, 0, Some(1), 0.95), entry_for_dir(0));
    }

    /// Direction is a different sprite: every one of the thirty-two facings has
    /// to produce different pixels from every other (brief 01 §2.2).
    #[test]
    fn every_facing_is_a_distinct_sprite() {
        for kind in [TrainKind::Transit, TrainKind::Transport] {
            let cells: Vec<Vec<u8>> = (0..BANK_ENTRIES)
                .map(|entry| paint_facing(kind, TrainPart::Loco, entry).px)
                .collect();
            for a in 0..BANK_ENTRIES {
                for b in (a + 1)..BANK_ENTRIES {
                    assert_ne!(cells[a], cells[b], "{kind:?} entries {a} and {b} match");
                }
            }
        }
    }

    /// The two kinds are told apart without reading a panel.
    #[test]
    fn the_two_kinds_do_not_draw_alike() {
        for entry in 0..BANK_ENTRIES {
            assert_ne!(
                paint_facing(TrainKind::Transit, TrainPart::Loco, entry).px,
                paint_facing(TrainKind::Transport, TrainPart::Loco, entry).px,
                "entry {entry} draws the same for both kinds"
            );
        }
    }

    /// Bake determinism: the bank is a pure function of kind and bearing, with
    /// no hash, no clock and no screen position in it (brief 01 §2.4).
    #[test]
    fn the_bake_is_deterministic() {
        for entry in [0usize, 1, 7, 18, 31] {
            let a = paint_facing(TrainKind::Transit, TrainPart::Loco, entry);
            let b = paint_facing(TrainKind::Transit, TrainPart::Loco, entry);
            assert_eq!(a.px, b.px, "entry {entry} baked differently twice");
        }
    }

    /// An east-facing body reads its own cross-section: the lamp leads, the
    /// outline rings it, and nothing spills outside the cell.
    #[test]
    fn a_facing_is_a_drawn_body_not_a_rectangle() {
        let canvas = paint_facing(TrainKind::Transit, TrainPart::Loco, entry_for_dir(2));
        assert_eq!(canvas.at(8, 0), rgba(PLASTER_L), "no lamp at the leading end");
        assert_eq!(canvas.at(-9, 0), rgba(OUTLINE), "no outline at the tail");
        assert_eq!(canvas.at(0, 0), rgba(ROOF_SLATE_L), "no roof line");
        // Nothing beyond the body.
        assert_eq!(canvas.at(12, 0), [0; 4]);
        assert_eq!(canvas.at(0, 6), [0; 4]);
    }

    /// Bake on demand, once per distinct facing (brief 01 §2.5).
    #[test]
    fn the_bank_bakes_each_facing_once() {
        let mut app = App::new();
        app.init_resource::<Assets<Image>>();
        let images = &mut app.world_mut().resource_mut::<Assets<Image>>();
        let mut bank = TrainBank::default();

        let first = bank.get(images, TrainKind::Transit, 0);
        let again = bank.get(images, TrainKind::Transit, 0);
        assert_eq!(bank.baked(), 1);
        assert_eq!(first.id(), again.id(), "the cache must reuse its handle");

        bank.get(images, TrainKind::Transit, 1);
        bank.get(images, TrainKind::Transport, 0);
        assert_eq!(bank.baked(), 3);

        // The whole bank, both kinds — the `spritebank` plate measures a full
        // rebake at under a millisecond and this must stay in that world.
        for kind in [TrainKind::Transit, TrainKind::Transport] {
            for entry in 0..BANK_ENTRIES {
                bank.get(images, kind, entry);
            }
        }
        assert_eq!(bank.baked(), BANK_ENTRIES * 2);
    }

    #[test]
    fn a_facing_cell_honours_the_pixel_contract() {
        assert_eq!(CELL, rail_map::TILE_SIZE as u32);
        let image = cell_image(paint_facing(TrainKind::Transport, TrainPart::Loco, 5));
        assert_eq!(image.width(), CELL);
        assert_eq!(image.height(), CELL);
        assert!(matches!(image.sampler, ImageSampler::Descriptor(_)));
    }
}
