//! The numeric floor the generator stands on: elevation bands, value noise,
//! and the two relaxations that keep a placed landform legal.
//!
//! # Why bands
//!
//! Design 02 §2.3 makes legibility a *generation* requirement: "elevation
//! resolves into a small number of discrete bands with visible steps between
//! them." So height is never a continuous field that happens to get quantised
//! for drawing — it **is** six values, and everything else is arranged around
//! what those six values mean to the rest of the game:
//!
//! | Band | Height | `rail_sim` reads it as | Renderer draws |
//! | --- | --- | --- | --- |
//! | 0 | 0 | flat plains, 1× | grass dark |
//! | 1 | 4 | plains, 1× on the flat | grass mid |
//! | 2 | 7 | hills, 3× on the flat | hill mid |
//! | 3 | 10 | hills, 3× on the flat | hill light |
//! | 4 | 13 | high mountain band, 10× | rock mid |
//! | 5 | 16 | **refused** — the wall | rock cap |
//!
//! The gaps are chosen, not rounded. Every step is 3 or 4 — at 3 the terrain
//! renderer draws a shadowed bank, at 4 (`MAX_GRADE`, the last delta track may
//! cross) a full banded cliff face. So *every* band boundary on a generated map
//! is a visible edge, and none of them is an invisible tax. Staying inside a band
//! is the cheap contour route; crossing one is the 6× cut-and-fill (§3.2).

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::features::Surface;

/// Tile height of each elevation band.
pub(crate) const BAND_HEIGHTS: [i8; 6] = [0, 4, 7, 10, 13, 16];

/// Highest band index.
pub(crate) const TOP_BAND: i8 = (BAND_HEIGHTS.len() - 1) as i8;

/// First band that `rail_sim` refuses to build on — the ridge crest.
pub(crate) const ROCK_BAND: i8 = TOP_BAND;

/// Highest band a tile touching water may reach.
///
/// Zero — the bank is the bottom of the ladder. Water sits at height −1 against
/// it, so the bank's local relief is 1 and track along a river costs the 1× base
/// rate. That is design 02 §2.2's "valleys — natural corridors, cheap to build
/// along" made literal: every watercourse cuts a cheap route through whatever it
/// crosses, and the relaxation then steps the ground back up one band per tile on
/// either side, which is the valley wall.
///
/// A band-1 bank would be relief 5 — legal, but the 6× cut-and-fill rate, which
/// would make the corridor the *expensive* line instead of the obvious one.
pub(crate) const SHORE_BAND: i8 = 0;

/// How far from water the low ground reaches. See [`Canvas::clamp_shores`].
pub(crate) const SHORE_APRON: u16 = 2;

/// Highest raw band that gets the full apron.
///
/// A river through open country spreads a floodplain either side of it; a river
/// cutting through high ground keeps its banks and reads as a gorge. Without this
/// the wider apron would plane the ridges down wherever a watercourse crossed
/// them, and the map would run short of the rock that makes a wall.
pub(crate) const APRON_LOWLAND_MAX: i8 = 2;

/// Depth of the deepest open sea, and of the middle of a wide river.
pub(crate) const SEA_DEPTH_MAX: i32 = 6;
pub(crate) const INLAND_DEPTH_MAX: i32 = 3;

#[inline]
pub(crate) fn band_height(band: i8) -> i8 {
    BAND_HEIGHTS[band.clamp(0, TOP_BAND) as usize]
}

/// Deterministic 64-bit mix (SplitMix64 finaliser).
#[inline]
pub(crate) fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// An independent random stream for one generation phase.
///
/// Phases draw from their own stream so that changing an option — two ridges
/// instead of three — re-shapes that phase without shifting the coastline out
/// from under the player. On the New Map screen that is the difference between
/// an option that steers and an option that re-rolls.
pub(crate) fn stream(seed: u64, salt: u64) -> StdRng {
    StdRng::seed_from_u64(splitmix64(seed ^ splitmix64(salt)))
}

/// Phase salts. Never reuse or reorder these — each one is a world's worth of
/// layout that a player may have shared as a seed.
pub(crate) mod salt {
    pub const COAST: u64 = 0x_c0a5_7000;
    pub const RIDGE: u64 = 0x_71d6_e000;
    pub const PLATEAU: u64 = 0x_91a7_ea00;
    pub const BASIN: u64 = 0x_ba51_0000;
    pub const VALLEY: u64 = 0x_7a11_e300;
    pub const GRAIN: u64 = 0x_6a41_0e00;
    pub const RIVER: u64 = 0x_81ee_5000;
    pub const LAKE: u64 = 0x_1a4e_0000;
    pub const SITES: u64 = 0x_5177_e500;
}

/// Smoothstep-interpolated value noise on a coarse lattice.
pub(crate) struct ValueNoise {
    gw: usize,
    gh: usize,
    cells: Vec<f32>,
}

impl ValueNoise {
    pub(crate) fn new(gw: usize, gh: usize, rng: &mut StdRng) -> Self {
        let gw = gw.max(2);
        let gh = gh.max(2);
        let mut cells = vec![0.0f32; gw * gh];
        for c in &mut cells {
            *c = rng.gen_range(-1.0..1.0);
        }
        Self { gw, gh, cells }
    }

    /// Sample at normalised `(u, v)` in `[0, 1]`.
    pub(crate) fn sample(&self, u: f32, v: f32) -> f32 {
        let fx = u.clamp(0.0, 1.0) * (self.gw - 1) as f32;
        let fy = v.clamp(0.0, 1.0) * (self.gh - 1) as f32;
        let x0 = (fx.floor() as usize).min(self.gw - 1);
        let y0 = (fy.floor() as usize).min(self.gh - 1);
        let x1 = (x0 + 1).min(self.gw - 1);
        let y1 = (y0 + 1).min(self.gh - 1);
        let tx = smoothstep(fx - x0 as f32);
        let ty = smoothstep(fy - y0 as f32);
        let at = |x: usize, y: usize| self.cells[y * self.gw + x];
        let a = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
        let b = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
        a * (1.0 - ty) + b * ty
    }
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A few octaves of [`ValueNoise`], summed. Decoration only — see
/// [`crate::options::TerrainStyle::grain`].
pub(crate) struct Grain {
    layers: Vec<(ValueNoise, f32)>,
}

impl Grain {
    pub(crate) fn new(rng: &mut StdRng) -> Self {
        let layers = [(5usize, 1.0f32), (11, 0.5), (23, 0.25)]
            .into_iter()
            .map(|(cells, weight)| (ValueNoise::new(cells, cells, rng), weight))
            .collect();
        Self { layers }
    }

    pub(crate) fn sample(&self, u: f32, v: f32) -> f32 {
        let mut total = 0.0;
        let mut norm = 0.0;
        for (noise, weight) in &self.layers {
            total += noise.sample(u, v) * weight;
            norm += weight;
        }
        total / norm.max(f32::EPSILON)
    }
}

/// The mutable world under construction: one band and one surface class per tile.
pub(crate) struct Canvas {
    pub(crate) w: usize,
    pub(crate) h: usize,
    pub(crate) band: Vec<i8>,
    pub(crate) surface: Vec<Surface>,
}

impl Canvas {
    pub(crate) fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            band: vec![0; w * h],
            surface: vec![Surface::Land; w * h],
        }
    }

    #[inline]
    pub(crate) fn idx(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return None;
        }
        Some(y as usize * self.w + x as usize)
    }

    #[inline]
    pub(crate) fn at(&self, x: i32, y: i32) -> usize {
        (y as usize) * self.w + (x as usize)
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.w * self.h
    }

    #[inline]
    pub(crate) fn is_land(&self, x: i32, y: i32) -> bool {
        self.idx(x, y)
            .is_some_and(|i| self.surface[i] == Surface::Land)
    }

    #[inline]
    pub(crate) fn is_water(&self, x: i32, y: i32) -> bool {
        self.idx(x, y).is_some_and(|i| self.surface[i].is_water())
    }

    /// Distance in tiles from every cell to the nearest cell matching `seed`,
    /// under the 4-neighbour metric. Two raster sweeps, exact.
    pub(crate) fn distance_to(&self, seed: impl Fn(usize) -> bool) -> Vec<u16> {
        let far = u16::MAX / 2;
        let mut dist: Vec<u16> = (0..self.len())
            .map(|i| if seed(i) { 0 } else { far })
            .collect();
        for y in 0..self.h {
            for x in 0..self.w {
                let i = y * self.w + x;
                let mut best = dist[i];
                if x > 0 {
                    best = best.min(dist[i - 1] + 1);
                }
                if y > 0 {
                    best = best.min(dist[i - self.w] + 1);
                }
                dist[i] = best;
            }
        }
        for y in (0..self.h).rev() {
            for x in (0..self.w).rev() {
                let i = y * self.w + x;
                let mut best = dist[i];
                if x + 1 < self.w {
                    best = best.min(dist[i + 1] + 1);
                }
                if y + 1 < self.h {
                    best = best.min(dist[i + self.w] + 1);
                }
                dist[i] = best;
            }
        }
        dist
    }

    /// Lower land until no two neighbours differ by more than one band, with
    /// water pinned at the bottom.
    ///
    /// This is what turns placed landforms into terrain the game can actually be
    /// played on. Every remaining step is one band — 3 or 4 height — which is at
    /// most `MAX_GRADE`, so **track can climb anything that is not rock**. The
    /// wall in a ridge is then the rock crest and nothing else, which is the only
    /// way §2.2's "a small number of passes" can be a true statement about a map
    /// rather than a hope.
    ///
    /// It only ever lowers, so a ridge keeps its crest and loses only the flanks
    /// it could not have supported — and terrain rises exactly one band per tile
    /// away from every shoreline, which is what carves a river's valley walls.
    pub(crate) fn relax_bands(&mut self) {
        for i in 0..self.len() {
            if self.surface[i].is_water() {
                self.band[i] = 0;
            }
        }
        for y in 0..self.h {
            for x in 0..self.w {
                let i = y * self.w + x;
                let mut best = self.band[i];
                if x > 0 {
                    best = best.min(self.band[i - 1] + 1);
                }
                if y > 0 {
                    best = best.min(self.band[i - self.w] + 1);
                }
                self.band[i] = best;
            }
        }
        for y in (0..self.h).rev() {
            for x in (0..self.w).rev() {
                let i = y * self.w + x;
                let mut best = self.band[i];
                if x + 1 < self.w {
                    best = best.min(self.band[i + 1] + 1);
                }
                if y + 1 < self.h {
                    best = best.min(self.band[i + self.w] + 1);
                }
                self.band[i] = best;
            }
        }
    }

    /// Tidy the rock mask: fill single-tile notches, drop lone specks.
    ///
    /// A crest edge roughened by grain sprouts one-tile bays, and every one of
    /// them reads to a pass finder — and to the player — as a way through. Twenty
    /// of those is the "texture" design 02 §2.2 warns about, so they are closed.
    /// Both moves are band-legal by construction: relaxation already guarantees
    /// that anything touching rock is at most one band below it, so promoting a
    /// well-surrounded tile cannot open a two-band step, and demoting one never
    /// can.
    pub(crate) fn tidy_rock(&mut self) {
        let rock_neighbours = |canvas: &Self, x: i32, y: i32| {
            let mut n = 0;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    if canvas
                        .idx(x + dx, y + dy)
                        .is_some_and(|i| canvas.band[i] >= ROCK_BAND)
                    {
                        n += 1;
                    }
                }
            }
            n
        };

        let mut changes: Vec<(usize, i8)> = Vec::new();
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                let i = self.at(x, y);
                if self.surface[i].is_water() {
                    continue;
                }
                let n = rock_neighbours(self, x, y);
                // Promote only where every orthogonal neighbour is already within
                // one band of rock, or the fill would open a two-band step — an
                // unclimbable seam the placement rules refuse and the renderer
                // draws as a wall nobody put there.
                let seated = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().all(|(dx, dy)| {
                    self.idx(x + dx, y + dy)
                        .is_none_or(|k| self.band[k] >= ROCK_BAND - 1)
                });
                if self.band[i] == ROCK_BAND - 1 && n >= 5 && seated {
                    changes.push((i, ROCK_BAND));
                } else if self.band[i] >= ROCK_BAND && n <= 2 {
                    changes.push((i, ROCK_BAND - 1));
                }
            }
        }
        for (i, band) in changes {
            self.band[i] = band;
        }
    }

    /// Pin the ground either side of every watercourse to [`SHORE_BAND`], out to
    /// [`SHORE_APRON`] tiles. Run before [`Self::relax_bands`], which then steps
    /// the land back up one band per tile and turns the apron into a valley.
    ///
    /// Two tiles, not one, and that width is the point. A single pinned tile is
    /// still a boundary tile — it has band-1 ground on its landward side, so
    /// `local_slope` is 4 and track along the bank pays the 6× cut-and-fill rate.
    /// At two tiles the inner one sees only water and its own band, costs 1×, and
    /// the river finally reads to the cost model the way §2.2 describes it: a
    /// corridor that is cheap to build along.
    pub(crate) fn clamp_shores(&mut self) {
        let from_water = self.distance_to(|i| self.surface[i].is_water());
        for ((band, surface), &distance) in self
            .band
            .iter_mut()
            .zip(&self.surface)
            .zip(from_water.iter())
        {
            if surface.is_water() || *band <= SHORE_BAND {
                continue;
            }
            let reach = if *band <= APRON_LOWLAND_MAX {
                SHORE_APRON
            } else {
                1
            };
            if distance <= reach {
                *band = SHORE_BAND;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::{MAX_GRADE, MOUNTAIN_HEIGHT_MIN};

    #[test]
    fn bands_step_by_what_the_renderer_and_the_sim_both_read() {
        // Every step is a drawn edge (3 = bank, 4 = full cliff face) and no step
        // exceeds what track may climb.
        for pair in BAND_HEIGHTS.windows(2) {
            let delta = pair[1] - pair[0];
            assert!(
                (3..=MAX_GRADE as i8).contains(&delta),
                "band step {delta} is neither a drawn edge nor climbable"
            );
        }
        // Exactly one band is the wall, and the one below it is buildable.
        assert!(band_height(ROCK_BAND) >= MOUNTAIN_HEIGHT_MIN);
        assert!(band_height(ROCK_BAND - 1) < MOUNTAIN_HEIGHT_MIN);
    }

    #[test]
    fn relaxing_leaves_no_step_bigger_than_one_band() {
        // A block of high ground, not a spike: relaxation is a *lower envelope*,
        // so a lone peak with nothing holding it up is correctly cut down to one
        // band above its neighbours. Only ground wide enough to earn its height
        // keeps it — which is why ridges are drawn with a width.
        let mut canvas = Canvas::new(16, 16);
        for y in 3..13i32 {
            for x in 3..13i32 {
                let i = canvas.at(x, y);
                canvas.band[i] = TOP_BAND;
            }
        }
        canvas.relax_bands();
        for y in 0..16i32 {
            for x in 0..16i32 {
                for (dx, dy) in [(1, 0), (0, 1)] {
                    let Some(n) = canvas.idx(x + dx, y + dy) else {
                        continue;
                    };
                    let delta = (canvas.band[canvas.at(x, y)] - canvas.band[n]).abs();
                    assert!(delta <= 1, "step of {delta} bands at ({x}, {y})");
                }
            }
        }
        // The middle survives; the flanks are what gave way.
        assert_eq!(canvas.band[canvas.at(8, 8)], TOP_BAND);
        assert_eq!(canvas.band[canvas.at(0, 8)], 0);
    }

    #[test]
    fn water_pins_its_own_shoreline_down() {
        let mut canvas = Canvas::new(8, 8);
        canvas.band.fill(TOP_BAND);
        let shore = canvas.at(0, 0);
        canvas.surface[shore] = Surface::Sea;
        canvas.clamp_shores();
        canvas.relax_bands();
        // High ground keeps its bank and reads as a gorge, so only one tile of
        // this all-mountain test map is pinned down.
        assert_eq!(canvas.band[canvas.at(0, 0)], 0);
        assert_eq!(canvas.band[canvas.at(1, 0)], SHORE_BAND);
        assert_eq!(canvas.band[canvas.at(2, 0)], SHORE_BAND + 1);
        assert_eq!(canvas.band[canvas.at(7, 7)], TOP_BAND);
    }

    #[test]
    fn distance_transform_is_exact_under_the_four_neighbour_metric() {
        let canvas = Canvas::new(5, 5);
        let dist = canvas.distance_to(|i| i == canvas.at(2, 2));
        assert_eq!(dist[canvas.at(2, 2)], 0);
        assert_eq!(dist[canvas.at(0, 0)], 4);
        assert_eq!(dist[canvas.at(4, 2)], 2);
    }

    #[test]
    fn streams_are_independent_and_deterministic() {
        let mut a = stream(42, salt::RIDGE);
        let mut b = stream(42, salt::RIDGE);
        let mut c = stream(42, salt::RIVER);
        let draw = |r: &mut StdRng| r.gen_range(0u32..1_000_000);
        assert_eq!(draw(&mut a), draw(&mut b));
        assert_ne!(draw(&mut a), draw(&mut c));
    }
}
