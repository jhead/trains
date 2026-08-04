//! Live ghost sprites for proposed build / demolish tiles.
//!
//! The ghost is how the player reads which tile the cursor resolved to, so its
//! footprint has to be the shape of a tile — a square from above, a tinted
//! diamond in isometric, because a square over a diamond grid says nothing.
//! Each call site therefore names both: the fraction of a diamond it wants and
//! the exact top-down rectangle it always drew. The vocabulary is the same
//! either way (hover faint, anchor small and solid, place bright, invalid
//! `WARN`), and neither view is a compromise for the other.
//!
//! # A tile that would take track draws the track
//!
//! Brief 04 §2.2: *"It is the **actual** track art at 55% opacity tinted `hi`,
//! not an abstract line — the player sees what they will get, including how the
//! curve will resolve."*
//!
//! That contract earns its keep the moment the ground has gradient. Deciding
//! whether to climb is the most consequential thing the build tool asks, and a
//! player deciding it while looking at a flat bar has been given the cost and
//! denied the picture — so in isometric a placeable ghost draws the very cell
//! its piece will draw, ramps and all, out of the same bank keyed the same way
//! (brief 15 §5). Ghost and placed art are the same asset; they cannot drift.
//!
//! Top-down keeps the bar it has. From above there is no ramp to show, the bar
//! is what shipped, and brief 15 is the isometric brief.

use std::collections::HashSet;

use bevy::prelude::*;
use rail_map::{tile_to_world, TILE_SIZE};
use rail_sim::ids::TileCoord;
use rail_sim::{TrackNetwork, GROUND_LAYER};

use crate::map::TileMark;

use super::iso_incline::links_for_occupancy;
use super::preview::TileGhostKind;
use super::tools::{DragKind, TrackToolState};
use super::visuals::{ghost_cell, TrackArt};

const HI: Color = Color::srgb(0xf2 as f32 / 255.0, 0xc1 as f32 / 255.0, 0x4e as f32 / 255.0);
const WARN: Color = Color::srgb(0xe8 as f32 / 255.0, 0x62 as f32 / 255.0, 0x4a as f32 / 255.0);

#[derive(Component)]
pub(crate) struct GhostSprite;

#[derive(Component)]
pub(crate) struct HoverHighlight;

#[derive(Component)]
pub(crate) struct AnchorMarker;

#[allow(clippy::too_many_arguments)] // One job; the extra params are the bank.
pub fn sync_track_ghosts(
    mut commands: Commands,
    state: Res<TrackToolState>,
    mark: Option<Res<TileMark>>,
    network: Option<Res<TrackNetwork>>,
    art: Option<ResMut<TrackArt>>,
    images: Option<ResMut<Assets<Image>>>,
    ghosts: Query<Entity, With<GhostSprite>>,
    hovers: Query<Entity, With<HoverHighlight>>,
    anchors: Query<Entity, With<AnchorMarker>>,
) {
    for entity in &ghosts {
        commands.entity(entity).despawn();
    }
    for entity in &hovers {
        commands.entity(entity).despawn();
    }
    for entity in &anchors {
        commands.entity(entity).despawn();
    }

    // Headless tests boot without the terrain plugin, so there is no mask to
    // tint; a ghost is presentation only and simply does not draw.
    let Some(mark) = mark else {
        return;
    };

    if let Some(tile) = state.hover_tile {
        spawn_tile(
            &mut commands,
            &mark,
            tile,
            HI.with_alpha(0.22),
            0.98,
            Vec2::splat(TILE_SIZE * 0.98),
            2.0,
            HoverHighlight,
        );
    }

    if let Some(anchor) = state.anchor {
        spawn_tile(
            &mut commands,
            &mark,
            anchor,
            HI.with_alpha(0.55),
            0.35,
            Vec2::splat(TILE_SIZE * 0.35),
            2.2,
            AnchorMarker,
        );
    }

    if let Some(preview) = &state.build_preview {
        let whole_warn = preview.reject.is_some();
        // What the ghost's links are derived against: everything already built,
        // plus the rest of the route the player is dragging. A tile the route
        // cannot place is not counted — nothing will ever link to it.
        let proposed: HashSet<(i32, i32)> = preview
            .tiles
            .iter()
            .filter(|g| !matches!(g.kind, TileGhostKind::Invalid))
            .map(|g| (g.tile.x, g.tile.y))
            .collect();
        // Isometric only, and only when the bank and the network are both here
        // — headless tests boot without either.
        let mut real_art = match (network.as_deref(), art, images) {
            (Some(network), Some(art), Some(images)) if rail_map::projection_is_iso() => {
                Some((network, art, images))
            }
            _ => None,
        };

        for ghost in &preview.tiles {
            let (color, scale, flat) = match ghost.kind {
                TileGhostKind::Place { is_bridge, .. } => {
                    let base = if ghost.valid && !whole_warn {
                        HI.with_alpha(0.55)
                    } else {
                        WARN.with_alpha(0.55)
                    };
                    // The piece this tile is about to become, drawn as itself.
                    if let Some((network, art, images)) = real_art.as_mut() {
                        let links = links_for_occupancy(ghost.tile, |c| {
                            proposed.contains(&(c.x, c.y))
                                || network.id_at(c, GROUND_LAYER).is_some()
                        });
                        let image =
                            ghost_cell(art, images, ghost.tile, links, is_bridge);
                        let (wx, wy) = tile_to_world(ghost.tile);
                        commands.spawn((
                            GhostSprite,
                            Sprite {
                                image,
                                color: base,
                                ..default()
                            },
                            Transform::from_xyz(wx, wy, 2.5),
                        ));
                        continue;
                    }
                    if is_bridge {
                        (base, 0.75, Vec2::new(TILE_SIZE * 0.75, TILE_SIZE * 0.28))
                    } else {
                        (base, 0.6, RAIL_BAR)
                    }
                }
                TileGhostKind::Existing => (HI.with_alpha(0.2), 0.6, RAIL_BAR),
                TileGhostKind::Invalid => (
                    WARN.with_alpha(0.6),
                    0.85,
                    Vec2::splat(TILE_SIZE * 0.85),
                ),
            };
            spawn_tile(
                &mut commands,
                &mark,
                ghost.tile,
                color,
                scale,
                flat,
                2.5,
                GhostSprite,
            );
        }
    }

    if let Some(preview) = &state.demolish_preview {
        for &tile in &preview.tiles {
            spawn_tile(
                &mut commands,
                &mark,
                tile,
                WARN.with_alpha(0.5),
                0.6,
                RAIL_BAR,
                2.6,
                GhostSprite,
            );
        }
        // While demolish-dragging, also mark empty tip so the path reads.
        if state.drag == Some(DragKind::Demolish) {
            if let Some(tip) = state.hover_tile {
                if !preview.tiles.contains(&tip) {
                    spawn_tile(
                        &mut commands,
                        &mark,
                        tip,
                        WARN.with_alpha(0.25),
                        0.5,
                        Vec2::splat(TILE_SIZE * 0.5),
                        2.4,
                        GhostSprite,
                    );
                }
            }
        }
    }
}

/// The bar a track ghost draws from above: a rail's width along the tile.
const RAIL_BAR: Vec2 = Vec2::new(TILE_SIZE * 0.7, TILE_SIZE * 0.2);

/// `scale` is the fraction of a diamond the isometric mark covers; `flat` is
/// the literal size the top-down one is drawn at.
fn spawn_tile<M: Component>(
    commands: &mut Commands,
    mark: &TileMark,
    tile: TileCoord,
    color: Color,
    scale: f32,
    flat: Vec2,
    z: f32,
    marker: M,
) {
    let (wx, wy) = tile_to_world(tile);
    commands.spawn((
        marker,
        mark.sprite(color, scale, flat),
        Transform::from_xyz(wx, wy, z),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::preview::{BuildPreview, GhostTile};
    use rail_map::Projection;

    /// A world with the ghost system, a bank, and a tile mask to tint.
    fn app(projection: Projection) -> (App, crate::map::tests::ProjectionGuard) {
        let guard = crate::map::tests::ProjectionGuard::new(projection);
        let mut app = App::new();
        app.init_resource::<Assets<Image>>();
        app.init_resource::<TrackArt>();
        app.init_resource::<TrackNetwork>();
        app.init_resource::<TrackToolState>();
        let mask = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::default());
        app.insert_resource(TileMark(mask));
        app.add_systems(Update, sync_track_ghosts);
        (app, guard)
    }

    /// Two tiles, the second one a climb away from the first.
    fn preview_of(tiles: &[TileCoord]) -> BuildPreview {
        BuildPreview {
            tiles: tiles
                .iter()
                .map(|&tile| GhostTile {
                    tile,
                    kind: TileGhostKind::Place {
                        cost_cents: 10_000,
                        is_bridge: false,
                    },
                    valid: true,
                })
                .collect(),
            new_tile_count: tiles.len() as u32,
            bridge_count: 0,
            total_cost_cents: 10_000 * tiles.len() as i64,
            balance_after_cents: 1_000_000,
            can_commit: true,
            reject: None,
            endpoint: *tiles.last().unwrap(),
        }
    }

    fn ghost_images(app: &mut App) -> Vec<Option<AssetId<Image>>> {
        app.world_mut()
            .query_filtered::<&Sprite, With<GhostSprite>>()
            .iter(app.world())
            .map(|s| {
                let id = s.image.id();
                (id != AssetId::<Image>::default()).then_some(id)
            })
            .collect()
    }

    /// Brief 04 §2.2 and 15 §5: in isometric the ghost *is* the track art, and
    /// the two tiles of a climbing route draw two different cells — because one
    /// of them ramps up and the other ramps down.
    #[test]
    fn an_isometric_ghost_draws_real_track_art() {
        let (mut app, _guard) = app(Projection::Iso);
        let mut map = rail_map::MapGrid::empty(16, 16, 1);
        map.get_mut(TileCoord { x: 5, y: 4 }).unwrap().height = 1;
        rail_map::set_iso_heights(&map);

        let route = [TileCoord { x: 4, y: 4 }, TileCoord { x: 5, y: 4 }];
        app.world_mut()
            .resource_mut::<TrackToolState>()
            .build_preview = Some(preview_of(&route));
        app.update();

        let images = ghost_images(&mut app);
        assert_eq!(images.len(), 2, "both route tiles should have a ghost");
        let baked: Vec<AssetId<Image>> = images.iter().flatten().copied().collect();
        assert_eq!(baked.len(), 2, "an isometric ghost must carry baked art");
        assert_ne!(
            baked[0], baked[1],
            "one tile climbs and the other descends: two different cells"
        );

        // And it came out of the shared bank, baked once each.
        assert_eq!(app.world().resource::<TrackArt>().baked(), 2);
        rail_map::clear_iso_heights();
    }

    /// The shipping view is untouched: still the bar it always drew, and the
    /// bank is never asked for anything.
    #[test]
    fn a_top_down_ghost_is_still_the_bar_it_always_was() {
        let (mut app, _guard) = app(Projection::TopDown);
        let route = [TileCoord { x: 4, y: 4 }, TileCoord { x: 5, y: 4 }];
        app.world_mut()
            .resource_mut::<TrackToolState>()
            .build_preview = Some(preview_of(&route));
        app.update();

        let sprites: Vec<(Option<Vec2>, Color)> = app
            .world_mut()
            .query_filtered::<&Sprite, With<GhostSprite>>()
            .iter(app.world())
            .map(|s| (s.custom_size, s.color))
            .collect();
        assert_eq!(sprites.len(), 2);
        for (size, color) in sprites {
            assert_eq!(size, Some(RAIL_BAR), "the top-down ghost changed shape");
            assert_eq!(color, HI.with_alpha(0.55));
        }
        assert_eq!(
            app.world().resource::<TrackArt>().baked(),
            0,
            "the flat view must not bake a cell to draw a ghost"
        );
    }
}
