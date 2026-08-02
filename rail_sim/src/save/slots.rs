//! Named slots, quick save, and the autosave rotation.
//!
//! `09-shell-and-menus.md` §6: autosave on an interval and on quit into a
//! rotating set of slots, named manual saves with headline stats, and quick
//! save / quick load on function keys. Those are the three [`SaveSlot`] kinds.

use bevy_ecs::prelude::World;

use super::codec::{decode_meta, decode_save, encode_save, SaveMeta};
use super::error::{SaveError, SaveResult};
use super::snapshot::WorldSnapshot;
use super::storage;

/// How many autosaves are kept before the oldest is reused.
pub const AUTOSAVE_SLOTS: u8 = 3;

/// Longest allowed manual save name.
pub const MAX_SLOT_NAME_LEN: usize = 64;

/// Which save this is.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SaveSlot {
    /// A manual save the player named.
    Named(String),
    /// One step of the autosave rotation, `0..`[`AUTOSAVE_SLOTS`].
    Auto(u8),
    /// The single quick-save slot (F5 / F9).
    Quick,
}

impl SaveSlot {
    /// A validated manual slot.
    ///
    /// Names are checked rather than silently mangled, so the name the player
    /// typed is the name they see in the list.
    pub fn named(name: impl Into<String>) -> SaveResult<Self> {
        let name = name.into();
        let trimmed = name.trim();
        let usable = !trimmed.is_empty()
            && trimmed.len() <= MAX_SLOT_NAME_LEN
            && trimmed != "."
            && trimmed != ".."
            && trimmed.chars().all(is_name_char);
        if !usable {
            return Err(SaveError::InvalidSlotName(name));
        }
        Ok(Self::Named(trimmed.to_string()))
    }

    /// Filesystem / storage-key stem for this slot.
    ///
    /// Defensive: a `Named` built by hand rather than via [`SaveSlot::named`]
    /// still cannot escape the save directory.
    pub fn stem(&self) -> String {
        match self {
            Self::Named(name) => {
                let cleaned: String = name
                    .trim()
                    .chars()
                    .map(|c| if is_name_char(c) { c } else { '_' })
                    .take(MAX_SLOT_NAME_LEN)
                    .collect();
                if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
                    "unnamed".to_string()
                } else {
                    format!("save-{cleaned}")
                }
            }
            Self::Auto(index) => format!("auto-{}", index % AUTOSAVE_SLOTS.max(1)),
            Self::Quick => "quicksave".to_string(),
        }
    }

    /// Label shown in the menu before any save exists there.
    pub fn display_name(&self) -> String {
        match self {
            Self::Named(name) => name.clone(),
            Self::Auto(index) => format!("Autosave {}", index.saturating_add(1)),
            Self::Quick => "Quick save".to_string(),
        }
    }

    pub fn is_autosave(&self) -> bool {
        matches!(self, Self::Auto(_))
    }

    /// Rebuild a slot from a storage stem (used when listing).
    pub fn from_stem(stem: &str) -> Option<Self> {
        if stem == "quicksave" {
            return Some(Self::Quick);
        }
        if let Some(index) = stem.strip_prefix("auto-") {
            return index.parse::<u8>().ok().map(Self::Auto);
        }
        stem.strip_prefix("save-")
            .map(|name| Self::Named(name.to_string()))
    }
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_')
}

/// A slot and the header of the save sitting in it.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotInfo {
    pub slot: SaveSlot,
    pub meta: SaveMeta,
}

impl SlotInfo {
    /// Name to show in the list — the label stored in the save, else the slot's.
    pub fn title(&self) -> String {
        if self.meta.label.is_empty() {
            self.slot.display_name()
        } else {
            self.meta.label.clone()
        }
    }
}

// ---------------------------------------------------------------------------
// Save
// ---------------------------------------------------------------------------

/// Snapshot `world` and write it to `slot`.
///
/// This encodes and writes on the calling thread. Use
/// [`save_to_slot_async`](super::save_to_slot_async) from the running game so a
/// save never costs the sim a frame; this one suits tests, quick save on a
/// paused world, and save-on-quit.
pub fn save_to_slot(world: &World, slot: &SaveSlot) -> SaveResult<SlotInfo> {
    let snapshot = WorldSnapshot::capture(world);
    let meta = SaveMeta::from_snapshot(&snapshot, slot.display_name());
    write_snapshot(&snapshot, meta, slot)
}

/// Write an already-captured snapshot. This is the half that can run off-thread.
pub fn write_snapshot(
    snapshot: &WorldSnapshot,
    meta: SaveMeta,
    slot: &SaveSlot,
) -> SaveResult<SlotInfo> {
    let bytes = encode_save(&meta, snapshot)?;
    storage::write(&slot.stem(), &bytes)?;
    Ok(SlotInfo {
        slot: slot.clone(),
        meta,
    })
}

// ---------------------------------------------------------------------------
// Load
// ---------------------------------------------------------------------------

/// Read the world back out of `slot`.
pub fn load_from_slot(slot: &SaveSlot) -> SaveResult<WorldSnapshot> {
    load_with_meta(slot).map(|(_, snapshot)| snapshot)
}

/// Read the world and its header together.
pub fn load_with_meta(slot: &SaveSlot) -> SaveResult<(SaveMeta, WorldSnapshot)> {
    let bytes = storage::read(&slot.stem())?;
    decode_save(&bytes)
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

/// Header of one slot, without decoding the world.
pub fn slot_info(slot: &SaveSlot) -> SaveResult<SlotInfo> {
    let bytes = storage::read(&slot.stem())?;
    let meta = decode_meta(&bytes)?;
    Ok(SlotInfo {
        slot: slot.clone(),
        meta,
    })
}

/// `true` when something is saved in this slot.
pub fn slot_exists(slot: &SaveSlot) -> bool {
    storage::exists(&slot.stem())
}

/// Every readable save, newest first.
///
/// Unreadable files (a save from another build, a damaged file) are skipped
/// rather than failing the whole listing — one bad file must not hide the rest.
/// Use [`slot_info`] on a specific slot to see why one is missing.
pub fn list_slots() -> SaveResult<Vec<SlotInfo>> {
    let mut out = Vec::new();
    for stem in storage::list_stems()? {
        let Some(slot) = SaveSlot::from_stem(&stem) else {
            continue;
        };
        if let Ok(info) = slot_info(&slot) {
            out.push(info);
        }
    }
    out.sort_by(|a, b| {
        b.meta
            .saved_at_unix
            .cmp(&a.meta.saved_at_unix)
            .then(b.meta.ordinal.cmp(&a.meta.ordinal))
            .then(a.slot.cmp(&b.slot))
    });
    Ok(out)
}

/// Forget the save in `slot`.
pub fn delete_slot(slot: &SaveSlot) -> SaveResult<()> {
    storage::delete(&slot.stem())
}

// ---------------------------------------------------------------------------
// Autosave rotation
// ---------------------------------------------------------------------------

/// The autosave slot to write next: the first empty one, else the oldest.
///
/// Ordering uses the stored ordinal rather than the wall clock, so rotation is
/// still correct on a platform with no clock (wasm) and on a machine whose
/// clock moved backwards.
pub fn next_autosave_slot() -> (SaveSlot, u64) {
    let mut oldest: Option<(u64, u8)> = None;
    let mut highest_ordinal = 0u64;

    for index in 0..AUTOSAVE_SLOTS {
        let slot = SaveSlot::Auto(index);
        match slot_info(&slot) {
            Err(_) => return (slot, highest_ordinal.saturating_add(1)),
            Ok(info) => {
                highest_ordinal = highest_ordinal.max(info.meta.ordinal);
                let is_older = match oldest {
                    None => true,
                    Some((ordinal, _)) => info.meta.ordinal < ordinal,
                };
                if is_older {
                    oldest = Some((info.meta.ordinal, index));
                }
            }
        }
    }

    let index = oldest.map(|(_, i)| i).unwrap_or(0);
    (SaveSlot::Auto(index), highest_ordinal.saturating_add(1))
}

/// Snapshot `world` into the next autosave slot.
pub fn autosave(world: &World) -> SaveResult<SlotInfo> {
    let snapshot = WorldSnapshot::capture(world);
    autosave_snapshot(&snapshot)
}

/// Write an already-captured snapshot into the next autosave slot.
pub fn autosave_snapshot(snapshot: &WorldSnapshot) -> SaveResult<SlotInfo> {
    let (slot, ordinal) = next_autosave_slot();
    let mut meta = SaveMeta::from_snapshot(snapshot, slot.display_name());
    meta.ordinal = ordinal;
    write_snapshot(snapshot, meta, &slot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::storage::use_test_root;

    #[test]
    fn names_are_validated_not_mangled() {
        assert!(SaveSlot::named("Westbrook run").is_ok());
        assert!(SaveSlot::named("  spaced  ").is_ok());
        assert!(SaveSlot::named("").is_err());
        assert!(SaveSlot::named("   ").is_err());
        assert!(SaveSlot::named("..").is_err());
        assert!(SaveSlot::named("../../etc/passwd").is_err());
        assert!(SaveSlot::named("a".repeat(MAX_SLOT_NAME_LEN + 1)).is_err());
    }

    #[test]
    fn stems_cannot_escape_the_save_directory() {
        // Built by hand, bypassing validation.
        let sneaky = SaveSlot::Named("../../etc/passwd".into());
        let stem = sneaky.stem();
        assert!(!stem.contains('/'), "{stem}");
        assert!(!stem.contains('\\'), "{stem}");
        assert!(!stem.contains(".."), "{stem}");
    }

    #[test]
    fn stems_round_trip_through_from_stem() {
        let cases = [
            SaveSlot::Named("Westbrook run".into()),
            SaveSlot::Auto(0),
            SaveSlot::Auto(2),
            SaveSlot::Quick,
        ];
        for slot in cases {
            let stem = slot.stem();
            assert_eq!(SaveSlot::from_stem(&stem), Some(slot.clone()), "{stem}");
        }
    }

    #[test]
    fn autosave_rotation_reuses_the_oldest_slot() {
        use_test_root();
        for index in 0..AUTOSAVE_SLOTS {
            let _ = delete_slot(&SaveSlot::Auto(index));
        }

        let snapshot = WorldSnapshot::default();
        let mut written = Vec::new();
        for _ in 0..AUTOSAVE_SLOTS {
            written.push(autosave_snapshot(&snapshot).expect("autosave").slot);
        }
        // Each of the three autosave slots was used exactly once.
        let mut sorted = written.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), AUTOSAVE_SLOTS as usize, "{written:?}");

        // The fourth autosave reuses the first slot written.
        let fourth = autosave_snapshot(&snapshot).expect("autosave").slot;
        assert_eq!(fourth, written[0], "expected the oldest slot to be reused");

        for index in 0..AUTOSAVE_SLOTS {
            let _ = delete_slot(&SaveSlot::Auto(index));
        }
    }

    #[test]
    fn listing_skips_unreadable_files_instead_of_failing() {
        use_test_root();
        let good = SaveSlot::Named("listing good".into());
        let _ = delete_slot(&good);

        let snapshot = WorldSnapshot::default();
        let meta = SaveMeta::from_snapshot(&snapshot, good.display_name());
        write_snapshot(&snapshot, meta, &good).expect("write");

        // A file that is in the directory but is not a save we can read.
        storage::write("save-listing junk", b"not a save at all").expect("write junk");

        let listed = list_slots().expect("list");
        assert!(listed.iter().any(|i| i.slot == good), "{listed:?}");
        assert!(
            !listed
                .iter()
                .any(|i| i.slot == SaveSlot::Named("listing junk".into())),
            "damaged saves must not appear as loadable"
        );

        let _ = delete_slot(&good);
        let _ = storage::delete("save-listing junk");
    }
}
