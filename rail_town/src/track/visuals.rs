//! Placeholder track sprites; bake-on-edit via [`TrackEdit`] messages.
//!
//! Discrete placeholder bars (not continuous Bresenham). Orientation uses the
//! first linked 8-dir when present so straight runs read clearly.
//!
//! Railhead polish (`docs/design/01-art-direction.md` §5.3): track a train has
//! just crossed brightens toward `railS` and decays over about four seconds, so
//! a busy main line visibly gleams and a branch nobody runs goes dull. That is
//! the network's usage written into the world art with no overlay and no
//! numbers — and it is the cheapest half of "congestion must be visible"
//! (`07-trains-and-lines.md` §4.1).

use std::collections::HashMap;

use bevy::prelude::*;
use rail_map::{tile_to_world, TILE_SIZE};
use rail_sim::track::DIR8;
use rail_sim::{TileOccupancy, TrackEdit, TrackId, TrackNetwork};

use crate::palette::RAIL_S;

/// Seconds a crossing takes to fade back to unpolished rail.
const POLISH_DECAY_SECS: f32 = 4.0;
/// How far toward `railS` a freshly crossed tile goes.
const POLISH_MAX: f32 = 0.9;

const GROUND_COLOR: Color = Color::srgb(0.22, 0.22, 0.24);
const BRIDGE_COLOR: Color = Color::srgb(0.55, 0.42, 0.28);

/// Marker on a track placeholder sprite.
#[derive(Component, Debug, Clone, Copy)]
pub struct TrackSprite {
    pub id: TrackId,
}

pub fn apply_track_sprites(
    mut commands: Commands,
    mut edits: MessageReader<TrackEdit>,
    network: Res<TrackNetwork>,
    existing: Query<(Entity, &TrackSprite)>,
) {
    for edit in edits.read() {
        match *edit {
            TrackEdit::Placed {
                id,
                tile,
                is_bridge,
                ..
            } => {
                let (wx, wy) = tile_to_world(tile);
                let color = base_color(is_bridge);
                let size = Vec2::new(TILE_SIZE * 0.7, TILE_SIZE * 0.2);
                let mut transform = Transform::from_xyz(wx, wy, 1.0);
                if let Some(piece) = network.piece(id) {
                    if let Some(dir) = (0..8).find(|&i| piece.links.has(i)) {
                        let (dx, dy) = DIR8[dir];
                        let angle = (dy as f32).atan2(dx as f32);
                        transform.rotation = Quat::from_rotation_z(angle);
                    }
                }
                commands.spawn((
                    Sprite::from_color(color, size),
                    transform,
                    TrackSprite { id },
                ));
            }
            TrackEdit::Removed { id, .. } => {
                for (entity, sprite) in existing.iter() {
                    if sprite.id == id {
                        commands.entity(entity).despawn();
                    }
                }
            }
            TrackEdit::Failed { .. } => {}
        }
    }
}

/// Brighten recently crossed track toward `railS`, fading over ~4 seconds.
///
/// The sim says *what* was crossed and when ([`TileOccupancy::last_crossed`]);
/// the fade is wall-clock and lives here, so the model stays fixed-step while
/// the gleam decays smoothly at any sim speed.
pub fn polish_railheads(
    time: Res<Time>,
    network: Res<TrackNetwork>,
    occupancy: Res<TileOccupancy>,
    mut sprites: Query<(&TrackSprite, &mut Sprite)>,
    mut heat: Local<HashMap<TrackId, f32>>,
    mut seen_tick: Local<u64>,
) {
    // Crossings recorded since the last frame we looked (FixedUpdate may have
    // run any number of times, or none).
    for (&id, &tick) in occupancy.last_crossed.iter() {
        if tick > *seen_tick {
            heat.insert(id, 1.0);
        }
    }
    *seen_tick = occupancy.tick;

    let fade = time.delta_secs() / POLISH_DECAY_SECS;
    heat.retain(|_, h| {
        *h -= fade;
        *h > 0.0
    });

    for (sprite_id, mut sprite) in sprites.iter_mut() {
        let base = base_color(
            network
                .piece(sprite_id.id)
                .is_some_and(|piece| piece.is_bridge()),
        );
        let gleam = heat.get(&sprite_id.id).copied().unwrap_or(0.0);
        let wanted = polished(base, gleam);
        if sprite.color != wanted {
            sprite.color = wanted;
        }
    }
}

fn base_color(is_bridge: bool) -> Color {
    if is_bridge {
        BRIDGE_COLOR
    } else {
        GROUND_COLOR
    }
}

/// Blend `base` toward the polished railhead by `gleam` in \[0, 1\].
fn polished(base: Color, gleam: f32) -> Color {
    let t = gleam.clamp(0.0, 1.0) * POLISH_MAX;
    if t <= 0.0 {
        return base;
    }
    let a = base.to_srgba();
    let b = RAIL_S.to_srgba();
    Color::srgb(
        a.red + (b.red - a.red) * t,
        a.green + (b.green - a.green) * t,
        a.blue + (b.blue - a.blue) * t,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channels(c: Color) -> (f32, f32, f32) {
        let s = c.to_srgba();
        (s.red, s.green, s.blue)
    }

    #[test]
    fn unused_track_keeps_its_base_colour() {
        assert_eq!(channels(polished(GROUND_COLOR, 0.0)), channels(GROUND_COLOR));
        assert_eq!(channels(polished(BRIDGE_COLOR, 0.0)), channels(BRIDGE_COLOR));
    }

    #[test]
    fn a_busy_line_gleams_brighter_than_a_quiet_one() {
        let quiet = channels(polished(GROUND_COLOR, 0.1));
        let busy = channels(polished(GROUND_COLOR, 1.0));
        assert!(busy.0 > quiet.0 && busy.1 > quiet.1 && busy.2 > quiet.2);
        // Full gleam approaches, but never overshoots, the polished railhead.
        let rail = channels(RAIL_S);
        assert!(busy.0 <= rail.0 && busy.1 <= rail.1 && busy.2 <= rail.2);
    }

    #[test]
    fn gleam_decays_monotonically() {
        let mut previous = 1.0f32;
        for step in 1..=4 {
            let gleam = 1.0 - step as f32 / POLISH_DECAY_SECS;
            let red = channels(polished(GROUND_COLOR, gleam)).0;
            assert!(red < previous);
            previous = red;
        }
    }
}
