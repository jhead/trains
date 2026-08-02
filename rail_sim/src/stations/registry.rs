//! Station registry — named stops for passengers and train placement.

use std::collections::HashMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::ids::{StationId, TileCoord};

/// A named passenger stop on a map tile.
///
/// Trains place here when the tile (or an immediate neighbor) has track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Station {
    pub id: StationId,
    pub name: String,
    pub tile: TileCoord,
    pub layer: u8,
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

    /// Insert a station; panics if the tile is already occupied by another station.
    pub fn insert(&mut self, name: impl Into<String>, tile: TileCoord, layer: u8) -> StationId {
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
            },
        );
        id
    }
}
