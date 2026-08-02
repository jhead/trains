//! Passenger and goods demand jobs.

use bevy_ecs::prelude::*;

use crate::commands::TrainKind;
use crate::ids::{StationId, TrackId};
use crate::lines::LineRegistry;
use crate::stations::{GoodKind, IndustryId, IndustryRegistry, StationRegistry, StationService};
use crate::track::{TrackNetwork, GROUND_LAYER};
use crate::trains::find_path_for_kind;
use crate::trains::{track_for_station, Train, TrainCargo, TrainLocation, TrainOnLine};

use super::payout::{GOODS_DELIVERY_CENTS, PASSENGER_FARE_CENTS};

/// Pending demand the player can fulfill with trains.
#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub kind: JobKind,
    pub reward_cents: i64,
}

/// Open jobs waiting for a train.
#[derive(Debug, Clone, Default, Resource)]
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
            // Shuttle empty along the line.
            if let Some(next_idx) = line.next_stop_index(on.next_stop, &mut on.forward) {
                let dest_station = line.stops[next_idx];
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
                    path_industries(&network, &industries, train.kind, from, to).is_some()
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
                    path_industries(network, industries, train.kind, from, to).is_some()
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

fn take_goods_job(
    board: &mut JobBoard,
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
    let Some(leg) = path_industries(network, industries, train.kind, from, to) else {
        board.jobs.push(Job {
            kind: JobKind::Goods { kind, from, to },
            reward_cents,
        });
        return false;
    };
    let Some(from_track) = industry_track(network, industries, from) else {
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

fn industry_track(
    network: &TrackNetwork,
    industries: &IndustryRegistry,
    id: IndustryId,
) -> Option<TrackId> {
    let ind = industries.get(id)?;
    track_for_station(network, ind.tile, GROUND_LAYER)
}

fn path_industries(
    network: &TrackNetwork,
    industries: &IndustryRegistry,
    kind: TrainKind,
    from: IndustryId,
    to: IndustryId,
) -> Option<Vec<TrackId>> {
    let a = industry_track(network, industries, from)?;
    let b = industry_track(network, industries, to)?;
    find_path_for_kind(network, a, b, kind)
}
