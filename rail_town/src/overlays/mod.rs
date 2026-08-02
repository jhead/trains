//! Toggleable diagnostic overlays (service, congestion, density).
//!
//! `Tab` cycles; direct keys: `F1` service, `F2` congestion, `F3` density, `F4` off.
//! Overlays tint with palette diagnostics (`OK` / `HI` / `WARN`) only.

mod render;
mod score;

use bevy::prelude::*;

use render::sync_overlay_sprites;

/// Active world tint mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum OverlayKind {
    #[default]
    None,
    Service,
    Congestion,
    Density,
}

impl OverlayKind {
    pub const CYCLE: [OverlayKind; 4] = [
        OverlayKind::None,
        OverlayKind::Service,
        OverlayKind::Congestion,
        OverlayKind::Density,
    ];

    pub fn next(self) -> Self {
        let i = Self::CYCLE.iter().position(|&k| k == self).unwrap_or(0);
        Self::CYCLE[(i + 1) % Self::CYCLE.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "Overlay off",
            Self::Service => "Service",
            Self::Congestion => "Congestion",
            Self::Density => "Density",
        }
    }
}

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ActiveOverlay(pub OverlayKind);

pub struct OverlaysPlugin;

impl Plugin for OverlaysPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActiveOverlay>()
            .add_systems(Startup, setup_overlay_legend)
            .add_systems(
                Update,
                (
                    overlay_hotkeys,
                    sync_overlay_sprites,
                    update_overlay_legend.after(overlay_hotkeys),
                ),
            );
    }
}

#[derive(Component)]
struct OverlayLegendRoot;

#[derive(Component)]
struct OverlayLegendText;

fn setup_overlay_legend(mut commands: Commands) {
    use crate::palette::{BG1, OUTLINE};
    use crate::ui::kit::{micro_font, text_secondary, SPACE_2, SPACE_3};

    commands
        .spawn((
            OverlayLegendRoot,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(SPACE_3 + 52.0),
                right: Val::Px(SPACE_3),
                padding: UiRect::all(Val::Px(SPACE_2)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::ZERO,
                display: Display::None,
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(BG1),
            BorderColor::all(OUTLINE),
            ZIndex(5),
        ))
        .with_children(|p| {
            p.spawn((
                OverlayLegendText,
                Text::new(""),
                micro_font(),
                text_secondary(),
            ));
        });
}

fn overlay_hotkeys(keys: Res<ButtonInput<KeyCode>>, mut overlay: ResMut<ActiveOverlay>) {
    if keys.just_pressed(KeyCode::Tab) {
        overlay.0 = overlay.0.next();
    }
    if keys.just_pressed(KeyCode::F1) {
        overlay.0 = OverlayKind::Service;
    }
    if keys.just_pressed(KeyCode::F2) {
        overlay.0 = OverlayKind::Congestion;
    }
    if keys.just_pressed(KeyCode::F3) {
        overlay.0 = OverlayKind::Density;
    }
    if keys.just_pressed(KeyCode::F4) {
        overlay.0 = OverlayKind::None;
    }
}

fn update_overlay_legend(
    overlay: Res<ActiveOverlay>,
    mut root: Query<&mut Node, With<OverlayLegendRoot>>,
    mut text: Query<&mut Text, With<OverlayLegendText>>,
) {
    let Ok(mut node) = root.single_mut() else {
        return;
    };
    let Ok(mut label) = text.single_mut() else {
        return;
    };

    if overlay.0 == OverlayKind::None {
        node.display = Display::None;
        return;
    }
    node.display = Display::Flex;
    let hint = match overlay.0 {
        OverlayKind::Service => "green=good - amber=fair - red=poor",
        OverlayKind::Congestion => "red=occupied - amber=busy corridor",
        OverlayKind::Density => "brighter = denser buildings",
        OverlayKind::None => "",
    };
    *label = Text::new(format!("{}  (Tab)\n{hint}", overlay.0.label()));
}
