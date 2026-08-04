//! Catchment ring, platform ghost, and the station tool's cost readout.
//!
//! The ring is drawn live during the hover so siting a stop is a decision made
//! with information rather than a guess ([04 — Building & Tools] §6). Colour is
//! diagnostic only — `HI` while the site is legal, `WARN` while it is not.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::tile_to_world;

use crate::map::TileMark;
use crate::palette::{BG0, HI, OUTLINE, WARN};
use crate::ui::kit;

use super::preview::station_hud_line;
use super::tools::StationToolState;

/// Ghost sprite for one catchment-ring or platform tile.
#[derive(Component, Debug, Clone, Copy)]
pub struct StationGhost;

#[derive(Component)]
pub(crate) struct StationHudRoot;

#[derive(Component)]
pub(crate) struct StationHudLine;

#[derive(Component)]
pub(crate) struct StationHudReason;

pub fn setup_station_hud(mut commands: Commands) {
    commands
        .spawn((
            StationHudRoot,
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(kit::SPACE_1),
                padding: UiRect::all(Val::Px(kit::SPACE_2)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(BG0.with_alpha(0.9)),
            BorderColor::all(OUTLINE),
            ZIndex(20),
        ))
        .with_children(|p| {
            p.spawn((
                StationHudLine,
                Text::new(""),
                kit::body_font(),
                TextColor(HI),
            ));
            p.spawn((
                StationHudReason,
                Text::new(""),
                kit::micro_font(),
                TextColor(WARN),
            ));
        });
}

/// Redraw the ring / platform ghost from the tool's live preview.
pub fn sync_station_ghosts(
    mut commands: Commands,
    state: Res<StationToolState>,
    mark: Option<Res<TileMark>>,
    existing: Query<Entity, With<StationGhost>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    if !state.active {
        return;
    }
    // The ring is tile-shaped, so it takes the tile's shape in either view.
    let Some(mark) = mark else {
        return;
    };
    let Some(preview) = state.preview.as_ref() else {
        return;
    };

    let tint = if preview.can_commit { HI } else { WARN };

    for &tile in &preview.ring {
        let (wx, wy) = tile_to_world(tile);
        commands.spawn((
            StationGhost,
            mark.square(tint.with_alpha(0.22), 0.9),
            Transform::from_xyz(wx, wy, 3.2),
        ));
    }
    for &tile in &preview.platforms {
        let (wx, wy) = tile_to_world(tile);
        commands.spawn((
            StationGhost,
            mark.square(tint.with_alpha(0.6), 0.75),
            Transform::from_xyz(wx, wy, 3.4),
        ));
    }
}

/// Follow the cursor with the tier / cost / catchment readout.
pub fn update_station_hud(
    windows: Query<&Window, With<PrimaryWindow>>,
    state: Res<StationToolState>,
    mut root: Query<(&mut Node, &mut BorderColor), With<StationHudRoot>>,
    mut line: Query<&mut Text, With<StationHudLine>>,
    mut reason: Query<&mut Text, (With<StationHudReason>, Without<StationHudLine>)>,
) {
    let Ok((mut node, mut border)) = root.single_mut() else {
        return;
    };
    let payload = state
        .preview
        .as_ref()
        .filter(|_| state.active)
        .map(|p| (station_hud_line(p), p.reject.clone(), p.can_commit));

    let Some((body, live_reject, ok)) = payload else {
        node.display = Display::None;
        return;
    };
    let Some(cursor) = windows.single().ok().and_then(|w| w.cursor_position()) else {
        node.display = Display::None;
        return;
    };

    node.display = Display::Flex;
    node.left = Val::Px(cursor.x + 18.0);
    node.top = Val::Px(cursor.y + 18.0);
    *border = BorderColor::all(if ok { HI } else { WARN });

    if let Ok(mut text) = line.single_mut() {
        *text = Text::new(body);
    }
    if let Ok(mut text) = reason.single_mut() {
        // The live reason wins; a sticky one from the last refused click follows.
        let msg = live_reject
            .or_else(|| state.reject.clone())
            .unwrap_or_default();
        *text = Text::new(msg);
    }
}
