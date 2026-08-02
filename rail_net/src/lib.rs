//! Neighbor exchange API.
//!
//! Multiplayer / async neighbor maps are out of MVP scope. The
//! [`NeighborBackend`] trait is the seam; [`NullNeighbor`] is the in-process
//! backend that never blocks single-player.
//!
//! `rail_town` inserts a backend as a Bevy [`Resource`](bevy_ecs::prelude::Resource)
//! (see `NeighborBackendResource` / `NullNeighbor` wiring in the app).

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Opaque id for a neighbor link / edge portal pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NeighborLink(pub u64);

/// Cargo / passenger payload crossing a portal (shape only for now).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargoManifest {
    pub link: NeighborLink,
    pub train_kind_tag: u8,
    pub payload_units: u32,
}

/// Something waiting in the inbox from a neighbor (or nothing, for null).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NeighborMessage {
    TrainArriving(CargoManifest),
}

/// Edge handoff backend. MVP: [`NullNeighbor`].
pub trait NeighborBackend: Send + Sync + 'static {
    /// Drain any inbound neighbor messages for this tick (may be empty).
    fn poll_inbox(&mut self) -> Vec<NeighborMessage>;

    /// Offer a train/cargo to a neighbor. Returns false if not accepted.
    fn send_train(&mut self, manifest: CargoManifest) -> bool;
}

/// In-process null neighbor: inbox always empty, sends always accepted as no-ops.
#[derive(Debug, Default, Resource)]
pub struct NullNeighbor;

impl NeighborBackend for NullNeighbor {
    fn poll_inbox(&mut self) -> Vec<NeighborMessage> {
        Vec::new()
    }

    fn send_train(&mut self, _manifest: CargoManifest) -> bool {
        // Accept and discard — single-player is never blocked by a missing neighbor.
        true
    }
}

/// Bevy resource wrapper so the app can hold any backend behind the trait object.
///
/// `rail_town` inserts `NeighborService(Box::new(NullNeighbor))` at startup.
#[derive(Resource)]
pub struct NeighborService(pub Box<dyn NeighborBackend>);

impl NeighborService {
    pub fn null() -> Self {
        Self(Box::new(NullNeighbor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_neighbor_never_blocks() {
        let mut n = NullNeighbor;
        assert!(n.poll_inbox().is_empty());
        assert!(n.send_train(CargoManifest {
            link: NeighborLink(1),
            train_kind_tag: 0,
            payload_units: 3,
        }));
    }
}
