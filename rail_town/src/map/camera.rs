//! Orthographic camera pan (WASD / arrows) and integer zoom (wheel, `+` / `-`, `Z`).
//!
//! Zoom is **1× / 2× / 3×** only (screen pixels per world texel). Ortho scale is
//! `1 / zoom` so a 32px tile stays crisp. Pan snaps to world texels; zoom is
//! cursor-anchored when the pointer is over the window (brief 01 §§2.1, 4).
//!
//! # A wheel and a trackpad are not the same gesture
//!
//! The ladder has three rungs and no rungs between them (§2.1), so every scroll
//! event that reaches it has to be worth a whole rung. A mouse wheel already is:
//! [`MouseScrollUnit::Line`] arrives once per detent, one deliberate notch of a
//! physical ratchet, and steps immediately.
//!
//! A trackpad has no detents. It emits [`MouseScrollUnit::Pixel`] — a stream of
//! small deltas, tens of them per flick, and macOS keeps sending them on inertia
//! for a second after the fingers have lifted. Stepping the ladder per event ran
//! it end to end on the lightest two-finger flick. Pixel scroll is therefore
//! *banked* ([`ZoomScroll`]) and spends a rung only when the bank crosses
//! [`PIXEL_STEP`], after which the bank is emptied and a [`STEP_COOLDOWN_SECS`]
//! gate closes so an inertia tail cannot immediately buy the next one. The bank
//! also bleeds away at [`PIXEL_DECAY_PER_SEC`], so a scroll slower than that
//! never reaches a rung at all however long it goes on.

use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::time::Real;
use bevy::window::PrimaryWindow;
use rail_map::{map_center_world, MapGrid};

use crate::input::{ControlAction, KeyBindings};

/// Allowed zoom multipliers (screen pixels per world texel). Nothing between or outside.
pub const ZOOM_FACTORS: [u8; 3] = [1, 2, 3];
/// Default zoom.
///
/// Brief 01 §2.1 says 2×. **Iso prototype**: the projection makes a tile 64 × 32
/// screen texels instead of 32 × 32, so 2× shows about ten tiles across — too
/// tight to read a ridge, which is the whole thing being evaluated. 1× shows
/// roughly twenty and is where this branch opens. The ladder is untouched:
/// 2× and 3× are still a scroll away.
pub const DEFAULT_ZOOM_FACTOR: u8 = 1;
pub(crate) const PAN_SPEED: f32 = 400.0;
/// Default index into [`ZOOM_FACTORS`].
pub(crate) const DEFAULT_ZOOM_INDEX: usize = 0; // ZOOM_FACTORS[0] == 1

/// Pixel-unit scroll that buys one rung of the ladder.
///
/// About four lines of classic wheel scroll, or a short deliberate two-finger
/// swipe — far more than the handful of pixels a resting hand produces, far
/// less than the several hundred a real flick delivers.
pub(crate) const PIXEL_STEP: f32 = 60.0;
/// How long the ladder is barred after a step, in seconds.
///
/// Long enough that one flick and its inertia tail cannot walk the whole ladder,
/// short enough that two deliberate swipes in a row both land.
pub(crate) const STEP_COOLDOWN_SECS: f32 = 0.25;
/// How fast banked pixel scroll bleeds away, in pixels per second.
///
/// The floor on what counts as scrolling: anything slower than this never
/// accumulates, so a drifting finger or a hand resting on the trackpad cannot
/// eventually trip a step.
pub(crate) const PIXEL_DECAY_PER_SEC: f32 = 120.0;

/// Orthographic projection scale for a zoom multiplier (`1×` → `1.0`, `2×` → `0.5`, …).
#[inline]
pub fn ortho_scale_for_zoom(factor: u8) -> f32 {
    debug_assert!(ZOOM_FACTORS.contains(&factor));
    1.0 / f32::from(factor)
}

#[inline]
pub fn zoom_factor_at(index: usize) -> u8 {
    ZOOM_FACTORS[index.min(ZOOM_FACTORS.len() - 1)]
}

#[derive(Component)]
pub struct MapCamera;

#[derive(Component, Debug, Clone, Copy)]
pub struct CameraZoomIndex(pub usize);

/// Scroll banked between ladder steps. Lives in a [`Local`] on [`camera_zoom`];
/// nothing else has any business reading it.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ZoomScroll {
    /// Unspent pixel-unit scroll, signed.
    pixels: f32,
    /// Seconds left before another step is allowed.
    cooldown: f32,
}

impl ZoomScroll {
    /// Fold one frame of scroll into at most one ladder step.
    ///
    /// `dt` is real seconds — zoom answers the hand, not the sim clock, and has
    /// to keep working while the game is paused.
    pub(crate) fn step(&mut self, unit: MouseScrollUnit, delta_y: f32, dt: f32) -> Option<isize> {
        self.cooldown = (self.cooldown - dt).max(0.0);

        // A still frame decays the bank and nothing else. Checked before the
        // unit, because an idle `AccumulatedMouseScroll` reports whatever unit
        // it happens to default to and must not be read as a wheel event.
        if delta_y == 0.0 {
            self.decay(dt);
            return None;
        }

        match unit {
            MouseScrollUnit::Line => {
                // One detent, one rung: a wheel has already done the quantising
                // in hardware, so there is nothing to bank.
                self.pixels = 0.0;
                if self.cooldown > 0.0 {
                    return None;
                }
                self.cooldown = STEP_COOLDOWN_SECS;
                Some(if delta_y > 0.0 { 1 } else { -1 })
            }
            MouseScrollUnit::Pixel => {
                if self.cooldown > 0.0 {
                    // Drop what arrives while the gate is shut rather than
                    // banking it, so an inertia tail cannot have a rung already
                    // paid for the moment the gate opens.
                    self.pixels = 0.0;
                    return None;
                }
                // Reversing abandons the other direction's bank outright: the
                // player has changed their mind, not part-paid for a rung.
                if self.pixels != 0.0 && self.pixels.signum() != delta_y.signum() {
                    self.pixels = 0.0;
                }
                self.pixels += delta_y;
                self.decay(dt);
                if self.pixels.abs() < PIXEL_STEP {
                    return None;
                }
                let dir = if self.pixels > 0.0 { 1 } else { -1 };
                self.pixels = 0.0;
                self.cooldown = STEP_COOLDOWN_SECS;
                Some(dir)
            }
        }
    }

    /// Bleed the bank toward zero, never through it.
    fn decay(&mut self, dt: f32) {
        let bleed = PIXEL_DECAY_PER_SEC * dt;
        if self.pixels.abs() <= bleed {
            self.pixels = 0.0;
        } else {
            self.pixels -= bleed * self.pixels.signum();
        }
    }

    /// A key press is its own deliberate act — it neither waits for the gate nor
    /// banks anything, but it does close the gate behind it so a key and a
    /// flick cannot compound.
    fn take_key_step(&mut self, step: isize) -> isize {
        self.pixels = 0.0;
        self.cooldown = STEP_COOLDOWN_SECS;
        step
    }
}

/// One-shot camera fly-to request (alerts / Town Talk). Consumed by [`apply_camera_focus`].
#[derive(Resource, Debug, Default)]
pub struct CameraFocusRequest(pub Option<Vec2>);

pub fn setup_map_camera(mut commands: Commands, map: Res<MapGrid>) {
    let (cx, cy) = map_center_world(map.width, map.height);
    commands.spawn((
        Camera2d,
        MapCamera,
        CameraZoomIndex(DEFAULT_ZOOM_INDEX),
        Transform::from_xyz(cx.round(), cy.round(), 1000.0),
        Projection::Orthographic(OrthographicProjection {
            scale: ortho_scale_for_zoom(DEFAULT_ZOOM_FACTOR),
            ..OrthographicProjection::default_2d()
        }),
    ));
}

pub fn apply_camera_focus(
    mut request: ResMut<CameraFocusRequest>,
    mut q: Query<&mut Transform, With<MapCamera>>,
) {
    let Some(target) = request.0.take() else {
        return;
    };
    let Ok(mut transform) = q.single_mut() else {
        return;
    };
    transform.translation.x = target.x.round();
    transform.translation.y = target.y.round();
}

/// Pan on the bound keys, with the arrows as a fixed alternate.
///
/// The arrows are deliberately not in the input map: they are the accessible
/// second route to the same verb (03 §10.3, and the Controls tab says so), and
/// a player who rebinds `WASD` still expects them to work.
pub fn camera_pan(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    mut q: Query<&mut Transform, With<MapCamera>>,
) {
    let Ok(mut transform) = q.single_mut() else {
        return;
    };

    let mut dir = Vec2::ZERO;
    for (action, arrow, delta) in [
        (ControlAction::PanUp, KeyCode::ArrowUp, Vec2::Y),
        (ControlAction::PanDown, KeyCode::ArrowDown, Vec2::NEG_Y),
        (ControlAction::PanLeft, KeyCode::ArrowLeft, Vec2::NEG_X),
        (ControlAction::PanRight, KeyCode::ArrowRight, Vec2::X),
    ] {
        if bindings.pressed(&keys, action) || keys.pressed(arrow) {
            dir += delta;
        }
    }

    if dir != Vec2::ZERO {
        let delta = dir.normalize() * PAN_SPEED * time.delta_secs();
        transform.translation.x += delta.x;
        transform.translation.y += delta.y;
    }

    // Snap after integration so tiles stay on pixel boundaries while moving.
    transform.translation.x = transform.translation.x.round();
    transform.translation.y = transform.translation.y.round();
}

pub fn camera_zoom(
    time: Res<Time<Real>>,
    scroll: Res<AccumulatedMouseScroll>,
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut banked: Local<ZoomScroll>,
    mut q: Query<
        (
            &mut Projection,
            &mut Transform,
            &mut CameraZoomIndex,
            &Camera,
            &GlobalTransform,
        ),
        With<MapCamera>,
    >,
) {
    // Fold the frame's scroll first, unconditionally: the bank has to decay and
    // the gate has to tick even on frames with no camera to move.
    let mut step = banked.step(scroll.unit, scroll.delta.y, time.delta_secs());

    let Ok((mut projection, mut transform, mut zoom_index, camera, cam_gt)) = q.single_mut()
    else {
        return;
    };

    let Projection::Orthographic(ortho) = projection.as_mut() else {
        return;
    };

    if keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd) {
        step = Some(banked.take_key_step(1));
    } else if keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract) {
        step = Some(banked.take_key_step(-1));
    } else if bindings.just_pressed(&keys, ControlAction::ResetZoom) {
        // Modifier-exact, so `Ctrl+Z` is undo and nothing else: reading the
        // literal, it used to undo the last build *and* reset the zoom.
        banked.take_key_step(0);
        apply_zoom(
            &mut transform,
            ortho,
            &mut zoom_index,
            DEFAULT_ZOOM_INDEX,
            cursor_world(windows, camera, cam_gt),
        );
        return;
    }

    let Some(step) = step else {
        return;
    };

    let next = (zoom_index.0 as isize + step).clamp(0, (ZOOM_FACTORS.len() - 1) as isize) as usize;
    if next == zoom_index.0 {
        return;
    }

    apply_zoom(
        &mut transform,
        ortho,
        &mut zoom_index,
        next,
        cursor_world(windows, camera, cam_gt),
    );
}

fn cursor_world(
    windows: Query<&Window, With<PrimaryWindow>>,
    camera: &Camera,
    cam_gt: &GlobalTransform,
) -> Option<Vec2> {
    let Ok(window) = windows.single() else {
        return None;
    };
    let cursor = window.cursor_position()?;
    camera.viewport_to_world_2d(cam_gt, cursor).ok()
}

/// Keep `anchor` (world) under the same screen point when changing ortho scale.
fn apply_zoom(
    transform: &mut Transform,
    ortho: &mut OrthographicProjection,
    zoom_index: &mut CameraZoomIndex,
    next: usize,
    anchor: Option<Vec2>,
) {
    let old_scale = ortho.scale;
    let new_scale = ortho_scale_for_zoom(zoom_factor_at(next));
    zoom_index.0 = next;
    ortho.scale = new_scale;

    if let Some(world) = anchor {
        let cam = transform.translation.truncate();
        let new_cam = world - (world - cam) * (new_scale / old_scale);
        transform.translation.x = new_cam.x.round();
        transform.translation.y = new_cam.y.round();
    } else {
        transform.translation.x = transform.translation.x.round();
        transform.translation.y = transform.translation.y.round();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_factors_are_exactly_one_two_three() {
        assert_eq!(ZOOM_FACTORS, [1, 2, 3]);
        // Iso prototype: the default rung is 1×, not §2.1's 2× — see the
        // constant. The ladder itself is unchanged and still has three rungs.
        assert_eq!(DEFAULT_ZOOM_FACTOR, 1);
        assert_eq!(zoom_factor_at(DEFAULT_ZOOM_INDEX), DEFAULT_ZOOM_FACTOR);
    }

    #[test]
    fn ortho_scale_maps_integer_zoom() {
        assert_eq!(ortho_scale_for_zoom(1), 1.0);
        assert_eq!(ortho_scale_for_zoom(2), 0.5);
        assert!((ortho_scale_for_zoom(3) - 1.0 / 3.0).abs() < f32::EPSILON);
    }

    /// 60 fps, the frame budget the rest of the pixel contract assumes.
    const DT: f32 = 1.0 / 60.0;

    /// Run `frames` frames of `delta` pixels each and count the rungs spent.
    fn pixel_burst(scroll: &mut ZoomScroll, delta: f32, frames: u32) -> i32 {
        (0..frames)
            .filter_map(|_| scroll.step(MouseScrollUnit::Pixel, delta, DT))
            .count() as i32
    }

    /// The bug: a two-finger flick is tens of small pixel deltas, and stepping
    /// on each one ran the ladder end to end.
    #[test]
    fn a_burst_of_small_pixel_deltas_spends_at_most_one_rung() {
        let mut scroll = ZoomScroll::default();
        // Half a second of a brisk trackpad swipe: 30 frames, 8 px each.
        let steps = pixel_burst(&mut scroll, 8.0, 30);
        assert_eq!(steps, 1, "a single swipe must not walk the ladder");
    }

    /// Hard flicks are allowed to be worth more than gentle ones — but only just.
    #[test]
    fn even_a_hard_flick_stays_inside_two_rungs() {
        for delta in [30.0, 60.0, 120.0, 400.0] {
            let mut scroll = ZoomScroll::default();
            // A third of a second at full tilt, which is a hard flick and then some.
            let steps = pixel_burst(&mut scroll, delta, 20);
            assert!(
                (1..=2).contains(&steps),
                "{delta} px/frame gave {steps} rungs"
            );
        }
    }

    /// A wheel detent has already been quantised by the hardware.
    #[test]
    fn a_wheel_detent_is_exactly_one_step() {
        let mut scroll = ZoomScroll::default();
        assert_eq!(scroll.step(MouseScrollUnit::Line, 1.0, DT), Some(1));
        assert_eq!(scroll.step(MouseScrollUnit::Line, -1.0, DT), None, "gated");
        // ... and after the gate, the next detent lands.
        for _ in 0..16 {
            scroll.step(MouseScrollUnit::Line, 0.0, DT);
        }
        assert_eq!(scroll.step(MouseScrollUnit::Line, -1.0, DT), Some(-1));
    }

    /// Several detents inside one frame are still one rung: the ladder has three.
    #[test]
    fn a_frame_holding_several_detents_still_steps_once() {
        let mut scroll = ZoomScroll::default();
        assert_eq!(scroll.step(MouseScrollUnit::Line, 4.0, DT), Some(1));
    }

    /// A hand resting on the trackpad must never eventually zoom.
    #[test]
    fn sub_threshold_drift_never_reaches_a_rung() {
        for delta in [0.4, 1.0, 1.9] {
            let mut scroll = ZoomScroll::default();
            // Ten seconds of it.
            assert_eq!(
                pixel_burst(&mut scroll, delta, 600),
                0,
                "{delta} px/frame drifted into a zoom"
            );
        }
    }

    /// Stopping puts the bank back, so two half-gestures do not add up.
    #[test]
    fn the_bank_bleeds_away_when_scrolling_stops() {
        let mut scroll = ZoomScroll::default();
        assert_eq!(pixel_burst(&mut scroll, 12.0, 4), 0, "not yet a rung");
        for _ in 0..30 {
            assert_eq!(scroll.step(MouseScrollUnit::Pixel, 0.0, DT), None);
        }
        assert_eq!(scroll.pixels, 0.0, "half a gesture must not keep");
        assert_eq!(pixel_burst(&mut scroll, 12.0, 4), 0);
    }

    /// Changing your mind mid-gesture does not part-pay for the other direction.
    #[test]
    fn reversing_abandons_the_bank() {
        let mut scroll = ZoomScroll::default();
        assert_eq!(pixel_burst(&mut scroll, 20.0, 2), 0);
        assert!(scroll.pixels > 0.0);
        assert_eq!(scroll.step(MouseScrollUnit::Pixel, -20.0, DT), None);
        assert!(scroll.pixels < 0.0 && scroll.pixels > -PIXEL_STEP);
    }

    /// Zoom answers the hand, so the gate has to keep ticking on still frames.
    #[test]
    fn the_gate_opens_again_on_its_own() {
        let mut scroll = ZoomScroll::default();
        assert_eq!(scroll.step(MouseScrollUnit::Line, 1.0, DT), Some(1));
        assert!(scroll.cooldown > 0.0);
        for _ in 0..20 {
            scroll.step(MouseScrollUnit::Line, 0.0, DT);
        }
        assert_eq!(scroll.cooldown, 0.0);
    }

    /// A step is a step: pixel scroll of either sign, once it is paid for.
    #[test]
    fn banked_pixels_step_in_the_direction_they_were_scrolled() {
        let mut up = ZoomScroll::default();
        let mut down = ZoomScroll::default();
        let mut up_steps = None;
        let mut down_steps = None;
        for _ in 0..10 {
            up_steps = up_steps.or(up.step(MouseScrollUnit::Pixel, 20.0, DT));
            down_steps = down_steps.or(down.step(MouseScrollUnit::Pixel, -20.0, DT));
        }
        assert_eq!(up_steps, Some(1));
        assert_eq!(down_steps, Some(-1));
    }

    /// The numbers have to hang together or the model does not hold.
    #[test]
    fn the_thresholds_are_self_consistent() {
        // A gesture slower than the bleed can never bank anything.
        assert!(PIXEL_DECAY_PER_SEC * DT < PIXEL_STEP);
        // The gate is shorter than the time a rung takes to earn at drift speed,
        // so it gates flicks rather than becoming the real threshold.
        assert!(STEP_COOLDOWN_SECS > 0.0 && STEP_COOLDOWN_SECS < 1.0);
        assert!(PIXEL_STEP > 0.0);
    }

    #[test]
    fn no_fractional_or_out_of_range_scales() {
        for &f in &ZOOM_FACTORS {
            let s = ortho_scale_for_zoom(f);
            // Must be 1/n for integer n in {1,2,3} — never 1.5 or 4×.
            assert!((s * f32::from(f) - 1.0).abs() < f32::EPSILON);
            assert!(s <= 1.0 + f32::EPSILON);
            assert!(s >= 1.0 / 3.0 - f32::EPSILON);
        }
    }
}
