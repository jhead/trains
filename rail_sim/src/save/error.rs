//! Failure modes for save / load.
//!
//! Every variant is something the shell can show the player in one sentence:
//! a save from an older build, a truncated file, a slot that is not there.

use std::fmt;

/// Convenience alias for save/load results.
pub type SaveResult<T> = Result<T, SaveError>;

/// Everything that can go wrong writing or reading a save blob.
#[derive(Debug)]
pub enum SaveError {
    /// Filesystem trouble (permissions, missing config dir, disk full).
    Io(std::io::Error),
    /// The snapshot could not be turned into bytes.
    Encode(String),
    /// The bytes could not be turned back into a snapshot.
    Decode(String),
    /// No save exists in that slot.
    NotFound(String),
    /// The file does not start with the Rail Town save magic.
    BadMagic { found: [u8; 4] },
    /// The file was written by a different save schema.
    VersionMismatch { found: u16, expected: u16 },
    /// Structurally wrong (header length past end of file, etc.).
    Corrupt(&'static str),
    /// Checksum did not match — the file was truncated or edited.
    Checksum { found: u32, expected: u32 },
    /// Slot name contained characters that cannot be a filename.
    InvalidSlotName(String),
    /// This platform has no persistent save storage (wasm without a backend).
    NoStorage,
    /// A background save for that slot is still running.
    Busy,
}

impl SaveError {
    /// `true` when the file is readable but from another schema version.
    ///
    /// The shell should offer "this save is from another version" rather than
    /// "this save is broken" — they are very different messages to a player.
    pub fn is_version_mismatch(&self) -> bool {
        matches!(self, Self::VersionMismatch { .. })
    }

    /// `true` when the bytes are damaged rather than merely old.
    pub fn is_corrupt(&self) -> bool {
        matches!(
            self,
            Self::BadMagic { .. } | Self::Corrupt(_) | Self::Checksum { .. } | Self::Decode(_)
        )
    }

    /// `true` when there is simply nothing saved there yet.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound(_))
    }
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "save storage error: {e}"),
            Self::Encode(e) => write!(f, "could not encode the world: {e}"),
            Self::Decode(e) => write!(f, "could not read the world from this save: {e}"),
            Self::NotFound(slot) => write!(f, "no save in slot “{slot}”"),
            Self::BadMagic { found } => {
                write!(f, "not a Rail Town save (header {found:?})")
            }
            Self::VersionMismatch { found, expected } => write!(
                f,
                "save is version {found}, this build reads version {expected}"
            ),
            Self::Corrupt(what) => write!(f, "save file is damaged: {what}"),
            Self::Checksum { found, expected } => write!(
                f,
                "save file is damaged: checksum {found:#010x} != {expected:#010x}"
            ),
            Self::InvalidSlotName(name) => write!(f, "“{name}” is not a usable save name"),
            Self::NoStorage => write!(f, "this platform has no save storage"),
            Self::Busy => write!(f, "a save is already in progress"),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SaveError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kinds_are_distinguishable() {
        let old = SaveError::VersionMismatch {
            found: 0,
            expected: 1,
        };
        assert!(old.is_version_mismatch());
        assert!(!old.is_corrupt());

        let broken = SaveError::Checksum {
            found: 1,
            expected: 2,
        };
        assert!(broken.is_corrupt());
        assert!(!broken.is_version_mismatch());

        let missing = SaveError::NotFound("auto-0".into());
        assert!(missing.is_not_found());
        assert!(!missing.is_corrupt());
    }

    #[test]
    fn messages_read_like_sentences() {
        let e = SaveError::VersionMismatch {
            found: 3,
            expected: 1,
        };
        assert_eq!(
            e.to_string(),
            "save is version 3, this build reads version 1"
        );
    }
}
