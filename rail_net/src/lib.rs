//! Neighbour exchange — deliberately, aggressively dumb.
//!
//! `docs/design/12-multiplayer.md` §8: *"A blob store keyed by link identity.
//! Put a manifest, get a manifest. No authoritative server. No game logic
//! server-side."* This crate is that seam and nothing else.
//!
//! # What is here in MP-1
//!
//! Nothing that talks to a network, because MP-1 needs none. What is here is the
//! shape MP-2 slots into:
//!
//! - [`ManifestStore`] — put bytes, get bytes, keyed by [`LinkKey`]. It moves
//!   **opaque bytes**: the store does not know what a manifest is, cannot
//!   validate a railway, and should not try.
//! - [`OfflineStore`] — the default. Puts succeed and are discarded, gets return
//!   nothing. This is what "the game is fully playable with the endpoint unset"
//!   means in code, and it is the same store a failed request degrades to.
//! - [`MemoryStore`] — an in-process store, for tests and for a local two-map
//!   loopback.
//! - [`RefreshSchedule`] — the slow, jittered poll clock. Nothing is real-time,
//!   so nothing needs to be.
//!
//! # Why nothing here can block the player
//!
//! Every method returns immediately and the ordinary failure is `Ok`-shaped at
//! the call site: a missing manifest is `Ok(None)`, and the caller's answer to
//! `None` is always "keep using the cached neighbour" (`rail_sim::border` holds
//! that cache). There is no method on this trait a caller could reasonably
//! await, which is §2.1 enforced by the signature rather than by discipline.
//!
//! # The manifest itself
//!
//! `rail_sim::border::BorderManifest` is the payload, and it deliberately does
//! not appear in this crate: the store is generic over bytes so the simulation's
//! types and the transport can never grow into each other. Encode on the way in;
//! decode and clamp on the way out with `BorderManifest::sanitised`.

use std::collections::HashMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Opaque id for a neighbour link / edge portal pair.
///
/// Numerically the same value as `rail_sim::border::LinkId`; kept as its own
/// type so the transport never depends on the simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NeighborLink(pub u64);

/// Preferred name for the blob store's key.
pub type LinkKey = NeighborLink;

/// Cargo / passenger payload crossing a portal (shape only for now).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargoManifest {
    pub link: NeighborLink,
    pub train_kind_tag: u8,
    pub payload_units: u32,
}

/// Something waiting in the inbox from a neighbour (or nothing, for null).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NeighborMessage {
    TrainArriving(CargoManifest),
}

/// Edge handoff backend. MVP: [`NullNeighbor`].
pub trait NeighborBackend: Send + Sync + 'static {
    /// Drain any inbound neighbour messages for this tick (may be empty).
    fn poll_inbox(&mut self) -> Vec<NeighborMessage>;

    /// Offer a train/cargo to a neighbour. Returns false if not accepted.
    fn send_train(&mut self, manifest: CargoManifest) -> bool;
}

/// In-process null neighbour: inbox always empty, sends always accepted as no-ops.
#[derive(Debug, Default, Resource)]
pub struct NullNeighbor;

impl NeighborBackend for NullNeighbor {
    fn poll_inbox(&mut self) -> Vec<NeighborMessage> {
        Vec::new()
    }

    fn send_train(&mut self, _manifest: CargoManifest) -> bool {
        // Accept and discard — single-player is never blocked by a missing neighbour.
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

// ---------------------------------------------------------------------------
// Blob store
// ---------------------------------------------------------------------------

/// Why a store call could not be served.
///
/// There is no `WouldBlock` and no `Pending`, because there is no state in which
/// the player waits. A store that is unreachable is simply
/// [`StoreError::Unavailable`], and the caller falls back to its cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// No endpoint configured, or the endpoint could not be reached.
    Unavailable,
    /// The blob was there but is not usable (truncated, oversized, …).
    Corrupt,
}

pub type StoreResult<T> = Result<T, StoreError>;

/// Largest blob a store will accept or return.
///
/// A border manifest is kilobytes (see `rail_sim::border::manifest`), so
/// anything approaching this is either a bug or an attempt to make the store do
/// something it is not for.
pub const MAX_BLOB_BYTES: usize = 64 * 1024;

/// Put a manifest, get a manifest. That is the entire protocol.
///
/// Implementations must never block for longer than a local call: MP-2's HTTP
/// store is expected to serve [`get`](ManifestStore::get) from a local cache and
/// refresh it out of band on [`RefreshSchedule`].
pub trait ManifestStore: Send + Sync + 'static {
    /// Publish our side of a link. Overwrites whatever was there.
    fn put(&mut self, key: LinkKey, blob: &[u8]) -> StoreResult<()>;

    /// Fetch their side of a link, if anything has ever been published.
    ///
    /// `Ok(None)` is the ordinary answer for "nobody has written this yet" and
    /// must never be treated as an error by the caller.
    fn get(&mut self, key: LinkKey) -> StoreResult<Option<Vec<u8>>>;

    /// Whether this store can currently reach anything at all.
    ///
    /// Purely informational — for the Neighbours panel to say "offline", never
    /// for gating gameplay.
    fn is_online(&self) -> bool {
        false
    }
}

/// The default store: accepts everything, remembers nothing, serves nothing.
///
/// This is what an unset endpoint gets, and it is also the correct degraded
/// behaviour for a failed request. A game running against this store trades
/// entirely with echo neighbours — which is MP-1 in full.
#[derive(Debug, Default, Clone, Copy)]
pub struct OfflineStore;

impl ManifestStore for OfflineStore {
    fn put(&mut self, _key: LinkKey, _blob: &[u8]) -> StoreResult<()> {
        Ok(())
    }

    fn get(&mut self, _key: LinkKey) -> StoreResult<Option<Vec<u8>>> {
        Ok(None)
    }
}

/// An in-process store: two maps in the same build can trade through it.
#[derive(Debug, Default, Clone)]
pub struct MemoryStore {
    blobs: HashMap<LinkKey, Vec<u8>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.blobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blobs.is_empty()
    }
}

impl ManifestStore for MemoryStore {
    fn put(&mut self, key: LinkKey, blob: &[u8]) -> StoreResult<()> {
        if blob.len() > MAX_BLOB_BYTES {
            return Err(StoreError::Corrupt);
        }
        self.blobs.insert(key, blob.to_vec());
        Ok(())
    }

    fn get(&mut self, key: LinkKey) -> StoreResult<Option<Vec<u8>>> {
        Ok(self.blobs.get(&key).cloned())
    }

    fn is_online(&self) -> bool {
        true
    }
}

/// Bevy resource holding whichever store the app was built with.
///
/// `rail_town` inserts [`ManifestService::offline`]; MP-2 swaps it for an
/// HTTP-backed one and nothing else in the codebase changes.
#[derive(Resource)]
pub struct ManifestService(pub Box<dyn ManifestStore>);

impl ManifestService {
    pub fn offline() -> Self {
        Self(Box::new(OfflineStore))
    }

    pub fn memory() -> Self {
        Self(Box::new(MemoryStore::new()))
    }
}

impl Default for ManifestService {
    fn default() -> Self {
        Self::offline()
    }
}

// ---------------------------------------------------------------------------
// Refresh schedule
// ---------------------------------------------------------------------------

/// Seconds between exchange attempts, before jitter.
pub const REFRESH_INTERVAL_SECS: u32 = 300;
/// Jitter spread, in seconds, so a thousand clients do not knock together.
pub const REFRESH_JITTER_SECS: u32 = 120;

/// The slow, jittered poll clock (§8).
///
/// Deterministic: the jitter is a function of the link key and the attempt
/// count, never of wall-clock randomness, so a replay of the same session asks
/// at the same moments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Resource)]
pub struct RefreshSchedule {
    pub interval_secs: u32,
    pub jitter_secs: u32,
}

impl Default for RefreshSchedule {
    fn default() -> Self {
        Self {
            interval_secs: REFRESH_INTERVAL_SECS,
            jitter_secs: REFRESH_JITTER_SECS,
        }
    }
}

impl RefreshSchedule {
    /// Seconds to wait before attempt `attempt` on `key`.
    pub fn delay_secs(&self, key: LinkKey, attempt: u32) -> u32 {
        if self.jitter_secs == 0 {
            return self.interval_secs;
        }
        let mut h = key.0 ^ u64::from(attempt).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
        self.interval_secs + (h % u64::from(self.jitter_secs)) as u32
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

    /// The unset-endpoint case, which is the shipping default: publishing
    /// succeeds, fetching finds nothing, and neither is an error the caller has
    /// to handle specially.
    #[test]
    fn an_offline_store_succeeds_at_doing_nothing() {
        let mut store = OfflineStore;
        assert_eq!(store.put(NeighborLink(7), b"anything"), Ok(()));
        assert_eq!(store.get(NeighborLink(7)), Ok(None));
        assert!(!store.is_online());
    }

    #[test]
    fn a_memory_store_round_trips_bytes() {
        let mut store = MemoryStore::new();
        assert_eq!(store.get(NeighborLink(1)), Ok(None));
        store.put(NeighborLink(1), b"manifest").expect("put");
        assert_eq!(store.get(NeighborLink(1)), Ok(Some(b"manifest".to_vec())));
        // A second put replaces, so replay is idempotent at the store too.
        store.put(NeighborLink(1), b"newer").expect("put");
        assert_eq!(store.get(NeighborLink(1)), Ok(Some(b"newer".to_vec())));
        assert_eq!(store.len(), 1);
        assert!(store.is_online());
    }

    #[test]
    fn an_absurd_blob_is_refused_rather_than_stored() {
        let mut store = MemoryStore::new();
        let huge = vec![0u8; MAX_BLOB_BYTES + 1];
        assert_eq!(store.put(NeighborLink(1), &huge), Err(StoreError::Corrupt));
        assert_eq!(store.get(NeighborLink(1)), Ok(None));
    }

    #[test]
    fn the_service_defaults_to_offline() {
        let mut service = ManifestService::default();
        assert!(!service.0.is_online());
        assert_eq!(service.0.get(NeighborLink(3)), Ok(None));
        assert_eq!(service.0.put(NeighborLink(3), b"ours"), Ok(()));
    }

    #[test]
    fn refresh_is_slow_and_jittered_but_reproducible() {
        let schedule = RefreshSchedule::default();
        let a = schedule.delay_secs(NeighborLink(1), 0);
        let b = schedule.delay_secs(NeighborLink(2), 0);
        assert_eq!(a, schedule.delay_secs(NeighborLink(1), 0), "deterministic");
        assert_ne!(a, b, "two links do not knock at the same moment");
        for link in 0..64u64 {
            for attempt in 0..4u32 {
                let d = schedule.delay_secs(NeighborLink(link), attempt);
                assert!(d >= REFRESH_INTERVAL_SECS);
                assert!(d < REFRESH_INTERVAL_SECS + REFRESH_JITTER_SECS);
            }
        }
    }
}
