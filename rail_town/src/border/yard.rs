//! The Border Yard — a strip of world beyond the map edge.
//!
//! `docs/design/12-multiplayer.md` §3.2. You never see your neighbour's map. You
//! see their side of the connection: their track approaching, their trains
//! arriving and departing, the silhouette of their town on the horizon, a sign
//! with its name, and what they are offering.
//!
//! # Everything here is drawn from published data
//!
//! The only inputs are the fields of the cached
//! [`BorderManifest`](rail_sim::border::BorderManifest) — a name, two numbers,
//! twelve roof heights and a trading rhythm. There is no path from this module
//! to a neighbour's world, which is what sidesteps the privacy question
//! entirely: nothing appears in your yard that they did not publish for exactly
//! this purpose.
//!
//! # Everything here is outside the map
//!
//! Every sprite sits at [`Portal::yard_tile`]-style coordinates past the
//! boundary. Nothing in the yard can occupy a tile, block a route, or be
//! selected — constraint §2.2 as geometry rather than as a rule to remember.
//! Their trains run on their track and stop at the border; the only thing that
//! ever crosses onto your rails is your own stock coming home.
//!
//! At night their windows are lit, on the same [`TimeOfDay`] the town uses. §12.6
//! of the brief is a whole acceptance criterion about that one detail.

use bevy::prelude::*;
use rail_map::{tile_to_world, TILE_SIZE};
use rail_sim::border::{BorderEdge, BorderRegistry, LinkId};

use crate::atmosphere::TimeOfDay;
use crate::palette::{
    BALLAST_D, PLASTER_D, RAIL_D, RAIL_M, ROOF_SLATE_D, TIE_D, WIN_LIT, WOOD_M,
};

/// Tiles of their track drawn approaching the portal.
const YARD_TRACK_TILES: i32 = 7;
/// How far past the boundary their town sits, in tiles.
const TOWN_DISTANCE: f32 = 10.0;
/// How far past the boundary the sign stands.
const SIGN_DISTANCE: f32 = 2.6;

/// Iso prototype: the yard root sits above the depth band (see the spawn).
const YARD_ROOT_Z: f32 = 60.0;
const YARD_TRACK_Z: f32 = 1.0;
const YARD_TOWN_Z: f32 = 1.6;
const YARD_WINDOW_Z: f32 = 1.7;
const YARD_TRAIN_Z: f32 = 3.0;
const YARD_SIGN_Z: f32 = 3.2;

/// World units of roof per unit of published silhouette height.
const ROOF_UNIT: f32 = TILE_SIZE * 0.18;
/// Frontage each published roof occupies.
const ROOF_PITCH: f32 = TILE_SIZE * 0.62;

/// Root of one edge's yard. Rebuilt when the neighbour publishes.
#[derive(Component, Debug, Clone, Copy)]
pub struct BorderYard {
    pub edge: BorderEdge,
    pub link: LinkId,
    /// Manifest sequence this yard was built from.
    pub sequence: u64,
}

/// Their train, running their side of the line on their own rhythm.
#[derive(Component, Debug, Clone, Copy)]
pub struct YardTrain {
    pub edge: BorderEdge,
}

/// A lit window in the far town.
#[derive(Component, Debug, Clone, Copy)]
pub struct YardWindow;

/// Outward and lateral unit vectors for an edge, in world space.
fn axes(edge: BorderEdge) -> (Vec2, Vec2) {
    let (dx, dy) = edge.outward();
    let out = Vec2::new(dx as f32, dy as f32);
    // Lateral is the outward vector turned a quarter turn; both are axis
    // aligned, so nothing here ever rotates a sprite.
    (out, Vec2::new(-out.y, out.x))
}

/// Spawn, rebuild and retire the yards behind every open link.
pub fn sync_border_yards(
    mut commands: Commands,
    registry: Res<BorderRegistry>,
    existing: Query<(Entity, &BorderYard)>,
) {
    let mut keep: Vec<BorderEdge> = Vec::new();

    for link in registry.iter() {
        let current = existing.iter().find(|(_, yard)| yard.edge == link.edge);
        let sequence = link.neighbour.sequence;
        match current {
            Some((entity, yard)) if yard.link == link.link && yard.sequence == sequence => {
                keep.push(link.edge);
                let _ = entity;
                continue;
            }
            Some((entity, _)) => commands.entity(entity).despawn(),
            None => {}
        }
        keep.push(link.edge);

        let (out, lat) = axes(link.edge);
        let (px, py) = tile_to_world(link.portal_tile);
        let origin = Vec2::new(px, py);
        let roofs = link.neighbour.presence.silhouette.roofs.clone();
        let name = link.town_name().to_string();
        let offering = format!("{} - {}", name, link.their_offer().good.label());

        commands
            .spawn((
                BorderYard {
                    edge: link.edge,
                    link: link.link,
                    sequence,
                },
                // Iso prototype: the yard has no sprite of its own, so
                // `map::iso_sort` never adopts it and its children keep this
                // root z. At 0 the whole yard sat under the depth band and was
                // buried by terrain. It is drawn beyond the map's edge with
                // nothing between it and the camera, so it goes above the band.
                //
                // Its *layout* is still top-down — the track stubs, town
                // silhouette and sign are laid out along world axes with
                // axis-aligned rectangles, and they do not follow the diamond
                // grid. It only appears with a neighbour link, which single
                // player (`NeighborService::null()`) never has, so it is left
                // as-is rather than redesigned for a projection nobody has
                // accepted yet.
                Transform::from_xyz(origin.x, origin.y, YARD_ROOT_Z),
                Visibility::default(),
                Name::new(format!("Border yard - {}", link.edge.label())),
            ))
            .with_children(|yard| {
                // ── Their track, running away from the portal ──
                for step in 1..=YARD_TRACK_TILES {
                    let p = out * (step as f32 * TILE_SIZE);
                    let along = TILE_SIZE * 0.9;
                    let across = TILE_SIZE * 0.26;
                    let size = if out.x.abs() > 0.0 {
                        Vec2::new(along, across)
                    } else {
                        Vec2::new(across, along)
                    };
                    // Fades toward the horizon so the yard reads as distance.
                    let tint = if step % 2 == 0 { TIE_D } else { RAIL_D };
                    yard.spawn((
                        Sprite::from_color(tint, size),
                        Transform::from_xyz(p.x, p.y, YARD_TRACK_Z),
                    ));
                }

                // ── Their town on the horizon ──
                let base = out * (TOWN_DISTANCE * TILE_SIZE);
                let span = roofs.len() as f32;
                for (i, roof) in roofs.iter().enumerate() {
                    if *roof == 0 {
                        continue;
                    }
                    let offset = (i as f32 - (span - 1.0) * 0.5) * ROOF_PITCH;
                    let height = *roof as f32 * ROOF_UNIT;
                    let p = base + lat * offset;
                    let wall = Vec2::new(TILE_SIZE * 0.5, height);
                    yard.spawn((
                        Sprite::from_color(PLASTER_D, wall),
                        Transform::from_xyz(p.x, p.y + height * 0.5, YARD_TOWN_Z),
                    ));
                    yard.spawn((
                        Sprite::from_color(
                            ROOF_SLATE_D,
                            Vec2::new(TILE_SIZE * 0.62, TILE_SIZE * 0.16),
                        ),
                        Transform::from_xyz(p.x, p.y + height + TILE_SIZE * 0.08, YARD_TOWN_Z),
                    ));
                    // One window per building, dark by day and lit at night.
                    yard.spawn((
                        YardWindow,
                        Sprite::from_color(
                            WIN_LIT.with_alpha(0.0),
                            Vec2::splat(TILE_SIZE * 0.16),
                        ),
                        Transform::from_xyz(p.x, p.y + height * 0.55, YARD_WINDOW_Z),
                    ));
                }

                // ── The sign ──
                let sign = out * (SIGN_DISTANCE * TILE_SIZE) + lat * (TILE_SIZE * 1.2);
                yard.spawn((
                    Sprite::from_color(WOOD_M, Vec2::new(TILE_SIZE * 3.0, TILE_SIZE * 0.7)),
                    Transform::from_xyz(sign.x, sign.y, YARD_SIGN_Z),
                ));
                yard.spawn((
                    Sprite::from_color(BALLAST_D, Vec2::new(TILE_SIZE * 0.12, TILE_SIZE * 0.6)),
                    Transform::from_xyz(sign.x, sign.y - TILE_SIZE * 0.6, YARD_SIGN_Z - 0.1),
                ));
                yard.spawn((
                    Text2d::new(offering.clone()),
                    TextFont::from_font_size(12.0),
                    TextColor(RAIL_M),
                    Transform::from_xyz(sign.x, sign.y, YARD_SIGN_Z + 0.1),
                ));

                // ── Their train ──
                let along = TILE_SIZE * 0.6;
                let across = TILE_SIZE * 0.24;
                let size = if out.x.abs() > 0.0 {
                    Vec2::new(along, across)
                } else {
                    Vec2::new(across, along)
                };
                yard.spawn((
                    YardTrain { edge: link.edge },
                    Sprite::from_color(RAIL_M, size),
                    Transform::from_xyz(0.0, 0.0, YARD_TRAIN_Z),
                ));
            });
    }

    // A severed link takes its yard with it: the boundary goes back to looking
    // exactly as it does in solo play.
    for (entity, yard) in existing.iter() {
        if !keep.contains(&yard.edge) {
            commands.entity(entity).despawn();
        }
    }
}

/// Run their train up and down their side of the line on their own rhythm.
///
/// Purely presentational: the phase comes from the cached offer's period and
/// touches nothing in the simulation. Their train reaches the border and turns
/// back — it never enters your network, because nothing of theirs ever can.
pub fn animate_yard_trains(
    registry: Res<BorderRegistry>,
    mut trains: Query<(&YardTrain, &mut Transform)>,
) {
    for (train, mut transform) in &mut trains {
        let Some(link) = registry.get(train.edge) else {
            continue;
        };
        let period = link.their_offer().period_ticks.max(1) as f32;
        let phase = link.their_phase as f32 / period;
        // Out to the far end and back, so it reads as a service rather than a
        // conveyor: 0 → 1 → 0 across the period.
        let travel = 1.0 - (phase * 2.0 - 1.0).abs();
        let depth = TILE_SIZE * (0.6 + travel * (YARD_TRACK_TILES as f32 - 0.6));
        let (out, _) = axes(train.edge);
        transform.translation.x = out.x * depth;
        transform.translation.y = out.y * depth;
    }
}

/// Their lights come on with yours.
pub fn light_yard_windows(
    time_of_day: Option<Res<TimeOfDay>>,
    mut windows: Query<&mut Sprite, With<YardWindow>>,
) {
    let lit = time_of_day.map(|t| t.window_lit).unwrap_or(0.0);
    if windows.is_empty() {
        return;
    }
    for mut sprite in &mut windows {
        sprite.color = WIN_LIT.with_alpha(lit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_are_perpendicular_and_axis_aligned() {
        for edge in BorderEdge::ALL {
            let (out, lat) = axes(edge);
            assert!((out.length() - 1.0).abs() < f32::EPSILON);
            assert!((lat.length() - 1.0).abs() < f32::EPSILON);
            assert!(out.dot(lat).abs() < f32::EPSILON, "no diagonal yards");
            // Axis aligned: exactly one component is non-zero on each.
            assert_eq!((out.x != 0.0) as u8 + (out.y != 0.0) as u8, 1);
            assert_eq!((lat.x != 0.0) as u8 + (lat.y != 0.0) as u8, 1);
        }
    }

    #[test]
    fn the_yard_never_reaches_back_onto_the_map() {
        // Everything is placed at a positive multiple of the outward vector, so
        // no sprite in the yard can land on or behind the boundary tile.
        for edge in BorderEdge::ALL {
            let (out, _) = axes(edge);
            for step in 1..=YARD_TRACK_TILES {
                let p = out * (step as f32 * TILE_SIZE);
                assert!(p.length() >= TILE_SIZE);
            }
            assert!(out.length() * TOWN_DISTANCE * TILE_SIZE > 0.0);
        }
    }
}
