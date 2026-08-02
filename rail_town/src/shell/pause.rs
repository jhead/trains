//! Pause menu — `Esc` dims the world to 50% and centres a small panel.
//!
//! Design [`09 §4`](../../../docs/design/09-shell-and-menus.md): it does **not**
//! hide the world. The player should still see what they were doing, which is why
//! the scrim is a dim rather than an opaque panel and why the in-game HUD stays
//! on screen underneath it.

use bevy::prelude::*;
use rail_map::MapGrid;

use crate::ui::kit::{SPACE_2, SPACE_3};

use super::save::{self, SaveStatus};
use super::widgets::{
    dim_scrim, screen_root, shell_panel, spawn_note, spawn_panel_title, spawn_row, MenuAction,
    MenuCursor, MenuItem, MenuList, LAYER_SCREEN,
};
use super::ShellState;

/// Panel width. Small — this is a five-item menu, not a screen.
const PANEL_W: f32 = 200.0;

/// Marker so the panel is spawned exactly once per pause.
#[derive(Component)]
pub struct PauseRoot;

pub fn spawn_pause_if_missing(
    mut commands: Commands,
    map: Res<MapGrid>,
    time: Res<Time<Virtual>>,
    status: Res<SaveStatus>,
    cursor: Res<MenuCursor>,
    existing: Query<(), With<PauseRoot>>,
) {
    if !existing.is_empty() {
        return;
    }
    let saves = save::slots();
    let footer = format!(
        "Rail Town - seed {} - {}",
        map.seed,
        played_label(time.elapsed_secs())
    );

    commands
        .spawn((
            PauseRoot,
            screen_root(
                "shell::pause",
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    row_gap: Val::Px(SPACE_3),
                    ..default()
                },
            ),
            DespawnOnExit(ShellState::Paused),
        ))
        .with_children(|root| {
            root.spawn(dim_scrim());

            root.spawn((
                MenuList {
                    selected: cursor.0,
                    layer: LAYER_SCREEN,
                },
                shell_panel(Node {
                    width: Val::Px(PANEL_W),
                    row_gap: Val::Px(SPACE_2),
                    ..default()
                }),
            ))
            .with_children(|panel| {
                spawn_panel_title(panel, "Paused");
                spawn_row(panel, MenuItem::new(0, MenuAction::Resume), "Resume", "");

                spawn_row(panel, MenuItem::new(1, MenuAction::Save), "Save", "");

                let load_row = MenuItem::new(2, MenuAction::Load);
                let (load_row, load_value) = if saves.is_empty() {
                    (load_row.disabled(), "none yet")
                } else {
                    (load_row, "")
                };
                spawn_row(panel, load_row, "Load", load_value);

                spawn_row(
                    panel,
                    MenuItem::new(3, MenuAction::OpenSettings),
                    "Settings",
                    "",
                );
                spawn_row(
                    panel,
                    MenuItem::new(4, MenuAction::QuitToTitle),
                    "Quit to Title",
                    "",
                );

                if let Some(message) = &status.message {
                    spawn_note(panel, message);
                }
                spawn_note(panel, &footer);
                spawn_note(panel, "Esc resumes");
            });
        });
}

/// Elapsed play time, rounded the way a player would say it.
fn played_label(seconds: f32) -> String {
    let total = seconds.max(0.0) as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m played")
    } else {
        format!("{minutes}m played")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_time_reads_naturally() {
        assert_eq!(played_label(0.0), "0m played");
        assert_eq!(played_label(59.0), "0m played");
        assert_eq!(played_label(90.0), "1m played");
        assert_eq!(played_label(3_660.0), "1h 01m played");
    }

    #[test]
    fn negative_time_does_not_underflow() {
        assert_eq!(played_label(-10.0), "0m played");
    }
}
