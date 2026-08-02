//! Saving without a hitch: snapshot on the sim thread, encode and write off it.
//!
//! `09-shell-and-menus.md` §6 is blunt about this — *"a calm game that stutters
//! every three minutes is not calm"*. So a save splits in two:
//!
//! 1. [`WorldSnapshot::capture`] runs where the world is. It clones resources
//!    and walks two small queries; nothing serialises, nothing touches disk.
//! 2. Encoding (bincode + checksum) and the file write happen on a worker
//!    thread. The sim never waits for either.
//!
//! On wasm there are no threads, so step 2 runs inline — writing to browser
//! storage is a memcpy, not a disk seek, and the handle comes back already
//! finished. The calling code is identical either way.

use bevy_app::{App, Plugin, Update};
use bevy_ecs::prelude::*;

use super::codec::SaveMeta;
use super::error::{SaveError, SaveResult};
use super::slots::{next_autosave_slot, write_snapshot, SaveSlot, SlotInfo};
use super::snapshot::WorldSnapshot;

/// A save in flight. Dropping it lets the write finish in the background.
#[derive(Debug)]
pub struct SaveHandle {
    slot: SaveSlot,
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<std::thread::JoinHandle<SaveResult<SlotInfo>>>,
    done: Option<SaveResult<SlotInfo>>,
}

impl SaveHandle {
    /// Which slot this save is heading for.
    pub fn slot(&self) -> &SaveSlot {
        &self.slot
    }

    /// `true` once the bytes are written (successfully or not).
    pub fn is_finished(&self) -> bool {
        if self.done.is_some() {
            return true;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            return self.worker.as_ref().is_some_and(|w| w.is_finished());
        }
        #[cfg(target_arch = "wasm32")]
        {
            true
        }
    }

    /// Take the outcome once it is ready. `None` means still writing.
    ///
    /// Never blocks — poll it from a normal `Update` system.
    pub fn take_result(&mut self) -> Option<SaveResult<SlotInfo>> {
        if let Some(result) = self.done.take() {
            return Some(result);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let finished = self.worker.as_ref().is_some_and(|w| w.is_finished());
            if finished {
                return self.worker.take().map(join_worker);
            }
        }
        None
    }

    /// Block until the save finishes. For quit-time saves only.
    pub fn wait(mut self) -> SaveResult<SlotInfo> {
        if let Some(result) = self.done.take() {
            return result;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(worker) = self.worker.take() {
                return join_worker(worker);
            }
        }
        Err(SaveError::Busy)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn join_worker(worker: std::thread::JoinHandle<SaveResult<SlotInfo>>) -> SaveResult<SlotInfo> {
    worker
        .join()
        .unwrap_or_else(|_| Err(SaveError::Encode("the save worker panicked".into())))
}

/// Encode and write `snapshot` off the calling thread.
pub fn write_snapshot_async(
    snapshot: WorldSnapshot,
    meta: SaveMeta,
    slot: SaveSlot,
) -> SaveHandle {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let worker_slot = slot.clone();
        let spawned = std::thread::Builder::new()
            .name("rail-town-save".to_string())
            .spawn({
                let snapshot = snapshot.clone();
                let meta = meta.clone();
                move || write_snapshot(&snapshot, meta, &worker_slot)
            });
        match spawned {
            Ok(worker) => SaveHandle {
                slot,
                worker: Some(worker),
                done: None,
            },
            // No thread available — better a brief write than a lost save.
            Err(_) => {
                let done = write_snapshot(&snapshot, meta, &slot);
                SaveHandle {
                    slot,
                    worker: None,
                    done: Some(done),
                }
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let done = write_snapshot(&snapshot, meta, &slot);
        SaveHandle {
            slot,
            done: Some(done),
        }
    }
}

/// Snapshot `world` now, write it in the background.
pub fn save_to_slot_async(world: &World, slot: &SaveSlot) -> SaveHandle {
    let snapshot = WorldSnapshot::capture(world);
    let meta = SaveMeta::from_snapshot(&snapshot, slot.display_name());
    write_snapshot_async(snapshot, meta, slot.clone())
}

/// Snapshot `world` now, autosave it in the background.
pub fn autosave_async(world: &World) -> SaveHandle {
    let snapshot = WorldSnapshot::capture(world);
    let (slot, ordinal) = next_autosave_slot();
    let mut meta = SaveMeta::from_snapshot(&snapshot, slot.display_name());
    meta.ordinal = ordinal;
    write_snapshot_async(snapshot, meta, slot)
}

/// Background saves the app is waiting on.
///
/// [`SavePlugin`] polls this every frame and collects finished writes, so the
/// shell only has to read [`SaveJobs::drain_completed`] when it wants to show
/// "Saved." or an error.
#[derive(Debug, Default, Resource)]
pub struct SaveJobs {
    in_flight: Vec<SaveHandle>,
    completed: Vec<SaveResult<SlotInfo>>,
}

impl SaveJobs {
    /// Track a handle so it is polled and its result collected.
    pub fn track(&mut self, handle: SaveHandle) {
        self.in_flight.push(handle);
    }

    /// `true` while any save is still writing.
    pub fn is_busy(&self) -> bool {
        !self.in_flight.is_empty()
    }

    /// `true` when a save to this slot is already running.
    pub fn is_saving(&self, slot: &SaveSlot) -> bool {
        self.in_flight.iter().any(|h| h.slot() == slot)
    }

    /// Move finished saves into `completed`. Called for you by [`SavePlugin`].
    pub fn collect_finished(&mut self) {
        let mut still_running = Vec::with_capacity(self.in_flight.len());
        for mut handle in self.in_flight.drain(..) {
            match handle.take_result() {
                Some(result) => self.completed.push(result),
                None => still_running.push(handle),
            }
        }
        self.in_flight = still_running;
    }

    /// Take the outcomes of saves that finished since the last call.
    pub fn drain_completed(&mut self) -> Vec<SaveResult<SlotInfo>> {
        std::mem::take(&mut self.completed)
    }
}

/// Capture `world` and queue a background save. Call from an exclusive system.
///
/// Refuses a second save to the same slot while one is running, so a held
/// hotkey cannot pile up writes to one file.
pub fn queue_save(world: &mut World, slot: SaveSlot) -> SaveResult<()> {
    ensure_jobs(world);
    if world.resource::<SaveJobs>().is_saving(&slot) {
        return Err(SaveError::Busy);
    }
    let handle = save_to_slot_async(world, &slot);
    world.resource_mut::<SaveJobs>().track(handle);
    Ok(())
}

/// Capture `world` and queue a background autosave.
pub fn queue_autosave(world: &mut World) -> SaveResult<()> {
    ensure_jobs(world);
    if world.resource::<SaveJobs>().is_busy() {
        return Err(SaveError::Busy);
    }
    let handle = autosave_async(world);
    world.resource_mut::<SaveJobs>().track(handle);
    Ok(())
}

fn ensure_jobs(world: &mut World) {
    if world.get_resource::<SaveJobs>().is_none() {
        world.insert_resource(SaveJobs::default());
    }
}

/// Registers [`SaveJobs`] and the system that reaps finished background saves.
pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SaveJobs>()
            .add_systems(Update, collect_finished_saves);
    }
}

fn collect_finished_saves(mut jobs: ResMut<SaveJobs>) {
    if jobs.is_busy() {
        jobs.collect_finished();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::slots::{delete_slot, load_from_slot};
    use crate::save::storage::use_test_root;

    #[test]
    fn a_background_save_completes_and_is_readable() {
        use_test_root();
        let slot = SaveSlot::Named("background write".into());
        let _ = delete_slot(&slot);

        let world = World::new();
        let mut handle = save_to_slot_async(&world, &slot);

        // Poll rather than sleep — this is exactly what the shell does.
        let mut result = None;
        for _ in 0..10_000 {
            if let Some(r) = handle.take_result() {
                result = Some(r);
                break;
            }
            std::thread::yield_now();
        }
        let info = result.expect("save finished").expect("save succeeded");
        assert_eq!(info.slot, slot);

        let loaded = load_from_slot(&slot).expect("load");
        assert_eq!(loaded.schema_version, super::super::SCHEMA_VERSION);
        let _ = delete_slot(&slot);
    }

    #[test]
    fn jobs_collect_finished_writes() {
        use_test_root();
        let slot = SaveSlot::Named("background jobs".into());
        let _ = delete_slot(&slot);

        let mut world = World::new();
        queue_save(&mut world, slot.clone()).expect("queued");
        assert!(world.resource::<SaveJobs>().is_busy());

        for _ in 0..10_000 {
            world.resource_mut::<SaveJobs>().collect_finished();
            if !world.resource::<SaveJobs>().is_busy() {
                break;
            }
            std::thread::yield_now();
        }

        let done = world.resource_mut::<SaveJobs>().drain_completed();
        assert_eq!(done.len(), 1);
        assert!(done[0].is_ok(), "{done:?}");
        let _ = delete_slot(&slot);
    }

    #[test]
    fn capture_does_not_need_a_mutable_world() {
        // Compile-time proof that a save can be taken from a shared reference:
        // the sim keeps running while the bytes are written.
        fn takes_shared(world: &World) -> WorldSnapshot {
            WorldSnapshot::capture(world)
        }
        let world = World::new();
        let snapshot = takes_shared(&world);
        assert_eq!(snapshot.schema_version, super::super::SCHEMA_VERSION);
    }
}
