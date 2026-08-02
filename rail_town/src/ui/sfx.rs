//! Phase A track SFX — short procedural pitches on place / fail.
//!
//! Enabled with the `sfx` feature (default). On WASM builds without `sfx`,
//! this module is not compiled. Playback failures are silent (mute-safe).

use bevy::prelude::*;
use rail_sim::TrackEdit;
use std::time::Duration;

/// Handles for clack (place) and thud (fail) pitches.
#[derive(Resource, Clone)]
pub struct TrackSfx {
    pub clack: Handle<bevy::audio::Pitch>,
    pub thud: Handle<bevy::audio::Pitch>,
    /// Rate-limit place sounds so autofill runs don't machine-gun.
    pub last_clack: f64,
}

pub fn setup_track_sfx(mut commands: Commands, mut pitches: ResMut<Assets<bevy::audio::Pitch>>) {
    let clack = pitches.add(bevy::audio::Pitch::new(880.0, Duration::from_millis(35)));
    let thud = pitches.add(bevy::audio::Pitch::new(110.0, Duration::from_millis(80)));
    commands.insert_resource(TrackSfx {
        clack,
        thud,
        last_clack: 0.0,
    });
}

pub fn play_track_sfx(
    mut edits: MessageReader<TrackEdit>,
    mut sfx: ResMut<TrackSfx>,
    time: Res<Time>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs_f64();
    for edit in edits.read() {
        match edit {
            TrackEdit::Placed { .. } => {
                // ~8 clacks/sec max (design: rate-limited rhythmic run).
                if now - sfx.last_clack < 0.12 {
                    continue;
                }
                sfx.last_clack = now;
                commands.spawn((
                    AudioPlayer(sfx.clack.clone()),
                    PlaybackSettings::DESPAWN.with_volume(bevy::audio::Volume::Linear(0.35)),
                ));
            }
            TrackEdit::Failed { .. } => {
                commands.spawn((
                    AudioPlayer(sfx.thud.clone()),
                    PlaybackSettings::DESPAWN.with_volume(bevy::audio::Volume::Linear(0.4)),
                ));
            }
            TrackEdit::Removed { .. } => {}
        }
    }
}
