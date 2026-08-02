//! Where save bytes live, per platform.
//!
//! Native builds write files into the platform config directory. The wasm build
//! has no filesystem, so it gets a `localStorage`-shaped key/value store backed
//! by process memory — the same four calls (`get` / `set` / `remove` / `keys`)
//! a real `web_sys::Storage` binding would implement, so swapping one in later
//! touches this file and nothing else.
//!
//! Both backends expose the same four functions, so nothing above this module
//! knows which one it is talking to.

use super::error::{SaveError, SaveResult};

/// Extension for a Rail Town save file.
pub const SAVE_EXTENSION: &str = "rtsave";

/// Directory name under the platform config dir.
pub const APP_DIR: &str = "RailTown";

/// Environment variable that redirects saves (portable installs, tests, CI).
pub const SAVE_DIR_ENV: &str = "RAIL_TOWN_SAVE_DIR";

pub use backend::{delete, exists, list_stems, read, set_save_root, save_root_display, write};

// ---------------------------------------------------------------------------
// Native: real files in the platform config directory
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
mod backend {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    use super::{SaveError, SaveResult, APP_DIR, SAVE_DIR_ENV, SAVE_EXTENSION};

    fn override_root() -> &'static Mutex<Option<PathBuf>> {
        static ROOT: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
        ROOT.get_or_init(|| Mutex::new(None))
    }

    /// Point saves at `path` for this process (portable builds, tests).
    pub fn set_save_root(path: impl Into<PathBuf>) {
        let path = path.into();
        if let Ok(mut root) = override_root().lock() {
            *root = Some(path);
        }
    }

    /// The directory saves are read from and written to.
    ///
    /// Order: explicit override, then [`SAVE_DIR_ENV`], then the platform
    /// config directory.
    pub fn save_root() -> SaveResult<PathBuf> {
        if let Ok(root) = override_root().lock() {
            if let Some(path) = root.as_ref() {
                return Ok(path.clone());
            }
        }
        if let Some(dir) = std::env::var_os(SAVE_DIR_ENV) {
            if !dir.is_empty() {
                return Ok(PathBuf::from(dir));
            }
        }
        Ok(config_dir()?.join(APP_DIR).join("saves"))
    }

    /// Human-readable save location for the settings screen.
    pub fn save_root_display() -> String {
        save_root()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|e| e.to_string())
    }

    #[cfg(target_os = "macos")]
    fn config_dir() -> SaveResult<PathBuf> {
        let home = std::env::var_os("HOME").ok_or(SaveError::NoStorage)?;
        Ok(PathBuf::from(home).join("Library").join("Application Support"))
    }

    #[cfg(target_os = "windows")]
    fn config_dir() -> SaveResult<PathBuf> {
        let appdata = std::env::var_os("APPDATA")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .ok_or(SaveError::NoStorage)?;
        Ok(PathBuf::from(appdata))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn config_dir() -> SaveResult<PathBuf> {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            if !xdg.is_empty() {
                return Ok(PathBuf::from(xdg));
            }
        }
        let home = std::env::var_os("HOME").ok_or(SaveError::NoStorage)?;
        Ok(PathBuf::from(home).join(".config"))
    }

    fn path_for(stem: &str) -> SaveResult<PathBuf> {
        Ok(save_root()?.join(format!("{stem}.{SAVE_EXTENSION}")))
    }

    /// Write bytes for `stem`, replacing whatever was there.
    ///
    /// Writes a sibling temp file and renames it, so a crash mid-write leaves
    /// the previous save intact rather than half a new one.
    pub fn write(stem: &str, bytes: &[u8]) -> SaveResult<()> {
        let path = path_for(stem)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension(format!("{SAVE_EXTENSION}.tmp"));
        fs::write(&temp, bytes)?;
        match fs::rename(&temp, &path) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = fs::remove_file(&temp);
                Err(SaveError::Io(e))
            }
        }
    }

    pub fn read(stem: &str) -> SaveResult<Vec<u8>> {
        let path = path_for(stem)?;
        match fs::read(&path) {
            Ok(bytes) => Ok(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(SaveError::NotFound(stem.to_string()))
            }
            Err(e) => Err(SaveError::Io(e)),
        }
    }

    pub fn exists(stem: &str) -> bool {
        path_for(stem).map(|p| p.is_file()).unwrap_or(false)
    }

    pub fn delete(stem: &str) -> SaveResult<()> {
        let path = path_for(stem)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(SaveError::NotFound(stem.to_string()))
            }
            Err(e) => Err(SaveError::Io(e)),
        }
    }

    /// Every save stem present, sorted. A missing directory means no saves yet.
    pub fn list_stems() -> SaveResult<Vec<String>> {
        let root = save_root()?;
        if !root.is_dir() {
            return Ok(Vec::new());
        }
        let mut stems = Vec::new();
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some(SAVE_EXTENSION) {
                continue;
            }
            if let Some(stem) = file_stem(&path) {
                stems.push(stem);
            }
        }
        stems.sort();
        Ok(stems)
    }

    fn file_stem(path: &Path) -> Option<String> {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    }
}

// ---------------------------------------------------------------------------
// wasm: localStorage-shaped key/value store
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
mod backend {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::{SaveError, SaveResult, APP_DIR};

    /// Key prefix, matching what a real `localStorage` backend would use.
    const KEY_PREFIX: &str = "railtown.save.";

    thread_local! {
        /// Stand-in for `window.localStorage`. Swap the four accessors below
        /// for `web_sys::Storage` calls when the web build grows a JS bridge;
        /// nothing outside this module changes.
        static STORE: RefCell<BTreeMap<String, Vec<u8>>> = RefCell::new(BTreeMap::new());
    }

    fn key_for(stem: &str) -> String {
        format!("{KEY_PREFIX}{stem}")
    }

    /// No-op on wasm — there is no directory to point at.
    pub fn set_save_root(_path: impl Into<std::path::PathBuf>) {}

    pub fn save_root_display() -> String {
        format!("{KEY_PREFIX}* (browser storage, app “{APP_DIR}”)")
    }

    pub fn write(stem: &str, bytes: &[u8]) -> SaveResult<()> {
        STORE.with(|store| {
            store.borrow_mut().insert(key_for(stem), bytes.to_vec());
        });
        Ok(())
    }

    pub fn read(stem: &str) -> SaveResult<Vec<u8>> {
        STORE
            .with(|store| store.borrow().get(&key_for(stem)).cloned())
            .ok_or_else(|| SaveError::NotFound(stem.to_string()))
    }

    pub fn exists(stem: &str) -> bool {
        STORE.with(|store| store.borrow().contains_key(&key_for(stem)))
    }

    pub fn delete(stem: &str) -> SaveResult<()> {
        STORE
            .with(|store| store.borrow_mut().remove(&key_for(stem)))
            .map(|_| ())
            .ok_or_else(|| SaveError::NotFound(stem.to_string()))
    }

    pub fn list_stems() -> SaveResult<Vec<String>> {
        Ok(STORE.with(|store| {
            store
                .borrow()
                .keys()
                .filter_map(|k| k.strip_prefix(KEY_PREFIX).map(|s| s.to_string()))
                .collect()
        }))
    }
}

/// Point saves at a per-process temp directory.
///
/// All storage-touching tests share one root and use distinct slot names, so
/// they stay correct under the default parallel test runner.
#[cfg(test)]
pub(crate) fn use_test_root() {
    let root = std::env::temp_dir().join(format!("rail_town_save_tests_{}", std::process::id()));
    set_save_root(root);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_delete_round_trip() {
        use_test_root();
        let stem = "storage_round_trip";
        write(stem, b"hello rails").expect("write");
        assert!(exists(stem));
        assert_eq!(read(stem).expect("read"), b"hello rails");
        assert!(list_stems().expect("list").iter().any(|s| s == stem));
        delete(stem).expect("delete");
        assert!(!exists(stem));
        assert!(read(stem).unwrap_err().is_not_found());
    }

    #[test]
    fn missing_slot_is_not_found_rather_than_io() {
        use_test_root();
        let err = read("storage_definitely_absent").unwrap_err();
        assert!(err.is_not_found(), "got {err:?}");
    }

    #[test]
    fn overwriting_replaces_the_previous_bytes() {
        use_test_root();
        let stem = "storage_overwrite";
        write(stem, b"first").expect("write");
        write(stem, b"second").expect("rewrite");
        assert_eq!(read(stem).expect("read"), b"second");
        let _ = delete(stem);
    }
}
