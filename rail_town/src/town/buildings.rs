//! The town, drawn as buildings on lots — and as events the player watches.
//!
//! Density from [`TownDensity`] is not a bar height. A tile is a **block** of
//! four **lots**, and density decides how many of them are taken up and how
//! tall they have grown (brief 06 §2). Every change is staged as something
//! visible: a stake, a scaffold that holds for eight seconds, a two-frame
//! settle — and on the way down, dark windows, boards, dereliction, and finally
//! a cleared lot whose foundation scar stays (§3). Every decline stage reverses
//! visibly when service comes back, because the design promises decline is
//! recoverable *at any point* and the player has to see it happen.
//!
//! Art is procedural: [`building_art`] bakes one nearest-sampled atlas at boot
//! from the binding palette ramps. Nothing here rotates, nothing scales
//! fractionally, positions are whole texels, and southerly buildings draw in
//! front (brief 01 §2 and §6.1).
//!
//! # Module layout
//!
//! The sibling files are declared here rather than in `town/mod.rs` so that
//! registering this feature stays a one-line change in a file other slices also
//! edit. They can be lifted into `mod.rs` verbatim later.

#[path = "building_art.rs"]
pub mod building_art;
#[path = "districts.rs"]
pub mod districts;
#[path = "lots.rs"]
pub mod lots;

use std::collections::HashMap;

use bevy::image::TextureAtlasLayout;
use bevy::prelude::*;
use bevy::sprite::Anchor;
use rail_map::MapGrid;
use rail_sim::{
    IndustryRegistry, SimClock, StationRegistry, TileCoord, TownDensity, TrackNetwork,
    GROUND_LAYER,
};

use building_art::{
    bake_atlas, world_hash, BuildingAtlas, BuildingKind, Decay, FRAME_RURAL, FRAME_SCAFFOLD,
    FRAME_SCAR, FRAME_STAKE,
};
use districts::{classify, nearest_good, District};
use lots::{
    fill_order, lot_base, lot_flip, lots_wanted, plan_kind, rural_farmstead, rural_prop,
    rural_slot, LOTS_PER_TILE, TILE_TEXELS,
};

// ─ Timings ─────────────────────────────────────────────
//
// Brief 06 §3.1: eight seconds is long enough to notice and short enough not to
// be tedious. Decline is slower still, because it must be legible and gradual.

/// Surveyor's stake — tiny, easy to miss, and the first hint.
pub const STAKE_SECS: f32 = 1.6;
/// Scaffold hold. The number the brief names.
pub const SCAFFOLD_SECS: f32 = 8.0;
/// One frame of the two-frame settle.
pub const SETTLE_SECS: f32 = 0.13;
/// How long a healthy building holds before its windows go dark.
pub const DECLINE_ONSET_SECS: f32 = 4.0;
/// Hold on each further decline stage.
pub const DECLINE_STEP_SECS: f32 = 11.0;
/// Hold on dereliction before the lot is cleared.
pub const CLEARED_AFTER_SECS: f32 = 15.0;
/// Hold on each step back toward health when service returns.
pub const RECOVERY_STEP_SECS: f32 = 2.5;
/// Pause on a cleared lot before rebuilding starts.
pub const REBUILD_DELAY_SECS: f32 = 1.0;

// ─ Depth band ──────────────────────────────────────────
//
// Layer bands (brief 01 §6.1): terrain → track → **buildings** → peeps →
// trains. Track sits at 1.0, peeps and stations at 2.0, so buildings own the
// gap and Y-sort inside it: further south draws in front.

const BUILDING_Z_BASE: f32 = 1.10;
const BUILDING_Z_SPAN: f32 = 0.80;

/// Depth for a building whose base sits at world row `base_y`.
pub fn lot_z(base_y: i32, map_height_tiles: u32) -> f32 {
    let span = (map_height_tiles.max(1) as i32 * TILE_TEXELS) as f32;
    let t = (base_y as f32 / span).clamp(0.0, 1.0);
    BUILDING_Z_BASE + BUILDING_Z_SPAN * (1.0 - t)
}

// ─ Components ──────────────────────────────────────────

/// Where a lot is in the construction / decline sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LotPhase {
    /// A stake in the ground; nothing built yet.
    Stake,
    /// Scaffold up, holding.
    Scaffold,
    /// First frame of the two-frame settle — the building drops into place.
    Settle,
    /// Standing, at some stage of decline.
    Standing(Decay),
    /// Cleared. The foundation scar persists.
    Cleared,
}

impl LotPhase {
    /// True while nothing has been built here yet, so abandoning is silent.
    pub fn is_preconstruction(self) -> bool {
        matches!(self, Self::Stake | Self::Scaffold)
    }
}

/// One building lot inside a tile's block.
#[derive(Component, Debug, Clone)]
pub struct BuildingLot {
    pub tile: TileCoord,
    /// `0..4` — lot within the block, `slot & 1` east, `slot >> 1` north.
    pub slot: u8,
    pub district: District,
    /// What stands (or is going up) here.
    pub kind: BuildingKind,
    /// What the sim wants here now; `None` means the lot should empty out.
    pub target: Option<BuildingKind>,
    pub phase: LotPhase,
    /// Seconds spent in the current phase.
    pub elapsed: f32,
}

/// Window metadata for the night-lighting layer.
///
/// The lit-windows pass is a **separate sprite layer** (brief 01 §3.4) and does
/// not live here. It should query `(&Transform, &BuildingWindows)`, and for any
/// lot whose `lit_frame` is `Some`, draw [`BuildingAtlas::image`] /
/// [`BuildingAtlas::layout`] at that index with the same transform, the same
/// [`Anchor::BOTTOM_CENTER`], the same `flip_x`, and a small +z. The mask frame
/// holds exactly this building's window texels, so it lines up by construction.
/// `lit_frame` is `None` whenever the building must not light: under
/// construction, cleared, or at any stage of decline from `Dimmed` down —
/// windows going dark *is* the first decline signal.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildingWindows {
    /// Atlas frame holding this building's windows in `WIN_LIT`, when lit.
    pub lit_frame: Option<usize>,
    /// Mirror flag the lighting sprite must match.
    pub flip_x: bool,
}

/// What a countryside object is — for hover, and for the frame it draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuralKind {
    /// A working farm: the anchor its fields are arranged around.
    Farmstead,
    /// Worked ground or a landmark, indexed into [`FRAME_RURAL`].
    Prop(usize),
}

impl RuralKind {
    /// Plain name for the hover tooltip.
    pub fn label(self) -> &'static str {
        match self {
            Self::Farmstead => "Farmstead",
            Self::Prop(i) => building_art::rural_label(i),
        }
    }
}

/// A countryside object on unserved land.
#[derive(Component, Debug, Clone, Copy)]
pub struct RuralProp {
    pub tile: TileCoord,
    pub kind: RuralKind,
}

// ─ Plugin ──────────────────────────────────────────────

/// Registers the town building presentation.
///
/// `town/mod.rs` may either keep its existing `sync_building_sprites`
/// registration or add this plugin — they do the same thing.
#[allow(dead_code)] // Optional registration path; `mod.rs` currently calls the system directly.
pub struct TownBuildingsPlugin;

impl Plugin for TownBuildingsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, sync_building_sprites);
    }
}

// ─ Cached layout ───────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
struct BlockPlan {
    district: District,
    lots: u8,
    kinds: [Option<BuildingKind>; LOTS_PER_TILE as usize],
}

/// Per-system bake cache. Lives in a `Local` so registering this feature needs
/// no resource init in the shared `town/mod.rs`.
///
/// A `Local` outlives the world it was filled for, which is the whole reason
/// [`TownState::world`] exists — see [`world_id`].
#[derive(Default)]
pub struct TownState {
    boot_frames: u32,
    rural_seeded: bool,
    /// Which world the standing lots, props and plans belong to.
    world: Option<(u32, u32, u64)>,
    plans: HashMap<(i32, i32), BlockPlan>,
}

/// Identity of the world on screen: its size and its seed.
///
/// Regenerating the same options gives the same tiles, so this is a true world
/// identity and not a content hash — nothing here draws from individual tiles,
/// only from where the town is allowed to be.
fn world_id(map: &MapGrid) -> (u32, u32, u64) {
    (map.width, map.height, map.seed)
}

/// Whether a tile is somewhere the map can actually draw.
///
/// The second of two guards against the same bug. The sim clamps
/// [`TownDensity`] to the map, and presentation refuses to draw a lot outside it
/// whatever it is handed — because "houses standing past the edge of the world"
/// is a class of failure that has come back more than once, and one guard is
/// only ever one refactor from being the guard that got deleted.
fn on_map(bounds: Option<(u32, u32)>, tile: TileCoord) -> bool {
    match bounds {
        Some((w, h)) => {
            tile.x >= 0 && tile.y >= 0 && (tile.x as u32) < w && (tile.y as u32) < h
        }
        None => true,
    }
}

// ─ The system ──────────────────────────────────────────

/// Bake on data change, advance the watchable phases, draw nothing per tile.
#[allow(clippy::too_many_arguments)]
pub fn sync_building_sprites(
    mut commands: Commands,
    time: Res<Time<Virtual>>,
    clock: Res<SimClock>,
    density: Res<TownDensity>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    network: Res<TrackNetwork>,
    map: Option<Res<MapGrid>>,
    atlas: Option<Res<BuildingAtlas>>,
    mut images: ResMut<Assets<Image>>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
    mut lots: Query<(Entity, &mut BuildingLot, &mut Sprite, &mut BuildingWindows)>,
    props: Query<(Entity, &RuralProp)>,
    mut state: Local<TownState>,
) {
    let _perf = crate::overlays::perf::scope("sync_building_sprites");
    let Some(atlas) = atlas else {
        // First run: bake the whole atlas once, then never again.
        let baked = bake_atlas(&mut images, &mut layouts);
        commands.insert_resource(baked);
        return;
    };
    state.boot_frames = state.boot_frames.saturating_add(1);

    let map_height = map.as_ref().map(|m| m.height).unwrap_or(64);
    let bounds = map.as_ref().map(|m| (m.width, m.height));

    // A new map is a new town. Everything standing — lots mid-scaffold, the
    // farmsteads the one-shot rural pass laid down, the cached plans — belongs
    // to the world that was here before, and the rural seed is a latch that
    // would otherwise never fire again. Keyed on the world's identity rather
    // than on `MapGrid::is_changed`, so an unrelated write costs one compare.
    if let Some(map) = map.as_ref() {
        let id = world_id(map);
        if state.world != Some(id) {
            let replacing = state.world.is_some();
            state.world = Some(id);
            if replacing {
                for (entity, ..) in lots.iter() {
                    commands.entity(entity).despawn();
                }
                for (entity, _) in props.iter() {
                    commands.entity(entity).despawn();
                }
                state.plans.clear();
                state.rural_seeded = false;
                state.boot_frames = 1;
                // The despawns above are queued, so this frame's queries would
                // still see the old town. Pick it up again next frame.
                return;
            }
        }
    }

    if !state.rural_seeded && state.boot_frames >= 3 {
        if let Some(map) = map.as_ref() {
            seed_rural(&mut commands, &atlas, map, &stations, &industries, &network);
            state.rural_seeded = true;
        }
    }

    let dt = if clock.is_running() {
        time.delta_secs()
    } else {
        0.0
    };

    let seeded = state.rural_seeded;
    let changed = replan_blocks(
        &density,
        &stations,
        &industries,
        &network,
        bounds,
        &mut state,
    );
    if !changed.is_empty() {
        apply_plans(
            &mut commands,
            &atlas,
            map_height,
            &changed,
            seeded,
            &mut lots,
            &props,
        );
    }

    advance_phases(&mut commands, &atlas, dt, bounds, &mut lots);
}

/// Recompute each dirty block's plan. Returns only the blocks that changed.
fn replan_blocks(
    density: &TownDensity,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    network: &TrackNetwork,
    bounds: Option<(u32, u32)>,
    state: &mut TownState,
) -> Vec<(TileCoord, BlockPlan)> {
    let mut changed = Vec::new();
    let mut live: Vec<(i32, i32)> = Vec::with_capacity(density.len());

    for (tile, d) in density.iter() {
        // Density off the map is not a block waiting to be built; it is a bug
        // upstream, and drawing it puts houses past the edge of the world.
        if !on_map(bounds, tile) {
            continue;
        }
        live.push((tile.x, tile.y));
        replan_one(tile, d, stations, industries, network, state, &mut changed);
    }

    // A block can vanish from the density map entirely — the sim drops cells
    // that decay to nothing, and a demolished station stops its ring being
    // ticked at all. Those blocks still have to empty out on screen, so any
    // cached plan the sim no longer mentions is replanned at zero.
    live.sort_unstable();
    let stale: Vec<(i32, i32)> = state
        .plans
        .keys()
        .filter(|k| live.binary_search(k).is_err())
        .copied()
        .collect();
    for (x, y) in stale {
        let tile = TileCoord { x, y };
        replan_one(tile, 0.0, stations, industries, network, state, &mut changed);
    }

    changed
}

fn replan_one(
    tile: TileCoord,
    d: f32,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    network: &TrackNetwork,
    state: &mut TownState,
    changed: &mut Vec<(TileCoord, BlockPlan)>,
) {
    let district = classify(tile, stations, industries, network);
    let held = state
        .plans
        .get(&(tile.x, tile.y))
        .map(|p| p.lots)
        .unwrap_or(0);
    let wanted = lots_wanted(d, district, held);
    let good = if district == District::Industrial {
        nearest_good(tile, industries)
    } else {
        None
    };

    let mut kinds = [None; LOTS_PER_TILE as usize];
    let order = fill_order(tile);
    for slot in order.iter().take(wanted as usize) {
        kinds[*slot as usize] = Some(plan_kind(tile, *slot, d, district, good));
    }
    let plan = BlockPlan {
        district,
        lots: wanted,
        kinds,
    };

    if state.plans.get(&(tile.x, tile.y)) != Some(&plan) {
        state.plans.insert((tile.x, tile.y), plan);
        changed.push((tile, plan));
    }
}

/// Spawn / retarget / abandon lots for the blocks whose plan moved.
fn apply_plans(
    commands: &mut Commands,
    atlas: &BuildingAtlas,
    map_height: u32,
    changed: &[(TileCoord, BlockPlan)],
    rural_seeded: bool,
    lots: &mut Query<(Entity, &mut BuildingLot, &mut Sprite, &mut BuildingWindows)>,
    props: &Query<(Entity, &RuralProp)>,
) {
    // Index only when something actually changed — the common frame does none
    // of this (pixel contract §2.5: bake on edit, not in a per-frame loop).
    let mut index: HashMap<(i32, i32, u8), Entity> = HashMap::new();
    for (entity, lot, _, _) in lots.iter() {
        index.insert((lot.tile.x, lot.tile.y, lot.slot), entity);
    }
    let mut prop_index: HashMap<(i32, i32), Entity> = HashMap::new();
    for (entity, prop) in props.iter() {
        prop_index.insert((prop.tile.x, prop.tile.y), entity);
    }

    for (tile, plan) in changed {
        for slot in 0..LOTS_PER_TILE {
            let want = plan.kinds[slot as usize];
            let existing = index.get(&(tile.x, tile.y, slot)).copied();
            match (existing, want) {
                (Some(entity), want) => {
                    if let Ok((_, mut lot, _, _)) = lots.get_mut(entity) {
                        if want.is_none() && lot.phase.is_preconstruction() {
                            // Nothing was ever built here — no scar to leave.
                            commands.entity(entity).despawn();
                            continue;
                        }
                        if lot.target != want || lot.district != plan.district {
                            lot.target = want;
                            lot.district = plan.district;
                            lot.elapsed = 0.0;
                        }
                    }
                }
                (None, Some(kind)) => {
                    spawn_lot(commands, atlas, map_height, *tile, slot, plan.district, kind);
                }
                (None, None) => {}
            }
        }

        // A prop and a building never share a block. Countryside returns when
        // a district empties out, but not before the one-shot seed has run —
        // otherwise a tile would end up with two props stacked on it.
        let prop = prop_index.get(&(tile.x, tile.y)).copied();
        let wants_prop = plan.lots == 0 && plan.district == District::Rural;
        match (prop, wants_prop) {
            (Some(entity), false) => commands.entity(entity).despawn(),
            (None, true) if rural_seeded => spawn_prop(commands, atlas, map_height, *tile),
            _ => {}
        }
    }
}

fn spawn_lot(
    commands: &mut Commands,
    atlas: &BuildingAtlas,
    map_height: u32,
    tile: TileCoord,
    slot: u8,
    district: District,
    kind: BuildingKind,
) {
    let (bx, by) = lot_base(tile, slot);
    let flip = lot_flip(tile, slot);
    let lot = BuildingLot {
        tile,
        slot,
        district,
        kind,
        target: Some(kind),
        phase: LotPhase::Stake,
        elapsed: 0.0,
    };
    let mut sprite = atlas.sprite(frame_for(&lot));
    sprite.flip_x = flip;
    commands.spawn((
        sprite,
        Anchor::BOTTOM_CENTER,
        Transform::from_xyz(bx as f32, by as f32, lot_z(by, map_height)),
        lot,
        BuildingWindows {
            lit_frame: None,
            flip_x: flip,
        },
    ));
}

fn spawn_prop(commands: &mut Commands, atlas: &BuildingAtlas, map_height: u32, tile: TileCoord) {
    let Some(prop) = rural_prop(tile) else {
        return;
    };
    let slot = rural_slot(tile);
    let (bx, by) = lot_base(tile, slot);
    let mut sprite = atlas.sprite(FRAME_RURAL + prop);
    sprite.flip_x = lot_flip(tile, slot);
    commands.spawn((
        sprite,
        Anchor::BOTTOM_CENTER,
        Transform::from_xyz(bx as f32, by as f32, lot_z(by, map_height)),
        RuralProp {
            tile,
            kind: RuralKind::Prop(prop),
        },
    ));
}

/// Lay the countryside out once.
///
/// The unserved map has to look *deliberately* rural (brief 06 §2.2) — and that
/// means mostly **open**. `lots.rs` only puts an object on a farm anchor, on the
/// worked ground next to one, or very occasionally on a lone landmark tile, so
/// this walk over the map plants a few dozen things rather than a few thousand.
fn seed_rural(
    commands: &mut Commands,
    atlas: &BuildingAtlas,
    map: &MapGrid,
    stations: &StationRegistry,
    industries: &IndustryRegistry,
    network: &TrackNetwork,
) {
    use rail_map::TerrainKind;

    for y in 0..map.height as i32 {
        for x in 0..map.width as i32 {
            let tile = TileCoord { x, y };
            let cell = map.tile(tile);
            if cell.water || !matches!(cell.kind, TerrainKind::Plains | TerrainKind::Hills) {
                continue;
            }
            if network.at(tile, GROUND_LAYER).is_some()
                || stations.at(tile, GROUND_LAYER).is_some()
                || industries.at(tile).is_some()
            {
                continue;
            }
            if let Some(kind) = rural_farmstead(tile) {
                let slot = rural_slot(tile);
                let (bx, by) = lot_base(tile, slot);
                let flip = lot_flip(tile, slot);
                let mut sprite = atlas.sprite(kind.frame(Decay::Healthy));
                sprite.flip_x = flip;
                commands.spawn((
                    sprite,
                    Anchor::BOTTOM_CENTER,
                    Transform::from_xyz(bx as f32, by as f32, lot_z(by, map.height)),
                    RuralProp {
                        tile,
                        kind: RuralKind::Farmstead,
                    },
                ));
                continue;
            }
            spawn_prop(commands, atlas, map.height, tile);
        }
    }
}

// ─ Phases ──────────────────────────────────────────────

/// Atlas frame a lot draws right now.
pub fn frame_for(lot: &BuildingLot) -> usize {
    match lot.phase {
        LotPhase::Stake => FRAME_STAKE + (world_hash(lot.tile.x, lot.tile.y, 0x57A4) % 2) as usize,
        LotPhase::Scaffold => {
            let class = lot.target.unwrap_or(lot.kind).scaffold_class();
            let half = if lot.elapsed >= SCAFFOLD_SECS * 0.5 { 1 } else { 0 };
            FRAME_SCAFFOLD + class * 2 + half
        }
        LotPhase::Settle => lot.kind.settle_frame(),
        LotPhase::Standing(decay) => lot.kind.frame(decay),
        LotPhase::Cleared => {
            FRAME_SCAR + (world_hash(lot.tile.x, lot.tile.y + 1, 0x5CA9) % 2) as usize
        }
    }
}

/// What a lot is, in words — the hover tier's answer to *what is this?*
pub fn lot_label(lot: &BuildingLot) -> &'static str {
    match lot.phase {
        LotPhase::Stake => "Surveyor's Stake",
        LotPhase::Scaffold => "Building Site",
        LotPhase::Cleared => "Cleared Lot",
        LotPhase::Settle | LotPhase::Standing(_) => lot.kind.label(),
    }
}

/// How a lot is doing. `None` while it is simply standing and well — a healthy
/// building has nothing to report, and saying so anyway is noise.
pub fn lot_condition(lot: &BuildingLot) -> Option<&'static str> {
    match lot.phase {
        LotPhase::Stake => Some("marked out"),
        LotPhase::Scaffold => Some("going up"),
        LotPhase::Settle | LotPhase::Standing(Decay::Healthy) => None,
        LotPhase::Standing(Decay::Dimmed) => Some("windows dark"),
        LotPhase::Standing(Decay::Boarded) => Some("boarded up"),
        LotPhase::Standing(Decay::Derelict) => Some("derelict"),
        LotPhase::Cleared => Some("cleared"),
    }
}

/// Frame holding this lot's lit windows, or `None` when it must stay dark.
pub fn lit_frame_for(lot: &BuildingLot) -> Option<usize> {
    match lot.phase {
        LotPhase::Standing(Decay::Healthy) => Some(lot.kind.lit_frame()),
        _ => None,
    }
}

/// Step one lot's phase machine. Returns `true` when the phase changed.
pub fn step_phase(lot: &mut BuildingLot, dt: f32) -> bool {
    lot.elapsed += dt;
    let wanted = lot.target;

    match lot.phase {
        LotPhase::Stake => {
            if lot.elapsed >= STAKE_SECS {
                return enter(lot, LotPhase::Scaffold);
            }
        }
        LotPhase::Scaffold => {
            if lot.elapsed >= SCAFFOLD_SECS {
                // The scaffold comes down onto whatever the block wants now.
                if let Some(kind) = wanted {
                    lot.kind = kind;
                }
                return enter(lot, LotPhase::Settle);
            }
        }
        LotPhase::Settle => {
            if lot.elapsed >= SETTLE_SECS {
                return enter(lot, LotPhase::Standing(Decay::Healthy));
            }
        }
        LotPhase::Standing(decay) => {
            if let Some(target) = wanted {
                if decay != Decay::Healthy {
                    // Recovery is visible: one stage back at a time.
                    if lot.elapsed >= RECOVERY_STEP_SECS {
                        let better = decay.better().unwrap_or(Decay::Healthy);
                        return enter(lot, LotPhase::Standing(better));
                    }
                } else if target.tier > lot.kind.tier {
                    // Growing taller is a build, and the player watches it.
                    return enter(lot, LotPhase::Scaffold);
                }
            } else {
                let hold = match decay {
                    Decay::Healthy => DECLINE_ONSET_SECS,
                    Decay::Derelict => CLEARED_AFTER_SECS,
                    _ => DECLINE_STEP_SECS,
                };
                if lot.elapsed >= hold {
                    return match decay.worse() {
                        Some(next) => enter(lot, LotPhase::Standing(next)),
                        None => enter(lot, LotPhase::Cleared),
                    };
                }
            }
        }
        LotPhase::Cleared => {
            if let Some(kind) = wanted {
                if lot.elapsed >= REBUILD_DELAY_SECS {
                    lot.kind = kind;
                    return enter(lot, LotPhase::Stake);
                }
            }
        }
    }
    false
}

fn enter(lot: &mut BuildingLot, phase: LotPhase) -> bool {
    lot.phase = phase;
    lot.elapsed = 0.0;
    true
}

fn advance_phases(
    commands: &mut Commands,
    atlas: &BuildingAtlas,
    dt: f32,
    bounds: Option<(u32, u32)>,
    lots: &mut Query<(Entity, &mut BuildingLot, &mut Sprite, &mut BuildingWindows)>,
) {
    for (entity, mut lot, mut sprite, mut windows) in lots.iter_mut() {
        // The backstop for anything already standing off the map — a save from
        // before density was clamped, or a future writer that forgets. This walk
        // happens every frame anyway, so the guard is free.
        if !on_map(bounds, lot.tile) {
            commands.entity(entity).despawn();
            continue;
        }
        if dt > 0.0 {
            step_phase(&mut lot, dt);
        }
        if lot.target.is_none() && lot.phase.is_preconstruction() {
            commands.entity(entity).despawn();
            continue;
        }

        let frame = frame_for(&lot);
        let current = sprite.texture_atlas.as_ref().map(|a| a.index);
        if current != Some(frame) {
            // Only touch the sprite when the drawn frame actually moves.
            sprite.texture_atlas = Some(TextureAtlas {
                layout: atlas.layout.clone(),
                index: frame,
            });
        }

        let lit = lit_frame_for(&lot);
        if windows.lit_frame != lit {
            windows.lit_frame = lit;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::building_art::{Family, Roof};
    use super::*;

    fn kind(tier: u8) -> BuildingKind {
        BuildingKind {
            family: Family::Town,
            tier,
            variant: 0,
            roof: Roof::Tile,
        }
    }

    fn lot(phase: LotPhase, target: Option<BuildingKind>) -> BuildingLot {
        BuildingLot {
            tile: TileCoord { x: 4, y: 4 },
            slot: 0,
            district: District::Residential,
            kind: kind(0),
            target,
            phase,
            elapsed: 0.0,
        }
    }

    /// Run the machine until `phase` settles or the budget runs out.
    fn run(lot: &mut BuildingLot, secs: f32) {
        let mut left = secs;
        while left > 0.0 {
            step_phase(lot, 0.1);
            left -= 0.1;
        }
    }

    #[test]
    fn construction_is_stake_then_scaffold_then_settle() {
        let mut l = lot(LotPhase::Stake, Some(kind(1)));
        run(&mut l, STAKE_SECS + 0.2);
        assert_eq!(l.phase, LotPhase::Scaffold);
        run(&mut l, SCAFFOLD_SECS - 1.0);
        assert_eq!(l.phase, LotPhase::Scaffold, "the scaffold must hold");
        // One step across the hold: the building drops in before it settles.
        step_phase(&mut l, 1.2);
        assert_eq!(l.phase, LotPhase::Settle);
        run(&mut l, SETTLE_SECS + 0.1);
        assert_eq!(l.phase, LotPhase::Standing(Decay::Healthy));
        assert_eq!(l.kind, kind(1), "the scaffold resolves to what was wanted");
    }

    #[test]
    fn scaffold_holds_for_about_eight_seconds() {
        let mut l = lot(LotPhase::Scaffold, Some(kind(0)));
        run(&mut l, 7.5);
        assert_eq!(l.phase, LotPhase::Scaffold);
        run(&mut l, 1.0);
        assert_ne!(l.phase, LotPhase::Scaffold);
    }

    #[test]
    fn scaffold_swaps_frame_halfway_so_it_is_not_a_static_prop() {
        let mut early = lot(LotPhase::Scaffold, Some(kind(2)));
        early.elapsed = 1.0;
        let mut late = early.clone();
        late.elapsed = SCAFFOLD_SECS * 0.75;
        assert_ne!(frame_for(&early), frame_for(&late));
    }

    #[test]
    fn decline_walks_every_stage_down_to_a_scar() {
        let mut l = lot(LotPhase::Standing(Decay::Healthy), None);
        let mut seen = vec![l.phase];
        for _ in 0..2000 {
            if step_phase(&mut l, 0.1) {
                seen.push(l.phase);
            }
        }
        assert_eq!(
            seen,
            vec![
                LotPhase::Standing(Decay::Healthy),
                LotPhase::Standing(Decay::Dimmed),
                LotPhase::Standing(Decay::Boarded),
                LotPhase::Standing(Decay::Derelict),
                LotPhase::Cleared,
            ]
        );
    }

    #[test]
    fn service_returning_reverses_every_decline_stage_visibly() {
        for stage in [Decay::Dimmed, Decay::Boarded, Decay::Derelict] {
            let mut l = lot(LotPhase::Standing(stage), Some(kind(0)));
            // One step back per recovery hold — not an instant snap.
            run(&mut l, RECOVERY_STEP_SECS + 0.1);
            let expected = stage.better().unwrap();
            assert_eq!(
                l.phase,
                LotPhase::Standing(expected),
                "{stage:?} must visibly step back, not jump to healthy"
            );
            run(&mut l, RECOVERY_STEP_SECS * 3.0 + 0.5);
            assert_eq!(l.phase, LotPhase::Standing(Decay::Healthy));
        }
    }

    #[test]
    fn a_cleared_lot_rebuilds_when_service_comes_back() {
        let mut l = lot(LotPhase::Cleared, None);
        run(&mut l, 5.0);
        assert_eq!(l.phase, LotPhase::Cleared, "the scar persists");
        l.target = Some(kind(2));
        run(&mut l, REBUILD_DELAY_SECS + 0.2);
        assert_eq!(l.phase, LotPhase::Stake);
        run(&mut l, STAKE_SECS + SCAFFOLD_SECS + SETTLE_SECS + 0.5);
        assert_eq!(l.phase, LotPhase::Standing(Decay::Healthy));
        assert_eq!(l.kind.tier, 2);
    }

    #[test]
    fn growing_taller_goes_back_through_a_scaffold() {
        let mut l = lot(LotPhase::Standing(Decay::Healthy), Some(kind(3)));
        assert!(step_phase(&mut l, 0.1));
        assert_eq!(l.phase, LotPhase::Scaffold);
        run(&mut l, SCAFFOLD_SECS + SETTLE_SECS + 0.3);
        assert_eq!(l.kind.tier, 3);
    }

    #[test]
    fn a_lot_never_shrinks_while_it_is_still_wanted() {
        let mut l = lot(LotPhase::Standing(Decay::Healthy), Some(kind(0)));
        l.kind = kind(3);
        run(&mut l, 60.0);
        assert_eq!(l.phase, LotPhase::Standing(Decay::Healthy));
        assert_eq!(l.kind.tier, 3, "tiers come down by decay, not by popping");
    }

    #[test]
    fn windows_only_light_on_a_healthy_standing_building() {
        assert!(lit_frame_for(&lot(LotPhase::Standing(Decay::Healthy), None)).is_some());
        for phase in [
            LotPhase::Stake,
            LotPhase::Scaffold,
            LotPhase::Settle,
            LotPhase::Standing(Decay::Dimmed),
            LotPhase::Standing(Decay::Boarded),
            LotPhase::Standing(Decay::Derelict),
            LotPhase::Cleared,
        ] {
            assert!(
                lit_frame_for(&lot(phase, None)).is_none(),
                "{phase:?} must not light up at night"
            );
        }
    }

    #[test]
    fn southerly_buildings_draw_in_front() {
        let north = lot_z(60 * TILE_TEXELS, 64);
        let south = lot_z(2 * TILE_TEXELS, 64);
        assert!(south > north, "further south must sort in front");
        // And the whole band sits between track (1.0) and peeps (2.0).
        for y in [0, 1000, 2047] {
            let z = lot_z(y, 64);
            assert!(z > 1.0 && z < 2.0, "z {z} left the building band");
        }
    }

    #[test]
    fn phase_frames_are_all_distinct() {
        let mut l = lot(LotPhase::Stake, Some(kind(1)));
        l.kind = kind(1);
        let mut frames = Vec::new();
        for phase in [
            LotPhase::Stake,
            LotPhase::Scaffold,
            LotPhase::Settle,
            LotPhase::Standing(Decay::Healthy),
            LotPhase::Standing(Decay::Dimmed),
            LotPhase::Standing(Decay::Boarded),
            LotPhase::Standing(Decay::Derelict),
            LotPhase::Cleared,
        ] {
            l.phase = phase;
            frames.push(frame_for(&l));
        }
        let mut sorted = frames.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), frames.len(), "phases share a frame: {frames:?}");
    }

    #[test]
    fn a_paused_sim_does_not_advance_construction() {
        let mut l = lot(LotPhase::Scaffold, Some(kind(0)));
        for _ in 0..200 {
            step_phase(&mut l, 0.0);
        }
        assert_eq!(l.phase, LotPhase::Scaffold);
    }
}

/// Wiring checks against a real (headless) app, so a system-param or boot-order
/// mistake fails here rather than on the first frame of the game.
#[cfg(test)]
mod app_tests {
    use super::*;
    use super::building_art::{Family, Roof};
    use bevy::image::TextureAtlasLayout;
    use bevy::MinimalPlugins;
    use rail_map::{generate_map, TerrainKind};

    fn kind(tier: u8) -> BuildingKind {
        BuildingKind {
            family: Family::Town,
            tier,
            variant: 0,
            roof: Roof::Tile,
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<Assets<Image>>();
        app.init_resource::<Assets<TextureAtlasLayout>>();
        app.insert_resource(generate_map(24, 24, 42));
        app.insert_resource(SimClock::default());
        app.init_resource::<TownDensity>();
        app.init_resource::<IndustryRegistry>();
        app.init_resource::<TrackNetwork>();

        let mut stations = StationRegistry::new();
        stations.insert("Westbrook", TileCoord { x: 8, y: 8 }, GROUND_LAYER);
        app.insert_resource(stations);

        app.add_systems(Update, sync_building_sprites);
        app
    }

    fn count<C: Component>(app: &mut App) -> usize {
        app.world_mut().query::<&C>().iter(app.world()).count()
    }

    fn settle(app: &mut App, frames: usize) {
        for _ in 0..frames {
            app.update();
        }
    }

    #[test]
    fn the_atlas_bakes_once_at_boot() {
        let mut app = test_app();
        app.update();
        let atlas = app
            .world()
            .get_resource::<BuildingAtlas>()
            .expect("atlas resource after the first frame");
        let image = atlas.image.clone();
        settle(&mut app, 5);
        assert_eq!(
            app.world().resource::<BuildingAtlas>().image,
            image,
            "the atlas must be baked once, not per frame"
        );
    }

    #[test]
    fn density_becomes_lots_that_start_as_stakes() {
        let mut app = test_app();
        settle(&mut app, 4);
        app.world_mut()
            .resource_mut::<TownDensity>()
            .set(TileCoord { x: 8, y: 8 }, 0.9);
        settle(&mut app, 2);

        let mut query = app.world_mut().query::<&BuildingLot>();
        let lots: Vec<LotPhase> = query.iter(app.world()).map(|l| l.phase).collect();
        assert_eq!(lots.len(), LOTS_PER_TILE as usize, "a full block is four lots");
        assert!(
            lots.iter().all(|p| *p == LotPhase::Stake),
            "growth starts at a surveyor's stake, not a finished building"
        );
    }

    #[test]
    fn losing_the_density_cell_still_empties_the_block() {
        let mut app = test_app();
        settle(&mut app, 4);
        app.world_mut()
            .resource_mut::<TownDensity>()
            .set(TileCoord { x: 8, y: 8 }, 0.9);
        settle(&mut app, 2);
        assert_eq!(count::<BuildingLot>(&mut app), 4);

        // The sim drops cells that decay to nothing; the block must not freeze.
        app.world_mut()
            .resource_mut::<TownDensity>()
            .set(TileCoord { x: 8, y: 8 }, 0.0);
        settle(&mut app, 2);

        let mut query = app.world_mut().query::<&BuildingLot>();
        assert!(
            query.iter(app.world()).all(|l| l.target.is_none()),
            "every lot must be told it is no longer wanted"
        );
    }

    #[test]
    fn unserved_land_is_open_country_with_the_odd_farm() {
        let mut app = test_app();
        settle(&mut app, 6);

        let map = app.world().resource::<rail_map::MapGrid>().clone();
        let eligible = (0..map.height as i32)
            .flat_map(|y| (0..map.width as i32).map(move |x| TileCoord { x, y }))
            .filter(|t| {
                let cell = map.tile(*t);
                !cell.water && matches!(cell.kind, TerrainKind::Plains | TerrainKind::Hills)
            })
            .count();

        let props = count::<RuralProp>(&mut app);
        assert!(eligible > 0, "the test map has no buildable land");
        assert!(
            props > 0,
            "unserved land needs *something* on it, or it reads as unfinished"
        );
        // The playtest verdict on the old scatter was "extremely numerous …
        // spread out all over the map". Open country is the point: a town can
        // only read as a place if there is empty land around it.
        assert!(
            props <= eligible / 8,
            "{props} props across {eligible} rural tiles carpets the map"
        );
    }

    /// The reported bug: "stuff spawned outside the map edge, e.g. homes."
    ///
    /// The sim clamps density to the map now, but presentation must refuse
    /// regardless — density is a sparse map keyed by tile and anything that can
    /// write it can put a house past the edge of the world.
    #[test]
    fn presentation_refuses_to_build_off_the_map() {
        let mut app = test_app();
        settle(&mut app, 4);

        {
            let mut density = app.world_mut().resource_mut::<TownDensity>();
            // A station two tiles from the border reaches past it — this is the
            // shape of ring the growth system used to write unchecked.
            density.set(TileCoord { x: -2, y: 8 }, 0.9);
            density.set(TileCoord { x: 8, y: -1 }, 0.9);
            density.set(TileCoord { x: 26, y: 8 }, 0.9);
            density.set(TileCoord { x: 8, y: 30 }, 0.9);
            // And one honest block, so the test can tell "refused" from "inert".
            density.set(TileCoord { x: 8, y: 8 }, 0.9);
        }
        settle(&mut app, 4);

        let map = app.world().resource::<rail_map::MapGrid>().clone();
        let mut query = app.world_mut().query::<&BuildingLot>();
        let offenders: Vec<TileCoord> = query
            .iter(app.world())
            .map(|l| l.tile)
            .filter(|t| !map.contains(*t))
            .collect();
        assert!(
            offenders.is_empty(),
            "houses standing off the map at {offenders:?}"
        );
        assert_eq!(
            count::<BuildingLot>(&mut app),
            LOTS_PER_TILE as usize,
            "the in-bounds block still builds"
        );
    }

    #[test]
    fn a_lot_already_standing_off_the_map_is_cleared() {
        // The backstop, for a save written before density was clamped.
        let mut app = test_app();
        settle(&mut app, 4);
        let atlas = app.world().resource::<BuildingAtlas>().clone();
        let stray = app
            .world_mut()
            .spawn((
                atlas.sprite(0),
                Anchor::BOTTOM_CENTER,
                Transform::from_xyz(0.0, 0.0, 1.5),
                BuildingLot {
                    tile: TileCoord { x: 90, y: 90 },
                    slot: 0,
                    district: District::Residential,
                    kind: kind(1),
                    target: Some(kind(1)),
                    phase: LotPhase::Standing(Decay::Healthy),
                    elapsed: 0.0,
                },
                BuildingWindows {
                    lit_frame: None,
                    flip_x: false,
                },
            ))
            .id();
        settle(&mut app, 2);
        assert!(
            app.world().get_entity(stray).is_err(),
            "a lot outside the map must be cleared, not merely left un-updated"
        );
    }

    /// The one-shot rural seed is a latch, and a `Local` outlives the world.
    ///
    /// After a New Map the old world's farmsteads stayed standing — including
    /// wherever they fell relative to a map of a different size — and the new
    /// world never got any of its own.
    #[test]
    fn a_new_map_clears_the_old_town_and_lays_out_a_new_one() {
        let mut app = test_app();
        settle(&mut app, 4);
        app.world_mut()
            .resource_mut::<TownDensity>()
            .set(TileCoord { x: 8, y: 8 }, 0.9);
        settle(&mut app, 4);
        assert!(count::<BuildingLot>(&mut app) > 0);
        assert!(count::<RuralProp>(&mut app) > 0);

        // A new world, smaller and differently shaped, with an empty town.
        app.world_mut().insert_resource(generate_map(16, 16, 7));
        app.world_mut()
            .insert_resource(TownDensity::default());
        let mut stations = StationRegistry::new();
        stations.insert("Eastgate", TileCoord { x: 5, y: 5 }, GROUND_LAYER);
        app.world_mut().insert_resource(stations);
        settle(&mut app, 8);

        assert_eq!(
            count::<BuildingLot>(&mut app),
            0,
            "the old town's buildings do not move to the new map"
        );
        let map = app.world().resource::<rail_map::MapGrid>().clone();
        let mut query = app.world_mut().query::<&RuralProp>();
        let props: Vec<TileCoord> = query.iter(app.world()).map(|p| p.tile).collect();
        assert!(
            !props.is_empty(),
            "the new world's countryside must be laid out too, not skipped \
             because the one-shot latch already fired"
        );
        for tile in props {
            assert!(
                map.contains(tile),
                "a farmstead from the old map is standing at {tile:?}"
            );
        }
    }

    #[test]
    fn a_settled_town_does_no_work_per_frame() {
        let mut app = test_app();
        settle(&mut app, 4);
        app.world_mut()
            .resource_mut::<TownDensity>()
            .set(TileCoord { x: 8, y: 8 }, 0.9);
        settle(&mut app, 4);

        let before = count::<BuildingLot>(&mut app);
        // Nothing about the world changes, so nothing should be respawned.
        settle(&mut app, 20);
        assert_eq!(count::<BuildingLot>(&mut app), before);
    }
}
