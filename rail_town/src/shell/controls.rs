//! The rebindable control list, grouped by context, with conflict detection.
//!
//! Design [`09 §5`](../../../docs/design/09-shell-and-menus.md) wants "a full
//! rebindable list, grouped by context, with conflict detection and a reset", and
//! [`03 §10.2`](../../../docs/design/03-ui-system.md) is the shortcut table.
//!
//! **What this is today:** the *authoritative list and data model*. Defaults are
//! read out of the shipping code — every row below is a key some system really
//! listens for — so the Controls tab is an accurate reference, and the conflict
//! detector reports real conflicts rather than invented ones. Gameplay systems
//! still read [`KeyCode`] directly, so a rebound key does not yet change what the
//! game listens for; that arrives with the input-map slice, at which point those
//! systems read [`ControlSettings::key_for`] instead of a literal. The tab says
//! this plainly rather than pretending otherwise.

use bevy::prelude::*;

use super::persist::{KvDoc, ParsedKv};

/// Context group a binding belongs to. Rows are listed group by group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlGroup {
    Build,
    Time,
    View,
    System,
}

impl ControlGroup {
    pub const ALL: &'static [Self] = &[Self::Build, Self::Time, Self::View, Self::System];

    pub fn label(self) -> &'static str {
        match self {
            Self::Build => "Build & edit",
            Self::Time => "Time",
            Self::View => "View",
            Self::System => "System",
        }
    }
}

/// One rebindable action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlAction {
    TrackTool,
    DemolishTool,
    LineTool,
    BuyTransit,
    BuyTransport,
    CommitLine,
    Undo,
    Redo,
    PauseResume,
    Speed1,
    Speed2,
    Speed3,
    PanUp,
    PanDown,
    PanLeft,
    PanRight,
    MapView,
    CycleOverlay,
    OverlayService,
    OverlayCongestion,
    OverlayDensity,
    OverlayOff,
    FollowSelection,
    ResetZoom,
    Ledger,
    Unwind,
}

/// A key plus whether it is taken with control held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub key: KeyCode,
    pub ctrl: bool,
}

impl Binding {
    pub const fn key(key: KeyCode) -> Self {
        Self { key, ctrl: false }
    }

    pub const fn ctrl(key: KeyCode) -> Self {
        Self { key, ctrl: true }
    }

    /// Human-readable, e.g. `Ctrl+Z` or `Space`.
    pub fn label(self) -> String {
        let name = key_label(self.key);
        if self.ctrl {
            format!("Ctrl+{name}")
        } else {
            name.to_string()
        }
    }

    fn storage_name(self) -> String {
        if self.ctrl {
            format!("Ctrl+{:?}", self.key)
        } else {
            format!("{:?}", self.key)
        }
    }

    fn from_storage_name(text: &str) -> Option<Self> {
        let (ctrl, name) = match text.strip_prefix("Ctrl+") {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        key_from_debug_name(name).map(|key| Self { key, ctrl })
    }
}

impl ControlAction {
    /// Every action, in the order the Controls tab lists them.
    pub const ALL: &'static [Self] = &[
        Self::TrackTool,
        Self::DemolishTool,
        Self::LineTool,
        Self::BuyTransit,
        Self::BuyTransport,
        Self::CommitLine,
        Self::Undo,
        Self::Redo,
        Self::PauseResume,
        Self::Speed1,
        Self::Speed2,
        Self::Speed3,
        Self::PanUp,
        Self::PanDown,
        Self::PanLeft,
        Self::PanRight,
        Self::MapView,
        Self::CycleOverlay,
        Self::OverlayService,
        Self::OverlayCongestion,
        Self::OverlayDensity,
        Self::OverlayOff,
        Self::FollowSelection,
        Self::ResetZoom,
        Self::Ledger,
        Self::Unwind,
    ];

    pub fn group(self) -> ControlGroup {
        match self {
            Self::TrackTool
            | Self::DemolishTool
            | Self::LineTool
            | Self::BuyTransit
            | Self::BuyTransport
            | Self::CommitLine
            | Self::Undo
            | Self::Redo => ControlGroup::Build,
            Self::PauseResume | Self::Speed1 | Self::Speed2 | Self::Speed3 => ControlGroup::Time,
            Self::PanUp
            | Self::PanDown
            | Self::PanLeft
            | Self::PanRight
            | Self::MapView
            | Self::CycleOverlay
            | Self::OverlayService
            | Self::OverlayCongestion
            | Self::OverlayDensity
            | Self::OverlayOff
            | Self::FollowSelection
            | Self::ResetZoom => ControlGroup::View,
            Self::Ledger | Self::Unwind => ControlGroup::System,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::TrackTool => "Track tool",
            Self::DemolishTool => "Demolish tool",
            Self::LineTool => "Line tool",
            Self::BuyTransit => "Buy transit train",
            Self::BuyTransport => "Buy transport train",
            Self::CommitLine => "Commit line",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::PauseResume => "Pause / resume",
            Self::Speed1 => "Speed 1×",
            Self::Speed2 => "Speed 2×",
            Self::Speed3 => "Speed 3×",
            Self::PanUp => "Pan up",
            Self::PanDown => "Pan down",
            Self::PanLeft => "Pan left",
            Self::PanRight => "Pan right",
            Self::MapView => "Map View",
            Self::CycleOverlay => "Cycle overlay",
            Self::OverlayService => "Overlay: service",
            Self::OverlayCongestion => "Overlay: congestion",
            Self::OverlayDensity => "Overlay: density",
            Self::OverlayOff => "Overlay: off",
            Self::FollowSelection => "Follow selection",
            Self::ResetZoom => "Reset zoom",
            Self::Ledger => "Ledger",
            Self::Unwind => "Unwind / pause menu",
        }
    }

    /// The key this action really uses in the shipping build.
    pub fn default_binding(self) -> Binding {
        match self {
            Self::TrackTool => Binding::key(KeyCode::KeyB),
            Self::DemolishTool => Binding::key(KeyCode::KeyX),
            Self::LineTool => Binding::key(KeyCode::KeyL),
            Self::BuyTransit => Binding::key(KeyCode::KeyT),
            Self::BuyTransport => Binding::key(KeyCode::KeyG),
            Self::CommitLine => Binding::key(KeyCode::Enter),
            Self::Undo => Binding::ctrl(KeyCode::KeyZ),
            Self::Redo => Binding::ctrl(KeyCode::KeyY),
            Self::PauseResume => Binding::key(KeyCode::Space),
            Self::Speed1 => Binding::key(KeyCode::Digit1),
            Self::Speed2 => Binding::key(KeyCode::Digit2),
            Self::Speed3 => Binding::key(KeyCode::Digit3),
            Self::PanUp => Binding::key(KeyCode::KeyW),
            Self::PanDown => Binding::key(KeyCode::KeyS),
            Self::PanLeft => Binding::key(KeyCode::KeyA),
            Self::PanRight => Binding::key(KeyCode::KeyD),
            Self::MapView => Binding::key(KeyCode::KeyM),
            Self::CycleOverlay => Binding::key(KeyCode::Tab),
            Self::OverlayService => Binding::key(KeyCode::F1),
            Self::OverlayCongestion => Binding::key(KeyCode::F2),
            Self::OverlayDensity => Binding::key(KeyCode::F3),
            Self::OverlayOff => Binding::key(KeyCode::F4),
            Self::FollowSelection => Binding::key(KeyCode::KeyF),
            Self::ResetZoom => Binding::key(KeyCode::KeyZ),
            Self::Ledger => Binding::key(KeyCode::KeyL),
            Self::Unwind => Binding::key(KeyCode::Escape),
        }
    }

    /// Stable persistence key.
    fn storage_key(self) -> String {
        format!("controls_{:?}", self)
    }
}

/// Bindings plus the Controls-tab-only settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlSettings {
    bindings: Vec<(ControlAction, Binding)>,
    /// Repeat interval for incremental controls (design 03 §10.3).
    pub hold_repeat_ms: u32,
}

impl Default for ControlSettings {
    fn default() -> Self {
        Self {
            bindings: ControlAction::ALL
                .iter()
                .map(|a| (*a, a.default_binding()))
                .collect(),
            hold_repeat_ms: 180,
        }
    }
}

impl ControlSettings {
    pub fn key_for(&self, action: ControlAction) -> Binding {
        self.bindings
            .iter()
            .find(|(a, _)| *a == action)
            .map(|(_, b)| *b)
            .unwrap_or_else(|| action.default_binding())
    }

    pub fn set(&mut self, action: ControlAction, binding: Binding) {
        match self.bindings.iter_mut().find(|(a, _)| *a == action) {
            Some(slot) => slot.1 = binding,
            None => self.bindings.push((action, binding)),
        }
    }

    /// Restore every default. The Controls tab's Reset.
    pub fn reset(&mut self) {
        *self = Self {
            hold_repeat_ms: Self::default().hold_repeat_ms,
            ..Self::default()
        };
    }

    /// Actions that share a key with at least one other action.
    ///
    /// A modifier is part of the identity, so `Ctrl+Z` and `Z` do not clash.
    pub fn conflicts(&self) -> Vec<ControlAction> {
        let mut clashing = Vec::new();
        for (action, binding) in &self.bindings {
            let shared = self
                .bindings
                .iter()
                .any(|(other, other_binding)| other != action && other_binding == binding);
            if shared {
                clashing.push(*action);
            }
        }
        clashing
    }

    pub fn has_conflict(&self, action: ControlAction) -> bool {
        let binding = self.key_for(action);
        self.bindings
            .iter()
            .any(|(other, other_binding)| *other != action && *other_binding == binding)
    }

    pub(super) fn write_to(&self, doc: &mut KvDoc) {
        doc.set_int("controls_hold_repeat_ms", self.hold_repeat_ms as i64);
        for action in ControlAction::ALL {
            doc.set_str(&action.storage_key(), &self.key_for(*action).storage_name());
        }
    }

    pub(super) fn read_from(doc: &ParsedKv) -> Self {
        let mut settings = Self {
            hold_repeat_ms: doc.int("controls_hold_repeat_ms", 180).clamp(40, 1000) as u32,
            ..Self::default()
        };
        for action in ControlAction::ALL {
            let fallback = action.default_binding().storage_name();
            let stored = doc.str(&action.storage_key(), &fallback);
            if let Some(binding) = Binding::from_storage_name(stored) {
                settings.set(*action, binding);
            }
        }
        settings
    }
}

/// Keys a player may bind to. Modifiers and mouse buttons are excluded: a
/// modifier alone is not a shortcut, and the mouse verbs are fixed by design.
pub const REBINDABLE_KEYS: &[KeyCode] = &[
    KeyCode::KeyA,
    KeyCode::KeyB,
    KeyCode::KeyC,
    KeyCode::KeyD,
    KeyCode::KeyE,
    KeyCode::KeyF,
    KeyCode::KeyG,
    KeyCode::KeyH,
    KeyCode::KeyI,
    KeyCode::KeyJ,
    KeyCode::KeyK,
    KeyCode::KeyL,
    KeyCode::KeyM,
    KeyCode::KeyN,
    KeyCode::KeyO,
    KeyCode::KeyP,
    KeyCode::KeyQ,
    KeyCode::KeyR,
    KeyCode::KeyS,
    KeyCode::KeyT,
    KeyCode::KeyU,
    KeyCode::KeyV,
    KeyCode::KeyW,
    KeyCode::KeyX,
    KeyCode::KeyY,
    KeyCode::KeyZ,
    KeyCode::Digit0,
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
    KeyCode::F1,
    KeyCode::F2,
    KeyCode::F3,
    KeyCode::F4,
    KeyCode::F5,
    KeyCode::F6,
    KeyCode::F7,
    KeyCode::F8,
    KeyCode::F9,
    KeyCode::F10,
    KeyCode::F11,
    KeyCode::F12,
    KeyCode::Space,
    KeyCode::Tab,
    KeyCode::Enter,
    KeyCode::Escape,
    KeyCode::Backquote,
    KeyCode::Minus,
    KeyCode::Equal,
    KeyCode::BracketLeft,
    KeyCode::BracketRight,
    KeyCode::Comma,
    KeyCode::Period,
    KeyCode::Slash,
    KeyCode::Semicolon,
    KeyCode::Backslash,
    KeyCode::Delete,
    KeyCode::Home,
    KeyCode::End,
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
];

/// `true` when a key may be bound at all (a bare modifier may not).
pub fn is_rebindable(key: KeyCode) -> bool {
    REBINDABLE_KEYS.contains(&key)
}

fn key_from_debug_name(name: &str) -> Option<KeyCode> {
    REBINDABLE_KEYS
        .iter()
        .copied()
        .find(|k| format!("{k:?}") == name)
}

/// Short display name — `KeyB` reads as `B`, `Digit1` as `1`.
pub fn key_label(key: KeyCode) -> String {
    let raw = format!("{key:?}");
    if let Some(letter) = raw.strip_prefix("Key") {
        return letter.to_string();
    }
    if let Some(digit) = raw.strip_prefix("Digit") {
        return digit.to_string();
    }
    match key {
        KeyCode::ArrowUp => "↑".into(),
        KeyCode::ArrowDown => "↓".into(),
        KeyCode::ArrowLeft => "←".into(),
        KeyCode::ArrowRight => "→".into(),
        KeyCode::Backquote => "`".into(),
        KeyCode::Minus => "-".into(),
        KeyCode::Equal => "=".into(),
        KeyCode::Comma => ",".into(),
        KeyCode::Period => ".".into(),
        KeyCode::Slash => "/".into(),
        KeyCode::Semicolon => ";".into(),
        KeyCode::Backslash => "\\".into(),
        KeyCode::BracketLeft => "[".into(),
        KeyCode::BracketRight => "]".into(),
        _ => raw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_has_a_group_a_label_and_a_default() {
        for action in ControlAction::ALL {
            assert!(!action.label().is_empty(), "{action:?} has no label");
            assert!(ControlGroup::ALL.contains(&action.group()));
            assert!(
                is_rebindable(action.default_binding().key),
                "{action:?} defaults to a key that cannot be rebound"
            );
        }
    }

    #[test]
    fn every_group_has_at_least_one_row() {
        for group in ControlGroup::ALL {
            assert!(
                ControlAction::ALL.iter().any(|a| a.group() == *group),
                "{} is empty",
                group.label()
            );
        }
    }

    #[test]
    fn conflict_detection_finds_the_real_l_key_clash() {
        // Line tool and the ledger panel both listen for `L` in the shipping
        // build. The tab exists to surface exactly this.
        let controls = ControlSettings::default();
        assert!(controls.has_conflict(ControlAction::LineTool));
        assert!(controls.has_conflict(ControlAction::Ledger));
        assert!(controls.conflicts().contains(&ControlAction::Ledger));
    }

    #[test]
    fn a_modifier_makes_a_binding_distinct() {
        let controls = ControlSettings::default();
        // Ctrl+Z (undo) must not read as a conflict with Z (reset zoom).
        assert!(!controls.has_conflict(ControlAction::Undo));
        assert!(!controls.has_conflict(ControlAction::ResetZoom));
    }

    #[test]
    fn rebinding_creates_and_reset_clears_a_conflict() {
        let mut controls = ControlSettings::default();
        controls.set(ControlAction::MapView, Binding::key(KeyCode::KeyB));
        assert!(controls.has_conflict(ControlAction::MapView));
        assert!(controls.has_conflict(ControlAction::TrackTool));

        controls.reset();
        assert!(!controls.has_conflict(ControlAction::MapView));
        assert_eq!(
            controls.key_for(ControlAction::MapView),
            Binding::key(KeyCode::KeyM)
        );
    }

    #[test]
    fn bindings_round_trip_through_the_file_format() {
        let mut controls = ControlSettings::default();
        controls.set(ControlAction::MapView, Binding::ctrl(KeyCode::F9));
        controls.hold_repeat_ms = 400;

        let mut doc = KvDoc::new();
        controls.write_to(&mut doc);
        let restored = ControlSettings::read_from(&KvDoc::parse(&doc.to_ron()));
        assert_eq!(restored, controls);
    }

    #[test]
    fn an_unknown_key_name_falls_back_to_the_default() {
        let parsed = KvDoc::parse("(\n  controls_MapView: \"KeyNope\",\n)");
        let restored = ControlSettings::read_from(&parsed);
        assert_eq!(
            restored.key_for(ControlAction::MapView),
            ControlAction::MapView.default_binding()
        );
    }

    #[test]
    fn key_labels_are_short_and_readable() {
        assert_eq!(key_label(KeyCode::KeyB), "B");
        assert_eq!(key_label(KeyCode::Digit3), "3");
        assert_eq!(key_label(KeyCode::Space), "Space");
        assert_eq!(Binding::ctrl(KeyCode::KeyZ).label(), "Ctrl+Z");
    }
}
