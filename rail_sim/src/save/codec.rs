//! The bytes on disk: envelope, header, checksum, and the bincode payload.
//!
//! ```text
//! 0..4    magic                b"RTSV"
//! 4..6    schema version       u16 LE
//! 6..8    flags (reserved)     u16 LE
//! 8..12   header length        u32 LE
//! 12..    header               bincode(SaveMeta)
//! ..n-4   payload              bincode(WorldSnapshot)
//! n-4..n  checksum             u32 LE, CRC-32 of every preceding byte
//! ```
//!
//! The header comes first and is small so the save menu can list twenty slots
//! without decoding twenty worlds ([`decode_meta`]). The checksum is what turns
//! "half a file" into a clear message rather than a panic somewhere in bincode.

use serde::{Deserialize, Serialize};

use super::error::{SaveError, SaveResult};
use super::snapshot::{
    WorldSnapshot, WorldSnapshotV4, WorldSnapshotV5, MIN_READABLE_SCHEMA, SCHEMA_VERSION,
};

/// File magic — "Rail Town SaVe".
pub const SAVE_MAGIC: [u8; 4] = *b"RTSV";

/// Bytes before the header payload.
const PREFIX_LEN: usize = 12;
/// Trailing checksum width.
const CHECKSUM_LEN: usize = 4;

fn config() -> impl bincode::config::Config {
    bincode::config::standard()
}

/// A small RGBA thumbnail of the map, for the save list.
///
/// Produced by the presentation layer (the sim has no renderer); `None` is fine
/// and the menu should fall back to seed / stats text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA8, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// Everything the save menu needs without loading the world.
///
/// `09-shell-and-menus.md` §6 asks for a name, a thumbnail, the date, elapsed
/// time and headline stats — that list is this struct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveMeta {
    /// Schema of the payload that follows.
    pub schema_version: u16,
    /// Player-facing name ("Westbrook run", "Autosave 2").
    pub label: String,
    /// Seconds since the Unix epoch, or `0` where the platform has no clock.
    pub saved_at_unix: u64,
    /// Monotonic write counter, used to rotate autosaves without a clock.
    pub ordinal: u64,
    /// Build that wrote this save.
    pub app_version: String,
    pub map_seed: u64,
    pub map_width: u32,
    pub map_height: u32,
    /// Sim ticks elapsed in this world.
    pub sim_tick: u64,
    /// Sim seconds elapsed, for "3h 20m" in the menu.
    pub elapsed_sim_secs: u64,
    pub money_cents: i64,
    pub station_count: u32,
    pub track_count: u32,
    pub train_count: u32,
    pub line_count: u32,
    pub peep_count: u32,
    pub thumbnail: Option<Thumbnail>,
}

impl SaveMeta {
    /// Derive headline stats from the world being saved.
    pub fn from_snapshot(snapshot: &WorldSnapshot, label: impl Into<String>) -> Self {
        Self {
            schema_version: snapshot.schema_version,
            label: label.into(),
            saved_at_unix: now_unix_secs(),
            ordinal: 0,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            map_seed: snapshot.map.seed,
            map_width: snapshot.map.width,
            map_height: snapshot.map.height,
            sim_tick: snapshot.sim_tick(),
            elapsed_sim_secs: snapshot.elapsed_sim_secs(),
            money_cents: snapshot.money_cents,
            station_count: snapshot.stations.stations.len() as u32,
            track_count: snapshot.track.len() as u32,
            train_count: snapshot.trains.placed.len() as u32,
            line_count: snapshot.lines.len() as u32,
            peep_count: snapshot.peeps.peeps.len() as u32,
            thumbnail: None,
        }
    }

    /// Attach a map thumbnail produced by the presentation layer.
    pub fn with_thumbnail(mut self, thumbnail: Thumbnail) -> Self {
        self.thumbnail = Some(thumbnail);
        self
    }
}

/// Seconds since the Unix epoch. `0` on platforms with no wall clock (wasm).
pub fn now_unix_secs() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        // `SystemTime::now` panics on wasm32-unknown-unknown. Saves still order
        // correctly via `SaveMeta::ordinal`.
        0
    }
}

/// Serialise a world into save bytes.
pub fn encode_save(meta: &SaveMeta, snapshot: &WorldSnapshot) -> SaveResult<Vec<u8>> {
    let header = bincode::serde::encode_to_vec(meta, config())
        .map_err(|e| SaveError::Encode(e.to_string()))?;
    let payload = bincode::serde::encode_to_vec(snapshot, config())
        .map_err(|e| SaveError::Encode(e.to_string()))?;

    let mut out = Vec::with_capacity(PREFIX_LEN + header.len() + payload.len() + CHECKSUM_LEN);
    out.extend_from_slice(&SAVE_MAGIC);
    out.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // flags
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&payload);

    let checksum = crc32(&out);
    out.extend_from_slice(&checksum.to_le_bytes());
    Ok(out)
}

/// Write a world in an older envelope and shape — **test only**.
///
/// A migration is only worth anything if it is proved against bytes laid out
/// the way the shipped build laid them out. Encoding an old snapshot type
/// through the same bincode config the real encoder uses is how that is done
/// without checking a binary fixture into the repo.
#[cfg(test)]
fn encode_save_as<T: Serialize>(version: u16, meta: &SaveMeta, snapshot: &T) -> SaveResult<Vec<u8>> {
    let header = bincode::serde::encode_to_vec(meta, config())
        .map_err(|e| SaveError::Encode(e.to_string()))?;
    let payload = bincode::serde::encode_to_vec(snapshot, config())
        .map_err(|e| SaveError::Encode(e.to_string()))?;

    let mut out = Vec::with_capacity(PREFIX_LEN + header.len() + payload.len() + CHECKSUM_LEN);
    out.extend_from_slice(&SAVE_MAGIC);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // flags
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&payload);

    let checksum = crc32(&out);
    out.extend_from_slice(&checksum.to_le_bytes());
    Ok(out)
}

/// Write a world in schema 4's envelope and shape — **test only**.
#[cfg(test)]
pub fn encode_save_v4(meta: &SaveMeta, snapshot: &WorldSnapshotV4) -> SaveResult<Vec<u8>> {
    encode_save_as(4, meta, snapshot)
}

/// Write a world in schema 5's envelope and shape — **test only**.
#[cfg(test)]
pub fn encode_save_v5(meta: &SaveMeta, snapshot: &WorldSnapshotV5) -> SaveResult<Vec<u8>> {
    encode_save_as(5, meta, snapshot)
}

/// Validate the envelope and split it into version, header bytes, payload bytes.
///
/// Versions from [`MIN_READABLE_SCHEMA`] up to [`SCHEMA_VERSION`] pass; the
/// caller decides how to read each one. Anything newer is refused outright — a
/// build cannot guess at a shape that had not been designed when it shipped.
fn split(bytes: &[u8]) -> SaveResult<(u16, &[u8], &[u8])> {
    if bytes.len() < PREFIX_LEN + CHECKSUM_LEN {
        return Err(SaveError::Corrupt("file is shorter than a save header"));
    }
    let magic: [u8; 4] = bytes[0..4].try_into().expect("checked length");
    if magic != SAVE_MAGIC {
        return Err(SaveError::BadMagic { found: magic });
    }

    let version = u16::from_le_bytes(bytes[4..6].try_into().expect("checked length"));
    if !(MIN_READABLE_SCHEMA..=SCHEMA_VERSION).contains(&version) {
        return Err(SaveError::VersionMismatch {
            found: version,
            expected: SCHEMA_VERSION,
        });
    }

    let body_end = bytes.len() - CHECKSUM_LEN;
    let stored = u32::from_le_bytes(bytes[body_end..].try_into().expect("checked length"));
    let actual = crc32(&bytes[..body_end]);
    if stored != actual {
        return Err(SaveError::Checksum {
            found: stored,
            expected: actual,
        });
    }

    let header_len =
        u32::from_le_bytes(bytes[8..12].try_into().expect("checked length")) as usize;
    let header_end = PREFIX_LEN
        .checked_add(header_len)
        .ok_or(SaveError::Corrupt("header length overflows"))?;
    if header_end > body_end {
        return Err(SaveError::Corrupt("header runs past the end of the file"));
    }

    Ok((
        version,
        &bytes[PREFIX_LEN..header_end],
        &bytes[header_end..body_end],
    ))
}

/// Read only the header — used to list slots cheaply.
pub fn decode_meta(bytes: &[u8]) -> SaveResult<SaveMeta> {
    let (_, header, _) = split(bytes)?;
    let (meta, _) = bincode::serde::decode_from_slice::<SaveMeta, _>(header, config())
        .map_err(|e| SaveError::Decode(e.to_string()))?;
    Ok(meta)
}

/// Read the header and the whole world, migrating an older schema on the way.
///
/// The envelope's version is the authority on how to read the payload, and the
/// snapshot's own `schema_version` must agree with it — a file whose two
/// versions disagree is not a file this build is going to guess about.
pub fn decode_save(bytes: &[u8]) -> SaveResult<(SaveMeta, WorldSnapshot)> {
    let (version, header, payload) = split(bytes)?;
    let (meta, _) = bincode::serde::decode_from_slice::<SaveMeta, _>(header, config())
        .map_err(|e| SaveError::Decode(e.to_string()))?;

    let snapshot = match version {
        4 => {
            let (old, _) =
                bincode::serde::decode_from_slice::<WorldSnapshotV4, _>(payload, config())
                    .map_err(|e| SaveError::Decode(e.to_string()))?;
            if old.schema_version != 4 {
                return Err(SaveError::VersionMismatch {
                    found: old.schema_version,
                    expected: 4,
                });
            }
            old.upgrade()
        }
        5 => {
            let (old, _) =
                bincode::serde::decode_from_slice::<WorldSnapshotV5, _>(payload, config())
                    .map_err(|e| SaveError::Decode(e.to_string()))?;
            if old.schema_version != 5 {
                return Err(SaveError::VersionMismatch {
                    found: old.schema_version,
                    expected: 5,
                });
            }
            old.upgrade()
        }
        _ => {
            let (snapshot, _) =
                bincode::serde::decode_from_slice::<WorldSnapshot, _>(payload, config())
                    .map_err(|e| SaveError::Decode(e.to_string()))?;
            if snapshot.schema_version != SCHEMA_VERSION {
                return Err(SaveError::VersionMismatch {
                    found: snapshot.schema_version,
                    expected: SCHEMA_VERSION,
                });
            }
            snapshot
        }
    };
    Ok((meta, snapshot))
}

/// CRC-32 (IEEE 802.3, reflected). Small enough not to be worth a dependency.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (SaveMeta, WorldSnapshot) {
        let snapshot = WorldSnapshot::default();
        let meta = SaveMeta::from_snapshot(&snapshot, "Sample");
        (meta, snapshot)
    }

    #[test]
    fn crc32_matches_known_vector() {
        // The canonical CRC-32 of "123456789".
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn envelope_round_trips() {
        let (meta, snapshot) = sample();
        let bytes = encode_save(&meta, &snapshot).expect("encode");
        assert_eq!(&bytes[0..4], &SAVE_MAGIC);

        let header_only = decode_meta(&bytes).expect("meta");
        assert_eq!(header_only, meta);

        let (read_meta, read_snapshot) = decode_save(&bytes).expect("decode");
        assert_eq!(read_meta, meta);
        assert_eq!(read_snapshot, snapshot);
    }

    #[test]
    fn foreign_bytes_are_not_mistaken_for_a_save() {
        let err = decode_save(b"this is not a save file at all").unwrap_err();
        assert!(matches!(err, SaveError::BadMagic { .. }), "got {err:?}");
        assert!(err.is_corrupt());
    }

    #[test]
    fn a_short_file_is_reported_not_panicked() {
        let err = decode_save(&[0u8; 4]).unwrap_err();
        assert!(matches!(err, SaveError::Corrupt(_)), "got {err:?}");
    }

    #[test]
    fn another_schema_version_is_named_as_such() {
        let (meta, snapshot) = sample();
        let mut bytes = encode_save(&meta, &snapshot).expect("encode");
        bytes[4] = bytes[4].wrapping_add(7);
        // Fix the checksum so the version check is what actually fires.
        let body_end = bytes.len() - CHECKSUM_LEN;
        let fixed = crc32(&bytes[..body_end]);
        bytes[body_end..].copy_from_slice(&fixed.to_le_bytes());

        let err = decode_save(&bytes).unwrap_err();
        assert!(err.is_version_mismatch(), "got {err:?}");
        assert!(!err.is_corrupt());
    }

    #[test]
    fn a_flipped_byte_is_caught_by_the_checksum() {
        let (meta, snapshot) = sample();
        let mut bytes = encode_save(&meta, &snapshot).expect("encode");
        let victim = bytes.len() - CHECKSUM_LEN - 1;
        bytes[victim] ^= 0xFF;

        let err = decode_save(&bytes).unwrap_err();
        assert!(matches!(err, SaveError::Checksum { .. }), "got {err:?}");
        assert!(err.is_corrupt());
    }

    #[test]
    fn truncation_is_caught() {
        let (meta, snapshot) = sample();
        let bytes = encode_save(&meta, &snapshot).expect("encode");
        let cut = &bytes[..bytes.len() - 10];
        assert!(decode_save(cut).is_err());
    }

    #[test]
    fn meta_carries_headline_stats() {
        let (meta, _) = sample();
        assert_eq!(meta.schema_version, SCHEMA_VERSION);
        assert_eq!(meta.label, "Sample");
        assert_eq!(meta.app_version, env!("CARGO_PKG_VERSION"));
    }
}
