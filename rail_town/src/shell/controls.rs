//! The rebindable control list, grouped by context, with conflict detection.
//!
//! Design [`09 §5`](../../../docs/design/09-shell-and-menus.md) wants "a full
//! rebindable list, grouped by context, with conflict detection and a reset", and
//! [`03 §10.2`](../../../docs/design/03-ui-system.md) is the shortcut table.
//!
//! **What this is:** the *authoritative table* — what each verb is called, which
//! group it belongs to, what it defaults to, and how it persists. Defaults are
//! read out of the shipping code, so the Controls tab is an accurate reference
//! and the conflict detector reports real conflicts rather than invented ones.
//!
//! The *live* half is [`crate::input::KeyBindings`], which adopts this map
//! whenever it changes; gameplay systems look actions up there instead of
//! reading a [`KeyCode`] literal. A rebind therefore reaches the game.
//!
//! **`L` used to be listed twice.** The Line tool owns it (03 §10.2), so the
//! Ledger answers to `K` — which is what the menu row and the brief's shortcut
//! table already said, while this file was still the stale one. A test asserts
//! the defaults are conflict-free.

use bevy::prelude::*;

use super::persist::{KvDoc, ParsedKv};

/// Context group a binding belongs to. Rows are listed group by group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlGroup {
    Build,
    Time,
    View,
    Windows,
    System,
}

impl ControlGroup {
    pub const ALL: &'static [Self] = &[
        Self::Build,
        Self::Time,
        Self::View,
        Self::Windows,
        Self::System,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Build => "Build & edit",
            Self::Time => "Time",
            Self::View => "View",
            Self::Windows => "Windows",
            Self::System => "System",
        }
    }
}

/// One rebindable action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlAction {
    LookTool,
    TrackTool,
    DemolishTool,
    LineTool,
    PlaceStation,
    UpgradeStation,
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
    WindowNetwork,
    WindowTownTalk,
    Ledger,
    WindowAlerts,
    WindowGoals,
    WindowNeighbours,
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
        Self::LookTool,
        Self::TrackTool,
        Self::DemolishTool,
        Self::LineTool,
        Self::PlaceStation,
        Self::UpgradeStation,
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
        Self::WindowNetwork,
        Self::WindowTownTalk,
        Self::Ledger,
        Self::WindowAlerts,
        Self::WindowGoals,
        Self::WindowNeighbours,
        Self::Unwind,
    ];

    pub fn group(self) -> ControlGroup {
        match self {
            Self::LookTool
            | Self::TrackTool
            | Self::DemolishTool
            | Self::LineTool
            | Self::PlaceStation
            | Self::UpgradeStation
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
            Self::WindowNetwork
            | Self::WindowTownTalk
            | Self::Ledger
            | Self::WindowAlerts
            | Self::WindowGoals
            | Self::WindowNeighbours => ControlGroup::Windows,
            Self::Unwind => ControlGroup::System,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::LookTool => "Look tool",
            Self::TrackTool => "Track tool",
            Self::DemolishTool => "Demolish tool",
            Self::LineTool => "Line tool",
            Self::PlaceStation => "Station tool",
            Self::UpgradeStation => "Upgrade station",
            Self::BuyTransit => "Buy transit train",
            Self::BuyTransport => "Buy transport train",
            Self::CommitLine => "Commit line",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::PauseResume => "Pause / resume",
            Self::Speed1 => "Speed 1x",
            Self::Speed2 => "Speed 2x",
            Self::Speed3 => "Speed 3x",
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
            Self::WindowNetwork => "Network",
            Self::WindowTownTalk => "Town Talk",
            Self::Ledger => "Ledger",
            Self::WindowAlerts => "Alerts",
            Self::WindowGoals => "Goals",
            Self::WindowNeighbours => "Neighbours",
            Self::Unwind => "Unwind / pause menu",
        }
    }

    /// The key this action really uses in the shipping build.
    ///
    /// The window group is 03 §10.2's second row, and it deliberately avoids
    /// every key a gameplay verb owns — which is why the Ledger is `K` and not
    /// `L`. A test asserts the whole table is conflict-free.
    pub fn default_binding(self) -> Binding {
        match self {
            Self::LookTool => Binding::key(KeyCode::KeyV),
            Self::TrackTool => Binding::key(KeyCode::KeyB),
            Self::DemolishTool => Binding::key(KeyCode::KeyX),
            Self::LineTool => Binding::key(KeyCode::KeyL),
            Self::PlaceStation => Binding::key(KeyCode::KeyP),
            Self::UpgradeStation => Binding::key(KeyCode::KeyU),
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
            Self::WindowNetwork => Binding::key(KeyCode::KeyH),
            Self::WindowTownTalk => Binding::key(KeyCode::KeyY),
            Self::Ledger => Binding::key(KeyCode::KeyK),
            Self::WindowAlerts => Binding::key(KeyCode::KeyC),
            Self::WindowGoals => Binding::key(KeyCode::KeyO),
            Self::WindowNeighbours => Binding::key(KeyCode::KeyN),
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
                if RETIRED_DEFAULTS.contains(&(*action, binding)) {
                    continue;
                }
                settings.set(*action, binding);
            }
        }
        settings
    }
}

/// Bindings that used to be a default and may not come back off disk.
///
/// The Ledger shipped on `L`, which the Line tool also owns. Every profile
/// written before the fix has `controls_Ledger: "KeyL"` in it, so without this
/// the clash would simply reload — and the Controls tab would flag a conflict
/// the player never chose. A stored value that is *exactly* the retired default
/// is read as unset; every other stored value is the player's own and is
/// honoured, conflict or not.
///
/// The cost is that a player who deliberately rebinds the Ledger back onto `L`
/// loses that choice on the next launch. It is one binding, it is the one the
/// brief says belongs to another verb, and the alternative is either a schema
/// version for a flat key-value file or leaving a shipped clash in place.
const RETIRED_DEFAULTS: &[(ControlAction, Binding)] =
    &[(ControlAction::Ledger, Binding::key(KeyCode::KeyL))];

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
        KeyCode::ArrowUp => "^".into(),
        KeyCode::ArrowDown => "v".into(),
        KeyCode::ArrowLeft => "<-".into(),
        KeyCode::ArrowRight => "->".into(),
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
            // 03 §3: the shipped font has no glyphs beyond ASCII.
            assert!(action.label().is_ascii(), "{action:?} has a tofu label");
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
    fn conflict_free_defaults() {
        // The shipping table must not ask two verbs for the same key. `L` was
        // listed twice — Line tool and Ledger — which is the clash the Controls
        // tab was built to surface and which this now keeps from coming back.
        let controls = ControlSettings::default();
        assert!(
            controls.conflicts().is_empty(),
            "default bindings clash: {:?}",
            controls
                .conflicts()
                .iter()
                .map(|a| (a.label(), a.default_binding().label()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_line_tool_keeps_l_and_the_ledger_moved_to_k() {
        // 03 §10.2: `L` belongs to the Line tool, so the Ledger answers to `K`.
        let controls = ControlSettings::default();
        assert_eq!(
            controls.key_for(ControlAction::LineTool),
            Binding::key(KeyCode::KeyL)
        );
        assert_eq!(
            controls.key_for(ControlAction::Ledger),
            Binding::key(KeyCode::KeyK)
        );
        assert!(!controls.has_conflict(ControlAction::LineTool));
        assert!(!controls.has_conflict(ControlAction::Ledger));
    }

    #[test]
    fn no_window_key_is_also_a_gameplay_verb() {
        // 03 §10.2: "Window keys avoid every key a gameplay verb already owns;
        // a test asserts it." This is that test, now that both halves of the
        // table live in one place.
        let controls = ControlSettings::default();
        let windows: Vec<ControlAction> = ControlAction::ALL
            .iter()
            .copied()
            .filter(|a| a.group() == ControlGroup::Windows)
            .collect();
        assert_eq!(windows.len(), 6, "six windows carry a key");
        for window in windows {
            let binding = controls.key_for(window);
            for other in ControlAction::ALL {
                if *other == window || other.group() == ControlGroup::Windows {
                    continue;
                }
                assert_ne!(
                    controls.key_for(*other),
                    binding,
                    "{:?} steals {} from {:?}",
                    window,
                    binding.label(),
                    other
                );
            }
        }
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
    fn an_old_profile_does_not_reload_the_l_clash() {
        // Every settings file written before the fix says `Ledger: KeyL`. Read
        // back literally, the conflict returns on the next launch and the tab
        // reports something the player never did.
        let parsed = KvDoc::parse("(\n  controls_Ledger: \"KeyL\",\n)");
        let restored = ControlSettings::read_from(&parsed);
        assert_eq!(
            restored.key_for(ControlAction::Ledger),
            Binding::key(KeyCode::KeyK)
        );
        assert!(restored.conflicts().is_empty());
    }

    #[test]
    fn a_deliberate_rebind_onto_a_retired_key_is_still_the_players() {
        // Only the retired *default* is dropped. Someone who genuinely wants
        // the Ledger on `M` keeps it, conflict or not — the tab flags it and
        // the Reset button is right there.
        let parsed = KvDoc::parse("(\n  controls_Ledger: \"KeyM\",\n)");
        let restored = ControlSettings::read_from(&parsed);
        assert_eq!(
            restored.key_for(ControlAction::Ledger),
            Binding::key(KeyCode::KeyM)
        );
        assert!(restored.has_conflict(ControlAction::Ledger));
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
