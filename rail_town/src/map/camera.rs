//! Orthographic camera pan (WASD / arrows) and zoom (scroll).
//!
//! Pan snaps to world pixels and zoom uses discrete scale steps so pixel art
//! does not shimmer under a sub-pixel camera (see `docs/RAILGEN_NOTES.md`).

use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use rail_map::{map_center_world, MapGrid};

const PAN_SPEED: f32 = 400.0;
/// Orthographic scales (smaller = more zoomed in). Integer-ish steps only.
const ZOOM_STEPS: [f32; 6] = [0.25, 0.5, 1.0, 1.5, 2.0, 4.0];
const DEFAULT_ZOOM_INDEX: usize = 2;

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
            scale: ZOOM_STEPS[DEFAULT_ZOOM_INDEX],
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
    mut q: Query<(&mut Projection, &mut CameraZoomIndex), With<MapCamera>>,
) {
    if scroll.delta.y == 0.0 {
        return;
    }

    let Ok((mut projection, mut zoom_index)) = q.single_mut() else {
        return;
    };

    let Projection::Orthographic(ortho) = projection.as_mut() else {
        return;
    };

    // Line scrolls usually step by ±1; pixel scrolls need a small threshold.
    let step = match scroll.unit {
        MouseScrollUnit::Line => {
            if scroll.delta.y > 0.0 {
                -1_isize
            } else {
                1
            }
        }
        MouseScrollUnit::Pixel => {
            if scroll.delta.y > 2.0 {
                -1
            } else if scroll.delta.y < -2.0 {
                1
            } else {
                return;
            }
        }
    };

    let next = (zoom_index.0 as isize + step).clamp(0, (ZOOM_STEPS.len() - 1) as isize) as usize;
    zoom_index.0 = next;
    ortho.scale = ZOOM_STEPS[next];
}
