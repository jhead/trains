//! Deriving a goal set from the map seed and the anchors that map produced.
//!
//! Design 08 §8 wants goals to be *a lens on the sandbox*, so the set is chosen
//! from what the world already contains: the stops the generator seeded, how far
//! apart they ended up, and how wide their catchments are. A cramped map asks
//! for a short first line; a spread-out one asks for a longer haul. Nothing here
//! places anything or changes the world.
//!
//! # Determinism
//!
//! Generation is a pure function of `(seed, anchors)`. Every registry read is
//! sorted by id first — [`StationRegistry::iter`] walks a `HashMap` and its
//! order is not stable — and every target is integer arithmetic. The same seed
//! on the same map therefore produces the same set, which is what makes a
//! shared seed code mean the same *game* and not merely the same terrain.

use crate::ids::{StationId, TileCoord};
use crate::peeps::TICKS_PER_DAY;
use crate::stations::{IndustryRegistry, StationRegistry};

use super::goal::{Goal, GoalId, GoalKind};

/// Most goals one map asks for. Fewer on a world with too few anchors to carry
/// them; never more.
pub const GOALS_PER_SET: usize = 6;

/// Deadline for each goal in the set, in sim days, before jitter.
///
/// The shape matters more than the numbers: an opening objective that lands
/// inside the first few minutes (design 08 §7 — first payout by minute three),
/// then a widening ladder, then one long haul that is still open at the hour
/// mark. A sim day is [`TICKS_PER_DAY`] ticks.
const DEADLINE_DAYS: [u64; GOALS_PER_SET] = [2, 4, 7, 10, 12, 16];

/// Paid runs asked for, before per-map scaling.
const DELIVERIES_BASE: u64 = 60;
/// Extra runs asked for per anchor the world started with.
const DELIVERIES_PER_ANCHOR: u64 = 20;

/// Residents asked for, before per-map scaling.
const POPULATION_BASE: u64 = 6;
/// Extra residents per anchor. Above the six a district houses unserved, so the
/// goal cannot be met by standing still.
const POPULATION_PER_ANCHOR: u64 = 14;

/// Service score a stop must hold to bank time toward a "keep it served" goal.
const SERVE_MIN_SCORE: u8 = 55;

/// Share of a catchment's theoretical fill a "build up" goal asks for, percent.
const GROW_QUALITY_PERCENT: u64 = 40;

/// How far a target may drift from its nominal value, percent either way.
const TARGET_JITTER_PERCENT: u64 = 20;

/// Build this world's goal set. Empty when the map has no anchors to talk about.
pub fn generate_goal_set(
    seed: u64,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
) -> Vec<Goal> {
    let Some(anchors) = Anchors::pick(stations) else {
        return Vec::new();
    };
    // Anchors the generator laid down — the size of the problem this map poses.
    let world_anchors = (stations.len() + industries.len()) as u64;

    let mut out: Vec<Goal> = Vec::new();
    let mut next = |kind: GoalKind, title: String, target: u64, index: usize| {
        let id = GoalId(index as u32);
        let deadline = deadline_tick(seed, index);
        out.push(Goal::new(id, kind, title, target, deadline));
    };

    let home_name = name_of(stations, anchors.home);

    // 1 — The opening beat. The nearest neighbour, not the farthest: design 02
    // §4.1 is explicit that maximal separation is the worst possible first ask.
    if let Some(near) = anchors.near {
        next(
            GoalKind::Connect {
                from: anchors.home,
                to: near,
            },
            format!("Connect {home_name} to {}", name_of(stations, near)),
            1,
            0,
        );
    }

    // 2 — Throughput. Reaching a place is not the same as running a railway.
    let runs = jitter(
        seed,
        1,
        DELIVERIES_BASE + world_anchors.saturating_mul(DELIVERIES_PER_ANCHOR),
    );
    next(
        GoalKind::Deliveries,
        format!("Complete {runs} paid runs"),
        runs,
        1,
    );

    // 3 — Reliability. Expansion is not the only virtue (design 08 §2).
    let served = anchors.near.unwrap_or(anchors.home);
    next(
        GoalKind::Serve {
            station: served,
            min_score: SERVE_MIN_SCORE,
        },
        format!("Keep {} served", name_of(stations, served)),
        TICKS_PER_DAY,
        2,
    );

    // 4 — The town answering back.
    let residents = jitter(
        seed,
        2,
        POPULATION_BASE + world_anchors.saturating_mul(POPULATION_PER_ANCHOR),
    );
    next(
        GoalKind::Population,
        format!("Grow the town to {residents} residents"),
        residents,
        3,
    );

    // 5 — Density where the player's first line landed.
    let grown = anchors.near.unwrap_or(anchors.home);
    let radius = stations
        .get(grown)
        .map(|s| s.tier.catchment())
        .unwrap_or(GROWTH_FALLBACK_RADIUS);
    next(
        GoalKind::Grow { station: grown },
        format!("Build up {}", name_of(stations, grown)),
        grow_target_tenths(radius),
        4,
    );

    // 6 — The long haul, and the only place maximal separation is the right
    // objective (design 02 §4.2). Skipped when it would repeat goal 1.
    if let Some(far) = anchors.far.filter(|far| Some(*far) != anchors.near) {
        next(
            GoalKind::Connect {
                from: anchors.home,
                to: far,
            },
            format!("Reach {} by rail", name_of(stations, far)),
            1,
            5,
        );
    }

    out
}

/// Catchment used when a goal's station has vanished mid-generation.
const GROWTH_FALLBACK_RADIUS: i32 = 5;

/// The three stops a goal set is built around.
struct Anchors {
    /// Nearest stop to the centre of gravity of every anchor — the home town.
    home: StationId,
    /// Closest other stop to `home`. The opening beat.
    near: Option<StationId>,
    /// Furthest stop from `home`. The late objective.
    far: Option<StationId>,
}

impl Anchors {
    fn pick(stations: &StationRegistry) -> Option<Self> {
        // Sorted first: registry iteration order is a `HashMap`'s, and a goal
        // set that depended on it would differ between runs of the same seed.
        let mut ids: Vec<(StationId, TileCoord)> =
            stations.iter().map(|s| (s.id, s.tile)).collect();
        ids.sort_by_key(|(id, _)| id.0);
        let (first, _) = *ids.first()?;

        let count = ids.len() as i64;
        let cx = (ids.iter().map(|(_, t)| t.x as i64).sum::<i64>() / count) as i32;
        let cy = (ids.iter().map(|(_, t)| t.y as i64).sum::<i64>() / count) as i32;
        let centre = TileCoord { x: cx, y: cy };

        let home = ids
            .iter()
            .min_by_key(|(id, tile)| (chebyshev(*tile, centre), id.0))
            .map(|(id, _)| *id)
            .unwrap_or(first);
        let home_tile = ids
            .iter()
            .find(|(id, _)| *id == home)
            .map(|(_, tile)| *tile)?;

        let others: Vec<&(StationId, TileCoord)> =
            ids.iter().filter(|(id, _)| *id != home).collect();
        let near = others
            .iter()
            .min_by_key(|(id, tile)| (chebyshev(*tile, home_tile), id.0))
            .map(|(id, _)| *id);
        let far = others
            .iter()
            .max_by_key(|(id, tile)| (chebyshev(*tile, home_tile), id.0))
            .map(|(id, _)| *id);

        Some(Self { home, near, far })
    }
}

fn chebyshev(a: TileCoord, b: TileCoord) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

fn name_of(stations: &StationRegistry, id: StationId) -> String {
    stations
        .get(id)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "the next town".to_string())
}

/// Built density a catchment of `radius` is asked to reach, in tenths.
///
/// Derived from the same falloff [`catchment_influence`](crate::stations::catchment_influence)
/// applies, so a wider platform is asked for proportionally more and the number
/// stays reachable at a plausible service score.
fn grow_target_tenths(radius: i32) -> u64 {
    if radius <= 0 {
        return 10;
    }
    let span = i64::from(radius);
    let mut fill_milli: i64 = 0;
    for dy in -span..=span {
        for dx in -span..=span {
            let dist = dx.abs().max(dy.abs());
            fill_milli += 1_000 - (dist * 1_000) / (span + 1);
        }
    }
    // fill_milli is Σfalloff × 1000; a tenth is a tenth of one fully built tile.
    ((fill_milli.max(0) as u64).saturating_mul(GROW_QUALITY_PERCENT) / 10_000).max(10)
}

/// Deadline for the `index`-th goal, nudged up to a day either way by the seed.
fn deadline_tick(seed: u64, index: usize) -> u64 {
    let nominal = DEADLINE_DAYS[index.min(DEADLINE_DAYS.len() - 1)];
    // Never earlier than the previous rung's nominal day, so the ladder holds.
    let floor = if index == 0 {
        1
    } else {
        DEADLINE_DAYS[index - 1]
    };
    let drift = mix64(seed ^ (index as u64).wrapping_mul(0x9e37_79b9)) % 3; // 0, 1, 2
    let days = (nominal + drift).saturating_sub(1).max(floor);
    days.saturating_mul(TICKS_PER_DAY)
}

/// `value` moved up to [`TARGET_JITTER_PERCENT`] either way, from the seed.
fn jitter(seed: u64, salt: u64, value: u64) -> u64 {
    if value == 0 {
        return 0;
    }
    let span = TARGET_JITTER_PERCENT.saturating_mul(2) + 1;
    let offset = (mix64(seed ^ salt.wrapping_mul(0x2545_f491)) % span) as i64
        - TARGET_JITTER_PERCENT as i64;
    let scaled = (value as i64).saturating_mul(100 + offset) / 100;
    scaled.max(1) as u64
}

/// SplitMix64 finaliser. `rail_sim` has no rng dependency and does not need one
/// for this: the generator wants a stable spread, not statistical quality.
fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stations::{seed_stations_and_industries, StationService};
    use crate::track::GROUND_LAYER;

    fn seeded_world() -> (StationRegistry, IndustryRegistry) {
        let mut stations = StationRegistry::new();
        let mut industries = IndustryRegistry::new();
        let mut service = StationService::default();
        seed_stations_and_industries(
            &mut stations,
            &mut industries,
            &mut service,
            48,
            48,
            |_| true,
        );
        (stations, industries)
    }

    #[test]
    fn a_seeded_world_gets_a_full_set() {
        let (stations, industries) = seeded_world();
        let goals = generate_goal_set(84_213, &stations, &industries);
        assert_eq!(goals.len(), GOALS_PER_SET);
        assert!(goals.iter().all(|g| !g.title.is_empty()));
        assert!(goals.iter().all(|g| g.target >= 1));
        assert!(goals.iter().all(|g| g.is_active()));
    }

    #[test]
    fn the_same_seed_and_map_produce_the_same_set() {
        let (stations, industries) = seeded_world();
        let a = generate_goal_set(84_213, &stations, &industries);
        let b = generate_goal_set(84_213, &stations, &industries);
        assert_eq!(a, b, "a shared seed must mean the same game, not just terrain");
    }

    #[test]
    fn a_different_seed_poses_a_different_problem() {
        let (stations, industries) = seeded_world();
        let a = generate_goal_set(1, &stations, &industries);
        let b = generate_goal_set(2, &stations, &industries);
        assert_ne!(a, b);
    }

    #[test]
    fn the_opening_goal_is_the_nearest_neighbour_not_the_farthest() {
        // Design 02 §4.1 — a long unrewarded haul is the worst possible opening.
        let mut stations = StationRegistry::new();
        let home = stations.insert("Home", TileCoord { x: 20, y: 20 }, GROUND_LAYER);
        let near = stations.insert("Near", TileCoord { x: 28, y: 20 }, GROUND_LAYER);
        let far = stations.insert("Far", TileCoord { x: 20, y: 44 }, GROUND_LAYER);
        let goals = generate_goal_set(7, &stations, &IndustryRegistry::new());

        let first = goals.first().expect("a set exists");
        assert_eq!(first.kind, GoalKind::Connect { from: home, to: near });
        assert!(first.title.contains("Near"));

        let last = goals.last().expect("a long haul closes the set");
        assert_eq!(last.kind, GoalKind::Connect { from: home, to: far });
        assert!(
            last.deadline_tick > first.deadline_tick,
            "the long haul is the late objective"
        );
    }

    #[test]
    fn deadlines_never_go_backwards_on_any_seed() {
        let (stations, industries) = seeded_world();
        for seed in 0..64u64 {
            let goals = generate_goal_set(seed, &stations, &industries);
            let mut previous = 0;
            for goal in &goals {
                assert!(
                    goal.deadline_tick >= previous,
                    "seed {seed}: '{}' is due before the rung above it",
                    goal.title
                );
                assert!(goal.deadline_tick > 0, "seed {seed}: no deadline at all");
                previous = goal.deadline_tick;
            }
        }
    }

    #[test]
    fn a_two_stop_world_does_not_ask_for_the_same_link_twice() {
        let mut stations = StationRegistry::new();
        stations.insert("Home", TileCoord { x: 10, y: 10 }, GROUND_LAYER);
        stations.insert("Other", TileCoord { x: 18, y: 10 }, GROUND_LAYER);
        let goals = generate_goal_set(3, &stations, &IndustryRegistry::new());

        let connects = goals
            .iter()
            .filter(|g| matches!(g.kind, GoalKind::Connect { .. }))
            .count();
        assert_eq!(connects, 1, "near and far are the same stop here");
        assert_eq!(goals.len(), GOALS_PER_SET - 1);
    }

    #[test]
    fn a_world_with_no_anchors_gets_no_goals_rather_than_nonsense() {
        let goals = generate_goal_set(1, &StationRegistry::new(), &IndustryRegistry::new());
        assert!(goals.is_empty());
    }

    #[test]
    fn a_single_stop_world_still_gets_something_to_do() {
        let mut stations = StationRegistry::new();
        stations.insert("Alone", TileCoord { x: 10, y: 10 }, GROUND_LAYER);
        let goals = generate_goal_set(5, &stations, &IndustryRegistry::new());
        assert!(!goals.is_empty());
        assert!(
            goals
                .iter()
                .all(|g| !matches!(g.kind, GoalKind::Connect { .. })),
            "nothing to connect to"
        );
    }

    #[test]
    fn a_bigger_world_asks_for_more() {
        let mut small = StationRegistry::new();
        small.insert("A", TileCoord { x: 5, y: 5 }, GROUND_LAYER);
        small.insert("B", TileCoord { x: 12, y: 5 }, GROUND_LAYER);
        let mut big = small.clone();
        for i in 0..6 {
            big.insert(
                format!("S{i}"),
                TileCoord { x: 20 + i * 4, y: 20 },
                GROUND_LAYER,
            );
        }
        let runs = |reg: &StationRegistry| {
            generate_goal_set(11, reg, &IndustryRegistry::new())
                .into_iter()
                .find(|g| g.kind == GoalKind::Deliveries)
                .map(|g| g.target)
                .unwrap_or(0)
        };
        assert!(runs(&big) > runs(&small));
    }

    #[test]
    fn a_grow_target_stays_inside_what_a_catchment_can_hold() {
        for radius in 1..=8 {
            let target = grow_target_tenths(radius);
            let tiles = (2 * radius as u64 + 1).pow(2);
            assert!(target >= 10, "radius {radius} asks for nothing");
            assert!(
                target < tiles * 10,
                "radius {radius} asks for {target} tenths but holds {} at most",
                tiles * 10
            );
        }
        // The default Station catchment is 5; that number is load-bearing for
        // the readout in the panel.
        assert_eq!(grow_target_tenths(5), 190);
    }

    #[test]
    fn jitter_stays_inside_its_stated_band() {
        for salt in 0..128u64 {
            let value = jitter(salt, salt, 100);
            assert!(
                (80..=120).contains(&value),
                "{value} escaped +/-{TARGET_JITTER_PERCENT}%"
            );
        }
        assert_eq!(jitter(1, 1, 0), 0);
    }
}
