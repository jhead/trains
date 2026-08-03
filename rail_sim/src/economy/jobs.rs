//! Passenger and goods demand jobs.

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::commands::TrainKind;
use crate::ids::{StationId, TrackId};
use crate::lines::LineRegistry;
use crate::peeps::DistrictFlow;
use crate::stations::{
    GoodKind, IndustryId, IndustryRegistry, StationRegistry, StationService, StationTier,
};
use crate::track::{TrackNetwork, GROUND_LAYER};
use crate::trains::find_path_for_kind;
use crate::trains::{track_for_station, Train, TrainCargo, TrainLocation, TrainOnLine};

use super::payout::{GOODS_DELIVERY_CENTS, PASSENGER_FARE_CENTS};

/// Pending demand the player can fulfill with trains.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobKind {
    Passenger {
        from: StationId,
        to: StationId,
    },
    Goods {
        kind: GoodKind,
        from: IndustryId,
        to: IndustryId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub kind: JobKind,
    pub reward_cents: i64,
}

/// Open jobs waiting for a train.
#[derive(Debug, Clone, Default, PartialEq, Eq, Resource, Serialize, Deserialize)]
pub struct JobBoard {
    pub jobs: Vec<Job>,
    /// Ticks since last spawn wave.
    pub spawn_cooldown: u16,
}

const MAX_PASSENGER_JOBS: usize = 8;
const MAX_GOODS_JOBS: usize = 4;
const SPAWN_EVERY_TICKS: u16 = 45;

/// Periodically create passenger A→B and goods industry→industry jobs.
pub fn spawn_demand_jobs(
    mut board: ResMut<JobBoard>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    mut service: ResMut<StationService>,
) {
    board.spawn_cooldown = board.spawn_cooldown.saturating_add(1);
    if board.spawn_cooldown < SPAWN_EVERY_TICKS {
        refresh_waiting(&board, &stations, &mut service);
        return;
    }
    board.spawn_cooldown = 0;

    let station_ids: Vec<StationId> = stations.iter().map(|s| s.id).collect();
    if station_ids.len() >= 2 {
        let passenger_count = board
            .jobs
            .iter()
            .filter(|j| matches!(j.kind, JobKind::Passenger { .. }))
            .count();
        if passenger_count < MAX_PASSENGER_JOBS {
            let tick = service.tick as usize;
            let from = station_ids[tick % station_ids.len()];
            let to = station_ids[(tick / station_ids.len() + 1) % station_ids.len()];
            if from != to
                && !board.jobs.iter().any(|j| {
                    matches!(
                        &j.kind,
                        JobKind::Passenger { from: f, to: t } if *f == from && *t == to
                    )
                })
            {
                board.jobs.push(Job {
                    kind: JobKind::Passenger { from, to },
                    reward_cents: PASSENGER_FARE_CENTS,
                });
            }
        }
    }

    let goods_count = board
        .jobs
        .iter()
        .filter(|j| matches!(j.kind, JobKind::Goods { .. }))
        .count();
    if goods_count < MAX_GOODS_JOBS {
        for good in [GoodKind::Lumber, GoodKind::Ore] {
            let Some(producer) = industries.producer_of(good) else {
                continue;
            };
            let Some(consumer) = industries.consumer_of(good) else {
                continue;
            };
            if producer.id == consumer.id {
                continue;
            }
            let exists = board.jobs.iter().any(|j| {
                matches!(
                    &j.kind,
                    JobKind::Goods { kind: k, from: f, to: t }
                        if *k == good && *f == producer.id && *t == consumer.id
                )
            });
            if !exists {
                board.jobs.push(Job {
                    kind: JobKind::Goods {
                        kind: good,
                        from: producer.id,
                        to: consumer.id,
                    },
                    reward_cents: GOODS_DELIVERY_CENTS,
                });
            }
        }
    }

    refresh_waiting(&board, &stations, &mut service);
}

/// Publish the peep platform queue into the service score.
///
/// Must run *before* [`spawn_demand_jobs`], whose `refresh_waiting` charges the
/// tick's crowding penalty from the blended total.
pub fn sync_peep_platform_pressure(
    flow: Res<DistrictFlow>,
    stations: Res<StationRegistry>,
    mut service: ResMut<StationService>,
) {
    for s in stations.iter() {
        service.set_peep_waiting(s.id, flow.get(s.id).waiting);
    }
}

/// Turn peep departures into real passenger jobs.
///
/// This is the join between the two demand models: routines decide *when*
/// people travel, and this drains those intents onto the board so trains
/// actually serve them. Without it a morning peak is visible in the town and
/// invisible to the railway.
pub fn drain_peep_demand(mut board: ResMut<JobBoard>, mut flow: ResMut<DistrictFlow>) {
    for (from, to) in flow.take_pending() {
        if from == to {
            continue;
        }
        if board.jobs.len() >= MAX_PASSENGER_JOBS + MAX_GOODS_JOBS {
            break;
        }
        let already = board.jobs.iter().any(|j| {
            matches!(
                &j.kind,
                JobKind::Passenger { from: f, to: t } if *f == from && *t == to
            )
        });
        if already {
            continue;
        }
        board.jobs.push(Job {
            kind: JobKind::Passenger { from, to },
            reward_cents: PASSENGER_FARE_CENTS,
        });
    }
}

fn refresh_waiting(board: &JobBoard, stations: &StationRegistry, service: &mut StationService) {
    for s in stations.iter() {
        let waiting = board
            .jobs
            .iter()
            .filter(|j| matches!(&j.kind, JobKind::Passenger { from, .. } if *from == s.id))
            .count() as u32;
        service.set_waiting(s.id, waiting);
    }
}

/// Assign open jobs to idle empty trains at destination (ready for a new run).
///
/// Trains on a line prefer jobs whose endpoints lie on that line; otherwise they
/// shuttle to the next stop. Free-roam trains take any compatible job.
pub fn assign_jobs(
    mut board: ResMut<JobBoard>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    network: Res<TrackNetwork>,
    lines: Res<LineRegistry>,
    mut q: Query<(
        &Train,
        &mut TrainLocation,
        &mut TrainCargo,
        Option<&mut TrainOnLine>,
    )>,
) {
    for (train, mut loc, mut cargo, on_line) in q.iter_mut() {
        if loc.parked || loc.dwell_remaining > 0 || !cargo.is_empty() || !loc.at_destination() {
            continue;
        }

        // Line-assigned: prefer on-line jobs, else shuttle.
        if let Some(mut on) = on_line {
            let Some(line) = lines.get(on.line) else {
                continue;
            };
            if try_assign_line_job(
                &mut board,
                &stations,
                &industries,
                &network,
                train,
                &mut loc,
                &mut cargo,
                line,
            ) {
                // Advance next_stop toward the job destination if passenger.
                if let TrainCargo::Passengers { to, .. } = *cargo {
                    if let Some(idx) = line.stop_index(to) {
                        on.next_stop = idx;
                    }
                }
                continue;
            }
            // Shuttle empty along the line. A stop demolished out from under
            // the schedule shifts every index after it, so the destination is
            // read through `get`: the train re-paths to whatever call is
            // actually there, and a dormant line simply hands it nothing.
            if let Some(next_idx) = line.next_stop_index(on.next_stop, &mut on.forward) {
                let Some(&dest_station) = line.stops.get(next_idx) else {
                    continue;
                };
                if let Some(path) =
                    path_to_station(&network, &stations, train.kind, loc.track, dest_station)
                {
                    loc.set_path(path);
                    on.next_stop = next_idx;
                }
            }
            continue;
        }

        let job_index = match train.kind {
            TrainKind::Transit => board.jobs.iter().position(|j| match j.kind {
                JobKind::Passenger { from, to } => {
                    path_stations(&network, &stations, train.kind, from, to).is_some()
                }
                _ => false,
            }),
            TrainKind::Transport => board.jobs.iter().position(|j| match j.kind {
                JobKind::Goods { from, to, .. } => {
                    path_industries(&network, &stations, &industries, train.kind, from, to)
                        .is_some()
                }
                _ => false,
            }),
        };

        let Some(idx) = job_index else {
            continue;
        };
        let job = board.jobs.remove(idx);

        match job.kind {
            JobKind::Passenger { from, to } => {
                if !take_passenger_job(
                    &mut board,
                    &stations,
                    &network,
                    train,
                    &mut loc,
                    &mut cargo,
                    from,
                    to,
                    job.reward_cents,
                ) {
                    continue;
                }
            }
            JobKind::Goods { kind, from, to } => {
                if !take_goods_job(
                    &mut board,
                    &stations,
                    &industries,
                    &network,
                    train,
                    &mut loc,
                    &mut cargo,
                    kind,
                    from,
                    to,
                    job.reward_cents,
                ) {
                    continue;
                }
            }
        }
    }
}

fn try_assign_line_job(
    board: &mut JobBoard,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    network: &TrackNetwork,
    train: &Train,
    loc: &mut TrainLocation,
    cargo: &mut TrainCargo,
    line: &crate::lines::Line,
) -> bool {
    match train.kind {
        TrainKind::Transit => {
            let idx = board.jobs.iter().position(|j| match j.kind {
                JobKind::Passenger { from, to } => {
                    line.contains_station(from)
                        && line.contains_station(to)
                        && path_stations(network, stations, train.kind, from, to).is_some()
                }
                _ => false,
            });
            let Some(idx) = idx else {
                return false;
            };
            let job = board.jobs.remove(idx);
            match job.kind {
                JobKind::Passenger { from, to } => take_passenger_job(
                    board,
                    stations,
                    network,
                    train,
                    loc,
                    cargo,
                    from,
                    to,
                    job.reward_cents,
                ),
                _ => false,
            }
        }
        TrainKind::Transport => {
            // Goods: prefer jobs whose from/to industries sit near line stations.
            // Pragmatic: any goods job that pathfinds; still "on line" when the
            // train is assigned (line preference over free-roam is the shuttle).
            let idx = board.jobs.iter().position(|j| match j.kind {
                JobKind::Goods { from, to, .. } => {
                    path_industries(network, stations, industries, train.kind, from, to).is_some()
                }
                _ => false,
            });
            let Some(idx) = idx else {
                return false;
            };
            let job = board.jobs.remove(idx);
            match job.kind {
                JobKind::Goods { kind, from, to } => take_goods_job(
                    board,
                    stations,
                    industries,
                    network,
                    train,
                    loc,
                    cargo,
                    kind,
                    from,
                    to,
                    job.reward_cents,
                ),
                _ => false,
            }
        }
    }
}

fn take_passenger_job(
    board: &mut JobBoard,
    stations: &StationRegistry,
    network: &TrackNetwork,
    train: &Train,
    loc: &mut TrainLocation,
    cargo: &mut TrainCargo,
    from: StationId,
    to: StationId,
    reward_cents: i64,
) -> bool {
    let Some(leg) = path_stations(network, stations, train.kind, from, to) else {
        board.jobs.push(Job {
            kind: JobKind::Passenger { from, to },
            reward_cents,
        });
        return false;
    };
    let Some(from_track) = station_track(network, stations, from) else {
        board.jobs.push(Job {
            kind: JobKind::Passenger { from, to },
            reward_cents,
        });
        return false;
    };
    let full = if loc.track == from_track {
        leg
    } else {
        let Some(to_from) = find_path_for_kind(network, loc.track, from_track, train.kind) else {
            board.jobs.push(Job {
                kind: JobKind::Passenger { from, to },
                reward_cents,
            });
            return false;
        };
        join_paths(to_from, leg)
    };
    loc.set_path(full);
    *cargo = TrainCargo::Passengers { from, to };
    true
}

#[allow(clippy::too_many_arguments)]
fn take_goods_job(
    board: &mut JobBoard,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    network: &TrackNetwork,
    train: &Train,
    loc: &mut TrainLocation,
    cargo: &mut TrainCargo,
    kind: GoodKind,
    from: IndustryId,
    to: IndustryId,
    reward_cents: i64,
) -> bool {
    let Some(leg) = path_industries(network, stations, industries, train.kind, from, to) else {
        board.jobs.push(Job {
            kind: JobKind::Goods { kind, from, to },
            reward_cents,
        });
        return false;
    };
    let Some(from_track) = industry_track(network, stations, industries, from) else {
        board.jobs.push(Job {
            kind: JobKind::Goods { kind, from, to },
            reward_cents,
        });
        return false;
    };
    let full = if loc.track == from_track {
        leg
    } else {
        let Some(to_from) = find_path_for_kind(network, loc.track, from_track, train.kind) else {
            board.jobs.push(Job {
                kind: JobKind::Goods { kind, from, to },
                reward_cents,
            });
            return false;
        };
        join_paths(to_from, leg)
    };
    loc.set_path(full);
    *cargo = TrainCargo::Goods { kind, from, to };
    true
}

fn join_paths(mut a: Vec<TrackId>, b: Vec<TrackId>) -> Vec<TrackId> {
    if a.last() == b.first() {
        a.extend(b.into_iter().skip(1));
    } else {
        a.extend(b);
    }
    a
}

fn station_track(
    network: &TrackNetwork,
    stations: &StationRegistry,
    id: StationId,
) -> Option<TrackId> {
    let s = stations.get(id)?;
    track_for_station(network, s.tile, s.layer)
}

fn path_to_station(
    network: &TrackNetwork,
    stations: &StationRegistry,
    kind: TrainKind,
    from_track: TrackId,
    to: StationId,
) -> Option<Vec<TrackId>> {
    let b = station_track(network, stations, to)?;
    find_path_for_kind(network, from_track, b, kind)
}

fn path_stations(
    network: &TrackNetwork,
    stations: &StationRegistry,
    kind: TrainKind,
    from: StationId,
    to: StationId,
) -> Option<Vec<TrackId>> {
    let a = station_track(network, stations, from)?;
    let b = station_track(network, stations, to)?;
    find_path_for_kind(network, a, b, kind)
}

/// Where a freight train calls to work an industry.
///
/// The goods platform built against the lot (04 §6) if the player put one
/// there, otherwise the railhead on the industry's own tile — a line run
/// straight into the works still loads, exactly as it did before platforms.
fn industry_track(
    network: &TrackNetwork,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    id: IndustryId,
) -> Option<TrackId> {
    let ind = industries.get(id)?;
    if let Some(platform) = goods_platform_for(stations, ind) {
        if let Some(track) = track_for_station(network, platform.tile, platform.layer) {
            return Some(track);
        }
    }
    track_for_station(network, ind.tile, GROUND_LAYER)
}

/// The goods platform serving `industry`, lowest [`StationId`] first.
fn goods_platform_for<'a>(
    stations: &'a StationRegistry,
    industry: &crate::stations::Industry,
) -> Option<&'a crate::stations::Station> {
    stations
        .iter()
        .filter(|s| s.tier == StationTier::GoodsPlatform && industry.abuts(s.tile))
        .min_by_key(|s| s.id.0)
}

fn path_industries(
    network: &TrackNetwork,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    kind: TrainKind,
    from: IndustryId,
    to: IndustryId,
) -> Option<Vec<TrackId>> {
    let a = industry_track(network, stations, industries, from)?;
    let b = industry_track(network, stations, industries, to)?;
    find_path_for_kind(network, a, b, kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::{App, Update};

    use crate::economy::MoneyLedger;
    use crate::ids::{TileCoord, TrainId};
    use crate::money::Money;
    use crate::stations::{IndustryTier, GOODS_PLATFORM_COST_CENTS};
    use crate::track::{try_place_track, TrackTerrain};

    /// Flat land with one east-west line along `y = 8`, `x = 2..=17`.
    fn line_world() -> TrackNetwork {
        let terrain = TrackTerrain::new(32, 32, (0..32 * 32).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut money = Money::new(10_000_000);
        let mut ledger = MoneyLedger::default();
        for x in 2..=17 {
            try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                TileCoord { x, y: 8 },
                GROUND_LAYER,
            )
            .expect("track");
        }
        network
    }

    struct Sim {
        app: App,
    }

    impl Sim {
        fn new(
            network: TrackNetwork,
            stations: StationRegistry,
            industries: IndustryRegistry,
            lines: LineRegistry,
        ) -> Self {
            let mut app = App::new();
            app.init_resource::<JobBoard>()
                .insert_resource(network)
                .insert_resource(stations)
                .insert_resource(industries)
                .insert_resource(lines)
                .add_systems(Update, assign_jobs);
            Self { app }
        }

        fn spawn(&mut self, kind: TrainKind, at: TrackId, on: Option<TrainOnLine>) {
            let mut entity = self.app.world_mut().spawn((
                Train {
                    id: TrainId(1),
                    kind,
                },
                TrainLocation::at_track(at),
                TrainCargo::Empty,
            ));
            if let Some(on) = on {
                entity.insert(on);
            }
        }

        fn location(&mut self) -> TrainLocation {
            let mut q = self.app.world_mut().query::<&TrainLocation>();
            q.iter(self.app.world()).next().expect("a train").clone()
        }
    }

    fn railhead(network: &TrackNetwork, x: i32) -> TrackId {
        network
            .id_at(TileCoord { x, y: 8 }, GROUND_LAYER)
            .expect("railhead")
    }

    /// A stop demolished from the middle of a route shifts every index after
    /// it. A train still holding the old index must find a call that is
    /// actually there, not stall or run off the end of the list.
    #[test]
    fn a_train_whose_next_stop_was_demolished_repaths_to_a_remaining_stop() {
        let network = line_world();
        let mut stations = StationRegistry::new();
        let a = stations.insert("Eastgate", TileCoord { x: 3, y: 8 }, GROUND_LAYER);
        let b = stations.insert("Millhaven", TileCoord { x: 9, y: 8 }, GROUND_LAYER);
        let c = stations.insert("Ridgeline", TileCoord { x: 15, y: 8 }, GROUND_LAYER);
        let mut lines = LineRegistry::new();
        let line = lines.create("Riverside Loop".into(), vec![a, b, c]).unwrap();

        // The middle stop is demolished while the train stands at the first.
        assert_eq!(
            lines.remove_stop(line, b).expect("b was a call").indices,
            vec![1]
        );
        stations.remove(b);

        let start = railhead(&network, 3);
        let far = railhead(&network, 15);
        let mut sim = Sim::new(network, stations, IndustryRegistry::new(), lines);
        sim.spawn(
            TrainKind::Transit,
            start,
            Some(TrainOnLine {
                line,
                // Stale: it was aiming past the end of the shortened route.
                next_stop: 2,
                forward: true,
            }),
        );

        // One tick to bounce off the clamped end of the shorter route, one to
        // set off again — the point is that it is never left with nothing.
        for _ in 0..2 {
            sim.app.update();
        }

        let loc = sim.location();
        assert!(
            !loc.at_destination(),
            "the train must be given somewhere to go, not left standing"
        );
        assert_eq!(
            loc.destination(),
            Some(far),
            "it re-paths to the stop the line still calls at"
        );
    }

    /// A line with nothing left to run: the train idles where it stands rather
    /// than wedging or driving at a station that no longer exists.
    #[test]
    fn a_train_on_a_dormant_line_idles_instead_of_wedging() {
        let network = line_world();
        let mut stations = StationRegistry::new();
        let a = stations.insert("Eastgate", TileCoord { x: 3, y: 8 }, GROUND_LAYER);
        let b = stations.insert("Millhaven", TileCoord { x: 9, y: 8 }, GROUND_LAYER);
        let mut lines = LineRegistry::new();
        let line = lines.create("Eastgate - Millhaven".into(), vec![a, b]).unwrap();

        assert!(lines.remove_stop(line, b).expect("b was a call").dormant);
        stations.remove(b);

        let start = railhead(&network, 3);
        let mut sim = Sim::new(network, stations, IndustryRegistry::new(), lines);
        sim.spawn(
            TrainKind::Transit,
            start,
            Some(TrainOnLine {
                line,
                next_stop: 1,
                forward: true,
            }),
        );

        for _ in 0..4 {
            sim.app.update();
        }

        let loc = sim.location();
        assert_eq!(loc.track, start, "it stays put");
        assert!(loc.at_destination(), "and asks for nothing it cannot reach");
    }

    /// 04 §6: the goods platform is what a freight train actually calls at.
    #[test]
    fn a_goods_train_routes_to_the_platform_built_against_the_industry() {
        let network = line_world();
        let mut industries = IndustryRegistry::new();
        // Both works sit a row off the line, so no track touches either tile.
        let saw = industries.insert_tier(
            "Pine Sawmill",
            TileCoord { x: 4, y: 6 },
            IndustryTier::Works,
            Some(GoodKind::Lumber),
            None,
        );
        let mill = industries.insert_tier(
            "Harbor Mill",
            TileCoord { x: 14, y: 6 },
            IndustryTier::Works,
            None,
            Some(GoodKind::Lumber),
        );
        let mut stations = StationRegistry::new();
        for (name, x) in [("Sawmill Goods Platform", 4), ("Harbor Goods Platform", 14)] {
            stations.insert_tier(
                name,
                TileCoord { x, y: 8 },
                GROUND_LAYER,
                StationTier::GoodsPlatform,
                GOODS_PLATFORM_COST_CENTS,
            );
        }

        let start = railhead(&network, 4);
        let delivery = railhead(&network, 14);
        let mut sim = Sim::new(network, stations, industries, LineRegistry::new());
        sim.app.world_mut().resource_mut::<JobBoard>().jobs.push(Job {
            kind: JobKind::Goods {
                kind: GoodKind::Lumber,
                from: saw,
                to: mill,
            },
            reward_cents: GOODS_DELIVERY_CENTS,
        });
        sim.spawn(TrainKind::Transport, start, None);

        sim.app.update();

        assert_eq!(
            sim.location().destination(),
            Some(delivery),
            "the run ends at the platform serving the mill, which is the only \
             railhead that reaches it at all"
        );
        assert!(
            sim.app.world().resource::<JobBoard>().jobs.is_empty(),
            "the job was taken"
        );
    }

    /// Without a platform, a line run straight into the works still loads.
    #[test]
    fn an_industry_with_no_platform_still_loads_off_its_own_railhead() {
        let network = line_world();
        let mut industries = IndustryRegistry::new();
        let saw = industries.insert_tier(
            "Pine Sawmill",
            TileCoord { x: 4, y: 8 },
            IndustryTier::Yard,
            Some(GoodKind::Lumber),
            None,
        );
        let mill = industries.insert_tier(
            "Harbor Mill",
            TileCoord { x: 14, y: 8 },
            IndustryTier::Yard,
            None,
            Some(GoodKind::Lumber),
        );

        let start = railhead(&network, 4);
        let end = railhead(&network, 14);
        let mut sim = Sim::new(
            network,
            StationRegistry::new(),
            industries,
            LineRegistry::new(),
        );
        sim.app.world_mut().resource_mut::<JobBoard>().jobs.push(Job {
            kind: JobKind::Goods {
                kind: GoodKind::Lumber,
                from: saw,
                to: mill,
            },
            reward_cents: GOODS_DELIVERY_CENTS,
        });
        sim.spawn(TrainKind::Transport, start, None);

        sim.app.update();

        assert_eq!(sim.location().destination(), Some(end));
    }
}
