//! Cost HUD, reason chip, and reject flash near the build cursor.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{tile_to_world, TILE_SIZE};
use rail_sim::ids::TileCoord;
use rail_sim::{PlacementError, TrackEdit};

use super::preview::{format_dollars, placement_reason, BuildPreview, DemolishPreview, RejectInfo};
use super::tools::{DragKind, TrackToolState};

/// Palette diagnostics (design brief 01) — UI / ghost only.
const HI: Color = Color::srgb(0xf2 as f32 / 255.0, 0xc1 as f32 / 255.0, 0x4e as f32 / 255.0);
const WARN: Color = Color::srgb(0xe8 as f32 / 255.0, 0x62 as f32 / 255.0, 0x4a as f32 / 255.0);
const OK: Color = Color::srgb(0x6f as f32 / 255.0, 0xd0 as f32 / 255.0, 0x8c as f32 / 255.0);

const CHIP_TTL_SECS: f32 = 2.4;
const FLASH_TTL_SECS: f32 = 0.45;

#[derive(Resource, Debug, Clone, Default)]
pub struct BuildFeedback {
    /// Sticky reject chip (survives after release).
    pub chip: Option<ReasonChip>,
    pub flashes: Vec<TileFlash>,
}

#[derive(Debug, Clone)]
pub struct ReasonChip {
    pub message: String,
    pub ttl: f32,
}

#[derive(Debug, Clone)]
pub struct TileFlash {
    pub tile: TileCoord,
    pub ttl: f32,
}

#[derive(Component)]
pub(crate) struct CostHudRoot;

#[derive(Component)]
pub(crate) struct CostHudLine;

#[derive(Component)]
pub(crate) struct ReasonChipRoot;

#[derive(Component)]
pub(crate) struct ReasonChipText;

#[derive(Component)]
pub(crate) struct FlashSprite;

pub fn setup_build_feedback(mut commands: Commands) {
    commands.insert_resource(BuildFeedback::default());

    commands
        .spawn((
            CostHudRoot,
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.0),
                padding: UiRect::all(Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.07, 0.08, 0.1, 0.88)),
            BorderColor::all(Color::srgba(0.95, 0.76, 0.3, 0.55)),
            ZIndex(20),
        ))
        .with_children(|p| {
            p.spawn((
                CostHudLine,
                Text::new(""),
                TextFont::from_font_size(14.0),
                TextColor(HI),
            ));
        });

    commands
        .spawn((
            ReasonChipRoot,
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.12, 0.06, 0.05, 0.92)),
            BorderColor::all(WARN),
            ZIndex(21),
        ))
        .with_children(|p| {
            p.spawn((
                ReasonChipText,
                Text::new(""),
                TextFont::from_font_size(14.0),
                TextColor(WARN),
            ));
        });
}

pub fn push_reject(feedback: &mut BuildFeedback, reject: &RejectInfo) {
    feedback.chip = Some(ReasonChip {
        message: reject.message.clone(),
        ttl: CHIP_TTL_SECS,
    });
    for &tile in &reject.tiles {
        feedback.flashes.push(TileFlash {
            tile,
            ttl: FLASH_TTL_SECS,
        });
    }
}

pub fn push_placement_fail(
    feedback: &mut BuildFeedback,
    error: PlacementError,
    tile: Option<TileCoord>,
    total_cost: i64,
    balance: i64,
) {
    let message = placement_reason(error, total_cost, balance);
    let tiles = tile.into_iter().collect();
    push_reject(feedback, &RejectInfo { message, tiles });
}

pub fn update_build_feedback_ui(
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    state: Res<TrackToolState>,
    mut feedback: ResMut<BuildFeedback>,
    mut fails: MessageReader<TrackEdit>,
    mut hud_q: Query<(&mut Node, &mut BorderColor), With<CostHudRoot>>,
    mut hud_text: Query<&mut Text, With<CostHudLine>>,
    mut chip_q: Query<&mut Node, (With<ReasonChipRoot>, Without<CostHudRoot>)>,
    mut chip_text: Query<&mut Text, (With<ReasonChipText>, Without<CostHudLine>)>,
) {
    for edit in fails.read() {
        if let TrackEdit::Failed { error, tile } = *edit {
            push_placement_fail(&mut feedback, error, tile, 0, 0);
        }
    }

    let dt = time.delta_secs();
    if let Some(chip) = feedback.chip.as_mut() {
        chip.ttl -= dt;
        if chip.ttl <= 0.0 {
            feedback.chip = None;
        }
    }
    feedback.flashes.retain_mut(|f| {
        f.ttl -= dt;
        f.ttl > 0.0
    });

    let Ok(window) = windows.single() else {
        return;
    };
    let cursor = window.cursor_position();

    let hud_payload = match (&state.drag, &state.build_preview, &state.demolish_preview) {
        (Some(DragKind::Build), Some(p), _) => Some(cost_hud_build(p)),
        (Some(DragKind::Demolish), _, Some(p)) => Some(cost_hud_demolish(p)),
        _ => None,
    };

    if let Ok((mut node, mut border)) = hud_q.single_mut() {
        if let (Some((line, warn)), Some(pos)) = (hud_payload, cursor) {
            node.display = Display::Flex;
            node.left = Val::Px(pos.x + 18.0);
            node.top = Val::Px(pos.y + 18.0);
            *border = BorderColor::all(if warn {
                Color::srgba(0.91, 0.38, 0.29, 0.85)
            } else {
                Color::srgba(0.95, 0.76, 0.3, 0.55)
            });
            if let Ok(mut text) = hud_text.single_mut() {
                *text = Text::new(line);
            }
        } else {
            node.display = Display::None;
        }
    }

    let live_reject = match (&state.drag, &state.build_preview, &state.demolish_preview) {
        (Some(DragKind::Build), Some(p), _) => p.reject.as_ref().map(|r| r.message.as_str()),
        (Some(DragKind::Demolish), _, Some(p)) => p.reject.as_ref().map(|r| r.message.as_str()),
        _ => None,
    };
    let chip_msg = live_reject.or_else(|| feedback.chip.as_ref().map(|c| c.message.as_str()));

    if let Ok(mut node) = chip_q.single_mut() {
        if let (Some(msg), Some(pos)) = (chip_msg, cursor) {
            node.display = Display::Flex;
            node.left = Val::Px(pos.x + 18.0);
            node.top = Val::Px(pos.y + 52.0);
            if let Ok(mut text) = chip_text.single_mut() {
                *text = Text::new(msg.to_string());
            }
        } else if let Some(msg) = chip_msg {
            node.display = Display::Flex;
            node.left = Val::Px(16.0);
            node.top = Val::Px(120.0);
            if let Ok(mut text) = chip_text.single_mut() {
                *text = Text::new(msg.to_string());
            }
        } else {
            node.display = Display::None;
        }
    }

    let _ = OK;
}

pub fn sync_flash_sprites(
    mut commands: Commands,
    feedback: Res<BuildFeedback>,
    existing: Query<Entity, With<FlashSprite>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    for flash in &feedback.flashes {
        let (wx, wy) = tile_to_world(flash.tile);
        let alpha = (flash.ttl / FLASH_TTL_SECS).clamp(0.2, 0.85);
        commands.spawn((
            FlashSprite,
            Sprite::from_color(WARN.with_alpha(alpha), Vec2::splat(TILE_SIZE * 0.92)),
            Transform::from_xyz(wx, wy, 3.5),
        ));
    }
}

fn cost_hud_build(p: &BuildPreview) -> (String, bool) {
    let warn = p.reject.is_some() || p.balance_after_cents < 0;
    let bridges = if p.bridge_count > 0 {
        format!(" · {} bridge", p.bridge_count)
    } else {
        String::new()
    };
    let line = format!(
        "{} tiles  {}{}\nBalance  {}",
        p.new_tile_count,
        format_dollars(p.total_cost_cents),
        bridges,
        format_dollars(p.balance_after_cents),
    );
    (line, warn)
}

fn cost_hud_demolish(p: &DemolishPreview) -> (String, bool) {
    let line = format!(
        "Demolish {}  +{}\nBalance  {}",
        p.track_count,
        format_dollars(p.refund_cents),
        format_dollars(p.balance_after_cents),
    );
    (line, p.reject.is_some())
}
