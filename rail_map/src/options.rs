//! New-map parameters. Seed + [`MapGenOptions`] fully determine a world.
//!
//! Option list follows [`docs/design/02-world-and-terrain.md`](../../docs/design/02-world-and-terrain.md) §5.
//! Each knob is a *generation parameter*, not a seed ingredient: stepping Terrain
//! from Rolling to Rugged keeps the same coastline and the same ridge skeleton and
//! makes that skeleton argue harder. That is the difference between an option that
//! steers and an option that re-rolls.
//!
//! Composition targets live here too, because they are what the water and
//! terrain knobs actually move.
//!
//! # Composition
//!
//! Brief 02 §2.1's table has been **superseded by playtest**. The reference is
//! Chris Sawyer's Locomotion and RollerCoaster Tycoon: a broad, open, mostly
//! buildable landscape where the constraint is the *shape* of the ground — hills,
//! ridges, valleys, the occasional river — not a coastline hemming the player in.
//! Water is an accent and an obstacle, never the surface most of the screen is
//! spent looking at.
//!
//! | Surface | Target | Brief 02 §2.1 said |
//! | --- | --- | --- |
//! | Buildable land | **85–92%** | 70–78% |
//! | Inland water | **4–8%** | 8–14% |
//! | Sea | **0–4%**, most maps none | 6–12% |
//! | Impassable rock | **4–8%** | unchanged |
//!
//! Two consequences run through the whole generator. Sea is *optional*: there is
//! no edge bias pulling the borders underwater, and a map is landlocked unless it
//! rolls a coast (see [`WaterStyle::coastal_chance`]). And elevation now carries
//! most of the routing puzzle, so ridges-with-passes and valley corridors matter
//! more than they did, not less.

use serde::{Deserialize, Serialize};

/// Map edge length in tiles (§5). Maps are square.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MapSize {
    Small,
    #[default]
    Standard,
    Large,
    Huge,
}

impl MapSize {
    pub const ALL: &'static [Self] = &[Self::Small, Self::Standard, Self::Large, Self::Huge];

    /// Map edge in tiles.
    pub const fn tiles(self) -> u32 {
        match self {
            Self::Small => 48,
            Self::Standard => 64,
            Self::Large => 96,
            Self::Huge => 128,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Standard => "Standard",
            Self::Large => "Large",
            Self::Huge => "Huge",
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Small => 0,
            Self::Standard => 1,
            Self::Large => 2,
            Self::Huge => 3,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// The listed size whose edge is nearest `tiles`.
    ///
    /// Callers that pass raw `(width, height)` — the old [`crate::generate_map`]
    /// signature, ragged test maps — still get a sensible feature budget.
    pub fn nearest(tiles: u32) -> Self {
        Self::ALL
            .iter()
            .copied()
            .min_by_key(|s| s.tiles().abs_diff(tiles))
            .unwrap_or(Self::Standard)
    }
}

/// How hard the terrain argues (§5): ridge count, relief, and rock share.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TerrainStyle {
    Gentle,
    #[default]
    Rolling,
    Rugged,
}

impl TerrainStyle {
    pub const ALL: &'static [Self] = &[Self::Gentle, Self::Rolling, Self::Rugged];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Gentle => "Gentle",
            Self::Rolling => "Rolling",
            Self::Rugged => "Rugged",
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Gentle => 0,
            Self::Rolling => 1,
            Self::Rugged => 2,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// Ridge spines on a Standard map.
    ///
    /// §2.2: a ridge is a *barrier*, and crossing one costs a run of 6× cut-and-
    /// fill however you take it. Two of them on a 64² map already means no route
    /// across the map is cheap, which is the opposite of the open countryside the
    /// owner asked for. One wall with a couple of passes is the decision; more
    /// than that is a maze.
    pub const fn ridges(self) -> usize {
        match self {
            Self::Gentle => 1,
            Self::Rolling => 1,
            Self::Rugged => 2,
        }
    }

    /// Passes per ridge. §2.2 exactly: "two passes is a decision; twenty is a
    /// texture." Gentle terrain is forgiving, so its one ridge gets three.
    pub const fn passes_per_ridge(self) -> usize {
        match self {
            Self::Gentle => 3,
            Self::Rolling => 2,
            Self::Rugged => 2,
        }
    }

    /// Crest height in bands. Anything at or above [`crate::ROCK_BAND`] is rock.
    pub fn crest(self) -> f32 {
        match self {
            Self::Gentle => 4.6,
            Self::Rolling => 5.2,
            Self::Rugged => 5.8,
        }
    }

    /// Amplitude, in bands, of the noise that decorates the placed landforms.
    ///
    /// Deliberately below half a band: noise is texture on top of features, never
    /// the source of them (§2.2).
    pub fn grain(self) -> f32 {
        match self {
            Self::Gentle => 0.22,
            Self::Rolling => 0.32,
            Self::Rugged => 0.46,
        }
    }

    /// Plateaus and basins placed on a Standard map.
    ///
    /// Deliberately few. Every band boundary is a 6× cut-and-fill to cross, so a
    /// map crowded with landforms is a map where no long route is affordable —
    /// and the owner's brief is explicit that the space *between* the interesting
    /// bits is a feature, not filler. One of each, plus the ridges, leaves most of
    /// the map as open 1× country to route freely across.
    pub const fn plateaus(self) -> usize {
        match self {
            Self::Gentle => 1,
            Self::Rolling => 1,
            Self::Rugged => 2,
        }
    }

    pub const fn basins(self) -> usize {
        1
    }

    /// Share of the map that should end up impassable rock (4–8%).
    ///
    /// Elevation carries most of the routing puzzle now that water is an accent,
    /// so rock is the only hard wall on a typical map and its share is what
    /// separates Gentle from Rugged.
    pub fn rock_target(self) -> f32 {
        match self {
            Self::Gentle => 4.5,
            Self::Rolling => 5.5,
            Self::Rugged => 6.5,
        }
    }
}

/// How much of the puzzle is crossings (§5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum WaterStyle {
    Sparse,
    #[default]
    Balanced,
    Riverlands,
}

impl WaterStyle {
    pub const ALL: &'static [Self] = &[Self::Sparse, Self::Balanced, Self::Riverlands];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Sparse => "Sparse",
            Self::Balanced => "Balanced",
            Self::Riverlands => "Riverlands",
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Sparse => 0,
            Self::Balanced => 1,
            Self::Riverlands => 2,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// River systems. Every standard map gets at least one (§2.1).
    pub const fn rivers(self) -> usize {
        match self {
            Self::Sparse => 1,
            Self::Balanced => 1,
            Self::Riverlands => 2,
        }
    }

    /// Tributaries hung off each trunk.
    pub const fn tributaries(self) -> usize {
        match self {
            Self::Sparse => 0,
            Self::Balanced => 1,
            Self::Riverlands => 2,
        }
    }

    /// Crossings on the trunk river. §2.1 asks for two to four of differing width.
    pub const fn crossings(self) -> usize {
        match self {
            Self::Sparse => 2,
            Self::Balanced => 3,
            Self::Riverlands => 4,
        }
    }

    /// Seeds in a hundred that get a coast at all.
    ///
    /// A Rail Town map is **inland countryside** by default — Locomotion and RCT,
    /// not an archipelago. Sea is optional scenery plus a harbour, so most maps
    /// have none and the ones that do get a single inlet, never a frame.
    pub const fn coastal_chance(self) -> u32 {
        match self {
            Self::Sparse => 0,
            Self::Balanced => 33,
            Self::Riverlands => 100,
        }
    }

    /// Inlets cut on a map that rolled a coast.
    pub const fn bays(self) -> usize {
        match self {
            Self::Sparse => 0,
            Self::Balanced => 1,
            Self::Riverlands => 2,
        }
    }

    /// Share of the map that should be sea **when this map has a coast at all**
    /// (0–4%, and most maps are landlocked).
    pub fn sea_target(self) -> f32 {
        match self {
            Self::Sparse => 0.0,
            Self::Balanced => 1.5,
            Self::Riverlands => 1.5,
        }
    }

    /// Share of the map that should be inland water — rivers first, then small
    /// lakes (4–8%). Narrow rivers, generously placed, beat big lakes.
    pub fn inland_target(self) -> f32 {
        match self {
            Self::Sparse => 5.0,
            Self::Balanced => 6.5,
            Self::Riverlands => 6.5,
        }
    }
}

/// Long hauls vs. dense networks (§5).
///
/// The generator has no ore seams to move, so this steers the thing it does own:
/// the distribution of the candidate sites in [`crate::MapFeatures::sites`] that
/// anchor placement draws from (§4.2 — "spacing follows a distribution, not an
/// extremum").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ResourceSpread {
    Clustered,
    #[default]
    Scattered,
}

impl ResourceSpread {
    pub const ALL: &'static [Self] = &[Self::Clustered, Self::Scattered];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Clustered => "Clustered",
            Self::Scattered => "Scattered",
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Clustered => 0,
            Self::Scattered => 1,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// How many basins the candidate sites are drawn around. One basin per
    /// cluster; `None` spreads them over the whole landmass.
    pub const fn clusters(self) -> Option<usize> {
        match self {
            Self::Clustered => Some(3),
            Self::Scattered => None,
        }
    }
}

/// What the four composition rows should come out as, in percent.
///
/// Every `(water, terrain)` pair, coastal or not, sums so that buildable land
/// lands inside 85–92% — see the `targets_sit_inside_the_design_bands` test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompositionTargets {
    pub buildable: f32,
    pub inland_water: f32,
    pub sea: f32,
    pub rock: f32,
}

/// Everything beyond the seed that shapes a map.
///
/// [`MapGenOptions::standard`] is the stock setup and is what the bare
/// [`crate::generate_map`] uses, so existing callers and saved worlds keep the
/// map they have always had.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct MapGenOptions {
    pub size: MapSize,
    pub terrain: TerrainStyle,
    pub water: WaterStyle,
    pub resources: ResourceSpread,
}

impl MapGenOptions {
    /// The stock setup: Standard 64², Rolling, Balanced, Scattered.
    pub const fn standard() -> Self {
        Self {
            size: MapSize::Standard,
            terrain: TerrainStyle::Rolling,
            water: WaterStyle::Balanced,
            resources: ResourceSpread::Scattered,
        }
    }

    /// Composition this setup aims for.
    ///
    /// `coastal` is the per-seed roll: a landlocked map spends the sea's share on
    /// buildable ground instead, which is where the owner wants it.
    pub fn composition_targets(self, coastal: bool) -> CompositionTargets {
        let inland_water = self.water.inland_target();
        let sea = if coastal { self.water.sea_target() } else { 0.0 };
        let rock = self.terrain.rock_target();
        CompositionTargets {
            buildable: 100.0 - inland_water - sea - rock,
            inland_water,
            sea,
            rock,
        }
    }

    /// Whether this seed's map has a coast at all.
    ///
    /// Sea is scenery, not structure: Sparse never has one, Balanced rolls for
    /// it, Riverlands always does. Rolled from the seed alone so the answer is
    /// stable and shareable.
    pub fn is_coastal(self, seed: u64) -> bool {
        let chance = self.water.coastal_chance();
        if chance == 0 {
            return false;
        }
        if chance >= 100 {
            return true;
        }
        // SplitMix64 finaliser, salted so the roll is independent of every other
        // decision the same seed drives.
        let mut z = seed ^ 0x0c0a_57a1_u64;
        z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) % 100) < chance as u64
    }

    /// Feature counts scale with area, so a Small map is not a Huge map's worth
    /// of landforms crammed into a quarter of the room.
    pub(crate) fn feature_scale(self, width: u32, height: u32) -> f32 {
        let area = (width as f32) * (height as f32);
        let standard = (MapSize::Standard.tiles() as f32).powi(2);
        (area / standard).sqrt().clamp(0.55, 2.0)
    }

    /// Options packed low bits first: size(2) · terrain(2) · water(2) · resources(1).
    ///
    /// Stable and tiny, so a save or a share code can carry the setup as one byte.
    pub const fn pack(self) -> u8 {
        (self.size.index() as u8)
            | ((self.terrain.index() as u8) << 2)
            | ((self.water.index() as u8) << 4)
            | ((self.resources.index() as u8) << 6)
    }

    /// Inverse of [`Self::pack`]. `None` on any bit pattern that names a choice
    /// that does not exist.
    pub fn unpack(bits: u8) -> Option<Self> {
        Some(Self {
            size: MapSize::from_index((bits & 0b11) as usize)?,
            terrain: TerrainStyle::from_index(((bits >> 2) & 0b11) as usize)?,
            water: WaterStyle::from_index(((bits >> 4) & 0b11) as usize)?,
            resources: ResourceSpread::from_index(((bits >> 6) & 0b1) as usize)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_is_the_default() {
        assert_eq!(MapGenOptions::default(), MapGenOptions::standard());
    }

    #[test]
    fn targets_sit_inside_the_design_bands() {
        // Every combination must be reachable, not just the middle one — a
        // Rugged Riverlands map is still an open landscape.
        for water in WaterStyle::ALL {
            for terrain in TerrainStyle::ALL {
                for coastal in [false, true] {
                    let targets = MapGenOptions {
                        water: *water,
                        terrain: *terrain,
                        ..MapGenOptions::standard()
                    }
                    .composition_targets(coastal);
                    let label = format!("{}/{} coastal={coastal}", water.label(), terrain.label());
                    assert!(
                        (85.0..=92.0).contains(&targets.buildable),
                        "{label}: buildable {}",
                        targets.buildable
                    );
                    assert!(
                        (4.0..=8.0).contains(&targets.inland_water),
                        "{label}: inland {}",
                        targets.inland_water
                    );
                    assert!(
                        (0.0..=4.0).contains(&targets.sea),
                        "{label}: sea {}",
                        targets.sea
                    );
                    assert!(
                        (4.0..=8.0).contains(&targets.rock),
                        "{label}: rock {}",
                        targets.rock
                    );
                }
            }
        }
    }

    #[test]
    fn most_maps_are_landlocked() {
        // "Many maps should have none at all." Sparse never; Riverlands always;
        // the default rolls, and the roll should come up dry more often than not.
        let coastal = |water| {
            (0u64..300)
                .filter(|seed| {
                    MapGenOptions {
                        water,
                        ..MapGenOptions::standard()
                    }
                    .is_coastal(*seed)
                })
                .count()
        };
        assert_eq!(coastal(WaterStyle::Sparse), 0);
        assert_eq!(coastal(WaterStyle::Riverlands), 300);
        let balanced = coastal(WaterStyle::Balanced);
        assert!(
            (60..140).contains(&balanced),
            "{balanced} of 300 Balanced maps rolled a coast"
        );
    }

    #[test]
    fn packing_round_trips_every_combination() {
        let mut seen = Vec::new();
        for size in MapSize::ALL {
            for terrain in TerrainStyle::ALL {
                for water in WaterStyle::ALL {
                    for resources in ResourceSpread::ALL {
                        let options = MapGenOptions {
                            size: *size,
                            terrain: *terrain,
                            water: *water,
                            resources: *resources,
                        };
                        let bits = options.pack();
                        assert_eq!(MapGenOptions::unpack(bits), Some(options));
                        assert!(!seen.contains(&bits), "packing collision at {bits:#010b}");
                        seen.push(bits);
                    }
                }
            }
        }
    }

    #[test]
    fn nearest_size_snaps_to_the_listed_edges() {
        assert_eq!(MapSize::nearest(64), MapSize::Standard);
        assert_eq!(MapSize::nearest(50), MapSize::Small);
        assert_eq!(MapSize::nearest(200), MapSize::Huge);
        assert_eq!(MapSize::nearest(2), MapSize::Small);
    }
}
