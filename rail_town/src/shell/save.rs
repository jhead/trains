//! Save / load call sites for the shell.
//!
//! **The shell does not implement saving.** Persisting a world is `rail_sim`'s
//! job (design §6: a save is the map, network, lines, trains, town and peeps with
//! their names and histories). This module is only the seam: the menus name a
//! slot, this routes it, and [`SaveStatus`] carries the answer back to the panel.
//!
//! # How it reaches `rail_sim::save`
//!
//! Saving and loading both need `&mut World`, so the menus never call them
//! directly. A button writes a [`ShellSaveRequest`], and the exclusive system
//! [`service_save_requests`] performs it on the next `Update`. That keeps the
//! menu code as ordinary systems and keeps the exclusive access to one place.
//!
//! Manual saves go through [`rail_sim::save::queue_save`] and autosaves through
//! [`rail_sim::save::queue_autosave`], both of which encode and write on a
//! background task — design §6 is explicit that saving must never cost the sim a
//! frame. Loading is synchronous by nature: there is nothing to keep running.
//!
//! Restoring replaces the world's contents but not its *art*: terrain chunks are
//! composited from [`rail_map::MapGrid`], so a load has to put the right grid
//! back and mark them dirty. The grid is rebuilt here by
//! [`regenerate_map_from_save`]; the dirty flag is raised outside this module —
//! see [`super::PendingWorld`] and the `WorldRebuildSet` docs.

use bevy::prelude::*;
use rail_sim::save::{self, SaveSlot, SlotInfo};
use rail_sim::MapDescriptor;

/// What the shell last heard back from the save layer. Drives the one-line
/// confirmation under the pause menu, so a save is visibly acknowledged.
#[derive(Resource, Debug, Default, Clone)]
pub struct SaveStatus {
    pub message: Option<String>,
}

impl SaveStatus {
    pub fn set(&mut self, message: impl Into<String>) {
        self.message = Some(message.into());
    }

    pub fn clear(&mut self) {
        self.message = None;
    }
}

/// A save or load the menus asked for, waiting for [`service_save_requests`].
#[derive(Resource, Debug, Default, Clone)]
pub struct ShellSaveRequest {
    pub save: Option<SaveSlot>,
    pub load: Option<SaveSlot>,
    /// Set when the pending load succeeds, so the shell knows to rebuild the world.
    pub loaded: bool,
}

/// Autosave countdown, driven by `Settings::gameplay.autosave_minutes`.
#[derive(Resource, Debug, Clone, Default)]
pub struct AutosaveTimer {
    elapsed_secs: f32,
}

impl AutosaveTimer {
    pub fn reset(&mut self) {
        self.elapsed_secs = 0.0;
    }

    /// Advance and report whether an autosave is due.
    pub fn tick(&mut self, delta_secs: f32, interval_minutes: u32) -> bool {
        if interval_minutes == 0 {
            self.elapsed_secs = 0.0;
            return false;
        }
        self.elapsed_secs += delta_secs;
        let interval = interval_minutes as f32 * 60.0;
        if self.elapsed_secs >= interval {
            self.elapsed_secs = 0.0;
            return true;
        }
        false
    }
}

/// Every slot, newest first. An unreadable file is skipped, never fatal.
pub fn slots() -> Vec<SlotInfo> {
    save::list_slots().unwrap_or_default()
}

/// Newest save, or `None` when the player has never saved. Powers Continue.
pub fn newest_slot() -> Option<SlotInfo> {
    slots().into_iter().next()
}

/// Ask for a manual save. `None` means "autosave rotation".
pub fn request_save(request: &mut ShellSaveRequest, slot: Option<SaveSlot>) {
    request.save = Some(slot.unwrap_or(SaveSlot::Quick));
}

/// Ask for a slot to be restored.
pub fn request_load(request: &mut ShellSaveRequest, slot: SaveSlot) {
    request.load = Some(slot);
}

/// Perform whatever the menus asked for.
///
/// Exclusive because both halves of the save API need `&mut World`. It does no
/// work at all on a frame with nothing pending, so the exclusive access costs
/// nothing in the common case.
pub fn service_save_requests(world: &mut World) {
    let Some(mut request) = world.get_resource_mut::<ShellSaveRequest>() else {
        return;
    };
    let (save, load) = (request.save.take(), request.load.take());
    if save.is_none() && load.is_none() {
        return;
    }

    let mut message = None;
    let mut loaded = false;

    if let Some(slot) = save {
        message = Some(match queue(world, &slot) {
            Ok(()) => format!("Saving {}...", slot.display_name()),
            Err(err) => describe(&err, "save"),
        });
    }

    if let Some(slot) = load {
        message = Some(match save::load_from_slot(&slot) {
            Ok(snapshot) => {
                let report = snapshot.restore(world);
                regenerate_map_from_save(world);
                loaded = true;
                describe_restore(&slot, &report)
            }
            Err(err) => describe(&err, "load"),
        });
    }

    if let Some(mut request) = world.get_resource_mut::<ShellSaveRequest>() {
        request.loaded = loaded;
    }
    if let Some(mut status) = world.get_resource_mut::<SaveStatus>() {
        match message {
            Some(text) => status.set(text),
            None => status.clear(),
        }
    }
}

/// Put the loaded world's map back, generated from the seed and knobs it saved.
///
/// A save stores the terrain the sim plays on but not the [`rail_map::MapGrid`]
/// it came from — that would be the same tiles twice, and the grid carries the
/// generator's own notes (sites, river crossings, ridge passes) besides. It
/// stores the seed and the packed generator options instead, which is enough to
/// make the same map again. That is design 02 §5's promise — a seed and its
/// settings reproduce a world — being spent rather than merely stated.
///
/// The restored `TrackTerrain` is left exactly as the save wrote it. The grid is
/// art and generator notes; the terrain is what the player built on, and if the
/// generator ever moves under a save the world must still be the one that was
/// saved. Only the look would drift, which is what `GENERATOR_VERSION` records.
fn regenerate_map_from_save(world: &mut World) {
    let Some(descriptor) = world.get_resource::<MapDescriptor>().copied() else {
        return;
    };
    // No knobs means the save never recorded how its world was made. Guessing a
    // setup would hand the player a different map than the one they saved, so
    // the grid already on screen stays.
    let Some(options) = descriptor.gen.knobs.and_then(rail_map::MapGenOptions::unpack) else {
        return;
    };
    if descriptor.width < 2 || descriptor.height < 2 {
        return;
    }
    // Sized from the descriptor rather than from the packed `size`, so a world
    // that was never square — a test map, a neighbour's chunk — comes back at
    // its own dimensions instead of being squared off.
    let map = rail_map::generate_map_with(
        descriptor.width,
        descriptor.height,
        descriptor.seed,
        options,
    );
    // The projection's height field belongs to the map that is installed, and
    // this is the moment it changes. Waiting for the next frame's
    // `map::projection::follow_map_heights` is too late: the restored
    // `TrackNetwork` went in a few lines above, and the track rebuild it
    // triggers runs later in *this* `Update` — placing every piece of the
    // loaded railway at the previous world's elevation, permanently, because a
    // track sprite is positioned once and then left alone.
    crate::map::projection::set_iso_heights(&map);
    world.insert_resource(map);
}

/// Serialises the tests that point the save root somewhere of their own.
///
/// `rail_sim::save::set_save_root` writes a process-global, and Rust runs a
/// crate's tests in parallel — so two tests that each set it will straddle each
/// other's writes and one of them will save into one directory and look for it
/// in another. Take this for as long as a test needs the root to hold still.
#[cfg(test)]
pub(crate) static SAVE_ROOT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Point the save root at a directory of this test's own, and hold it there.
#[cfg(test)]
pub(crate) fn lock_save_root(name: &str) -> std::sync::MutexGuard<'static, ()> {
    let guard = SAVE_ROOT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    save::set_save_root(
        std::env::temp_dir().join(format!("rail_town_{name}_{}", std::process::id())),
    );
    guard
}

/// Quick and named slots are explicit writes; the autosave slot rotates itself.
fn queue(world: &mut World, slot: &SaveSlot) -> save::SaveResult<()> {
    match slot {
        SaveSlot::Auto(_) => save::queue_autosave(world),
        other => save::queue_save(world, other.clone()),
    }
}

/// Autosave on the interval, straight onto the rotation.
pub fn queue_autosave_now(world: &mut World) {
    let message = match save::queue_autosave(world) {
        Ok(()) => "Autosaving...".to_string(),
        // A save already in flight is not an error worth telling the player about.
        Err(save::SaveError::Busy) => return,
        Err(err) => describe(&err, "autosave"),
    };
    if let Some(mut status) = world.get_resource_mut::<SaveStatus>() {
        status.set(message);
    }
}

/// Player-facing wording. A version mismatch is a different message from damage —
/// "from another version" is actionable, "broken" is not.
fn describe(error: &save::SaveError, verb: &str) -> String {
    if error.is_version_mismatch() {
        return "That save is from another version of the game".into();
    }
    match error {
        save::SaveError::NotFound(_) => "That save is gone".into(),
        save::SaveError::NoStorage => "No save storage on this platform".into(),
        save::SaveError::Busy => "A save is already running".into(),
        save::SaveError::InvalidSlotName(_) => "That save name cannot be used".into(),
        _ => format!("Could not {verb} - the file could not be read"),
    }
}

fn describe_restore(slot: &SaveSlot, report: &save::RestoreReport) -> String {
    let name = slot.display_name();
    if report.is_clean() {
        format!("Loaded {name}")
    } else {
        // Restoring is deliberately forgiving; say so rather than pretending the
        // load was exact.
        format!("Loaded {name} - some of it could not be restored")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autosave_fires_once_per_interval() {
        let mut timer = AutosaveTimer::default();
        assert!(!timer.tick(59.0, 1));
        assert!(timer.tick(2.0, 1), "one minute elapsed");
        assert!(!timer.tick(1.0, 1), "timer restarts after firing");
    }

    #[test]
    fn autosave_off_never_fires() {
        let mut timer = AutosaveTimer::default();
        for _ in 0..100 {
            assert!(!timer.tick(60.0, 0));
        }
    }

    #[test]
    fn resetting_the_timer_restarts_the_interval() {
        let mut timer = AutosaveTimer::default();
        assert!(!timer.tick(59.0, 1));
        timer.reset();
        assert!(!timer.tick(59.0, 1), "the clock restarted");
    }

    #[test]
    fn requests_are_recorded_for_the_exclusive_system_to_pick_up() {
        let mut request = ShellSaveRequest::default();
        request_save(&mut request, None);
        assert_eq!(request.save, Some(SaveSlot::Quick));

        request_load(&mut request, SaveSlot::Auto(0));
        assert_eq!(request.load, Some(SaveSlot::Auto(0)));
    }

    #[test]
    fn a_version_mismatch_reads_differently_from_damage() {
        let mismatch = save::SaveError::VersionMismatch {
            found: 1,
            expected: 99,
        };
        let damaged = save::SaveError::Corrupt("header past end of file");
        assert_ne!(describe(&mismatch, "load"), describe(&damaged, "load"));
        assert!(describe(&mismatch, "load").contains("another version"));
    }

    #[test]
    fn a_missing_save_says_so_plainly() {
        let missing = save::SaveError::NotFound("quick".into());
        assert_eq!(describe(&missing, "load"), "That save is gone");
    }
}
