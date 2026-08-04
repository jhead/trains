//! Settings model, disk persistence, and the systems that apply them live.
//!
//! Four groups, matching the tabs in
//! [`docs/design/09-shell-and-menus.md`](../../../docs/design/09-shell-and-menus.md) §5.
//! Accessibility is spread across the groups rather than quarantined, as the
//! brief requires.
//!
//! Every row is described by a [`SettingId`], which knows its tab, its label, how
//! to render its current value, and how to cycle it. The panel is then generic
//! over `SettingId::ALL` — adding a setting is one enum variant plus three match
//! arms, and no UI code changes.
//!
//! **Honesty rule (design §5 / acceptance bar 4):** a setting either applies live
//! or says plainly that it does not. [`SettingId::pending_note`] returns the note
//! shown beside rows whose consumer does not exist yet; it is not decoration, and
//! it should be deleted as each consumer lands.

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, WindowMode};

use super::persist::{self, KvDoc, ParsedKv};

/// Settings tabs, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Display,
    Audio,
    Gameplay,
    Controls,
}

impl SettingsTab {
    pub const ALL: &'static [Self] = &[Self::Display, Self::Audio, Self::Gameplay, Self::Controls];

    pub fn label(self) -> &'static str {
        match self {
            Self::Display => "Display",
            Self::Audio => "Audio",
            Self::Gameplay => "Gameplay",
            Self::Controls => "Controls",
        }
    }
}

/// Window presentation choice. Exclusive fullscreen is deliberately omitted —
/// it buys nothing for a 2D pixel game and costs alt-tab reliability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowModeChoice {
    #[default]
    Windowed,
    Borderless,
}

impl WindowModeChoice {
    const ALL: &'static [Self] = &[Self::Windowed, Self::Borderless];

    pub fn label(self) -> &'static str {
        match self {
            Self::Windowed => "Windowed",
            Self::Borderless => "Borderless",
        }
    }

    fn to_window_mode(self) -> WindowMode {
        match self {
            Self::Windowed => WindowMode::Windowed,
            Self::Borderless => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
        }
    }
}

/// How much the town says. Design §5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TownTalkVerbosity {
    Quiet,
    #[default]
    Normal,
    Chatty,
}

impl TownTalkVerbosity {
    const ALL: &'static [Self] = &[Self::Quiet, Self::Normal, Self::Chatty];

    pub fn label(self) -> &'static str {
        match self {
            Self::Quiet => "Quiet",
            Self::Normal => "Normal",
            Self::Chatty => "Chatty",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplaySettings {
    pub window_mode: WindowModeChoice,
    /// Integer UI scale, `1`–`3`, or `0` for [`UI_SCALE_AUTO`].
    ///
    /// **Never fractional** (design 03 §2). Every metric in the kit is a whole
    /// number of texels, so a whole-number scale keeps every border, gap and
    /// glyph on whole pixels; `1.25×` would put a 1-texel border on 1.25 px and
    /// undo the pixel contract at a stroke.
    ///
    /// The default is `Auto`, which reads the window's **logical** size — that
    /// is already corrected for display density, so a HiDPI screen does not need
    /// a bigger number here. On the shipping 1280×720 window Auto resolves to
    /// `1×`, which is where the playtest's "at least 25% smaller" lands.
    pub ui_scale: u32,
    /// World zoom a new game starts at, 1×–3×.
    pub world_zoom_default: u32,
    /// Draw the world in 2:1 dimetric instead of from directly above.
    ///
    /// A view mode, not a world property: the same world and the same save read
    /// either way, and flipping it mid-session is a presentation rebuild and
    /// nothing else (`map::projection`). It lives here because the settings file
    /// is a flat key-value document with no schema — an absent key reads as the
    /// default, so an old profile opens top-down and an older build ignores the
    /// key entirely. Nothing about it reaches a save.
    pub isometric: bool,
    pub vsync: bool,
    /// `0` means uncapped.
    pub frame_cap: u32,
    pub tile_grid: bool,
    pub edge_pan: bool,
    /// Accessibility: disables tweens and the title-screen world drift.
    pub reduced_motion: bool,
    /// Accessibility: colour-blind-safe palette variant.
    pub colour_blind_safe: bool,
    /// Accessibility: allow flashes and screen shake at all.
    pub flashes_and_shake: bool,
}

/// Sentinel for "pick the UI scale from the window size".
pub const UI_SCALE_AUTO: u32 = 0;

/// Largest scale the ladder offers.
pub const UI_SCALE_MAX: u32 = 4;

/// Resolve [`DisplaySettings::ui_scale`] against a window's logical size.
///
/// Logical size is already divided by the display's scale factor, so this is a
/// judgement about how much room there is, not about pixel density. The
/// thresholds are generous: too-small chrome is a much worse first impression
/// than slightly-small chrome, and the setting is right there either way.
pub fn resolve_ui_scale(setting: u32, logical_width: f32, logical_height: f32) -> u32 {
    if setting != UI_SCALE_AUTO {
        return setting.clamp(1, UI_SCALE_MAX);
    }
    if logical_height >= 2000.0 || logical_width >= 3200.0 {
        3
    } else if logical_height >= 1300.0 || logical_width >= 2100.0 {
        2
    } else {
        1
    }
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            window_mode: WindowModeChoice::Windowed,
            ui_scale: UI_SCALE_AUTO,
            world_zoom_default: 2,
            isometric: false,
            vsync: true,
            frame_cap: 0,
            tile_grid: false,
            edge_pan: true,
            reduced_motion: false,
            colour_blind_safe: false,
            flashes_and_shake: true,
        }
    }
}

/// Volumes are whole percentages so the readout never shows a fractional number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioSettings {
    pub master: u32,
    pub music: u32,
    pub ambience: u32,
    pub effects: u32,
    pub ui: u32,
    pub mute_on_focus_loss: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            master: 80,
            music: 70,
            ambience: 70,
            effects: 80,
            ui: 60,
            mute_on_focus_loss: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameplaySettings {
    /// Minutes between autosaves; `0` disables autosave.
    pub autosave_minutes: u32,
    pub tooltip_delay_ms: u32,
    pub confirm_destructive: bool,
    pub show_cost_while_building: bool,
    /// Default off, per design §5.
    pub pause_on_alert: bool,
    pub town_talk: TownTalkVerbosity,
}

impl Default for GameplaySettings {
    fn default() -> Self {
        Self {
            autosave_minutes: 5,
            // Design 03 §8.3: tooltips appear after 400 ms of hover.
            tooltip_delay_ms: 400,
            confirm_destructive: true,
            show_cost_while_building: true,
            pause_on_alert: false,
            town_talk: TownTalkVerbosity::Normal,
        }
    }
}

/// Everything the player can change, in one resource.
#[derive(Resource, Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    pub display: DisplaySettings,
    pub audio: AudioSettings,
    pub gameplay: GameplaySettings,
    pub controls: super::controls::ControlSettings,
}

/// One row on a settings tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingId {
    // Display
    WindowMode,
    UiScale,
    WorldZoomDefault,
    Projection,
    Vsync,
    FrameCap,
    TileGrid,
    EdgePan,
    ReducedMotion,
    ColourBlindSafe,
    FlashesAndShake,
    // Audio
    VolumeMaster,
    VolumeMusic,
    VolumeAmbience,
    VolumeEffects,
    VolumeUi,
    MuteOnFocusLoss,
    // Gameplay
    AutosaveMinutes,
    TooltipDelay,
    ConfirmDestructive,
    ShowCostWhileBuilding,
    PauseOnAlert,
    TownTalk,
    // Controls
    HoldRepeat,
}

impl SettingId {
    pub const ALL: &'static [Self] = &[
        Self::WindowMode,
        Self::UiScale,
        Self::WorldZoomDefault,
        Self::Projection,
        Self::Vsync,
        Self::FrameCap,
        Self::TileGrid,
        Self::EdgePan,
        Self::ReducedMotion,
        Self::ColourBlindSafe,
        Self::FlashesAndShake,
        Self::VolumeMaster,
        Self::VolumeMusic,
        Self::VolumeAmbience,
        Self::VolumeEffects,
        Self::VolumeUi,
        Self::MuteOnFocusLoss,
        Self::AutosaveMinutes,
        Self::TooltipDelay,
        Self::ConfirmDestructive,
        Self::ShowCostWhileBuilding,
        Self::PauseOnAlert,
        Self::TownTalk,
        Self::HoldRepeat,
    ];

    pub fn tab(self) -> SettingsTab {
        match self {
            Self::WindowMode
            | Self::UiScale
            | Self::WorldZoomDefault
            | Self::Projection
            | Self::Vsync
            | Self::FrameCap
            | Self::TileGrid
            | Self::EdgePan
            | Self::ReducedMotion
            | Self::ColourBlindSafe
            | Self::FlashesAndShake => SettingsTab::Display,
            Self::VolumeMaster
            | Self::VolumeMusic
            | Self::VolumeAmbience
            | Self::VolumeEffects
            | Self::VolumeUi
            | Self::MuteOnFocusLoss => SettingsTab::Audio,
            Self::AutosaveMinutes
            | Self::TooltipDelay
            | Self::ConfirmDestructive
            | Self::ShowCostWhileBuilding
            | Self::PauseOnAlert
            | Self::TownTalk => SettingsTab::Gameplay,
            Self::HoldRepeat => SettingsTab::Controls,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::WindowMode => "Window",
            Self::UiScale => "UI scale",
            Self::WorldZoomDefault => "World zoom",
            Self::Projection => "World view",
            Self::Vsync => "Vsync",
            Self::FrameCap => "Frame cap",
            Self::TileGrid => "Tile grid",
            Self::EdgePan => "Edge pan",
            Self::ReducedMotion => "Reduced motion",
            Self::ColourBlindSafe => "Colour-blind safe",
            Self::FlashesAndShake => "Flashes & shake",
            Self::VolumeMaster => "Master",
            Self::VolumeMusic => "Music",
            Self::VolumeAmbience => "Ambience",
            Self::VolumeEffects => "Effects",
            Self::VolumeUi => "UI",
            Self::MuteOnFocusLoss => "Mute unfocused",
            Self::AutosaveMinutes => "Autosave",
            Self::TooltipDelay => "Tooltip delay",
            Self::ConfirmDestructive => "Confirm destructive",
            Self::ShowCostWhileBuilding => "Show cost while building",
            Self::PauseOnAlert => "Pause on alert",
            Self::TownTalk => "Town Talk",
            Self::HoldRepeat => "Hold-to-repeat",
        }
    }

    /// Current value, rendered for the row.
    pub fn value_label(self, settings: &Settings) -> String {
        let d = &settings.display;
        let a = &settings.audio;
        let g = &settings.gameplay;
        match self {
            Self::WindowMode => d.window_mode.label().into(),
            Self::UiScale => {
                if d.ui_scale == UI_SCALE_AUTO {
                    "Auto".into()
                } else {
                    format!("{}x", d.ui_scale)
                }
            }
            Self::WorldZoomDefault => format!("{}x", d.world_zoom_default),
            Self::Projection => crate::map::projection::projection_for(d.isometric)
                .label()
                .into(),
            Self::Vsync => on_off(d.vsync),
            Self::FrameCap => {
                if d.frame_cap == 0 {
                    "Uncapped".into()
                } else {
                    format!("{} fps", d.frame_cap)
                }
            }
            Self::TileGrid => on_off(d.tile_grid),
            Self::EdgePan => on_off(d.edge_pan),
            Self::ReducedMotion => on_off(d.reduced_motion),
            Self::ColourBlindSafe => on_off(d.colour_blind_safe),
            Self::FlashesAndShake => on_off(d.flashes_and_shake),
            Self::VolumeMaster => format!("{}%", a.master),
            Self::VolumeMusic => format!("{}%", a.music),
            Self::VolumeAmbience => format!("{}%", a.ambience),
            Self::VolumeEffects => format!("{}%", a.effects),
            Self::VolumeUi => format!("{}%", a.ui),
            Self::MuteOnFocusLoss => on_off(a.mute_on_focus_loss),
            Self::AutosaveMinutes => {
                if g.autosave_minutes == 0 {
                    "Off".into()
                } else {
                    format!("{} min", g.autosave_minutes)
                }
            }
            Self::TooltipDelay => format!("{} ms", g.tooltip_delay_ms),
            Self::ConfirmDestructive => on_off(g.confirm_destructive),
            Self::ShowCostWhileBuilding => on_off(g.show_cost_while_building),
            Self::PauseOnAlert => on_off(g.pause_on_alert),
            Self::TownTalk => g.town_talk.label().into(),
            Self::HoldRepeat => format!("{} ms", settings.controls.hold_repeat_ms),
        }
    }

    /// Step the value. `delta` is `-1` or `+1`; every row wraps.
    pub fn cycle(self, settings: &mut Settings, delta: i32) {
        let d = &mut settings.display;
        let a = &mut settings.audio;
        let g = &mut settings.gameplay;
        match self {
            Self::WindowMode => {
                d.window_mode = cycle_list(WindowModeChoice::ALL, d.window_mode, delta)
            }
            // The ladder includes Auto, so the row wraps Auto → 1× → … → 4×.
            Self::UiScale => d.ui_scale = cycle_range(d.ui_scale, UI_SCALE_AUTO, UI_SCALE_MAX, 1, delta),
            Self::WorldZoomDefault => {
                d.world_zoom_default = cycle_range(d.world_zoom_default, 1, 3, 1, delta)
            }
            // Two members, so either direction is the same flip.
            Self::Projection => d.isometric = !d.isometric,
            Self::Vsync => d.vsync = !d.vsync,
            Self::FrameCap => {
                d.frame_cap = cycle_values(&[0, 60, 90, 120, 144, 240], d.frame_cap, delta)
            }
            Self::TileGrid => d.tile_grid = !d.tile_grid,
            Self::EdgePan => d.edge_pan = !d.edge_pan,
            Self::ReducedMotion => d.reduced_motion = !d.reduced_motion,
            Self::ColourBlindSafe => d.colour_blind_safe = !d.colour_blind_safe,
            Self::FlashesAndShake => d.flashes_and_shake = !d.flashes_and_shake,
            Self::VolumeMaster => a.master = cycle_range(a.master, 0, 100, 10, delta),
            Self::VolumeMusic => a.music = cycle_range(a.music, 0, 100, 10, delta),
            Self::VolumeAmbience => a.ambience = cycle_range(a.ambience, 0, 100, 10, delta),
            Self::VolumeEffects => a.effects = cycle_range(a.effects, 0, 100, 10, delta),
            Self::VolumeUi => a.ui = cycle_range(a.ui, 0, 100, 10, delta),
            Self::MuteOnFocusLoss => a.mute_on_focus_loss = !a.mute_on_focus_loss,
            Self::AutosaveMinutes => {
                g.autosave_minutes = cycle_values(&[0, 2, 5, 10, 15, 30], g.autosave_minutes, delta)
            }
            Self::TooltipDelay => {
                g.tooltip_delay_ms =
                    cycle_values(&[200, 300, 400, 600, 800], g.tooltip_delay_ms, delta)
            }
            Self::ConfirmDestructive => g.confirm_destructive = !g.confirm_destructive,
            Self::ShowCostWhileBuilding => g.show_cost_while_building = !g.show_cost_while_building,
            Self::PauseOnAlert => g.pause_on_alert = !g.pause_on_alert,
            Self::TownTalk => g.town_talk = cycle_list(TownTalkVerbosity::ALL, g.town_talk, delta),
            Self::HoldRepeat => {
                settings.controls.hold_repeat_ms = cycle_values(
                    &[80, 120, 180, 250, 400],
                    settings.controls.hold_repeat_ms,
                    delta,
                )
            }
        }
    }

    /// `Some(fill)` when this row should draw the kit's meter beside its value.
    ///
    /// The five volume rows, and only those: a level is a *quantity*, and a
    /// quantity the player is trying to balance against four others is far
    /// easier to judge as a bar than as five numbers read one at a time. Every
    /// other row on every tab is a choice from a short list, where a meter would
    /// mean nothing.
    pub fn meter_percent(self, settings: &Settings) -> Option<u32> {
        let a = &settings.audio;
        match self {
            Self::VolumeMaster => Some(a.master),
            Self::VolumeMusic => Some(a.music),
            Self::VolumeAmbience => Some(a.ambience),
            Self::VolumeEffects => Some(a.effects),
            Self::VolumeUi => Some(a.ui),
            _ => None,
        }
    }

    /// `Some(note)` when this row is stored but nothing consumes it yet.
    ///
    /// Delete the arm as each consumer lands — an empty match here is the goal.
    pub fn pending_note(self) -> Option<&'static str> {
        match self {
            Self::WorldZoomDefault => Some("on next new game"),
            Self::FrameCap => Some("not wired yet"),
            Self::TileGrid => Some("not wired yet"),
            Self::EdgePan => Some("not wired yet"),
            Self::ColourBlindSafe => Some("not wired yet"),
            Self::FlashesAndShake => Some("not wired yet"),
            Self::TooltipDelay => Some("not wired yet"),
            Self::ConfirmDestructive => Some("not wired yet"),
            Self::ShowCostWhileBuilding => Some("not wired yet"),
            Self::PauseOnAlert => Some("not wired yet"),
            Self::TownTalk => Some("not wired yet"),
            Self::HoldRepeat => Some("not wired yet"),
            _ => None,
        }
    }
}

fn on_off(value: bool) -> String {
    // Never colour alone (design 03 §4): the word carries the state.
    if value {
        "On".into()
    } else {
        "Off".into()
    }
}

fn cycle_list<T: Copy + PartialEq>(all: &[T], current: T, delta: i32) -> T {
    if all.is_empty() {
        return current;
    }
    let index = all.iter().position(|v| *v == current).unwrap_or(0) as i32;
    let next = (index + delta).rem_euclid(all.len() as i32) as usize;
    all[next]
}

fn cycle_values(all: &[u32], current: u32, delta: i32) -> u32 {
    cycle_list(all, current, delta)
}

fn cycle_range(current: u32, min: u32, max: u32, step: u32, delta: i32) -> u32 {
    let span = (max - min) / step + 1;
    let index = (current.saturating_sub(min) / step) as i32;
    let next = (index + delta).rem_euclid(span as i32) as u32;
    min + next * step
}

// ─ Persistence ─────────────────────────────────────────────

impl Settings {
    /// Load from disk, falling back to defaults for anything absent.
    pub fn load() -> Self {
        match persist::load_settings_doc() {
            Some(doc) => Self::from_doc(&doc),
            None => Self::default(),
        }
    }

    /// Write to disk. Errors are returned, never panicked on — a read-only
    /// profile must not stop the game.
    pub fn save(&self) -> std::io::Result<()> {
        persist::save_settings_doc(&self.to_doc())
    }

    pub fn to_doc(&self) -> KvDoc {
        let mut doc = KvDoc::new();
        let d = &self.display;
        doc.set_str("display_window_mode", d.window_mode.label());
        doc.set_int("display_ui_scale", d.ui_scale as i64);
        doc.set_int("display_world_zoom_default", d.world_zoom_default as i64);
        doc.set_bool("display_isometric", d.isometric);
        doc.set_bool("display_vsync", d.vsync);
        doc.set_int("display_frame_cap", d.frame_cap as i64);
        doc.set_bool("display_tile_grid", d.tile_grid);
        doc.set_bool("display_edge_pan", d.edge_pan);
        doc.set_bool("display_reduced_motion", d.reduced_motion);
        doc.set_bool("display_colour_blind_safe", d.colour_blind_safe);
        doc.set_bool("display_flashes_and_shake", d.flashes_and_shake);

        let a = &self.audio;
        doc.set_int("audio_master", a.master as i64);
        doc.set_int("audio_music", a.music as i64);
        doc.set_int("audio_ambience", a.ambience as i64);
        doc.set_int("audio_effects", a.effects as i64);
        doc.set_int("audio_ui", a.ui as i64);
        doc.set_bool("audio_mute_on_focus_loss", a.mute_on_focus_loss);

        let g = &self.gameplay;
        doc.set_int("gameplay_autosave_minutes", g.autosave_minutes as i64);
        doc.set_int("gameplay_tooltip_delay_ms", g.tooltip_delay_ms as i64);
        doc.set_bool("gameplay_confirm_destructive", g.confirm_destructive);
        doc.set_bool("gameplay_show_cost", g.show_cost_while_building);
        doc.set_bool("gameplay_pause_on_alert", g.pause_on_alert);
        doc.set_str("gameplay_town_talk", g.town_talk.label());

        self.controls.write_to(&mut doc);
        doc
    }

    pub fn from_doc(doc: &ParsedKv) -> Self {
        let base = Self::default();
        let d = base.display;
        let a = base.audio;
        let g = base.gameplay;
        Self {
            display: DisplaySettings {
                window_mode: label_of(
                    WindowModeChoice::ALL,
                    doc.str("display_window_mode", d.window_mode.label()),
                    WindowModeChoice::label,
                )
                .unwrap_or(d.window_mode),
                ui_scale: doc
                    .int("display_ui_scale", d.ui_scale as i64)
                    .clamp(UI_SCALE_AUTO as i64, UI_SCALE_MAX as i64)
                    as u32,
                world_zoom_default: doc
                    .int("display_world_zoom_default", d.world_zoom_default as i64)
                    .clamp(1, 3) as u32,
                isometric: doc.bool("display_isometric", d.isometric),
                vsync: doc.bool("display_vsync", d.vsync),
                frame_cap: doc
                    .int("display_frame_cap", d.frame_cap as i64)
                    .clamp(0, 480) as u32,
                tile_grid: doc.bool("display_tile_grid", d.tile_grid),
                edge_pan: doc.bool("display_edge_pan", d.edge_pan),
                reduced_motion: doc.bool("display_reduced_motion", d.reduced_motion),
                colour_blind_safe: doc.bool("display_colour_blind_safe", d.colour_blind_safe),
                flashes_and_shake: doc.bool("display_flashes_and_shake", d.flashes_and_shake),
            },
            audio: AudioSettings {
                master: volume(doc.int("audio_master", a.master as i64)),
                music: volume(doc.int("audio_music", a.music as i64)),
                ambience: volume(doc.int("audio_ambience", a.ambience as i64)),
                effects: volume(doc.int("audio_effects", a.effects as i64)),
                ui: volume(doc.int("audio_ui", a.ui as i64)),
                mute_on_focus_loss: doc.bool("audio_mute_on_focus_loss", a.mute_on_focus_loss),
            },
            gameplay: GameplaySettings {
                autosave_minutes: doc
                    .int("gameplay_autosave_minutes", g.autosave_minutes as i64)
                    .clamp(0, 120) as u32,
                tooltip_delay_ms: doc
                    .int("gameplay_tooltip_delay_ms", g.tooltip_delay_ms as i64)
                    .clamp(0, 2000) as u32,
                confirm_destructive: doc
                    .bool("gameplay_confirm_destructive", g.confirm_destructive),
                show_cost_while_building: doc
                    .bool("gameplay_show_cost", g.show_cost_while_building),
                pause_on_alert: doc.bool("gameplay_pause_on_alert", g.pause_on_alert),
                town_talk: label_of(
                    TownTalkVerbosity::ALL,
                    doc.str("gameplay_town_talk", g.town_talk.label()),
                    TownTalkVerbosity::label,
                )
                .unwrap_or(g.town_talk),
            },
            controls: super::controls::ControlSettings::read_from(doc),
        }
    }
}

fn volume(raw: i64) -> u32 {
    raw.clamp(0, 100) as u32
}

fn label_of<T: Copy>(all: &[T], needle: &str, label: fn(T) -> &'static str) -> Option<T> {
    all.iter().copied().find(|v| label(*v) == needle)
}

// ─ Live application ────────────────────────────────────────

/// Window mode, vsync and UI scale, applied the moment they change.
///
/// UI scale is re-resolved on a window resize as well as on a settings change,
/// because `Auto` is a function of the window's size.
pub fn apply_display_settings(
    settings: Res<Settings>,
    mut ui_scale: ResMut<UiScale>,
    mut windows: Query<&mut Window>,
    mut last_size: Local<(f32, f32)>,
) {
    let size = windows
        .iter()
        .next()
        .map(|w| (w.width(), w.height()))
        .unwrap_or((1280.0, 720.0));
    let resized = size != *last_size;
    if !settings.is_changed() && !resized {
        return;
    }
    *last_size = size;
    let target = resolve_ui_scale(settings.display.ui_scale, size.0, size.1) as f32;
    if ui_scale.0 != target {
        ui_scale.0 = target;
    }

    let mode = settings.display.window_mode.to_window_mode();
    let present = if settings.display.vsync {
        PresentMode::AutoVsync
    } else {
        PresentMode::AutoNoVsync
    };
    for mut window in &mut windows {
        if window.mode != mode {
            window.mode = mode;
        }
        if window.present_mode != present {
            window.present_mode = present;
        }
    }
}

/// Master volume, plus mute-on-focus-loss. Runs every frame because focus can
/// change without the settings changing.
pub fn apply_audio_settings(
    settings: Res<Settings>,
    windows: Query<&Window>,
    volume: Option<ResMut<bevy::audio::GlobalVolume>>,
) {
    let Some(mut volume) = volume else {
        return;
    };
    let focused = windows.iter().any(|w| w.focused);
    let muted = settings.audio.mute_on_focus_loss && !focused;
    let level = if muted {
        0.0
    } else {
        settings.audio.master as f32 / 100.0
    };
    let current = volume.volume.to_linear();
    if (current - level).abs() > f32::EPSILON {
        volume.volume = bevy::audio::Volume::Linear(level);
    }
}

/// Persist to disk whenever the settings change, so nothing is lost on a crash.
pub fn persist_settings_on_change(settings: Res<Settings>) {
    if !settings.is_changed() || settings.is_added() {
        return;
    }
    if let Err(err) = settings.save() {
        warn!("could not write settings: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setting_belongs_to_exactly_one_tab_and_has_a_label() {
        for id in SettingId::ALL {
            assert!(!id.label().is_empty(), "{id:?} has no label");
            assert!(SettingsTab::ALL.contains(&id.tab()), "{id:?} has no tab");
        }
        // Every tab has at least one row, so no tab opens empty.
        for tab in SettingsTab::ALL {
            let rows = SettingId::ALL.iter().filter(|id| id.tab() == *tab).count();
            if *tab == SettingsTab::Controls {
                continue; // Controls is mostly the binding list, checked separately.
            }
            assert!(rows > 0, "{} tab has no rows", tab.label());
        }
    }

    #[test]
    fn cycling_forward_then_back_returns_the_original_value() {
        for id in SettingId::ALL {
            let mut settings = Settings::default();
            let before = id.value_label(&settings);
            id.cycle(&mut settings, 1);
            let stepped = id.value_label(&settings);
            id.cycle(&mut settings, -1);
            assert_eq!(
                id.value_label(&settings),
                before,
                "{id:?} did not return to its original value"
            );
            assert_ne!(stepped, before, "{id:?} did not change when cycled");
        }
    }

    #[test]
    fn every_row_wraps_rather_than_sticking() {
        for id in SettingId::ALL {
            let mut settings = Settings::default();
            let start = id.value_label(&settings);
            // Twelve steps is more than the longest list; it must come home.
            let mut seen_start_again = false;
            for _ in 0..12 {
                id.cycle(&mut settings, 1);
                if id.value_label(&settings) == start {
                    seen_start_again = true;
                    break;
                }
            }
            assert!(seen_start_again, "{id:?} does not wrap");
        }
    }

    #[test]
    fn settings_round_trip_through_the_file_format() {
        let mut settings = Settings::default();
        settings.display.ui_scale = 3;
        settings.display.window_mode = WindowModeChoice::Borderless;
        settings.display.vsync = false;
        settings.audio.master = 30;
        settings.audio.mute_on_focus_loss = false;
        settings.gameplay.autosave_minutes = 15;
        settings.gameplay.town_talk = TownTalkVerbosity::Chatty;

        let restored = Settings::from_doc(&KvDoc::parse(&settings.to_doc().to_ron()));
        assert_eq!(restored, settings);
    }

    #[test]
    fn a_corrupt_file_degrades_to_defaults_instead_of_failing() {
        let restored = Settings::from_doc(&KvDoc::parse("(\n  display_ui_scale: 99,\n  junk\n)"));
        assert_eq!(
            restored.display.ui_scale, UI_SCALE_MAX,
            "out-of-range value is clamped"
        );
        assert_eq!(restored.audio, AudioSettings::default());
    }

    #[test]
    fn ui_scale_is_only_ever_a_whole_number() {
        // Design 03 §2. A fractional scale puts 1-texel borders on fractions of
        // a pixel, which is the single fastest way to make a pixel game look
        // cheap — so the ladder has no half steps to reach for.
        let mut settings = Settings::default();
        for _ in 0..12 {
            SettingId::UiScale.cycle(&mut settings, 1);
            let scale = settings.display.ui_scale;
            assert!(scale <= UI_SCALE_MAX);
            let resolved = resolve_ui_scale(scale, 1280.0, 720.0);
            assert!((1..=UI_SCALE_MAX).contains(&resolved), "{resolved}");
        }
    }

    #[test]
    fn the_default_ui_scale_is_at_least_a_quarter_smaller_than_it_was() {
        // The playtest asked for "at least a 25% reduction in scale". The old
        // default was a hard 2x; Auto resolves to 1x on the shipping window,
        // and the row still offers 2x-4x for anyone who wants it back.
        assert_eq!(DisplaySettings::default().ui_scale, UI_SCALE_AUTO);
        assert_eq!(resolve_ui_scale(UI_SCALE_AUTO, 1280.0, 720.0), 1);
        assert_eq!(resolve_ui_scale(UI_SCALE_AUTO, 640.0, 360.0), 1);
    }

    #[test]
    fn auto_grows_on_a_genuinely_large_desktop() {
        // Logical size is already density-corrected, so this is about room, not
        // about a retina panel.
        assert_eq!(resolve_ui_scale(UI_SCALE_AUTO, 2560.0, 1440.0), 2);
        assert_eq!(resolve_ui_scale(UI_SCALE_AUTO, 3840.0, 2160.0), 3);
    }

    #[test]
    fn an_explicit_scale_ignores_the_window_entirely() {
        for scale in 1..=UI_SCALE_MAX {
            assert_eq!(resolve_ui_scale(scale, 640.0, 360.0), scale);
            assert_eq!(resolve_ui_scale(scale, 3840.0, 2160.0), scale);
        }
    }

    #[test]
    fn pause_on_alert_is_off_by_default() {
        assert!(!GameplaySettings::default().pause_on_alert);
    }

    #[test]
    fn every_bus_has_a_slider_and_nothing_else_does() {
        // Design 09 §5: master, music, ambience, effects and UI, each with a
        // level the player can see rather than infer from a number.
        let settings = Settings::default();
        let metered: Vec<SettingId> = SettingId::ALL
            .iter()
            .copied()
            .filter(|id| id.meter_percent(&settings).is_some())
            .collect();
        assert_eq!(
            metered,
            vec![
                SettingId::VolumeMaster,
                SettingId::VolumeMusic,
                SettingId::VolumeAmbience,
                SettingId::VolumeEffects,
                SettingId::VolumeUi,
            ]
        );
        for id in &metered {
            assert_eq!(id.tab(), SettingsTab::Audio, "{id:?} is not an Audio row");
        }
    }

    #[test]
    fn a_slider_reads_the_value_it_is_drawn_beside() {
        // The bar and the numeral come from one place, so they cannot disagree.
        let mut settings = Settings::default();
        SettingId::VolumeMusic.cycle(&mut settings, -1);
        let percent = SettingId::VolumeMusic
            .meter_percent(&settings)
            .expect("music is a slider");
        assert_eq!(settings.audio.music, percent);
        assert_eq!(
            SettingId::VolumeMusic.value_label(&settings),
            format!("{percent}%")
        );
        assert!(percent <= 100, "a meter cannot overfill");
    }

    #[test]
    fn no_audio_row_claims_to_be_unwired() {
        // The honesty rule (§5 / acceptance bar 4) run the other way: every
        // volume now reaches `audio::AudioMix`, so none of them may still carry
        // a pending note.
        for id in SettingId::ALL.iter().filter(|id| id.tab() == SettingsTab::Audio) {
            assert_eq!(
                id.pending_note(),
                None,
                "{id:?} still says it does nothing"
            );
        }
    }
}
