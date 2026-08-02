//! Station registry — named stops for passengers and train placement.

use std::collections::HashMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::ids::{StationId, TileCoord};

use super::tier::StationTier;

/// A named passenger stop on a map tile.
///
/// Trains place here when the tile (or an immediate neighbor) has track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Station {
    pub id: StationId,
    pub name: String,
    pub tile: TileCoord,
    pub layer: u8,
    /// Platform grade — drives catchment, dwell, capacity and cost.
    pub tier: StationTier,
    /// Cents spent on this stop so far; demolish refunds it in full.
    pub paid_cents: i64,
}

/// All stations keyed by id and tile.
#[derive(Debug, Clone, Default, Resource)]
pub struct StationRegistry {
    stations: HashMap<StationId, Station>,
    by_tile: HashMap<(i32, i32, u8), StationId>,
    next_id: u64,
}

impl StationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.stations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stations.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Station> {
        self.stations.values()
    }

    pub fn get(&self, id: StationId) -> Option<&Station> {
        self.stations.get(&id)
    }

    pub fn at(&self, tile: TileCoord, layer: u8) -> Option<&Station> {
        let id = *self.by_tile.get(&(tile.x, tile.y, layer))?;
        self.stations.get(&id)
    }

    pub fn id_at(&self, tile: TileCoord, layer: u8) -> Option<StationId> {
        self.by_tile.get(&(tile.x, tile.y, layer)).copied()
    }

    /// Insert a default-tier ([`StationTier::Station`]) stop with nothing paid.
    ///
    /// Auto-seeded anchors and revealed settlements use this; player builds go
    /// through [`StationRegistry::insert_tier`] so the spend is refundable.
    pub fn insert(&mut self, name: impl Into<String>, tile: TileCoord, layer: u8) -> StationId {
        self.insert_tier(name, tile, layer, StationTier::default(), 0)
    }

    /// Insert a station at a tier; panics if the tile is already occupied.
    ///
    /// Callers that can fail gracefully should check [`StationRegistry::at`]
    /// first — [`try_place_station`](super::place::try_place_station) does.
    pub fn insert_tier(
        &mut self,
        name: impl Into<String>,
        tile: TileCoord,
        layer: u8,
        tier: StationTier,
        paid_cents: i64,
    ) -> StationId {
        self.next_id = self.next_id.saturating_add(1);
        let id = StationId(self.next_id);
        let key = (tile.x, tile.y, layer);
        assert!(
            !self.by_tile.contains_key(&key),
            "station tile already occupied"
        );
        self.by_tile.insert(key, id);
        self.stations.insert(
            id,
            Station {
                id,
                name: name.into(),
                tile,
                layer,
                tier,
                paid_cents,
            },
        );
        id
    }

    /// Remove by id; refunding `paid_cents` is the caller's job.
    pub fn remove(&mut self, id: StationId) -> Option<Station> {
        let station = self.stations.remove(&id)?;
        self.by_tile
            .remove(&(station.tile.x, station.tile.y, station.layer));
        Some(station)
    }

    /// Retier in place, keeping the id (and therefore any line that stops here).
    ///
    /// `paid_cents` is the new running total spent on the stop.
    pub fn set_tier(&mut self, id: StationId, tier: StationTier, paid_cents: i64) -> bool {
        let Some(station) = self.stations.get_mut(&id) else {
            return false;
        };
        station.tier = tier;
        station.paid_cents = paid_cents;
        true
    }

    /// Closest station to `tile` by Chebyshev distance, skipping `except`.
    pub fn nearest(
        &self,
        tile: TileCoord,
        except: Option<StationId>,
    ) -> Option<(StationId, i32)> {
        self.stations
            .values()
            .filter(|s| Some(s.id) != except)
            .map(|s| {
                let dist = (s.tile.x - tile.x).abs().max((s.tile.y - tile.y).abs());
                (s.id, dist)
            })
            .min_by_key(|(id, dist)| (*dist, id.0))
    }
}
