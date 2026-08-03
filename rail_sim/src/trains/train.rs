//! Train components and buy costs.

use bevy_ecs::prelude::{Component, Resource};
use serde::{Deserialize, Serialize};

use crate::commands::TrainKind;
use crate::ids::{LineId, StationId, TrackId, TrainId};
use crate::stations::{GoodKind, IndustryId};

use super::profile::TrainProfile;

/// Cost to buy a transit (passenger) train: $500.
pub const TRANSIT_COST_CENTS: i64 = 300_000;
/// Cost to buy a transport (goods) train: $750.
pub const TRANSPORT_COST_CENTS: i64 = 450_000;

pub fn buy_cost(kind: TrainKind) -> i64 {
    match kind {
        TrainKind::Transit => TRANSIT_COST_CENTS,
        TrainKind::Transport => TRANSPORT_COST_CENTS,
    }
}

/// Bought trains waiting to be placed at a station.
#[derive(Debug, Clone, Default, PartialEq, Resource, Serialize, Deserialize)]
pub struct TrainYard {
    next_id: u64,
    /// FIFO unplaced stock.
    unplaced: Vec<(TrainId, TrainKind)>,
}

impl TrainYard {
    pub fn buy(&mut self, kind: TrainKind) -> TrainId {
        self.next_id = self.next_id.saturating_add(1);
        let id = TrainId(self.next_id);
        self.unplaced.push((id, kind));
        id
    }

    pub fn unplaced(&self) -> &[(TrainId, TrainKind)] {
        &self.unplaced
    }

    pub fn peek_kind(&self, kind: TrainKind) -> Option<TrainId> {
        self.unplaced
            .iter()
            .find(|(_, k)| *k == kind)
            .map(|(id, _)| *id)
    }

    /// Take a specific unplaced train, or `None` if not in the yard.
    pub fn take(&mut self, id: TrainId) -> Option<TrainKind> {
        let idx = self.unplaced.iter().position(|(t, _)| *t == id)?;
        Some(self.unplaced.remove(idx).1)
    }

    /// Take the oldest unplaced train of `kind`.
    pub fn take_kind(&mut self, kind: TrainKind) -> Option<(TrainId, TrainKind)> {
        let idx = self.unplaced.iter().position(|(_, k)| *k == kind)?;
        Some(self.unplaced.remove(idx))
    }

    /// Put a train back in the yard (failed place).
    pub fn return_train(&mut self, id: TrainId, kind: TrainKind) {
        self.unplaced.push((id, kind));
    }
}

/// Core train identity on an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Component, Serialize, Deserialize)]
pub struct Train {
    pub id: TrainId,
    pub kind: TrainKind,
}

/// Position along the track graph + remaining path.
#[derive(Debug, Clone, PartialEq, Eq, Component, Serialize, Deserialize)]
pub struct TrainLocation {
    pub track: TrackId,
    /// Full path including current tile; `path_index` points at `track`.
    pub path: Vec<TrackId>,
    pub path_index: usize,
    /// Progress toward the next tile (0..ticks_needed).
    pub progress: u16,
    /// Soft-parked when opex can't be paid — still occupies tile, doesn't move.
    pub parked: bool,
    /// Remaining dwell ticks at current stop (blocks new jobs / movement).
    pub dwell_remaining: u16,
}

impl TrainLocation {
    pub fn at_track(track: TrackId) -> Self {
        Self {
            track,
            path: vec![track],
            path_index: 0,
            progress: 0,
            parked: false,
            dwell_remaining: 0,
        }
    }

    pub fn set_path(&mut self, path: Vec<TrackId>) {
        if let Some(pos) = path.iter().position(|t| *t == self.track) {
            self.path = path;
            self.path_index = pos;
        } else if let Some(&first) = path.first() {
            // Path doesn't include us — snap to start (caller should pathfind from here).
            self.track = first;
            self.path = path;
            self.path_index = 0;
        }
        self.progress = 0;
    }

    /// Replace the route from the current tile onward, keeping the travelled
    /// prefix and `path_index`.
    ///
    /// `ahead[0]` must be the current tile. Unlike [`Self::set_path`] this never
    /// re-searches for our position, so a detour route may legitimately repeat a
    /// tile (duck into a passing loop and come back) without rewinding the train.
    /// Progress into the current tile is kept: a train that has already earned
    /// its crossing time leaves as soon as the new next tile is free.
    pub fn set_route_ahead(&mut self, ahead: Vec<TrackId>) {
        if ahead.first() != Some(&self.track) {
            return;
        }
        self.path.truncate(self.path_index);
        self.path.extend(ahead);
    }

    pub fn destination(&self) -> Option<TrackId> {
        self.path.last().copied()
    }

    pub fn at_destination(&self) -> bool {
        self.path_index + 1 >= self.path.len()
    }

    pub fn begin_dwell(&mut self, kind: TrainKind) {
        self.dwell_remaining = TrainProfile::for_kind(kind).dwell_ticks;
    }

    /// Dwell scaled by the platform actually stopped at — an interchange
    /// turns a train around at 60%, a halt boards at 150%. This is what makes
    /// a tier a service upgrade rather than a catchment number
    /// ([`StationTierSpec::dwell_percent`](crate::stations::StationTierSpec)).
    pub fn begin_dwell_at(&mut self, kind: TrainKind, tier: crate::stations::StationTier) {
        self.dwell_remaining = tier.dwell_ticks(TrainProfile::for_kind(kind).dwell_ticks);
    }
}

/// Train assigned to a player line — prefers line jobs / shuttle over free-roam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Component, Serialize, Deserialize)]
pub struct TrainOnLine {
    pub line: LineId,
    /// Index into the line's stop list we are heading toward (or last arrived).
    pub next_stop: usize,
    /// Out-and-back direction.
    pub forward: bool,
}

/// What the train is carrying (if anything).
#[derive(Debug, Clone, PartialEq, Eq, Component, Serialize, Deserialize)]
pub enum TrainCargo {
    Empty,
    Passengers {
        from: StationId,
        to: StationId,
    },
    Goods {
        kind: GoodKind,
        from: IndustryId,
        to: IndustryId,
    },
}

impl Default for TrainCargo {
    fn default() -> Self {
        Self::Empty
    }
}

impl TrainCargo {
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }
}
