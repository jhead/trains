//! Audio — the other half of calm (`docs/design/10-audio-and-feel.md`).
//!
//! A silent building game feels like a tech demo no matter how good it looks.
//! This module is the whole soundtrack: an ambience bed composed from what the
//! camera is looking at, the railway punctuating it, a sparse score that is
//! absent more often than present, and interface sound quiet enough to be felt
//! rather than heard.
//!
//! # There are no audio assets
//!
//! Every sound is synthesised. Two custom asset types carry it:
//!
//! - [`clip::SampleClip`] — finite PCM, baked once at startup by [`bank`]. All
//!   the one-shots: the track clack and its five siblings, the invalid thud, the
//!   station chord, whistles, chimes, ticks.
//! - [`voice::LiveVoice`] — an **endless generator** with lock-free controls.
//!   All the beds, the trains and the music. Nothing loops because there is no
//!   loop: the brief's "nothing loops audibly" is structural here rather than a
//!   thing somebody has to remember when authoring a file.
//!
//! Neither needs a format decoder, so this module adds no dependency and no
//! Cargo feature beyond the existing `sfx`.
//!
//! # The three rules
//!
//! 1. **Never startle.** Every gain in the game is slewed
//!    ([`dsp::approach`]), every clip is de-clicked at both ends
//!    ([`clip::Canvas::finish`]), the loudest single event is a whistle at
//!    `0.34` linear, and the whole dynamic range is about ten decibels wide.
//! 2. **Everything is positional.** [`mixer::AudioMix`] owns distance, the
//!    near/far clip choice, panning and zoom perspective.
//! 3. **Silence is a texture.** The score is quiet more than half the time and
//!    the bed thins out over empty country rather than filling it in.
//!
//! # Layout
//!
//! | Module | Brief |
//! | --- | --- |
//! | [`dsp`] | synthesis primitives |
//! | [`clip`] / [`bank`] | §3.1 one-shots, baked at boot |
//! | [`voice`] / [`voices`] | endless generators and their handles |
//! | [`mixer`] | §7 buses, distance, zoom, ducking, voice limits |
//! | [`build`] | §3.1 the sounds the player causes |
//! | [`ambience`] | §2 the bed |
//! | [`trains`] | §3.2 the sounds the network makes |
//! | [`ui_sound`] | §5 interface, and the aggregated money chime |
//! | [`music`] | §4 the score |
//!
//! # Wiring
//!
//! Add [`AudioPlugin`] after `SimPlugin`, `MapPlugin` and `AtmospherePlugin` —
//! it reads their resources and writes only its own. With the `sfx` feature off
//! the plugin compiles to nothing, so the call site needs no `cfg`.

use bevy::prelude::*;

#[cfg(feature = "sfx")]
mod ambience;
#[cfg(feature = "sfx")]
mod bank;
#[cfg(feature = "sfx")]
mod build;
#[cfg(feature = "sfx")]
mod clip;
#[cfg(feature = "sfx")]
mod dsp;
#[cfg(feature = "sfx")]
mod mixer;
#[cfg(feature = "sfx")]
mod music;
#[cfg(feature = "sfx")]
mod trains;
#[cfg(feature = "sfx")]
mod ui_sound;
#[cfg(feature = "sfx")]
mod voice;
#[cfg(feature = "sfx")]
mod voices;

/// Interface sounds any module may request by writing this message.
///
/// The audio module never reaches into another slice's state, so panels,
/// toggles and milestones ask for their sound rather than being watched for it.
/// Registered by [`AudioPlugin`]; sending one with the `sfx` feature off is a
/// no-op because the message type is still registered.
#[cfg(feature = "sfx")]
#[allow(unused_imports)] // Inbound API for the panel / toggle / milestone owners.
pub use ui_sound::UiCue;

/// Ordered phases inside `Update`.
///
/// Everything that decides *how loud* runs in [`AudioSet::Mix`]; everything that
/// plays runs in [`AudioSet::Play`] and sees a settled mixer. Without this split
/// a sound spawned early in a frame would use last frame's listener position,
/// which is audible as a lag when panning quickly.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg(feature = "sfx")]
pub enum AudioSet {
    Mix,
    Play,
}

/// The whole soundtrack.
pub struct AudioPlugin;

#[cfg(feature = "sfx")]
impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        use bevy::audio::AddAudioSource;

        app.add_audio_source::<clip::SampleClip>()
            .add_audio_source::<voice::LiveVoice>()
            .add_message::<ui_sound::UiCue>()
            // Shared with the visual ambience so both freeze together on pause.
            .init_resource::<crate::atmosphere::AmbientClock>()
            .init_resource::<crate::atmosphere::TimeOfDay>()
            .init_resource::<mixer::AudioClock>()
            .init_resource::<mixer::AudioMix>()
            .init_resource::<mixer::Duck>()
            .init_resource::<mixer::VoiceBudget>()
            .init_resource::<ambience::AmbienceBeds>()
            .init_resource::<build::BuildAudio>()
            .init_resource::<trains::TrainAudio>()
            .init_resource::<ui_sound::UiAudio>()
            .init_resource::<music::MusicDirector>()
            .add_systems(
                Startup,
                (
                    bank::bake_bank,
                    mixer::spawn_listener,
                    ambience::spawn_ambience,
                    music::spawn_music,
                ),
            )
            .configure_sets(Update, (AudioSet::Mix, AudioSet::Play).chain())
            .add_systems(
                Update,
                (
                    mixer::advance_audio_clock,
                    mixer::sync_listener,
                    mixer::refresh_mix,
                    mixer::refresh_budget,
                )
                    .chain()
                    .in_set(AudioSet::Mix),
            )
            .add_systems(
                Update,
                (
                    // §3.1 — collect, then drain on the run's own rhythm.
                    build::collect_track_edits,
                    build::drain_build_queue.after(build::collect_track_edits),
                    build::watch_stations,
                    build::watch_tool_switch,
                    // §2 — the bed.
                    ambience::drive_ambience,
                    // §3.2 — the network.
                    trains::drive_rolling_voices,
                    // §5 — interface.
                    ui_sound::play_ui_cues,
                    ui_sound::button_clicks,
                    ui_sound::map_view_sweep,
                    ui_sound::money_sound,
                    ui_sound::alert_sound,
                    // §4 — the score, after ambience so it can read the town.
                    music::drive_music.after(ambience::drive_ambience),
                )
                    .in_set(AudioSet::Play),
            );
    }
}

/// With `sfx` off there is no `bevy_audio` to build against, so the plugin is a
/// no-op and the call site stays identical.
#[cfg(not(feature = "sfx"))]
impl Plugin for AudioPlugin {
    fn build(&self, _app: &mut App) {}
}

#[cfg(all(test, feature = "sfx"))]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;
    use rail_map::generate_map;
    use rail_sim::{
        AlertBoard, IndustryRegistry, Money, StationRegistry, TileOccupancy, TownDensity,
        TrackEdit, TrackNetwork,
    };

    use crate::lines::LineToolState;
    use crate::track::TrackToolState;
    use crate::trains::TrainToolState;

    /// A headless app with everything the audio systems read.
    ///
    /// `bevy_audio`'s own plugin is included: with no audio device it logs a
    /// warning and every sink creation becomes a no-op, which is exactly the
    /// mute-safe path this module has to survive on a CI box.
    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::transform::TransformPlugin,
            bevy::audio::AudioPlugin::default(),
        ));
        app.insert_resource(generate_map(32, 32, 42));
        app.insert_resource(TownDensity::default());
        app.insert_resource(IndustryRegistry::default());
        app.insert_resource(StationRegistry::default());
        app.insert_resource(TrackNetwork::default());
        app.insert_resource(TileOccupancy::default());
        app.insert_resource(Money::sandbox_starting());
        app.insert_resource(AlertBoard::default());
        app.insert_resource(TrackToolState::default());
        app.insert_resource(TrainToolState::default());
        app.insert_resource(LineToolState::default());
        app.add_message::<TrackEdit>();
        app.add_plugins(AudioPlugin);
        app
    }

    #[test]
    fn the_plugin_builds_and_runs_without_an_audio_device() {
        // The real value of this test is that it exercises every system's
        // parameters: a conflicting borrow or a missing resource panics here
        // rather than on a player's machine.
        let mut app = test_app();
        for _ in 0..8 {
            app.update();
        }
    }

    #[test]
    fn the_bank_bakes_and_the_beds_spawn() {
        let mut app = test_app();
        app.update();
        app.update();
        assert!(
            app.world().get_resource::<bank::SfxBank>().is_some(),
            "the one-shot bank must exist after startup"
        );
        let beds = app.world().resource::<ambience::AmbienceBeds>();
        assert_eq!(
            beds.voices.len(),
            voice::VoiceKind::BEDS.len(),
            "every ambience layer gets a voice"
        );
    }

    #[test]
    fn a_long_run_of_track_does_not_produce_a_burst() {
        // Brief §3.1: rate-limited so a long run is a rhythmic run. Sixty tiles
        // committed in one frame must not become sixty voices in one frame.
        let mut app = test_app();
        app.update();
        app.update();

        app.world_mut()
            .resource_mut::<Messages<TrackEdit>>()
            .write_batch((0..60i32).map(|i| TrackEdit::Placed {
                id: rail_sim::TrackId(i as u64 + 1),
                tile: rail_sim::TileCoord { x: i % 30, y: 4 },
                layer: rail_sim::GROUND_LAYER,
                is_bridge: false,
            }));
        app.update();

        let playing = app
            .world_mut()
            .query::<&mixer::OneShot>()
            .iter(app.world())
            .count();
        assert!(
            playing <= 2,
            "{playing} clacks landed in one frame — that is a burst"
        );
    }

    #[test]
    fn a_rejected_placement_answers_immediately() {
        let mut app = test_app();
        app.update();
        app.update();
        let before = app
            .world_mut()
            .query::<&mixer::OneShot>()
            .iter(app.world())
            .count();

        app.world_mut()
            .resource_mut::<Messages<TrackEdit>>()
            .write(TrackEdit::Failed {
                error: rail_sim::PlacementError::OutOfBounds,
                tile: Some(rail_sim::TileCoord { x: 5, y: 5 }),
            });
        app.update();

        let after = app
            .world_mut()
            .query::<&mixer::OneShot>()
            .iter(app.world())
            .count();
        assert_eq!(after, before + 1, "the thud is immediate, not queued");
    }

    #[test]
    fn forty_rejections_in_a_row_produce_one_thud() {
        // Dragging across a mountainside must not buzz.
        let mut app = test_app();
        app.update();
        app.update();
        for _ in 0..40 {
            app.world_mut()
                .resource_mut::<Messages<TrackEdit>>()
                .write(TrackEdit::Failed {
                    error: rail_sim::PlacementError::OutOfBounds,
                    tile: Some(rail_sim::TileCoord { x: 5, y: 5 }),
                });
        }
        app.update();
        let playing = app
            .world_mut()
            .query::<&mixer::OneShot>()
            .iter(app.world())
            .count();
        assert!(playing <= 1, "{playing} thuds from one drag");
    }

    #[test]
    fn the_music_does_not_start_at_launch() {
        let mut app = test_app();
        for _ in 0..30 {
            app.update();
        }
        let director = app.world().resource::<music::MusicDirector>();
        assert_eq!(director.gain(), 0.0, "music was audible in the first seconds");
    }
}
