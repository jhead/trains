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
use crate::trains::{
    track_for_station, Train, TrainCargo, TrainConsist, TrainLocation, TrainOnLine,
};

use super::payout::{goods_delivery_cents, haul_tiles, passenger_fare_cents};

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
    /// What this run is worth — the same distance-scaled figure
    /// [`super::payout`] will pay on arrival, so the board never advertises a
    /// price the delivery does not honour.
    pub reward_cents: i64,
}

/// Fare a passenger job between two stops is worth, or the shortest fare when
/// one of them has been demolished since the job was posted.
fn passenger_reward(stations: &StationRegistry, from: StationId, to: StationId) -> i64 {
    let tiles = match (stations.get(from), stations.get(to)) {
        (Some(a), Some(b)) => haul_tiles(a.tile, b.tile),
        _ => 1,
    };
    passenger_fare_cents(tiles)
}

/// Payout a goods job between two industries is worth.
fn goods_reward(industries: &IndustryRegistry, from: IndustryId, to: IndustryId) -> i64 {
    let tiles = match (industries.get(from), industries.get(to)) {
        (Some(a), Some(b)) => haul_tiles(a.tile, b.tile),
        _ => 1,
    };
    goods_delivery_cents(tiles)
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

/// How deep a single origin→destination queue is allowed to get.
///
/// # The demand a queue is made of
///
/// A pair used to hold **one** open job, and every further departure between
/// the same two places was dropped on the floor — see [`drain_peep_demand`].
/// With one carriage that cost nothing, because a train that can only take one
/// load cannot tell a queue of one from a queue of three. With
/// [consists](crate::TrainConsist) it is the whole difference between a second
/// carriage that earns and a second carriage that is weight, so real departures
/// now queue instead of vanishing.
///
/// **Three, and only from people.** `spawn_demand_jobs`' station-pair walk is
/// synthetic demand — a fixed heartbeat that exists so a new line has something
/// to carry — and letting *that* stack would mint fares out of the spawn
/// interval. Peep departures are people who decided to travel and are standing
/// on a platform; keeping them is bookkeeping, not generosity. Three is the
/// deepest queue the board will hold and exactly the longest consist a transit
/// couples ([`TRANSIT_PROFILE`](crate::TRANSIT_PROFILE)), so no carriage exists
/// that the board can never fill.
pub const MAX_PENDING_PER_PAIR: usize = 3;

/// The board only carries work a train could actually take.
///
/// # Why this filter exists
///
/// A job is not "somebody wants to travel" — the town wants that whether or not
/// there is a railway, and [`crate::demand::spawn_new_demand`] is what puts an
/// unserved place on the player's map. A **job** is a run the railway can make,
/// and the board is a fixed-size queue with no expiry: anything posted between
/// two places no train can reach sits there forever, holding a slot.
///
/// Left unfiltered that is fatal at cold start, and it was. `spawn_new_demand`
/// plants a new settlement every few minutes and every one of them is
/// unconnected *by definition*; each adds ordered pairs to the walk below, and
/// within a couple of minutes all eight passenger slots are demand between
/// villages with no track, while the one line the player actually built cannot
/// get a job posted. Measured on the opening beat: two fares in fifteen minutes,
/// against $440/min of running costs.
fn passenger_route_exists(
    network: &TrackNetwork,
    stations: &StationRegistry,
    from: StationId,
    to: StationId,
) -> bool {
    path_stations(network, stations, TrainKind::Transit, from, to).is_some()
}

fn goods_route_exists(
    network: &TrackNetwork,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    from: IndustryId,
    to: IndustryId,
) -> bool {
    path_industries(network, stations, industries, TrainKind::Transport, from, to).is_some()
}

/// Periodically create passenger A→B and goods industry→industry jobs.
pub fn spawn_demand_jobs(
    mut board: ResMut<JobBoard>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    network: Res<TrackNetwork>,
    mut service: ResMut<StationService>,
) {
    board.spawn_cooldown = board.spawn_cooldown.saturating_add(1);
    if board.spawn_cooldown < SPAWN_EVERY_TICKS {
        refresh_waiting(&board, &stations, &mut service);
        return;
    }
    board.spawn_cooldown = 0;

    // Sweep work the railway can no longer make. Track lifted between two stops
    // strands whatever was posted between them, and without this the slot is
    // gone for the rest of the session.
    board.jobs.retain(|job| match job.kind {
        JobKind::Passenger { from, to } => passenger_route_exists(&network, &stations, from, to),
        JobKind::Goods { from, to, .. } => {
            goods_route_exists(&network, &stations, &industries, from, to)
        }
    });

    // Sorted, because [`StationRegistry::iter`] walks a `HashMap` and the pair
    // this picks is indexed by a counter. Left unsorted, two runs of the same
    // seed send different people between different towns — and now that a fare
    // scales with the distance travelled, they earn different money for it.
    // Walk the stops rail has reached, not every stop on the map. This is the
    // half of the cold-start fix that matters most: the walk's period is
    // `n(n-1)`, so a served pair's share of the waves falls off as `1/n²` if the
    // whole map is in the domain. A player two minutes into a session has two
    // connected stops and a world busily adding unconnected ones — walking all
    // of them would push their one line's turn from every other wave to one in
    // thirty, and then one in ninety, purely because the world spoke up.
    let mut station_ids: Vec<StationId> = stations.iter().map(|s| s.id).collect();
    station_ids.sort_unstable_by_key(|id| id.0);
    let connected: Vec<StationId> = station_ids
        .into_iter()
        .filter(|id| {
            stations
                .get(*id)
                .and_then(|s| track_for_station(&network, s.tile, s.layer))
                .is_some()
        })
        .collect();

    if connected.len() >= 2 {
        let passenger_count = board
            .jobs
            .iter()
            .filter(|j| matches!(j.kind, JobKind::Passenger { .. }))
            .count();
        if passenger_count < MAX_PASSENGER_JOBS {
            if let Some((from, to)) = next_pair(&connected, spawn_wave(service.tick)) {
                let already = board.jobs.iter().any(|j| {
                    matches!(
                        &j.kind,
                        JobKind::Passenger { from: f, to: t } if *f == from && *t == to
                    )
                });
                if !already && passenger_route_exists(&network, &stations, from, to) {
                    board.jobs.push(Job {
                        kind: JobKind::Passenger { from, to },
                        reward_cents: passenger_reward(&stations, from, to),
                    });
                }
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
            if !exists
                && goods_route_exists(&network, &stations, &industries, producer.id, consumer.id)
            {
                board.jobs.push(Job {
                    kind: JobKind::Goods {
                        kind: good,
                        from: producer.id,
                        to: consumer.id,
                    },
                    reward_cents: goods_reward(&industries, producer.id, consumer.id),
                });
            }
        }
    }

    refresh_waiting(&board, &stations, &mut service);
}

/// Which spawn wave a tick belongs to.
///
/// [`StationService::tick`] advances once per Advance tick and this system fires
/// every [`SPAWN_EVERY_TICKS`], so this advances by exactly one per wave — a
/// per-wave counter that costs no new state on [`JobBoard`], which is
/// positionally serialised into the save.
fn spawn_wave(tick: u64) -> u64 {
    tick / SPAWN_EVERY_TICKS as u64
}

/// The `wave`-th ordered pair of `ids`, walking every pair before repeating.
///
/// # The walk that did not walk
///
/// This used to be `from = ids[tick % n]`, `to = ids[(tick / n + 1) % n]`, with
/// `tick` the raw sim tick. Between two spawn waves the tick advances by exactly
/// [`SPAWN_EVERY_TICKS`] = 45, so with three stations `tick % 3` never changed
/// (45 is a multiple of 3) and `tick / 3` advanced by 15, which is also a
/// multiple of 3 — **both** indices were frozen. Three seeded anchors is what
/// every new world opens with, so the board posted one ordered pair, forever,
/// and a one-in-three chance decided whether the player's first line was ever
/// given a single fare to earn. `economy_arc.rs` never saw it because it builds
/// two-station worlds, and `gcd(45, 2) = 1`.
///
/// Indexing by wave rather than by tick removes the shared factor, and
/// enumerating the pairs directly removes the modular arithmetic that hid it:
/// `n` stations have `n(n-1)` ordered pairs and this returns each in turn.
fn next_pair(ids: &[StationId], wave: u64) -> Option<(StationId, StationId)> {
    let n = ids.len();
    if n < 2 {
        return None;
    }
    let pairs = (n * (n - 1)) as u64;
    let k = (wave % pairs) as usize;
    let from = k / (n - 1);
    let offset = k % (n - 1);
    // Skip the diagonal: `to` is never `from`.
    let to = if offset >= from { offset + 1 } else { offset };
    Some((ids[from], ids[to]))
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
pub fn drain_peep_demand(
    mut board: ResMut<JobBoard>,
    mut flow: ResMut<DistrictFlow>,
    stations: Res<StationRegistry>,
    network: Res<TrackNetwork>,
) {
    for (from, to) in flow.take_pending() {
        if from == to {
            continue;
        }
        if board.jobs.len() >= MAX_PASSENGER_JOBS + MAX_GOODS_JOBS {
            break;
        }
        // Somebody in an unconnected village still wants to travel — they walk,
        // and the flow model grades that as a failed journey. What they cannot
        // do is become a *rail* job holding a board slot no train can clear.
        // See [`passenger_route_exists`].
        if !passenger_route_exists(&network, &stations, from, to) {
            continue;
        }
        // A queue, up to [`MAX_PENDING_PER_PAIR`] deep. This used to drop any
        // departure for a pair that already had one open, which threw away the
        // only demand signal in the game that says *more people are waiting
        // than one carriage can lift*.
        let queued = board
            .jobs
            .iter()
            .filter(|j| {
                matches!(
                    &j.kind,
                    JobKind::Passenger { from: f, to: t } if *f == from && *t == to
                )
            })
            .count();
        if queued >= MAX_PENDING_PER_PAIR {
            continue;
        }
        let reward_cents = passenger_reward(&stations, from, to);
        board.jobs.push(Job {
            kind: JobKind::Passenger { from, to },
            reward_cents,
        });
    }
}

/// Put the run a train was carrying back on the board.
///
/// [`assign_jobs`] **removes** a job from the board when a train takes it, so a
/// job in transit exists only as that train's [`TrainCargo`]. Selling the train
/// would therefore delete demand the town still has — the passengers on the
/// platform did not stop wanting to travel because the player sold the engine.
/// So the job goes back exactly the way a failed assignment returns it, priced
/// fresh from the same distance rule so the board never advertises a fare the
/// delivery will not honour.
///
/// `loads` is how many carloads of it there were — a three-car transit sold
/// mid-run puts three runs back, because three carriages of people were on it.
///
/// Returns `true` when something was put back. An empty train, or one whose
/// queue is already at [`MAX_PENDING_PER_PAIR`], adds nothing.
pub fn requeue_cargo(
    board: &mut JobBoard,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    cargo: &TrainCargo,
    loads: u8,
) -> bool {
    let job = match cargo {
        TrainCargo::Empty => return false,
        TrainCargo::Passengers { from, to } => Job {
            kind: JobKind::Passenger {
                from: *from,
                to: *to,
            },
            reward_cents: passenger_reward(stations, *from, *to),
        },
        TrainCargo::Goods { kind, from, to } => Job {
            kind: JobKind::Goods {
                kind: *kind,
                from: *from,
                to: *to,
            },
            reward_cents: goods_reward(industries, *from, *to),
        },
    };
    let mut queued = board.jobs.iter().filter(|j| j.kind == job.kind).count();
    let mut put_back = false;
    for _ in 0..loads.max(1) {
        if queued >= MAX_PENDING_PER_PAIR {
            break;
        }
        board.jobs.push(job.clone());
        queued += 1;
        put_back = true;
    }
    put_back
}

/// Take up to `extra` further copies of `kind` off the board.
///
/// The board holds a queue for a pair, and a consist lifts as much of it as it
/// has cars for. Every entry with the same [`JobKind`] is the same working
/// between the same two places, so the train's route is already right — this is
/// only how many carriages of it are filled.
fn take_extra_loads(board: &mut JobBoard, kind: &JobKind, extra: u8) -> u8 {
    let mut taken = 0;
    while taken < extra {
        let Some(idx) = board.jobs.iter().position(|j| j.kind == *kind) else {
            break;
        };
        board.jobs.remove(idx);
        taken = taken.saturating_add(1);
    }
    taken
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
///
/// # Boarding fills the train, not a car
///
/// A train takes one working — one origin, one destination — and then fills as
/// many of its cars as the queue for that pair can supply
/// ([`take_extra_loads`]). What is left stays on the board for whatever calls
/// next, so a car nobody is waiting for earns nothing and a queue three deep is
/// cleared in one call by a train long enough to do it. **That is the whole
/// capacity model**, and it is why a second carriage is worthless on a quiet
/// line and worth its price on a busy one.
// The query is a five-tuple with two optional components in it, which is what a
// train *is* — identity, position, load, assignment, composition. Naming a type
// alias for it would move the list somewhere the reader has to go and find.
#[allow(clippy::type_complexity)]
pub fn assign_jobs(
    mut board: ResMut<JobBoard>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    network: Res<TrackNetwork>,
    lines: Res<LineRegistry>,
    mut service: ResMut<StationService>,
    mut q: Query<(
        &Train,
        &mut TrainLocation,
        &mut TrainCargo,
        Option<&mut TrainOnLine>,
        Option<&mut TrainConsist>,
    )>,
) {
    for (train, mut loc, mut cargo, on_line, mut consist) in q.iter_mut() {
        if loc.parked || loc.dwell_remaining > 0 || !cargo.is_empty() || !loc.at_destination() {
            continue;
        }
        // An empty train has every car free; a train with no consist component
        // is the single car it always was.
        let cars = consist.as_ref().map(|c| c.cars.max(1)).unwrap_or(1);

        // Line-assigned: prefer on-line jobs, else shuttle.
        if let Some(mut on) = on_line {
            let Some(line) = lines.get(on.line) else {
                continue;
            };
            // Where this train is standing, before it sets off again. A train
            // that leaves a platform has called there, and 06 §5 counts that as
            // service whether or not anybody was riding to it.
            let calling_at = station_at_track(&network, &stations, loc.track);
            let loads = try_assign_line_job(
                &mut board,
                &stations,
                &industries,
                &network,
                train,
                &mut loc,
                &mut cargo,
                line,
                cars,
            );
            if loads > 0 {
                if let Some(c) = consist.as_mut() {
                    c.load(loads);
                }
                if let Some(here) = calling_at {
                    service.record_call(here);
                }
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
                    if let Some(here) = calling_at {
                        service.record_call(here);
                    }
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

        let loads = match job.kind {
            JobKind::Passenger { from, to } => take_passenger_job(
                &mut board,
                &stations,
                &network,
                train,
                &mut loc,
                &mut cargo,
                from,
                to,
                job.reward_cents,
                cars,
            ),
            JobKind::Goods { kind, from, to } => take_goods_job(
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
                cars,
            ),
        };
        if loads > 0 {
            if let Some(c) = consist.as_mut() {
                c.load(loads);
            }
        }
    }
}

/// Loads boarded — `0` when the line had no work this train could take.
#[allow(clippy::too_many_arguments)]
fn try_assign_line_job(
    board: &mut JobBoard,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    network: &TrackNetwork,
    train: &Train,
    loc: &mut TrainLocation,
    cargo: &mut TrainCargo,
    line: &crate::lines::Line,
    cars: u8,
) -> u8 {
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
                return 0;
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
                    cars,
                ),
                _ => 0,
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
                return 0;
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
                    cars,
                ),
                _ => 0,
            }
        }
    }
}

/// Board a passenger working, filling as many cars as the queue allows.
///
/// Returns the loads aboard, or `0` when the run could not be taken at all —
/// in which case the job has already been put back exactly as it was.
#[allow(clippy::too_many_arguments)]
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
    cars: u8,
) -> u8 {
    let Some(leg) = path_stations(network, stations, train.kind, from, to) else {
        board.jobs.push(Job {
            kind: JobKind::Passenger { from, to },
            reward_cents,
        });
        return 0;
    };
    let Some(from_track) = station_track(network, stations, from) else {
        board.jobs.push(Job {
            kind: JobKind::Passenger { from, to },
            reward_cents,
        });
        return 0;
    };
    let full = if loc.track == from_track {
        leg
    } else {
        let Some(to_from) = find_path_for_kind(network, loc.track, from_track, train.kind) else {
            board.jobs.push(Job {
                kind: JobKind::Passenger { from, to },
                reward_cents,
            });
            return 0;
        };
        join_paths(to_from, leg)
    };
    loc.set_path(full);
    *cargo = TrainCargo::Passengers { from, to };
    // The rest of the queue for this pair, up to the length of the train.
    1 + take_extra_loads(
        board,
        &JobKind::Passenger { from, to },
        cars.max(1).saturating_sub(1),
    )
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
    cars: u8,
) -> u8 {
    let Some(leg) = path_industries(network, stations, industries, train.kind, from, to) else {
        board.jobs.push(Job {
            kind: JobKind::Goods { kind, from, to },
            reward_cents,
        });
        return 0;
    };
    let Some(from_track) = industry_track(network, stations, industries, from) else {
        board.jobs.push(Job {
            kind: JobKind::Goods { kind, from, to },
            reward_cents,
        });
        return 0;
    };
    let full = if loc.track == from_track {
        leg
    } else {
        let Some(to_from) = find_path_for_kind(network, loc.track, from_track, train.kind) else {
            board.jobs.push(Job {
                kind: JobKind::Goods { kind, from, to },
                reward_cents,
            });
            return 0;
        };
        join_paths(to_from, leg)
    };
    loc.set_path(full);
    *cargo = TrainCargo::Goods { kind, from, to };
    // Freight runs one wagon today, so this asks for nothing extra — but it is
    // the same call the passenger side makes, so the day an industry carries
    // stock the wagons fill without a second code path.
    1 + take_extra_loads(
        board,
        &JobKind::Goods { kind, from, to },
        cars.max(1).saturating_sub(1),
    )
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

/// The stop a train standing on `track` is standing at, if any.
///
/// The reverse of [`station_track`], and a linear walk on purpose: there are a
/// handful of stops on a map, this is asked once per train that is idle at a
/// destination, and a second index would be one more thing a save has to keep
/// in step with the registry.
fn station_at_track(
    network: &TrackNetwork,
    stations: &StationRegistry,
    track: TrackId,
) -> Option<StationId> {
    stations
        .iter()
        .find(|s| track_for_station(network, s.tile, s.layer) == Some(track))
        .map(|s| s.id)
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
pub(crate) fn goods_platform_for<'a>(
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

    use crate::economy::{MoneyLedger, GOODS_DELIVERY_CENTS};
    use crate::ids::{TileCoord, TrainId};
    use crate::money::Money;
    use crate::stations::{IndustryTier, GOODS_PLATFORM_COST_CENTS};
    use crate::track::{try_place_track, TrackTerrain};

    fn ids(n: u64) -> Vec<StationId> {
        (1..=n).map(StationId).collect()
    }

    /// Every ordered pair comes round, for every size of network.
    ///
    /// The walk is the whole demand supply for a passenger railway. One pair it
    /// can never reach is a line the player built that can never earn.
    #[test]
    fn the_pair_walk_reaches_every_ordered_pair() {
        for n in 2..=6u64 {
            let stops = ids(n);
            let pairs = (n * (n - 1)) as usize;
            let mut seen: Vec<(StationId, StationId)> = (0..pairs as u64)
                .map(|wave| next_pair(&stops, wave).expect("two or more stops"))
                .collect();
            assert!(
                seen.iter().all(|(a, b)| a != b),
                "{n} stops: the walk sent somebody to the town they started in"
            );
            seen.sort_unstable_by_key(|(a, b)| (a.0, b.0));
            seen.dedup();
            assert_eq!(
                seen.len(),
                pairs,
                "{n} stops has {pairs} ordered pairs; the walk found {}",
                seen.len()
            );
        }
    }

    /// **The regression this file exists for.**
    ///
    /// The walk used to be indexed by the raw sim tick, and it fires every
    /// [`SPAWN_EVERY_TICKS`] ticks — so between waves the index advanced by
    /// exactly 45. With three stations `tick % 3` never changed, because 45 is a
    /// multiple of 3, and `tick / 3` advanced by 15, which is also a multiple of
    /// 3: **both ends of the pair were frozen for the whole session**. Three
    /// seeded anchors is what every new world opens with, so a one-in-three
    /// chance decided whether the player's first line was ever offered a fare.
    ///
    /// Indexing by wave is what fixes it, and this is the check that the two
    /// numbers cannot silently share a factor again.
    #[test]
    fn the_walk_still_walks_when_the_spawn_interval_divides_the_station_count() {
        for n in 2..=6u64 {
            let stops = ids(n);
            let waves: Vec<(StationId, StationId)> = (0..8)
                .map(|w| {
                    let tick = w * u64::from(SPAWN_EVERY_TICKS);
                    next_pair(&stops, spawn_wave(tick)).expect("two or more stops")
                })
                .collect();
            let distinct = {
                let mut v = waves.clone();
                v.sort_unstable_by_key(|(a, b)| (a.0, b.0));
                v.dedup();
                v.len()
            };
            assert!(
                distinct > 1,
                "{n} stops: eight consecutive spawn waves produced one pair \
                 forever — {waves:?}"
            );
        }
    }

    #[test]
    fn a_lone_station_has_nobody_to_send_anywhere() {
        assert_eq!(next_pair(&ids(1), 0), None);
        assert_eq!(next_pair(&[], 7), None);
    }

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
                // A line train banks a call as it leaves a platform, so the
                // assignment pass writes service now.
                .init_resource::<StationService>()
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

        /// The same, as a train of `cars` cars.
        fn spawn_consist(&mut self, kind: TrainKind, at: TrackId, cars: u8) {
            self.app.world_mut().spawn((
                Train {
                    id: TrainId(1),
                    kind,
                },
                TrainLocation::at_track(at),
                TrainCargo::Empty,
                TrainConsist::of(cars),
            ));
        }

        fn location(&mut self) -> TrainLocation {
            let mut q = self.app.world_mut().query::<&TrainLocation>();
            q.iter(self.app.world()).next().expect("a train").clone()
        }

        fn consist(&mut self) -> TrainConsist {
            let mut q = self.app.world_mut().query::<&TrainConsist>();
            *q.iter(self.app.world()).next().expect("a consist")
        }

        fn board_len(&self) -> usize {
            self.app.world().resource::<JobBoard>().jobs.len()
        }
    }

    /// Put `count` copies of one passenger working on the board.
    fn queue_pair(sim: &mut Sim, from: StationId, to: StationId, count: usize) {
        let mut board = sim.app.world_mut().resource_mut::<JobBoard>();
        for _ in 0..count {
            board.jobs.push(Job {
                kind: JobKind::Passenger { from, to },
                reward_cents: passenger_fare_cents(6),
            });
        }
    }

    /// Two stops on the straight line, six tiles apart.
    fn two_stops() -> StationRegistry {
        let mut stations = StationRegistry::new();
        stations.insert("Eastgate", TileCoord { x: 3, y: 8 }, GROUND_LAYER);
        stations.insert("Westbrook", TileCoord { x: 9, y: 8 }, GROUND_LAYER);
        stations
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

    // ─ Consists ────────────────────────────────────────────

    /// **The capacity claim, at the platform.** A three-car train clears a
    /// three-deep queue in one call; a single car takes one and leaves the rest
    /// standing, exactly as it always did.
    #[test]
    fn a_consist_boards_one_load_per_car_and_leaves_the_rest_on_the_board() {
        let network = line_world();
        let stations = two_stops();
        let (east, west) = (StationId(1), StationId(2));
        let start = railhead(&network, 3);

        let mut sim = Sim::new(
            network,
            stations,
            IndustryRegistry::new(),
            LineRegistry::new(),
        );
        queue_pair(&mut sim, east, west, 3);
        sim.spawn_consist(TrainKind::Transit, start, 3);
        sim.app.update();

        assert_eq!(
            sim.consist(),
            TrainConsist { cars: 3, laden: 3 },
            "three carriages of a three-deep queue"
        );
        assert_eq!(sim.board_len(), 0, "the platform is cleared");

        // One car, same queue: one load, and two people still waiting.
        let network = line_world();
        let start = railhead(&network, 3);
        let mut sim = Sim::new(
            network,
            two_stops(),
            IndustryRegistry::new(),
            LineRegistry::new(),
        );
        queue_pair(&mut sim, east, west, 3);
        sim.spawn_consist(TrainKind::Transit, start, 1);
        sim.app.update();
        assert_eq!(sim.consist(), TrainConsist { cars: 1, laden: 1 });
        assert_eq!(sim.board_len(), 2, "the rest wait for the next train");
    }

    /// **The trap, made real.** A car with no queue behind it carries nothing —
    /// it is weight the train drags round for free, which is why buying one on
    /// a quiet line is a mistake and buying one on a busy line is not.
    #[test]
    fn a_car_with_no_queue_behind_it_boards_nothing() {
        let network = line_world();
        let (east, west) = (StationId(1), StationId(2));
        let start = railhead(&network, 3);
        let mut sim = Sim::new(
            network,
            two_stops(),
            IndustryRegistry::new(),
            LineRegistry::new(),
        );
        queue_pair(&mut sim, east, west, 1);
        sim.spawn_consist(TrainKind::Transit, start, 3);
        sim.app.update();

        assert_eq!(
            sim.consist(),
            TrainConsist { cars: 3, laden: 1 },
            "one fare, three carriages: two of them are dead weight"
        );
    }

    /// A consist only ever carries **one working**. Two carloads for different
    /// destinations would be two routes, and a train has one path.
    #[test]
    fn a_consist_takes_one_working_not_a_carriage_for_each_destination() {
        let network = line_world();
        let mut stations = two_stops();
        let north = stations.insert("Northgate", TileCoord { x: 15, y: 8 }, GROUND_LAYER);
        let (east, west) = (StationId(1), StationId(2));
        let start = railhead(&network, 3);

        let mut sim = Sim::new(
            network,
            stations,
            IndustryRegistry::new(),
            LineRegistry::new(),
        );
        queue_pair(&mut sim, east, west, 1);
        queue_pair(&mut sim, east, north, 1);
        sim.spawn_consist(TrainKind::Transit, start, 3);
        sim.app.update();

        assert_eq!(
            sim.consist().laden,
            1,
            "the other working is somebody else's run"
        );
        assert_eq!(sim.board_len(), 1, "and it is still posted");
    }

    /// Freight runs one wagon, so a goods train boards one load however deep
    /// the works' queue somehow got. The boarding code is shared, and this is
    /// what stops the profile cap being the only thing holding it.
    #[test]
    fn a_goods_train_boards_one_load() {
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

        let mut sim = Sim::new(
            network,
            StationRegistry::new(),
            industries,
            LineRegistry::new(),
        );
        {
            let mut board = sim.app.world_mut().resource_mut::<JobBoard>();
            for _ in 0..2 {
                board.jobs.push(Job {
                    kind: JobKind::Goods {
                        kind: GoodKind::Lumber,
                        from: saw,
                        to: mill,
                    },
                    reward_cents: GOODS_DELIVERY_CENTS,
                });
            }
        }
        sim.spawn_consist(TrainKind::Transport, start, 1);
        sim.app.update();

        assert_eq!(sim.consist().laden, 1);
        assert_eq!(sim.board_len(), 1);
    }

    /// **Where a queue comes from.** A departure for a pair that already has one
    /// posted used to be dropped on the floor; the second and third now wait
    /// their turn instead, and the fourth is still dropped.
    ///
    /// The demand arrives a tick at a time because
    /// [`DistrictFlow::request_trip`](crate::peeps::DistrictFlow::request_trip)
    /// dedupes its own queue — a pair that keeps producing travellers builds a
    /// queue *over time*, which is precisely the signal a second carriage is
    /// meant to answer.
    #[test]
    fn peep_departures_queue_up_to_three_deep_and_no_further() {
        let network = line_world();
        let stations = two_stops();
        let (east, west) = (StationId(1), StationId(2));

        let mut app = App::new();
        app.init_resource::<JobBoard>()
            .init_resource::<DistrictFlow>()
            .insert_resource(network)
            .insert_resource(stations)
            .add_systems(Update, drain_peep_demand);

        let mut depths = Vec::new();
        for _ in 0..6 {
            app.world_mut()
                .resource_mut::<DistrictFlow>()
                .request_trip(east, west);
            app.update();
            depths.push(app.world().resource::<JobBoard>().jobs.len());
        }

        assert_eq!(
            depths,
            vec![1, 2, 3, 3, 3, 3],
            "the queue should deepen to the cap and stop"
        );
        let board = app.world().resource::<JobBoard>();
        assert!(board.jobs.iter().all(|j| j.kind
            == JobKind::Passenger {
                from: east,
                to: west
            }));
        assert_eq!(board.jobs.len(), MAX_PENDING_PER_PAIR);
    }

    /// The station-pair walk is a heartbeat, not a crowd: it posts **one** job
    /// per pair however often it fires. Letting synthetic demand stack would
    /// mint fares out of the spawn interval.
    #[test]
    fn the_pair_walk_still_posts_one_job_per_pair() {
        let network = line_world();
        let stations = two_stops();

        let mut app = App::new();
        app.init_resource::<JobBoard>()
            .init_resource::<StationService>()
            .insert_resource(network)
            .insert_resource(stations)
            .insert_resource(IndustryRegistry::new())
            .add_systems(Update, spawn_demand_jobs);

        // Long enough for many spawn waves.
        for _ in 0..(SPAWN_EVERY_TICKS as u32 * 6) {
            app.world_mut().resource_mut::<StationService>().tick += 1;
            app.update();
        }

        let board = app.world().resource::<JobBoard>();
        for job in &board.jobs {
            let same = board.jobs.iter().filter(|j| j.kind == job.kind).count();
            assert_eq!(same, 1, "the walk stacked a pair: {:?}", board.jobs);
        }
    }

    /// Selling a loaded consist puts **every** carload back, not one of them.
    #[test]
    fn requeueing_a_consist_puts_back_a_run_per_loaded_car() {
        let stations = two_stops();
        let (east, west) = (StationId(1), StationId(2));
        let mut board = JobBoard::default();
        let cargo = TrainCargo::Passengers {
            from: east,
            to: west,
        };

        assert!(requeue_cargo(
            &mut board,
            &stations,
            &IndustryRegistry::new(),
            &cargo,
            3
        ));
        assert_eq!(board.jobs.len(), 3, "three carriages of people, three runs");

        // …and it never overfills the queue past the cap.
        assert!(!requeue_cargo(
            &mut board,
            &stations,
            &IndustryRegistry::new(),
            &cargo,
            2
        ));
        assert_eq!(board.jobs.len(), MAX_PENDING_PER_PAIR);
    }

    /// Determinism: the same board and the same trains must board the same way
    /// every run. Boarding walks a `Vec` and the consist is read per entity, so
    /// there is no map iteration in the path — this is the guard that keeps it
    /// that way.
    #[test]
    fn boarding_a_queue_is_deterministic_across_runs() {
        let outcome = || {
            let network = line_world();
            let stations = two_stops();
            let (east, west) = (StationId(1), StationId(2));
            let start = railhead(&network, 3);
            let mut sim = Sim::new(
                network,
                stations,
                IndustryRegistry::new(),
                LineRegistry::new(),
            );
            queue_pair(&mut sim, east, west, 3);
            queue_pair(&mut sim, west, east, 2);
            sim.spawn_consist(TrainKind::Transit, start, 2);
            sim.app.update();
            let board: Vec<JobKind> = sim
                .app
                .world()
                .resource::<JobBoard>()
                .jobs
                .iter()
                .map(|j| j.kind.clone())
                .collect();
            (sim.consist(), sim.location().path, board)
        };
        let first = outcome();
        for _ in 0..4 {
            assert_eq!(outcome(), first, "the same railway boarded differently");
        }
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
