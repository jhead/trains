//! The Map View's schematic render — a second, purpose-built drawing of the
//! world, baked to one texture on edit.
//!
//! # Why this is not the world, scaled down
//!
//! Brief 02 §6 is explicit: *"It is not a zoomed-out camera. It is a second,
//! purpose-built rendering that answers different questions."* Once terrain was
//! textured, pointing the play camera at four texels per tile meant
//! nearest-sampling a 32-texel autotiled cell down to a 4-texel square — one
//! texel in sixty-four survives, and which one changes as the camera moves. That
//! is the `downsample` plate's failure mode arriving by the back door, in the one
//! view whose whole job is legibility.
//!
//! So the Map View draws its own vocabulary at its own resolution, one texel to
//! one screen pixel (brief 01 §2.1):
//!
//! | Element | How it is drawn |
//! | --- | --- |
//! | Terrain | flat material colour per tile, straight off the [`crate::palette`] ramps |
//! | Elevation | banded — a 1-texel shadow lip wherever a tile steps down to its south or east neighbour |
//! | Coast | a 1-texel `waterF` lip on every water tile touching land, so rivers and shorelines are *lines* |
//! | Impassable rock | a world-hashed `rockL` tick, so the walls read as walls |
//! | Track | clean 1–2 texel strokes from tile centre along each linked direction |
//! | Lines | those strokes take the line's colour where a line runs over them |
//! | Stations | icons, sized by tier, ringed in `outline` so they separate from the ground |
//! | Unserved demand | the same icons in `hi` — the one thing on the map the player is being asked to notice |
//! | Trains | live dots, the only thing here that moves (brief 05 §6) |
//!
//! No world texture is sampled, nothing is filtered, and nothing rotates.
//!
//! `hi` appears here and nowhere in the world art. Brief 01 §3.1 reserves the
//! diagnostic accents for "UI, overlays, ghosts and selection" — the plate is a
//! schematic overlay on the same footing as the §5 overlays it hosts, and
//! marking unserved demand is precisely the job brief 02 §6 gives it.
//!
//! # When it bakes
//!
//! Brief 01 §2.5: art is baked when data changes, never per frame. Terrain never
//! changes after generation, track and stations change on an edit, and lines
//! change when the player authors one — so the bake is driven by
//! [`TrackEdit`] / [`StationEdit`] messages plus the change ticks of the map,
//! station, industry and line resources.
//!
//! Those are the *gate*. The **truth** is [`signature`], exactly as in
//! [`super::terrain::chunk`]: a resource can be written for a reason that changes
//! no texel, and a re-bake plus a texture upload is far too expensive to spend on
//! a false positive. Nothing is baked at all until the player first opens the
//! view.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rail_map::{top_down_map_center, MapGrid, TerrainKind, TILE_SIZE};
use rail_sim::ids::TileCoord;
use rail_sim::track::DIR16;
use rail_sim::{
    line_path, DemandSpawner, IndustryRegistry, LineRegistry, StationEdit, StationRegistry,
    StationTier, TrackEdit, TrackId, TrackNetwork, MOUNTAIN_HEIGHT_MIN,
};

use super::map_view::MapViewState;
use super::terrain::material::{
    elevation_band, material_of, rgba, shade_for, surface_height, terrain_color,
};
use crate::hash::world_hash;
use crate::palette::{BG0, HI, OUTLINE, PLASTER_M, RAIL_L, RAIL_S, ROCK_L, WATER_F};
use crate::trains::TrainSprite;

/// Texels per tile in the baked schematic.
///
/// One texel is one screen pixel at the Map View's ortho scale — the whole point
/// of baking at this size rather than sampling the world down to it.
pub const TEXELS_PER_TILE: u32 = 4;

/// Above the day tint (`atmosphere::DAY_TINT_Z`), because a strategic read has no
/// time of day, and above every world band so the world is simply not visible.
const SCHEMATIC_Z: f32 = 200.0;
/// The `bg0` ground the schematic sits on — brief 01 §3.1 names `bg0`
/// "map-view ground" in as many words. Occludes the world and its tint.
const BACKDROP_Z: f32 = SCHEMATIC_Z - 1.0;
/// Live train dots, over the baked plate.
const DOT_Z: f32 = SCHEMATIC_Z + 1.0;

/// Where a map-wide overlay draws while the Map View is open.
///
/// Brief 05 §6: *"All the overlays from §5 render here too, and at map scale
/// they're often more useful."* Overlay tiles are flat colour on a tile-sized
/// quad, so they are already schematic — they just have to be above the plate
/// instead of under it.
pub const SCHEMATIC_OVERLAY_Z: f32 = SCHEMATIC_Z + 0.5;

/// Extra world units of backdrop beyond the viewport on every side, so a camera
/// that pans after this system in the same frame cannot show a seam.
const BACKDROP_MARGIN: f32 = 64.0;

/// Track stroke half-width in texels: a 2-texel line at 4 texels to the tile.
const STROKE_HALF: f32 = 0.5;
/// Sub-texel step when walking a stroke, so a diagonal leaves no holes.
const WALK_STEP: f32 = 0.25;

/// A live train dot, in world units. Two screen pixels at Map View scale.
const DOT_SIZE: f32 = TILE_SIZE * 0.5;

const ROCK_SALT: u32 = 0x3C71_9A05;

// ── The plate ──────────────────────────────────────────────────────────────

/// The baked plate, and what it was baked from.
#[derive(Resource, Debug)]
pub struct SchematicState {
    /// A trigger has fired since the last bake. The gate, not the truth.
    dirty: bool,
    /// Signature of the world the plate was last painted from.
    signature: Option<u64>,
    /// Bakes performed this session — the number a test watches.
    bakes: u32,
}

impl Default for SchematicState {
    fn default() -> Self {
        Self {
            // Nothing has been painted yet, so the first open has to paint.
            dirty: true,
            signature: None,
            bakes: 0,
        }
    }
}

impl SchematicState {
    #[cfg(test)]
    pub fn bakes(&self) -> u32 {
        self.bakes
    }
}

/// The baked whole-map plate.
#[derive(Component)]
pub struct SchematicPlate {
    /// Tile dimensions the current image was sized for.
    tiles: (u32, u32),
}

/// The `bg0` ground under the plate, chasing the camera.
#[derive(Component)]
pub struct SchematicBackdrop;

/// One live train dot.
#[derive(Component)]
pub struct SchematicTrainDot(pub rail_sim::TrainId);

// ── Painting ───────────────────────────────────────────────────────────────

/// Everything the plate is painted from, borrowed for one bake.
struct World<'a> {
    map: &'a MapGrid,
    network: &'a TrackNetwork,
    stations: &'a StationRegistry,
    industries: &'a IndustryRegistry,
    lines: &'a LineRegistry,
    demand: &'a DemandSpawner,
}

/// A plate under construction. Texel `(0, 0)` is the map's **south-west** corner
/// and y runs north, which is how every other coordinate in the game reads; the
/// flip into image rows happens once, in [`Canvas::into_image`].
struct Canvas {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

impl Canvas {
    fn new(tiles_w: u32, tiles_h: u32) -> Self {
        let w = tiles_w * TEXELS_PER_TILE;
        let h = tiles_h * TEXELS_PER_TILE;
        Self {
            w,
            h,
            px: vec![0u8; (w * h) as usize * 4],
        }
    }

    #[inline]
    fn put(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        // Row 0 of the image is the map's north edge.
        let row = self.h - 1 - y as u32;
        let o = ((row * self.w + x as u32) * 4) as usize;
        self.px[o..o + 4].copy_from_slice(&color);
    }

    #[cfg(test)]
    fn at(&self, x: i32, y: i32) -> [u8; 4] {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return [0; 4];
        }
        let row = self.h - 1 - y as u32;
        let o = ((row * self.w + x as u32) * 4) as usize;
        [self.px[o], self.px[o + 1], self.px[o + 2], self.px[o + 3]]
    }

    fn into_image(self) -> Image {
        let mut image = Image::new(
            Extent3d {
                width: self.w,
                height: self.h,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            self.px,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        // One texel is one screen pixel — never filtered, never mipmapped.
        image.sampler = ImageSampler::nearest();
        image
    }
}

/// South-west texel of a tile.
#[inline]
fn tile_origin(tile: TileCoord) -> (i32, i32) {
    (
        tile.x * TEXELS_PER_TILE as i32,
        tile.y * TEXELS_PER_TILE as i32,
    )
}

/// Tile centre in continuous texel coordinates.
#[inline]
fn tile_center(tile: TileCoord) -> Vec2 {
    let (ox, oy) = tile_origin(tile);
    let half = TEXELS_PER_TILE as f32 * 0.5;
    Vec2::new(ox as f32 + half, oy as f32 + half)
}

/// Paint the whole plate, bottom band up.
fn paint(world: &World) -> Canvas {
    let mut canvas = Canvas::new(world.map.width, world.map.height);
    paint_terrain(&mut canvas, world.map);
    paint_track(&mut canvas, world);
    paint_stops(&mut canvas, world);
    canvas
}

/// Flat material colour per tile, an elevation lip where the ground steps down,
/// a coast line, and a tick on rock the player cannot build on.
fn paint_terrain(canvas: &mut Canvas, map: &MapGrid) {
    let tpt = TEXELS_PER_TILE as i32;
    for ty in 0..map.height as i32 {
        for tx in 0..map.width as i32 {
            let coord = TileCoord { x: tx, y: ty };
            let Some(tile) = map.get(coord) else {
                continue;
            };
            let (ox, oy) = tile_origin(coord);
            let fill = rgba(terrain_color(tile.kind, tile.height));
            for dy in 0..tpt {
                for dx in 0..tpt {
                    canvas.put(ox + dx, oy + dy, fill);
                }
            }

            // Elevation as a band, not a gradient (brief 02 §2.3): where this
            // tile stands a band above the ground south or east of it, the step
            // draws as a shadow lip. Contours fall out of that, and a ridge
            // reads as a ridge from directly above.
            let material = material_of(tile.kind);
            let shadow = rgba(material.shadow(shade_for(tile.kind, tile.height)));
            let band = elevation_band(surface_height(tile.height, tile.water));
            let lower = |dx: i32, dy: i32| {
                map.get(TileCoord {
                    x: tx + dx,
                    y: ty + dy,
                })
                .is_some_and(|n| elevation_band(surface_height(n.height, n.water)) < band)
            };
            if lower(0, -1) {
                for dx in 0..tpt {
                    canvas.put(ox + dx, oy, shadow);
                }
            }
            if lower(1, 0) {
                for dy in 0..tpt {
                    canvas.put(ox + tpt - 1, oy + dy, shadow);
                }
            }

            // A coastline is a line, with a lip — not a colour change (§6.2.1).
            if tile.water {
                let foam = rgba(WATER_F);
                let land = |dx: i32, dy: i32| {
                    map.get(TileCoord {
                        x: tx + dx,
                        y: ty + dy,
                    })
                    .is_some_and(|n| !n.water)
                };
                if land(0, 1) {
                    for dx in 0..tpt {
                        canvas.put(ox + dx, oy + tpt - 1, foam);
                    }
                }
                if land(0, -1) {
                    for dx in 0..tpt {
                        canvas.put(ox + dx, oy, foam);
                    }
                }
                if land(-1, 0) {
                    for dy in 0..tpt {
                        canvas.put(ox, oy + dy, foam);
                    }
                }
                if land(1, 0) {
                    for dy in 0..tpt {
                        canvas.put(ox + tpt - 1, oy + dy, foam);
                    }
                }
            }

            // Impassable rock is a wall, and a wall the player can see is a wall
            // they route around. World-anchored, so the ticks belong to the
            // ground rather than to the screen (§2.4).
            if tile.kind == TerrainKind::Mountain && tile.height >= MOUNTAIN_HEIGHT_MIN {
                let tick = rgba(ROCK_L);
                if world_hash(tx, ty, ROCK_SALT) % 2 == 0 {
                    canvas.put(ox + 1, oy + 1, tick);
                    canvas.put(ox + 2, oy + 2, tick);
                } else {
                    canvas.put(ox + 1, oy + 2, tick);
                    canvas.put(ox + 2, oy + 1, tick);
                }
            }
        }
    }
}

/// Which line, if any, owns each piece of track. The lowest line id wins a
/// shared tile — an arbitrary rule, but a *stable* one, and the plate has to
/// paint the same texels every time or the signature guard means nothing.
fn line_colours(world: &World) -> HashMap<TrackId, [u8; 4]> {
    let mut lines: Vec<_> = world.lines.iter().collect();
    lines.sort_by_key(|line| line.id.0);
    let mut owned: HashMap<TrackId, [u8; 4]> = HashMap::new();
    for line in lines {
        let Some(path) = line_path(world.network, world.stations, &line.stops) else {
            continue;
        };
        let [r, g, b] = line.colour.rgba();
        for id in path {
            owned.entry(id).or_insert([r, g, b, 255]);
        }
    }
    owned
}

/// Track as clean strokes: a core at the tile centre and a 2-texel run out along
/// every linked direction, meeting its neighbour's half in the middle.
fn paint_track(canvas: &mut Canvas, world: &World) {
    let owned = line_colours(world);
    let plain = rgba(RAIL_L);
    // The network is keyed by a hash map, so paint in tile order: a half-step
    // stroke reaches across its neighbours' tiles and two pieces can want the
    // same texel in different colours.
    let mut pieces: Vec<_> = world.network.iter().collect();
    pieces.sort_by_key(|piece| (piece.tile.y, piece.tile.x));
    for piece in pieces {
        let color = owned.get(&piece.id).copied().unwrap_or(plain);
        let center = tile_center(piece.tile);
        // The core, so an isolated piece is still a mark on the map.
        for dx in [-1, 0] {
            for dy in [-1, 0] {
                canvas.put(center.x as i32 + dx, center.y as i32 + dy, color);
            }
        }
        for dir in piece.links.dirs() {
            stroke(canvas, center, dir, color);
        }
    }
}

/// Walk half a link from the tile centre, 2 texels wide.
fn stroke(canvas: &mut Canvas, center: Vec2, dir: usize, color: [u8; 4]) {
    let (dx, dy) = DIR16[dir];
    let v = Vec2::new(dx as f32, dy as f32);
    let reach = v.length() * TEXELS_PER_TILE as f32 * 0.5;
    let along = v.normalize();
    let across = Vec2::new(-along.y, along.x);
    let mut t = 0.0;
    while t <= reach {
        for s in [-STROKE_HALF, STROKE_HALF] {
            let p = center + along * t + across * s;
            canvas.put(p.x.floor() as i32, p.y.floor() as i32, color);
        }
        t += WALK_STEP;
    }
}

/// Icon side in texels for a station tier — the schematic's stand-in for brief
/// 05 §6's "sized by throughput", since tier *is* the throughput grade.
fn tier_icon_side(tier: StationTier) -> i32 {
    match tier {
        StationTier::Halt => 2,
        StationTier::Station | StationTier::GoodsPlatform => 3,
        StationTier::Terminus => 4,
        StationTier::Interchange => 5,
    }
}

/// Stations and industries as ringed icons; anything still off-network takes
/// `hi`, which is what makes the next expansion obvious at a glance.
fn paint_stops(canvas: &mut Canvas, world: &World) {
    let mut stations: Vec<_> = world.stations.iter().collect();
    stations.sort_by_key(|station| station.id.0);
    for station in stations {
        let open = world.demand.is_open_station(station.id);
        let color = if open { HI } else { RAIL_S };
        icon(
            canvas,
            tile_center(station.tile),
            tier_icon_side(station.tier),
            rgba(color),
        );
    }
    let mut industries: Vec<_> = world.industries.iter().collect();
    industries.sort_by_key(|industry| industry.id.0);
    for industry in industries {
        let open = world.demand.is_open_industry(industry.id);
        let color = if open { HI } else { PLASTER_M };
        icon(canvas, tile_center(industry.tile), 3, rgba(color));
    }
}

/// A filled square with a 1-texel `outline` ring, so an icon separates from
/// whatever ground it stands on.
fn icon(canvas: &mut Canvas, center: Vec2, side: i32, color: [u8; 4]) {
    let lo_x = (center.x - side as f32 * 0.5).floor() as i32;
    let lo_y = (center.y - side as f32 * 0.5).floor() as i32;
    let ring = rgba(OUTLINE);
    for dy in -1..=side {
        for dx in -1..=side {
            let inside = (0..side).contains(&dx) && (0..side).contains(&dy);
            canvas.put(lo_x + dx, lo_y + dy, if inside { color } else { ring });
        }
    }
}

// ── The signature ──────────────────────────────────────────────────────────

/// FNV-1a over everything the plate draws, and nothing it does not.
///
/// The change ticks that bring us here answer *did anybody write this resource*,
/// which is a different question from *did the map move*. This one answers the
/// question we actually have.
fn signature(world: &World) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |value: u64| {
        for byte in value.to_le_bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };

    eat(world.map.width as u64);
    eat(world.map.height as u64);
    for tile in world.map.tiles() {
        eat(tile.height as u8 as u64);
        eat(tile.water as u64);
        eat(tile.kind as u64);
    }
    // Registries are hash maps; fold them in a fixed order or the "signature"
    // would change every frame on iteration order alone.
    let mut pieces: Vec<_> = world.network.iter().collect();
    pieces.sort_by_key(|piece| piece.id.0);
    for piece in pieces {
        eat(piece.id.0);
        eat(piece.tile.x as u32 as u64);
        eat(piece.tile.y as u32 as u64);
        eat(piece.links.0 as u64);
    }
    let mut stations: Vec<_> = world.stations.iter().collect();
    stations.sort_by_key(|station| station.id.0);
    for station in stations {
        eat(station.id.0);
        eat(station.tile.x as u32 as u64);
        eat(station.tile.y as u32 as u64);
        eat(station.tier as u64);
        eat(world.demand.is_open_station(station.id) as u64);
    }
    let mut industries: Vec<_> = world.industries.iter().collect();
    industries.sort_by_key(|industry| industry.id.0);
    for industry in industries {
        eat(industry.id.0);
        eat(industry.tile.x as u32 as u64);
        eat(industry.tile.y as u32 as u64);
        eat(world.demand.is_open_industry(industry.id) as u64);
    }
    let mut lines: Vec<_> = world.lines.iter().collect();
    lines.sort_by_key(|line| line.id.0);
    for line in lines {
        eat(line.id.0);
        eat(line.colour.0 as u64);
        for stop in &line.stops {
            eat(stop.0);
        }
    }
    hash
}

// ── Systems ────────────────────────────────────────────────────────────────

pub fn setup_schematic(mut commands: Commands) {
    commands.spawn((
        SchematicBackdrop,
        Sprite::from_color(BG0, Vec2::ONE),
        Transform::from_xyz(0.0, 0.0, BACKDROP_Z),
        Visibility::Hidden,
    ));
    commands.spawn((
        SchematicPlate { tiles: (0, 0) },
        Sprite {
            custom_size: Some(Vec2::ONE),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, SCHEMATIC_Z),
        Visibility::Hidden,
    ));
}

/// Note that something the plate draws may have moved. Costs a message drain.
///
/// Track and station edits arrive as messages, which have a two-frame life, so
/// this has to run every frame whether or not the view is open — a reader that
/// stops reading does not queue, it loses.
pub fn mark_schematic_dirty(
    mut state: ResMut<SchematicState>,
    mut track_edits: MessageReader<TrackEdit>,
    mut station_edits: MessageReader<StationEdit>,
    map: Res<MapGrid>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    lines: Res<LineRegistry>,
) {
    let mut touched = map.is_changed()
        || stations.is_changed()
        || industries.is_changed()
        || lines.is_changed();
    // A failed placement changed nothing, but the messages still have to be
    // drained or they pile into the next frame's read.
    for edit in track_edits.read() {
        touched |= !matches!(edit, TrackEdit::Failed { .. });
    }
    for edit in station_edits.read() {
        touched |= !matches!(edit, StationEdit::Failed { .. });
    }
    if touched && !state.dirty {
        state.dirty = true;
    }
}

/// Re-paint the plate when the world it was painted from has actually moved.
///
/// Idle unless the Map View is open *and* a trigger has fired, and even then the
/// signature has the last word.
#[allow(clippy::too_many_arguments)]
pub fn rebake_schematic(
    view: Res<MapViewState>,
    mut state: ResMut<SchematicState>,
    map: Res<MapGrid>,
    network: Res<TrackNetwork>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    lines: Res<LineRegistry>,
    demand: Res<DemandSpawner>,
    mut images: ResMut<Assets<Image>>,
    mut plate: Query<(&mut SchematicPlate, &mut Sprite, &mut Transform)>,
) {
    let _perf = crate::overlays::perf::scope("rebake_schematic");
    if !view.active || !state.dirty {
        return;
    }
    let Ok((mut plate, mut sprite, mut transform)) = plate.single_mut() else {
        return;
    };

    let world = World {
        map: &map,
        network: &network,
        stations: &stations,
        industries: &industries,
        lines: &lines,
        demand: &demand,
    };
    state.dirty = false;

    let signature = signature(&world);
    if state.signature == Some(signature) {
        return;
    }
    state.signature = Some(signature);
    state.bakes += 1;

    let tiles = (map.width, map.height);
    let canvas = paint(&world);
    if plate.tiles == tiles {
        // Same size: overwrite the texels in place, exactly as a terrain chunk
        // does, so the handle and its GPU texture are reused.
        if let Some(image) = images.get_mut(&sprite.image) {
            if let Some(data) = image.data.as_mut() {
                data.copy_from_slice(&canvas.px);
                return;
            }
        }
    }

    // First bake, or a map of a different size: a new image, and the old one
    // dropped rather than left behind.
    let fresh = images.add(canvas.into_image());
    let stale = std::mem::replace(&mut sprite.image, fresh);
    if plate.tiles != (0, 0) {
        images.remove(&stale);
    }
    plate.tiles = tiles;
    sprite.custom_size = Some(Vec2::new(
        map.width as f32 * TILE_SIZE,
        map.height as f32 * TILE_SIZE,
    ));
    // The plate's own extent, never the world's. `top_down_map_center` is a
    // plan-view helper that answers the same in either projection, which is what
    // keeps the plate a drawing rather than a picture of the camera's view.
    let (cx, cy) = top_down_map_center(map.width, map.height);
    transform.translation.x = cx;
    transform.translation.y = cy;
}

/// Show the plate and its ground while the Map View is open, and keep the ground
/// over the viewport.
pub fn sync_schematic_visibility(
    view: Res<MapViewState>,
    camera: Query<(&Transform, &Projection), With<super::camera::MapCamera>>,
    mut plate: Query<&mut Visibility, (With<SchematicPlate>, Without<SchematicBackdrop>)>,
    mut backdrop: Query<
        (&mut Transform, &mut Sprite, &mut Visibility),
        (With<SchematicBackdrop>, Without<super::camera::MapCamera>),
    >,
) {
    let wanted = if view.active {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    if let Ok(mut visibility) = plate.single_mut() {
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
    let Ok((mut transform, mut sprite, mut visibility)) = backdrop.single_mut() else {
        return;
    };
    if *visibility != wanted {
        *visibility = wanted;
    }
    if !view.active {
        return;
    }
    let Ok((cam, Projection::Orthographic(ortho))) = camera.single() else {
        return;
    };
    let size = ortho.area.size() + Vec2::splat(BACKDROP_MARGIN * 2.0);
    sprite.custom_size = Some(size.ceil());
    transform.translation.x = cam.translation.x.round();
    transform.translation.y = cam.translation.y.round();
}

/// Live train positions as moving dots (brief 05 §6) — the one thing in the Map
/// View that is not baked, because it is the one thing that moves.
///
/// The dots mirror the world sprites' transforms rather than re-deriving the
/// interpolation, so a dot can never disagree with the train it stands for.
pub fn sync_schematic_trains(
    mut commands: Commands,
    view: Res<MapViewState>,
    lines: Res<LineRegistry>,
    trains: Query<(&TrainSprite, &Transform), Without<SchematicTrainDot>>,
    mut dots: Query<(Entity, &SchematicTrainDot, &mut Transform, &mut Sprite)>,
) {
    let _perf = crate::overlays::perf::scope("sync_schematic_trains");
    if !view.active {
        for (entity, _, _, _) in dots.iter() {
            commands.entity(entity).despawn();
        }
        return;
    }

    let mut placed: HashMap<rail_sim::TrainId, Entity> = HashMap::new();
    for (entity, dot, _, _) in dots.iter() {
        placed.insert(dot.0, entity);
    }

    let mut seen = Vec::with_capacity(trains.iter().len());
    for (train, transform) in trains.iter() {
        seen.push(train.id);
        let color = lines
            .line_for_train(train.id)
            .map(|line| {
                let [r, g, b] = line.colour.rgba();
                Color::srgb_u8(r, g, b)
            })
            .unwrap_or(RAIL_S);
        if let Some(&entity) = placed.get(&train.id) {
            if let Ok((_, _, mut dot_tf, mut sprite)) = dots.get_mut(entity) {
                dot_tf.translation.x = transform.translation.x;
                dot_tf.translation.y = transform.translation.y;
                sprite.color = color;
            }
        } else {
            commands.spawn((
                Sprite::from_color(color, Vec2::splat(DOT_SIZE)),
                Transform::from_xyz(transform.translation.x, transform.translation.y, DOT_Z),
                SchematicTrainDot(train.id),
            ));
        }
    }

    for (entity, dot, _, _) in dots.iter() {
        if !seen.contains(&dot.0) {
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::map_view::{map_view_ortho_scale, MAP_VIEW_TEXELS_PER_TILE};
    use super::*;
    use rail_map::{generate_map, DEFAULT_MAP_SEED};
    use rail_sim::track::{step, try_place_track, TrackTerrain};
    use rail_sim::{
        DemandOpportunity, DemandOpportunityKind, Money, MoneyLedger, GROUND_LAYER,
    };

    /// Everything a plate can be painted from, owned so a test can hold it.
    struct Fixture {
        map: MapGrid,
        network: TrackNetwork,
        stations: StationRegistry,
        industries: IndustryRegistry,
        lines: LineRegistry,
        demand: DemandSpawner,
    }

    impl Fixture {
        fn new(map: MapGrid) -> Self {
            Self {
                map,
                network: TrackNetwork::new(),
                stations: StationRegistry::new(),
                industries: IndustryRegistry::new(),
                lines: LineRegistry::new(),
                demand: DemandSpawner::default(),
            }
        }

        fn flat(w: u32, h: u32) -> Self {
            Self::new(flat_map(w, h))
        }

        fn world(&self) -> World<'_> {
            World {
                map: &self.map,
                network: &self.network,
                stations: &self.stations,
                industries: &self.industries,
                lines: &self.lines,
                demand: &self.demand,
            }
        }

        fn paint(&self) -> Canvas {
            paint(&self.world())
        }

        /// Lay real track through the placement rules, so the link masks under
        /// test are the ones the graph actually produces.
        fn lay(&mut self, tiles: &[TileCoord]) {
            let (w, h) = (self.map.width, self.map.height);
            let terrain = TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 0i8)));
            let mut money = Money::new(10_000_000);
            let mut ledger = MoneyLedger::default();
            for tile in tiles {
                try_place_track(
                    &mut self.network,
                    &mut money,
                    &mut ledger,
                    &terrain,
                    *tile,
                    GROUND_LAYER,
                )
                .expect("track should place on flat land");
            }
        }

        /// Every texel of `color` on the plate.
        fn marks(&self, color: [u8; 4]) -> Vec<(i32, i32)> {
            let canvas = self.paint();
            (0..canvas.h as i32)
                .flat_map(|y| (0..canvas.w as i32).map(move |x| (x, y)))
                .filter(|&(x, y)| canvas.at(x, y) == color)
                .collect()
        }
    }

    fn flat_map(w: u32, h: u32) -> MapGrid {
        let mut map = MapGrid::empty(w, h, 1);
        for tile in map.tiles_mut() {
            tile.height = 1;
            tile.water = false;
            tile.kind = TerrainKind::Plains;
        }
        map
    }

    /// One texel is one screen pixel: the plate is baked at exactly the scale
    /// the Map View shows it at, so nothing is ever resampled (brief 01 §2.1).
    #[test]
    fn the_plate_is_baked_at_the_map_views_own_scale() {
        assert_eq!(TEXELS_PER_TILE as f32, MAP_VIEW_TEXELS_PER_TILE);
        let canvas = Canvas::new(9, 5);
        assert_eq!(canvas.w, 9 * TEXELS_PER_TILE);
        assert_eq!(canvas.h, 5 * TEXELS_PER_TILE);
        // The plate is drawn over the map's world extent, so one texel covers
        // exactly the world units the camera shows in one screen pixel.
        let world_per_texel = TILE_SIZE / TEXELS_PER_TILE as f32;
        assert_eq!(world_per_texel, map_view_ortho_scale());
    }

    /// The plate is a drawing, not a photograph — no atlas cell is ever read,
    /// and every colour on it is a named palette step.
    #[test]
    fn every_terrain_texel_comes_from_the_palette() {
        let fixture = Fixture::new(generate_map(48, 48, DEFAULT_MAP_SEED));
        let canvas = fixture.paint();

        let allowed: Vec<[u8; 4]> = {
            let mut v = vec![rgba(WATER_F), rgba(ROCK_L)];
            for kind in [
                TerrainKind::Water,
                TerrainKind::Beach,
                TerrainKind::Plains,
                TerrainKind::Hills,
                TerrainKind::Mountain,
            ] {
                for h in -16i8..=16 {
                    v.push(rgba(terrain_color(kind, h)));
                    v.push(rgba(material_of(kind).shadow(shade_for(kind, h))));
                }
            }
            v.sort();
            v.dedup();
            v
        };
        for y in 0..canvas.h as i32 {
            for x in 0..canvas.w as i32 {
                let px = canvas.at(x, y);
                assert!(
                    allowed.contains(&px),
                    "texel ({x}, {y}) is {px:?}, which is not on a palette ramp"
                );
            }
        }
    }

    /// Opaque edge to edge: the plate hides the world rather than letting a
    /// downsampled corner of it show through.
    #[test]
    fn the_plate_is_opaque_edge_to_edge() {
        let fixture = Fixture::new(generate_map(32, 32, 7));
        assert!(fixture.paint().px.chunks_exact(4).all(|p| p[3] == 255));
    }

    /// Elevation resolves into bands with a visible step, not a soft gradient
    /// (brief 02 §2.3).
    #[test]
    fn a_step_down_draws_a_banded_lip() {
        let mut fixture = Fixture::flat(4, 4);
        for y in 0..4i32 {
            for x in 0..4i32 {
                let tile = fixture.map.get_mut(TileCoord { x, y }).unwrap();
                // The north half stands three bands above the south half.
                tile.height = if y >= 2 { 9 } else { 0 };
            }
        }
        let canvas = fixture.paint();

        let (ox, oy) = tile_origin(TileCoord { x: 1, y: 2 });
        let lip = rgba(material_of(TerrainKind::Plains).shadow(shade_for(TerrainKind::Plains, 9)));
        assert_eq!(canvas.at(ox + 1, oy), lip, "the step down drew no lip");
        assert_ne!(
            canvas.at(ox + 1, oy + 1),
            lip,
            "the lip must be one texel, not a gradient"
        );
        // Ground inside a band draws no lip at all.
        let (fx, fy) = tile_origin(TileCoord { x: 1, y: 3 });
        assert_ne!(canvas.at(fx + 1, fy), lip);
    }

    /// Track is a stroke, not a tile fill: it marks its own tile without
    /// swallowing it (brief 05 §6, "track drawn thin").
    #[test]
    fn track_draws_as_a_thin_stroke() {
        let mut fixture = Fixture::flat(5, 5);
        fixture.lay(&[
            TileCoord { x: 1, y: 2 },
            TileCoord { x: 2, y: 2 },
            TileCoord { x: 3, y: 2 },
        ]);
        let canvas = fixture.paint();

        let (ox, oy) = tile_origin(TileCoord { x: 2, y: 2 });
        let rail = rgba(RAIL_L);
        let across: Vec<i32> = (0..TEXELS_PER_TILE as i32)
            .filter(|&dy| canvas.at(ox + 1, oy + dy) == rail)
            .collect();
        assert_eq!(across, vec![1, 2], "a stroke is two texels across, not four");
        // And it runs the width of the tile, meeting its neighbours' halves.
        for dx in 0..TEXELS_PER_TILE as i32 {
            assert_eq!(
                canvas.at(ox + dx, oy + 1),
                rail,
                "the stroke has a gap at dx={dx}"
            );
        }
    }

    /// Direction is drawn, not implied: each of the sixteen leaves its own mark.
    #[test]
    fn every_linked_direction_draws_its_own_stroke() {
        let center = TileCoord { x: 4, y: 4 };
        let mut seen = std::collections::HashSet::new();
        for dir in 0..DIR16.len() {
            let mut fixture = Fixture::flat(9, 9);
            fixture.lay(&[center, step(center, dir)]);
            let marks = fixture.marks(rgba(RAIL_L));
            assert!(!marks.is_empty(), "direction {dir} drew nothing");
            assert!(seen.insert(marks), "direction {dir} draws like another");
        }
        assert_eq!(seen.len(), DIR16.len());
    }

    /// Bake determinism: the same world always paints the same texels, which is
    /// what lets the signature stand in for the plate.
    #[test]
    fn the_bake_is_deterministic() {
        let laid = [
            TileCoord { x: 4, y: 4 },
            TileCoord { x: 5, y: 4 },
            TileCoord { x: 7, y: 5 },
        ];
        let mut fixture = Fixture::flat(32, 32);
        fixture.lay(&laid);
        fixture
            .stations
            .insert("Eastgate", TileCoord { x: 4, y: 4 }, GROUND_LAYER);
        assert_eq!(fixture.paint().px, fixture.paint().px);
        assert_eq!(signature(&fixture.world()), signature(&fixture.world()));

        // And a second, independently built world paints identically.
        let mut twin = Fixture::flat(32, 32);
        twin.lay(&laid);
        twin.stations
            .insert("Eastgate", TileCoord { x: 4, y: 4 }, GROUND_LAYER);
        assert_eq!(fixture.paint().px, twin.paint().px);
        assert_eq!(signature(&fixture.world()), signature(&twin.world()));
    }

    /// The signature has to move when anything drawn moves, or an edit would
    /// silently keep the old plate.
    #[test]
    fn the_signature_follows_everything_the_plate_draws() {
        let base = Fixture::flat(16, 16);
        let base_sig = signature(&base.world());

        let mut laid = Fixture::flat(16, 16);
        laid.lay(&[TileCoord { x: 3, y: 3 }]);
        assert_ne!(base_sig, signature(&laid.world()), "track must be seen");

        let mut stopped = Fixture::flat(16, 16);
        stopped
            .stations
            .insert("Eastgate", TileCoord { x: 8, y: 8 }, GROUND_LAYER);
        assert_ne!(base_sig, signature(&stopped.world()), "a stop must be seen");

        let mut raised = Fixture::flat(16, 16);
        raised.map.get_mut(TileCoord { x: 2, y: 2 }).unwrap().height = 12;
        assert_ne!(base_sig, signature(&raised.world()), "terrain must be seen");

        let resized = Fixture::flat(17, 16);
        assert_ne!(
            base_sig,
            signature(&resized.world()),
            "a new world must be seen"
        );
    }

    /// A station is an icon, and its grade is legible from its size.
    #[test]
    fn station_icons_scale_with_tier() {
        let mut sizes = Vec::new();
        for tier in [
            StationTier::Halt,
            StationTier::Station,
            StationTier::Terminus,
            StationTier::Interchange,
        ] {
            let mut fixture = Fixture::flat(12, 12);
            fixture
                .stations
                .insert_tier("Stop", TileCoord { x: 6, y: 6 }, GROUND_LAYER, tier, 0);
            sizes.push(fixture.marks(rgba(RAIL_S)).len());
        }
        assert!(
            sizes.windows(2).all(|w| w[0] < w[1]),
            "tiers must read apart: {sizes:?}"
        );
    }

    /// Unserved demand is marked, which is most of what the view is for.
    #[test]
    fn unserved_demand_takes_the_accent() {
        let mut served = Fixture::flat(12, 12);
        let id = served
            .stations
            .insert("Far Field", TileCoord { x: 6, y: 6 }, GROUND_LAYER);
        assert_eq!(
            served.marks(rgba(HI)).len(),
            0,
            "a served stop must not use hi"
        );

        let mut open = Fixture::flat(12, 12);
        open.stations
            .insert("Far Field", TileCoord { x: 6, y: 6 }, GROUND_LAYER);
        open.demand.open.push(DemandOpportunity {
            kind: DemandOpportunityKind::Settlement(id),
            name: "Far Field".into(),
            tile: TileCoord { x: 6, y: 6 },
        });
        assert!(
            !open.marks(rgba(HI)).is_empty(),
            "an unserved stop must be marked"
        );
    }

    // ── Bake on change, never per frame ────────────────────────────────────

    fn bake_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Image>>()
            .init_resource::<SchematicState>()
            .init_resource::<MapViewState>()
            .init_resource::<TrackNetwork>()
            .init_resource::<StationRegistry>()
            .init_resource::<IndustryRegistry>()
            .init_resource::<LineRegistry>()
            .init_resource::<DemandSpawner>()
            .insert_resource(flat_map(16, 16))
            .add_message::<TrackEdit>()
            .add_message::<StationEdit>()
            .add_systems(Startup, setup_schematic)
            .add_systems(
                Update,
                (
                    mark_schematic_dirty,
                    rebake_schematic,
                    sync_schematic_visibility,
                    sync_schematic_trains,
                )
                    .chain(),
            );
        // The Map View camera the backdrop chases.
        app.world_mut().spawn((
            super::super::camera::MapCamera,
            Transform::from_xyz(256.0, 256.0, 1000.0),
            Projection::Orthographic(OrthographicProjection {
                scale: super::super::map_view::map_view_ortho_scale(),
                ..OrthographicProjection::default_2d()
            }),
        ));
        app
    }

    fn bakes(app: &App) -> u32 {
        app.world().resource::<SchematicState>().bakes()
    }

    /// Nothing is painted until the player asks for the view.
    #[test]
    fn a_closed_map_view_bakes_nothing() {
        let mut app = bake_app();
        for _ in 0..8 {
            app.update();
        }
        assert_eq!(bakes(&app), 0);
    }

    /// The whole point of the debt item: the plate is art, and art is baked
    /// when data changes, never per frame (brief 01 §2.5).
    #[test]
    fn an_open_map_view_bakes_once_and_then_idles() {
        let mut app = bake_app();
        app.update();
        app.world_mut().resource_mut::<MapViewState>().active = true;
        app.update();
        assert_eq!(bakes(&app), 1, "opening the view has to paint the plate");
        for _ in 0..30 {
            app.update();
        }
        assert_eq!(bakes(&app), 1, "an idle map view re-baked");
    }

    /// ... and a real edit still repaints it.
    #[test]
    fn an_edit_repaints_the_plate() {
        let mut app = bake_app();
        app.world_mut().resource_mut::<MapViewState>().active = true;
        app.update();
        assert_eq!(bakes(&app), 1);

        // A refused build changed nothing, and must cost nothing.
        app.world_mut().write_message(TrackEdit::Failed {
            tile: Some(TileCoord { x: 1, y: 1 }),
            error: rail_sim::PlacementError::AlreadyOccupied,
        });
        app.update();
        assert_eq!(bakes(&app), 1, "a refused build repainted the plate");

        // A station that really appears does.
        app.world_mut()
            .resource_mut::<StationRegistry>()
            .insert("Eastgate", TileCoord { x: 6, y: 6 }, GROUND_LAYER);
        app.update();
        assert_eq!(bakes(&app), 2, "a new station did not repaint the plate");

        let images = app.world().resource::<Assets<Image>>();
        assert_eq!(images.len(), 1, "a re-bake must reuse its texture");
    }

    /// The plate is laid out in tile order at its own fixed scale, which is what
    /// keeps click-to-fly honest: the tile the pointer is over on the plate is
    /// the tile [`super::super::map_view::map_view_click_fly`] resolves.
    ///
    /// Checked in **both** projections, and the answer has to be the same one.
    /// Brief 02 §6 says the plate is "a second, purpose-built rendering" rather
    /// than a zoomed-out camera, so it was never a picture of the world and its
    /// geometry must not move when the world's does.
    #[test]
    fn the_plate_covers_exactly_the_map() {
        for projection in [rail_map::Projection::TopDown, rail_map::Projection::Iso] {
            let _guard = crate::map::tests::ProjectionGuard::new(projection);
            let mut app = bake_app();
            app.world_mut().resource_mut::<MapViewState>().active = true;
            app.update();

            let (sprite, transform) = app
                .world_mut()
                .query_filtered::<(&Sprite, &Transform), With<SchematicPlate>>()
                .single(app.world())
                .map(|(s, t)| (s.clone(), *t))
                .expect("plate");
            assert_eq!(sprite.custom_size, Some(Vec2::splat(16.0 * TILE_SIZE)));
            assert_eq!(
                transform.translation.truncate(),
                Vec2::splat(16.0 * TILE_SIZE * 0.5),
                "the plate must be centred on the map, not on the camera"
            );
            // And it never carries a transform of its own beyond that placement.
            assert_eq!(transform.rotation, Quat::IDENTITY);
            assert_eq!(transform.scale, Vec3::ONE);
        }
    }

    /// The view shows the plate and its `bg0` ground, and puts both away again.
    #[test]
    fn the_plate_shows_only_while_the_view_is_open() {
        let mut app = bake_app();
        app.update();
        let visible = |app: &mut App| {
            app.world_mut()
                .query_filtered::<&Visibility, Or<(With<SchematicPlate>, With<SchematicBackdrop>)>>()
                .iter(app.world())
                .filter(|v| **v == Visibility::Visible)
                .count()
        };
        assert_eq!(visible(&mut app), 0, "a closed view must show nothing");

        app.world_mut().resource_mut::<MapViewState>().active = true;
        app.update();
        assert_eq!(visible(&mut app), 2, "plate and ground both show");

        app.world_mut().resource_mut::<MapViewState>().active = false;
        app.update();
        assert_eq!(visible(&mut app), 0, "closing must put both away");
    }

    /// Trains are dots on the plate while the view is open, and nothing at all
    /// while it is closed.
    #[test]
    fn trains_show_as_dots_only_in_the_map_view() {
        let mut app = bake_app();
        app.world_mut().spawn((
            crate::trains::TrainSprite {
                id: rail_sim::TrainId(1),
            },
            Transform::from_xyz(100.0, 120.0, 3.0),
        ));
        app.update();
        let dots = |app: &mut App| {
            app.world_mut()
                .query::<(&SchematicTrainDot, &Transform)>()
                .iter(app.world())
                .map(|(_, t)| t.translation)
                .collect::<Vec<_>>()
        };
        assert!(dots(&mut app).is_empty());

        app.world_mut().resource_mut::<MapViewState>().active = true;
        app.update();
        assert_eq!(
            dots(&mut app),
            vec![Vec3::new(100.0, 120.0, DOT_Z)],
            "a dot must stand where its train does"
        );

        app.world_mut().resource_mut::<MapViewState>().active = false;
        app.update();
        assert!(dots(&mut app).is_empty(), "dots must not outlive the view");
    }

    /// A new map of a different size gets a plate of a different size.
    #[test]
    fn a_new_world_resizes_the_plate() {
        let mut app = bake_app();
        app.world_mut().resource_mut::<MapViewState>().active = true;
        app.update();

        app.insert_resource(flat_map(24, 20));
        app.update();
        assert_eq!(bakes(&app), 2);

        let handle = app
            .world_mut()
            .query_filtered::<&Sprite, With<SchematicPlate>>()
            .single(app.world())
            .expect("plate")
            .image
            .clone();
        let images = app.world().resource::<Assets<Image>>();
        let image = images.get(&handle).expect("plate image");
        assert_eq!(image.width(), 24 * TEXELS_PER_TILE);
        assert_eq!(image.height(), 20 * TEXELS_PER_TILE);
        assert_eq!(images.len(), 1, "the old plate must not be left behind");
    }
}

