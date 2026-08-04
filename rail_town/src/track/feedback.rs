//! Cost HUD, reason chip, and reject flash near the build cursor.
//!
//! # The price is always on screen
//!
//! Brief 04 §2.3 wants the running total and the balance it would leave
//! visible while building, and "while building" means *whenever there is a
//! ghost*, not "while a mouse button is held". Those are different windows: the
//! Build tool keeps its anchor after every commit, so the ghost follows the
//! cursor between drags, and a continuous-build player spends most of their
//! time in exactly that state. Reading the readout off the drag left them
//! pricing a run they could see and could not cost.
//!
//! Both the readout and the reason chip therefore key off the *preview*. If a
//! ghost is on screen its price is on screen, from the first tile, in every
//! mode; and if the pointer has left the window they fall back to a fixed
//! corner rather than disappearing with it.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{tile_to_world, TILE_SIZE};
use rail_sim::ids::TileCoord;
use rail_sim::{PlacementError, TrackEdit};

use super::preview::{format_dollars, placement_reason, BuildPreview, DemolishPreview, RejectInfo};
use super::tools::TrackToolState;

/// Palette diagnostics (design brief 01) — UI / ghost only.
const HI: Color = Color::srgb(0xf2 as f32 / 255.0, 0xc1 as f32 / 255.0, 0x4e as f32 / 255.0);
const WARN: Color = Color::srgb(0xe8 as f32 / 255.0, 0x62 as f32 / 255.0, 0x4a as f32 / 255.0);
const OK: Color = Color::srgb(0x6f as f32 / 255.0, 0xd0 as f32 / 255.0, 0x8c as f32 / 255.0);

const CHIP_TTL_SECS: f32 = 2.4;
const FLASH_TTL_SECS: f32 = 0.45;

/// Where the readout and the chip sit relative to the pointer, in logical px.
const HUD_CURSOR_OFFSET: Vec2 = Vec2::new(18.0, 18.0);
const CHIP_CURSOR_OFFSET: Vec2 = Vec2::new(18.0, 52.0);

/// …and where they sit when there is no pointer to sit beside.
///
/// A pointer that has left the window does not mean the player stopped caring
/// what the run costs, so neither readout goes away with it.
const HUD_FALLBACK: Vec2 = Vec2::new(16.0, 88.0);
const CHIP_FALLBACK: Vec2 = Vec2::new(16.0, 120.0);

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

    let hud_payload = cost_hud_line(&state);

    if let Ok((mut node, mut border)) = hud_q.single_mut() {
        if let Some((line, warn)) = hud_payload {
            let pos = cursor.map_or(HUD_FALLBACK, |c| c + HUD_CURSOR_OFFSET);
            node.display = Display::Flex;
            node.left = Val::Px(pos.x);
            node.top = Val::Px(pos.y);
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

    let chip_msg =
        live_reject(&state).or_else(|| feedback.chip.as_ref().map(|c| c.message.as_str()));

    if let Ok(mut node) = chip_q.single_mut() {
        if let Some(msg) = chip_msg {
            let pos = cursor.map_or(CHIP_FALLBACK, |c| c + CHIP_CURSOR_OFFSET);
            node.display = Display::Flex;
            node.left = Val::Px(pos.x);
            node.top = Val::Px(pos.y);
            if let Ok(mut text) = chip_text.single_mut() {
                *text = Text::new(msg.to_string());
            }
        } else {
            node.display = Display::None;
        }
    }

    let _ = OK;
}

/// What the cost readout says this frame, and whether it should read as a
/// warning — `None` when there is nothing proposed to price.
///
/// Keyed off the previews rather than off `state.drag`, which is the whole
/// point: see the module docs.
pub(crate) fn cost_hud_line(state: &TrackToolState) -> Option<(String, bool)> {
    if let Some(preview) = &state.build_preview {
        return Some(cost_hud_build(preview));
    }
    state.demolish_preview.as_ref().map(cost_hud_demolish)
}

/// The refusal the *current* proposal carries, if any.
///
/// Live for as long as the ghost is, so an illegal run explains itself while
/// the player is still pointing at it rather than only once they let go.
fn live_reject(state: &TrackToolState) -> Option<&str> {
    let build = state.build_preview.as_ref().and_then(|p| p.reject.as_ref());
    let demolish = state
        .demolish_preview
        .as_ref()
        .and_then(|p| p.reject.as_ref());
    build.or(demolish).map(|r| r.message.as_str())
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
    // Deck tiles get their own line item: a wide crossing is most of the bill
    // and the player should be able to see that is what they are paying for.
    let bridges = match p.bridge_count {
        0 => String::new(),
        1 => " - 1 bridge tile".to_string(),
        n => format!(" - {n} bridge tiles"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::ids::TileCoord;

    use super::super::preview::{GhostTile, TileGhostKind};
    use super::super::tools::{BuildTool, DragKind};

    fn ghost(x: i32, cost: i64, is_bridge: bool) -> GhostTile {
        GhostTile {
            tile: TileCoord { x, y: 0 },
            kind: TileGhostKind::Place {
                cost_cents: cost,
                is_bridge,
            },
            valid: true,
        }
    }

    fn a_priced_ghost() -> BuildPreview {
        BuildPreview {
            tiles: vec![ghost(0, 10_000, false), ghost(1, 720_000, true)],
            new_tile_count: 2,
            bridge_count: 1,
            total_cost_cents: 730_000,
            balance_after_cents: 270_000,
            can_commit: true,
            reject: None,
            endpoint: TileCoord { x: 1, y: 0 },
        }
    }

    /// The reported bug: the readout was keyed off `state.drag`, so between
    /// drags — which is most of a continuous build, since the anchor survives a
    /// commit — the ghost was on screen with no price on it.
    #[test]
    fn the_readout_follows_the_ghost_not_the_mouse_button() {
        let mut state = TrackToolState {
            tool: BuildTool::Build,
            build_preview: Some(a_priced_ghost()),
            ..Default::default()
        };
        assert!(state.drag.is_none(), "no button is held in this state");
        let (line, warn) = cost_hud_line(&state).expect("a ghost must carry its price");
        assert!(line.contains("2 tiles"), "{line}");
        assert!(line.contains("$7300.00"), "{line}");
        assert!(line.contains("Balance  $2700.00"), "{line}");
        assert!(line.contains("1 bridge tile"), "{line}");
        assert!(!warn);

        // …and it does not change when a button *is* held.
        state.drag = Some(DragKind::Build);
        assert_eq!(cost_hud_line(&state).map(|(l, _)| l), Some(line));

        // Nothing proposed, nothing to say.
        state.build_preview = None;
        assert!(cost_hud_line(&state).is_none());
    }

    /// A run that cannot be paid for still shows its price — that *is* the
    /// information — and reads as a warning.
    #[test]
    fn an_unaffordable_run_still_prices_itself_and_warns() {
        let mut preview = a_priced_ghost();
        preview.balance_after_cents = -50_000;
        preview.can_commit = false;
        preview.reject = Some(RejectInfo {
            message: "Short by $500.00".into(),
            tiles: vec![TileCoord { x: 1, y: 0 }],
        });
        let state = TrackToolState {
            tool: BuildTool::Build,
            build_preview: Some(preview),
            ..Default::default()
        };
        let (line, warn) = cost_hud_line(&state).expect("priced even when refused");
        assert!(warn, "{line}");
        assert!(line.contains("$7300.00"), "{line}");
        assert_eq!(live_reject(&state), Some("Short by $500.00"));
    }

    /// End to end through the real system: whenever a build preview exists the
    /// readout entity is visible and carries the total and the balance — with
    /// or without a pointer to sit beside.
    #[test]
    fn the_cost_readout_is_on_screen_whenever_a_ghost_is() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .add_message::<TrackEdit>()
            .init_resource::<TrackToolState>()
            .add_systems(Startup, setup_build_feedback)
            .add_systems(Update, update_build_feedback_ui);
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.update();

        let hud_shown = |app: &mut App| {
            let mut q = app.world_mut().query_filtered::<&Node, With<CostHudRoot>>();
            q.single(app.world()).expect("one HUD root").display
        };
        let hud_text = |app: &mut App| {
            let mut q = app.world_mut().query_filtered::<&Text, With<CostHudLine>>();
            q.single(app.world()).expect("one HUD line").0.clone()
        };

        assert_eq!(hud_shown(&mut app), Display::None, "nothing to price yet");

        app.world_mut()
            .resource_mut::<TrackToolState>()
            .build_preview = Some(a_priced_ghost());
        app.update();
        assert_eq!(hud_shown(&mut app), Display::Flex, "a ghost with no price");
        let line = hud_text(&mut app);
        assert!(line.contains("$7300.00"), "{line}");
        assert!(line.contains("Balance  $2700.00"), "{line}");

        app.world_mut()
            .resource_mut::<TrackToolState>()
            .build_preview = None;
        app.update();
        assert_eq!(hud_shown(&mut app), Display::None);
    }
}
