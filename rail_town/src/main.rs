//! Rail Town — Bevy application entry.
//!
//! Boot: window, map (terrain + camera), sim plugin (command buffer / money /
//! clock / track / town / peeps), HUD, input bridge for pause / speed, and
//! track build tools. Domain systems join `FixedUpdate` via [`rail_sim::SimSet`].

mod map;
mod sim_bridge;
mod stations;
mod town;
mod track;
mod trains;
mod ui;

use bevy::prelude::*;
use map::MapPlugin;
use rail_net::NeighborService;
use rail_sim::SimPlugin;
use sim_bridge::SimBridgePlugin;
use stations::StationsPlugin;
use town::TownPresentationPlugin;
use track::TrackPlugin;
use trains::TrainsPlugin;
use ui::UiPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Rail Town".into(),
                resolution: (1280, 720).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(SimPlugin)
        .add_plugins(SimBridgePlugin)
        // Seeded terrain + pan/zoom camera (default seed 42, 64×64).
        .add_plugins(MapPlugin::default())
        .add_plugins(TrackPlugin)
        .add_plugins(StationsPlugin)
        .add_plugins(TrainsPlugin)
        .add_plugins(TownPresentationPlugin)
        .add_plugins(UiPlugin)
        // Null neighbor backend: single-player never blocks on edge handoff.
        .insert_resource(NeighborService::null())
        .insert_resource(ClearColor(Color::srgb(0.12, 0.14, 0.18)))
        .run();
}
