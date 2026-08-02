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
//! composited from [`rail_map::MapGrid`], so a load has to mark them dirty. That
//! hook lives outside this module — see [`super::PendingWorld`] and the
//! `WorldRebuildSet` docs.

use bevy::prelude::*;
use rail_sim::save::{self, SaveSlot, SlotInfo};

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
            Ok(()) => format!("Saving {}…", slot.display_name()),
            Err(err) => describe(&err, "save"),
        });
    }

    if let Some(slot) = load {
        message = Some(match save::load_from_slot(&slot) {
            Ok(snapshot) => {
                let report = snapshot.restore(world);
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
        Ok(()) => "Autosaving…".to_string(),
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
        _ => format!("Could not {verb} — the file could not be read"),
    }
}

fn describe_restore(slot: &SaveSlot, report: &save::RestoreReport) -> String {
    let name = slot.display_name();
    if report.is_clean() {
        format!("Loaded {name}")
    } else {
        // Restoring is deliberately forgiving; say so rather than pretending the
        // load was exact.
        format!("Loaded {name} — some of it could not be restored")
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
