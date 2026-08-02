//! CI-style MVP playable loop (no GUI).
//!
//! Lives in `rail_town` because the loop needs both `rail_map::generate_map` and
//! `rail_sim::SimPlugin`; `rail_sim` cannot depend on `rail_map` (cycle).

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::prelude::{App, FixedUpdate, Update};
use rail_map::{
    generate_map, MapGrid, DEFAULT_MAP_HEIGHT, DEFAULT_MAP_SEED, DEFAULT_MAP_WIDTH,
};
use rail_sim::ids::{StationId, TileCoord, TrainId};
use rail_sim::{
    AutoFillTrack, BuyTrain, CommandBuffer, CommandKind, ComplaintFeed, IndustryRegistry, JobBoard,
    Money, PlaceTrack, PlaceTrain, SimPlugin, StationRegistry, StationService, TrackNetwork,
    TrackTerrain, Train, TrainKind, TrainLocation, TrainYard, GROUND_LAYER, MAX_BRIDGE_SPAN,
    STARTING_CASH_CENTS, TRANSIT_COST_CENTS, TRANSPORT_COST_CENTS,
};

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

fn tile_placeable(terrain: &TrackTerrain, tile: TileCoord) -> bool {
    if !terrain.contains(tile) {
        return false;
    }
    if !terrain.is_water(tile) {
        return true;
    }
    let h = terrain.water_span_horizontal(tile);
    let v = terrain.water_span_vertical(tile);
    h.min(v) <= MAX_BRIDGE_SPAN
}

/// 4-connected BFS over land / short bridges.
fn find_build_path(terrain: &TrackTerrain, from: TileCoord, to: TileCoord) -> Option<Vec<TileCoord>> {
    if from == to {
        return Some(vec![from]);
    }
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

/// Collapse a polyline into maximal orthogonal/diagonal straight autofill segments.
fn straight_segments(path: &[TileCoord]) -> Vec<(TileCoord, TileCoord)> {
    if path.is_empty() {
        return Vec::new();
    }
    if path.len() == 1 {
        return vec![(path[0], path[0])];
    }

    let mut segs = Vec::new();
    let mut start = path[0];
    let mut prev = path[0];
    let mut dir: Option<(i32, i32)> = None;

    for &tile in &path[1..] {
        let step = ((tile.x - prev.x).signum(), (tile.y - prev.y).signum());
        match dir {
            None => dir = Some(step),
            Some(d) if d == step => {}
            Some(_) => {
                segs.push((start, prev));
                start = prev;
                dir = Some((
                    (tile.x - start.x).signum(),
                    (tile.y - start.y).signum(),
                ));
            }
        }
        prev = tile;
    }
    segs.push((start, prev));
    segs
}

fn enqueue_path_track(buf: &mut CommandBuffer, path: &[TileCoord]) {
    for (from, to) in straight_segments(path) {
        if from == to {
            buf.push(CommandKind::PlaceTrack(PlaceTrack {
                tile: from,
                layer: GROUND_LAYER,
            }));
        } else {
            buf.push(CommandKind::AutoFillTrack(AutoFillTrack {
                from,
                to,
                layer: GROUND_LAYER,
            }));
        }
    }
}

fn run_fixed(app: &mut App, ticks: u32) {
    for _ in 0..ticks {
        app.world_mut().run_schedule(FixedUpdate);
    }
}

fn total_deliveries(service: &StationService) -> u32 {
    service.scores.values().map(|s| s.deliveries).sum()
}

fn any_train_moved(app: &mut App, starts: &HashMap<TrainId, TileCoord>) -> bool {
    let tracks: HashMap<_, _> = app
        .world()
        .resource::<TrackNetwork>()
        .iter()
        .map(|p| (p.id, p.tile))
        .collect();
    let mut q = app.world_mut().query::<(&Train, &TrainLocation)>();
    for (train, loc) in q.iter(app.world()) {
        let Some(start) = starts.get(&train.id) else {
            continue;
        };
        let Some(&tile) = tracks.get(&loc.track) else {
            continue;
        };
        if tile != *start || loc.path_index > 0 {
            return true;
        }
    }
    false
}

#[test]
fn mvp_playable_loop_delivers_and_complains() {
    let map = generate_map(DEFAULT_MAP_WIDTH, DEFAULT_MAP_HEIGHT, DEFAULT_MAP_SEED);
    assert_eq!(map.width, 64);
    assert_eq!(map.height, 64);
    assert_eq!(map.seed, 42);
    assert!(!map.portals().is_empty(), "edge portals should exist");

    let terrain = terrain_from_map(&map);

    let mut app = App::new();
    app.add_plugins(SimPlugin);
    app.insert_resource(terrain.clone());

    // Seed stations / industries via SimPlugin's Update hook.
    app.world_mut().run_schedule(Update);

    let stations: Vec<(StationId, TileCoord, String)> = {
        let reg = app.world().resource::<StationRegistry>();
        assert!(
            reg.len() >= 2,
            "expected at least 2 seeded stations, got {}",
            reg.len()
        );
        reg.iter()
            .map(|s| (s.id, s.tile, s.name.clone()))
            .collect()
    };
    let industries: Vec<TileCoord> = {
        let reg = app.world().resource::<IndustryRegistry>();
        assert_eq!(reg.len(), 2, "expected producer + consumer industries");
        reg.iter().map(|i| i.tile).collect()
    };

    // Prefer Eastgate as hub when present (deterministic spanning tree).
    let hub = stations
        .iter()
        .find(|(_, _, name)| name == "Eastgate")
        .map(|(_, t, _)| *t)
        .unwrap_or(stations[0].1);
    let mut anchors: Vec<TileCoord> = stations.iter().map(|(_, t, _)| *t).collect();
    anchors.extend(industries.iter().copied());

    let mut seen = HashSet::new();
    {
        let mut buf = app.world_mut().resource_mut::<CommandBuffer>();
        for &anchor in &anchors {
            if !seen.insert((anchor.x, anchor.y)) {
                continue;
            }
            let path = find_build_path(&terrain, hub, anchor)
                .unwrap_or_else(|| panic!("no buildable path from hub {hub:?} to {anchor:?}"));
            enqueue_path_track(&mut buf, &path);
        }
    }
    // Apply all place/autofill commands.
    run_fixed(&mut app, 1);

    let network_len = app.world().resource::<TrackNetwork>().len();
    assert!(
        network_len >= 2,
        "expected track between stations, got {network_len} pieces"
    );

    // Ensure passenger path exists between first two stations.
    {
        let network = app.world().resource::<TrackNetwork>();
        let a = rail_sim::track_for_station(network, stations[0].1, GROUND_LAYER)
            .expect("track at station 0");
        let b = rail_sim::track_for_station(network, stations[1].1, GROUND_LAYER)
            .expect("track at station 1");
        assert!(
            rail_sim::find_path(network, a, b).is_some(),
            "stations should be rail-connected"
        );
    }

    // Top up cash if track spend left too little for both trains.
    {
        let mut money = app.world_mut().resource_mut::<Money>();
        let need = TRANSIT_COST_CENTS + TRANSPORT_COST_CENTS + 50_000;
        let cents = money.cents();
        if cents < need {
            money.credit(need - cents);
        }
    }
    let money_after_track = app.world().resource::<Money>().cents();

    // Buy + place transit at station 0, transport at station 1 (or 0 if only one has track).
    {
        let mut buf = app.world_mut().resource_mut::<CommandBuffer>();
        buf.push(CommandKind::BuyTrain(BuyTrain {
            kind: TrainKind::Transit,
        }));
        buf.push(CommandKind::BuyTrain(BuyTrain {
            kind: TrainKind::Transport,
        }));
    }
    run_fixed(&mut app, 1);

    let (transit_id, transport_id) = {
        let yard = app.world().resource::<TrainYard>();
        let transit = yard
            .peek_kind(TrainKind::Transit)
            .expect("transit in yard");
        let transport = yard
            .peek_kind(TrainKind::Transport)
            .expect("transport in yard");
        (transit, transport)
    };

    {
        let mut buf = app.world_mut().resource_mut::<CommandBuffer>();
        buf.push(CommandKind::PlaceTrain(PlaceTrain {
            train: transit_id,
            at_station: stations[0].0,
        }));
        buf.push(CommandKind::PlaceTrain(PlaceTrain {
            train: transport_id,
            at_station: stations[1].0,
        }));
    }
    run_fixed(&mut app, 1);

    let train_count = {
        let mut q = app.world_mut().query::<&Train>();
        q.iter(app.world()).count()
    };
    assert_eq!(train_count, 2, "both trains should be placed on the map");
    assert!(
        app.world().resource::<Money>().cents()
            <= money_after_track - TRANSIT_COST_CENTS - TRANSPORT_COST_CENTS + 1,
        "buying trains should debit treasury"
    );

    let start_tiles: HashMap<TrainId, TileCoord> = {
        let tracks: HashMap<_, _> = app
            .world()
            .resource::<TrackNetwork>()
            .iter()
            .map(|p| (p.id, p.tile))
            .collect();
        let mut q = app.world_mut().query::<(&Train, &TrainLocation)>();
        q.iter(app.world())
            .filter_map(|(t, loc)| tracks.get(&loc.track).copied().map(|tile| (t.id, tile)))
            .collect()
    };

    let money_before_sim = app.world().resource::<Money>().cents();
    let deliveries_before = total_deliveries(app.world().resource::<StationService>());
    let jobs_before = app.world().resource::<JobBoard>().jobs.len();

    // Job spawn wave every 45 ticks; long routes need hundreds of movement ticks.
    run_fixed(&mut app, 6_000);
    {
        let mut q = app.world_mut().query::<(&rail_sim::Train, &rail_sim::TrainCargo, &rail_sim::TrainLocation)>();
        for (tr, cargo, loc) in q.iter(app.world()) {
            eprintln!("train {:?} {:?} parked={} path_idx={}/{} track={:?}", tr.kind, cargo, loc.parked, loc.path_index, loc.path.len(), loc.track);
        }
        eprintln!("jobs={:?}", app.world().resource::<JobBoard>().jobs);
    }

    let deliveries_after = total_deliveries(app.world().resource::<StationService>());
    let money_after = app.world().resource::<Money>().cents();
    let jobs_after = app.world().resource::<JobBoard>().jobs.len();
    let moved = any_train_moved(&mut app, &start_tiles);
    let _ = (jobs_before, jobs_after, STARTING_CASH_CENTS);

    let service_score_up = app
        .world()
        .resource::<StationService>()
        .scores
        .values()
        .any(|s| s.score > 0);

    // Core loop: at least one passenger delivery (StationService) and/or a train
    // that left its spawn tile. Net money may fall from opex even with payouts.
    assert!(
        deliveries_after > deliveries_before || moved || service_score_up,
        "expected deliveries, service score, or train movement; \
         deliveries {deliveries_before}->{deliveries_after}, money {money_before_sim}->{money_after}, moved={moved}"
    );
    assert!(
        deliveries_after > deliveries_before,
        "expected at least one passenger delivery via StationService"
    );

    // Peep wait path should populate the complaint feed during a long sim.
    let feed = app.world().resource::<ComplaintFeed>();
    assert!(
        !feed.is_empty(),
        "expected at least one peep wait complaint after long sim"
    );
    let line = feed.latest_line().expect("complaint line");
    assert!(
        line.contains("waited") && line.contains("min"),
        "unexpected complaint line: {line}"
    );
}
