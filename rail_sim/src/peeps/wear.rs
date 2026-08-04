//! Desire paths — ground that remembers being walked on.
//!
//! Brief 16 in one paragraph: a tile a peep walks across takes a **footfall**;
//! enough footfalls wear the grass through to bare earth in three visible steps;
//! and when the walking stops the grass comes back, slowly. The town's habits
//! draw themselves on the ground, and nobody had to be told to look.
//!
//! # What a footfall is, and what it deliberately is not
//!
//! **One footfall is one walkable tile entered by a peep who is walking.** Not a
//! tick of standing on it — a peep occupies a tile for
//! [`WALK_TICKS_PER_TILE`](super::journey::WALK_TICKS_PER_TILE) ticks, so
//! counting ticks would make wear a function of walking *speed* — and
//! emphatically not a rendered frame. A six-tile lane takes six footfalls per
//! crossing whatever the speed multiplier is and whatever the frame rate is.
//!
//! Everything here is integer, tick-scheduled and iterated in sorted order, so
//! two runs of the same world wear the same ground to the same value.
//!
//! # What presentation is allowed to know
//!
//! Not the wear number. Wear moves on almost every tick in a living town, and a
//! renderer keyed on "did this change" would re-composite continuously — the
//! exact regression [`TerrainDirty`](../../../rail_town/src/map/terrain/chunk.rs)
//! carries scar tissue for. So wear is quantised to four **levels**, and the only
//! thing that crosses into presentation is a list of tiles whose *level* moved.
//! A tile climbing from 700 to 701 produces no event and costs nothing.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;

use crate::ids::TileCoord;
use crate::stations::StationService;
use crate::track::TrackTerrain;

use super::budget::PeepDetail;
use super::journey::PeepPosition;
use super::resident::Peep;
use super::PeepId;

// ── The numbers (brief 16 §2.2) ────────────────────────────────────────────

/// Wear one crossing deposits on a tile.
pub const WEAR_PER_FOOTFALL: u16 = 64;

/// Saturation. About nineteen crossings of headroom above [`WEAR_BARE`], which
/// is what bounds how long an abandoned trunk lane takes to forget itself.
pub const WEAR_MAX: u16 = 1_200;

/// Level 1 — scattered earth showing through thinning grass. Four footfalls.
pub const WEAR_FAINT: u16 = 256;
/// Level 2 — a broken earth ribbon. Ten footfalls.
pub const WEAR_WORN: u16 = 640;
/// Level 3 — trodden earth with tufts at the corners. Sixteen footfalls.
pub const WEAR_BARE: u16 = 1_024;

/// How far *below* a threshold wear must fall before the level drops again.
///
/// Without this, a tile parked on a boundary flips level every time a footfall
/// lands and every time regrowth ticks it back — one chunk re-composite each
/// way, forever, on every boundary tile in town. With it, falling out of a level
/// costs sixteen regrowth steps, which is half a sim-day.
pub const WEAR_RELEASE: u16 = 32;

/// Ticks between regrowth steps — 48 sim-minutes, thirty steps per sim-day.
pub const REGROWTH_INTERVAL_TICKS: u64 = 288;

/// Wear removed per regrowth step. Sixty per sim-day.
pub const REGROWTH_PER_STEP: u16 = 2;

/// Visual levels, including clean ground.
pub const WEAR_LEVELS: usize = 4;

/// How long a peep is remembered after they were last seen.
///
/// This exists only to bound the sightings map against peeps who have left
/// town, so it is deliberately generous: a sim-day is far longer than the gap
/// between two runs of the wear pass for anybody who still exists at full
/// detail. Forgetting a peep who is merely standing still would cost them the
/// first crossing of their next walk, which is a silent under-count and exactly
/// the kind of bug that hides for months.
pub const SIGHTING_MEMORY_TICKS: u64 = super::TICKS_PER_DAY;

/// Pending level changes held before presentation drains them. Past this the
/// list is dropped and a full resync is asked for instead, so a headless sim
/// with no renderer attached cannot grow this without bound.
pub const MAX_PENDING_CHANGES: usize = 4_096;

/// Threshold a level is entered at. Level 0 is clean ground.
#[inline]
pub fn level_threshold(level: u8) -> u16 {
    match level {
        0 => 0,
        1 => WEAR_FAINT,
        2 => WEAR_WORN,
        _ => WEAR_BARE,
    }
}

/// The level a wear value alone implies — the pure quantisation, no hysteresis.
#[inline]
pub fn raw_level(wear: u16) -> u8 {
    if wear >= WEAR_BARE {
        3
    } else if wear >= WEAR_WORN {
        2
    } else if wear >= WEAR_FAINT {
        1
    } else {
        0
    }
}

/// The level a tile should show, given what it is showing now.
///
/// Rises **exactly** at a threshold. Falls only once wear drops
/// [`WEAR_RELEASE`] below the threshold of the level currently held, which is
/// what stops a boundary tile churning (brief 16 §4.3).
#[inline]
pub fn level_for(wear: u16, current: u8) -> u8 {
    let raw = raw_level(wear);
    if raw >= current {
        return raw;
    }
    if wear < level_threshold(current).saturating_sub(WEAR_RELEASE) {
        raw
    } else {
        current
    }
}

/// One tile whose drawn level moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathLevelChange {
    pub tile: TileCoord,
    pub level: u8,
}

/// Where the town has walked, and how recently.
///
/// Dense wear plus a sorted index of the tiles that carry any, so regrowth costs
/// one decrement per *worn* tile rather than a sweep of the whole map.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct PathWear {
    width: u32,
    height: u32,
    /// Row-major wear, `y * width + x`.
    wear: Vec<u16>,
    /// Row-major drawn level — the hysteresis state, not a pure function of
    /// `wear`, which is why it is stored rather than recomputed.
    levels: Vec<u8>,
    /// Indices with non-zero wear, ascending. Sorted iteration, by construction.
    worn: BTreeSet<u32>,
    /// Level transitions awaiting a renderer.
    changes: Vec<PathLevelChange>,
    /// Set when `changes` overflowed: presentation must rebuild from scratch.
    resync: bool,
    /// Tile each full-detail peep was last seen on, with the tick it was seen,
    /// so a peep who has left town stops being remembered.
    last_seen: BTreeMap<PeepId, (TileCoord, u64)>,
    /// Tick the last regrowth step ran on.
    last_regrowth_tick: u64,
}

impl PathWear {
    /// An empty map of a given size.
    pub fn new(width: u32, height: u32) -> Self {
        let len = (width as usize).saturating_mul(height as usize);
        Self {
            width,
            height,
            wear: vec![0; len],
            levels: vec![0; len],
            ..Default::default()
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Drop everything and resize. Used when a world of another size arrives.
    pub fn resize(&mut self, width: u32, height: u32) {
        *self = Self::new(width, height);
        self.resync = true;
    }

    #[inline]
    fn index(&self, tile: TileCoord) -> Option<u32> {
        if tile.x < 0 || tile.y < 0 {
            return None;
        }
        let (x, y) = (tile.x as u32, tile.y as u32);
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(y * self.width + x)
    }

    /// Tile a row-major index refers to.
    #[inline]
    pub fn tile_of(&self, index: u32) -> TileCoord {
        if self.width == 0 {
            return TileCoord { x: 0, y: 0 };
        }
        TileCoord {
            x: (index % self.width) as i32,
            y: (index / self.width) as i32,
        }
    }

    /// Wear on a tile, or zero off the map.
    pub fn wear_at(&self, tile: TileCoord) -> u16 {
        self.index(tile)
            .and_then(|i| self.wear.get(i as usize).copied())
            .unwrap_or(0)
    }

    /// Drawn level of a tile, or zero off the map.
    pub fn level_at(&self, tile: TileCoord) -> u8 {
        self.index(tile)
            .and_then(|i| self.levels.get(i as usize).copied())
            .unwrap_or(0)
    }

    /// Every tile carrying any wear at all, ascending by index.
    pub fn worn_tiles(&self) -> impl Iterator<Item = (TileCoord, u16, u8)> + '_ {
        self.worn.iter().map(move |&i| {
            (
                self.tile_of(i),
                self.wear[i as usize],
                self.levels[i as usize],
            )
        })
    }

    /// Every tile currently drawing a path, ascending by index.
    pub fn drawn_tiles(&self) -> impl Iterator<Item = (TileCoord, u8)> + '_ {
        self.worn.iter().filter_map(move |&i| {
            let level = self.levels[i as usize];
            (level > 0).then(|| (self.tile_of(i), level))
        })
    }

    /// How many tiles carry any wear (diagnostics, and the regrowth cost).
    pub fn worn_count(&self) -> usize {
        self.worn.len()
    }

    /// True when presentation must rebuild every path from scratch.
    pub fn needs_resync(&self) -> bool {
        self.resync
    }

    /// Take the pending level changes, clearing the resync flag with them.
    ///
    /// The caller is the one renderer currently drawing; when `resync` comes
    /// back true the returned list is incomplete and the caller must walk
    /// [`Self::drawn_tiles`] instead.
    pub fn drain_changes(&mut self) -> (Vec<PathLevelChange>, bool) {
        let resync = self.resync;
        self.resync = false;
        (std::mem::take(&mut self.changes), resync)
    }

    /// Ask for a full rebuild — a view flip, a fresh world, a load.
    pub fn request_resync(&mut self) {
        self.changes.clear();
        self.resync = true;
    }

    fn note_change(&mut self, tile: TileCoord, level: u8) {
        if self.resync {
            return; // a full rebuild is already owed; individual tiles are moot
        }
        if self.changes.len() >= MAX_PENDING_CHANGES {
            self.request_resync();
            return;
        }
        self.changes.push(PathLevelChange { tile, level });
    }

    /// Apply a wear delta to one tile and reconcile its level.
    fn set_wear(&mut self, index: u32, wear: u16) {
        let i = index as usize;
        let before = self.levels[i];
        self.wear[i] = wear;
        if wear == 0 {
            self.worn.remove(&index);
        } else {
            self.worn.insert(index);
        }
        let after = level_for(wear, before);
        if after != before {
            self.levels[i] = after;
            let tile = self.tile_of(index);
            self.note_change(tile, after);
        }
    }

    /// Record one crossing of a tile. Off-map tiles are ignored.
    pub fn add_footfall(&mut self, tile: TileCoord) {
        let Some(index) = self.index(tile) else {
            return;
        };
        let wear = self.wear[index as usize]
            .saturating_add(WEAR_PER_FOOTFALL)
            .min(WEAR_MAX);
        self.set_wear(index, wear);
    }

    /// One regrowth step over every worn tile, in index order.
    pub fn regrow(&mut self) {
        // Collect first: `set_wear` mutates `worn`, and the order must not
        // depend on how a set behaves while it is being edited.
        let worn: Vec<u32> = self.worn.iter().copied().collect();
        for index in worn {
            let wear = self.wear[index as usize].saturating_sub(REGROWTH_PER_STEP);
            self.set_wear(index, wear);
        }
    }

    /// Replace the whole map from saved data (see [`crate::save`]).
    ///
    /// Levels are recomputed from wear without hysteresis, which is the right
    /// reading of a world whose ground has just come back into existence.
    pub fn restore_from(&mut self, width: u32, height: u32, entries: &[(u32, u16)]) {
        *self = Self::new(width, height);
        for &(index, wear) in entries {
            if wear == 0 || index as usize >= self.wear.len() {
                continue;
            }
            let wear = wear.min(WEAR_MAX);
            self.wear[index as usize] = wear;
            self.levels[index as usize] = raw_level(wear);
            self.worn.insert(index);
        }
        self.resync = true;
    }

    /// Non-zero wear as `(index, wear)` pairs, ascending — the save blob.
    pub fn to_entries(&self) -> Vec<(u32, u16)> {
        self.worn
            .iter()
            .map(|&i| (i, self.wear[i as usize]))
            .collect()
    }
}

/// Record a footfall for every full-detail peep who moved onto a new tile.
///
/// Runs after [`advance_journeys`](super::journey::advance_journeys), so the
/// positions read are the ones this tick produced.
///
/// Three gates, each of them load-bearing:
///
/// - **`walking`** — a peep put down somewhere by a level-of-detail promotion
///   has `walking` false, so a teleport never leaves a mark.
/// - **not water** — a bridge deck is a built structure, not ground. Brief 16
///   §3.2: a bridge never wears.
/// - **the tile actually changed** — the unit is a tile entered, not a tick
///   spent, so standing still on a tile for twenty-four ticks is one footfall
///   and not twenty-four.
pub fn accumulate_path_wear(
    service: Res<StationService>,
    terrain: Option<Res<TrackTerrain>>,
    mut paths: ResMut<PathWear>,
    peeps: Query<(&Peep, &PeepPosition, &PeepDetail)>,
) {
    let tick = service.tick;

    // A world of another size means this map is not about this world.
    if let Some(terrain) = terrain.as_deref() {
        if paths.width != terrain.width() || paths.height != terrain.height() {
            paths.resize(terrain.width(), terrain.height());
        }
    }
    if paths.wear.is_empty() {
        return;
    }

    // Sorted by peep id: iteration order over an ECS query is not a promise,
    // and wear must not depend on archetype layout.
    let mut walkers: Vec<(PeepId, TileCoord, bool)> = peeps
        .iter()
        .filter(|(_, _, detail)| detail.is_full())
        .map(|(peep, pos, _)| (peep.id, pos.tile(), pos.walking))
        .collect();
    walkers.sort_unstable_by_key(|(id, _, _)| *id);

    for (id, tile, walking) in walkers {
        let previous = paths.last_seen.insert(id, (tile, tick));
        if !walking {
            continue;
        }
        let Some((was, _)) = previous else {
            // First sight of this peep — no crossing has been observed yet.
            continue;
        };
        if was == tile {
            continue;
        }
        if terrain
            .as_deref()
            .is_some_and(|t| t.is_water(tile) || !t.contains(tile))
        {
            continue;
        }
        paths.add_footfall(tile);
    }
}

/// Grass back over everything, every [`REGROWTH_INTERVAL_TICKS`] ticks.
pub fn regrow_paths(service: Res<StationService>, mut paths: ResMut<PathWear>) {
    let tick = service.tick;
    if tick < paths.last_regrowth_tick.saturating_add(REGROWTH_INTERVAL_TICKS) {
        return;
    }
    paths.last_regrowth_tick = tick;
    paths.regrow();

    // Peeps who have left town stop being remembered, so the sightings map is
    // bounded by the living population rather than by every peep who ever was.
    let cutoff = tick.saturating_sub(SIGHTING_MEMORY_TICKS);
    paths.last_seen.retain(|_, (_, seen)| *seen >= cutoff);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peeps::TICKS_PER_DAY;

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    /// Wear and regrowth for one tile over `days`, crossed `crossings` times a
    /// sim-day. Returns the wear it settles at.
    fn simulate(crossings: u32, days: u32) -> u16 {
        let mut paths = PathWear::new(8, 8);
        let steps_per_day = (TICKS_PER_DAY / REGROWTH_INTERVAL_TICKS) as u32;
        for _ in 0..days {
            for _ in 0..crossings {
                paths.add_footfall(tile(1, 1));
            }
            for _ in 0..steps_per_day {
                paths.regrow();
            }
        }
        paths.wear_at(tile(1, 1))
    }

    #[test]
    fn the_time_base_the_brief_quotes_is_the_one_the_code_has() {
        assert_eq!(TICKS_PER_DAY, 8_640);
        // Thirty regrowth steps a sim-day, exactly — no remainder to drift on.
        assert_eq!(TICKS_PER_DAY % REGROWTH_INTERVAL_TICKS, 0);
        assert_eq!(TICKS_PER_DAY / REGROWTH_INTERVAL_TICKS, 30);
        // Sixty per sim-day, which is what every horizon in §2.3 is built on.
        assert_eq!(
            (TICKS_PER_DAY / REGROWTH_INTERVAL_TICKS) as u16 * REGROWTH_PER_STEP,
            60
        );
    }

    #[test]
    fn one_stroll_never_paints() {
        let mut paths = PathWear::new(8, 8);
        paths.add_footfall(tile(2, 2));
        assert_eq!(paths.wear_at(tile(2, 2)), WEAR_PER_FOOTFALL);
        assert_eq!(paths.level_at(tile(2, 2)), 0, "one crossing must draw nothing");

        // And it is gone again inside a sim-day.
        for _ in 0..(TICKS_PER_DAY / REGROWTH_INTERVAL_TICKS) {
            paths.regrow();
        }
        assert_eq!(paths.wear_at(tile(2, 2)), 4);
        assert_eq!(paths.worn_count(), 1);

        // Even three separate strollers in a day leave nothing behind.
        assert_eq!(simulate(3, 1), 132);
        assert_eq!(raw_level(132), 0);
    }

    #[test]
    fn a_commuter_wears_a_path_within_a_few_sim_days() {
        // Out and back, once a day. The brief promises Faint on day four.
        assert!(raw_level(simulate(2, 3)) == 0, "not before day four");
        assert!(
            raw_level(simulate(2, 4)) >= 1,
            "a commuter's lane must read by day four"
        );
        // …and keeps deepening while they keep walking.
        assert_eq!(raw_level(simulate(2, 10)), 2);
        assert_eq!(raw_level(simulate(2, 16)), 3);
    }

    #[test]
    fn a_station_approach_wears_bare_in_under_two_sim_days() {
        // Five commuters converging: ten crossings a day.
        assert!(raw_level(simulate(10, 1)) >= 1);
        assert_eq!(raw_level(simulate(10, 2)), 3, "the trunk goes bare quickly");
    }

    #[test]
    fn wear_saturates_rather_than_wrapping() {
        let mut paths = PathWear::new(4, 4);
        for _ in 0..10_000 {
            paths.add_footfall(tile(0, 0));
        }
        assert_eq!(paths.wear_at(tile(0, 0)), WEAR_MAX);
        assert_eq!(paths.level_at(tile(0, 0)), 3);
    }

    #[test]
    fn regrowth_returns_the_ground_to_clean_over_the_designed_horizon() {
        let mut paths = PathWear::new(4, 4);
        for _ in 0..64 {
            paths.add_footfall(tile(1, 1));
        }
        assert_eq!(paths.wear_at(tile(1, 1)), WEAR_MAX);

        let steps_per_day = (TICKS_PER_DAY / REGROWTH_INTERVAL_TICKS) as u32;
        let mut days_to_clean = None;
        let mut left_bare = None;
        for day in 1..=40u32 {
            for _ in 0..steps_per_day {
                paths.regrow();
            }
            if left_bare.is_none() && paths.level_at(tile(1, 1)) < 3 {
                left_bare = Some(day);
            }
            if paths.wear_at(tile(1, 1)) == 0 {
                days_to_clean = Some(day);
                break;
            }
        }
        // Loses its deepest read quickly…
        assert_eq!(left_bare, Some(4), "bare earth must green over promptly");
        // …but the ghost lingers for the better part of three weeks.
        assert_eq!(days_to_clean, Some(20));
        assert_eq!(paths.level_at(tile(1, 1)), 0);
        assert_eq!(paths.worn_count(), 0, "a clean tile leaves the worn index");
    }

    #[test]
    fn levels_rise_exactly_at_their_thresholds() {
        for (threshold, level) in [(WEAR_FAINT, 1), (WEAR_WORN, 2), (WEAR_BARE, 3)] {
            assert_eq!(raw_level(threshold - 1), level - 1, "early at {threshold}");
            assert_eq!(raw_level(threshold), level, "late at {threshold}");
            // Rising is never held back by hysteresis.
            assert_eq!(level_for(threshold, level - 1), level);
        }
        assert_eq!(raw_level(0), 0);
    }

    #[test]
    fn levels_fall_only_after_the_release_band() {
        for (threshold, level) in [(WEAR_FAINT, 1u8), (WEAR_WORN, 2), (WEAR_BARE, 3)] {
            // Sitting just under the threshold holds the level it has.
            assert_eq!(level_for(threshold - 1, level), level);
            assert_eq!(level_for(threshold - WEAR_RELEASE, level), level);
            // One below the release band and it drops.
            assert_eq!(
                level_for(threshold - WEAR_RELEASE - 1, level),
                raw_level(threshold - WEAR_RELEASE - 1),
                "did not release at {threshold}"
            );
        }
    }

    /// The churn budget, counted rather than asserted: a tile parked on a
    /// boundary must not flip level more than a couple of times a sim-day.
    #[test]
    fn a_tile_parked_on_a_boundary_does_not_churn() {
        let mut paths = PathWear::new(4, 4);
        // Walk it up to exactly the Faint threshold.
        for _ in 0..4 {
            paths.add_footfall(tile(1, 1));
        }
        assert_eq!(paths.wear_at(tile(1, 1)), WEAR_FAINT);
        let (_, _) = paths.drain_changes();

        // A sim-day of one crossing a day against thirty regrowth steps.
        let steps_per_day = (TICKS_PER_DAY / REGROWTH_INTERVAL_TICKS) as u32;
        for _ in 0..10 {
            paths.add_footfall(tile(1, 1));
            for _ in 0..steps_per_day {
                paths.regrow();
            }
        }
        let (changes, resync) = paths.drain_changes();
        assert!(!resync);
        assert!(
            changes.len() <= 2,
            "a boundary tile churned {} times over ten sim-days",
            changes.len()
        );
    }

    #[test]
    fn only_tiles_that_change_level_are_published() {
        let mut paths = PathWear::new(8, 8);
        let (_, _) = paths.drain_changes();

        // Three footfalls: wear moves every time, level never does.
        for _ in 0..3 {
            paths.add_footfall(tile(4, 4));
        }
        let (changes, _) = paths.drain_changes();
        assert!(changes.is_empty(), "sub-threshold wear must be silent");

        // The fourth crosses.
        paths.add_footfall(tile(4, 4));
        let (changes, _) = paths.drain_changes();
        assert_eq!(
            changes,
            vec![PathLevelChange {
                tile: tile(4, 4),
                level: 1
            }]
        );
    }

    #[test]
    fn an_undrained_change_list_asks_for_a_resync_rather_than_growing() {
        let mut paths = PathWear::new(256, 256);
        // Push every tile over the Faint threshold without ever draining.
        'outer: for y in 0..256 {
            for x in 0..256 {
                for _ in 0..4 {
                    paths.add_footfall(tile(x, y));
                }
                if paths.needs_resync() {
                    break 'outer;
                }
            }
        }
        assert!(paths.needs_resync(), "the list grew without bound");
        let (changes, resync) = paths.drain_changes();
        assert!(resync);
        assert!(changes.is_empty());
        assert!(!paths.needs_resync(), "draining clears the flag");
    }

    #[test]
    fn worn_tiles_come_back_in_index_order() {
        let mut paths = PathWear::new(16, 16);
        for t in [tile(9, 3), tile(1, 1), tile(4, 12), tile(0, 0)] {
            paths.add_footfall(t);
        }
        let seen: Vec<TileCoord> = paths.worn_tiles().map(|(t, _, _)| t).collect();
        assert_eq!(seen, vec![tile(0, 0), tile(1, 1), tile(9, 3), tile(4, 12)]);
    }

    #[test]
    fn off_map_footfalls_are_ignored() {
        let mut paths = PathWear::new(4, 4);
        for t in [tile(-1, 0), tile(0, -1), tile(4, 0), tile(0, 4)] {
            paths.add_footfall(t);
        }
        assert_eq!(paths.worn_count(), 0);
        assert_eq!(paths.wear_at(tile(-1, 0)), 0);
        assert_eq!(paths.level_at(tile(9, 9)), 0);
    }

    #[test]
    fn entries_round_trip_through_the_save_shape() {
        let mut paths = PathWear::new(12, 8);
        for _ in 0..5 {
            paths.add_footfall(tile(3, 2));
        }
        paths.add_footfall(tile(7, 6));
        let entries = paths.to_entries();
        assert_eq!(entries.len(), 2);
        assert!(entries.windows(2).all(|w| w[0].0 < w[1].0), "not sorted");

        let mut restored = PathWear::new(1, 1);
        restored.restore_from(12, 8, &entries);
        assert_eq!(restored.wear_at(tile(3, 2)), 5 * WEAR_PER_FOOTFALL);
        assert_eq!(restored.level_at(tile(3, 2)), 1);
        assert_eq!(restored.wear_at(tile(7, 6)), WEAR_PER_FOOTFALL);
        assert_eq!(restored.level_at(tile(7, 6)), 0);
        assert_eq!(restored.to_entries(), entries);
        assert!(restored.needs_resync(), "a restored world must redraw");
    }

    #[test]
    fn a_restored_world_of_another_size_drops_what_it_had() {
        let mut paths = PathWear::new(8, 8);
        paths.add_footfall(tile(1, 1));
        paths.resize(16, 4);
        assert_eq!((paths.width(), paths.height()), (16, 4));
        assert_eq!(paths.worn_count(), 0);
        assert_eq!(paths.wear_at(tile(1, 1)), 0);
    }

    // ── The systems, driven against a real world ───────────────────────────

    use crate::peeps::HouseholdId;
    use bevy_ecs::system::RunSystemOnce;

    /// Flat land with a north-south river at `x = 5`, one dry crossing at y = 3.
    fn walking_world() -> World {
        let mut world = World::new();
        let cells = (0..8u32).flat_map(|y| {
            (0..12u32).map(move |x| (x == 5 && y != 3, 0i8))
        });
        world.insert_resource(TrackTerrain::new(12, 8, cells));
        world.insert_resource(StationService::default());
        world.insert_resource(PathWear::default());
        world
    }

    fn spawn_walker(world: &mut World, id: u64, at: TileCoord) -> Entity {
        world
            .spawn((
                Peep::new(PeepId(id), "Test Walker", at, HouseholdId(0), 0),
                PeepPosition::at_tile(at, id),
                PeepDetail::Full,
            ))
            .id()
    }

    /// Put a peep on a tile as though they had walked there, and run the pass.
    fn walk_onto(world: &mut World, who: Entity, tile: TileCoord) {
        {
            let mut pos = world.get_mut::<PeepPosition>(who).expect("a position");
            pos.snap_to(tile);
            pos.walking = true;
        }
        world
            .run_system_once(accumulate_path_wear)
            .expect("the wear pass runs");
    }

    #[test]
    fn the_pass_sizes_itself_from_the_terrain() {
        let mut world = walking_world();
        spawn_walker(&mut world, 1, tile(1, 1));
        world
            .run_system_once(accumulate_path_wear)
            .expect("the wear pass runs");
        let paths = world.resource::<PathWear>();
        assert_eq!((paths.width(), paths.height()), (12, 8));
    }

    #[test]
    fn a_walked_route_takes_one_footfall_per_tile_entered() {
        let mut world = walking_world();
        let who = spawn_walker(&mut world, 1, tile(1, 1));
        // First sight establishes where they are; it is not a crossing.
        world
            .run_system_once(accumulate_path_wear)
            .expect("the wear pass runs");
        assert_eq!(world.resource::<PathWear>().worn_count(), 0);

        for x in 2..=4 {
            walk_onto(&mut world, who, tile(x, 1));
        }

        let paths = world.resource::<PathWear>();
        assert_eq!(paths.wear_at(tile(1, 1)), 0, "the tile they set off from");
        for x in 2..=4 {
            assert_eq!(
                paths.wear_at(tile(x, 1)),
                WEAR_PER_FOOTFALL,
                "tile {x} took the wrong number of footfalls"
            );
        }
        assert_eq!(paths.worn_count(), 3);
    }

    #[test]
    fn standing_on_a_tile_for_many_ticks_is_still_one_footfall() {
        let mut world = walking_world();
        let who = spawn_walker(&mut world, 1, tile(1, 1));
        world
            .run_system_once(accumulate_path_wear)
            .expect("the wear pass runs");
        walk_onto(&mut world, who, tile(2, 1));
        // Twenty-four ticks is how long a peep occupies one tile.
        for _ in 0..24 {
            world
                .run_system_once(accumulate_path_wear)
                .expect("the wear pass runs");
        }
        assert_eq!(
            world.resource::<PathWear>().wear_at(tile(2, 1)),
            WEAR_PER_FOOTFALL,
            "wear must not be a function of how long a tile is occupied"
        );
    }

    #[test]
    fn a_teleport_leaves_no_mark() {
        let mut world = walking_world();
        let who = spawn_walker(&mut world, 1, tile(1, 1));
        world
            .run_system_once(accumulate_path_wear)
            .expect("the wear pass runs");

        // A level-of-detail promotion puts a peep down without walking them.
        {
            let mut pos = world.get_mut::<PeepPosition>(who).expect("a position");
            pos.snap_to(tile(9, 6));
            pos.walking = false;
        }
        world
            .run_system_once(accumulate_path_wear)
            .expect("the wear pass runs");
        assert_eq!(world.resource::<PathWear>().worn_count(), 0);

        // And the jump itself is not charged to the next real step either.
        walk_onto(&mut world, who, tile(9, 5));
        let paths = world.resource::<PathWear>();
        assert_eq!(paths.wear_at(tile(9, 6)), 0);
        assert_eq!(paths.wear_at(tile(9, 5)), WEAR_PER_FOOTFALL);
    }

    #[test]
    fn an_abstracted_peep_leaves_no_mark() {
        let mut world = walking_world();
        let who = spawn_walker(&mut world, 1, tile(1, 1));
        *world.get_mut::<PeepDetail>(who).expect("a detail") = PeepDetail::Abstract;
        walk_onto(&mut world, who, tile(2, 1));
        assert_eq!(world.resource::<PathWear>().worn_count(), 0);
    }

    #[test]
    fn a_bridge_never_wears() {
        let mut world = walking_world();
        let who = spawn_walker(&mut world, 1, tile(4, 1));
        world
            .run_system_once(accumulate_path_wear)
            .expect("the wear pass runs");
        // Straight across the river, as a peep on a bridge deck would.
        walk_onto(&mut world, who, tile(5, 1));
        walk_onto(&mut world, who, tile(6, 1));

        let paths = world.resource::<PathWear>();
        assert_eq!(paths.wear_at(tile(5, 1)), 0, "a deck is not ground");
        assert_eq!(paths.wear_at(tile(6, 1)), WEAR_PER_FOOTFALL);
        // The dry ford at y = 3 is ordinary ground and does wear.
        walk_onto(&mut world, who, tile(5, 3));
        assert_eq!(
            world.resource::<PathWear>().wear_at(tile(5, 3)),
            WEAR_PER_FOOTFALL
        );
    }

    /// A commuter walking a fixed lane every sim-day, through the real systems.
    #[test]
    fn a_scripted_commuter_wears_a_lane_and_a_one_off_trip_does_not() {
        let mut world = walking_world();
        let commuter = spawn_walker(&mut world, 1, tile(1, 1));
        let stroller = spawn_walker(&mut world, 2, tile(1, 6));
        world
            .run_system_once(accumulate_path_wear)
            .expect("the wear pass runs");

        // Home at (1, 1), the platform at (4, 1); the lane between them is the
        // two tiles the commuter crosses in *both* directions every day.
        let lane = [tile(2, 1), tile(3, 1)];
        let out = [tile(2, 1), tile(3, 1), tile(4, 1)];
        let back = [tile(3, 1), tile(2, 1), tile(1, 1)];

        // The stroller goes exactly once, on day one.
        for x in 2..=4 {
            walk_onto(&mut world, stroller, tile(x, 6));
        }

        let steps_per_day = TICKS_PER_DAY / REGROWTH_INTERVAL_TICKS;
        for day in 0..5u64 {
            for t in out.iter().chain(back.iter()) {
                walk_onto(&mut world, commuter, *t);
            }
            // A sim-day of regrowth, driven by the real systems in the order
            // the schedule chains them — the wear pass keeps seeing the peeps
            // standing about, exactly as it does in the running game.
            for step in 0..steps_per_day {
                let tick = (day * steps_per_day + step + 1) * REGROWTH_INTERVAL_TICKS;
                world.resource_mut::<StationService>().tick = tick;
                world
                    .run_system_once(accumulate_path_wear)
                    .expect("the wear pass runs");
                world
                    .run_system_once(regrow_paths)
                    .expect("the regrowth pass runs");
            }
        }

        let paths = world.resource::<PathWear>();
        // The lane reads, on the brief's promised horizon…
        for t in &lane {
            assert!(
                paths.level_at(*t) >= 1,
                "the commuter's lane at {t:?} never appeared (wear {})",
                paths.wear_at(*t)
            );
        }
        // …the doorsteps at either end do not, because they are entered once a
        // day rather than twice — the lane wears, the threshold does not.
        for t in [tile(1, 1), tile(4, 1)] {
            assert_eq!(
                paths.level_at(t),
                0,
                "a doorstep at {t:?} wore through (wear {})",
                paths.wear_at(t)
            );
        }
        // …and the stroller's single trip left nothing at all.
        for x in 2..=4 {
            assert_eq!(
                paths.level_at(tile(x, 6)),
                0,
                "a single trip painted a path at ({x}, 6)"
            );
            assert!(paths.wear_at(tile(x, 6)) < WEAR_FAINT);
        }
    }

    #[test]
    fn peeps_who_leave_town_stop_being_remembered() {
        let mut world = walking_world();
        for id in 0..8u64 {
            let who = spawn_walker(&mut world, id, tile(1, id as i32 % 8));
            walk_onto(&mut world, who, tile(2, id as i32 % 8));
        }
        assert_eq!(world.resource::<PathWear>().last_seen.len(), 8);

        // Everybody moves away; the sightings map must not keep them for ever.
        let ids: Vec<Entity> = world
            .query_filtered::<Entity, With<Peep>>()
            .iter(&world)
            .collect();
        for entity in ids {
            world.entity_mut(entity).despawn();
        }
        // A regrowth step inside the memory window keeps them…
        world.resource_mut::<StationService>().tick = SIGHTING_MEMORY_TICKS;
        world
            .run_system_once(regrow_paths)
            .expect("the regrowth pass runs");
        assert_eq!(world.resource::<PathWear>().last_seen.len(), 8);

        // …and the next step past it forgets them.
        world.resource_mut::<StationService>().tick =
            SIGHTING_MEMORY_TICKS + REGROWTH_INTERVAL_TICKS;
        world
            .run_system_once(regrow_paths)
            .expect("the regrowth pass runs");
        assert!(world.resource::<PathWear>().last_seen.is_empty());
    }

    #[test]
    fn regrowth_only_runs_on_its_schedule() {
        let mut world = walking_world();
        let who = spawn_walker(&mut world, 1, tile(1, 1));
        world
            .run_system_once(accumulate_path_wear)
            .expect("the wear pass runs");
        walk_onto(&mut world, who, tile(2, 1));
        let before = world.resource::<PathWear>().wear_at(tile(2, 1));
        assert_eq!(before, WEAR_PER_FOOTFALL);

        for tick in 0..REGROWTH_INTERVAL_TICKS {
            world.resource_mut::<StationService>().tick = tick;
            world
                .run_system_once(regrow_paths)
                .expect("the regrowth pass runs");
        }
        assert_eq!(
            world.resource::<PathWear>().wear_at(tile(2, 1)),
            before,
            "regrowth ran early"
        );

        world.resource_mut::<StationService>().tick = REGROWTH_INTERVAL_TICKS;
        world
            .run_system_once(regrow_paths)
            .expect("the regrowth pass runs");
        assert_eq!(
            world.resource::<PathWear>().wear_at(tile(2, 1)),
            before - REGROWTH_PER_STEP
        );
    }

    /// The budget in brief 16 §4.1, measured rather than asserted.
    ///
    /// Generous bounds on purpose: this is a guard against an order-of-magnitude
    /// regression — somebody scanning the whole map every tick, or sorting the
    /// worn set into a fresh allocation — not a benchmark. A machine under load
    /// must not turn it red.
    #[test]
    fn a_busy_towns_wear_pass_costs_microseconds() {
        use std::time::Instant;

        // A full-detail set at the cap, all walking, on a large map.
        let mut world = World::new();
        let cells = (0..128u32).flat_map(|_| (0..128u32).map(|_| (false, 0i8)));
        world.insert_resource(TrackTerrain::new(128, 128, cells));
        world.insert_resource(StationService::default());
        world.insert_resource(PathWear::default());
        // One lane each, so the town wears thousands of tiles rather than one
        // row of them — regrowth is priced per worn tile, not per peep.
        let walkers: Vec<Entity> = (0..crate::peeps::MAX_DETAILED_PEEPS as u64)
            .map(|i| spawn_walker(&mut world, i, tile(0, i as i32)))
            .collect();

        // Warm the map up so the timed run is the steady state, not the growth.
        for step in 0..100i32 {
            for (i, &who) in walkers.iter().enumerate() {
                walk_onto(&mut world, who, tile(step, i as i32));
            }
        }
        assert!(
            world.resource::<PathWear>().worn_count() > 1_000,
            "the fixture is not a busy town"
        );

        // Time the accumulate pass on its own, over many ticks.
        const TICKS: u32 = 400;
        let started = Instant::now();
        for step in 0..TICKS as i32 {
            for (i, &who) in walkers.iter().enumerate() {
                let mut pos = world.get_mut::<PeepPosition>(who).expect("a position");
                pos.snap_to(tile(step % 100, i as i32));
                pos.walking = true;
            }
            world
                .run_system_once(accumulate_path_wear)
                .expect("the wear pass runs");
        }
        let per_tick = started.elapsed() / TICKS;
        eprintln!("wear pass: {per_tick:?} a tick, 64 walking peeps");
        assert!(
            per_tick.as_micros() < 500,
            "the wear pass costs {per_tick:?} a tick on a busy town"
        );

        // And regrowth, which touches every worn tile rather than every peep.
        let worn = world.resource::<PathWear>().worn_count();
        let started = Instant::now();
        for step in 1..=64u64 {
            world.resource_mut::<StationService>().tick = step * REGROWTH_INTERVAL_TICKS;
            world
                .run_system_once(regrow_paths)
                .expect("the regrowth pass runs");
        }
        let per_step = started.elapsed() / 64;
        eprintln!("regrowth: {per_step:?} a step over {worn} worn tiles");
        assert!(
            per_step.as_micros() < 4_000,
            "regrowth over {worn} worn tiles costs {per_step:?} a step"
        );
    }

    /// Two identical worlds, walked identically, must wear identically.
    #[test]
    fn two_runs_of_the_same_world_wear_the_same_ground() {
        let run = || {
            let mut world = walking_world();
            let a = spawn_walker(&mut world, 7, tile(1, 1));
            let b = spawn_walker(&mut world, 3, tile(1, 5));
            world
                .run_system_once(accumulate_path_wear)
                .expect("the wear pass runs");
            for step in 0..60i32 {
                walk_onto(&mut world, a, tile(1 + step % 4, 1));
                walk_onto(&mut world, b, tile(1 + step % 3, 5));
                if step % 9 == 0 {
                    world.resource_mut::<StationService>().tick =
                        (step as u64 / 9 + 1) * REGROWTH_INTERVAL_TICKS;
                    world
                        .run_system_once(regrow_paths)
                        .expect("the regrowth pass runs");
                }
            }
            world.resource::<PathWear>().to_entries()
        };
        let first = run();
        assert!(!first.is_empty(), "the fixture wore nothing");
        assert_eq!(first, run());
    }

    #[test]
    fn wear_is_deterministic_across_runs() {
        let run = || {
            let mut paths = PathWear::new(32, 32);
            for step in 0..500u32 {
                paths.add_footfall(tile((step % 17) as i32, (step % 13) as i32));
                if step % 7 == 0 {
                    paths.regrow();
                }
            }
            paths.to_entries()
        };
        assert_eq!(run(), run());
    }
}
