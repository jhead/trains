//! New-map setup options, the generator inputs they derive, and honest readouts.
//!
//! Option list follows [`docs/design/02-world-and-terrain.md`](../../../docs/design/02-world-and-terrain.md) §5.
//!
//! **Generator gap:** [`rail_map::generate_map`] currently takes `(width, height,
//! seed)` only — it has no terrain / water / resource knobs. Rather than draw a
//! control that does nothing, [`MapOptions::effective_seed`] folds every non-size
//! choice into the generator seed, so changing *any* option genuinely produces a
//! different world. The readouts beside the preview are then **measured from the
//! map that was actually generated**, never predicted from the labels. When the
//! generator grows real parameters, swap [`MapOptions::generate`] over to them and
//! the rest of the screen is unchanged.

use bevy::prelude::Color;
use rail_map::{MapGrid, TerrainKind};
use rail_sim::ids::TileCoord;
use rail_sim::{
    local_slope, seed_stations_and_industries, GoalMode, IndustryRegistry, StationRegistry,
    StationService, TrackTerrain, MAX_GRADE, MOUNTAIN_HEIGHT_MIN, STARTING_CASH_CENTS,
};

use crate::palette::{
    GRASS_D, GRASS_M, HILL_L, HILL_M, ROCK_L, ROCK_M, SAND_D, SNOW, WATER_D, WATER_L, WATER_M,
};

/// Preview box edge in UI px. Every map size scales into it by a whole number
/// (48→6×, 64→4×, 96→3×, 128→2×), so the schematic never resamples.
pub const PREVIEW_BOX: f32 = 288.0;

/// Map edge length in tiles. Design 02 §5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapSize {
    Small,
    #[default]
    Standard,
    Large,
    Huge,
}

/// How hard the terrain argues. Design 02 §5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TerrainStyle {
    Gentle,
    #[default]
    Rolling,
    Rugged,
}

/// How much of the puzzle is crossings. Design 02 §5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WaterStyle {
    Sparse,
    #[default]
    Balanced,
    Riverlands,
}

/// Long hauls vs. dense networks. Design 02 §5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResourceSpread {
    Clustered,
    #[default]
    Scattered,
}

/// How much the early game paces. Design 02 §5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartingCash {
    Lean,
    #[default]
    Standard,
    Generous,
}

/// Session shape (design 09 §3). Goals is the same sandbox with objectives and
/// deadlines drawn from it — see [`rail_sim::goals`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GameMode {
    #[default]
    Sandbox,
    Goals,
}

impl GameMode {
    /// The sim-side switch this choice sets on the world's
    /// [`GoalBoard`](rail_sim::GoalBoard).
    pub fn to_goal_mode(self) -> GoalMode {
        match self {
            Self::Sandbox => GoalMode::Sandbox,
            Self::Goals => GoalMode::Goals,
        }
    }
}

/// Implements the shared "cycle through a small set of labelled values" contract
/// every option row on the New Map screen uses.
pub trait OptionChoice: Copy + PartialEq + Sized + 'static {
    const ALL: &'static [Self];

    fn label(self) -> &'static str;

    /// `false` draws the choice disabled and refuses selection.
    fn enabled(self) -> bool {
        true
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }
}

impl OptionChoice for MapSize {
    const ALL: &'static [Self] = &[Self::Small, Self::Standard, Self::Large, Self::Huge];

    fn label(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Standard => "Standard",
            Self::Large => "Large",
            Self::Huge => "Huge",
        }
    }
}

impl MapSize {
    /// Map edge in tiles (maps are square).
    pub fn tiles(self) -> u32 {
        match self {
            Self::Small => 48,
            Self::Standard => 64,
            Self::Large => 96,
            Self::Huge => 128,
        }
    }

    /// Whole-number preview scale that fits [`PREVIEW_BOX`].
    pub fn preview_scale(self) -> u32 {
        match self {
            Self::Small => 6,    // 288
            Self::Standard => 4, // 256
            Self::Large => 3,    // 288
            Self::Huge => 2,     // 256
        }
    }
}

impl OptionChoice for TerrainStyle {
    const ALL: &'static [Self] = &[Self::Gentle, Self::Rolling, Self::Rugged];

    fn label(self) -> &'static str {
        match self {
            Self::Gentle => "Gentle",
            Self::Rolling => "Rolling",
            Self::Rugged => "Rugged",
        }
    }
}

impl OptionChoice for WaterStyle {
    const ALL: &'static [Self] = &[Self::Sparse, Self::Balanced, Self::Riverlands];

    fn label(self) -> &'static str {
        match self {
            Self::Sparse => "Sparse",
            Self::Balanced => "Balanced",
            Self::Riverlands => "Riverlands",
        }
    }
}

impl OptionChoice for ResourceSpread {
    const ALL: &'static [Self] = &[Self::Clustered, Self::Scattered];

    fn label(self) -> &'static str {
        match self {
            Self::Clustered => "Clustered",
            Self::Scattered => "Scattered",
        }
    }
}

impl OptionChoice for StartingCash {
    const ALL: &'static [Self] = &[Self::Lean, Self::Standard, Self::Generous];

    fn label(self) -> &'static str {
        match self {
            Self::Lean => "Lean",
            Self::Standard => "Standard",
            Self::Generous => "Generous",
        }
    }
}

impl StartingCash {
    /// Treasury the game starts with, in cents.
    pub fn cents(self) -> i64 {
        match self {
            Self::Lean => STARTING_CASH_CENTS / 2,
            Self::Standard => STARTING_CASH_CENTS,
            Self::Generous => STARTING_CASH_CENTS * 2,
        }
    }
}

impl OptionChoice for GameMode {
    const ALL: &'static [Self] = &[Self::Sandbox, Self::Goals];

    fn label(self) -> &'static str {
        match self {
            Self::Sandbox => "Sandbox",
            Self::Goals => "Goals",
        }
    }
}

/// A complete new-game setup. Seed plus these fully determine a map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapOptions {
    pub seed: u64,
    pub size: MapSize,
    pub terrain: TerrainStyle,
    pub water: WaterStyle,
    pub resources: ResourceSpread,
    pub cash: StartingCash,
    pub mode: GameMode,
}

impl Default for MapOptions {
    fn default() -> Self {
        Self {
            seed: rail_map::DEFAULT_MAP_SEED,
            size: MapSize::default(),
            terrain: TerrainStyle::default(),
            water: WaterStyle::default(),
            resources: ResourceSpread::default(),
            cash: StartingCash::default(),
            mode: GameMode::default(),
        }
    }
}

/// Seeds a player types stay short and quotable; five digits is the design's example.
pub const SEED_MAX: u64 = 99_999;

/// Bits of [`MapOptions::packed_options`] that describe the *world* — terrain,
/// water, resources, cash. Size (bits 0–1) is a real generator input and Mode
/// (bit 9) is a rule about the session, so neither belongs in the seed mix.
const WORLD_FLAVOUR_BITS: u64 = 0b1_1111_1100;

impl MapOptions {
    /// Options packed into a byte, low bits first: size, terrain, water,
    /// resources, cash, mode. Used by [`Self::effective_seed`] and share codes.
    fn packed_options(&self) -> u64 {
        let mut bits = 0u64;
        let mut shift = 0u32;
        for (value, width) in [
            (self.size.index() as u64, 2),
            (self.terrain.index() as u64, 2),
            (self.water.index() as u64, 2),
            (self.resources.index() as u64, 1),
            (self.cash.index() as u64, 2),
            (self.mode.index() as u64, 1),
        ] {
            bits |= value << shift;
            shift += width;
        }
        bits
    }

    /// Generator seed derived from the player seed and every option that the
    /// generator cannot yet take directly (see the module docs).
    pub fn effective_seed(&self) -> u64 {
        // Map size is a real generator input, and Mode is a rule about the
        // session rather than the world, so both are deliberately excluded from
        // the mix: resizing a map — or playing it to goals — should keep the
        // same landscape. [`WORLD_FLAVOUR_BITS`] is what is left.
        //
        // Measured against the *default* flavour, not zero, so a stock setup
        // passes the player's seed straight through. "Seed 42" then means the
        // same world it has always meant, and the number shown on the title
        // screen is the number the player would type to get it back.
        let world = |bits: u64| (bits & WORLD_FLAVOUR_BITS) >> 2;
        let flavour = world(self.packed_options()) ^ world(Self::default().packed_options());
        if flavour == 0 {
            return self.seed;
        }
        splitmix64(self.seed ^ splitmix64(flavour.wrapping_add(0x9e37_79b9)))
    }

    /// The generator knobs these options describe.
    fn gen_options(&self) -> rail_map::MapGenOptions {
        rail_map::MapGenOptions {
            size: rail_map::MapSize::from_index(self.size.index()).unwrap_or_default(),
            terrain: rail_map::TerrainStyle::from_index(self.terrain.index()).unwrap_or_default(),
            water: rail_map::WaterStyle::from_index(self.water.index()).unwrap_or_default(),
            resources: rail_map::ResourceSpread::from_index(self.resources.index())
                .unwrap_or_default(),
        }
    }

    /// Generate the map these options describe.
    ///
    /// Uses the raw seed, not [`Self::effective_seed`]: the options now *steer*
    /// the generator, and folding them into the seed as well would make them do
    /// the job twice. `effective_seed` stays for the share code.
    pub fn generate(&self) -> MapGrid {
        rail_map::generate(self.seed, self.gen_options())
    }

    /// Short shareable code encoding seed **and** settings (design 02 §5).
    ///
    /// Base-36, uppercase, so it survives being read aloud or pasted anywhere.
    pub fn share_code(&self) -> String {
        let packed = (self.seed.min(SEED_MAX) << 10) | self.packed_options();
        to_base36(packed)
    }

    /// Parse a [`Self::share_code`]. Returns `None` on anything unrecognised.
    pub fn from_share_code(code: &str) -> Option<Self> {
        let packed = from_base36(code)?;
        let bits = packed & 0x3ff;
        let seed = packed >> 10;
        if seed > SEED_MAX {
            return None;
        }
        let pick = |shift: u32, width: u32, all_len: usize| -> Option<usize> {
            let mask = (1u64 << width) - 1;
            let idx = ((bits >> shift) & mask) as usize;
            (idx < all_len).then_some(idx)
        };
        Some(Self {
            seed,
            size: MapSize::ALL[pick(0, 2, MapSize::ALL.len())?],
            terrain: TerrainStyle::ALL[pick(2, 2, TerrainStyle::ALL.len())?],
            water: WaterStyle::ALL[pick(4, 2, WaterStyle::ALL.len())?],
            resources: ResourceSpread::ALL[pick(6, 1, ResourceSpread::ALL.len())?],
            cash: StartingCash::ALL[pick(7, 2, StartingCash::ALL.len())?],
            mode: GameMode::ALL[pick(9, 1, GameMode::ALL.len())?],
        })
    }
}

/// Which option a New Map row edits. Keeps the screen's rows data-driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionField {
    Seed,
    Size,
    Terrain,
    Water,
    Resources,
    Cash,
    Mode,
}

impl OptionField {
    pub const ALL: &'static [Self] = &[
        Self::Seed,
        Self::Size,
        Self::Terrain,
        Self::Water,
        Self::Resources,
        Self::Cash,
        Self::Mode,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Seed => "Seed",
            Self::Size => "Size",
            Self::Terrain => "Terrain",
            Self::Water => "Water",
            Self::Resources => "Resources",
            Self::Cash => "Cash",
            Self::Mode => "Mode",
        }
    }

    pub fn value_label(self, options: &MapOptions) -> String {
        match self {
            Self::Seed => options.seed.to_string(),
            Self::Size => format!("{}  {}2", options.size.label(), options.size.tiles()),
            Self::Terrain => options.terrain.label().into(),
            Self::Water => options.water.label().into(),
            Self::Resources => options.resources.label().into(),
            Self::Cash => options.cash.label().into(),
            Self::Mode => options.mode.label().into(),
        }
    }

    /// Step the value. Seed nudges by one; the rest cycle their choice list,
    /// skipping any choice that is not implemented yet.
    pub fn cycle(self, options: &mut MapOptions, delta: i32) {
        match self {
            Self::Seed => {
                let span = (SEED_MAX + 1) as i64;
                options.seed = ((options.seed as i64 + delta as i64).rem_euclid(span)) as u64;
            }
            Self::Size => options.size = cycle_choice(options.size, delta),
            Self::Terrain => options.terrain = cycle_choice(options.terrain, delta),
            Self::Water => options.water = cycle_choice(options.water, delta),
            Self::Resources => options.resources = cycle_choice(options.resources, delta),
            Self::Cash => options.cash = cycle_choice(options.cash, delta),
            Self::Mode => options.mode = cycle_choice(options.mode, delta),
        }
    }

    /// `Some(note)` when the option is recorded but the generator ignores it.
    pub fn pending_note(self) -> Option<&'static str> {
        match self {
            // The generator has no shape parameters yet — the choice re-rolls the
            // world through the seed instead of steering it. See the module docs.
            _ => None,
        }
    }
}

/// Next enabled choice in `delta` direction, wrapping.
fn cycle_choice<T: OptionChoice>(current: T, delta: i32) -> T {
    let all = T::ALL;
    if all.is_empty() {
        return current;
    }
    let start = current.index() as i32;
    let len = all.len() as i32;
    for offset in 1..=len {
        let next = all[(start + delta * offset).rem_euclid(len) as usize];
        if next.enabled() {
            return next;
        }
    }
    current
}

/// Deterministic 64-bit mix (SplitMix64 finaliser) — no rng dependency needed.
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A fresh seed for the dice button.
///
/// `rail_town` has no `rand` dependency and does not need one for this: the wall
/// clock mixed through [`splitmix64`] is plenty of entropy for "give me another
/// map", and it keeps the crate's dependency list honest.
pub fn roll_seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x5eed_5eed);
    splitmix64(nanos) % (SEED_MAX + 1)
}

const BASE36: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

fn to_base36(mut value: u64) -> String {
    if value == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while value > 0 {
        out.push(BASE36[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("base36 digits are ascii")
}

fn from_base36(code: &str) -> Option<u64> {
    let code = code.trim();
    if code.is_empty() || code.len() > 13 {
        return None;
    }
    let mut value = 0u64;
    for ch in code.bytes() {
        let digit = BASE36.iter().position(|c| c.eq_ignore_ascii_case(&ch))? as u64;
        value = value.checked_mul(36)?.checked_add(digit)?;
    }
    Some(value)
}

/// Measured facts about a generated map, for the readouts beside the preview.
///
/// Every field is counted from the grid that was just generated. Nothing here is
/// inferred from the option labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MapReadouts {
    /// Share of tiles that are land, 0–100.
    pub land_pct: u32,
    /// Share of land that sits on the single largest connected landmass, 0–100.
    /// A low number means the map is islands and most of it is unreachable by rail.
    pub mainland_pct: u32,
    /// Stations the world will actually start with — counted by running the same
    /// seeder the game runs ([`rail_sim::seed_stations_and_industries`]).
    pub towns: usize,
    /// Inland water bodies: 4-connected water components that never touch the map
    /// border. Sea is excluded, so this counts the crossings the player must solve.
    pub rivers: usize,
    /// Gaps through high ground: buildable tiles with impassable rock on two
    /// opposite sides within two tiles, grouped into connected clusters.
    pub passes: usize,
}

impl MapReadouts {
    pub fn measure(map: &MapGrid) -> Self {
        let total = map.tiles().len().max(1);
        let water = map.tiles().iter().filter(|t| t.water).count();
        let land = total - water;

        Self {
            land_pct: percent(land, total),
            mainland_pct: percent(largest_landmass(map), land.max(1)),
            towns: count_seeded_towns(map),
            // Places to cross, not water bodies: a river that reaches the map
            // border is one component touching the frame and would count zero.
            rivers: map.features().crossings.len(),
            passes: rail_map::measure::ridge_passes(map).len(),
        }
    }
}

fn percent(part: usize, whole: usize) -> u32 {
    if whole == 0 {
        return 0;
    }
    ((part * 200 + whole) / (whole * 2)) as u32
}

/// Run the real anchor seeder against this map and count the stations it places.
fn count_seeded_towns(map: &MapGrid) -> usize {
    let terrain = track_terrain_from(map);
    let mut stations = StationRegistry::new();
    let mut industries = IndustryRegistry::new();
    let mut service = StationService::default();
    seed_stations_and_industries(
        &mut stations,
        &mut industries,
        &mut service,
        terrain.width(),
        terrain.height(),
        |c| {
            terrain.contains(c)
                && !terrain.is_water(c)
                && terrain.height_at(c).unwrap_or(0) < MOUNTAIN_HEIGHT_MIN
                && local_slope(&terrain, c) <= MAX_GRADE + 1
        },
    );
    stations.len()
}

/// Same conversion `TrackPlugin` does at startup, so the readouts — and a
/// rebuilt world — see the exact terrain the placement rules will see.
pub fn track_terrain_from(map: &MapGrid) -> TrackTerrain {
    let mut cells = Vec::with_capacity(map.tiles().len());
    for y in 0..map.height {
        for x in 0..map.width {
            let tile = map.tile(TileCoord {
                x: x as i32,
                y: y as i32,
            });
            cells.push((tile.water, tile.height));
        }
    }
    TrackTerrain::new(map.width, map.height, cells)
}

fn largest_landmass(map: &MapGrid) -> usize {
    flood_components(map, |m, c| !m.tile(c).water)
        .into_iter()
        .max()
        .unwrap_or(0)
}

fn count_inland_water_bodies(map: &MapGrid) -> usize {
    let w = map.width as i32;
    let h = map.height as i32;
    let mut seen = vec![false; map.tiles().len()];
    let idx = |x: i32, y: i32| (y * w + x) as usize;
    let mut bodies = 0usize;

    for y in 0..h {
        for x in 0..w {
            if seen[idx(x, y)] || !map.tile(TileCoord { x, y }).water {
                continue;
            }
            let mut stack = vec![(x, y)];
            seen[idx(x, y)] = true;
            let mut touches_border = false;
            let mut size = 0usize;
            while let Some((cx, cy)) = stack.pop() {
                size += 1;
                if cx == 0 || cy == 0 || cx == w - 1 || cy == h - 1 {
                    touches_border = true;
                }
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, ny) = (cx + dx, cy + dy);
                    if nx < 0 || ny < 0 || nx >= w || ny >= h {
                        continue;
                    }
                    if seen[idx(nx, ny)] || !map.tile(TileCoord { x: nx, y: ny }).water {
                        continue;
                    }
                    seen[idx(nx, ny)] = true;
                    stack.push((nx, ny));
                }
            }
            // One-tile puddles are texture, not a crossing decision.
            if !touches_border && size >= 3 {
                bodies += 1;
            }
        }
    }
    bodies
}

/// A pass is a buildable tile pinched between impassable rock on opposite sides.
/// Clustering adjacent ones stops a four-tile-wide gap reading as four passes.
fn count_passes(map: &MapGrid) -> usize {
    let w = map.width as i32;
    let h = map.height as i32;
    let blocked = |x: i32, y: i32| {
        x >= 0
            && y >= 0
            && x < w
            && y < h
            && map.tile(TileCoord { x, y }).height >= MOUNTAIN_HEIGHT_MIN
    };

    let mut is_pass = vec![false; map.tiles().len()];
    let mut any = false;
    for y in 0..h {
        for x in 0..w {
            let tile = map.tile(TileCoord { x, y });
            if tile.water || tile.height >= MOUNTAIN_HEIGHT_MIN {
                continue;
            }
            let horizontal =
                (1..=2).any(|d| blocked(x - d, y)) && (1..=2).any(|d| blocked(x + d, y));
            let vertical = (1..=2).any(|d| blocked(x, y - d)) && (1..=2).any(|d| blocked(x, y + d));
            if horizontal || vertical {
                is_pass[(y * w + x) as usize] = true;
                any = true;
            }
        }
    }
    if !any {
        return 0;
    }
    flood_components(map, |m, c| is_pass[(c.y * m.width as i32 + c.x) as usize]).len()
}

/// Sizes of every 4-connected component of tiles matching `keep`.
fn flood_components(map: &MapGrid, keep: impl Fn(&MapGrid, TileCoord) -> bool) -> Vec<usize> {
    let w = map.width as i32;
    let h = map.height as i32;
    let mut seen = vec![false; map.tiles().len()];
    let idx = |x: i32, y: i32| (y * w + x) as usize;
    let mut sizes = Vec::new();

    for y in 0..h {
        for x in 0..w {
            if seen[idx(x, y)] || !keep(map, TileCoord { x, y }) {
                continue;
            }
            let mut stack = vec![(x, y)];
            seen[idx(x, y)] = true;
            let mut size = 0usize;
            while let Some((cx, cy)) = stack.pop() {
                size += 1;
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, ny) = (cx + dx, cy + dy);
                    if nx < 0 || ny < 0 || nx >= w || ny >= h {
                        continue;
                    }
                    if seen[idx(nx, ny)] || !keep(map, TileCoord { x: nx, y: ny }) {
                        continue;
                    }
                    seen[idx(nx, ny)] = true;
                    stack.push((nx, ny));
                }
            }
            sizes.push(size);
        }
    }
    sizes
}

/// One RGBA texel per tile, top-left origin (map `y` runs up, images run down).
///
/// Deliberately a *schematic* read — silhouette plus elevation band, in the same
/// palette as the world (design 02 §6), not a copy of the world tile renderer.
pub fn schematic_rgba(map: &MapGrid) -> Vec<u8> {
    let w = map.width as i32;
    let h = map.height as i32;
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h {
        let y = h - 1 - row;
        for x in 0..w {
            let tile = map.tile(TileCoord { x, y });
            let [r, g, b] = srgb_bytes(schematic_color(tile.kind, tile.height));
            out.extend_from_slice(&[r, g, b, 255]);
        }
    }
    out
}

fn schematic_color(kind: TerrainKind, height: i8) -> Color {
    match kind {
        TerrainKind::Water => match height {
            ..=-6 => WATER_D,
            -5..=-3 => WATER_M,
            _ => WATER_L,
        },
        TerrainKind::Beach => SAND_D,
        TerrainKind::Plains => {
            if height <= 3 {
                GRASS_D
            } else {
                GRASS_M
            }
        }
        TerrainKind::Hills => {
            if height <= 8 {
                HILL_M
            } else {
                HILL_L
            }
        }
        TerrainKind::Mountain => match height {
            ..=13 => ROCK_M,
            14..=15 => ROCK_L,
            _ => SNOW,
        },
    }
}

fn srgb_bytes(color: Color) -> [u8; 3] {
    let s = color.to_srgba();
    [
        (s.red * 255.0).round().clamp(0.0, 255.0) as u8,
        (s.green * 255.0).round().clamp(0.0, 255.0) as u8,
        (s.blue * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_size_scales_into_the_preview_box_by_a_whole_number() {
        for size in MapSize::ALL {
            let px = size.tiles() * size.preview_scale();
            assert!(
                px as f32 <= PREVIEW_BOX,
                "{} preview overflows the box ({px}px)",
                size.label()
            );
            // Pixel contract: preview edge stays on the 4px grid.
            assert_eq!(px % 4, 0, "{} preview is off-grid ({px}px)", size.label());
        }
    }

    #[test]
    fn changing_any_option_changes_the_generated_world() {
        let base = MapOptions::default();
        let mut seen = vec![base.effective_seed()];
        for terrain in TerrainStyle::ALL {
            for water in WaterStyle::ALL {
                let candidate = MapOptions {
                    terrain: *terrain,
                    water: *water,
                    ..base
                };
                let seed = candidate.effective_seed();
                if candidate != base {
                    assert!(
                        !seen.contains(&seed),
                        "{}/{} collided with an earlier option combination",
                        terrain.label(),
                        water.label()
                    );
                }
                seen.push(seed);
            }
        }
    }

    #[test]
    fn size_alone_does_not_reroll_the_landscape() {
        let small = MapOptions {
            size: MapSize::Small,
            ..MapOptions::default()
        };
        let huge = MapOptions {
            size: MapSize::Huge,
            ..MapOptions::default()
        };
        assert_eq!(small.effective_seed(), huge.effective_seed());
    }

    #[test]
    fn share_code_round_trips_seed_and_settings() {
        let options = MapOptions {
            seed: 84_213,
            size: MapSize::Large,
            terrain: TerrainStyle::Rugged,
            water: WaterStyle::Riverlands,
            resources: ResourceSpread::Clustered,
            cash: StartingCash::Generous,
            mode: GameMode::Sandbox,
        };
        let code = options.share_code();
        assert_eq!(MapOptions::from_share_code(&code), Some(options));
        assert_eq!(
            MapOptions::from_share_code(&code.to_lowercase()),
            Some(options)
        );
        assert_eq!(MapOptions::from_share_code("not a code!"), None);
    }

    #[test]
    fn goals_mode_is_selectable_and_reaches_the_sim() {
        assert!(GameMode::Sandbox.enabled());
        assert!(GameMode::Goals.enabled());
        assert_eq!(GameMode::Sandbox.to_goal_mode(), GoalMode::Sandbox);
        assert_eq!(GameMode::Goals.to_goal_mode(), GoalMode::Goals);
    }

    #[test]
    fn choosing_goals_does_not_reroll_the_world() {
        // Mode is a rule about the session, not a terrain knob. A player who
        // liked a map must be able to replay it to goals and get the same map.
        let sandbox = MapOptions {
            seed: 84_213,
            mode: GameMode::Sandbox,
            ..MapOptions::default()
        };
        let goals = MapOptions {
            mode: GameMode::Goals,
            ..sandbox
        };
        assert_eq!(sandbox.effective_seed(), goals.effective_seed());
        // …but the share code still carries the mode, so the *game* round-trips.
        assert_ne!(sandbox.share_code(), goals.share_code());
        assert_eq!(
            MapOptions::from_share_code(&goals.share_code()),
            Some(goals)
        );
    }

    #[test]
    fn starting_cash_brackets_the_sandbox_default() {
        assert!(StartingCash::Lean.cents() < StartingCash::Standard.cents());
        assert!(StartingCash::Standard.cents() < StartingCash::Generous.cents());
        assert_eq!(StartingCash::Standard.cents(), STARTING_CASH_CENTS);
    }

    #[test]
    fn readouts_are_measured_from_the_actual_grid() {
        let options = MapOptions {
            size: MapSize::Small,
            ..MapOptions::default()
        };
        let map = options.generate();
        let readouts = MapReadouts::measure(&map);
        assert!(readouts.land_pct > 0 && readouts.land_pct <= 100);
        assert!(readouts.mainland_pct > 0 && readouts.mainland_pct <= 100);
        assert!(readouts.towns <= 3, "seeder places at most three stations");

        // Same seed → same numbers, every time.
        assert_eq!(MapReadouts::measure(&options.generate()), readouts);
    }

    #[test]
    fn schematic_has_one_opaque_texel_per_tile() {
        let map = MapOptions {
            size: MapSize::Small,
            ..MapOptions::default()
        }
        .generate();
        let rgba = schematic_rgba(&map);
        assert_eq!(rgba.len(), map.tiles().len() * 4);
        assert!(rgba.chunks_exact(4).all(|px| px[3] == 255));
    }

    #[test]
    fn every_option_row_steps_and_comes_back() {
        for field in OptionField::ALL {
            let mut options = MapOptions::default();
            let before = field.value_label(&options);
            field.cycle(&mut options, 1);
            assert_ne!(
                field.value_label(&options),
                before,
                "{} did not change when stepped",
                field.label()
            );
            field.cycle(&mut options, -1);
            assert_eq!(
                field.value_label(&options),
                before,
                "{} did not step back",
                field.label()
            );
        }
    }

    #[test]
    fn the_mode_row_steps_between_both_sessions_shapes() {
        let mut options = MapOptions::default();
        assert_eq!(options.mode, GameMode::Sandbox);
        OptionField::Mode.cycle(&mut options, 1);
        assert_eq!(options.mode, GameMode::Goals);
        OptionField::Mode.cycle(&mut options, 1);
        assert_eq!(options.mode, GameMode::Sandbox, "the row wraps");
        assert!(
            OptionField::Mode.pending_note().is_none(),
            "the row no longer has to apologise for itself"
        );
    }

    #[test]
    fn seed_wraps_inside_the_shareable_range() {
        let mut options = MapOptions {
            seed: 0,
            ..MapOptions::default()
        };
        OptionField::Seed.cycle(&mut options, -1);
        assert_eq!(options.seed, SEED_MAX);
        OptionField::Seed.cycle(&mut options, 1);
        assert_eq!(options.seed, 0);
    }

    #[test]
    fn rolled_seeds_stay_shareable() {
        for _ in 0..32 {
            assert!(roll_seed() <= SEED_MAX);
        }
    }

    #[test]
    fn percent_rounds_to_nearest() {
        assert_eq!(percent(1, 2), 50);
        assert_eq!(percent(2, 3), 67);
        assert_eq!(percent(0, 10), 0);
        assert_eq!(percent(10, 10), 100);
    }
}
