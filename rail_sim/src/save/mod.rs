//! Save and load — a complete world in, a complete world out.
//!
//! # What a save is
//!
//! [`WorldSnapshot`] is the whole world: the map (seed, size, generation
//! options and the terrain itself), the track network, stations and industries,
//! lines, every train with its position, cargo and line assignment, town
//! density, the residents *with their names and the Town Talk they generated*,
//! money, the clock, the job board, service scores, and the demand/site state.
//!
//! Peep names and histories surviving a save is not a detail — it is the whole
//! point. `docs/design/09-shell-and-menus.md` §6: a saved town should feel like
//! a continuous place rather than a re-rolled state.
//!
//! # The multiplayer seam
//!
//! `IMPLEMENTATION_PLAN.md` § Multiplayer seams point 6 says this blob shape
//! will later double as a neighbour map chunk. So the snapshot holds only plain
//! data keyed by the stable ids in [`crate::ids`] — never a Bevy `Entity`,
//! which means nothing on the other side of a link. [`TerrainChunk`] is already
//! shaped to be handed over on its own.
//!
//! # Using it
//!
//! ```ignore
//! use rail_sim::save::{list_slots, load_from_slot, save_to_slot, SaveSlot};
//!
//! let slot = SaveSlot::named("Westbrook run")?;
//! save_to_slot(world, &slot)?;                  // blocking; fine while paused
//! let snapshot = load_from_slot(&slot)?;
//! snapshot.restore(&mut world);
//!
//! for info in list_slots()? {                   // headers only, no worlds decoded
//!     println!("{} — {} stations", info.title(), info.meta.station_count);
//! }
//! ```
//!
//! From the running game, save through [`queue_save`] / [`queue_autosave`]
//! instead: those capture the world on the spot and hand the encode and the
//! file write to a worker thread, so a save costs the sim nothing.
//!
//! # Versioning
//!
//! Every file carries [`SCHEMA_VERSION`]. A save from another schema fails with
//! [`SaveError::VersionMismatch`] — a different message to the player than
//! [`SaveError::Checksum`], which means the file is damaged. Bump the version
//! whenever the blob shape changes.
//!
//! # Extending the snapshot
//!
//! Most sections embed a sim type whole — the track network, lines, job board,
//! ledger, alert board, demand spawner, train yard, tile occupancy, every peep
//! component, and the [`crate::stations::Station`] / [`crate::stations::Industry`]
//! / [`crate::peeps::Household`] records. Adding a serialisable field to any of
//! those needs no change here at all.
//!
//! Three things are mirrored by hand because their types cannot be
//! deserialised, and each needs a line adding when the type grows:
//!
//! | Mirror | Mirrors | Watch for |
//! | --- | --- | --- |
//! | [`ServiceScoreSnapshot`] | `StationServiceScore` | new per-station score fields |
//! | [`ClockSnapshot`] | `SimClock` | new clock fields |
//! | [`BudgetSnapshot`] | `PeepBudget` | new level-of-detail tunables |
//!
//! **The compiler is what enforces that.** Every one of them destructures the
//! sim type field by field, in both directions, with no `..` rest pattern — so a
//! new field on the sim type is a build error at the mirror until someone
//! decides whether it belongs in the blob. Fields deliberately left out carry a
//! comment saying why. They used to restore with `..Default::default()`, which
//! is exactly how `StationServiceScore::peep_waiting` came to be dropped by
//! every save without a word.
//!
//! Registries whose ids can have holes (stations after a demolition, households
//! after a family leaves) are rebuilt by replaying inserts and stepping the
//! counter over the holes, so a line that stops at station 7 still finds
//! station 7 after a load.

mod background;
mod codec;
mod error;
mod slots;
mod snapshot;
mod storage;

#[cfg(test)]
mod tests;

pub use background::{
    autosave_async, queue_autosave, queue_save, save_to_slot_async, write_snapshot_async,
    SaveHandle, SaveJobs, SavePlugin,
};
pub use codec::{
    crc32, decode_meta, decode_save, encode_save, now_unix_secs, SaveMeta, Thumbnail, SAVE_MAGIC,
};
pub use error::{SaveError, SaveResult};
pub use slots::{
    autosave, autosave_snapshot, delete_slot, list_slots, load_from_slot, load_with_meta,
    next_autosave_slot, save_to_slot, slot_exists, slot_info, write_snapshot, SaveSlot, SlotInfo,
    AUTOSAVE_SLOTS, MAX_SLOT_NAME_LEN,
};
pub use snapshot::{
    BudgetSnapshot, ClockSnapshot, EconomySnapshot, MapDescriptor, MapGenOptions, MapSnapshot,
    PeepSnapshot, PeepsSnapshot, RestoreReport, ServiceScoreSnapshot, StationsSnapshot,
    TalkKindSnapshot, TerrainChunk, TownSnapshot, TownTalkSnapshot, TrainSnapshot, TrainsSnapshot,
    WorldSnapshot, GENERATOR_VERSION, SCHEMA_VERSION,
};
pub use storage::{save_root_display, set_save_root, SAVE_DIR_ENV, SAVE_EXTENSION};
