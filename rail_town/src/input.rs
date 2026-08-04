//! The input map — named player verbs, and the key each one answers to.
//!
//! Design [`03 §10.2`](../../docs/design/03-ui-system.md) is the shortcut table
//! and [`09 §5`](../../docs/design/09-shell-and-menus.md) requires the list to be
//! rebindable. This module is the *live* half of that: one resource holding the
//! key every action currently answers to, and the lookup gameplay systems use in
//! place of a [`KeyCode`] literal.
//!
//! The *table* — what a verb is called, which group it belongs to, what it
//! defaults to, and how it persists — stays in [`crate::shell::controls`],
//! because the Settings tab is the authority on all four. Nothing is duplicated:
//! [`KeyBindings::adopt`] copies the stored map across whenever it changes, and
//! [`ControlAction`] and [`Binding`] are re-exported here so a gameplay system
//! needs exactly one import.
//!
//! # Using it
//!
//! ```ignore
//! fn my_tool(keys: Res<ButtonInput<KeyCode>>, bindings: Res<KeyBindings>) {
//!     if bindings.just_pressed(&keys, ControlAction::TrackTool) { … }
//! }
//! ```
//!
//! [`KeyBindings`] defaults to the shipping table, so a plugin that
//! `init_resource`s it works headless and in tests with no shell present.
//! [`InputMapPlugin`] is what makes a *rebind* reach the game: it copies
//! `Settings::controls` into the resource in `PreStartup` (so the first frame the
//! menu row draws already carries the player's own keys) and again in `PreUpdate`
//! whenever the settings change.
//!
//! # Modifiers
//!
//! A modifier is part of a binding's identity — that is what lets `Ctrl+Z` and
//! `Z` coexist, and [`ControlSettings::conflicts`](crate::shell::controls::ControlSettings::conflicts)
//! already says so. [`KeyBindings::just_pressed`] therefore requires an *exact*
//! modifier match: `Ctrl+Z` no longer also resets the zoom on the way past.
//! [`KeyBindings::pressed`] does not, because it exists for held movement keys
//! and a player who happens to be holding Ctrl still expects panning to work.

use bevy::input::InputSystems;
use bevy::prelude::*;
use rail_sim::{ComplaintEntry, ComplaintFeed, StationService, TalkKind};

pub use crate::shell::controls::{Binding, ControlAction};
use crate::shell::controls::ControlSettings;
use crate::shell::Settings;

/// The key every action answers to right now.
///
/// Defaults to the shipping table, so this resource is always usable even with
/// no shell, no settings file and no player profile.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct KeyBindings {
    bindings: Vec<(ControlAction, Binding)>,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            bindings: ControlAction::ALL
                .iter()
                .map(|a| (*a, a.default_binding()))
                .collect(),
        }
    }
}

impl KeyBindings {
    /// The binding for `action`, falling back to its default.
    pub fn binding(&self, action: ControlAction) -> Binding {
        self.bindings
            .iter()
            .find(|(a, _)| *a == action)
            .map(|(_, b)| *b)
            .unwrap_or_else(|| action.default_binding())
    }

    /// Just the key, for display and for posting a synthetic press.
    pub fn key(&self, action: ControlAction) -> KeyCode {
        self.binding(action).key
    }

    /// Short display name — `B`, `3`, `Ctrl+Z`. What the menu row draws.
    pub fn label(&self, action: ControlAction) -> String {
        self.binding(action).label()
    }

    /// `true` on the frame `action`'s key went down, with its modifier state.
    pub fn just_pressed(&self, keys: &ButtonInput<KeyCode>, action: ControlAction) -> bool {
        let binding = self.binding(action);
        keys.just_pressed(binding.key) && ctrl_held(keys) == binding.ctrl
    }

    /// `true` while `action`'s key is held. Modifier-agnostic — see the module
    /// docs.
    pub fn pressed(&self, keys: &ButtonInput<KeyCode>, action: ControlAction) -> bool {
        keys.pressed(self.binding(action).key)
    }

    /// `true` when any of `actions` was just pressed. Tools use this to notice
    /// that another verb has reclaimed the pointer.
    pub fn any_just_pressed(
        &self,
        keys: &ButtonInput<KeyCode>,
        actions: &[ControlAction],
    ) -> bool {
        actions.iter().any(|a| self.just_pressed(keys, *a))
    }

    /// Take the player's stored map. The Settings tab is the only writer.
    pub fn adopt(&mut self, controls: &ControlSettings) {
        for (action, binding) in self.bindings.iter_mut() {
            *binding = controls.key_for(*action);
        }
    }

    /// Rebind one action. Tests and the odd programmatic override; the player's
    /// route is the Controls tab, which goes through [`Self::adopt`].
    #[allow(dead_code)] // Public API of the map; the shipping path is `adopt`.
    pub fn set(&mut self, action: ControlAction, binding: Binding) {
        match self.bindings.iter_mut().find(|(a, _)| *a == action) {
            Some(slot) => slot.1 = binding,
            None => self.bindings.push((action, binding)),
        }
    }
}

/// Ctrl or Cmd. The two are one modifier everywhere in this game.
fn ctrl_held(keys: &ButtonInput<KeyCode>) -> bool {
    keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight)
        || keys.pressed(KeyCode::SuperLeft)
        || keys.pressed(KeyCode::SuperRight)
}

/// Every system that turns a press into a gameplay verb — tools, panels,
/// camera control, speed, undo.
///
/// `main.rs` gates this whole set on `ShellState::Playing`, which is the real
/// state gating the burn-down asked for: the shell's input suppression stays
/// as a safety net, but a menu being up now means the verbs never run at all.
/// Sim and presentation stay out of the set on purpose — the title screen's
/// world is alive by design (09 §2), it just isn't listening.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerVerbSet;

/// Installs [`KeyBindings`] and keeps it in step with the Controls tab.
///
/// Gameplay plugins `init_resource::<KeyBindings>()` for themselves so they work
/// standalone; this plugin adds the half that needs the shell.
pub struct InputMapPlugin;

impl Plugin for InputMapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KeyBindings>()
            // `PreStartup` because the menu row draws its key labels in
            // `Startup`, which is earlier than any `PreUpdate` will ever run.
            .add_systems(PreStartup, sync_bindings_from_settings)
            .add_systems(
                PreUpdate,
                sync_bindings_from_settings
                    .after(InputSystems)
                    .run_if(resource_exists_and_changed::<Settings>),
            );
    }
}

/// Copy `Settings::controls` into the live map, and say so if two verbs clash.
///
/// `Settings` is optional so the plugin still builds headless, where the shell
/// does not exist and the defaults are the whole story.
///
/// # Why the warning is here
///
/// A playtester reported *"B is a hotkey for both map and build?"*. The shipping
/// defaults are conflict-free and a test holds them that way, so the only way to
/// get there is a **stored profile** — one written before a default moved, or one
/// the player rebound themselves. The Controls tab already flags a clash, but
/// only for somebody who goes looking; the player who hits it is by definition
/// not in the menu, they are on the map wondering why one key does two things.
///
/// So the moment the game adopts a table it checks it, and one Town Talk line
/// names both verbs, the key, and where to fix it. Once per adoption: the
/// adoption itself only happens when the stored map differs from the live one,
/// so a settings change that leaves the bindings alone says nothing.
///
/// One limit worth knowing: starting a **new map** clears the feed with the
/// rest of the world, so a boot-time notice does not survive into a world made
/// after it. The line comes back the next time the table is adopted — which is
/// the next rebind — and the Controls tab flags the clash in the meantime.
pub fn sync_bindings_from_settings(
    settings: Option<Res<Settings>>,
    service: Option<Res<StationService>>,
    mut talk: Option<ResMut<ComplaintFeed>>,
    mut bindings: ResMut<KeyBindings>,
) {
    let Some(settings) = settings else {
        return;
    };
    let mut next = bindings.clone();
    next.adopt(&settings.controls);
    if next == *bindings {
        return;
    }
    *bindings = next;

    let Some(talk) = talk.as_mut() else {
        return;
    };
    let Some((first, second, binding)) = first_conflict(&settings.controls) else {
        return;
    };
    talk.push(ComplaintEntry {
        kind: TalkKind::Warning,
        peep_name: format!(
            "{} is bound to both {} and {} - fix in Settings > Controls",
            binding.label(),
            first.label(),
            second.label()
        ),
        station_name: String::new(),
        wait_minutes: 0,
        sim_tick: service.map(|s| s.tick).unwrap_or(0),
        peep_id: None,
        station_id: None,
        tile: None,
        count: 1,
    });
}

/// The first two verbs sharing a key, in [`ControlAction::ALL`] order.
///
/// Detection is [`ControlSettings::conflicts`] — the Controls tab's own
/// machinery, so the map and the menu can never disagree about what a conflict
/// is. This only picks which pair to name, and it names one pair rather than
/// every pair because a line the player cannot read is not a warning.
fn first_conflict(controls: &ControlSettings) -> Option<(ControlAction, ControlAction, Binding)> {
    let clashing = controls.conflicts();
    if clashing.is_empty() {
        return None;
    }
    let ordered: Vec<ControlAction> = ControlAction::ALL
        .iter()
        .copied()
        .filter(|a| clashing.contains(a))
        .collect();
    let first = *ordered.first()?;
    let binding = controls.key_for(first);
    let second = ordered
        .iter()
        .copied()
        .find(|a| *a != first && controls.key_for(*a) == binding)?;
    Some((first, second, binding))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(down: &[KeyCode]) -> ButtonInput<KeyCode> {
        let mut input = ButtonInput::<KeyCode>::default();
        for key in down {
            input.press(*key);
        }
        input
    }

    #[test]
    fn the_map_starts_from_the_shipping_defaults() {
        let bindings = KeyBindings::default();
        for action in ControlAction::ALL {
            assert_eq!(
                bindings.binding(*action),
                action.default_binding(),
                "{action:?} did not start from its default"
            );
        }
    }

    #[test]
    fn a_lookup_replaces_the_literal_it_was_written_as() {
        let bindings = KeyBindings::default();
        let down = keys(&[KeyCode::KeyB]);
        assert!(bindings.just_pressed(&down, ControlAction::TrackTool));
        assert!(!bindings.just_pressed(&down, ControlAction::DemolishTool));
    }

    #[test]
    fn rebinding_moves_what_the_game_listens_for() {
        // The whole point of the slice: the stored key is the key the game
        // reads, not a note in a settings file nothing consumes.
        let mut bindings = KeyBindings::default();
        bindings.set(ControlAction::TrackTool, Binding::key(KeyCode::KeyJ));
        assert!(bindings.just_pressed(&keys(&[KeyCode::KeyJ]), ControlAction::TrackTool));
        assert!(!bindings.just_pressed(&keys(&[KeyCode::KeyB]), ControlAction::TrackTool));
    }

    #[test]
    fn a_modifier_is_part_of_the_press_as_well_as_of_the_binding() {
        // Undo is Ctrl+Z and reset-zoom is Z. Reading the literal, Ctrl+Z did
        // both: it undid the last build *and* threw the camera back to the
        // default zoom on the way past.
        let bindings = KeyBindings::default();
        let plain = keys(&[KeyCode::KeyZ]);
        let with_ctrl = keys(&[KeyCode::ControlLeft, KeyCode::KeyZ]);

        assert!(bindings.just_pressed(&plain, ControlAction::ResetZoom));
        assert!(!bindings.just_pressed(&plain, ControlAction::Undo));
        assert!(bindings.just_pressed(&with_ctrl, ControlAction::Undo));
        assert!(!bindings.just_pressed(&with_ctrl, ControlAction::ResetZoom));
    }

    #[test]
    fn cmd_counts_as_ctrl() {
        let bindings = KeyBindings::default();
        let with_cmd = keys(&[KeyCode::SuperLeft, KeyCode::KeyZ]);
        assert!(bindings.just_pressed(&with_cmd, ControlAction::Undo));
    }

    #[test]
    fn held_movement_keys_do_not_care_about_modifiers() {
        // Panning is a held key, not a chord; a stray Ctrl must not stop the
        // camera dead.
        let bindings = KeyBindings::default();
        let down = keys(&[KeyCode::ControlLeft, KeyCode::KeyW]);
        assert!(bindings.pressed(&down, ControlAction::PanUp));
        assert!(!bindings.pressed(&down, ControlAction::PanDown));
    }

    #[test]
    fn any_just_pressed_spots_a_verb_reclaiming_the_pointer() {
        let bindings = KeyBindings::default();
        let watched = [
            ControlAction::TrackTool,
            ControlAction::DemolishTool,
            ControlAction::BuyTransit,
        ];
        assert!(bindings.any_just_pressed(&keys(&[KeyCode::KeyT]), &watched));
        assert!(!bindings.any_just_pressed(&keys(&[KeyCode::KeyM]), &watched));
    }

    #[test]
    fn the_live_map_adopts_the_stored_one() {
        let mut controls = ControlSettings::default();
        controls.set(ControlAction::MapView, Binding::key(KeyCode::KeyJ));

        let mut bindings = KeyBindings::default();
        bindings.adopt(&controls);
        assert_eq!(bindings.key(ControlAction::MapView), KeyCode::KeyJ);
        // Everything else is untouched.
        assert_eq!(bindings.key(ControlAction::TrackTool), KeyCode::KeyB);
    }

    #[test]
    fn the_plugin_carries_a_stored_rebind_into_the_game() {
        // End to end, in a real schedule: the Controls tab writes `Settings`,
        // the plugin copies it across, and every gameplay system that asks for
        // an action now gets the player's key. Before this slice the copy did
        // not exist and the stored value went nowhere.
        let mut settings = Settings::default();
        settings
            .controls
            .set(ControlAction::MapView, Binding::key(KeyCode::KeyJ));

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::input::InputPlugin))
            .insert_resource(settings)
            .add_plugins(InputMapPlugin);
        app.update();
        assert_eq!(
            app.world().resource::<KeyBindings>().key(ControlAction::MapView),
            KeyCode::KeyJ,
            "the stored binding did not reach the live map"
        );

        // And a change made while playing lands on the next frame.
        app.world_mut()
            .resource_mut::<Settings>()
            .controls
            .set(ControlAction::MapView, Binding::key(KeyCode::KeyQ));
        app.update();
        assert_eq!(
            app.world().resource::<KeyBindings>().key(ControlAction::MapView),
            KeyCode::KeyQ
        );
    }

    /// Number of Town Talk lines about a clash.
    fn warnings(app: &App) -> Vec<String> {
        app.world()
            .resource::<ComplaintFeed>()
            .iter()
            .filter(|e| e.kind == TalkKind::Warning)
            .map(|e| e.display_line())
            .collect()
    }

    fn app_with(settings: Settings) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::input::InputPlugin))
            .init_resource::<ComplaintFeed>()
            .insert_resource(settings)
            .add_plugins(InputMapPlugin);
        app
    }

    /// **The playtest report.** "B is a hotkey for both map and build?" — not
    /// reachable from the defaults, but a stored profile can carry it, and the
    /// player who hits it is on the map, not in the menu. The game says so.
    #[test]
    fn adopting_a_clashing_profile_says_which_two_verbs_want_the_key() {
        let mut settings = Settings::default();
        settings
            .controls
            .set(ControlAction::MapView, Binding::key(KeyCode::KeyB));

        let mut app = app_with(settings);
        app.update();

        let lines = warnings(&app);
        assert_eq!(lines.len(), 1, "one line per adoption: {lines:?}");
        assert_eq!(
            lines[0],
            "B is bound to both Track tool and Map View - fix in Settings > Controls"
        );
        assert!(lines[0].is_ascii(), "the shipped font draws tofu otherwise");

        // A further frame is not a further adoption — the table has not moved.
        app.update();
        app.update();
        assert_eq!(warnings(&app).len(), 1, "the warning does not repeat");
    }

    #[test]
    fn a_conflict_free_profile_says_nothing_at_all() {
        let mut settings = Settings::default();
        settings
            .controls
            .set(ControlAction::MapView, Binding::key(KeyCode::KeyJ));

        let mut app = app_with(settings);
        app.update();

        assert!(warnings(&app).is_empty());
        assert_eq!(
            app.world().resource::<KeyBindings>().key(ControlAction::MapView),
            KeyCode::KeyJ,
            "and the rebind still landed"
        );
    }

    #[test]
    fn a_modifier_still_keeps_two_verbs_apart() {
        // Ctrl+Z and Z are different bindings, and the warning must not invent
        // a clash the Controls tab does not report.
        let mut app = app_with(Settings::default());
        app.update();
        assert!(first_conflict(&ControlSettings::default()).is_none());
        assert!(warnings(&app).is_empty());
    }

    #[test]
    fn the_named_pair_is_stable_whichever_verb_was_moved() {
        // Either rebind produces the same sentence, because the pair is read in
        // the table's own order rather than in the order they were edited.
        let mut moved_map = ControlSettings::default();
        moved_map.set(ControlAction::MapView, Binding::key(KeyCode::KeyB));
        let mut moved_track = ControlSettings::default();
        moved_track.set(ControlAction::TrackTool, Binding::key(KeyCode::KeyM));

        assert_eq!(
            first_conflict(&moved_map),
            Some((
                ControlAction::TrackTool,
                ControlAction::MapView,
                Binding::key(KeyCode::KeyB)
            ))
        );
        assert_eq!(
            first_conflict(&moved_track),
            Some((
                ControlAction::TrackTool,
                ControlAction::MapView,
                Binding::key(KeyCode::KeyM)
            ))
        );
    }

    #[test]
    fn with_no_shell_the_defaults_are_the_whole_story() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::input::InputPlugin))
            .add_plugins(InputMapPlugin);
        app.update();
        assert_eq!(*app.world().resource::<KeyBindings>(), KeyBindings::default());
    }

    #[test]
    fn labels_read_the_way_the_menu_row_draws_them() {
        let bindings = KeyBindings::default();
        assert_eq!(bindings.label(ControlAction::TrackTool), "B");
        assert_eq!(bindings.label(ControlAction::Speed3), "3");
        assert_eq!(bindings.label(ControlAction::Undo), "Ctrl+Z");
        // 03 §3: the shipped font has no glyphs beyond ASCII.
        for action in ControlAction::ALL {
            let label = bindings.label(*action);
            assert!(label.is_ascii(), "{action:?} draws {label:?}");
        }
    }
}
