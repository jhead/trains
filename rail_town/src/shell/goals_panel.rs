//! Goals panel — what this map is asking for, and how far along it is.
//!
//! Only ever on screen in a goals world; a sandbox world never sees it, and the
//! systems here return on their first line. Built entirely from
//! [`crate::ui::kit`] and [`crate::palette`], so it is made of the same material
//! as every other panel (design 09 §1).
//!
//! Each row is a title, a design 03 §8.4 meter, and a plain readout —
//! `12 / 40 runs · 2d left`. The meter's fill colour is derived from progress
//! (`warn` low, `hi` middle, `ok` high) but it never carries the state alone:
//! the row always spells out `open` / `met` / `missed` beside it (design 03 §4).
//!
//! The panel is **not** a shell screen. It carries no [`ShellUi`](super::ShellUi) marker, so
//! `hide_game_hud` treats it as game chrome: visible in `Playing` and `Paused`,
//! hidden behind the title and New Map screens along with the rest of the HUD.

use bevy::prelude::*;
use rail_sim::{GoalBoard, GoalStatus, StationService};

use crate::palette::{BALLAST_D, BALLAST_M, BG0, HI, OK, OUTLINE, RAIL_L, WARN};
use crate::ui::kit::{
    micro_font, panel_node, text_secondary, WorldClickBlocker, SPACE_1, SPACE_2,
};
use crate::ui::{UiWindow, WindowId};

use super::ShellState;

/// Panel width. Wide enough for `Connect Eastgate to Westbrook` on one line.
const PANEL_W: f32 = 264.0;

/// Meter height (design 03 §8.4: a 4-texel recessed bar).
const METER_H: f32 = 4.0;

/// Root of the panel, so a rebuild can find and drop the previous one.
#[derive(Component)]
pub struct GoalsPanelRoot;

/// Last painted state. The panel is rebuilt whole when this changes, which at
/// six rows is cheaper than diffing and impossible to get out of step.
#[derive(Resource, Debug, Default)]
pub struct GoalsPanelCache {
    signature: String,
}

/// Rebuild the panel whenever the board's readout changes.
///
/// An empty signature means "no panel", which covers three cases at once: a
/// sandbox world, a goals world whose set has not been derived yet, and any
/// screen that is not the game. The last of those is why the panel is despawned
/// rather than merely hidden behind the title — it is cheap at six rows, and it
/// leaves nothing to go stale while the shell owns the screen.
pub fn rebuild_goals_panel(
    mut commands: Commands,
    state: Res<State<ShellState>>,
    board: Option<Res<GoalBoard>>,
    service: Option<Res<StationService>>,
    mut cache: ResMut<GoalsPanelCache>,
    existing: Query<Entity, With<GoalsPanelRoot>>,
) {
    // Paused keeps it up: design 09 §4 dims the world rather than hiding it, so
    // the player can still see what they were working toward.
    let in_play = matches!(state.get(), ShellState::Playing | ShellState::Paused);
    let now = service.map(|s| s.tick).unwrap_or(0);
    let signature = board
        .as_ref()
        .filter(|b| in_play && b.is_active() && !b.is_empty())
        .map(|b| panel_signature(b, now))
        .unwrap_or_default();

    if signature == cache.signature && (signature.is_empty() == existing.is_empty()) {
        return;
    }
    cache.signature = signature.clone();
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    if signature.is_empty() {
        return;
    }
    // Unwrap is safe: an empty signature is the only way past the filter above.
    let board = board.expect("a signature means a board");

    // Position, open state and stacking belong to the window manager (03 §5).
    let (node, bg, border) = panel_node(Node {
        position_type: PositionType::Absolute,
        width: Val::Px(PANEL_W),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(SPACE_2),
        padding: UiRect::all(Val::Px(SPACE_2)),
        display: Display::None,
        ..default()
    });

    commands
        .spawn((
            GoalsPanelRoot,
            UiWindow::new(WindowId::Goals),
            WorldClickBlocker,
            Interaction::default(),
            node,
            bg,
            border,
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new(board.summary_line()),
                micro_font(),
                text_secondary(),
            ));

            for goal in board.iter() {
                spawn_goal_row(
                    panel,
                    &goal.title,
                    goal.percent(),
                    goal.status,
                    &format!(
                        "{} - {}",
                        goal.progress_label(),
                        goal.time_label(now)
                    ),
                );
            }
        });
}

/// One goal: title, meter, readout.
fn spawn_goal_row(
    parent: &mut ChildSpawnerCommands,
    title: &str,
    percent: u32,
    status: GoalStatus,
    readout: &str,
) {
    let (fill, text) = row_colours(percent, status);
    parent
        .spawn(Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(SPACE_1),
            ..default()
        })
        .with_children(|row| {
            row.spawn((Text::new(title.to_string()), micro_font(), TextColor(text)));

            // Recessed track, filled portion inside it.
            row.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(METER_H),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::ZERO,
                    ..default()
                },
                BackgroundColor(BG0),
                BorderColor::all(OUTLINE),
            ))
            .with_children(|track| {
                track.spawn((
                    Node {
                        width: Val::Percent(percent as f32),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(fill),
                ));
            });

            row.spawn((
                Text::new(format!("{}  {readout}", status.label())),
                micro_font(),
                TextColor(BALLAST_M),
            ));
        });
}

/// Meter fill and title colour. Design 03 §8.4 bands, with resolved goals
/// overriding — a missed goal is spent, not alarming.
fn row_colours(percent: u32, status: GoalStatus) -> (Color, Color) {
    match status {
        GoalStatus::Complete => (OK, OK),
        GoalStatus::Failed => (BALLAST_D, BALLAST_M),
        GoalStatus::Active => {
            let fill = if percent < 34 {
                WARN
            } else if percent < 67 {
                HI
            } else {
                OK
            };
            (fill, RAIL_L)
        }
    }
}

/// Everything the panel draws, flattened. Cheap to build and cheap to compare.
fn panel_signature(board: &GoalBoard, now: u64) -> String {
    let mut out = board.summary_line();
    for goal in board.iter() {
        out.push('|');
        out.push_str(&goal.title);
        out.push('#');
        out.push_str(&goal.percent().to_string());
        out.push('#');
        out.push_str(goal.status.label());
        out.push('#');
        out.push_str(&goal.progress_label());
        out.push('#');
        out.push_str(&goal.time_label(now));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::{Goal, GoalId, GoalKind, GoalMode};

    fn board() -> GoalBoard {
        let mut board = GoalBoard::default();
        board.start(GoalMode::Goals, 42);
        board.install(vec![
            Goal::new(GoalId(0), GoalKind::Deliveries, "Complete 40 paid runs", 40, 100),
            Goal::new(GoalId(1), GoalKind::Population, "Grow the town", 50, 200),
        ]);
        board
    }

    #[test]
    fn a_sandbox_world_draws_no_panel_at_all() {
        let mut sandbox = board();
        sandbox.start(GoalMode::Sandbox, 42);
        assert!(!sandbox.is_active());
        // The system's filter is the same predicate; an inactive board yields
        // no signature, and no signature means no panel.
        assert!(!sandbox.is_active() || sandbox.is_empty());
    }

    #[test]
    fn the_signature_moves_when_progress_does() {
        let mut board = board();
        let before = panel_signature(&board, 0);
        board.iter_mut().next().unwrap().current = 20;
        assert_ne!(panel_signature(&board, 0), before);
    }

    #[test]
    fn the_signature_also_moves_as_a_deadline_approaches() {
        // Otherwise the countdown would freeze until progress happened to change.
        let board = board();
        assert_ne!(panel_signature(&board, 0), panel_signature(&board, 90));
    }

    #[test]
    fn the_meter_walks_the_design_bands() {
        assert_eq!(row_colours(0, GoalStatus::Active).0, WARN);
        assert_eq!(row_colours(50, GoalStatus::Active).0, HI);
        assert_eq!(row_colours(90, GoalStatus::Active).0, OK);
        assert_eq!(row_colours(100, GoalStatus::Complete).0, OK);
    }

    #[test]
    fn a_missed_goal_reads_as_spent_rather_than_as_an_alarm() {
        // Nothing in this game shouts at the player for missing a deadline.
        let (fill, _) = row_colours(40, GoalStatus::Failed);
        assert_ne!(fill, WARN);
        assert_eq!(GoalStatus::Failed.label(), "missed");
    }

    #[test]
    fn every_row_says_its_state_in_words() {
        // Design 03 §4 — colour never carries meaning on its own.
        for status in [GoalStatus::Active, GoalStatus::Complete, GoalStatus::Failed] {
            assert!(!status.label().is_empty());
        }
    }
}
