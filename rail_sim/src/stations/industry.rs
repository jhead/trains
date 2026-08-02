//! Industries that produce / consume goods for transport trains.

use std::collections::HashMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::ids::TileCoord;

/// Stable industry id (save / commands / jobs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IndustryId(pub u64);

/// Cargo types moved by transport trains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GoodKind {
    Lumber,
    Ore,
}

impl GoodKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Lumber => "lumber",
            Self::Ore => "ore",
        }
    }
}

/// A produce-and/or-consume site on land.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Industry {
    pub id: IndustryId,
    pub name: String,
    pub tile: TileCoord,
    pub produces: Option<GoodKind>,
    pub consumes: Option<GoodKind>,
}

/// Registry of industries.
#[derive(Debug, Clone, Default, Resource)]
pub struct IndustryRegistry {
    industries: HashMap<IndustryId, Industry>,
    by_tile: HashMap<(i32, i32), IndustryId>,
    next_id: u64,
}

impl IndustryRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.industries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.industries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Industry> {
        self.industries.values()
    }

    pub fn get(&self, id: IndustryId) -> Option<&Industry> {
        self.industries.get(&id)
    }

    pub fn at(&self, tile: TileCoord) -> Option<&Industry> {
        let id = *self.by_tile.get(&(tile.x, tile.y))?;
        self.industries.get(&id)
    }

    pub fn insert(
        &mut self,
        name: impl Into<String>,
        tile: TileCoord,
        produces: Option<GoodKind>,
        consumes: Option<GoodKind>,
    ) -> IndustryId {
        self.next_id = self.next_id.saturating_add(1);
        let id = IndustryId(self.next_id);
        let key = (tile.x, tile.y);
        assert!(
            !self.by_tile.contains_key(&key),
            "industry tile already occupied"
        );
        self.by_tile.insert(key, id);
        self.industries.insert(
            id,
            Industry {
                id,
                name: name.into(),
                tile,
                produces,
                consumes,
            },
        );
        id
    }

    /// First industry that produces `good`, if any.
    pub fn producer_of(&self, good: GoodKind) -> Option<&Industry> {
        self.industries
            .values()
            .find(|i| i.produces == Some(good))
    }

    /// First industry that consumes `good`, if any.
    pub fn consumer_of(&self, good: GoodKind) -> Option<&Industry> {
        self.industries
            .values()
            .find(|i| i.consumes == Some(good))
    }
}
