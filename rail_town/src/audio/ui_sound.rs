//! Interface sound (brief §5) — quiet, low, soft-edged, felt more than heard.
//!
//! ## The money sound
//!
//! The brief is explicit and it is the most important rule in this file: income
//! arrives constantly in a working network, and an unthrottled chime turns it
//! into a coin-drop machine. So the balance is **aggregated over a window and
//! played once**: whatever happened in the last few seconds becomes at most one
//! sound, whose level barely varies with the amount. A network earning ten times
//! as much does not chime ten times as often.
//!
//! The first payout of a session is the one exception the brief allows itself
//! (§6): it gets the warm milestone sound rather than the ordinary chime,
//! because that is the loop closing for the first time.
//!
//! Other modules can ask for interface sound by writing a [`UiCue`] message;
//! nothing in this module reaches into anyone else's state.

use bevy::prelude::*;
use rail_sim::{AlertBoard, Money};

use crate::map::MapViewState;

use super::bank::SfxBank;
use super::mixer::{gain, play, AudioClock, AudioMix, Cue, Duck, VoiceBudget};

/// Aggregation window for money. Long enough that a busy network chimes on a
/// human rhythm, short enough that a payout still feels like a response.
const MONEY_WINDOW_SECS: f32 = 3.5;

/// Below this many cents of net movement, nothing sounds at all.
const MONEY_FLOOR_CENTS: i64 = 500;

/// Minimum spacing between alert tones.
const ALERT_COOLDOWN_SECS: f32 = 6.0;

/// Minimum spacing between interface clicks.
const CLICK_COOLDOWN_SECS: f32 = 0.05;

/// Interface sounds any module may request.
///
/// This is the module's only inbound API. Panels, toggles and milestones are
/// owned by the slices that draw them, so they ask rather than being watched.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
// Constructed by the slices that own panels and toggles, not by this module.
#[allow(dead_code)]
pub enum UiCue {
    /// A soft low tick.
    Click,
    /// A brief airy sweep, up.
    PanelOpen,
    /// The same sweep, down.
    PanelClose,
    ToggleOn,
    ToggleOff,
    /// The one genuinely warm moment. Rare enough to stay special.
    Milestone,
}

#[derive(Resource, Debug)]
pub struct UiAudio {
    last_click: f64,
    last_alert: f64,
    /// Balance at the start of the open aggregation window.
    window_opened_at: f64,
    window_start_cents: i64,
    cents_seeded: bool,
    first_income_done: bool,
    alerts: Vec<u64>,
    alerts_seeded: bool,
    map_view: bool,
    map_view_seeded: bool,
}

impl Default for UiAudio {
    fn default() -> Self {
        Self {
            last_click: f64::NEG_INFINITY,
            last_alert: f64::NEG_INFINITY,
            window_opened_at: 0.0,
            window_start_cents: 0,
            cents_seeded: false,
            first_income_done: false,
            alerts: Vec::new(),
            alerts_seeded: false,
            map_view: false,
            map_view_seeded: false,
        }
    }
}

/// Play whatever anyone asked for.
pub fn play_ui_cues(
    mut cues: MessageReader<UiCue>,
    mix: Res<AudioMix>,
    bank: Option<Res<SfxBank>>,
    mut budget: ResMut<VoiceBudget>,
    mut commands: Commands,
) {
    let Some(bank) = bank else {
        return;
    };
    for cue in cues.read() {
        let (clip, level) = match cue {
            UiCue::Click => (bank.ui_click.near(0), gain::UI_CLICK),
            UiCue::PanelOpen => (bank.panel_open.near(0), gain::PANEL),
            UiCue::PanelClose => (bank.panel_close.near(0), gain::PANEL),
            UiCue::ToggleOn => (bank.toggle_on.near(0), gain::TOGGLE),
            UiCue::ToggleOff => (bank.toggle_off.near(0), gain::TOGGLE),
            UiCue::Milestone => (bank.milestone.near(0), gain::MILESTONE),
        };
        play(&mut commands, &mut budget, &mix, Cue::ui(clip, level));
    }
}

/// A soft tick on any button press. Hover is deliberately silent (§5).
pub fn button_clicks(
    interactions: Query<&Interaction, (Changed<Interaction>, With<Button>)>,
    clock: Res<AudioClock>,
    mix: Res<AudioMix>,
    bank: Option<Res<SfxBank>>,
    mut audio: ResMut<UiAudio>,
    mut budget: ResMut<VoiceBudget>,
    mut commands: Commands,
) {
    let Some(bank) = bank else {
        return;
    };
    let pressed = interactions
        .iter()
        .any(|i| matches!(i, Interaction::Pressed));
    if !pressed {
        return;
    }
    let now = clock.elapsed;
    if now - audio.last_click < CLICK_COOLDOWN_SECS as f64 {
        return;
    }
    audio.last_click = now;
    play(
        &mut commands,
        &mut budget,
        &mix,
        Cue::ui(bank.ui_click.near(0), gain::UI_CLICK),
    );
}

/// Map View is the one panel this module can see without reaching into another
/// slice, and it is the one that most wants a sweep.
pub fn map_view_sweep(
    state: Option<Res<MapViewState>>,
    mix: Res<AudioMix>,
    bank: Option<Res<SfxBank>>,
    mut audio: ResMut<UiAudio>,
    mut budget: ResMut<VoiceBudget>,
    mut commands: Commands,
) {
    let (Some(state), Some(bank)) = (state, bank) else {
        return;
    };
    if !audio.map_view_seeded {
        audio.map_view = state.active;
        audio.map_view_seeded = true;
        return;
    }
    if state.active == audio.map_view {
        return;
    }
    audio.map_view = state.active;
    let clip = if state.active {
        bank.panel_open.near(0)
    } else {
        bank.panel_close.near(0)
    };
    play(&mut commands, &mut budget, &mix, Cue::ui(clip, gain::PANEL));
}

/// Aggregate the balance over a window and play at most one sound.
#[allow(clippy::too_many_arguments)]
pub fn money_sound(
    money: Res<Money>,
    clock: Res<AudioClock>,
    mix: Res<AudioMix>,
    duck: Res<Duck>,
    bank: Option<Res<SfxBank>>,
    mut audio: ResMut<UiAudio>,
    mut budget: ResMut<VoiceBudget>,
    mut commands: Commands,
) {
    let Some(bank) = bank else {
        return;
    };
    let now = clock.elapsed;
    let cents = money.cents();
    if !audio.cents_seeded {
        audio.window_start_cents = cents;
        audio.window_opened_at = now;
        audio.cents_seeded = true;
        return;
    }
    if now - audio.window_opened_at < MONEY_WINDOW_SECS as f64 {
        return;
    }

    let net = cents - audio.window_start_cents;
    audio.window_start_cents = cents;
    audio.window_opened_at = now;

    if net.abs() < MONEY_FLOOR_CENTS {
        return;
    }

    if net > 0 {
        if !audio.first_income_done {
            // The loop closing for the first time (§6). Slightly more than it
            // strictly deserves, once, and never again.
            audio.first_income_done = true;
            play(
                &mut commands,
                &mut budget,
                &mix,
                Cue::ui(bank.milestone.near(0), gain::MILESTONE),
            );
            return;
        }
        play(
            &mut commands,
            &mut budget,
            &mix,
            Cue::ui(bank.money_gain.near(0), gain::MONEY_GAIN),
        );
    } else {
        // While the player is laying track the clacks have already reported the
        // spend; a second sound for the same action is clutter.
        if duck.build > 0.15 {
            return;
        }
        play(
            &mut commands,
            &mut budget,
            &mix,
            Cue::ui(bank.money_spend.near(0), gain::MONEY_SPEND),
        );
    }
}

/// A gentle two-note when something new needs attention. Never urgent.
#[allow(clippy::too_many_arguments)]
pub fn alert_sound(
    board: Res<AlertBoard>,
    clock: Res<AudioClock>,
    mix: Res<AudioMix>,
    bank: Option<Res<SfxBank>>,
    mut audio: ResMut<UiAudio>,
    mut budget: ResMut<VoiceBudget>,
    mut duck: ResMut<Duck>,
    mut commands: Commands,
) {
    let Some(bank) = bank else {
        return;
    };
    let current: Vec<u64> = board.iter().map(|a| a.id).collect();
    if current == audio.alerts {
        return;
    }
    let fresh = audio.alerts_seeded && current.iter().any(|id| !audio.alerts.contains(id));
    audio.alerts = current;
    audio.alerts_seeded = true;
    if !fresh {
        return;
    }

    let now = clock.elapsed;
    if now - audio.last_alert < ALERT_COOLDOWN_SECS as f64 {
        return;
    }
    audio.last_alert = now;
    // Alerts duck ambience slightly. Nothing ducks hard (§7).
    duck.on_alert();
    play(
        &mut commands,
        &mut budget,
        &mix,
        Cue::ui(bank.alert.near(0), gain::ALERT),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_busy_network_cannot_become_a_slot_machine() {
        // Acceptance bar 4. Whatever the income rate, the chime rate is capped
        // by the window — under twenty an hour at the absolute worst.
        let per_minute = 60.0 / MONEY_WINDOW_SECS;
        assert!(per_minute <= 20.0, "{per_minute} chimes a minute is a casino");
        assert!(MONEY_WINDOW_SECS >= 2.0, "and the window must be a real one");
    }

    #[test]
    fn tiny_movements_are_silent() {
        // Per-tick maintenance is a few cents; it must never chime.
        assert!(MONEY_FLOOR_CENTS >= 100, "a dollar is the floor at least");
    }

    #[test]
    fn interface_sound_stays_under_everything_else() {
        // "Felt more than heard" (§5).
        assert!(gain::UI_CLICK < gain::CLACK);
        assert!(gain::PANEL < gain::CLACK);
        assert!(gain::ALERT < gain::WHISTLE);
        assert!(gain::MONEY_SPEND < gain::MONEY_GAIN, "spending is the quieter one");
    }

    #[test]
    fn the_milestone_is_the_warmest_thing_in_the_interface() {
        assert!(gain::MILESTONE > gain::MONEY_GAIN);
        assert!(gain::MILESTONE > gain::ALERT);
    }
}
