//! Train components and buy costs.

use bevy_ecs::prelude::{Component, Resource};
use serde::{Deserialize, Serialize};

use crate::commands::TrainKind;
use crate::ids::{LineId, StationId, TrackId, TrainId};
use crate::stations::{GoodKind, IndustryId};

use super::profile::TrainProfile;

/// Cost to buy a transit (passenger) train: **$3,000**.
pub const TRANSIT_COST_CENTS: i64 = 300_000;
/// Cost to buy a transport (goods) train: **$4,500**.
///
/// Half as much again as a transit, against a
/// [starting balance](crate::money::STARTING_CASH_CENTS) of $10,000 that also
/// has to cover track and platforms. A player who could afford their first
/// passenger train can very easily not afford their first goods train, which is
/// why [`apply_train_commands`](super::apply::apply_train_commands) says so out
/// loud rather than declining in silence.
pub const TRANSPORT_COST_CENTS: i64 = 450_000;

pub fn buy_cost(kind: TrainKind) -> i64 {
    match kind {
        TrainKind::Transit => TRANSIT_COST_CENTS,
        TrainKind::Transport => TRANSPORT_COST_CENTS,
    }
}

/// Cost to couple one more car to a transit: **$1,500** — half a train.
///
/// # Why half, and not less
///
/// A car and a second train are the two ways to move more people, and the
/// choice between them has to be a real one at the moment the player makes it.
/// A second train serves a **second pair**: new demand, its own route, its own
/// flexibility, and it keeps running when the first is held. A car **deepens
/// one pair**: it lifts the queue that is already forming at one platform, and
/// only while that queue is more than one carriage deep — at every other moment
/// it is weight ([`TrainProfile::for_consist`](super::TrainProfile::for_consist)).
///
/// Half the price of a train is what makes that trade honest. Cheaper, and the
/// car is a reflex — a player would buy one on the first line, where the board
/// never has two loads waiting, and simply run slower for it. Dearer, and it
/// could never beat a second train even on a line that badly wants one. At half
/// price the second car pays its own capital back in about the same time the
/// second train does *given a queue*, and never at all without one, which is
/// the state the opening beat is in.
///
/// See `docs/design/07-trains-and-lines.md` §3 for the measured comparison.
pub const TRANSIT_CAR_COST_CENTS: i64 = TRANSIT_COST_CENTS / 2;

/// Cost to couple one more wagon to a goods train: **$2,250**.
///
/// Priced by the same rule, and today unreachable: freight runs one wagon until
/// industries carry stock (see [`TRANSPORT_PROFILE`](super::TRANSPORT_PROFILE)).
/// It is a real number rather than a `None` so that the day the cap moves, the
/// price is already the one this rule implies.
pub const TRANSPORT_CAR_COST_CENTS: i64 = TRANSPORT_COST_CENTS / 2;

/// What one more car costs on a train of this kind.
pub fn car_cost(kind: TrainKind) -> i64 {
    match kind {
        TrainKind::Transit => TRANSIT_CAR_COST_CENTS,
        TrainKind::Transport => TRANSPORT_CAR_COST_CENTS,
    }
}

/// What a whole consist of `cars` cost to put together — the sale price.
///
/// Rolling stock is reversible in full (07 §5), and a consist is rolling stock:
/// selling a three-car transit has to hand back the train *and* both cars, or
/// the reversibility promise quietly stops applying to the upgrade.
pub fn consist_cost(kind: TrainKind, cars: u8) -> i64 {
    let extra = i64::from(cars.max(1).saturating_sub(1));
    buy_cost(kind).saturating_add(car_cost(kind).saturating_mul(extra))
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

/// How many cars a train runs, and how many of them are loaded.
///
/// # Why this is its own component
///
/// A train's composition is a separate fact from its identity, the same way its
/// [`TrainLocation`], its [`TrainCargo`] and its [`TrainOnLine`] are — and, like
/// `TrainOnLine`, **its absence is a true statement**: a train with no consist
/// is a single car, which is every train the game had before this existed. That
/// is what lets a hundred existing spawns keep saying `Train { id, kind }` and
/// mean exactly what they meant.
///
/// [`Self::laden`] rides here rather than on [`TrainCargo`] on purpose. The
/// cargo says *what working the train is on* — one origin, one destination, one
/// commodity — and that is unchanged by carrying three carloads of it. The
/// count of carloads is a property of the consist, and keeping it here leaves
/// the cargo enum the exact shape it has always been.
///
/// The invariant, held by [`Self::load`] and [`Self::unload`]: `laden` is `0`
/// whenever the cargo is [`TrainCargo::Empty`], and never exceeds `cars`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Component, Serialize, Deserialize)]
pub struct TrainConsist {
    /// Cars in the consist, including the locomotive's own. Never below 1.
    pub cars: u8,
    /// Loads aboard right now — all of them the same working.
    pub laden: u8,
}

impl Default for TrainConsist {
    fn default() -> Self {
        Self { cars: 1, laden: 0 }
    }
}

impl TrainConsist {
    /// A consist of `cars` cars, empty.
    pub fn of(cars: u8) -> Self {
        Self {
            cars: cars.max(1),
            laden: 0,
        }
    }

    /// Cars with nothing in them — how many more loads this train can take.
    #[inline]
    pub fn free_cars(self) -> u8 {
        self.cars.max(1).saturating_sub(self.laden)
    }

    /// Record `loads` boarding, never past the length of the train.
    pub fn load(&mut self, loads: u8) {
        self.laden = self.laden.saturating_add(loads).min(self.cars.max(1));
    }

    /// Everything aboard gets off; returns what did.
    pub fn unload(&mut self) -> u8 {
        std::mem::take(&mut self.laden)
    }
}

/// Cars a train is running, reading an absent consist as the single car it is.
#[inline]
pub fn cars_of(consist: Option<&TrainConsist>) -> u8 {
    consist.map(|c| c.cars.max(1)).unwrap_or(1)
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

    pub fn begin_dwell(&mut self, kind: TrainKind, cars: u8) {
        self.dwell_remaining = TrainProfile::for_kind(kind).for_consist(cars).dwell_ticks;
    }

    /// Dwell scaled by the platform actually stopped at — an interchange
    /// turns a train around at 60%, a halt boards at 150%. This is what makes
    /// a tier a service upgrade rather than a catchment number
    /// ([`StationTierSpec::dwell_percent`](crate::stations::StationTierSpec)).
    ///
    /// The consist is inside the figure the tier scales, which is where the two
    /// systems meet: a three-car transit boards in 8 ticks at a Station, 12 at a
    /// Halt and 5 at an Interchange. **A long train is what makes a better
    /// platform worth paying for**, and it is a cost rather than a cap — no tier
    /// refuses a consist, they only board them at different speeds.
    pub fn begin_dwell_at(
        &mut self,
        kind: TrainKind,
        cars: u8,
        tier: crate::stations::StationTier,
    ) {
        self.dwell_remaining =
            tier.dwell_ticks(TrainProfile::for_kind(kind).for_consist(cars).dwell_ticks);
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
