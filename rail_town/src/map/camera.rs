//! Orthographic camera pan (WASD / arrows) and zoom (scroll).

use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use rail_map::{map_center_world, MapGrid};

const PAN_SPEED: f32 = 400.0;
const ZOOM_MIN: f32 = 0.25;
const ZOOM_MAX: f32 = 4.0;
const ZOOM_LINE_FACTOR: f32 = 0.1;
const ZOOM_PIXEL_FACTOR: f32 = 0.001;

#[derive(Component)]
pub struct MapCamera;

pub fn setup_map_camera(mut commands: Commands, map: Res<MapGrid>) {
    let (cx, cy) = map_center_world(map.width, map.height);
    commands.spawn((
        Camera2d,
        MapCamera,
        Transform::from_xyz(cx, cy, 1000.0),
        Projection::Orthographic(OrthographicProjection {
            scale: 1.0,
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
}

pub fn camera_zoom(
    scroll: Res<AccumulatedMouseScroll>,
    mut q: Query<&mut Projection, With<MapCamera>>,
) {
    if scroll.delta.y == 0.0 {
        return;
    }

    let Ok(mut projection) = q.single_mut() else {
        return;
    };

    let Projection::Orthographic(ortho) = projection.as_mut() else {
        return;
    };

    let factor = match scroll.unit {
        MouseScrollUnit::Line => ZOOM_LINE_FACTOR,
        MouseScrollUnit::Pixel => ZOOM_PIXEL_FACTOR,
    };
    // Scroll up (positive y) → zoom in (smaller scale).
    ortho.scale *= 1.0 - scroll.delta.y * factor;
    ortho.scale = ortho.scale.clamp(ZOOM_MIN, ZOOM_MAX);
}
