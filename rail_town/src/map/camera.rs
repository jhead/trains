//! Orthographic camera pan (WASD / arrows) and integer zoom (wheel, `+` / `-`, `Z`).
//!
//! Zoom is **1× / 2× / 3×** only (screen pixels per world texel). Ortho scale is
//! `1 / zoom` so a 32px tile stays crisp. Pan snaps to world texels; zoom is
//! cursor-anchored when the pointer is over the window (brief 01 §§2.1, 4).

use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{map_center_world, MapGrid};

/// Allowed zoom multipliers (screen pixels per world texel). Nothing between or outside.
pub const ZOOM_FACTORS: [u8; 3] = [1, 2, 3];
/// Default zoom: 2× (brief 01 §2.1).
pub const DEFAULT_ZOOM_FACTOR: u8 = 2;
const DEFAULT_ZOOM_INDEX: usize = 1; // ZOOM_FACTORS[1] == 2
const PAN_SPEED: f32 = 400.0;

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

pub fn camera_pan(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut q: Query<&mut Transform, With<MapCamera>>,
) {
    let Ok(mut transform) = q.single_mut() else {
        return;
    };

    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        dir.y += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
        dir.y -= 1.0;
    }
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        dir.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        dir.x += 1.0;
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
    scroll: Res<AccumulatedMouseScroll>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
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
    let Ok((mut projection, mut transform, mut zoom_index, camera, cam_gt)) = q.single_mut()
    else {
        return;
    };

    let Projection::Orthographic(ortho) = projection.as_mut() else {
        return;
    };

    let mut step: Option<isize> = None;

    if scroll.delta.y != 0.0 {
        let scroll_step = match scroll.unit {
            MouseScrollUnit::Line => {
                if scroll.delta.y > 0.0 {
                    Some(1)
                } else {
                    Some(-1)
                }
            }
            MouseScrollUnit::Pixel => {
                if scroll.delta.y > 2.0 {
                    Some(1)
                } else if scroll.delta.y < -2.0 {
                    Some(-1)
                } else {
                    None
                }
            }
        };
        step = scroll_step;
    }

    if keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd) {
        step = Some(1);
    } else if keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract) {
        step = Some(-1);
    } else if keys.just_pressed(KeyCode::KeyZ) {
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
        assert_eq!(DEFAULT_ZOOM_FACTOR, 2);
        assert_eq!(zoom_factor_at(DEFAULT_ZOOM_INDEX), DEFAULT_ZOOM_FACTOR);
    }

    #[test]
    fn ortho_scale_maps_integer_zoom() {
        assert_eq!(ortho_scale_for_zoom(1), 1.0);
        assert_eq!(ortho_scale_for_zoom(2), 0.5);
        assert!((ortho_scale_for_zoom(3) - 1.0 / 3.0).abs() < f32::EPSILON);
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
