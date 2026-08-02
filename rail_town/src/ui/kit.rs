#![allow(dead_code)] // kit surface — not every helper is used by every panel
//! Pixel UI kit helpers — spacing, type roles, panel / button / meter chrome.
//!
//! Binding standard: [`docs/design/03-ui-system.md`](../../../docs/design/03-ui-system.md).
//!
//! **Scale (03 §2).** Every dimension here is a whole number of UI texels, and
//! [`UiScale`](bevy::prelude::UiScale) is only ever a whole number, so the
//! product is always a whole number of pixels. That is the entire reason the
//! player-facing "UI scale" setting is a small integer ladder rather than a
//! percentage slider: a 1-texel border at 1.25× is 1.25 px, and a pixel game
//! with a 1.25 px border looks cheap.
//!
//! **Bitmap font follow-up:** there is no bitmap pixel-font asset yet. We use
//! Bevy's default font at **integer sizes only** so glyphs stay on hard edges at
//! integer UI scale. Replace with a true bitmap font (Display 11 / Body 7 /
//! Micro 5 texel cap heights × UI scale) when art lands — do not introduce
//! fractional `font_size` values in the meantime.

use bevy::prelude::*;

use crate::palette::{BALLAST_D, BALLAST_L, BALLAST_M, BG0, BG1, HI, OK, OUTLINE, RAIL_L, WARN};

/// Base spacing unit (UI texels). All gaps / insets are multiples of this.
pub const UNIT: f32 = 4.0;

pub const SPACE_1: f32 = UNIT; // 4
pub const SPACE_2: f32 = UNIT * 2.0; // 8
pub const SPACE_3: f32 = UNIT * 3.0; // 12
pub const SPACE_4: f32 = UNIT * 4.0; // 16
pub const SPACE_6: f32 = UNIT * 6.0; // 24
pub const SPACE_8: f32 = UNIT * 8.0; // 32
pub const SPACE_12: f32 = UNIT * 12.0; // 48

/// Menu row height — the top row of verbs and window buttons (03 §5).
pub const MENU_H: f32 = 24.0;
/// Status strip row height (03 §6).
pub const STATUS_ROW_H: f32 = 20.0;
/// Network health strip height (03 §6).
pub const HEALTH_H: f32 = 20.0;

/// Height of the whole top chrome block: menu row, status strip, health strip.
///
/// The block is laid out in flow and measures whatever its rows need; this is
/// the clearance figure everything else uses, rounded up onto the 4-texel grid
/// with room for the row borders.
pub const TOP_CHROME_H: f32 = 76.0;

/// What panels anchor beneath.
///
/// This used to be the status strip's own height, back when the status strip
/// was the only thing at the top of the screen. It now names the clearance
/// under the whole top block, because that is what every caller outside this
/// module ever wanted it for — `top: STATUS_H + SPACE_2` means "just below the
/// bar", and it still does. The strip's own height is [`STATUS_ROW_H`].
pub const STATUS_H: f32 = TOP_CHROME_H;

/// Legacy toolbar slot size. The bottom toolbar is gone (03 §5) but the
/// onboarding hint arrows still measure against it.
pub const TOOL_SLOT: f32 = 32.0;

/// Integer type sizes (stand-in for bitmap font until assets exist).
pub const FONT_DISPLAY: f32 = 16.0;
pub const FONT_BODY: f32 = 12.0;
pub const FONT_MICRO: f32 = 9.0;

/// Hard shadow offset (design: 2 texels, no blur).
pub const SHADOW_OFFSET: f32 = 2.0;

/// Height of a window title bar.
pub const TITLE_BAR_H: f32 = 16.0;

/// Meter bar height (03 §8.4: a 4-texel recessed bar).
pub const METER_H: f32 = 4.0;

/// Opaque panel: square corners, 1-texel outline, `bg1` fill.
pub fn panel_node(extra: Node) -> (Node, BackgroundColor, BorderColor) {
    let mut node = Node {
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::ZERO,
        ..extra
    };
    // Ensure square corners even if caller set a radius.
    node.border_radius = BorderRadius::ZERO;
    (node, BackgroundColor(BG1), BorderColor::all(OUTLINE))
}

/// Raised inner edge colour (design: `ballastM`).
pub fn raised_border() -> BorderColor {
    BorderColor::all(BALLAST_M)
}

/// Recessed fill (`bg0`).
pub fn recessed_bg() -> BackgroundColor {
    BackgroundColor(BG0)
}

pub fn text_primary() -> TextColor {
    TextColor(RAIL_L)
}

pub fn text_secondary() -> TextColor {
    TextColor(BALLAST_L)
}

pub fn text_disabled() -> TextColor {
    TextColor(BALLAST_M)
}

pub fn text_accent() -> TextColor {
    TextColor(HI)
}

pub fn text_warn() -> TextColor {
    TextColor(WARN)
}

pub fn divider_color() -> Color {
    BALLAST_D
}

/// Default body label style.
pub fn body_font() -> TextFont {
    TextFont::from_font_size(FONT_BODY)
}

pub fn display_font() -> TextFont {
    TextFont::from_font_size(FONT_DISPLAY)
}

pub fn micro_font() -> TextFont {
    TextFont::from_font_size(FONT_MICRO)
}

/// Border for a control that can be selected and hovered.
///
/// Selection is `hi`; hover only lightens. Nothing here carries meaning by
/// colour alone (03 §4) — every caller also changes a label or an icon.
pub fn control_border(selected: bool, hovered: bool) -> BorderColor {
    if selected {
        BorderColor::all(HI)
    } else if hovered {
        BorderColor::all(BALLAST_L)
    } else {
        BorderColor::all(OUTLINE)
    }
}

/// Fill colour for a meter, by value (03 §8.4).
pub fn meter_fill(percent: u32) -> Color {
    if percent < 34 {
        WARN
    } else if percent < 67 {
        HI
    } else {
        OK
    }
}

/// A compact chrome button: square, 1-texel outline, `bg1`, micro label.
///
/// Returns the node parts only — the caller adds `Button` plus whatever marker
/// component identifies it, so one helper serves the menu row, the speed
/// segments and every window's close box.
pub fn chrome_button_node(padding_x: f32, padding_y: f32) -> (Node, BackgroundColor, BorderColor) {
    (
        Node {
            padding: UiRect::axes(Val::Px(padding_x), Val::Px(padding_y)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::ZERO,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(BG1),
        BorderColor::all(OUTLINE),
    )
}

/// Spawn a recessed meter track with a filled portion.
///
/// A meter never appears without a numeral beside it — see the callers.
pub fn spawn_meter(parent: &mut ChildSpawnerCommands, width: f32, percent: u32, fill: Color) {
    parent
        .spawn((
            Node {
                width: Val::Px(width),
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
                    width: Val::Percent(percent.min(100) as f32),
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(fill),
            ));
        });
}

/// A 1-texel horizontal rule.
pub fn spawn_rule(parent: &mut ChildSpawnerCommands) {
    parent.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(1.0),
            ..default()
        },
        BackgroundColor(BALLAST_D),
    ));
}

/// Convert a cursor position into the space `Val::Px` is written in.
///
/// # The three UI coordinate spaces, and why this exists
///
/// Bevy 0.18 folds [`UiScale`](bevy::prelude::UiScale) into the UI's scale
/// factor (`bevy_ui::update::propagate_ui_target_cameras`:
/// `scale_factor = camera.target_scaling_factor() * ui_scale.0`), and taffy then
/// lays out in **physical** pixels. So:
///
/// | Space | Conversion |
/// | --- | --- |
/// | **UI px** — what you write in `Node`, `Val::Px` | the space this returns |
/// | **Logical window px** — what [`Window::cursor_position`](bevy::window::Window::cursor_position) returns | `ui_px × ui_scale` |
/// | **Physical px** — `ComputedNode`, and a UI node's `GlobalTransform` | `ui_px × ui_scale × window.scale_factor` |
///
/// `ComputedNode::inverse_scale_factor` is `1.0 / (window.scale_factor × ui_scale)`,
/// which is the way back from the third row to the first.
///
/// **Anything that turns a cursor position into a `Val::Px` must divide by
/// `UiScale` first.** Skipping it places the element `ui_scale` times too far
/// right and down — at the old default of `2×`, comfortably off the screen. If a
/// panel or tooltip is landing far from the pointer, this is the first thing to
/// check.
#[inline]
pub fn cursor_to_ui(cursor_logical: Vec2, ui_scale: f32) -> Vec2 {
    cursor_logical / ui_scale.max(f32::EPSILON)
}

/// Screen size in UI px — the space window positions are clamped against.
#[inline]
pub fn screen_to_ui(logical_size: Vec2, ui_scale: f32) -> Vec2 {
    cursor_to_ui(logical_size, ui_scale)
}

/// Marker on opaque chrome that should swallow world clicks (inspector, etc.).
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct WorldClickBlocker;

/// True when the pointer is over interactive chrome (buttons or blockers).
pub fn pointer_blocks_world(
    interactions: &Query<&Interaction, Or<(With<Button>, With<WorldClickBlocker>)>>,
) -> bool {
    interactions
        .iter()
        .any(|i| matches!(i, Interaction::Hovered | Interaction::Pressed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_metric_sits_on_the_four_texel_grid() {
        // 03 §2: base unit is 4 UI texels and nothing falls between the steps.
        for value in [
            SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_6, SPACE_8, SPACE_12, MENU_H, STATUS_ROW_H,
            HEALTH_H, TOOL_SLOT, METER_H, TITLE_BAR_H, TOP_CHROME_H,
        ] {
            assert_eq!(value % UNIT, 0.0, "{value} is off the 4-texel grid");
        }
    }

    #[test]
    fn type_sizes_are_whole_numbers() {
        // A fractional font size at any integer UI scale is still fractional.
        for size in [FONT_DISPLAY, FONT_BODY, FONT_MICRO] {
            assert_eq!(size.fract(), 0.0, "{size} is not a whole number");
        }
        assert!(FONT_DISPLAY > FONT_BODY && FONT_BODY > FONT_MICRO);
    }

    #[test]
    fn the_meter_walks_the_design_bands() {
        assert_eq!(meter_fill(0), WARN);
        assert_eq!(meter_fill(50), HI);
        assert_eq!(meter_fill(90), OK);
    }

    #[test]
    fn a_cursor_position_divides_by_ui_scale_to_reach_val_px() {
        // Bevy folds UiScale into the UI scale factor, so a Node written in
        // Val::Px lands `ui_scale` times further out than the raw cursor value.
        // At 1x the two spaces coincide, which is exactly why a bug here hides
        // until someone changes the scale.
        assert_eq!(cursor_to_ui(Vec2::new(400.0, 200.0), 1.0), Vec2::new(400.0, 200.0));
        assert_eq!(cursor_to_ui(Vec2::new(400.0, 200.0), 2.0), Vec2::new(200.0, 100.0));
        assert_eq!(screen_to_ui(Vec2::new(1280.0, 720.0), 2.0), Vec2::new(640.0, 360.0));
    }

    #[test]
    fn a_zero_ui_scale_does_not_produce_infinities() {
        let out = cursor_to_ui(Vec2::new(10.0, 10.0), 0.0);
        assert!(out.x.is_finite() && out.y.is_finite());
    }
}
