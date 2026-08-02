//! Hover — the middle tier of interrogation.
//!
//! Brief 05 §1 lays out three tiers, each cheaper than the last: **ambient**
//! (free), **hover** (a moment) and **select** (a click). Hover answers *what is
//! this, and what is its headline number?* and nothing more; the Inspector owns
//! *why is it doing that, and what can I do?*
//!
//! Playtest: *"Identifying anything requires clicking; I think it should be
//! hover with a minimal, unobtrusive hover text/overlay. There do seem to be
//! tooltips on click but they appear far offscreen so I cannot read them."*
//!
//! So this module does three things and no more.
//!
//! - **Names everything.** Buildings, countryside, peeps, trains, stations,
//!   industries, track — and, when nothing else is there, the ground itself.
//!   Nothing under the pointer is allowed to stay a mystery (brief 05 §8.6).
//! - **Highlight.** Corner brackets in `railL` hugging the object's baked
//!   silhouette. Selection keeps `railS`, the brightest value in the palette
//!   (brief 05 §2), so a hovered thing never looks selected; a build ghost keeps
//!   `hi` (brief 01 §3.3), so hover never fights the accent that owns the screen
//!   during a build. Bare ground gets **no** bracket — a box that follows the
//!   cursor over empty grass is noise, and would fight the build ghost.
//! - **Chip.** After 400 ms (brief 03 §8.3), a `bg0` chip with a 1-texel outline
//!   naming the thing and giving one telling fact. Micro type, two lines at
//!   most. It is placed **outside the screen rectangle of the thing under the
//!   cursor**, flipping side or above as the window edge demands.
//!
//! Picking is [`super::selection::pick_world`], the same test a click uses, so
//! hovering something and clicking it can never disagree. Buildings and
//! countryside are not yet [`Selectable`], so they are resolved here, below
//! stations and industries and above track.
//!
//! # Three coordinate spaces, and the bug that came of mixing them
//!
//! | Space | Origin | Who speaks it |
//! | --- | --- | --- |
//! | **world** | texels, `+y` up | sprites, [`Camera::viewport_to_world_2d`] |
//! | **screen** | logical window px, `+y` down | [`Window::cursor_position`], [`Camera::world_to_viewport`], [`Window::width`] |
//! | **ui** | `Val::Px` units, `+y` down | [`Node::left`] / [`Node::top`], [`ComputedNode`] |
//!
//! `screen` and `ui` are *not* the same space. Bevy multiplies every `Val::Px`
//! by the layout scale factor, which is the window's scale factor **times
//! [`UiScale`]** (`bevy_ui::update`), and this game drives `UiScale` from the
//! display settings — 2 in the build the playtest ran on, and now whatever
//! `shell::settings::resolve_ui_scale` picks for the window, up to 4.
//!
//! So `screen = ui * UiScale`. The old code measured the tile in `screen`,
//! placed the tooltip in `screen`, and then wrote that number into
//! `Node::left`, where it was multiplied by `UiScale` all over again. At 2x, a
//! tile at screen x 1114 in a 1280-wide window put the chip's left edge at
//! 2244 — half a window past the right edge, which is exactly the *"they
//! appear far offscreen"* the playtest reported. It was not a maths error; the
//! maths was correct in both spaces. It was a units error, and the unit test
//! could not see it because it only ever worked in one space.
//!
//! [`tooltip_anchor`] is now the one place the conversion happens, and the
//! tests drive it at every `UiScale` the settings screen can produce.
//!
//! `screen = ui * UiScale` is not a reading of Bevy's source — it is measured
//! against the real layout engine by
//! [`bevy_multiplies_val_px_by_ui_scale`](app_tests::bevy_multiplies_val_px_by_ui_scale),
//! which lays out a node at four different scales and reads back where it
//! landed. **Anything else positioning a `Val::Px` against a value that came
//! from a `Window` or a `Camera` needs the same division** — window placement
//! included.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::ui::UiScale;
use bevy::window::PrimaryWindow;
use rail_map::{world_to_tile, MapGrid, TerrainKind, TILE_SIZE};
use rail_sim::{
    commands::TrainKind, IndustryRegistry, Mood, Peep, StationRegistry, StationService, TileCoord,
    TrackNetwork, Train, TrainLocation, WaitingAtStation,
};

use crate::map::{MapCamera, MapViewState};
use crate::palette::{BG0, OUTLINE, RAIL_L};
use crate::town::{
    lot_condition, lot_label, BuildingAtlas, BuildingLot, District, RuralKind, RuralProp,
};
use crate::ui::kit::{micro_font, text_primary, text_secondary, SPACE_1, SPACE_2};
use crate::ui::UiBlocksWorld;

use super::pick::{PickPriority, Selectable};
use super::selection::{pick_world, WorldPickSprites};

/// Dwell before the chip appears (brief 03 §8.3).
pub const TOOLTIP_DELAY: f32 = 0.4;

/// Gap between the thing under the cursor and the chip, in **ui** px.
const TOOLTIP_GAP: f32 = 6.0;

/// Length of one arm of a corner bracket, in world texels.
const BRACKET_ARM: f32 = 5.0;

/// The hover ring sits above the world and below the build ghost, which owns
/// `hi` while it is showing.
const BRACKET_Z: f32 = 3.0;

/// Pointer travel, in screen px, that counts as the pointer having moved.
const POINTER_EPSILON: f32 = 0.5;

/// Longest a pick may go stale while nothing the player controls has moved.
///
/// The world moves on its own — trains roll, peeps queue, scaffolding comes
/// down — so a pointer resting on a moving thing still has to be re-resolved.
/// Four times a second is under the reaction threshold and costs ~3% of what
/// picking every frame did.
const HOVER_REFRESH: f32 = 0.25;

/// Half-width of a town sprite's atlas cell (`building_art::CELL_W` is 24),
/// plus slack. Sprites are `Anchor::BOTTOM_CENTER` on their transform.
const CELL_REACH_X: f32 = 16.0;
/// Cell height (`building_art::CELL_H` is 48) plus slack, upward from the base.
const CELL_REACH_UP: f32 = 52.0;
/// Slack below the base, for sprites that dip under their anchor.
const CELL_REACH_DOWN: f32 = 4.0;

/// What the pointer is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoverTarget {
    /// A building lot, at whatever stage of its life.
    Lot(Entity),
    /// A farm, a field, a wall, a tree.
    Rural(Entity),
    /// Anything a click can already select.
    World(Selectable),
    /// Bare ground — the last resort, so nothing is ever unnamed.
    Ground(TileCoord),
}

/// Current hover state. One resource, read by the bracket and chip systems.
#[derive(Resource, Debug, Default)]
pub struct Hovered {
    pub target: Option<HoverTarget>,
    /// Seconds the pointer has rested on this exact target.
    pub held: f32,
    /// **world** rectangle of the thing, in texels. `None` for bare ground,
    /// which gets no bracket.
    pub bounds: Option<Rect>,
    /// **screen** rectangle the chip may never cover: the tile under the cursor
    /// (brief 03 §8.3) unioned with the hovered object's own silhouette, so a
    /// tall building is not hidden by the label describing it.
    pub obstacle: Option<Rect>,
    /// What it is: "Cottage", "Pine Sawmill", "Eastgate".
    pub title: String,
    /// One fact about it: "Residential - 3 residents", "Produces lumber".
    pub detail: String,
}

impl Hovered {
    fn clear(&mut self) {
        self.target = None;
        self.held = 0.0;
        self.bounds = None;
        self.obstacle = None;
        self.title.clear();
        self.detail.clear();
    }

    /// True once the pointer has rested long enough for the chip.
    pub fn tooltip_ready(&self) -> bool {
        self.target.is_some() && self.held >= TOOLTIP_DELAY && !self.title.is_empty()
    }
}

/// What the last pick was taken against, so the next frame can decide whether
/// to bother taking another one.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PickContext {
    cursor: Vec2,
    camera: Vec2,
    zoom: f32,
    window: Vec2,
}

impl PickContext {
    /// True when the player has moved the pointer, the camera, or the window —
    /// anything that changes *which* texel the pointer is over.
    fn differs_from(&self, other: &Self) -> bool {
        self.cursor.distance_squared(other.cursor) > POINTER_EPSILON * POINTER_EPSILON
            || self.camera != other.camera
            || self.zoom != other.zoom
            || self.window != other.window
    }
}

/// Hover's own bookkeeping: when the last pick happened, and how many there
/// have been. `picks` / `frames` is the headline cost number for this module.
#[derive(Resource, Debug, Default)]
pub struct HoverProbe {
    last: Option<PickContext>,
    since_refresh: f32,
    /// Picks actually taken.
    pub picks: u64,
    /// Frames [`hover_pick`] ran at all.
    pub frames: u64,
}

/// One arm-pair of the hover bracket, indexed `0..4` clockwise from bottom-left.
#[derive(Component, Debug, Clone, Copy)]
pub struct HoverBracket(pub usize);

#[derive(Component)]
pub struct HoverTooltipRoot;

#[derive(Component)]
pub struct HoverTooltipTitle;

#[derive(Component)]
pub struct HoverTooltipDetail;

/// The registries and sprite queries picking needs, bundled so the hover system
/// stays well inside Bevy's system-parameter limit.
#[derive(SystemParam)]
pub struct HoverWorld<'w, 's> {
    map: Res<'w, MapGrid>,
    map_view: Res<'w, MapViewState>,
    ui_blocks: Res<'w, UiBlocksWorld>,
    network: Res<'w, TrackNetwork>,
    stations: Res<'w, StationRegistry>,
    service: Res<'w, StationService>,
    industries: Res<'w, IndustryRegistry>,
    trains: Query<'w, 's, (&'static Train, &'static TrainLocation)>,
    sprites: WorldPickSprites<'w, 's>,
}

/// Peeps, and where they are queueing if they are.
type PeepQuery<'w, 's> = Query<'w, 's, (&'static Peep, Option<&'static WaitingAtStation>)>;
type LotQuery<'w, 's> =
    Query<'w, 's, (Entity, &'static BuildingLot, &'static Sprite, &'static Transform)>;
type PropQuery<'w, 's> =
    Query<'w, 's, (Entity, &'static RuralProp, &'static Sprite, &'static Transform)>;

// ─ Setup ───────────────────────────────────────────────

pub fn setup_hover(mut commands: Commands) {
    for i in 0..4 {
        commands.spawn((
            HoverBracket(i),
            Sprite::from_color(RAIL_L, Vec2::ONE),
            Transform::from_xyz(0.0, 0.0, BRACKET_Z),
            Visibility::Hidden,
        ));
        commands.spawn((
            HoverBracket(i + 4),
            Sprite::from_color(RAIL_L, Vec2::ONE),
            Transform::from_xyz(0.0, 0.0, BRACKET_Z),
            Visibility::Hidden,
        ));
    }

    commands
        .spawn((
            HoverTooltipRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                // Brief 03 §8.3 asks for an 8 inset; tightened on the short axis
                // so this reads as a chip rather than a second panel.
                padding: UiRect::axes(Val::Px(SPACE_2), Val::Px(SPACE_1)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::ZERO,
                display: Display::None,
                ..default()
            },
            BackgroundColor(BG0),
            BorderColor::all(OUTLINE),
            // No `Interaction` and no `WorldClickBlocker`: a chip that ate
            // clicks would make the thing it describes unclickable.
            ZIndex(30),
        ))
        .with_children(|root| {
            root.spawn((
                HoverTooltipTitle,
                Text::new(""),
                micro_font(),
                text_primary(),
            ));
            root.spawn((
                HoverTooltipDetail,
                Node::default(),
                Text::new(""),
                micro_font(),
                text_secondary(),
            ));
        });
}

// ─ Picking ─────────────────────────────────────────────

/// Resolve what the pointer is over, and how long it has been there.
///
/// Cheap by construction: it takes a pick only when the pointer, the camera or
/// the window moved, or when [`HOVER_REFRESH`] has elapsed. On every other
/// frame it advances the dwell timer and returns — and it advances it *without*
/// tripping change detection, so the bracket system can skip too.
#[allow(clippy::too_many_arguments)]
pub fn hover_pick(
    time: Res<Time<Real>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform, &Projection), With<MapCamera>>,
    world: HoverWorld,
    atlas: Option<Res<BuildingAtlas>>,
    lots: LotQuery,
    props: PropQuery,
    peeps: PeepQuery,
    mut hovered: ResMut<Hovered>,
    mut probe: ResMut<HoverProbe>,
) {
    let _perf = crate::overlays::perf::scope("hover_pick");
    let dt = time.delta_secs();
    probe.frames += 1;

    // Map View owns the pointer, and chrome under it is not the world.
    if world.map_view.active || world.ui_blocks.0 {
        forget(&mut hovered, &mut probe);
        return;
    }
    let Ok(window) = windows.single() else {
        forget(&mut hovered, &mut probe);
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        forget(&mut hovered, &mut probe);
        return;
    };
    let Ok((camera, cam_gt, projection)) = camera_q.single() else {
        forget(&mut hovered, &mut probe);
        return;
    };

    let context = PickContext {
        cursor,
        camera: cam_gt.translation().truncate(),
        zoom: match projection {
            Projection::Orthographic(ortho) => ortho.scale,
            _ => 1.0,
        },
        window: Vec2::new(window.width(), window.height()),
    };

    probe.since_refresh += dt;
    let stale = probe.since_refresh >= HOVER_REFRESH;
    let moved = probe.last.is_none_or(|last| context.differs_from(&last));
    if !moved && !stale {
        // Nothing under the pointer can have changed. Advance the dwell clock
        // only, and leave change detection alone so nothing downstream wakes.
        if hovered.target.is_some() {
            hovered.bypass_change_detection().held += dt;
        }
        return;
    }
    probe.last = Some(context);
    probe.since_refresh = 0.0;
    probe.picks += 1;

    let Ok(point) = camera.viewport_to_world_2d(cam_gt, cursor) else {
        hovered.clear();
        return;
    };
    let tile = world_to_tile(point.x, point.y);
    if !world.map.contains(tile) {
        hovered.clear();
        return;
    }

    let (target, bounds) = resolve(point, tile, &world, &atlas, &lots, &props);

    if target != hovered.target {
        hovered.target = target;
        hovered.held = 0.0;
    } else if target.is_some() {
        hovered.held += dt;
    }
    let (title, detail) = describe(target, &world, &lots, &props, &peeps);
    hovered.title = title;
    hovered.detail = detail;
    hovered.bounds = bounds;
    hovered.obstacle = obstacle_rect(tile, bounds, camera, cam_gt);
}

/// Drop the hover *and* the gate, so the next frame with a pointer re-picks
/// immediately rather than waiting out [`HOVER_REFRESH`].
fn forget(hovered: &mut Hovered, probe: &mut HoverProbe) {
    if hovered.target.is_some() || !hovered.title.is_empty() {
        hovered.clear();
    }
    probe.last = None;
    probe.since_refresh = HOVER_REFRESH;
}

/// What is under `point`, and the world rectangle to bracket.
///
/// Order is the click order plus two tiers a click does not have: buildings and
/// countryside sit below stations and industries and above track, and bare
/// ground catches everything else so nothing goes unnamed.
fn resolve(
    point: Vec2,
    tile: TileCoord,
    world: &HoverWorld,
    atlas: &Option<Res<BuildingAtlas>>,
    lots: &LotQuery,
    props: &PropQuery,
) -> (Option<HoverTarget>, Option<Rect>) {
    let clicked = pick_world(
        point,
        tile,
        &world.network,
        &world.stations,
        &world.industries,
        &world.sprites,
    );

    // A station or an industry on the same texels beats the building art, so
    // resolve those before paying for the town scan at all.
    if let Some(sel) = clicked.filter(|s| s.priority() <= PickPriority::Industry) {
        let bounds = world_bounds(sel, world, tile);
        return (Some(HoverTarget::World(sel)), bounds);
    }

    if let Some((target, rect)) = pick_town(point, atlas, lots, props) {
        return (Some(target), Some(rect));
    }

    if let Some(sel) = clicked {
        return (
            Some(HoverTarget::World(sel)),
            world_bounds(sel, world, tile).or_else(|| Some(tile_world_rect(tile))),
        );
    }

    // Bare ground: named, but not bracketed.
    (Some(HoverTarget::Ground(tile)), None)
}

/// Hit-test buildings and countryside, whatever draws in front winning.
///
/// Two rejections happen before the atlas is touched: a four-compare bounding
/// test against the sprite's own transform ([`within_cell_reach`]), and a depth
/// test against the best candidate so far. Only survivors pay for a silhouette
/// lookup, which on a full map is a handful of entities out of thousands.
fn pick_town(
    point: Vec2,
    atlas: &Option<Res<BuildingAtlas>>,
    lots: &LotQuery,
    props: &PropQuery,
) -> Option<(HoverTarget, Rect)> {
    let atlas = atlas.as_ref()?;
    let mut best: Option<(f32, HoverTarget, Rect)> = None;

    for (entity, _, sprite, tf) in lots.iter() {
        consider_town_sprite(
            point,
            atlas,
            sprite,
            tf,
            HoverTarget::Lot(entity),
            &mut best,
        );
    }
    for (entity, _, sprite, tf) in props.iter() {
        consider_town_sprite(
            point,
            atlas,
            sprite,
            tf,
            HoverTarget::Rural(entity),
            &mut best,
        );
    }

    best.map(|(_, target, rect)| (target, rect))
}

fn consider_town_sprite(
    point: Vec2,
    atlas: &BuildingAtlas,
    sprite: &Sprite,
    tf: &Transform,
    target: HoverTarget,
    best: &mut Option<(f32, HoverTarget, Rect)>,
) {
    let z = tf.translation.z;
    if !within_cell_reach(point, tf.translation.truncate()) {
        return;
    }
    // Whatever draws in front is what the player thinks they are pointing at,
    // so anything already beaten on depth never needs its silhouette read.
    if best.as_ref().is_some_and(|(bz, _, _)| z <= *bz) {
        return;
    }
    let Some(rect) = sprite_bounds(atlas, sprite, tf) else {
        return;
    };
    if rect.contains(point) {
        *best = Some((z, target, rect));
    }
}

/// Conservative bound on where a bottom-centre-anchored town sprite can draw.
///
/// Four compares on data already in the query. Never rejects a sprite whose
/// silhouette actually covers `point` — the reach constants are the atlas cell
/// plus slack, and the cell is what [`BuildingAtlas::frame_rect`] is clipped to.
#[inline]
fn within_cell_reach(point: Vec2, base: Vec2) -> bool {
    point.x >= base.x - CELL_REACH_X
        && point.x <= base.x + CELL_REACH_X
        && point.y >= base.y - CELL_REACH_DOWN
        && point.y <= base.y + CELL_REACH_UP
}

/// World rectangle of a town sprite's *drawn* texels.
///
/// The atlas cell is mostly empty sky above the roof, so the baked silhouette
/// bounds are what make the bracket hug the building and the hit test honest.
fn sprite_bounds(atlas: &BuildingAtlas, sprite: &Sprite, tf: &Transform) -> Option<Rect> {
    let frame = sprite.texture_atlas.as_ref()?.index;
    atlas.frame_rect(frame, tf.translation.truncate(), sprite.flip_x)
}

/// The tile's own rectangle, as a fallback highlight for tile-resident things.
fn tile_world_rect(tile: TileCoord) -> Rect {
    Rect::new(
        tile.x as f32 * TILE_SIZE,
        tile.y as f32 * TILE_SIZE,
        (tile.x + 1) as f32 * TILE_SIZE,
        (tile.y + 1) as f32 * TILE_SIZE,
    )
}

/// Where a clickable world object sits, so the bracket can frame it too.
///
/// The sprite is the truth — it is what picking matched against and what the
/// player is looking at. Registries are only the fallback for a thing whose
/// sprite has not been spawned yet (a station placed this frame, say).
fn world_bounds(sel: Selectable, world: &HoverWorld, tile: TileCoord) -> Option<Rect> {
    if let Some(rect) = world.sprites.rect_of(sel) {
        return Some(rect);
    }
    let centred = |t: TileCoord, size: f32| {
        let (x, y) = rail_map::tile_to_world(t);
        Some(Rect::from_center_size(Vec2::new(x, y), Vec2::splat(size)))
    };
    match sel {
        Selectable::Station(id) => world
            .stations
            .get(id)
            .and_then(|s| centred(s.tile, TILE_SIZE * 0.7)),
        Selectable::Industry(id) => world
            .industries
            .get(id)
            .and_then(|i| centred(i.tile, TILE_SIZE * 0.7)),
        Selectable::Track(_) => centred(tile, TILE_SIZE * 0.8),
        // A peep or a train with no sprite is off-screen or despawning; framing
        // its tile would put a bracket around empty ground.
        Selectable::Peep(_) | Selectable::Train(_) => None,
    }
}

/// The **screen** rectangle the chip must stay clear of.
///
/// The tile under the cursor is the binding one (brief 03 §8.3 — while building,
/// that tile is the whole point of the gesture); the hovered silhouette is
/// unioned in so a two-storey building is not hidden behind its own label.
fn obstacle_rect(
    tile: TileCoord,
    bounds: Option<Rect>,
    camera: &Camera,
    cam_gt: &GlobalTransform,
) -> Option<Rect> {
    let tile_rect = world_rect_to_screen(tile_world_rect(tile), camera, cam_gt)?;
    let Some(shape) = bounds.and_then(|b| world_rect_to_screen(b, camera, cam_gt)) else {
        return Some(tile_rect);
    };
    Some(tile_rect.union(shape))
}

/// Project a **world** rectangle into **screen** space (logical window px).
fn world_rect_to_screen(world: Rect, camera: &Camera, cam_gt: &GlobalTransform) -> Option<Rect> {
    // World +y is up, screen +y is down, so the world min maps to the screen max.
    let a = camera
        .world_to_viewport(cam_gt, Vec3::new(world.min.x, world.min.y, 0.0))
        .ok()?;
    let b = camera
        .world_to_viewport(cam_gt, Vec3::new(world.max.x, world.max.y, 0.0))
        .ok()?;
    Some(Rect::new(
        a.x.min(b.x),
        a.y.min(b.y),
        a.x.max(b.x),
        a.y.max(b.y),
    ))
}

// ─ Words ───────────────────────────────────────────────

/// Name and one fact. ASCII only — the shipped font has no other glyphs.
fn describe(
    target: Option<HoverTarget>,
    world: &HoverWorld,
    lots: &LotQuery,
    props: &PropQuery,
    peeps: &PeepQuery,
) -> (String, String) {
    match target {
        None => (String::new(), String::new()),
        Some(HoverTarget::Lot(entity)) => match lots.get(entity) {
            Ok((_, lot, _, _)) => (lot_label(lot).to_string(), lot_detail(lot, peeps)),
            Err(_) => (String::new(), String::new()),
        },
        Some(HoverTarget::Rural(entity)) => match props.get(entity) {
            Ok((_, prop, _, _)) => (
                prop.kind.label().to_string(),
                match prop.kind {
                    RuralKind::Farmstead => "Open country - unserved".into(),
                    RuralKind::Prop(_) => "Open country".into(),
                },
            ),
            Err(_) => (String::new(), String::new()),
        },
        Some(HoverTarget::World(sel)) => describe_world(sel, world, peeps),
        Some(HoverTarget::Ground(tile)) => describe_ground(tile, world),
    }
}

/// A building's headline: who lives there, or what is happening to it.
fn lot_detail(lot: &BuildingLot, peeps: &PeepQuery) -> String {
    let district = lot.district.label();
    if let Some(condition) = lot_condition(lot) {
        return format!("{district} - {condition}");
    }
    if lot.district == District::Industrial {
        return format!("{district} - goods");
    }
    let residents = peeps.iter().filter(|(p, _)| p.home == lot.tile).count();
    match residents {
        0 => format!("{district} - empty"),
        1 => format!("{district} - 1 resident"),
        n => format!("{district} - {n} residents"),
    }
}

/// The ground itself. Terrain is what the Cost and Gradient overlays teach
/// (brief 05 §5); hover should teach the same thing one tile at a time.
fn describe_ground(tile: TileCoord, world: &HoverWorld) -> (String, String) {
    match world.map.get(tile) {
        Some(cell) => (terrain_label(cell.kind).to_string(), ground_detail(cell)),
        None => (String::new(), String::new()),
    }
}

fn terrain_label(kind: TerrainKind) -> &'static str {
    match kind {
        TerrainKind::Water => "Water",
        TerrainKind::Beach => "Beach",
        TerrainKind::Plains => "Plains",
        TerrainKind::Hills => "Hills",
        TerrainKind::Mountain => "Mountain",
    }
}

/// What the ground can be used for — the same lesson the Cost and Gradient
/// overlays teach (brief 05 §5), one tile at a time.
fn ground_detail(cell: &rail_map::Tile) -> String {
    if !cell.is_walkable_for_track() {
        return "Track needs a bridge".to_string();
    }
    format!("Height {} - open ground", cell.height)
}

fn describe_world(sel: Selectable, world: &HoverWorld, peeps: &PeepQuery) -> (String, String) {
    match sel {
        Selectable::Station(id) => match world.stations.get(id) {
            Some(s) => {
                let score = world.service.score(id);
                (
                    s.name.clone(),
                    format!("{} - service {}", s.tier.label(), score.score),
                )
            }
            None => ("Station".into(), "Stop".into()),
        },
        Selectable::Industry(id) => match world.industries.get(id) {
            Some(i) => {
                let detail = match (i.produces, i.consumes) {
                    (Some(p), Some(c)) => format!("Makes {} from {}", p.label(), c.label()),
                    (Some(p), None) => format!("Produces {}", p.label()),
                    (None, Some(c)) => format!("Takes {}", c.label()),
                    (None, None) => "Industry".into(),
                };
                (i.name.clone(), detail)
            }
            None => ("Industry".into(), "Industry".into()),
        },
        Selectable::Track(id) => {
            let detail = match world.network.piece(id) {
                Some(p) if p.is_bridge() => "Bridge".to_string(),
                Some(p) => format!("Grade {} - curve {}", p.max_grade, p.curve),
                None => "Rail".to_string(),
            };
            ("Track".into(), detail)
        }
        Selectable::Train(id) => {
            let found = world.trains.iter().find(|(t, _)| t.id == id);
            let detail = match found {
                Some((train, loc)) => {
                    let kind = match train.kind {
                        TrainKind::Transit => "Transit",
                        TrainKind::Transport => "Transport",
                    };
                    format!("{kind} - {}", train_status(loc))
                }
                None => "Rolling stock".to_string(),
            };
            (format!("Train {}", id.0), detail)
        }
        Selectable::Peep(id) => match peeps.iter().find(|(p, _)| p.id == id) {
            Some((p, waiting)) => {
                let mood = match p.mood {
                    Mood::Content => "Content",
                    Mood::Uneasy => "Uneasy",
                    Mood::Frustrated => "Frustrated",
                };
                let detail = match waiting {
                    Some(w) => format!("{mood} - waiting {} min", w.wait_secs / 60),
                    None => mood.to_string(),
                };
                (p.name.clone(), detail)
            }
            None => ("Someone".into(), "Resident".into()),
        },
    }
}

fn train_status(loc: &TrainLocation) -> &'static str {
    if loc.parked {
        "parked"
    } else if loc.dwell_remaining > 0 {
        "loading"
    } else if loc.at_destination() {
        "idle"
    } else {
        "running"
    }
}

// ─ Highlight ───────────────────────────────────────────

/// Corner brackets around whatever the pointer is over.
///
/// Skips entirely unless the pick changed — [`hover_pick`] advances the dwell
/// clock without tripping change detection, so a resting pointer costs nothing
/// here either.
pub fn sync_hover_brackets(
    hovered: Res<Hovered>,
    mut brackets: Query<(&HoverBracket, &mut Transform, &mut Sprite, &mut Visibility)>,
) {
    if !hovered.is_changed() {
        return;
    }
    let _perf = crate::overlays::perf::scope("sync_hover_brackets");

    let Some(rect) = hovered.bounds else {
        for (_, _, _, mut vis) in brackets.iter_mut() {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
        }
        return;
    };

    for (bracket, mut tf, mut sprite, mut vis) in brackets.iter_mut() {
        let (centre, size) = bracket_arm(bracket.0, rect);
        // Whole texels: the pixel contract does not bend for a highlight.
        tf.translation.x = centre.x.round();
        tf.translation.y = centre.y.round();
        tf.translation.z = BRACKET_Z;
        sprite.color = RAIL_L;
        sprite.custom_size = Some(size);
        if *vis != Visibility::Visible {
            *vis = Visibility::Visible;
        }
    }
}

/// Centre and size of bracket arm `index` (`0..8`) around `rect`.
///
/// Arms `0..4` are the horizontal ones, `4..8` the vertical ones, corners
/// ordered anticlockwise from the bottom left. A bracket rather than a full box
/// because a closed rectangle around every building the pointer crosses reads
/// as a selection, and selection is the *next* tier down.
pub fn bracket_arm(index: usize, rect: Rect) -> (Vec2, Vec2) {
    let arm = BRACKET_ARM
        .min(rect.width() * 0.45)
        .min(rect.height() * 0.45)
        .max(1.0);
    let corner = index % 4;
    let left = corner == 0 || corner == 3;
    let bottom = corner < 2;

    if index < 4 {
        let x = if left {
            rect.min.x + arm * 0.5
        } else {
            rect.max.x - arm * 0.5
        };
        let y = if bottom {
            rect.min.y + 0.5
        } else {
            rect.max.y - 0.5
        };
        (Vec2::new(x, y), Vec2::new(arm, 1.0))
    } else {
        let x = if left {
            rect.min.x + 0.5
        } else {
            rect.max.x - 0.5
        };
        let y = if bottom {
            rect.min.y + arm * 0.5
        } else {
            rect.max.y - arm * 0.5
        };
        (Vec2::new(x, y), Vec2::new(1.0, arm))
    }
}

// ─ Chip ────────────────────────────────────────────────

/// Show, fill and place the chip. Never over the thing under the cursor, never
/// off the screen.
pub fn update_hover_tooltip(
    hovered: Res<Hovered>,
    ui_scale: Res<UiScale>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut root: Query<(&mut Node, &ComputedNode), With<HoverTooltipRoot>>,
    mut title: Query<&mut Text, (With<HoverTooltipTitle>, Without<HoverTooltipDetail>)>,
    mut detail: Query<
        (&mut Text, &mut Node),
        (
            With<HoverTooltipDetail>,
            Without<HoverTooltipTitle>,
            Without<HoverTooltipRoot>,
        ),
    >,
) {
    let Ok((mut node, computed)) = root.single_mut() else {
        return;
    };

    if !hovered.tooltip_ready() {
        if node.display != Display::None {
            node.display = Display::None;
        }
        return;
    }
    let _perf = crate::overlays::perf::scope("update_hover_tooltip");

    if let Ok(mut text) = title.single_mut() {
        if text.0 != hovered.title {
            text.0 = hovered.title.clone();
        }
    }
    if let Ok((mut text, mut detail_node)) = detail.single_mut() {
        if text.0 != hovered.detail {
            text.0 = hovered.detail.clone();
        }
        // One line or two, never an empty second row padding out the chip.
        let wanted = if hovered.detail.is_empty() {
            Display::None
        } else {
            Display::Flex
        };
        if detail_node.display != wanted {
            detail_node.display = wanted;
        }
    }
    // Guarded, not assigned: touching `Node` re-syncs the taffy tree, and a
    // chip that sits on screen for a few seconds would do that every frame.
    if node.display != Display::Flex {
        node.display = Display::Flex;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Some(obstacle) = hovered.obstacle else {
        return;
    };
    let at = tooltip_anchor(
        obstacle,
        Vec2::new(window.width(), window.height()),
        tooltip_size(computed, &hovered.title, &hovered.detail),
        ui_scale.0,
    );
    // Whole ui pixels. `UiScale` is always a whole number (design 03 §2), so an
    // integer here stays an integer after Bevy scales it — pixel contract §2.
    let (left, top) = (Val::Px(at.x.round()), Val::Px(at.y.round()));
    if node.left != left {
        node.left = left;
    }
    if node.top != top {
        node.top = top;
    }
}

/// Measured size once layout has run, estimated on the first frame. **ui** px.
///
/// `ComputedNode::size` is physical pixels and `inverse_scale_factor` is
/// `1 / (window scale * UiScale)`, so the product is the size in the same units
/// `Node::left` / `top` are written in.
fn tooltip_size(computed: &ComputedNode, title: &str, detail: &str) -> Vec2 {
    let measured = computed.size * computed.inverse_scale_factor();
    if measured.x > 1.0 && measured.y > 1.0 {
        return measured;
    }
    // Micro type, roughly 6 ui px per character.
    let chars = title.chars().count().max(detail.chars().count()) as f32;
    let lines = if detail.is_empty() { 1.0 } else { 2.0 };
    Vec2::new(
        chars * 6.0 + SPACE_2 * 2.0 + 2.0,
        lines * 12.0 + SPACE_1 * 2.0 + 2.0,
    )
}

/// Convert the obstacle and the window from **screen** px into **ui** px, then
/// place the chip. The one place the two spaces meet.
///
/// This is the function the offscreen-tooltip bug lived in — or rather, the
/// function that did not exist, because the conversion was never done. Tests
/// drive it at every `UiScale` the settings screen can produce.
pub fn tooltip_anchor(obstacle: Rect, window: Vec2, size: Vec2, ui_scale: f32) -> Vec2 {
    let scale = if ui_scale.is_finite() && ui_scale > 0.0 {
        ui_scale
    } else {
        1.0
    };
    let obstacle_ui = Rect {
        min: obstacle.min / scale,
        max: obstacle.max / scale,
    };
    place_tooltip(obstacle_ui, size, window / scale)
}

/// Place a `size` box beside `obstacle` without covering it, and without
/// leaving `screen`. Every argument is in the **same** space; callers should go
/// through [`tooltip_anchor`] rather than guessing which one.
///
/// Brief 03 §8.3 is explicit that a tooltip must never cover the tile under the
/// cursor — while building, that tile is the whole point of the gesture. Staying
/// on screen is the harder invariant of the two, though: a chip the player
/// cannot see is worse than one that overlaps. So the four sides are tried in
/// preference order, and if none of them both fits and clears, the chip takes
/// the on-screen corner that covers the obstacle least.
pub fn place_tooltip(obstacle: Rect, size: Vec2, screen: Vec2) -> Vec2 {
    let low = Vec2::splat(TOOLTIP_GAP);
    // When the chip is larger than the window, `high` drops below `low`; taking
    // the min keeps the top-left corner on screen instead of inverting.
    let high = (screen - size - low).max(Vec2::ZERO);
    let clamp = |v: Vec2| v.clamp(low.min(high), high);

    let level = clamp(Vec2::new(0.0, obstacle.min.y)).y;
    let aligned = clamp(Vec2::new(obstacle.min.x, 0.0)).x;

    let sides = [
        Vec2::new(obstacle.max.x + TOOLTIP_GAP, level), // right of it
        Vec2::new(obstacle.min.x - TOOLTIP_GAP - size.x, level), // left of it
        Vec2::new(aligned, obstacle.max.y + TOOLTIP_GAP), // under it
        Vec2::new(aligned, obstacle.min.y - TOOLTIP_GAP - size.y), // over it
    ];
    for at in sides {
        if on_screen(at, size, screen) && !rects_overlap(box_at(at, size), obstacle) {
            return at;
        }
    }

    // The obstacle fills the window (zoomed hard into one tile, say). Stay
    // readable: whichever corner it eats the least of.
    [
        Vec2::new(low.x, low.y),
        Vec2::new(high.x, low.y),
        Vec2::new(low.x, high.y),
        Vec2::new(high.x, high.y),
    ]
    .into_iter()
    .map(clamp)
    .min_by(|a, b| {
        overlap_area(box_at(*a, size), obstacle)
            .partial_cmp(&overlap_area(box_at(*b, size), obstacle))
            .unwrap_or(std::cmp::Ordering::Equal)
    })
    .unwrap_or(low)
}

#[inline]
fn box_at(at: Vec2, size: Vec2) -> Rect {
    Rect::new(at.x, at.y, at.x + size.x, at.y + size.y)
}

#[inline]
fn on_screen(at: Vec2, size: Vec2, screen: Vec2) -> bool {
    at.x >= 0.0 && at.y >= 0.0 && at.x + size.x <= screen.x && at.y + size.y <= screen.y
}

#[inline]
fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
}

fn overlap_area(a: Rect, b: Rect) -> f32 {
    let w = (a.max.x.min(b.max.x) - a.min.x.max(b.min.x)).max(0.0);
    let h = (a.max.y.min(b.max.y) - a.min.y.max(b.min.y)).max(0.0);
    w * h
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Vec2 = Vec2::new(1280.0, 720.0);
    /// Every scale `shell::settings::resolve_ui_scale` can hand back — always a
    /// whole number (design 03 §2), and now as high as 4 on a large display.
    const UI_SCALES: [f32; 4] = [1.0, 2.0, 3.0, 4.0];

    fn tile_at(x: f32, y: f32) -> Rect {
        Rect::new(x, y, x + 64.0, y + 64.0)
    }

    /// Every screen position a tile can sit at, in **screen** px.
    fn sweep() -> impl Iterator<Item = Rect> {
        (0..20).flat_map(|gx| (0..12).map(move |gy| tile_at(gx as f32 * 64.0, gy as f32 * 64.0)))
    }

    /// Where the chip will actually be drawn, back in **screen** px.
    fn drawn(anchor: Vec2, size_ui: Vec2, ui_scale: f32) -> Rect {
        box_at(anchor * ui_scale, size_ui * ui_scale)
    }

    // ─ The offscreen bug ───────────────────────────────

    /// The regression. The old code fed a **screen**-space rect and a
    /// **screen**-space window into `place_tooltip` and wrote the result into
    /// `Node::left`, which Bevy multiplies by `UiScale`. At the shipped default
    /// of 2x that put the chip at twice its intended distance from the top-left
    /// corner, which for anything right of centre is off the window entirely —
    /// exactly what the playtest saw. The old unit tests could not catch it
    /// because they only ever exercised one space.
    #[test]
    fn the_chip_stays_on_screen_at_every_ui_scale() {
        let size_ui = Vec2::new(120.0, 30.0);
        for scale in UI_SCALES {
            for tile in sweep() {
                let at = tooltip_anchor(tile, SCREEN, size_ui, scale);
                let rect = drawn(at, size_ui, scale);
                assert!(
                    rect.min.x >= 0.0
                        && rect.min.y >= 0.0
                        && rect.max.x <= SCREEN.x
                        && rect.max.y <= SCREEN.y,
                    "at {scale}x ui scale a chip anchored at {at:?} draws at {rect:?}, \
                     off a {SCREEN:?} window"
                );
            }
        }
    }

    /// The fix, stated directly: the anchor is a `Val::Px` value, so raising
    /// the ui scale must *lower* it in proportion. Scaled back up, every ui
    /// scale has to name the same place on the window — give or take the gap,
    /// which is a design constant in ui texels and so is deliberately the same
    /// 6 texels at 1x as at 3x.
    ///
    /// The old code returned the identical anchor at every scale, which fails
    /// here by hundreds of pixels.
    #[test]
    fn the_anchor_is_in_ui_pixels_not_window_pixels() {
        let tile = tile_at(400.0, 300.0);
        let size_ui = Vec2::new(120.0, 30.0);
        let at_1x = tooltip_anchor(tile, SCREEN, size_ui, 1.0);
        for scale in UI_SCALES {
            let at = tooltip_anchor(tile, SCREEN, size_ui, scale);
            let slack = (scale - 1.0) * TOOLTIP_GAP + 0.001;
            let drift = (at * scale - at_1x).abs();
            assert!(
                drift.x <= slack && drift.y <= slack,
                "{scale}x anchor {at:?} scales back to {:?}, not {at_1x:?}",
                at * scale
            );
        }
    }

    /// A degenerate `UiScale` must not produce NaN coordinates, which taffy
    /// turns into a node nowhere on screen.
    #[test]
    fn a_broken_ui_scale_still_lands_on_screen() {
        let size_ui = Vec2::new(120.0, 30.0);
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let at = tooltip_anchor(tile_at(200.0, 200.0), SCREEN, size_ui, scale);
            assert!(at.is_finite(), "{scale} produced {at:?}");
            assert!(at.x >= 0.0 && at.y >= 0.0, "{scale} produced {at:?}");
        }
    }

    // ─ Placement ───────────────────────────────────────

    #[test]
    fn the_chip_never_covers_the_thing_under_the_cursor() {
        let size = Vec2::new(180.0, 44.0);
        for tile in sweep() {
            let at = place_tooltip(tile, size, SCREEN);
            assert!(
                !rects_overlap(box_at(at, size), tile),
                "chip at {at:?} covers the thing {tile:?} under the cursor"
            );
        }
    }

    /// A tall building's silhouette is unioned into the obstacle, so the chip
    /// has to clear the whole shape and not merely the tile it stands on.
    #[test]
    fn the_chip_clears_a_tall_silhouette_not_just_its_tile() {
        let size = Vec2::new(140.0, 30.0);
        let building = Rect::new(600.0, 200.0, 660.0, 460.0);
        let at = place_tooltip(building, size, SCREEN);
        assert!(!rects_overlap(box_at(at, size), building));
    }

    #[test]
    fn the_chip_flips_side_rather_than_running_off_the_edge() {
        let size = Vec2::new(200.0, 44.0);
        let near_right = tile_at(SCREEN.x - 80.0, 300.0);
        let at = place_tooltip(near_right, size, SCREEN);
        assert!(
            at.x + size.x <= near_right.min.x,
            "against the right edge the chip must flip to the left"
        );
    }

    /// When the obstacle swallows the window there is no placement that clears
    /// it. On-screen wins: a chip you cannot read is worse than one that laps.
    #[test]
    fn an_obstacle_bigger_than_the_window_still_leaves_the_chip_readable() {
        let size = Vec2::new(200.0, 44.0);
        let huge = Rect::new(-500.0, -500.0, 2000.0, 1500.0);
        let at = place_tooltip(huge, size, SCREEN);
        assert!(
            on_screen(at, size, SCREEN),
            "chip at {at:?} left the screen when the obstacle covered everything"
        );
    }

    #[test]
    fn a_chip_wider_than_the_window_pins_to_the_corner() {
        let size = Vec2::new(2000.0, 44.0);
        let at = place_tooltip(tile_at(600.0, 300.0), size, SCREEN);
        assert!(at.x >= 0.0 && at.y >= 0.0, "chip pinned off-origin at {at:?}");
        assert!(at.x <= SCREEN.x && at.y <= SCREEN.y);
    }

    // ─ Dwell ───────────────────────────────────────────

    #[test]
    fn hover_waits_before_it_speaks() {
        let mut hovered = Hovered {
            target: Some(HoverTarget::World(Selectable::Track(rail_sim::TrackId(1)))),
            title: "Track".into(),
            ..Default::default()
        };
        assert!(!hovered.tooltip_ready(), "a chip must not appear instantly");
        hovered.held = TOOLTIP_DELAY - 0.05;
        assert!(!hovered.tooltip_ready());
        hovered.held = TOOLTIP_DELAY + 0.01;
        assert!(hovered.tooltip_ready());
        hovered.clear();
        assert!(!hovered.tooltip_ready());
    }

    // ─ Cheap picking ───────────────────────────────────

    /// The narrowing must never reject a sprite it should have tested. The
    /// atlas cell is 24 x 48 anchored bottom-centre, so every texel a town
    /// sprite can possibly draw has to be inside the reach.
    #[test]
    fn the_cheap_reach_test_covers_the_whole_atlas_cell() {
        let base = Vec2::new(100.0, 60.0);
        for dx in -12..=12 {
            for dy in 0..=48 {
                let p = base + Vec2::new(dx as f32, dy as f32);
                assert!(
                    within_cell_reach(p, base),
                    "{p:?} is inside the cell at {base:?} but the reach test rejected it"
                );
            }
        }
    }

    /// And it must actually reject: a test that never says no is not a filter.
    #[test]
    fn the_cheap_reach_test_rejects_the_next_block_over() {
        let base = Vec2::new(100.0, 60.0);
        assert!(!within_cell_reach(base + Vec2::new(40.0, 0.0), base));
        assert!(!within_cell_reach(base - Vec2::new(40.0, 0.0), base));
        assert!(!within_cell_reach(base + Vec2::new(0.0, 96.0), base));
        assert!(!within_cell_reach(base - Vec2::new(0.0, 32.0), base));
    }

    #[test]
    fn a_still_pointer_is_not_a_new_pick() {
        let a = PickContext {
            cursor: Vec2::new(400.0, 300.0),
            camera: Vec2::ZERO,
            zoom: 1.0,
            window: SCREEN,
        };
        assert!(!a.differs_from(&a));
        let jitter = PickContext {
            cursor: a.cursor + Vec2::new(0.1, 0.1),
            ..a
        };
        assert!(!a.differs_from(&jitter), "sub-pixel jitter is not movement");
        for moved in [
            PickContext {
                cursor: a.cursor + Vec2::new(4.0, 0.0),
                ..a
            },
            PickContext {
                camera: Vec2::new(1.0, 0.0),
                ..a
            },
            PickContext { zoom: 2.0, ..a },
            PickContext {
                window: Vec2::new(800.0, 600.0),
                ..a
            },
        ] {
            assert!(a.differs_from(&moved), "{moved:?} should force a re-pick");
        }
    }

    // ─ Brackets ────────────────────────────────────────

    #[test]
    fn the_bracket_marks_the_corners_and_stays_inside_the_shape() {
        let rect = Rect::new(100.0, 40.0, 124.0, 88.0);
        let mut corners_touched = 0;
        for i in 0..8 {
            let (centre, size) = bracket_arm(i, rect);
            let half = size * 0.5;
            let arm = Rect::new(
                centre.x - half.x,
                centre.y - half.y,
                centre.x + half.x,
                centre.y + half.y,
            );
            assert!(
                arm.min.x >= rect.min.x - 0.01
                    && arm.max.x <= rect.max.x + 0.01
                    && arm.min.y >= rect.min.y - 0.01
                    && arm.max.y <= rect.max.y + 0.01,
                "arm {i} at {arm:?} hangs off the shape {rect:?}"
            );
            // One texel thin on its short axis: this is an outline, not a wash.
            assert!(size.x == 1.0 || size.y == 1.0, "arm {i} is {size:?}");
            let on_edge = arm.min.x <= rect.min.x + 1.0
                || arm.max.x >= rect.max.x - 1.0
                || arm.min.y <= rect.min.y + 1.0
                || arm.max.y >= rect.max.y - 1.0;
            assert!(on_edge, "arm {i} floats inside the shape");
            corners_touched += 1;
        }
        assert_eq!(corners_touched, 8);
    }

    #[test]
    fn a_tiny_shape_still_gets_a_bracket() {
        let rect = Rect::new(0.0, 0.0, 2.0, 2.0);
        for i in 0..8 {
            let (_, size) = bracket_arm(i, rect);
            assert!(size.x >= 1.0 && size.y >= 1.0, "arm {i} collapsed to {size:?}");
        }
    }

    #[test]
    fn buildings_lose_to_stations_and_beat_track() {
        // The ordering hover relies on: anything a click would rather have wins.
        assert!(PickPriority::Station <= PickPriority::Industry);
        assert!(PickPriority::Industry < PickPriority::Track);
    }

    // ─ Words ───────────────────────────────────────────

    /// Brief 05 §8.6: nothing in the world is a mystery the interface refuses
    /// to discuss. Bare ground included — it is most of what the player points
    /// at while they are deciding where to build.
    #[test]
    fn every_terrain_band_names_itself_in_ascii() {
        use rail_map::Tile;
        for kind in [
            TerrainKind::Water,
            TerrainKind::Beach,
            TerrainKind::Plains,
            TerrainKind::Hills,
            TerrainKind::Mountain,
        ] {
            let label = terrain_label(kind);
            assert!(!label.is_empty() && label.is_ascii(), "{kind:?} -> {label:?}");
            let cell = Tile {
                height: 3,
                water: kind == TerrainKind::Water,
                kind,
            };
            let detail = ground_detail(&cell);
            assert!(!detail.is_empty() && detail.is_ascii(), "{kind:?} -> {detail:?}");
        }
    }

    #[test]
    fn water_tells_the_player_it_needs_a_bridge() {
        use rail_map::Tile;
        let wet = Tile {
            height: -1,
            water: true,
            kind: TerrainKind::Water,
        };
        assert_eq!(ground_detail(&wet), "Track needs a bridge");
        let dry = Tile {
            height: 4,
            water: false,
            kind: TerrainKind::Hills,
        };
        assert_eq!(ground_detail(&dry), "Height 4 - open ground");
    }

    /// The chip is drawn in the shipped font, which has no glyphs beyond ASCII.
    #[test]
    fn train_status_words_are_ascii() {
        for status in ["parked", "loading", "idle", "running"] {
            assert!(status.is_ascii());
        }
        let mut loc = TrainLocation::at_track(rail_sim::TrackId(1));
        assert_eq!(train_status(&loc), "idle");
        loc.dwell_remaining = 3;
        assert_eq!(train_status(&loc), "loading");
        loc.parked = true;
        assert_eq!(train_status(&loc), "parked");
    }
}

/// Cost of the town scan, measured rather than asserted.
///
/// ```text
/// cargo test -p rail_town --release hover_scan -- --ignored --nocapture
/// ```
///
/// Ignored because a timing number is not a pass/fail condition — a loaded
/// machine would make it flap. It is here so the claim in the module docs
/// ("a handful of entities out of thousands") can be re-checked rather than
/// taken on trust, and so the next person to touch [`pick_town`] can see what
/// they moved.
#[cfg(test)]
mod scan_cost {
    use super::*;

    /// A stand-in for [`BuildingAtlas`]'s baked bounds table, whose fields are
    /// private. The arithmetic below is `BuildingAtlas::frame_rect` verbatim,
    /// so the measured inner loop is the real one.
    struct Bounds(Vec<[i32; 4]>);

    impl Bounds {
        fn rect(&self, frame: usize, base: Vec2) -> Option<Rect> {
            let b = self.0.get(frame)?;
            let origin = base.x - 24.0 / 2.0;
            Some(Rect::new(
                origin + b[0] as f32,
                base.y + b[1] as f32,
                origin + b[2] as f32 + 1.0,
                base.y + b[3] as f32 + 1.0,
            ))
        }
    }

    /// One scan over `sprites`, with the cheap reach rejection on or off.
    ///
    /// `narrow: false` is the loop as it stood before this change: every entity
    /// paid for a bounds lookup and a rectangle.
    fn scan(point: Vec2, sprites: &[(f32, Vec2, usize)], bounds: &Bounds, narrow: bool) -> usize {
        let mut hits = 0usize;
        let mut best_z = f32::NEG_INFINITY;
        for &(z, base, frame) in sprites {
            if narrow {
                if !within_cell_reach(point, base) {
                    continue;
                }
                if z <= best_z {
                    continue;
                }
            }
            let Some(rect) = bounds.rect(frame, base) else {
                continue;
            };
            if rect.contains(point) && z > best_z {
                best_z = z;
                hits += 1;
            }
        }
        hits
    }

    /// A town's worth of sprites, laid out four to a 32-texel tile the way
    /// `town::lots::lot_base` does.
    fn town(tiles: i32) -> Vec<(f32, Vec2, usize)> {
        let mut out = Vec::new();
        for ty in 0..tiles {
            for tx in 0..tiles {
                for slot in 0..4 {
                    let base = Vec2::new(
                        (tx * 32 + 8 + (slot % 2) * 16) as f32,
                        (ty * 32 + 6 + (slot / 2) * 16) as f32,
                    );
                    out.push((base.y * -0.001, base, (slot as usize * 7) % 32));
                }
            }
        }
        out
    }

    #[test]
    #[ignore = "timing measurement, not a pass/fail condition"]
    fn hover_scan_cost() {
        let bounds = Bounds((0..64).map(|i| [4, 0, 19, 20 + (i as i32 % 8)]).collect());
        for tiles in [16, 32, 48] {
            let sprites = town(tiles);
            // Somewhere in the middle, so the scan cannot short-circuit early.
            let point = Vec2::new(tiles as f32 * 16.0 + 9.0, tiles as f32 * 16.0 + 7.0);
            let reps = 200;

            let mut timings = Vec::new();
            for narrow in [false, true] {
                // Warm the caches so the first mode does not pay for both.
                scan(point, &sprites, &bounds, narrow);
                let started = std::time::Instant::now();
                let mut sink = 0usize;
                for _ in 0..reps {
                    sink += scan(point, &sprites, &bounds, narrow);
                }
                assert!(sink < usize::MAX);
                timings.push(started.elapsed().as_secs_f64() * 1e6 / reps as f64);
            }
            println!(
                "{:>6} sprites  full {:7.1} us/scan   narrowed {:7.1} us/scan   {:5.1}x",
                sprites.len(),
                timings[0],
                timings[1],
                timings[0] / timings[1].max(f64::EPSILON),
            );
        }
    }
}

/// Wiring checks against a real (headless) app, so a system-param clash or a
/// missing resource fails here rather than on the first frame of the game.
#[cfg(test)]
mod app_tests {
    use super::*;
    use bevy::MinimalPlugins;
    use rail_map::generate_map;

    /// The one Bevy behaviour this whole module is built on, pinned against
    /// the real layout engine rather than against a reading of its source.
    ///
    /// `bevy_ui::update::propagate_ui_target_cameras` computes the layout scale
    /// factor as `camera.target_scaling_factor() * UiScale`, and
    /// `bevy_ui::layout::convert` turns `Val::Px(v)` into `scale_factor * v`
    /// taffy units, which are physical pixels. So:
    ///
    /// ```text
    /// physical = Val::Px(v) * window_scale_factor * UiScale
    /// logical  = Val::Px(v) * UiScale
    /// ```
    ///
    /// That second line is the whole bug. `Camera::world_to_viewport` and
    /// `Window::width()` speak *logical* pixels; `Node::left` does not. If this
    /// test ever fails, [`tooltip_anchor`] is converting for a Bevy that no
    /// longer works this way, and every window position in `ui::window` is
    /// wrong in the same direction.
    ///
    /// The camera is built the way `bevy_ui`'s own layout tests build theirs:
    /// a `RenderTargetInfo` filled in by hand, since nothing here runs a
    /// renderer to fill it in for us.
    #[test]
    fn bevy_multiplies_val_px_by_ui_scale() {
        use bevy::camera::{Camera, ComputedCameraValues, RenderTargetInfo};

        const WINDOW_SCALE: f32 = 2.0;
        const OFFSET: f32 = 100.0;
        const PHYSICAL: UVec2 = UVec2::new(1280, 720);

        for ui_scale in [1.0_f32, 2.0, 3.0, 4.0] {
            let mut app = App::new();
            // The layout half of `UiPlugin`, wired by hand exactly as
            // `bevy_ui`'s own layout tests wire it. The rest of `UiPlugin` is
            // focus and picking, which need a window and an input pipeline and
            // have nothing to do with where a `Val::Px` lands.
            app.add_plugins((
                bevy::app::HierarchyPropagatePlugin::<ComputedUiTargetCamera>::new(PostUpdate),
                bevy::app::HierarchyPropagatePlugin::<ComputedUiRenderTargetInfo>::new(PostUpdate),
            ));
            app.init_resource::<bevy::ui::ui_surface::UiSurface>()
                .init_resource::<bevy::text::TextPipeline>()
                .init_resource::<bevy::text::CosmicFontSystem>()
                .init_resource::<bevy::text::SwashCache>()
                .insert_resource(UiScale(ui_scale));
            app.add_systems(
                PostUpdate,
                (
                    bevy::ui::update::propagate_ui_target_cameras,
                    bevy::ui::ui_layout_system,
                )
                    .chain(),
            );
            app.configure_sets(
                PostUpdate,
                (
                    bevy::app::PropagateSet::<ComputedUiTargetCamera>::default(),
                    bevy::app::PropagateSet::<ComputedUiRenderTargetInfo>::default(),
                )
                    .after(bevy::ui::update::propagate_ui_target_cameras)
                    .before(bevy::ui::ui_layout_system),
            );

            app.world_mut().spawn((
                Camera2d,
                Camera {
                    computed: ComputedCameraValues {
                        target_info: Some(RenderTargetInfo {
                            physical_size: PHYSICAL,
                            scale_factor: WINDOW_SCALE,
                        }),
                        ..default()
                    },
                    ..default()
                },
                bevy::ui::IsDefaultUiCamera,
            ));

            let node = app
                .world_mut()
                .spawn(Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(OFFSET),
                    top: Val::Px(OFFSET),
                    width: Val::Px(40.0),
                    height: Val::Px(20.0),
                    ..default()
                })
                .id();

            app.update();

            let world = app.world();
            let computed = world.get::<ComputedNode>(node).expect("laid out");
            let global = world
                .get::<UiGlobalTransform>(node)
                .expect("laid out")
                .translation;
            let top_left = global - computed.size * 0.5;

            let expected = OFFSET * WINDOW_SCALE * ui_scale;
            assert!(
                (top_left.x - expected).abs() <= 0.5 && (top_left.y - expected).abs() <= 0.5,
                "at UiScale {ui_scale} a Val::Px({OFFSET}) node drew at physical {top_left:?}, \
                 expected {expected} on both axes"
            );
            // And the inverse the tooltip relies on: ui px -> logical px.
            let logical = top_left / WINDOW_SCALE;
            assert!(
                (logical.x - OFFSET * ui_scale).abs() <= 0.5,
                "logical position {logical:?} is not Val::Px * UiScale at {ui_scale}"
            );
            // `ComputedNode` reports physical size; the inverse scale factor
            // that `tooltip_size` divides by must undo *both* factors.
            assert!(
                (computed.inverse_scale_factor() - 1.0 / (WINDOW_SCALE * ui_scale)).abs() < 1e-4,
                "inverse_scale_factor {} does not undo window scale and UiScale together",
                computed.inverse_scale_factor()
            );
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(generate_map(16, 16, 7));
        app.init_resource::<MapViewState>();
        app.init_resource::<UiBlocksWorld>();
        app.init_resource::<UiScale>();
        app.init_resource::<TrackNetwork>();
        app.init_resource::<StationRegistry>();
        app.init_resource::<StationService>();
        app.init_resource::<IndustryRegistry>();
        app.init_resource::<Hovered>();
        app.init_resource::<HoverProbe>();
        app.add_systems(Startup, setup_hover);
        app.add_systems(
            Update,
            (
                hover_pick,
                sync_hover_brackets.after(hover_pick),
                update_hover_tooltip.after(hover_pick),
            ),
        );
        app
    }

    #[test]
    fn the_hover_systems_run_without_a_pointer() {
        let mut app = test_app();
        for _ in 0..4 {
            app.update();
        }
        assert!(
            app.world().resource::<Hovered>().target.is_none(),
            "nothing is hovered when there is no window to point at"
        );
    }

    #[test]
    fn the_bracket_starts_hidden_and_has_eight_arms() {
        let mut app = test_app();
        app.update();
        let mut q = app.world_mut().query::<(&HoverBracket, &Visibility)>();
        let arms: Vec<Visibility> = q.iter(app.world()).map(|(_, v)| *v).collect();
        assert_eq!(arms.len(), 8, "four corners, two arms each");
        assert!(arms.iter().all(|v| *v == Visibility::Hidden));
    }

    #[test]
    fn the_chip_starts_hidden() {
        let mut app = test_app();
        app.update();
        let mut q = app.world_mut().query_filtered::<&Node, With<HoverTooltipRoot>>();
        let node = q.single(app.world()).expect("one chip root");
        assert_eq!(node.display, Display::None);
    }

    /// Without a pointer there is nothing to re-pick, so the gate must not be
    /// spending picks on empty frames either.
    #[test]
    fn a_pointerless_app_takes_no_picks() {
        let mut app = test_app();
        for _ in 0..30 {
            app.update();
        }
        let probe = app.world().resource::<HoverProbe>();
        assert_eq!(probe.picks, 0, "picked {} times with no window", probe.picks);
        assert!(probe.frames >= 30);
    }
}
