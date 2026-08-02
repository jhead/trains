//! Rail Town — Bevy application entry.
//!
//! Slice 0: window, clear color, camera, fixed-timestep schedule shell,
//! and null neighbor resource. Sim systems join `FixedUpdate` in later slices.

use bevy::prelude::*;
use rail_net::NeighborService;
use rail_sim::SimPlugin;

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
        // Sim plugin shell — systems will register into FixedUpdate.
        .add_plugins(SimPlugin)
        // Null neighbor backend: single-player never blocks on edge handoff.
        // Swap the boxed backend later for a real `NeighborBackend` impl.
        .insert_resource(NeighborService::null())
        .insert_resource(ClearColor(Color::srgb(0.12, 0.14, 0.18)))
        .add_systems(Startup, setup_camera)
        // Fixed-tick shell: economy, movement, growth, and command application
        // belong here (not in Update). Render interpolates from sim state.
        .add_systems(FixedUpdate, fixed_tick_shell)
        .run();
}

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

/// Placeholder fixed-tick system. Replace / expand with real sim systems.
fn fixed_tick_shell() {
    // Intentionally empty in Slice 0.
    // Later: drain command buffer → apply PlaceTrack / Demolish / … → advance sim.
}
