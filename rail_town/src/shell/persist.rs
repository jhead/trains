//! Settings file on disk — a flat RON struct, hand-rolled.
//!
//! `rail_town` has no serde / ron dependency and this module owns no manifest, so
//! the reader and writer here are deliberately tiny: one `key: value,` per line
//! inside a `( … )` struct body. Values are bare integers, `true` / `false`, or
//! quoted strings. Unknown keys are ignored on read and dropped on write, so an
//! older file never blocks a newer build (and vice versa).
//!
//! If `ron` is ever added to `rail_town/Cargo.toml`, [`KvDoc`] can be deleted and
//! `Settings` given `#[derive(Serialize, Deserialize)]` with no change elsewhere.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Directory override, mostly so tests and CI never touch a real profile.
pub const CONFIG_DIR_ENV: &str = "RAIL_TOWN_CONFIG_DIR";

/// File name inside the config directory.
pub const SETTINGS_FILE: &str = "settings.ron";

/// Ordered key → value document. Order is stable so the file does not churn.
#[derive(Debug, Default, Clone)]
pub struct KvDoc {
    entries: Vec<(String, String)>,
}

impl KvDoc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_int(&mut self, key: &str, value: i64) {
        self.entries.push((key.into(), value.to_string()));
    }

    pub fn set_bool(&mut self, key: &str, value: bool) {
        self.entries.push((key.into(), value.to_string()));
    }

    pub fn set_str(&mut self, key: &str, value: &str) {
        self.entries
            .push((key.into(), format!("\"{}\"", escape(value))));
    }

    /// RON struct body, one field per line.
    pub fn to_ron(&self) -> String {
        let mut out =
            String::from("// Rail Town settings. Rewritten whenever a setting changes.\n(\n");
        for (key, value) in &self.entries {
            out.push_str("    ");
            out.push_str(key);
            out.push_str(": ");
            out.push_str(value);
            out.push_str(",\n");
        }
        out.push_str(")\n");
        out
    }

    pub fn parse(text: &str) -> ParsedKv {
        let mut map = HashMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty()
                || line.starts_with("//")
                || line == "("
                || line == ")"
                || line.starts_with('#')
            {
                continue;
            }
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim().trim_end_matches(',').trim();
            if key.is_empty() {
                continue;
            }
            map.insert(key.to_string(), unquote(value));
        }
        ParsedKv { map }
    }
}

/// Read side: every getter falls back to the caller's default.
#[derive(Debug, Default, Clone)]
pub struct ParsedKv {
    map: HashMap<String, String>,
}

impl ParsedKv {
    pub fn int(&self, key: &str, fallback: i64) -> i64 {
        self.map
            .get(key)
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(fallback)
    }

    pub fn bool(&self, key: &str, fallback: bool) -> bool {
        match self.map.get(key).map(String::as_str) {
            Some("true") => true,
            Some("false") => false,
            _ => fallback,
        }
    }

    pub fn str<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        self.map.get(key).map(String::as_str).unwrap_or(fallback)
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        trimmed[1..trimmed.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else {
        trimmed.to_string()
    }
}

/// Per-user config directory, honouring [`CONFIG_DIR_ENV`] first.
///
/// No `dirs` dependency: the platform conventions are three lines each.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(CONFIG_DIR_ENV) {
        return Some(PathBuf::from(dir));
    }
    if cfg!(target_os = "windows") {
        return std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("RailTown"));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    if cfg!(target_os = "macos") {
        return Some(home.join("Library/Application Support/RailTown"));
    }
    Some(
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("rail_town"),
    )
}

/// Path of a named document inside the config directory.
pub fn doc_path(file: &str) -> Option<PathBuf> {
    config_dir().map(|d| d.join(file))
}

pub fn settings_path() -> Option<PathBuf> {
    doc_path(SETTINGS_FILE)
}

/// Read a document from the config directory. `None` when there is no readable
/// file yet, which is simply how a first run looks.
pub fn load_doc(file: &str) -> Option<ParsedKv> {
    let path = doc_path(file)?;
    let text = fs::read_to_string(path).ok()?;
    let parsed = KvDoc::parse(&text);
    (!parsed.is_empty()).then_some(parsed)
}

/// Write a document, creating the config directory if needed.
///
/// Failure is not fatal — a read-only profile should never stop the game — so the
/// error is returned for the caller to log rather than unwrapped.
pub fn save_doc(file: &str, doc: &KvDoc) -> std::io::Result<()> {
    let path = doc_path(file).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no config directory for this platform",
        )
    })?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, doc.to_ron())
}

/// Read the settings file. `None` when there is no readable file yet.
pub fn load_settings_doc() -> Option<ParsedKv> {
    load_doc(SETTINGS_FILE)
}

/// Write the settings file.
pub fn save_settings_doc(doc: &KvDoc) -> std::io::Result<()> {
    save_doc(SETTINGS_FILE, doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_value_kind() {
        let mut doc = KvDoc::new();
        doc.set_int("ui_scale", 3);
        doc.set_bool("vsync", false);
        doc.set_str("window_mode", "Borderless");

        let parsed = KvDoc::parse(&doc.to_ron());
        assert_eq!(parsed.int("ui_scale", 1), 3);
        assert!(!parsed.bool("vsync", true));
        assert_eq!(parsed.str("window_mode", "Windowed"), "Borderless");
    }

    #[test]
    fn missing_and_malformed_keys_fall_back() {
        let parsed = KvDoc::parse("(\n  ui_scale: banana,\n  stray line\n)\n");
        assert_eq!(parsed.int("ui_scale", 2), 2);
        assert_eq!(parsed.int("absent", 7), 7);
        assert!(parsed.bool("absent", true));
        assert_eq!(parsed.str("absent", "fallback"), "fallback");
    }

    #[test]
    fn unknown_keys_are_ignored_not_fatal() {
        let parsed = KvDoc::parse("(\n  from_a_newer_build: 12,\n  ui_scale: 2,\n)\n");
        assert_eq!(parsed.int("ui_scale", 1), 2);
    }

    #[test]
    fn quotes_and_backslashes_survive() {
        let mut doc = KvDoc::new();
        doc.set_str("name", r#"a "quoted" \ name"#);
        let parsed = KvDoc::parse(&doc.to_ron());
        assert_eq!(parsed.str("name", ""), r#"a "quoted" \ name"#);
    }

    #[test]
    fn ron_body_is_a_flat_struct() {
        let mut doc = KvDoc::new();
        doc.set_int("a", 1);
        let ron = doc.to_ron();
        assert!(ron.contains("(\n"));
        assert!(ron.trim_end().ends_with(')'));
        assert!(ron.contains("    a: 1,\n"));
    }

    #[test]
    fn any_named_document_lands_beside_the_settings() {
        // Onboarding "seen" state is per-player, not per-world, so it lives here
        // rather than in a save. It must share the settings' directory.
        let Some(dir) = config_dir() else {
            return;
        };
        let path = doc_path("onboarding.ron").expect("named docs follow the config dir");
        assert_eq!(path.parent(), Some(dir.as_path()));
        assert_eq!(settings_path(), doc_path(SETTINGS_FILE));
    }

    #[test]
    fn settings_path_sits_inside_the_config_dir() {
        // Not asserting an absolute path: the point is that the file always lands
        // under whatever directory the platform (or the env override) resolves to.
        let Some(dir) = config_dir() else {
            return; // No HOME / APPDATA in this environment; nothing to check.
        };
        let path = settings_path().expect("settings path follows the config dir");
        assert_eq!(path.parent(), Some(dir.as_path()));
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some(SETTINGS_FILE)
        );
    }
}
