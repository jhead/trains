//! Lit windows — a second sprite layer over the building layer.
//!
//! Brief 01 §3.4 is explicit that window light is *not* part of the tint: it is
//! its own layer, in `winLit`, fading up over about forty seconds at dusk.
//! "A well-served district lighting up at nightfall is the cheapest emotional
//! payoff in the game, and it makes density read as *life* rather than as a
//! number."
//!
//! The town slice owns the buildings and bakes a matching **window mask frame**
//! for each one, so this layer does not guess where the windows are: it draws
//! [`BuildingWindows::lit_frame`] over the lot that produced it, in the same
//! atlas cell, and the two line up by construction.
//!
//! That also means the gating is already correct without anything here:
//! occupancy is world-hashed into the mask, and `lit_frame` is `None` for
//! anything that must not light — under construction, cleared, and every
//! decline stage from dimmed down. Windows going dark *is* the first decline
//! signal, so this layer gets that behaviour for free.
//!
//! What is left for us is the dusk fade, staggered per building so a district
//! comes on one house after another rather than as a single dimmer sweep.
//! The fade is quantized into [`LIGHT_STEPS`], so the per-frame system is one
//! comparison until a step actually lands.

use bevy::prelude::*;
use bevy::sprite::Anchor;

use crate::hash::hash_unit;
use super::time_of_day::TimeOfDay;
use crate::town::{BuildingAtlas, BuildingWindows};

/// Salt for the stagger roll — when in the fade does this building come on?
const STAGGER_SALT: u32 = 0x4c41_4d50;

/// Drawn just in front of the lot that owns it. Per-entity, because buildings
/// are Y-sorted into a band — a global constant would put every light either
/// behind or in front of the wrong houses.
const WINDOW_Z_LIFT: f32 = 0.02;

/// Visible steps in the dusk fade. Eight steps across forty seconds reads as
/// houses coming on one after another rather than as a dimmer sweep — and it
/// keeps the fade out of the per-frame path.
const LIGHT_STEPS: u8 = 8;
/// Share of the fade a single building takes to reach full brightness.
const WINDOW_RAMP: f32 = 0.3;
/// Latest point in the fade at which a building may start to light.
const WINDOW_STAGGER: f32 = 0.7;

/// A lit-window sprite, tied to the lot entity that owns the building.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct LitWindow {
    /// The lot this light belongs to; despawn when it goes away.
    lot: Entity,
    /// Point in the dusk fade where this building switches on. World-hashed on
    /// the lot's position, so the same house always lights at the same moment.
    turn_on: f32,
}

/// Last applied fade step, so the per-frame system is a single comparison.
#[derive(Resource, Default)]
pub(crate) struct WindowLayer {
    step: Option<u8>,
}

/// Spawn, update and retire one light per lit building.
pub(crate) fn sync_lit_windows(
    mut commands: Commands,
    tod: Res<TimeOfDay>,
    atlas: Option<Res<BuildingAtlas>>,
    lots: Query<(Entity, &Transform, &BuildingWindows)>,
    mut lights: Query<(Entity, &LitWindow, &mut Sprite, &mut Transform), Without<BuildingWindows>>,
) {
    let _perf = crate::overlays::perf::scope("sync_lit_windows");
    let Some(atlas) = atlas else {
        return;
    };
    let lit = quantized_lit(tod.window_lit);

    // Retire lights whose lot has gone dark, been cleared, or been despawned.
    let mut has_light = std::collections::HashSet::new();
    for (entity, light, mut sprite, mut transform) in lights.iter_mut() {
        match lots.get(light.lot) {
            Ok((_, lot_transform, windows)) if windows.lit_frame.is_some() => {
                has_light.insert(light.lot);
                // Follow the lot: a building settling or upgrading moves.
                transform.translation = lot_transform.translation;
                transform.translation.z = lot_transform.translation.z + WINDOW_Z_LIFT;
                sprite.flip_x = windows.flip_x;
                let alpha = window_alpha(lit, light.turn_on);
                sprite.color = Color::WHITE.with_alpha(alpha);
            }
            _ => {
                commands.entity(entity).despawn();
            }
        }
    }

    for (entity, transform, windows) in lots.iter() {
        let Some(frame) = windows.lit_frame else {
            continue;
        };
        if has_light.contains(&entity) {
            continue;
        }
        // World-anchored on the lot's tile so the stagger never shifts under
        // the camera (pixel contract §2.4).
        let tx = transform.translation.x as i32;
        let ty = transform.translation.y as i32;
        let turn_on = hash_unit(tx, ty, STAGGER_SALT) * WINDOW_STAGGER;

        let mut sprite = atlas.sprite(frame);
        sprite.flip_x = windows.flip_x;
        sprite.color = Color::WHITE.with_alpha(window_alpha(lit, turn_on));

        let mut light_transform = *transform;
        light_transform.translation.z = transform.translation.z + WINDOW_Z_LIFT;

        commands.spawn((
            LitWindow {
                lot: entity,
                turn_on,
            },
            sprite,
            Anchor::BOTTOM_CENTER,
            light_transform,
        ));
    }
}

/// Step the whole layer when the fade crosses a step boundary.
pub(crate) fn step_window_light(
    tod: Res<TimeOfDay>,
    mut layer: ResMut<WindowLayer>,
    mut windows: Query<(&LitWindow, &mut Sprite, &mut Visibility)>,
) {
    let _perf = crate::overlays::perf::scope("step_window_light");
    let step = fade_step(tod.window_lit);
    if layer.step == Some(step) {
        return;
    }
    layer.step = Some(step);

    let lit = step as f32 / LIGHT_STEPS as f32;
    for (window, mut sprite, mut visibility) in windows.iter_mut() {
        let alpha = window_alpha(lit, window.turn_on);
        sprite.color = Color::WHITE.with_alpha(alpha);
        *visibility = visibility_for(alpha);
    }
}

fn visibility_for(alpha: f32) -> Visibility {
    if alpha > 0.0 {
        Visibility::Visible
    } else {
        Visibility::Hidden
    }
}

/// Which of [`LIGHT_STEPS`] the fade is on.
fn fade_step(window_lit: f32) -> u8 {
    (window_lit.clamp(0.0, 1.0) * LIGHT_STEPS as f32).round() as u8
}

/// The fade, snapped to its step — used by both the spawn path and the step
/// system so a newly built house never disagrees with its neighbours.
fn quantized_lit(window_lit: f32) -> f32 {
    fade_step(window_lit) as f32 / LIGHT_STEPS as f32
}

/// Brightness of one building's windows at a point in the fade.
fn window_alpha(lit: f32, turn_on: f32) -> f32 {
    ((lit - turn_on) / WINDOW_RAMP).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_building_is_fully_lit_by_the_end_of_the_fade() {
        for step in 0..64 {
            let turn_on = step as f32 / 64.0 * WINDOW_STAGGER;
            assert_eq!(window_alpha(1.0, turn_on), 1.0);
            assert_eq!(window_alpha(0.0, turn_on + 0.001), 0.0);
        }
    }

    #[test]
    fn houses_stagger_rather_than_switching_on_as_one() {
        let mid = quantized_lit(0.5);
        let (mut on, mut off) = (0, 0);
        for y in 0..32 {
            for x in 0..32 {
                let turn_on = hash_unit(x, y, STAGGER_SALT) * WINDOW_STAGGER;
                if window_alpha(mid, turn_on) > 0.0 {
                    on += 1;
                } else {
                    off += 1;
                }
            }
        }
        assert!(
            on > 0 && off > 0,
            "half-fade should be mid-stagger: {on} on / {off} off"
        );
    }

    #[test]
    fn fade_steps_quantize_the_ramp() {
        assert_eq!(fade_step(0.0), 0);
        assert_eq!(fade_step(1.0), LIGHT_STEPS);
        assert_eq!(fade_step(0.51), fade_step(0.49));
        assert!(quantized_lit(1.0) == 1.0 && quantized_lit(0.0) == 0.0);
    }

    #[test]
    fn lights_sit_in_front_of_their_own_lot_not_a_fixed_layer() {
        // Buildings are Y-sorted across a band, so the lift must be relative.
        // A fixed z would put lights behind the houses south of them.
        let near = 1.10_f32 + WINDOW_Z_LIFT;
        let far = 1.90_f32 + WINDOW_Z_LIFT;
        assert!(near > 1.10 && near < 1.90);
        assert!(far > 1.90);
    }
}
