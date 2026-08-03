//! Rail Town — Bevy application entry.
//!
//! Boot: window, map (terrain + camera), sim plugin (command buffer / money /
//! clock / track / town / peeps), HUD, input bridge for pause / speed, and
//! track build tools. Domain systems join `FixedUpdate` via [`rail_sim::SimSet`].

mod atmosphere;
mod audio;
mod border;
mod hash;
mod input;
mod inspect;
mod lines;
mod map;
mod onboarding;
mod overlays;
mod palette;
mod shell;
mod sim_bridge;
mod stations;
mod town;
mod track;
mod trains;
mod ui;

use atmosphere::AtmospherePlugin;
use audio::AudioPlugin;
use bevy::prelude::*;
use border::BorderPresentationPlugin;
use input::InputMapPlugin;
use inspect::InspectPlugin;
use lines::LinesPlugin;
use map::MapPlugin;
use onboarding::OnboardingPlugin;
use overlays::OverlaysPlugin;
use palette::BG0;
use rail_net::{ManifestService, NeighborService};
use rail_sim::SimPlugin;
use shell::{ShellPlugin, ShellState};
use sim_bridge::SimBridgePlugin;
use stations::StationsPlugin;
use town::TownPresentationPlugin;
use track::TrackPlugin;
use trains::TrainsPlugin;
use ui::UiPlugin;

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Rail Town".into(),
                        resolution: (1280, 720).into(),
                        ..default()
                    }),
                    ..default()
                })
                // Pixel contract: nearest sampling is the default for every
                // texture, so a new atlas can't silently sample linearly.
                .set(ImagePlugin::default_nearest()),
        )
        // Title / New Map / Pause / Settings. Needs `StatesPlugin`, so it must
        // follow `DefaultPlugins`. `SavePlugin` is already registered inside
        // `SimPlugin` — don't add it a second time here.
        .add_plugins(ShellPlugin::default())
        // The live input map. After the shell, which is where the player's own
        // bindings are read off disk, and before anything that reads a key.
        .add_plugins(InputMapPlugin)
        // The real state gating (burn-down, "gameplay plugins aren't
        // state-gated"): every player verb sits in one set, and the set only
        // runs while the player is playing. The world itself keeps running
        // behind the title - it just isn't listening.
        .configure_sets(
            Update,
            input::PlayerVerbSet.run_if(in_state(ShellState::Playing)),
        )
        .add_plugins(SimPlugin)
        .add_plugins(SimBridgePlugin)
        // Seeded terrain + pan/zoom camera (default seed 42, 64×64).
        .add_plugins(MapPlugin::default())
        .add_plugins(TrackPlugin)
        .add_plugins(StationsPlugin)
        .add_plugins(TrainsPlugin)
        .add_plugins(LinesPlugin)
        .add_plugins(BorderPresentationPlugin)
        .add_plugins(TownPresentationPlugin)
        // Time-of-day tint, lit windows, and world-anchored ambient motion.
        .add_plugins(AtmospherePlugin)
        // Procedural ambience, railway sound, interface sound and the score.
        // Reads the sim, map and atmosphere resources, so it follows them.
        .add_plugins(AudioPlugin)
        .add_plugins(InspectPlugin)
        .add_plugins(OverlaysPlugin)
        // Opening nudge, one-shot hints, and the first-payout moment.
        .add_plugins(OnboardingPlugin)
        .add_plugins(UiPlugin)
        // Null neighbor backend: single-player never blocks on edge handoff.
        .insert_resource(NeighborService::null())
        // Offline blob store: MP-1 trades entirely with echo neighbours, and
        // an unset endpoint is the shipping default.
        .insert_resource(ManifestService::offline())
        .insert_resource(ClearColor(BG0))
        .run();
}
