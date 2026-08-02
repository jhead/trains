//! Hover — the middle tier of interrogation.
//!
//! Brief 05 §1 lays out three tiers, each cheaper than the last: **ambient**
//! (free), **hover** (a moment) and **select** (a click). Hover was missing
//! entirely, which left the world in the state the playtest described — *"I'm
//! not sure what they are or mean … there's no hover state."*
//!
//! What hover owes the player is small and specific: *what is this, and what is
//! its headline number?* So this module does two things and no more.
//!
//! - **Highlight.** Corner brackets in `railL` hugging the object's baked
//!   silhouette. Selection keeps `railS`, the brightest value in the palette
//!   (brief 05 §2), so a hovered thing never looks selected; and a build ghost
//!   keeps `hi` (brief 01 §3.3), so hover never fights the accent that owns the
//!   screen during a build.
//! - **Tooltip.** After 400 ms (brief 03 §8.3), a `bg0` panel with a 1-texel
//!   outline naming the thing and giving one fact about it. It is placed
//!   **outside the screen rectangle of the tile under the cursor**, flipping
//!   side or above as the window edge demands — covering that tile is exactly
//!   what you cannot do while somebody is trying to build on it.
//!
//! Picking is [`super::selection::pick_world`], the same test a click uses, so
//! hovering something and clicking it can never disagree. Buildings and
//! countryside are not yet [`Selectable`], so they are resolved here, below
//! stations and industries and above track.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rail_map::{world_to_tile, MapGrid, TILE_SIZE};
use rail_sim::{
    commands::TrainKind, IndustryRegistry, Mood, Peep, StationRegistry, TileCoord, TrackNetwork,
    Train,
};

use crate::map::{MapCamera, MapViewState};
use crate::palette::{BG0, OUTLINE, RAIL_L};
use crate::town::{
    lot_condition, lot_label, BuildingAtlas, BuildingLot, District, RuralKind, RuralProp,
};
use crate::ui::kit::{body_font, micro_font, text_primary, text_secondary, SPACE_2};
use crate::ui::UiBlocksWorld;

use super::pick::{PickPriority, Selectable};
use super::selection::{pick_world, WorldPickSprites};

/// Dwell before the tooltip appears (brief 03 §8.3).
pub const TOOLTIP_DELAY: f32 = 0.4;

/// Gap between the cursor's tile and the tooltip, in window pixels.
const TOOLTIP_GAP: f32 = 8.0;

/// Length of one arm of a corner bracket, in world texels.
const BRACKET_ARM: f32 = 5.0;

/// The hover ring sits above the world and below the build ghost, which owns
/// `hi` while it is showing.
const BRACKET_Z: f32 = 3.0;

/// What the pointer is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoverTarget {
    /// A building lot, at whatever stage of its life.
    Lot(Entity),
    /// A farm, a field, a wall, a tree.
    Rural(Entity),
    /// Anything a click can already select.
    World(Selectable),
}

/// Current hover state. One resource, read by the bracket and tooltip systems.
#[derive(Resource, Debug, Default)]
pub struct Hovered {
    pub target: Option<HoverTarget>,
    /// Seconds the pointer has rested on this exact target.
    pub held: f32,
    /// World-space rectangle of the thing, in texels (min, max).
    pub bounds: Option<Rect>,
    /// Screen rectangle of the tile under the cursor — the one area the
    /// tooltip may never cover.
    pub tile_rect: Option<Rect>,
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
        self.tile_rect = None;
        self.title.clear();
        self.detail.clear();
    }

    /// True once the pointer has rested long enough for the tooltip.
    pub fn tooltip_ready(&self) -> bool {
        self.target.is_some() && self.held >= TOOLTIP_DELAY && !self.title.is_empty()
    }
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
    industries: Res<'w, IndustryRegistry>,
    trains: Query<'w, 's, &'static Train>,
    sprites: WorldPickSprites<'w, 's>,
}

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
                padding: UiRect::all(Val::Px(SPACE_2)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::ZERO,
                display: Display::None,
                ..default()
            },
            BackgroundColor(BG0),
            BorderColor::all(OUTLINE),
            // No `Interaction` and no `WorldClickBlocker`: a tooltip that ate
            // clicks would make the thing it describes unclickable.
            ZIndex(30),
        ))
        .with_children(|root| {
            root.spawn((
                HoverTooltipTitle,
                Text::new(""),
                body_font(),
                text_primary(),
            ));
            root.spawn((
                HoverTooltipDetail,
                Text::new(""),
                micro_font(),
                text_secondary(),
            ));
        });
}

// ─ Picking ─────────────────────────────────────────────

/// Resolve what the pointer is over, and how long it has been there.
#[allow(clippy::too_many_arguments)]
pub fn hover_pick(
    time: Res<Time<Real>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MapCamera>>,
    world: HoverWorld,
    atlas: Option<Res<BuildingAtlas>>,
    lots: Query<(Entity, &BuildingLot, &Sprite, &Transform)>,
    props: Query<(Entity, &RuralProp, &Sprite, &Transform)>,
    peeps: Query<&Peep>,
    mut hovered: ResMut<Hovered>,
) {
    let dt = time.delta_secs();

    // Map View owns the pointer, and chrome under it is not the world.
    if world.map_view.active || world.ui_blocks.0 {
        hovered.clear();
        return;
    }
    let Ok(window) = windows.single() else {
        hovered.clear();
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        hovered.clear();
        return;
    };
    let Ok((camera, cam_gt)) = camera_q.single() else {
        hovered.clear();
        return;
    };
    let Ok(point) = camera.viewport_to_world_2d(cam_gt, cursor) else {
        hovered.clear();
        return;
    };
    let tile = world_to_tile(point.x, point.y);
    if !world.map.contains(tile) {
        hovered.clear();
        return;
    }

    // Buildings and countryside first, because they are what the map is mostly
    // made of; a station or an industry sitting on the same texels still wins.
    let town = pick_town(point, &atlas, &lots, &props);
    let clicked = pick_world(
        point,
        tile,
        &world.network,
        &world.stations,
        &world.industries,
        &world.sprites,
    );

    let (target, bounds) = match (clicked, town) {
        (Some(sel), _) if sel.priority() <= PickPriority::Industry => {
            (Some(HoverTarget::World(sel)), None)
        }
        (_, Some((target, rect))) => (Some(target), Some(rect)),
        (Some(sel), None) => (Some(HoverTarget::World(sel)), None),
        (None, None) => (None, None),
    };

    if target != hovered.target {
        hovered.target = target;
        hovered.held = 0.0;
        let (title, detail) = describe(target, &world, &lots, &props, &peeps);
        hovered.title = title;
        hovered.detail = detail;
    } else if target.is_some() {
        hovered.held += dt;
    }

    hovered.bounds = match (target, bounds) {
        (Some(_), Some(rect)) => Some(rect),
        (Some(HoverTarget::World(sel)), None) => {
            world_bounds(sel, &world, tile).or_else(|| Some(tile_world_rect(tile)))
        }
        _ => None,
    };
    hovered.tile_rect = tile_screen_rect(tile, camera, cam_gt);
}

/// Hit-test buildings and countryside, nearest (most southerly) first.
fn pick_town(
    point: Vec2,
    atlas: &Option<Res<BuildingAtlas>>,
    lots: &Query<(Entity, &BuildingLot, &Sprite, &Transform)>,
    props: &Query<(Entity, &RuralProp, &Sprite, &Transform)>,
) -> Option<(HoverTarget, Rect)> {
    let atlas = atlas.as_ref()?;
    let mut best: Option<(f32, HoverTarget, Rect)> = None;

    let mut consider = |z: f32, target: HoverTarget, rect: Rect| {
        if !rect.contains(point) {
            return;
        }
        // Whatever draws in front is what the player thinks they are pointing at.
        if best.as_ref().is_none_or(|(bz, _, _)| z > *bz) {
            best = Some((z, target, rect));
        }
    };

    for (entity, _, sprite, tf) in lots.iter() {
        if let Some(rect) = sprite_bounds(atlas, sprite, tf) {
            consider(tf.translation.z, HoverTarget::Lot(entity), rect);
        }
    }
    for (entity, _, sprite, tf) in props.iter() {
        if let Some(rect) = sprite_bounds(atlas, sprite, tf) {
            consider(tf.translation.z, HoverTarget::Rural(entity), rect);
        }
    }

    best.map(|(_, target, rect)| (target, rect))
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
fn world_bounds(sel: Selectable, world: &HoverWorld, tile: TileCoord) -> Option<Rect> {
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
        // Peeps and trains move every frame; their sprite transform is the
        // truth and picking already matched against it.
        Selectable::Peep(_) | Selectable::Train(_) => None,
    }
}

/// Screen rectangle of a tile, for keeping the tooltip off it.
fn tile_screen_rect(tile: TileCoord, camera: &Camera, cam_gt: &GlobalTransform) -> Option<Rect> {
    let world = tile_world_rect(tile);
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
    lots: &Query<(Entity, &BuildingLot, &Sprite, &Transform)>,
    props: &Query<(Entity, &RuralProp, &Sprite, &Transform)>,
    peeps: &Query<&Peep>,
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
                    RuralKind::Farmstead => "Open country - no station in reach".into(),
                    RuralKind::Prop(_) => "Open country".into(),
                },
            ),
            Err(_) => (String::new(), String::new()),
        },
        Some(HoverTarget::World(sel)) => describe_world(sel, world, peeps),
    }
}

/// A building's headline: who lives there, or what is happening to it.
fn lot_detail(lot: &BuildingLot, peeps: &Query<&Peep>) -> String {
    let district = lot.district.label();
    if let Some(condition) = lot_condition(lot) {
        return format!("{district} - {condition}");
    }
    if lot.district == District::Industrial {
        return format!("{district} - goods");
    }
    let residents = peeps.iter().filter(|p| p.home == lot.tile).count();
    match residents {
        0 => format!("{district} - empty"),
        1 => format!("{district} - 1 resident"),
        n => format!("{district} - {n} residents"),
    }
}

fn describe_world(sel: Selectable, world: &HoverWorld, peeps: &Query<&Peep>) -> (String, String) {
    match sel {
        Selectable::Station(id) => match world.stations.get(id) {
            Some(s) => (s.name.clone(), format!("{} station", s.tier.label())),
            None => ("Station".into(), String::new()),
        },
        Selectable::Industry(id) => match world.industries.get(id) {
            Some(i) => {
                let detail = match (i.produces, i.consumes) {
                    (Some(p), _) => format!("Produces {}", p.label()),
                    (None, Some(c)) => format!("Takes {}", c.label()),
                    (None, None) => "Industry".into(),
                };
                (i.name.clone(), detail)
            }
            None => ("Industry".into(), String::new()),
        },
        Selectable::Track(_) => ("Track".into(), "Click to inspect the run".into()),
        Selectable::Train(id) => {
            let kind = world
                .trains
                .iter()
                .find(|t| t.id == id)
                .map(|t| match t.kind {
                    TrainKind::Transit => "Transit",
                    TrainKind::Transport => "Transport",
                })
                .unwrap_or("Rolling stock");
            ("Train".into(), format!("{kind} service"))
        }
        Selectable::Peep(id) => match peeps.iter().find(|p| p.id == id) {
            Some(p) => (
                p.name.clone(),
                match p.mood {
                    Mood::Content => "Content".into(),
                    Mood::Uneasy => "Uneasy".into(),
                    Mood::Frustrated => "Frustrated".into(),
                },
            ),
            None => ("Someone".into(), String::new()),
        },
    }
}

// ─ Highlight ───────────────────────────────────────────

/// Corner brackets around whatever the pointer is over.
pub fn sync_hover_brackets(
    hovered: Res<Hovered>,
    mut brackets: Query<(&HoverBracket, &mut Transform, &mut Sprite, &mut Visibility)>,
) {
    let Some(rect) = hovered.bounds else {
        for (_, _, _, mut vis) in brackets.iter_mut() {
            *vis = Visibility::Hidden;
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
        *vis = Visibility::Visible;
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

// ─ Tooltip ─────────────────────────────────────────────

/// Show, fill and place the tooltip. Never over the tile under the cursor.
pub fn update_hover_tooltip(
    hovered: Res<Hovered>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut root: Query<(&mut Node, &ComputedNode), With<HoverTooltipRoot>>,
    mut title: Query<&mut Text, (With<HoverTooltipTitle>, Without<HoverTooltipDetail>)>,
    mut detail: Query<&mut Text, (With<HoverTooltipDetail>, Without<HoverTooltipTitle>)>,
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

    if let Ok(mut text) = title.single_mut() {
        if text.0 != hovered.title {
            text.0 = hovered.title.clone();
        }
    }
    if let Ok(mut text) = detail.single_mut() {
        if text.0 != hovered.detail {
            text.0 = hovered.detail.clone();
        }
    }
    node.display = Display::Flex;

    let Ok(window) = windows.single() else {
        return;
    };
    let Some(tile_rect) = hovered.tile_rect else {
        return;
    };
    let size = tooltip_size(computed, &hovered.title, &hovered.detail);
    let placed = place_tooltip(
        tile_rect,
        size,
        Vec2::new(window.width(), window.height()),
    );
    node.left = Val::Px(placed.x);
    node.top = Val::Px(placed.y);
}

/// Measured size once layout has run, estimated on the first frame.
///
/// `ComputedNode::size` is physical pixels; `Node::left` / `top` are logical,
/// hence the scale-factor conversion.
fn tooltip_size(computed: &ComputedNode, title: &str, detail: &str) -> Vec2 {
    let measured = computed.size * computed.inverse_scale_factor();
    if measured.x > 1.0 && measured.y > 1.0 {
        return measured;
    }
    let chars = title.chars().count().max(detail.chars().count()) as f32;
    Vec2::new(chars * 8.0 + SPACE_2 * 2.0, 42.0)
}

/// Place a `size` box beside `tile_rect` without covering it, and without
/// leaving `screen`.
///
/// Brief 03 §8.3 is explicit that a tooltip must never cover the tile under the
/// cursor — while building, that tile is the whole point of the gesture. So the
/// tile's own rectangle is the obstacle, not the cursor hotspot.
pub fn place_tooltip(tile_rect: Rect, size: Vec2, screen: Vec2) -> Vec2 {
    let right = tile_rect.max.x + TOOLTIP_GAP;
    let left = tile_rect.min.x - TOOLTIP_GAP - size.x;
    let x = if right + size.x <= screen.x - TOOLTIP_GAP {
        right
    } else if left >= TOOLTIP_GAP {
        left
    } else {
        // Neither side fits: sit clear of the tile vertically instead.
        (screen.x - size.x - TOOLTIP_GAP).max(TOOLTIP_GAP)
    };

    let below = tile_rect.max.y + TOOLTIP_GAP;
    let above = tile_rect.min.y - TOOLTIP_GAP - size.y;
    let clear_horizontally = x >= tile_rect.max.x || x + size.x <= tile_rect.min.x;
    let y = if clear_horizontally {
        // Free to sit level with the tile.
        (tile_rect.min.y).clamp(TOOLTIP_GAP, (screen.y - size.y - TOOLTIP_GAP).max(TOOLTIP_GAP))
    } else if below + size.y <= screen.y - TOOLTIP_GAP {
        below
    } else {
        above.max(TOOLTIP_GAP)
    };

    Vec2::new(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Vec2 = Vec2::new(1280.0, 720.0);

    fn tile_at(x: f32, y: f32) -> Rect {
        Rect::new(x, y, x + 64.0, y + 64.0)
    }

    fn overlaps(a: Rect, b: Rect) -> bool {
        a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
    }

    #[test]
    fn the_tooltip_never_covers_the_tile_under_the_cursor() {
        let size = Vec2::new(180.0, 44.0);
        // Sweep the whole screen, including all four corners.
        for gx in 0..20 {
            for gy in 0..12 {
                let tile = tile_at(gx as f32 * 64.0, gy as f32 * 64.0);
                let at = place_tooltip(tile, size, SCREEN);
                let placed = Rect::new(at.x, at.y, at.x + size.x, at.y + size.y);
                assert!(
                    !overlaps(placed, tile),
                    "tooltip {placed:?} covers the tile {tile:?} under the cursor"
                );
            }
        }
    }

    #[test]
    fn the_tooltip_stays_on_screen() {
        let size = Vec2::new(220.0, 44.0);
        for gx in 0..20 {
            for gy in 0..12 {
                let tile = tile_at(gx as f32 * 64.0, gy as f32 * 64.0);
                let at = place_tooltip(tile, size, SCREEN);
                assert!(at.x >= 0.0 && at.y >= 0.0, "tooltip left the screen at {at:?}");
                assert!(
                    at.x + size.x <= SCREEN.x && at.y + size.y <= SCREEN.y,
                    "tooltip {at:?} + {size:?} overflows {SCREEN:?}"
                );
            }
        }
    }

    #[test]
    fn the_tooltip_flips_side_rather_than_running_off_the_edge() {
        let size = Vec2::new(200.0, 44.0);
        let near_right = tile_at(SCREEN.x - 80.0, 300.0);
        let at = place_tooltip(near_right, size, SCREEN);
        assert!(
            at.x + size.x <= near_right.min.x,
            "against the right edge the tooltip must flip to the left"
        );
    }

    #[test]
    fn hover_waits_before_it_speaks() {
        let mut hovered = Hovered {
            target: Some(HoverTarget::World(Selectable::Track(rail_sim::TrackId(1)))),
            title: "Track".into(),
            ..Default::default()
        };
        assert!(!hovered.tooltip_ready(), "a tooltip must not appear instantly");
        hovered.held = TOOLTIP_DELAY - 0.05;
        assert!(!hovered.tooltip_ready());
        hovered.held = TOOLTIP_DELAY + 0.01;
        assert!(hovered.tooltip_ready());
        hovered.clear();
        assert!(!hovered.tooltip_ready());
    }

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
}

/// Wiring checks against a real (headless) app, so a system-param clash or a
/// missing resource fails here rather than on the first frame of the game.
#[cfg(test)]
mod app_tests {
    use super::*;
    use bevy::MinimalPlugins;
    use rail_map::generate_map;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(generate_map(16, 16, 7));
        app.init_resource::<MapViewState>();
        app.init_resource::<UiBlocksWorld>();
        app.init_resource::<TrackNetwork>();
        app.init_resource::<StationRegistry>();
        app.init_resource::<IndustryRegistry>();
        app.init_resource::<Hovered>();
        app.add_systems(Startup, setup_hover);
        app.add_systems(
            Update,
            (hover_pick, sync_hover_brackets.after(hover_pick), update_hover_tooltip),
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
    fn the_tooltip_starts_hidden() {
        let mut app = test_app();
        app.update();
        let mut q = app.world_mut().query_filtered::<&Node, With<HoverTooltipRoot>>();
        let node = q.single(app.world()).expect("one tooltip root");
        assert_eq!(node.display, Display::None);
    }
}
