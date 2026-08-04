//! The promise, end to end: *"once a station is placed and serviced by a line,
//! I would expect the city to grow around it."*
//!
//! Every step here is the command path the player's own clicks take —
//! [`CommandKind::PlaceTrack`] for the line, [`CommandKind::PlaceStation`] for
//! the platform, [`CommandKind::CreateLine`] to put the new stop on a route, and
//! [`CommandKind::AssignTrainToLine`] to send a train round it. Nothing pokes
//! [`StationRegistry`] or [`StationService`] directly, because the point is that
//! the loop closes without a hand on the scales.
//!
//! Lives in `rail_town` because it needs both `rail_map::generate_map` and
//! `rail_sim::SimPlugin`, and `rail_sim` cannot depend on `rail_map` (cycle).

use std::collections::{HashMap, VecDeque};

use bevy::prelude::{App, FixedUpdate, Update};
use rail_map::{generate_map, MapGrid, DEFAULT_MAP_HEIGHT, DEFAULT_MAP_SEED, DEFAULT_MAP_WIDTH};
use rail_sim::ids::{StationId, TileCoord};
use rail_sim::{
    AssignTrainToLine, BuyTrain, CommandBuffer, CommandKind, ComplaintFeed, CreateLine, LineId,
    LineRegistry, Money, PlaceStation, PlaceTrack, PlaceTrain, RemoveLine, SimPlugin,
    StationRegistry, StationService, StationTier, TownDensity, TrackNetwork, TrackTerrain,
    TrainKind, TrainYard, CHEAP_BRIDGE_SPAN, GROUND_LAYER, MAX_GRADE, MIN_STATION_SPACING,
    MOUNTAIN_HEIGHT_MIN,
};

/// Ticks the railway is given to turn a new platform into houses.
///
/// Jobs are posted every 45 ticks and a transit covers a tile in ~3, so this is
/// tens of round trips — generous, but bounded: a promise that only comes true
/// eventually is not a promise.
const HORIZON_TICKS: u32 = 9_000;

/// How far out of the new stop's ring the growth is measured.
const MEASURE_RADIUS: i32 = 3;

fn terrain_from_map(map: &MapGrid) -> TrackTerrain {
    let mut cells = Vec::with_capacity((map.width as usize) * (map.height as usize));
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

/// Tiles this test is willing to route over — land, or water narrow enough for
/// a cheap span (see `mvp_playable_loop` for why the wide ones are excluded).
fn tile_placeable(terrain: &TrackTerrain, tile: TileCoord) -> bool {
    if !terrain.contains(tile) {
        return false;
    }
    if terrain.is_water(tile) {
        let h = terrain.water_span_horizontal(tile);
        let v = terrain.water_span_vertical(tile);
        return h.min(v) <= CHEAP_BRIDGE_SPAN;
    }
    if terrain.height_at(tile).unwrap_or(0) >= MOUNTAIN_HEIGHT_MIN {
        return false;
    }
    rail_sim::local_slope(terrain, tile) <= MAX_GRADE + 1
}

fn grade_ok(terrain: &TrackTerrain, a: TileCoord, b: TileCoord) -> bool {
    if terrain.is_water(a) || terrain.is_water(b) {
        return true;
    }
    let ha = terrain.height_at(a).unwrap_or(0);
    let hb = terrain.height_at(b).unwrap_or(0);
    (ha as i16 - hb as i16).unsigned_abs() as u8 <= MAX_GRADE
}

/// 4-connected BFS over land / short bridges, respecting grade limits.
fn find_build_path(
    terrain: &TrackTerrain,
    from: TileCoord,
    to: TileCoord,
) -> Option<Vec<TileCoord>> {
    if !tile_placeable(terrain, from) || !tile_placeable(terrain, to) {
        return None;
    }
    let mut prev: HashMap<(i32, i32), (i32, i32)> = HashMap::new();
    let mut q = VecDeque::new();
    q.push_back(from);
    prev.insert((from.x, from.y), (from.x, from.y));

    const ORTHO: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    while let Some(cur) = q.pop_front() {
        for (dx, dy) in ORTHO {
            let n = TileCoord {
                x: cur.x + dx,
                y: cur.y + dy,
            };
            if prev.contains_key(&(n.x, n.y)) || !tile_placeable(terrain, n) {
                continue;
            }
            if !grade_ok(terrain, cur, n) {
                continue;
            }
            prev.insert((n.x, n.y), (cur.x, cur.y));
            if n == to {
                let mut path = vec![to];
                let mut c = (to.x, to.y);
                while c != (from.x, from.y) {
                    c = prev[&c];
                    path.push(TileCoord { x: c.0, y: c.1 });
                }
                path.reverse();
                return Some(path);
            }
            q.push_back(n);
        }
    }
    None
}

fn chebyshev(a: TileCoord, b: TileCoord) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

fn run_fixed(app: &mut App, ticks: u32) {
    for _ in 0..ticks {
        app.world_mut().run_schedule(FixedUpdate);
    }
}

/// Keep the treasury liquid: this test is about growth, not about the
/// affordability curve (`rail_sim/tests/economy_arc.rs` owns that).
fn top_up(app: &mut App) {
    let mut money = app.world_mut().resource_mut::<Money>();
    if money.cents() < 1_000_000 {
        let need = 1_000_000 - money.cents();
        money.credit(need);
    }
}

fn push(app: &mut App, kind: CommandKind) {
    app.world_mut().resource_mut::<CommandBuffer>().push(kind);
}

/// Total density in a square of `MEASURE_RADIUS` around `tile`, counting only
/// tiles no *other* station already reaches — otherwise a neighbour's town
/// would be credited to the platform under test.
fn density_around(
    density: &TownDensity,
    stations: &StationRegistry,
    tile: TileCoord,
    ignore: StationId,
) -> f32 {
    let mut total = 0.0;
    for dy in -MEASURE_RADIUS..=MEASURE_RADIUS {
        for dx in -MEASURE_RADIUS..=MEASURE_RADIUS {
            let t = TileCoord {
                x: tile.x + dx,
                y: tile.y + dy,
            };
            let claimed = stations
                .iter()
                .any(|s| s.id != ignore && chebyshev(s.tile, t) <= s.tier.catchment());
            if !claimed {
                total += density.get(t);
            }
        }
    }
    total
}

#[test]
fn a_new_station_on_a_serviced_line_grows_a_town_around_it() {
    let map = generate_map(DEFAULT_MAP_WIDTH, DEFAULT_MAP_HEIGHT, DEFAULT_MAP_SEED);
    let terrain = terrain_from_map(&map);

    let mut app = App::new();
    app.add_plugins(SimPlugin);
    app.insert_resource(terrain.clone());
    app.insert_resource(Money::new(10_000_000));
    // Seed the opening anchors through `SimPlugin`'s own hook.
    app.world_mut().run_schedule(Update);

    // Sorted, because the registry walks a `HashMap`: unsorted, two runs of the
    // same seed pick different anchors and this stops being one test.
    let mut seeded: Vec<(StationId, TileCoord)> = app
        .world()
        .resource::<StationRegistry>()
        .iter()
        .map(|s| (s.id, s.tile))
        .collect();
    seeded.sort_by_key(|(id, _)| id.0);
    assert!(seeded.len() >= 2, "a generated world seeds its anchors");

    // ---- The railway the player already has: two anchors joined by track. ----
    let (a, b, path) = seeded
        .iter()
        .enumerate()
        .flat_map(|(i, from)| seeded.iter().skip(i + 1).map(move |to| (from, to)))
        .find_map(|((a_id, a_tile), (b_id, b_tile))| {
            let path = find_build_path(&terrain, *a_tile, *b_tile)?;
            // Long enough to hold a third stop three tiles clear of both ends.
            (path.len() >= 3 * MIN_STATION_SPACING as usize)
                .then_some((*a_id, *b_id, path))
        })
        .expect("two seeded anchors a railway can join");

    for tile in &path {
        push(
            &mut app,
            CommandKind::PlaceTrack(PlaceTrack {
                tile: *tile,
                layer: GROUND_LAYER,
            }),
        );
    }
    run_fixed(&mut app, 1);
    {
        let network = app.world().resource::<TrackNetwork>();
        assert!(
            path.iter()
                .all(|t| network.id_at(*t, GROUND_LAYER).is_some()),
            "the whole route should be track"
        );
    }

    push(
        &mut app,
        CommandKind::CreateLine(CreateLine {
            name: Some("Valley Line".into()),
            stops: vec![a, b],
        }),
    );
    run_fixed(&mut app, 1);
    let line: LineId = app
        .world()
        .resource::<LineRegistry>()
        .iter()
        .next()
        .expect("the line the player just drew")
        .id;

    // One train, because the line is single track: a second meets it head-on,
    // which is the network this game has until passing loops exist.
    push(
        &mut app,
        CommandKind::BuyTrain(BuyTrain {
            kind: TrainKind::Transit,
        }),
    );
    run_fixed(&mut app, 1);
    let train = app
        .world()
        .resource::<TrainYard>()
        .peek_kind(TrainKind::Transit)
        .expect("a transit in the yard");
    push(
        &mut app,
        CommandKind::PlaceTrain(PlaceTrain {
            train,
            at_station: a,
        }),
    );
    push(
        &mut app,
        CommandKind::AssignTrainToLine(AssignTrainToLine { train, line }),
    );
    run_fixed(&mut app, 200);

    // ---- The new thing: a stop of the player's own, mid-route. ----
    //
    // Sited the way a player would: clear of the stops either end, on ground
    // that can actually hold houses.
    let habitable = |tile: TileCoord| {
        let mut land = 0;
        for dy in -MEASURE_RADIUS..=MEASURE_RADIUS {
            for dx in -MEASURE_RADIUS..=MEASURE_RADIUS {
                let t = TileCoord {
                    x: tile.x + dx,
                    y: tile.y + dy,
                };
                if terrain.contains(t) && !terrain.is_water(t) {
                    land += 1;
                }
            }
        }
        land
    };
    let site = *path
        .iter()
        .filter(|t| !terrain.is_water(**t))
        .filter(|t| {
            app.world()
                .resource::<StationRegistry>()
                .iter()
                .all(|s| chebyshev(s.tile, **t) >= MIN_STATION_SPACING + 3)
        })
        .max_by_key(|t| habitable(**t))
        .expect("a mid-route site clear of the anchors");

    let before = {
        let world = app.world();
        density_around(
            world.resource::<TownDensity>(),
            world.resource::<StationRegistry>(),
            site,
            StationId(u64::MAX),
        )
    };
    assert_eq!(before, 0.0, "nothing stands at the new site yet");

    // A control the railway never reaches: whatever the world does elsewhere,
    // this stays open country.
    let control = (0..map.height as i32)
        .flat_map(|y| (0..map.width as i32).map(move |x| TileCoord { x, y }))
        .filter(|t| terrain.contains(*t) && !terrain.is_water(*t))
        .find(|t| {
            habitable(*t) > 40
                && app
                    .world()
                    .resource::<StationRegistry>()
                    .iter()
                    .all(|s| chebyshev(s.tile, *t) >= 20)
                && path.iter().all(|p| chebyshev(*p, *t) >= 20)
        })
        .expect("open country far from every anchor");

    push(
        &mut app,
        CommandKind::PlaceStation(PlaceStation::new(
            site,
            GROUND_LAYER,
            StationTier::Station,
            Some("Brackwell".into()),
        )),
    );
    run_fixed(&mut app, 1);

    let new_stop = app
        .world()
        .resource::<StationRegistry>()
        .at(site, GROUND_LAYER)
        .map(|s| s.id)
        .expect("the platform the player paid for");

    // The town says so — a build the world does not acknowledge is a build the
    // player cannot tell landed.
    assert!(
        app.world()
            .resource::<ComplaintFeed>()
            .iter()
            .any(|e| e.display_line().starts_with("Brackwell opened")),
        "opening a station should reach Town Talk"
    );

    // ---- Extend the line over it, the way the Line tool does. ----
    push(&mut app, CommandKind::RemoveLine(RemoveLine { line }));
    push(
        &mut app,
        CommandKind::CreateLine(CreateLine {
            name: Some("Valley Line".into()),
            stops: vec![a, new_stop, b],
        }),
    );
    run_fixed(&mut app, 1);

    let extended = app
        .world()
        .resource::<LineRegistry>()
        .iter()
        .find(|l| l.stops.contains(&new_stop))
        .map(|l| l.id)
        .expect("the line now calls at the new stop");
    // Dropping the old line unassigned the train; put it back on the new one.
    push(
        &mut app,
        CommandKind::AssignTrainToLine(AssignTrainToLine {
            train,
            line: extended,
        }),
    );
    run_fixed(&mut app, 1);

    // ---- Run it. ----
    //
    // The score is sampled as it goes rather than only read at the end: it
    // rises on a call and slides between them, so the claim being pinned is
    // "the railway earned this stop a reputation", not "it happened to hold one
    // on the last tick".
    let mut peak_score = 0u8;
    for _ in 0..(HORIZON_TICKS / 500) {
        top_up(&mut app);
        run_fixed(&mut app, 500);
        peak_score = peak_score.max(
            app.world()
                .resource::<StationService>()
                .score(new_stop)
                .score,
        );
    }

    let world = app.world();
    let service = world.resource::<StationService>();
    let score = service.score(new_stop);
    assert!(
        score.deliveries > 0,
        "a stop on a serviced line should see arrivals ({score:?})"
    );
    assert!(
        peak_score > 0,
        "service should turn into a score at the new stop ({score:?})"
    );

    let after = density_around(
        world.resource::<TownDensity>(),
        world.resource::<StationRegistry>(),
        site,
        new_stop,
    );
    assert!(
        after > before,
        "the town should grow around a served station: {before} -> {after}"
    );

    let control_density = density_around(
        world.resource::<TownDensity>(),
        world.resource::<StationRegistry>(),
        control,
        StationId(u64::MAX),
    );
    assert_eq!(
        control_density, 0.0,
        "unserved country should stay open country"
    );
}
