//! Title screen — a five-item menu over the live, drifting world.
//!
//! Design [`09 §2`](../../../docs/design/09-shell-and-menus.md): the world *is*
//! the background. Nothing here paints over it except the menu block itself, and
//! the only motion in the frame belongs to the world.

use bevy::prelude::*;
use rail_map::{map_center_world, MapGrid, TILE_SIZE};

use crate::map::MapCamera;
use crate::palette::BG1;
use crate::ui::kit::{display_font, text_primary, SPACE_2, SPACE_6, SPACE_8};

use super::save;
use super::settings::Settings;
use super::widgets::{
    screen_root, shell_panel, spawn_corner_stamp, spawn_note, spawn_row, MenuAction, MenuItem,
    MenuList, LAYER_SCREEN,
};
use super::ShellState;

/// Menu block width. Narrow on purpose — the world should dominate the frame.
const MENU_W: f32 = 240.0;

/// Drift amplitude as a share of the map's half-extent.
const DRIFT_AMPLITUDE: f32 = 0.16;
/// Seconds for one full horizontal sweep. Slow enough to read as calm.
const DRIFT_PERIOD_X: f32 = 96.0;
/// Deliberately not a multiple of the horizontal period, so the path never
/// retraces the same loop and the background stays quietly unpredictable.
const DRIFT_PERIOD_Y: f32 = 151.0;

/// Accumulated drift time. Kept out of `Time` so pausing the sim never freezes
/// the title screen (design §2: it is the game playing itself, quietly).
#[derive(Resource, Debug, Default)]
pub struct DriftClock {
    seconds: f32,
}

/// Marker so the screen is spawned exactly once per visit.
#[derive(Component)]
pub struct TitleRoot;

/// Put the title screen up while [`ShellState::Title`] is current.
///
/// Spawned from `Update` rather than `OnEnter` on purpose: `StateTransition`
/// runs *before* `PreStartup` at boot, so an `OnEnter` spawn would look for the
/// world before the shell has installed it. Checking for the root instead is
/// order-independent and matches how the other shell screens rebuild.
pub fn spawn_title_if_missing(
    mut commands: Commands,
    map: Res<MapGrid>,
    existing: Query<(), With<TitleRoot>>,
) {
    if !existing.is_empty() {
        return;
    }
    let saves = save::slots();
    let continue_value = match saves.first() {
        Some(info) => info.title(),
        // No save yet: Continue plays the world already on screen. Design §2 —
        // "a player who likes what they see can just start there".
        None => format!("seed {}", map.seed),
    };

    commands
        .spawn((
            TitleRoot,
            screen_root(
                "shell::title",
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    row_gap: Val::Px(SPACE_8),
                    ..default()
                },
            ),
            DespawnOnExit(ShellState::Title),
        ))
        .with_children(|root| {
            // Wordmark. Letter-spacing is done with real spaces: the kit forbids
            // letter-spacing tricks, and a bitmap font would not honour them.
            root.spawn((
                Text::new("R A I L   T O W N"),
                display_font(),
                text_primary(),
                BackgroundColor(BG1),
                Node {
                    padding: UiRect::axes(Val::Px(SPACE_6), Val::Px(SPACE_2)),
                    ..default()
                },
            ));

            root.spawn((
                MenuList {
                    selected: 0,
                    layer: LAYER_SCREEN,
                },
                shell_panel(Node {
                    width: Val::Px(MENU_W),
                    row_gap: Val::Px(SPACE_2),
                    ..default()
                }),
            ))
            .with_children(|menu| {
                spawn_row(
                    menu,
                    MenuItem::new(0, MenuAction::Continue),
                    "Continue",
                    &continue_value,
                );
                spawn_row(menu, MenuItem::new(1, MenuAction::NewMap), "New Map", "");

                let load = MenuItem::new(2, MenuAction::Load);
                let (load, load_value) = if saves.is_empty() {
                    (load.disabled(), "none yet")
                } else {
                    (load, "")
                };
                spawn_row(menu, load, "Load", load_value);

                spawn_row(
                    menu,
                    MenuItem::new(3, MenuAction::OpenSettings),
                    "Settings",
                    "",
                );
                spawn_row(menu, MenuItem::new(4, MenuAction::Quit), "Quit", "");
                spawn_note(menu, "↑ ↓ select   ↵ confirm");
            });

            spawn_corner_stamp(
                root,
                &format!("v{} · seed {}", env!("CARGO_PKG_VERSION"), map.seed),
            );
        });
}

/// The world drifts slowly behind the menu.
///
/// Runs in `PostUpdate` so it lands after the play camera's own pan for the
/// frame — the title owns the camera while it is up, without either system
/// needing to know about the other. Positions snap to whole texels; sub-pixel
/// camera placement is the fastest way to make a pixel game look soft.
pub fn drift_background_world(
    time: Res<Time>,
    settings: Res<Settings>,
    map: Res<MapGrid>,
    mut clock: ResMut<DriftClock>,
    mut cameras: Query<&mut Transform, With<MapCamera>>,
) {
    if settings.display.reduced_motion {
        return;
    }
    let Ok(mut transform) = cameras.single_mut() else {
        return;
    };
    clock.seconds += time.delta_secs();

    let (cx, cy) = map_center_world(map.width, map.height);
    let (dx, dy) = drift_offset(
        clock.seconds,
        map.width as f32 * TILE_SIZE,
        map.height as f32 * TILE_SIZE,
    );
    transform.translation.x = (cx + dx).round();
    transform.translation.y = (cy + dy).round();
}

/// Offset from the map centre at `seconds`, in world units.
fn drift_offset(seconds: f32, world_w: f32, world_h: f32) -> (f32, f32) {
    let tau = std::f32::consts::TAU;
    let ax = world_w * 0.5 * DRIFT_AMPLITUDE;
    let ay = world_h * 0.5 * DRIFT_AMPLITUDE;
    (
        (seconds * tau / DRIFT_PERIOD_X).sin() * ax,
        (seconds * tau / DRIFT_PERIOD_Y).sin() * ay,
    )
}

/// Park the camera back on the map centre when play starts, so a new game is
/// always framed the same way regardless of where the drift had wandered to.
pub fn centre_camera_on_map(map: &MapGrid, transform: &mut Transform) {
    let (cx, cy) = map_center_world(map.width, map.height);
    transform.translation.x = cx.round();
    transform.translation.y = cy.round();
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 64.0 * TILE_SIZE;

    #[test]
    fn drift_starts_at_the_map_centre() {
        let (dx, dy) = drift_offset(0.0, W, W);
        assert!(dx.abs() < f32::EPSILON);
        assert!(dy.abs() < f32::EPSILON);
    }

    #[test]
    fn drift_never_leaves_the_map() {
        let limit = W * 0.5 * DRIFT_AMPLITUDE + 1.0;
        for step in 0..2000 {
            let (dx, dy) = drift_offset(step as f32 * 0.5, W, W);
            assert!(dx.abs() <= limit, "drifted off the map horizontally");
            assert!(dy.abs() <= limit, "drifted off the map vertically");
        }
    }

    #[test]
    fn drift_is_slow_enough_to_read_as_calm() {
        // Under two world texels per frame at 60 fps, everywhere on the path.
        let mut fastest: f32 = 0.0;
        let mut previous = drift_offset(0.0, W, W);
        for step in 1..6000 {
            let now = drift_offset(step as f32 / 60.0, W, W);
            let speed = ((now.0 - previous.0).powi(2) + (now.1 - previous.1).powi(2)).sqrt();
            fastest = fastest.max(speed);
            previous = now;
        }
        assert!(fastest < 2.0, "drift moves {fastest} texels per frame");
    }

    #[test]
    fn the_drift_path_does_not_retrace_itself_quickly() {
        // Different periods on each axis: the position after one horizontal
        // sweep must not repeat the start.
        let start = drift_offset(0.0, W, W);
        let after = drift_offset(DRIFT_PERIOD_X, W, W);
        assert!((after.1 - start.1).abs() > 1.0);
    }
}
