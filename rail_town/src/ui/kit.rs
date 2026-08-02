#![allow(dead_code)] // kit surface — not every helper is used in Phase A
//! Pixel UI kit helpers — spacing, type roles, panel / button chrome.
//!
//! Binding standard: [`docs/design/03-ui-system.md`](../../../docs/design/03-ui-system.md).
//!
//! **Bitmap font follow-up:** there is no bitmap pixel-font asset yet. We use Bevy’s
//! default font at **integer sizes only** (10 / 14 / 22) so glyphs stay on hard
//! edges at integer UI scale. Replace with a true bitmap font (Display 11 / Body 7 /
//! Micro 5 texel cap heights × UI scale) when art lands — do not introduce
//! fractional `font_size` values in the meantime.

use bevy::prelude::*;

use crate::palette::{BALLAST_D, BALLAST_L, BALLAST_M, BG0, BG1, HI, OUTLINE, RAIL_L, WARN};

/// Base spacing unit (UI texels). All gaps / insets are multiples of this.
pub const UNIT: f32 = 4.0;

pub const SPACE_1: f32 = UNIT; // 4
pub const SPACE_2: f32 = UNIT * 2.0; // 8
pub const SPACE_3: f32 = UNIT * 3.0; // 12
pub const SPACE_4: f32 = UNIT * 4.0; // 16
pub const SPACE_6: f32 = UNIT * 6.0; // 24
pub const SPACE_8: f32 = UNIT * 8.0; // 32
pub const SPACE_12: f32 = UNIT * 12.0; // 48

/// Status strip height (design: 24).
pub const STATUS_H: f32 = SPACE_6;
/// Toolbar slot size (design: 48×48).
pub const TOOL_SLOT: f32 = SPACE_12;

/// Integer type sizes (stand-in for bitmap font until assets exist).
pub const FONT_DISPLAY: f32 = 22.0;
pub const FONT_BODY: f32 = 14.0;
pub const FONT_MICRO: f32 = 10.0;

/// Hard shadow offset (design: 2 texels, no blur).
pub const SHADOW_OFFSET: f32 = 2.0;

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
