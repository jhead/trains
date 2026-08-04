//! Round-trip tests: build a world, save it, load it, and prove nothing moved.
//!
//! The highest-value test in the crate is [`save_load_round_trip_is_exact`] —
//! if a snapshot survives encode → decode → restore → re-capture unchanged,
//! then every field that matters is in the blob and comes back where it was.

use bevy_ecs::prelude::*;
use bevy_ecs::system::RunSystemOnce;

use crate::clock::SimClock;
use crate::commands::TrainKind;
use crate::demand::{DemandOpportunity, DemandOpportunityKind, DemandSpawner};
use crate::economy::{
    refresh_alerts, spawn_demand_jobs, AlertBoard, JobBoard, MoneyCategory, MoneyLedger,
};
use crate::ids::{StationId, TileCoord, TrainId};
use crate::lines::LineRegistry;
use crate::money::Money;
use crate::peeps::{
    BodyType, ComplaintEntry, ComplaintFeed, DistrictFlow, HouseholdRegistry, Journey,
    JourneyMemory, JourneyOutcome, JourneyRecord, JourneyStage, Mood, PathWear, Peep, PeepBudget,
    PeepDetail, PeepId, PeepPosition, PeepSpawnState, Routine, TalkKind, WaitingAtStation,
    WEAR_MAX, WEAR_PER_FOOTFALL,
};
use crate::stations::{
    GoodKind, IndustryRegistry, StationRegistry, StationService, StationServiceScore, StationTier,
    HALT_COST_CENTS, INTERCHANGE_COST_CENTS,
};
use crate::town::TownDensity;
use crate::track::{try_place_track, TrackNetwork, TrackTerrain, GROUND_LAYER};
use crate::trains::{TileOccupancy, Train, TrainCargo, TrainLocation, TrainOnLine, TrainYard};
use crate::WorldAnchorsSeeded;

use super::codec::{decode_save, encode_save, SaveMeta};
use super::slots::{delete_slot, list_slots, load_from_slot, save_to_slot, SaveSlot};
use super::snapshot::{MapDescriptor, WorldSnapshot, SCHEMA_VERSION};
use super::storage::use_test_root;

const MAP_W: u32 = 12;
const MAP_H: u32 = 8;
const MAP_SEED: u64 = 4_242;
/// A packed `rail_map::MapGenOptions` — opaque here, because `rail_sim` cannot
/// see `rail_map`. This pattern is Standard / Rugged / Riverlands / Scattered,
/// and is deliberately not the stock setup: a save that only ever carried the
/// default would not prove the knobs travel.
const MAP_KNOBS: u8 = 0b0110_1001;
/// Odd, so the job spawner picks two different stations from a two-stop world.
const SIM_TICK: u64 = 4_311;

/// Flat land with a two-tile water channel, so bridges and grades are exercised.
fn terrain() -> TrackTerrain {
    TrackTerrain::new(
        MAP_W,
        MAP_H,
        (0..MAP_H).flat_map(|y| {
            (0..MAP_W).map(move |x| {
                let water = x == 6 && y != 3;
                let height = if x > 8 { 2i8 } else { 0i8 };
                (water, height)
            })
        }),
    )
}

/// A world with something of everything: track, anchors, a line, trains with
/// cargo and assignments, buildings, named residents mid-complaint, money that
/// has moved, open jobs, and a pending demand opportunity.
fn lived_in_world() -> World {
    let mut world = World::new();
    let terrain = terrain();

    // --- map -------------------------------------------------------------
    world.insert_resource(MapDescriptor::new(MAP_SEED, MAP_W, MAP_H).with_knobs(MAP_KNOBS));

    // --- track -----------------------------------------------------------
    let mut network = TrackNetwork::new();
    let mut money = Money::new(750_000);
    let mut ledger = MoneyLedger::default();
    for x in 1..=5 {
        try_place_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            TileCoord { x, y: 3 },
            GROUND_LAYER,
        )
        .expect("track on land");
    }
    let first_track = network
        .id_at(TileCoord { x: 1, y: 3 }, GROUND_LAYER)
        .expect("placed");
    let last_track = network
        .id_at(TileCoord { x: 5, y: 3 }, GROUND_LAYER)
        .expect("placed");

    // --- stations / industries / service ----------------------------------
    let mut stations = StationRegistry::new();
    // A player-built interchange and a cheap halt: tier and spend must both
    // come back, or a reloaded town silently downgrades its own platforms.
    let eastgate = stations.insert_tier(
        "Eastgate",
        TileCoord { x: 1, y: 3 },
        GROUND_LAYER,
        StationTier::Interchange,
        INTERCHANGE_COST_CENTS,
    );
    let westbrook = stations.insert_tier(
        "Westbrook",
        TileCoord { x: 5, y: 3 },
        GROUND_LAYER,
        StationTier::Halt,
        HALT_COST_CENTS,
    );
    // A stop the player demolished, so ids are not contiguous.
    let doomed = stations.insert("Gravesend", TileCoord { x: 3, y: 3 }, GROUND_LAYER);
    stations.remove(doomed);

    // Both works stand a row off the line, so a railhead reaches each of them.
    // That is load-bearing: `spawn_demand_jobs` only posts work a train could
    // take, so a quarry out in the fields produces no goods job and the
    // snapshot below would carry no `Goods` variant to round-trip.
    let mut industries = IndustryRegistry::new();
    let quarry = industries.insert(
        "Quarry Ridge",
        TileCoord { x: 2, y: 4 },
        Some(GoodKind::Ore),
        None,
    );
    let foundry = industries.insert(
        "Harbor Foundry",
        TileCoord { x: 4, y: 2 },
        None,
        Some(GoodKind::Ore),
    );

    let mut service = StationService::default();
    service.tick = SIM_TICK;

    // --- lines ------------------------------------------------------------
    let mut lines = LineRegistry::new();
    let line = lines
        .create("Eastgate - Westbrook".into(), vec![eastgate, westbrook])
        .expect("line");
    lines.assign_train(line, TrainId(1));

    // --- trains -----------------------------------------------------------
    let mut yard = TrainYard::default();
    let bought = yard.buy(TrainKind::Transit);
    let _spare = yard.buy(TrainKind::Transport); // stays unplaced
    yard.take(bought).expect("bought train leaves the yard");

    let mut running = TrainLocation::at_track(first_track);
    running.set_path(vec![first_track, last_track]);
    running.progress = 2;
    running.dwell_remaining = 1;
    world.spawn((
        Train {
            id: bought,
            kind: TrainKind::Transit,
        },
        running,
        TrainCargo::Passengers {
            from: eastgate,
            to: westbrook,
        },
        TrainOnLine {
            line,
            next_stop: 1,
            forward: true,
        },
    ));

    let mut parked = TrainLocation::at_track(last_track);
    parked.parked = true;
    world.spawn((
        Train {
            id: TrainId(9),
            kind: TrainKind::Transport,
        },
        parked,
        TrainCargo::Goods {
            kind: GoodKind::Ore,
            from: quarry,
            to: foundry,
        },
    ));

    let mut occupancy = TileOccupancy::default();
    occupancy.by_track.insert(first_track, bought);
    occupancy.by_track.insert(last_track, TrainId(9));
    occupancy.blocked_by.insert(TrainId(9), bought);

    // --- town -------------------------------------------------------------
    let mut density = TownDensity::default();
    density.set(TileCoord { x: 1, y: 3 }, 0.75);
    density.set(TileCoord { x: 2, y: 4 }, 0.5);
    density.set(TileCoord { x: 5, y: 2 }, 0.125);

    // --- peeps: names, families, journeys, memories, and the talk ---------
    let mut households = HouseholdRegistry::new();
    let alderton = households.insert(TileCoord { x: 1, y: 4 }, eastgate, 200);
    let brambles = households.insert(TileCoord { x: 5, y: 4 }, westbrook, 900);

    // Mara: mid-commute, on a platform, out of patience, with a bad streak.
    let mara_routine = Routine::from_seed(
        11,
        TileCoord { x: 1, y: 4 },
        eastgate,
        TileCoord { x: 5, y: 4 },
        westbrook,
    );
    let mut mara_journey = Journey::new(&mara_routine);
    mara_journey.set_stage(JourneyStage::WaitingOnPlatform);
    mara_journey.stage_ticks = 61;
    mara_journey.leg_secs = 800;
    mara_journey.leg_wait_secs = 742;
    mara_journey.last_depart_day = Some(3);
    let mut mara_memory = JourneyMemory::default();
    mara_memory.record(JourneyRecord {
        from: eastgate,
        to: westbrook,
        wait_secs: 660,
        total_secs: 900,
        outcome: JourneyOutcome::Slow,
        ended_tick: 4_000,
    });
    mara_memory.record(JourneyRecord {
        from: westbrook,
        to: eastgate,
        wait_secs: 900,
        total_secs: 1_500,
        outcome: JourneyOutcome::GaveUp,
        ended_tick: 4_180,
    });
    world.spawn((
        Peep {
            id: PeepId(1),
            name: "Mara Alderton".into(),
            home: TileCoord { x: 1, y: 4 },
            mood: Mood::Frustrated,
            household: alderton,
            body: BodyType::Tall,
            portrait: 2,
            moved_in_tick: 200,
        },
        WaitingAtStation {
            station: eastgate,
            wait_secs: 742,
            ticks_since_complaint: 12,
            ticks_since_praise: 400,
        },
        mara_routine,
        mara_journey,
        PeepPosition::at_tile(TileCoord { x: 1, y: 3 }, 11),
        mara_memory,
        PeepDetail::Full,
    ));
    households.add_member(alderton, PeepId(1));

    // Theo: riding a train, content, remembered as a good commute.
    let theo_routine = Routine::from_seed(
        22,
        TileCoord { x: 5, y: 4 },
        westbrook,
        TileCoord { x: 1, y: 4 },
        eastgate,
    );
    let mut theo_journey = Journey::new(&theo_routine);
    theo_journey.set_stage(JourneyStage::Riding);
    theo_journey.riding = Some(TrainId(1));
    theo_journey.leg_secs = 120;
    let mut theo_memory = JourneyMemory::default();
    theo_memory.record(JourneyRecord {
        from: westbrook,
        to: eastgate,
        wait_secs: 40,
        total_secs: 300,
        outcome: JourneyOutcome::Good,
        ended_tick: 4_250,
    });
    world.spawn((
        Peep {
            id: PeepId(2),
            name: "Theo Bramble".into(),
            home: TileCoord { x: 5, y: 4 },
            mood: Mood::Content,
            household: brambles,
            body: BodyType::Stocky,
            portrait: 1,
            moved_in_tick: 900,
        },
        WaitingAtStation {
            station: westbrook,
            wait_secs: 0,
            ticks_since_complaint: 900,
            ticks_since_praise: 5,
        },
        theo_routine,
        theo_journey,
        PeepPosition::at_tile(TileCoord { x: 3, y: 3 }, 22),
        theo_memory,
        PeepDetail::Full,
    ));
    households.add_member(brambles, PeepId(2));

    // Nia: abstracted into district flow — a resident with no journey state,
    // which must still come back as a named person in the same household.
    world.spawn((
        Peep {
            id: PeepId(3),
            name: "Nia Bramble".into(),
            home: TileCoord { x: 5, y: 4 },
            mood: Mood::Uneasy,
            household: brambles,
            body: BodyType::Round,
            portrait: 3,
            moved_in_tick: 900,
        },
        PeepDetail::Abstract,
    ));
    households.add_member(brambles, PeepId(3));

    let mut spawn_state = PeepSpawnState::default();
    spawn_state.next_id = 3;
    spawn_state.spawned_for.insert(eastgate);
    spawn_state.spawned_for.insert(westbrook);

    let mut flow = DistrictFlow::default();
    {
        let district = flow.entry(westbrook);
        district.residents = 14;
        district.waiting = 6;
        district.completed = 21;
        district.gave_up = 3;
        district.pressure_secs = 1_800;
    }
    flow.request_trip(westbrook, eastgate);

    // A level-of-detail budget the player's machine settled on, both tunables
    // away from their defaults so a mirror that dropped one would show.
    let budget = PeepBudget {
        max_detailed: 12,
        rebalance_every: 7,
        ..PeepBudget::default()
    };

    let mut feed = ComplaintFeed::default();
    feed.push(ComplaintEntry {
        kind: TalkKind::Complaint,
        peep_name: "Mara".into(),
        station_name: "Eastgate".into(),
        wait_minutes: 12,
        sim_tick: 4_100,
        peep_id: Some(PeepId(1)),
        station_id: Some(eastgate),
        tile: Some(TileCoord { x: 1, y: 3 }),
        count: 1,
    });
    feed.push(ComplaintEntry {
        kind: TalkKind::Praise,
        peep_name: "Theo".into(),
        station_name: "Westbrook".into(),
        wait_minutes: 0,
        sim_tick: 4_250,
        peep_id: Some(PeepId(2)),
        station_id: Some(westbrook),
        tile: Some(TileCoord { x: 5, y: 3 }),
        count: 1,
    });

    // --- economy ----------------------------------------------------------
    // One tick short of a spawn wave, so a single system run below fills the
    // board with real jobs rather than hand-built ones.
    let mut jobs = JobBoard::default();
    jobs.spawn_cooldown = 44;

    ledger.credit(&mut money, MoneyCategory::Fares, 12_500);
    ledger
        .try_debit(&mut money, MoneyCategory::TrainOpex, 900)
        .expect("affordable");
    ledger.on_sim_secs(90);

    // --- demand -----------------------------------------------------------
    let mut demand = DemandSpawner::default();
    demand.ticks_until_next = 33;
    demand.spawned_count = 2;
    demand.next_settlement = 1;
    demand.next_industry = 2;
    demand.next_is_settlement = false;
    demand.open.push(DemandOpportunity {
        kind: DemandOpportunityKind::Settlement(StationId(7)),
        name: "Ridgeline".into(),
        tile: TileCoord { x: 10, y: 6 },
    });

    // --- install ----------------------------------------------------------
    world.insert_resource(terrain);
    world.insert_resource(network);
    world.insert_resource(stations);
    world.insert_resource(industries);
    world.insert_resource(service);
    world.insert_resource(lines);
    world.insert_resource(yard);
    world.insert_resource(occupancy);
    world.insert_resource(density);
    world.insert_resource(spawn_state);
    world.insert_resource(households);
    world.insert_resource(flow);
    world.insert_resource(budget);
    world.insert_resource(feed);
    world.insert_resource(jobs);
    world.insert_resource(ledger);
    world.insert_resource(AlertBoard::default());
    world.insert_resource(demand);
    world.insert_resource(SimClock {
        paused: true,
        speed_multiplier: 3,
        ..Default::default()
    });
    world.insert_resource(money);
    world.insert_resource(WorldAnchorsSeeded(true));

    // Let the real systems fill the job board and the alert board, so the test
    // saves the state the game actually produces rather than a hand-made one.
    world
        .run_system_once(spawn_demand_jobs)
        .expect("job spawn ran");
    world.run_system_once(refresh_alerts).expect("alerts ran");

    // Fixed service scores last: job spawning nudges them, and the assertions
    // below want values that do not depend on hash iteration order.
    {
        let mut service = world.resource_mut::<StationService>();
        service.tick = SIM_TICK;
        // `peep_waiting` is the named residents standing on the platform, and it
        // is deliberately different from `waiting_passengers` at both stops: the
        // two have different writers, and a mirror that dropped one of them
        // would still look right if they matched.
        service.scores.insert(
            eastgate,
            StationServiceScore {
                deliveries: 27,
                last_arrival_tick: 4_180,
                waiting_passengers: 3,
                peep_waiting: 6,
                score: 74,
                tier: StationTier::Interchange,
            },
        );
        service.scores.insert(
            westbrook,
            StationServiceScore {
                deliveries: 4,
                last_arrival_tick: 3_002,
                waiting_passengers: 9,
                peep_waiting: 2,
                score: 21,
                tier: StationTier::Halt,
            },
        );
        // The demolished stop's score is still on the board — demolition does
        // not sweep it. Its id must therefore never be handed out again.
        service.scores.insert(
            StationId(3),
            StationServiceScore {
                deliveries: 1,
                ..Default::default()
            },
        );
    }
    world.resource_mut::<JobBoard>().spawn_cooldown = 17;

    // Ground the town has worn: a short lane at three different depths, so a
    // round trip has to carry wear values and not merely "there was a path".
    let mut paths = PathWear::new(MAP_W, MAP_H);
    for (tile, crossings) in [
        (TileCoord { x: 2, y: 2 }, 1u32),  // below every threshold
        (TileCoord { x: 3, y: 2 }, 5),     // Faint
        (TileCoord { x: 4, y: 2 }, 11),    // Worn
        (TileCoord { x: 5, y: 2 }, 40),    // saturated
    ] {
        for _ in 0..crossings {
            paths.add_footfall(tile);
        }
    }
    world.insert_resource(paths);

    world
}

// ---------------------------------------------------------------------------
// The round trip
// ---------------------------------------------------------------------------

#[test]
fn save_load_round_trip_is_exact() {
    let world = lived_in_world();
    let saved = WorldSnapshot::capture(&world);

    let meta = SaveMeta::from_snapshot(&saved, "Round trip");
    let bytes = encode_save(&meta, &saved).expect("encode");
    let (_, loaded) = decode_save(&bytes).expect("decode");

    assert_eq!(loaded, saved, "the blob must survive encode -> decode");

    // And restoring it into a fresh world must reproduce the same snapshot.
    let mut fresh = World::new();
    let report = loaded.restore(&mut fresh);
    assert!(report.is_clean(), "restore warnings: {:?}", report.warnings);

    let recaptured = WorldSnapshot::capture(&fresh);
    assert_eq!(
        recaptured, saved,
        "a restored world must snapshot back to the same bytes"
    );
}

#[test]
fn restoring_twice_is_stable() {
    let world = lived_in_world();
    let saved = WorldSnapshot::capture(&world);

    let mut target = World::new();
    saved.restore(&mut target);
    saved.restore(&mut target); // loading over a loaded game must not duplicate
    let recaptured = WorldSnapshot::capture(&target);

    assert_eq!(recaptured.trains.placed.len(), saved.trains.placed.len());
    assert_eq!(recaptured.peeps.peeps.len(), saved.peeps.peeps.len());
    assert_eq!(recaptured, saved);
}

// ---------------------------------------------------------------------------
// The parts the design calls out by name
// ---------------------------------------------------------------------------

#[test]
fn peep_names_moods_and_histories_survive() {
    let world = lived_in_world();
    let snapshot = WorldSnapshot::capture(&world);

    let bytes = encode_save(&SaveMeta::from_snapshot(&snapshot, "Peeps"), &snapshot)
        .expect("encode");
    let (_, loaded) = decode_save(&bytes).expect("decode");

    let names: Vec<&str> = loaded
        .peeps
        .peeps
        .iter()
        .map(|p| p.peep.name.as_str())
        .collect();
    assert_eq!(names, vec!["Mara Alderton", "Theo Bramble", "Nia Bramble"]);

    // Mara: her mood, her wait, her trip, and what she remembers of the last two.
    let mara = &loaded.peeps.peeps[0];
    assert_eq!(mara.peep.mood, Mood::Frustrated);
    assert_eq!(mara.peep.body, BodyType::Tall);
    assert_eq!(mara.peep.moved_in_tick, 200);
    let waiting = mara.waiting.as_ref().expect("Mara was waiting");
    assert_eq!(waiting.wait_secs, 742);
    assert_eq!(waiting.ticks_since_complaint, 12);
    let journey = mara.journey.as_ref().expect("Mara was mid-journey");
    assert_eq!(journey.stage, JourneyStage::WaitingOnPlatform);
    assert_eq!(journey.leg_wait_secs, 742);
    assert_eq!(journey.last_depart_day, Some(3));
    let memory = mara.memory.as_ref().expect("Mara remembers");
    assert_eq!(memory.lifetime_journeys, 2);
    assert_eq!(memory.lifetime_gave_up, 1);
    assert_eq!(memory.recent.len(), 2);
    assert_eq!(memory.recent[0].outcome, JourneyOutcome::GaveUp);
    assert!(mara.routine.is_some(), "her habitual day survives too");

    // Theo is aboard a train — the ride itself is part of the save.
    let theo = &loaded.peeps.peeps[1];
    assert_eq!(
        theo.journey.as_ref().expect("riding").riding,
        Some(TrainId(1))
    );

    // Nia is abstracted into district flow; she must still come back a person.
    let nia = loaded
        .peeps
        .peeps
        .iter()
        .find(|p| p.peep.name == "Nia Bramble")
        .expect("Nia");
    assert!(nia.journey.is_none());
    assert_eq!(nia.detail, Some(PeepDetail::Abstract));
    assert_eq!(nia.peep.home, TileCoord { x: 5, y: 4 });

    // Families keep their name, their home, and who lives there.
    assert_eq!(loaded.peeps.households.len(), 2);
    let brambles = loaded
        .peeps
        .households
        .iter()
        .find(|h| h.members.contains(&PeepId(3)))
        .expect("the household Nia lives in");
    assert_eq!(brambles.members, vec![PeepId(2), PeepId(3)]);
    assert_eq!(brambles.home_station, StationId(2));
    assert_eq!(brambles.moved_in_tick, 900);

    // The town's memory of what happened, newest first.
    assert_eq!(loaded.peeps.town_talk.len(), 2);
    assert_eq!(loaded.peeps.town_talk[0].peep_name, "Theo");
    assert_eq!(loaded.peeps.town_talk[0].kind, super::TalkKindSnapshot::Praise);
    assert_eq!(loaded.peeps.town_talk[1].peep_name, "Mara");
    assert_eq!(loaded.peeps.town_talk[1].wait_minutes, 12);
    assert_eq!(loaded.peeps.next_id, 3);

    // …and it is still in the feed, in order, after a restore.
    let mut fresh = World::new();
    loaded.restore(&mut fresh);
    let feed = fresh.resource::<ComplaintFeed>();
    let lines: Vec<String> = feed.iter().map(|e| e.display_line()).collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("Theo"), "{lines:?}");
    assert!(lines[1].contains("Mara"), "{lines:?}");

    // The households resource is rebuilt, ids intact, so every peep's
    // `household` field still points at the family they live with.
    let households = fresh.resource::<HouseholdRegistry>();
    assert_eq!(households.len(), 2);
    let mut peep_query = fresh.try_query::<&Peep>().expect("peeps registered");
    for peep in peep_query.iter(&fresh) {
        assert!(
            households.get(peep.household).is_some(),
            "{} lost their household",
            peep.name
        );
    }
}

#[test]
fn trains_keep_position_cargo_and_assignment() {
    let world = lived_in_world();
    let snapshot = WorldSnapshot::capture(&world);

    let running = &snapshot.trains.placed[0];
    assert_eq!(running.train.kind, TrainKind::Transit);
    assert_eq!(running.location.progress, 2);
    assert_eq!(running.location.dwell_remaining, 1);
    assert_eq!(running.location.path.len(), 2);
    assert!(matches!(running.cargo, TrainCargo::Passengers { .. }));
    let on_line = running.on_line.expect("assigned to a line");
    assert_eq!(on_line.next_stop, 1);

    let parked = &snapshot.trains.placed[1];
    assert!(parked.location.parked);
    assert!(matches!(parked.cargo, TrainCargo::Goods { .. }));
    assert!(parked.on_line.is_none());

    // Unplaced stock is stock too.
    assert_eq!(snapshot.trains.yard.unplaced().len(), 1);

    let mut fresh = World::new();
    snapshot.restore(&mut fresh);
    let mut query = fresh
        .try_query::<(&Train, &TrainLocation, &TrainCargo)>()
        .expect("trains registered");
    assert_eq!(query.iter(&fresh).count(), 2);
}

/// Seed sharing is only a promise if the *settings* travel with the seed.
///
/// The generator's knobs steer it for real, so a save that recorded only that a
/// world had been generated recorded nothing anyone could regenerate from. They
/// go through the whole pipe here — capture, encode, decode, restore — because
/// the app on the other end unpacks them and makes the map again.
#[test]
fn the_generator_knobs_travel_with_the_seed() {
    let world = lived_in_world();
    let snapshot = WorldSnapshot::capture(&world);
    assert_eq!(snapshot.map.gen.knobs, Some(MAP_KNOBS));

    let bytes = encode_save(&SaveMeta::from_snapshot(&snapshot, "Knobs"), &snapshot)
        .expect("encode");
    let (_, loaded) = decode_save(&bytes).expect("decode");
    assert_eq!(loaded.map.gen.knobs, Some(MAP_KNOBS));

    let mut fresh = World::new();
    loaded.restore(&mut fresh);
    let descriptor = *fresh.resource::<MapDescriptor>();
    assert_eq!(descriptor.seed, MAP_SEED);
    assert_eq!(descriptor.width, MAP_W);
    assert_eq!(descriptor.height, MAP_H);
    assert_eq!(
        descriptor.gen.knobs,
        Some(MAP_KNOBS),
        "a loaded world must know how it was generated, not merely that it was"
    );
}

/// A world nobody described stays undescribed. Inventing a setup for it would
/// have a loader regenerate a map the player never played.
#[test]
fn a_world_that_never_declared_its_knobs_does_not_acquire_any() {
    let mut world = World::new();
    world.insert_resource(MapDescriptor::new(7, MAP_W, MAP_H));
    let snapshot = WorldSnapshot::capture(&world);
    assert_eq!(snapshot.map.gen.knobs, None);
    assert_eq!(snapshot.map.seed, 7);
}

#[test]
fn map_terrain_and_seed_come_back() {
    let world = lived_in_world();
    let snapshot = WorldSnapshot::capture(&world);

    assert_eq!(snapshot.map.seed, MAP_SEED);
    assert_eq!(snapshot.map.width, MAP_W);
    assert_eq!(snapshot.map.height, MAP_H);
    assert_eq!(snapshot.map.gen.generator_version, super::GENERATOR_VERSION);

    let chunk = snapshot.map.terrain.as_ref().expect("terrain captured");
    assert_eq!(chunk.water.len(), (MAP_W * MAP_H) as usize);

    let rebuilt = chunk.to_terrain().expect("terrain rebuilds");
    let original = terrain();
    for y in 0..MAP_H as i32 {
        for x in 0..MAP_W as i32 {
            let c = TileCoord { x, y };
            assert_eq!(rebuilt.is_water(c), original.is_water(c), "water at {c:?}");
            assert_eq!(rebuilt.height_at(c), original.height_at(c), "height at {c:?}");
        }
    }
}

#[test]
fn money_clock_and_economy_come_back() {
    let world = lived_in_world();
    let snapshot = WorldSnapshot::capture(&world);

    let expected_cents = world.resource::<Money>().cents();
    assert_eq!(snapshot.money_cents, expected_cents);
    assert!(snapshot.clock.paused);
    assert_eq!(snapshot.clock.speed_multiplier, 3);
    assert_eq!(snapshot.economy.jobs.jobs.len(), 2);
    assert_eq!(snapshot.economy.jobs.spawn_cooldown, 17);
    assert_eq!(snapshot.stations.service_tick, SIM_TICK);
    assert_eq!(snapshot.demand.open.len(), 1);
    assert!(snapshot.anchors_seeded);

    let mut fresh = World::new();
    snapshot.restore(&mut fresh);
    assert_eq!(fresh.resource::<Money>().cents(), expected_cents);
    assert!(fresh.resource::<SimClock>().paused);
    assert_eq!(fresh.resource::<SimClock>().speed_multiplier, 3);
    assert!(fresh.resource::<WorldAnchorsSeeded>().0);
    assert_eq!(
        fresh.resource::<MoneyLedger>().total(MoneyCategory::Fares),
        12_500
    );
    assert_eq!(fresh.resource::<StationService>().tick, SIM_TICK);
    assert_eq!(fresh.resource::<StationService>().score(StationId(1)).score, 74);
}

/// Every field of the three hand-written mirrors, through the whole pipe.
///
/// These are the sections that cannot embed their sim type directly, so each is
/// a list someone has to remember to extend. `peep_waiting` is the field that
/// proves the point: it was added to `StationServiceScore`, the mirror restored
/// with `..Default::default()`, and every save since had quietly forgotten how
/// many named residents were standing on each platform. Both halves now
/// destructure the source type, so the next one is a build error instead.
#[test]
fn the_hand_written_mirrors_carry_every_field_they_should() {
    let world = lived_in_world();
    let snapshot = WorldSnapshot::capture(&world);

    // Survive the bytes, not just the clone.
    let bytes = encode_save(&SaveMeta::from_snapshot(&snapshot, "Mirrors"), &snapshot)
        .expect("encode");
    let (_, loaded) = decode_save(&bytes).expect("decode");

    let eastgate = loaded
        .stations
        .service
        .iter()
        .find(|s| s.station == StationId(1))
        .expect("Eastgate's score");
    assert_eq!(eastgate.deliveries, 27);
    assert_eq!(eastgate.last_arrival_tick, 4_180);
    assert_eq!(eastgate.waiting_passengers, 3);
    assert_eq!(eastgate.peep_waiting, 6);
    assert_eq!(eastgate.score, 74);
    assert_eq!(eastgate.tier, StationTier::Interchange);

    assert!(loaded.clock.paused);
    assert_eq!(loaded.clock.speed_multiplier, 3);
    assert_eq!(loaded.peeps.budget.max_detailed, 12);
    assert_eq!(loaded.peeps.budget.rebalance_every, 7);

    let mut fresh = World::new();
    loaded.restore(&mut fresh);

    let restored = fresh.resource::<StationService>().score(StationId(1));
    assert_eq!(
        restored.peep_waiting, 6,
        "the platform's named residents must come back, not default to nobody"
    );
    assert_eq!(
        restored.total_waiting(),
        9,
        "the blended queue is what crowding is charged from"
    );
    assert_eq!(restored.waiting_passengers, 3);
    assert_eq!(restored.deliveries, 27);
    assert_eq!(restored.last_arrival_tick, 4_180);
    assert_eq!(restored.score, 74);
    assert_eq!(restored.tier, StationTier::Interchange);

    let clock = *fresh.resource::<SimClock>();
    assert!(clock.paused);
    assert_eq!(clock.speed_multiplier, 3);

    // The budget's tunables are restored; its readouts and its rebalance
    // countdown deliberately are not, and start fresh.
    let budget = *fresh.resource::<PeepBudget>();
    assert_eq!(budget.max_detailed, 12);
    assert_eq!(budget.rebalance_every, 7);
    assert_eq!(budget.detailed, 0);
    assert_eq!(budget.abstracted, 0);
}

#[test]
fn station_and_industry_ids_are_preserved() {
    let world = lived_in_world();
    let snapshot = WorldSnapshot::capture(&world);

    let mut fresh = World::new();
    let report = snapshot.restore(&mut fresh);
    assert!(report.is_clean(), "{:?}", report.warnings);

    let stations = fresh.resource::<StationRegistry>();
    assert_eq!(stations.len(), 2);
    let eastgate = stations.get(StationId(1)).expect("station 1");
    assert_eq!(eastgate.name, "Eastgate");
    assert_eq!(eastgate.tile, TileCoord { x: 1, y: 3 });
    // Platform grade and the money sunk into it are part of the world.
    assert_eq!(eastgate.tier, StationTier::Interchange);
    assert_eq!(eastgate.paid_cents, INTERCHANGE_COST_CENTS);
    let westbrook = stations.get(StationId(2)).expect("station 2");
    assert_eq!(westbrook.tier, StationTier::Halt);
    assert_eq!(westbrook.paid_cents, HALT_COST_CENTS);
    // Ids must still resolve through the tile index, not just the id map.
    assert_eq!(
        stations.id_at(TileCoord { x: 5, y: 3 }, GROUND_LAYER),
        Some(StationId(2))
    );
    // The demolished stop left a hole at id 3; nothing may reoccupy it.
    assert!(stations.get(StationId(3)).is_none());
    assert_eq!(
        fresh.resource::<StationService>().tier(StationId(1)),
        StationTier::Interchange,
        "the cached service tier follows the station"
    );

    // Building after a load must not hand out a demolished stop's id — a stale
    // service score still names it, and it would attach to the new platform.
    let fresh_id = fresh.resource_mut::<StationRegistry>().insert(
        "Newbuilt",
        TileCoord { x: 2, y: 5 },
        GROUND_LAYER,
    );
    assert!(
        fresh_id.0 > 3,
        "id {} reuses a demolished station",
        fresh_id.0
    );

    let industries = fresh.resource::<IndustryRegistry>();
    assert_eq!(industries.len(), 2);
    assert!(industries.producer_of(GoodKind::Ore).is_some());
    assert!(industries.consumer_of(GoodKind::Ore).is_some());
}

#[test]
fn the_track_graph_still_links_after_a_load() {
    let world = lived_in_world();
    let snapshot = WorldSnapshot::capture(&world);

    let mut fresh = World::new();
    snapshot.restore(&mut fresh);

    let network = fresh.resource::<TrackNetwork>();
    assert_eq!(network.len(), 5);
    let middle = network
        .id_at(TileCoord { x: 3, y: 3 }, GROUND_LAYER)
        .expect("track by tile");
    assert_eq!(network.neighbor_ids(middle).len(), 2, "graph edges survive");
    let end = network
        .id_at(TileCoord { x: 1, y: 3 }, GROUND_LAYER)
        .expect("track by tile");
    assert_eq!(network.neighbor_ids(end).len(), 1);
}

#[test]
fn lines_keep_their_stops_colour_and_trains() {
    let world = lived_in_world();
    let snapshot = WorldSnapshot::capture(&world);

    let mut fresh = World::new();
    snapshot.restore(&mut fresh);

    let lines = fresh.resource::<LineRegistry>();
    assert_eq!(lines.len(), 1);
    let line = lines.iter().next().expect("one line");
    assert_eq!(line.name, "Eastgate - Westbrook");
    assert_eq!(line.stops, vec![StationId(1), StationId(2)]);
    assert_eq!(line.trains, vec![TrainId(1)]);
    assert!(lines.line_for_train(TrainId(1)).is_some());
}

#[test]
fn town_density_survives_including_fractions() {
    let world = lived_in_world();
    let snapshot = WorldSnapshot::capture(&world);

    let mut fresh = World::new();
    snapshot.restore(&mut fresh);

    let density = fresh.resource::<TownDensity>();
    assert_eq!(density.len(), 3);
    assert_eq!(density.get(TileCoord { x: 1, y: 3 }), 0.75);
    assert_eq!(density.get(TileCoord { x: 5, y: 2 }), 0.125);
    assert_eq!(density.get(TileCoord { x: 11, y: 7 }), 0.0);
}

// ---------------------------------------------------------------------------
// Desire paths (brief 16 §7)
// ---------------------------------------------------------------------------

#[test]
fn worn_ground_survives_a_save() {
    let world = lived_in_world();
    let snapshot = WorldSnapshot::capture(&world);

    // Only worn tiles travel, ascending by index, with no zero entries.
    assert_eq!(snapshot.paths.width, MAP_W);
    assert_eq!(snapshot.paths.height, MAP_H);
    assert_eq!(snapshot.paths.wear.len(), 4);
    assert!(
        snapshot.paths.wear.windows(2).all(|w| w[0].0 < w[1].0),
        "the wear blob must be sorted: {:?}",
        snapshot.paths.wear
    );
    assert!(snapshot.paths.wear.iter().all(|(_, w)| *w > 0));

    let bytes = encode_save(&SaveMeta::from_snapshot(&snapshot, "Paths"), &snapshot)
        .expect("encode");
    let (_, loaded) = decode_save(&bytes).expect("decode");

    let mut fresh = World::new();
    loaded.restore(&mut fresh);
    let paths = fresh.resource::<PathWear>();

    // Values, not merely presence — and the levels they imply.
    assert_eq!(paths.wear_at(TileCoord { x: 2, y: 2 }), WEAR_PER_FOOTFALL);
    assert_eq!(paths.level_at(TileCoord { x: 2, y: 2 }), 0);
    assert_eq!(paths.wear_at(TileCoord { x: 3, y: 2 }), 5 * WEAR_PER_FOOTFALL);
    assert_eq!(paths.level_at(TileCoord { x: 3, y: 2 }), 1);
    assert_eq!(paths.wear_at(TileCoord { x: 4, y: 2 }), 11 * WEAR_PER_FOOTFALL);
    assert_eq!(paths.level_at(TileCoord { x: 4, y: 2 }), 2);
    assert_eq!(paths.wear_at(TileCoord { x: 5, y: 2 }), WEAR_MAX);
    assert_eq!(paths.level_at(TileCoord { x: 5, y: 2 }), 3);

    // Ground nobody has crossed stays clean.
    assert_eq!(paths.wear_at(TileCoord { x: 9, y: 6 }), 0);
    assert_eq!(paths.worn_count(), 4);

    // A world that has just come back must redraw all of it.
    assert!(paths.needs_resync());
}

/// The one that protects the owner's live playtest worlds.
#[test]
fn a_schema_4_save_still_loads() {
    use super::codec::encode_save_v4;
    use super::snapshot::{MIN_READABLE_SCHEMA, WorldSnapshotV4};

    assert_eq!(MIN_READABLE_SCHEMA, 4);
    assert_eq!(SCHEMA_VERSION, 5);

    let world = lived_in_world();
    let old = WorldSnapshotV4::capture(&world);
    assert_eq!(old.schema_version, 4);

    let meta = SaveMeta::from_snapshot(&WorldSnapshot::capture(&world), "A v4 world");
    let bytes = encode_save_v4(&meta, &old).expect("encode v4");
    // The envelope really does say 4, or this test is proving nothing.
    assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 4);

    let (read_meta, loaded) = decode_save(&bytes).expect("a v4 save must still open");
    assert_eq!(read_meta.label, "A v4 world");
    assert_eq!(
        loaded.schema_version, SCHEMA_VERSION,
        "a migrated world is a current world"
    );

    // Everything v4 knew about comes back untouched…
    let now = WorldSnapshot::capture(&world);
    assert_eq!(loaded.stations, now.stations);
    assert_eq!(loaded.peeps, now.peeps);
    assert_eq!(loaded.track, now.track);
    assert_eq!(loaded.trains, now.trains);
    assert_eq!(loaded.lines, now.lines);
    assert_eq!(loaded.economy, now.economy);
    assert_eq!(loaded.goals, now.goals);
    assert_eq!(loaded.borders, now.borders);
    assert_eq!(loaded.clock, now.clock);
    assert_eq!(loaded.money_cents, now.money_cents);
    assert_eq!(loaded.map, now.map);
    assert_eq!(loaded.town, now.town);
    assert_eq!(loaded.demand, now.demand);
    assert_eq!(loaded.anchors_seeded, now.anchors_seeded);

    // …and its ground is unmarked, which is the truth about a world whose
    // habits were never recorded.
    assert_eq!(loaded.paths, Default::default());

    // It restores, and it plays.
    let mut fresh = World::new();
    let report = loaded.restore(&mut fresh);
    assert!(report.is_clean(), "restore warnings: {:?}", report.warnings);
    assert_eq!(fresh.resource::<PathWear>().worn_count(), 0);

    // Saved again, it is a v5 world with a wear section of its own.
    let resaved = WorldSnapshot::capture(&fresh);
    assert_eq!(resaved.schema_version, SCHEMA_VERSION);
    let bytes = encode_save(&SaveMeta::from_snapshot(&resaved, "Migrated"), &resaved)
        .expect("encode");
    assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), SCHEMA_VERSION);
    assert!(decode_save(&bytes).is_ok());
}

#[test]
fn a_schema_from_the_future_is_still_refused() {
    let snapshot = WorldSnapshot::default();
    let mut bytes = encode_save(&SaveMeta::from_snapshot(&snapshot, "Tomorrow"), &snapshot)
        .expect("encode");
    bytes[4..6].copy_from_slice(&(SCHEMA_VERSION + 1).to_le_bytes());
    let end = bytes.len() - 4;
    let fixed = super::codec::crc32(&bytes[..end]);
    bytes[end..].copy_from_slice(&fixed.to_le_bytes());

    let err = decode_save(&bytes).unwrap_err();
    assert!(err.is_version_mismatch(), "got {err:?}");

    // And so is anything older than the oldest schema this build can read.
    let mut bytes = encode_save(&SaveMeta::from_snapshot(&snapshot, "Ancient"), &snapshot)
        .expect("encode");
    bytes[4..6].copy_from_slice(&3u16.to_le_bytes());
    let end = bytes.len() - 4;
    let fixed = super::codec::crc32(&bytes[..end]);
    bytes[end..].copy_from_slice(&fixed.to_le_bytes());
    assert!(decode_save(&bytes).unwrap_err().is_version_mismatch());
}

#[test]
fn loading_clears_undo_history_and_queued_commands() {
    use crate::command_buffer::CommandBuffer;
    use crate::commands::CommandKind;
    use crate::history::CommandHistory;

    let world = lived_in_world();
    let snapshot = WorldSnapshot::capture(&world);

    let mut fresh = World::new();
    let mut buffer = CommandBuffer::new();
    buffer.push(CommandKind::pause(true));
    fresh.insert_resource(buffer);
    let mut history = CommandHistory::new();
    history.record_player_action(vec![CommandKind::pause(false)]);
    fresh.insert_resource(history);

    snapshot.restore(&mut fresh);

    assert!(
        fresh.resource::<CommandBuffer>().is_empty(),
        "intent aimed at the old world must not survive a load"
    );
    assert!(
        !fresh.resource::<CommandHistory>().can_undo(),
        "undo inverses point at track that no longer exists"
    );
}

// ---------------------------------------------------------------------------
// Slots on real storage
// ---------------------------------------------------------------------------

#[test]
fn a_named_slot_round_trips_through_storage() {
    use_test_root();
    let slot = SaveSlot::named("Westbrook run").expect("valid name");
    let _ = delete_slot(&slot);

    let world = lived_in_world();
    let expected = WorldSnapshot::capture(&world);

    let info = save_to_slot(&world, &slot).expect("save");
    assert_eq!(info.meta.station_count, 2);
    assert_eq!(info.meta.track_count, 5);
    assert_eq!(info.meta.train_count, 2);
    assert_eq!(info.meta.peep_count, 3);
    assert_eq!(info.meta.line_count, 1);
    assert_eq!(info.meta.map_seed, MAP_SEED);
    assert_eq!(info.meta.sim_tick, SIM_TICK);
    assert!(info.meta.elapsed_sim_secs > 0);

    let loaded = load_from_slot(&slot).expect("load");
    assert_eq!(loaded, expected);

    let listed = list_slots().expect("list");
    assert!(
        listed.iter().any(|i| i.slot == slot && i.title() == "Westbrook run"),
        "{listed:?}"
    );

    delete_slot(&slot).expect("delete");
    assert!(load_from_slot(&slot).unwrap_err().is_not_found());
}

#[test]
fn an_empty_slot_reports_not_found() {
    use_test_root();
    let slot = SaveSlot::named("never written here").expect("valid name");
    let _ = delete_slot(&slot);
    let err = load_from_slot(&slot).unwrap_err();
    assert!(err.is_not_found(), "got {err:?}");
}
