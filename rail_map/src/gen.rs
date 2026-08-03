//! Seeded procedural terrain generation.
//!
//! # The argument
//!
//! Design 02 §1: a map has succeeded if the player thinks *"obviously the line
//! goes through there"*, then *"…but that's a long way round, what if I cut over
//! the ridge?"*, then *"I can't afford that yet."* An obvious answer, a tempting
//! expensive one, and a reason to come back. The failure state is a map where the
//! straight line between two points is also the cheapest and the fastest — where
//! terrain is wallpaper.
//!
//! Noise does not produce that triple. So generation here is **feature-first**
//! (§2.2): a coastline is placed, ridges are drawn as spines with a named handful
//! of passes, plateaus and basins are stamped, rivers are routed down the
//! resulting valleys and given authored narrows, and only then is a little noise
//! added on top. Every stage is in a module of its own:
//!
//! | Stage | Module | Brief |
//! | --- | --- | --- |
//! | Elevation bands, relaxations | `field` | §2.3 legibility |
//! | Coast, bays, ridges, plateaus, basins, valleys | `shape` | §2.1, §2.2 |
//! | Rivers, narrows, lakes | `hydro` | §2.1, §3.4 |
//! | Opening beat, growth sites | `sites` | §4.1, §4.2 |
//!
//! # Hitting the numbers
//!
//! §2.1 states a composition target and this module *solves* for it rather than
//! hoping: the coastline bisects a margin thickness against the sea share, lakes
//! fill whatever inland-water budget the rivers left, and a single global offset
//! is bisected against the impassable-rock share. [`crate::measure`] then reads
//! the result back, and the tests below assert it across seeds.

mod field;
mod hydro;
mod shape;
mod sites;

use rail_sim::ids::TileCoord;

use crate::features::{MapFeatures, Surface};
use crate::grid::MapGrid;
use crate::options::MapGenOptions;
use crate::portal::Portal;
use crate::tile::{TerrainKind, Tile};
use crate::{EdgeFacing, PortalId};

use field::{band_height, Canvas, INLAND_DEPTH_MAX, SEA_DEPTH_MAX};

/// Generate a deterministic map: same `(width, height, seed)` → identical tiles & portals.
///
/// Uses [`MapGenOptions::standard`], so every existing caller keeps the stock map
/// shape. Reach for [`generate_map_with`] to steer it.
pub fn generate_map(width: u32, height: u32, seed: u64) -> MapGrid {
    generate_map_with(width, height, seed, MapGenOptions::standard())
}

/// Generate the map `options` describes, at the size they name.
///
/// Same `(seed, options)` → identical map, which is the whole of §5's share-code
/// promise: "sharing a code should reproduce someone else's world exactly."
pub fn generate(seed: u64, options: MapGenOptions) -> MapGrid {
    let n = options.size.tiles();
    generate_map_with(n, n, seed, options)
}

/// Generate at an explicit size with explicit options.
///
/// Kept separate from [`generate`] so ragged and test-sized maps stay possible;
/// the feature budget scales with area rather than assuming 64².
pub fn generate_map_with(width: u32, height: u32, seed: u64, options: MapGenOptions) -> MapGrid {
    assert!(width >= 2 && height >= 2, "map must be at least 2×2");

    let mut canvas = Canvas::new(width as usize, height as usize);
    let scale = options.feature_scale(width, height);

    // 1. A coast, if this map rolled one at all. Most do not: a Rail Town map is
    //    inland countryside, and land runs to the border unless an inlet says so.
    shape::carve_sea(&mut canvas, seed, options, scale);

    // 2. Landforms, then a provisional banding for water to run downhill over.
    let elevation = shape::landform_field(&canvas, seed, options, scale);
    shape::apply_field(&mut canvas, &elevation, 0.0);

    // 3. Rivers first — they are the best feature in the game and they get the
    //    valleys they want — then lakes to finish the inland-water budget.
    let rivers = hydro::carve_rivers(&mut canvas, seed, options, scale);
    let inland_target = (options.water.inland_target() / 100.0 * canvas.len() as f32) as usize;
    let river_tiles = canvas
        .surface
        .iter()
        .filter(|s| **s == Surface::River)
        .count();
    hydro::place_lakes(&mut canvas, seed, inland_target.saturating_sub(river_tiles));

    // 4. Re-band against the finished water, scaling the ridges until the
    //    impassable-rock share is on target.
    shape::solve_ridge_gain(&mut canvas, &elevation, options.terrain.rock_target());

    // 5. Materialise.
    let mut grid = MapGrid::empty(width, height, seed);
    compose(&canvas, &mut grid);
    place_edge_portals(&mut grid);

    // 6. Write down what generation meant, so anchor placement need not guess.
    let found = sites::choose_sites(&canvas, seed, options);
    let mut features = MapFeatures {
        surface: canvas.surface.clone(),
        home: found.home,
        near: found.near,
        sites: found.hints,
        crossings: Vec::new(),
        passes: Vec::new(),
    };
    for river in &rivers {
        features.crossings.extend(river.crossings.iter().copied());
    }
    *grid.features_mut() = features;
    grid.features_mut().passes = verified_passes(&grid, &canvas, &elevation.saddles);

    grid
}

/// Turn the saddles generation designed into passes it can stand behind.
///
/// A designed notch is only a pass if the finished map has buildable ground
/// there **and** rock either side of it — otherwise the ridge never reached rock
/// at that point and the "pass" is just open country. Reporting only the ones
/// that survived is what makes [`crate::MapFeatures::passes`] worth trusting.
fn verified_passes(
    grid: &MapGrid,
    canvas: &Canvas,
    saddles: &[usize],
) -> Vec<TileCoord> {
    let reach = crate::measure::PASS_REACH;
    let rock = |c: TileCoord| {
        grid.get(c)
            .is_some_and(|t| !t.water && t.height >= crate::measure::ROCK_HEIGHT_MIN)
    };
    let mut out: Vec<TileCoord> = Vec::new();

    for &cell in saddles {
        let origin = TileCoord {
            x: (cell % canvas.w) as i32,
            y: (cell / canvas.w) as i32,
        };
        let found = (0..=reach)
            .flat_map(|r| {
                (-r..=r).flat_map(move |dy| {
                    (-r..=r).map(move |dx| (r, dx, dy))
                })
            })
            .filter(|(r, dx, dy)| dx.abs().max(dy.abs()) == *r)
            .map(|(_, dx, dy)| TileCoord {
                x: origin.x + dx,
                y: origin.y + dy,
            })
            .find(|&c| {
                crate::measure::is_buildable(grid, c)
                    && [(1, 0), (0, 1), (1, 1), (1, -1)].into_iter().any(|(dx, dy)| {
                        (1..=reach).any(|d| {
                            rock(TileCoord {
                                x: c.x - dx * d,
                                y: c.y - dy * d,
                            })
                        }) && (1..=reach).any(|d| {
                            rock(TileCoord {
                                x: c.x + dx * d,
                                y: c.y + dy * d,
                            })
                        })
                    })
            });
        if let Some(tile) = found {
            if !out
                .iter()
                .any(|p| (p.x - tile.x).abs() + (p.y - tile.y).abs() < 4)
            {
                out.push(tile);
            }
        }
    }
    out
}

/// Turn bands and surface classes into the tiles the rest of the game reads.
///
/// Height is the band's height, flat — no in-band jitter. That is deliberate:
/// §2.3 wants "discrete bands with visible steps", and a tile nudged one unit off
/// its band would put a 1× contour route next to a 6× cut-and-fill for no reason
/// the player can see.
fn compose(canvas: &Canvas, grid: &mut MapGrid) {
    // Depth is distance from dry land, so shallows ring every shore and the ramp
    // only reaches its dark step out in open water — §2.1's "sea is punctuation"
    // drawn the way brief 01 §6.2.3 asks.
    let from_land = canvas.distance_to(|i| !canvas.surface[i].is_water());

    for y in 0..canvas.h as i32 {
        for x in 0..canvas.w as i32 {
            let index = canvas.at(x, y);
            let surface = canvas.surface[index];
            let tile = match surface {
                Surface::Land => {
                    let band = canvas.band[index];
                    let coastal = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|(dx, dy)| {
                        canvas.idx(x + dx, y + dy).is_some_and(|n| {
                            matches!(canvas.surface[n], Surface::Sea | Surface::Lake)
                        })
                    });
                    let kind = if band == 0 && coastal {
                        // Sand is a shoreline, not an elevation band: rivers keep
                        // their green banks so a river reads as a river.
                        TerrainKind::Beach
                    } else {
                        match band {
                            0 | 1 => TerrainKind::Plains,
                            2 | 3 => TerrainKind::Hills,
                            _ => TerrainKind::Mountain,
                        }
                    };
                    Tile {
                        height: band_height(band),
                        water: false,
                        kind,
                    }
                }
                _ => {
                    let cap = if surface == Surface::Sea {
                        SEA_DEPTH_MAX
                    } else {
                        INLAND_DEPTH_MAX
                    };
                    let depth = (from_land[index] as i32).clamp(1, cap);
                    Tile {
                        height: -depth as i8,
                        water: true,
                        kind: TerrainKind::Water,
                    }
                }
            };
            *grid.get_mut(TileCoord { x, y }).expect("in-bounds") = tile;
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
    use crate::measure;
    use crate::options::{MapSize, ResourceSpread, TerrainStyle, WaterStyle};
    use rail_sim::{
        tile_build_cost, TrackTerrain, MAX_BRIDGE_SPAN, MAX_GRADE, MOUNTAIN_HEIGHT_MIN,
        TRACK_COST_CENTS,
    };
    use std::collections::BTreeMap;

    /// Seeds every composition / feature claim is checked against. Six worlds,
    /// not one lucky one.
    const SEEDS: [u64; 6] = [1, 42, 777, 9_001, 31_415, 65_535];

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
        assert_eq!(a.features(), b.features());
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
    fn ragged_and_tiny_maps_still_generate() {
        // Sizes the renderer's own tests use, plus the stated minimum.
        for (w, h) in [(2, 2), (16, 16), (24, 24), (37, 21), (50, 34), (32, 32)] {
            let map = generate_map(w, h, 3);
            assert_eq!(map.tiles().len(), (w * h) as usize);
            assert!(
                map.tiles().iter().any(|t| !t.water),
                "{w}×{h} drowned entirely"
            );
        }
    }

    // -- Design 02 §2.1: composition ---------------------------------------

    #[test]
    fn composition_meets_the_design_targets_across_seeds() {
        for seed in SEEDS {
            let map = generate_map(64, 64, seed);
            let c = measure::composition(&map);
            assert!(
                c.meets_design_targets(),
                "seed {seed}: buildable {:.1}% inland {:.1}% sea {:.1}% rock {:.1}%",
                c.buildable,
                c.inland_water,
                c.sea,
                c.rock
            );
        }
    }

    #[test]
    fn every_size_and_style_lands_in_the_design_bands() {
        // Not one lucky configuration: every Terrain × Water pair at two sizes,
        // over several seeds. A handful of maps miss a band by a point — a small
        // map whose one ridge the shoreline planed down runs light on rock — so
        // the bar is that the generator lands it essentially always, and that
        // nothing is ever wildly out.
        let mut checked = 0;
        let mut on_target = 0;
        for size in [MapSize::Small, MapSize::Standard] {
            for terrain in TerrainStyle::ALL {
                for water in WaterStyle::ALL {
                    for seed in 0..3u64 {
                        let options = MapGenOptions {
                            size,
                            terrain: *terrain,
                            water: *water,
                            resources: ResourceSpread::Scattered,
                        };
                        let c = measure::composition(&generate(seed, options));
                        let label = format!(
                            "{} {}/{} seed {seed}",
                            size.label(),
                            terrain.label(),
                            water.label()
                        );
                        checked += 1;
                        on_target += usize::from(c.meets_design_targets());
                        // Nothing may be wildly out, whatever the seed.
                        assert!(
                            (83.0..=94.0).contains(&c.buildable)
                                && c.inland_water <= 8.0
                                && c.sea <= 4.0
                                && c.rock <= 8.0,
                            "{label}: buildable {:.1}% inland {:.1}% sea {:.1}% rock {:.1}%",
                            c.buildable,
                            c.inland_water,
                            c.sea,
                            c.rock
                        );
                    }
                }
            }
        }
        assert!(
            on_target * 100 >= checked * 95,
            "only {on_target} of {checked} maps hit every composition band"
        );
    }

    #[test]
    fn the_default_map_is_inland_countryside() {
        // Playtest: "the default map should read as inland countryside", and the
        // edge bias that made every map an island is gone. Land runs to the frame.
        let map = generate_map(DEFAULT_MAP_WIDTH, DEFAULT_MAP_HEIGHT, DEFAULT_MAP_SEED);
        let edge = map.width as i32 - 1;
        let dry = (0..=edge)
            .flat_map(|k| [(k, 0), (k, edge), (0, k), (edge, k)])
            .filter(|&(x, y)| !map.tile(TileCoord { x, y }).water)
            .count();
        assert!(
            dry * 100 / (4 * (edge as usize + 1)) >= 75,
            "only {dry} border tiles are dry — the map is still an island"
        );
        assert!(measure::composition(&map).sea <= 4.0);
    }

    #[test]
    fn water_has_both_a_shore_and_an_open_middle() {
        // The atmosphere layer draws foam where water meets land and glints where
        // it does not, so a map with only one-tile puddles would have nothing to
        // shimmer. A four-tile channel is wide enough for both.
        let map = generate_map(64, 64, DEFAULT_MAP_SEED);
        let mut shore = 0usize;
        let mut open = 0usize;
        for y in 0..64i32 {
            for x in 0..64i32 {
                if !map.tile(TileCoord { x, y }).water {
                    continue;
                }
                let touches_land = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|(dx, dy)| {
                    map.get(TileCoord {
                        x: x + dx,
                        y: y + dy,
                    })
                    .is_some_and(|t| !t.water)
                });
                if touches_land {
                    shore += 1;
                } else {
                    open += 1;
                }
            }
        }
        assert!(shore > 0 && open > 0, "shore {shore}, open {open}");
    }

    // -- Design 02 §2.1 / §3.4: rivers -------------------------------------

    #[test]
    fn every_map_has_a_river_with_two_to_four_crossings_of_differing_width() {
        for seed in SEEDS {
            let map = generate_map(64, 64, seed);
            let crossings = map.features().crossings.clone();
            assert!(
                (2..=4).contains(&crossings.len()),
                "seed {seed}: {} crossings",
                crossings.len()
            );
            let mut spans: Vec<u32> = crossings.iter().map(|c| c.span).collect();
            spans.sort_unstable();
            spans.dedup();
            assert!(
                spans.len() >= 2,
                "seed {seed}: every crossing is the same width ({spans:?})"
            );
            for crossing in &crossings {
                assert!(crossing.span <= MAX_BRIDGE_SPAN);
                assert_eq!(
                    measure::crossing_span(&map, crossing.tile),
                    Some(crossing.span),
                    "seed {seed}: recorded crossing {:?} is not actually bridgeable",
                    crossing.tile
                );
            }
        }
    }

    #[test]
    fn crossings_are_scarce_enough_to_be_worth_scouting() {
        // §3.4: "scarce enough to be worth scouting and plentiful enough that no
        // map has exactly one answer."
        for seed in SEEDS {
            let map = generate_map(64, 64, seed);
            let found = measure::river_crossings(&map);
            assert!(
                (2..=12).contains(&found.len()),
                "seed {seed}: {} places to cross is not a decision",
                found.len()
            );
        }
    }

    #[test]
    fn rivers_are_systems_not_puddles() {
        for seed in SEEDS {
            let map = generate_map(64, 64, seed);
            let surfaces = measure::surfaces(&map);
            let river = surfaces.iter().filter(|s| **s == Surface::River).count();
            assert!(
                river > 120,
                "seed {seed}: {river} river tiles is a stream, not a system"
            );
        }
    }

    // -- Design 02 §2.2 / §2.3: landforms and legibility --------------------

    #[test]
    fn ridges_have_a_small_number_of_passes() {
        // §2.2 exactly: "A ridge with exactly two passes is a decision; a ridge
        // with twenty is a texture." These are the saddles generation *designed*,
        // kept only where the finished map really does have rock either side.
        for seed in SEEDS {
            let map = generate_map(64, 64, seed);
            let passes = map.features().passes.clone();
            assert!(
                (1..=6).contains(&passes.len()),
                "seed {seed}: {} passes",
                passes.len()
            );
            // One ridge on a Standard Rolling map with two saddles designed, and
            // only the ones the finished map really walls in are kept.
            assert!(passes.len() <= 2 + 2 * TerrainStyle::Rolling.ridges());
            for tile in passes {
                assert!(
                    measure::is_buildable(&map, tile),
                    "seed {seed}: pass {tile:?} is not buildable — it is not a pass"
                );
            }
        }
    }

    #[test]
    fn the_shape_finder_agrees_that_the_gaps_are_few() {
        // The New Map screen reads passes off the finished grid rather than out
        // of the generator's notes, so the two counts must tell the same story.
        for seed in SEEDS {
            let map = generate_map(64, 64, seed);
            let found = measure::ridge_passes(&map).len();
            assert!(found <= 12, "seed {seed}: {found} gaps is a texture");
        }
    }

    #[test]
    fn rock_forms_walls_rather_than_confetti() {
        // A wall is a big connected clump. Speckled single tiles read as noise,
        // and the renderer would draw them as cliff confetti.
        for seed in SEEDS {
            let map = generate_map(64, 64, seed);
            let mut seen = vec![false; map.tiles().len()];
            let idx = |x: i32, y: i32| (y * 64 + x) as usize;
            let rock = |x: i32, y: i32| {
                map.get(TileCoord { x, y })
                    .is_some_and(|t| !t.water && t.height >= MOUNTAIN_HEIGHT_MIN)
            };
            let mut largest = 0usize;
            let mut total = 0usize;
            let mut clumped = 0usize;
            for y in 0..64i32 {
                for x in 0..64i32 {
                    if seen[idx(x, y)] || !rock(x, y) {
                        continue;
                    }
                    let mut stack = vec![(x, y)];
                    seen[idx(x, y)] = true;
                    let mut size = 0usize;
                    while let Some((cx, cy)) = stack.pop() {
                        size += 1;
                        total += 1;
                        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                            let (nx, ny) = (cx + dx, cy + dy);
                            if nx < 0 || ny < 0 || nx >= 64 || ny >= 64 {
                                continue;
                            }
                            if seen[idx(nx, ny)] || !rock(nx, ny) {
                                continue;
                            }
                            seen[idx(nx, ny)] = true;
                            stack.push((nx, ny));
                        }
                    }
                    largest = largest.max(size);
                    if size >= 10 {
                        clumped += size;
                    }
                }
            }
            // Two or three ridges, each broken at its passes, is a handful of
            // long clumps. What must not happen is a dust of single tiles: those
            // read as confetti and the renderer draws a cliff face round each one.
            assert!(
                largest >= 20,
                "seed {seed}: biggest rock clump is {largest} tiles — no walls here"
            );
            assert!(
                clumped * 100 / total.max(1) >= 70,
                "seed {seed}: only {clumped} of {total} rock tiles are in real walls"
            );
        }
    }

    #[test]
    fn every_step_between_neighbouring_land_tiles_is_a_drawn_edge() {
        // §2.3: elevation resolves into discrete bands with visible steps. The
        // renderer draws a bank at Δ3 and a full cliff face at Δ4 = MAX_GRADE,
        // so a generated step must be 0, 3 or 4 — never 1, 2, or 5+.
        let map = generate_map(64, 64, DEFAULT_MAP_SEED);
        for y in 0..64i32 {
            for x in 0..64i32 {
                let here = map.tile(TileCoord { x, y });
                if here.water {
                    continue;
                }
                for (dx, dy) in [(1, 0), (0, 1)] {
                    let Some(next) = map.get(TileCoord {
                        x: x + dx,
                        y: y + dy,
                    }) else {
                        continue;
                    };
                    if next.water {
                        continue;
                    }
                    let delta = (here.height - next.height).unsigned_abs();
                    assert!(
                        delta == 0 || (3..=MAX_GRADE).contains(&delta),
                        "({x}, {y}) steps {delta} — neither flat nor a drawn edge"
                    );
                }
            }
        }
    }

    #[test]
    fn every_rung_of_the_cost_ladder_occurs_on_a_generated_map() {
        // A rung the generator never produces is a row of §3.1 that does not
        // exist, so every one is counted on real terrain rather than trusted
        // from the table — the 1.5× gentle slope in particular, which is only
        // reachable because the step above the plains band is Δ3 and the cost
        // bands are cut to match. Tunnels are not built yet, so the brief's 15×
        // row has no rung here.
        const LADDER: [i64; 8] = [1_000, 1_500, 3_000, 6_000, 8_000, 10_000, 14_000, 20_000];

        for seed in SEEDS {
            let map = generate_map(64, 64, seed);
            let terrain = TrackTerrain::new(
                map.width,
                map.height,
                map.tiles().iter().map(|t| (t.water, t.height)),
            );
            let mut counts = BTreeMap::new();
            let mut refused = 0usize;
            for y in 0..64i32 {
                for x in 0..64i32 {
                    match tile_build_cost(&terrain, TileCoord { x, y }) {
                        // Milli-multiples of base, so 1.5× is exact.
                        Ok(cost) => *counts
                            .entry(cost * 1_000 / TRACK_COST_CENTS)
                            .or_insert(0usize) += 1,
                        Err(_) => refused += 1,
                    }
                }
            }

            for rung in LADDER {
                assert!(
                    counts.get(&rung).copied().unwrap_or(0) > 0,
                    "seed {seed}: nothing on the map costs {}× — realised ladder {counts:?}",
                    rung as f32 / 1_000.0
                );
            }
            assert!(refused > 0, "seed {seed}: no cliff refuses track at all");
        }
    }

    #[test]
    fn land_is_climbable_everywhere_that_is_not_rock() {
        // The wall is the rock crest and nothing else, so §2.2's "a small number
        // of passes" is a true statement about the map rather than a hope.
        for seed in SEEDS {
            let map = generate_map(64, 64, seed);
            for y in 0..64i32 {
                for x in 0..64i32 {
                    let coord = TileCoord { x, y };
                    if !measure::is_buildable(&map, coord) {
                        continue;
                    }
                    let here = map.tile(coord);
                    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let n = TileCoord {
                            x: x + dx,
                            y: y + dy,
                        };
                        let Some(next) = map.get(n) else { continue };
                        let delta = (here.height - next.height).unsigned_abs();
                        assert!(
                            delta <= MAX_GRADE + 1,
                            "seed {seed}: ({x}, {y}) has local relief {delta}, \
                             which placement refuses outright"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_mainland_is_one_place_you_can_build_across() {
        for seed in SEEDS {
            let map = generate_map(64, 64, seed);
            let buildable = map
                .tiles()
                .iter()
                .filter(|t| !t.water && t.height < MOUNTAIN_HEIGHT_MIN)
                .count();
            let largest = measure::largest_buildable_region(&map);
            assert!(
                largest * 100 / buildable.max(1) >= 45,
                "seed {seed}: biggest buildable region is {largest} of {buildable}"
            );
        }
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
        assert!(
            measure::largest_buildable_region(&map) >= 200,
            "largest landmass too small for track loop"
        );
    }

    // -- Design 02 §4.1: the opening beat -----------------------------------

    #[test]
    fn the_opening_pair_is_eight_to_twelve_tiles_apart() {
        for seed in SEEDS {
            let map = generate_map(64, 64, seed);
            let features = map.features();
            let home = features.home.expect("a home town");
            let near = features.near.expect("a near destination");
            let d = (((home.x - near.x).pow(2) + (home.y - near.y).pow(2)) as f32).sqrt();
            assert!(
                (8.0..=12.0).contains(&d),
                "seed {seed}: opening pair {d:.1} tiles apart"
            );
            assert!(measure::is_buildable(&map, home));
            assert!(measure::is_buildable(&map, near));
        }
    }

    #[test]
    fn home_is_near_the_centre_and_clear_of_the_edge() {
        for seed in SEEDS {
            let map = generate_map(64, 64, seed);
            let home = map.features().home.expect("a home town");
            let from_centre = (home.x - 31).abs().max((home.y - 31).abs());
            assert!(
                from_centre <= 16,
                "seed {seed}: home at {home:?} is not near the centre"
            );
            for tile in [home, map.features().near.expect("near")] {
                let edge = tile.x.min(tile.y).min(63 - tile.x).min(63 - tile.y);
                assert!(edge >= 4, "seed {seed}: {tile:?} is wedged against the edge");
            }
        }
    }

    #[test]
    fn the_opening_pair_is_on_one_piece_of_buildable_ground() {
        // The first line must be layable without solving the map first.
        for seed in SEEDS {
            let map = generate_map(64, 64, seed);
            let home = map.features().home.expect("home");
            let near = map.features().near.expect("near");
            let mut seen = vec![false; map.tiles().len()];
            let idx = |c: TileCoord| (c.y * 64 + c.x) as usize;
            let mut stack = vec![home];
            seen[idx(home)] = true;
            let mut reached = false;
            while let Some(c) = stack.pop() {
                if c == near {
                    reached = true;
                    break;
                }
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let n = TileCoord {
                        x: c.x + dx,
                        y: c.y + dy,
                    };
                    if !measure::is_buildable(&map, n) || seen[idx(n)] {
                        continue;
                    }
                    seen[idx(n)] = true;
                    stack.push(n);
                }
            }
            assert!(
                reached,
                "seed {seed}: the opening pair is not connected by land"
            );
        }
    }

    #[test]
    fn anchor_hints_are_sensible_ground() {
        for seed in SEEDS {
            let map = generate_map(64, 64, seed);
            let hints = map.features().anchor_hints();
            assert!(hints.len() >= 5, "seed {seed}: only {} hints", hints.len());
            for tile in hints {
                assert!(
                    measure::is_buildable(&map, tile),
                    "seed {seed}: hint {tile:?} is not buildable"
                );
            }
        }
    }

    // -- Design 02 §5: the options actually steer ---------------------------

    #[test]
    fn terrain_style_changes_how_hard_the_terrain_argues() {
        let base = MapGenOptions::standard();
        let gentle = measure::composition(&generate(
            2024,
            MapGenOptions {
                terrain: TerrainStyle::Gentle,
                ..base
            },
        ));
        let rugged = measure::composition(&generate(
            2024,
            MapGenOptions {
                terrain: TerrainStyle::Rugged,
                ..base
            },
        ));
        assert!(
            rugged.rock > gentle.rock + 1.0,
            "Rugged {:.1}% rock vs Gentle {:.1}%",
            rugged.rock,
            gentle.rock
        );
    }

    #[test]
    fn water_style_changes_how_much_of_the_puzzle_is_crossings() {
        let base = MapGenOptions::standard();
        let sparse = generate(
            2024,
            MapGenOptions {
                water: WaterStyle::Sparse,
                ..base
            },
        );
        let riverlands = generate(
            2024,
            MapGenOptions {
                water: WaterStyle::Riverlands,
                ..base
            },
        );
        assert!(
            measure::composition(&riverlands).inland_water
                > measure::composition(&sparse).inland_water + 1.0
        );
        assert!(
            riverlands.features().crossings.len() > sparse.features().crossings.len(),
            "Riverlands should offer more places to cross"
        );
    }

    #[test]
    fn options_steer_without_being_stirred_into_the_seed() {
        // The coastline is drawn from its own stream, so stepping Terrain keeps
        // the world recognisable instead of rolling a new one. That is the whole
        // difference between an option that steers and an option that re-rolls.
        let base = MapGenOptions {
            size: MapSize::Small,
            ..MapGenOptions::standard()
        };
        let a = generate(
            99,
            MapGenOptions {
                terrain: TerrainStyle::Gentle,
                ..base
            },
        );
        let b = generate(
            99,
            MapGenOptions {
                terrain: TerrainStyle::Rugged,
                ..base
            },
        );
        let sea_a = measure::surfaces(&a);
        let sea_b = measure::surfaces(&b);
        let same = (0..a.tiles().len())
            .filter(|&i| (sea_a[i] == Surface::Sea) == (sea_b[i] == Surface::Sea))
            .count();
        assert!(
            same * 100 / a.tiles().len() >= 90,
            "the coastline moved when only the terrain style changed"
        );
    }

    #[test]
    fn resource_spread_moves_the_growth_sites() {
        let spread = |resources| {
            let map = generate(
                7,
                MapGenOptions {
                    resources,
                    ..MapGenOptions::standard()
                },
            );
            let towns: Vec<TileCoord> = map
                .features()
                .sites_of(crate::features::SiteKind::Town)
                .collect();
            let mut worst = 0i32;
            for (i, a) in towns.iter().enumerate() {
                for b in &towns[i + 1..] {
                    worst = worst.max((a.x - b.x).abs() + (a.y - b.y).abs());
                }
            }
            worst
        };
        assert!(
            spread(ResourceSpread::Scattered) > spread(ResourceSpread::Clustered),
            "Clustered towns should sit closer together than Scattered ones"
        );
    }

    #[test]
    fn generation_is_fast_enough_for_a_live_preview() {
        // The New Map screen regenerates synchronously on every option change, so
        // a whole map has to fit inside a frame. The budget below is the release
        // build the player runs; an unoptimised test build is roughly 20× slower
        // and is bounded only loosely enough to catch a real regression.
        let budget = if cfg!(debug_assertions) { 600 } else { 25 };
        let started = std::time::Instant::now();
        for seed in SEEDS {
            let _ = generate_map(128, 128, seed);
        }
        let each = started.elapsed() / SEEDS.len() as u32;
        assert!(
            each.as_millis() < budget,
            "128² took {each:?} per map — too slow to preview live"
        );
    }
}




