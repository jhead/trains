//! Border links: opening one as a construction project, and what it holds.
//!
//! `12-multiplayer.md` §3.1 — a map has four edges and each can host exactly one
//! link. An edge with no link is closed: the map boundary, as it is in solo play
//! today. Opening one is a construction project and should feel like one, so it
//! is validated, priced and refunded exactly the way a station is
//! ([`crate::stations::place`]) rather than being a toggle in a menu.
//!
//! # Why the money only ever runs one way
//!
//! §2.2 says a neighbour can never take, spend or cost you money. So:
//!
//! - The portal has a **one-off build cost and no upkeep at all**. It is not in
//!   [`crate::economy::opex`] and it never will be.
//! - [`try_close_border`] refunds `paid_cents` **in full**, which also makes
//!   undo exact.
//! - Every other cent that moves across a border is a **credit**.
//!
//! There is no code path in this crate by which a neighbour debits the player.
//! That is not an accident of the current balance, it is the constraint.

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::commands::TrainKind;
use crate::economy::{MoneyCategory, MoneyLedger};
use crate::ids::{TileCoord, TrackId, TrainId};
use crate::money::Money;
use crate::stations::GoodKind;
use crate::track::{TrackNetwork, TrackTerrain, GROUND_LAYER};

use super::echo::{echo_link_id, echo_manifest};
use super::edge::{edge_for_tile, BorderEdge};
use super::manifest::{
    BorderManifest, Departure, LinkId, StandingOffer, StandingRequest, MANIFEST_SCHEMA_VERSION,
};

/// Opening a portal: $1,500.
///
/// §3.1 wants "expensive enough to be a real commitment competing with domestic
/// expansion". Against a $10,000 opening treasury that is an interchange
/// ($400) plus two goods trains ($750 each) plus most of a mile of track — the
/// most distant, least immediately useful thing on the build menu, and it should
/// pay off over hours.
pub const BORDER_PORTAL_COST_CENTS: i64 = 1_500_000;

/// Sim ticks a train spends in transit beyond the portal.
///
/// At ten sim-seconds a tick this is half a sim-hour each way. §4.2: "A border
/// route is a slow, patient thing, and that suits the game's temperament."
pub const BORDER_CROSSING_TICKS: u64 = 180;

/// Paid per unit of border goods landed: $40.
///
/// Twice [`GOODS_DELIVERY_CENTS`](crate::economy::GOODS_DELIVERY_CENTS),
/// because §4.3 says border goods are worth more — they are what you cannot
/// produce. Landed at the portal, so it pays even when nothing on your map
/// consumes the commodity yet.
pub const BORDER_ARRIVAL_CENTS: i64 = 4_000;

/// Crossings before a relationship is fully mature.
///
/// §4.3: "A link that has been running a long time trades at better rates."
pub const MATURITY_CROSSINGS: u32 = 24;

/// Extra percent paid on a fully mature link.
pub const MATURITY_BONUS_PERCENT: i64 = 50;

/// Sim ticks between border lines in Town Talk, per link.
///
/// The feed is the game's ambient voice, not a log; a trade route that spoke
/// every crossing would drown the town.
pub const BORDER_TALK_COOLDOWN_TICKS: u64 = 120;

/// Why a border action was refused.
///
/// Mirrors [`PlacementError`](crate::track::PlacementError) and
/// [`StationPlacementError`](crate::stations::StationPlacementError): every
/// rejection names its rule, and where the rule has a number it carries both the
/// value and the limit so the player learns it rather than bouncing off it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderError {
    /// Only ground layer (`0`) is buildable in MVP.
    InvalidLayer,
    /// The tile is not on the map boundary — a border needs an edge.
    NotOnEdge,
    /// The line has to reach the boundary first; there is no track here.
    NoTrack,
    /// The tile is on the boundary, but not on the edge that was asked for.
    WrongEdge { tile_edge: BorderEdge, asked: BorderEdge },
    /// One link per edge (§3.1). Close the existing one first.
    EdgeAlreadyOpen { edge: BorderEdge },
    /// Not enough money, with the price so the player learns it.
    InsufficientFunds { need: i64, have: i64 },
    /// Close / trade / dispatch aimed at an edge with no link.
    EdgeClosed { edge: BorderEdge },
    /// Dispatch aimed at a train that is not on the map.
    UnknownTrain { train: TrainId },
    /// No route from the train's railhead to the portal.
    NoRouteToPortal { edge: BorderEdge },
}

impl BorderError {
    /// Plain, specific reason chip — the established rejection voice.
    pub fn reason(self) -> String {
        match self {
            Self::InvalidLayer => "Borders are ground-level only".into(),
            Self::NotOnEdge => "A border portal has to sit on the map edge".into(),
            Self::NoTrack => "Run a line to the edge first".into(),
            Self::WrongEdge { tile_edge, asked } => format!(
                "That tile is on the {} edge, not the {}",
                tile_edge.label(),
                asked.label()
            ),
            Self::EdgeAlreadyOpen { edge } => {
                format!("The {} border is already open", edge.label())
            }
            Self::InsufficientFunds { need, have } => format!(
                "A portal costs ${}, and you have ${}",
                need / 100,
                have / 100
            ),
            Self::EdgeClosed { edge } => format!("The {} border is not open", edge.label()),
            Self::UnknownTrain { train } => format!("Train {} is not on the map", train.0),
            Self::NoRouteToPortal { edge } => {
                format!("No route from there to the {} portal", edge.label())
            }
        }
    }
}

/// A train that has left the map through a portal.
///
/// This is the state that makes §2.1 true. A train that reaches the border
/// **leaves** — it stops being an entity, stops occupying a tile, stops being
/// anybody's blocker — and `due_tick` is written down at that moment from data
/// already in hand. Nothing about its return waits on anybody.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitTrain {
    pub train: TrainId,
    pub kind: TrainKind,
    /// Border tick it crossed on.
    pub sent_tick: u64,
    /// Border tick it comes back on. Decided at departure, never later.
    pub due_tick: u64,
    /// Railhead it re-enters the map on.
    pub home: TrackId,
    /// What it took over, if anything.
    pub carried: Option<GoodKind>,
    pub units: u32,
}

/// One open border and everything behind it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BorderLink {
    pub edge: BorderEdge,
    pub link: LinkId,
    /// The border tile the player's line reached.
    pub portal_tile: TileCoord,
    pub layer: u8,
    /// Refunded in full on close, so undo is exact.
    pub paid_cents: i64,
    /// Border tick the portal opened, for growth and for the panel.
    pub opened_tick: u64,

    /// **The cache** (§5). The last manifest we heard, whoever sent it.
    ///
    /// Return trains are generated from this on our own tick. It persists
    /// through a save, so an offer heard once keeps supplying forever.
    pub neighbour: BorderManifest,
    /// What we publish outward. MP-2 puts this in a blob store unchanged.
    pub outbound: BorderManifest,

    /// Trains currently beyond the border.
    pub transit: Vec<TransitTrain>,
    /// Completed crossings, which is what matures the relationship.
    pub crossings: u32,
    pub sent_units: u32,
    pub received_units: u32,
    /// Their rhythm, `0..offer.period_ticks`. Drives the yard; touches nothing.
    pub their_phase: u32,
    /// Border tick this link last spoke in Town Talk.
    pub last_spoke_tick: u64,
}

impl BorderLink {
    /// A freshly opened link with its echo neighbour already cached.
    ///
    /// The cache is warm from the first tick, so trade can begin immediately and
    /// there is no "awaiting neighbour" state to sit in.
    pub fn opened(
        edge: BorderEdge,
        portal_tile: TileCoord,
        layer: u8,
        seed: u64,
        tick: u64,
        paid_cents: i64,
    ) -> Self {
        let link = echo_link_id(seed, edge);
        Self {
            edge,
            link,
            portal_tile,
            layer,
            paid_cents,
            opened_tick: tick,
            neighbour: echo_manifest(seed, edge, 0),
            outbound: BorderManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                link,
                edge,
                ..Default::default()
            },
            transit: Vec::new(),
            crossings: 0,
            sent_units: 0,
            received_units: 0,
            their_phase: 0,
            last_spoke_tick: 0,
        }
    }

    pub fn town_name(&self) -> &str {
        self.neighbour.town_name()
    }

    pub fn is_echo(&self) -> bool {
        self.neighbour.is_echo()
    }

    /// What they will send us.
    pub fn their_offer(&self) -> StandingOffer {
        self.neighbour.offer
    }

    /// What they would like from us.
    pub fn their_request(&self) -> StandingRequest {
        self.neighbour.request
    }

    /// What we have told them we supply.
    pub fn our_offer(&self) -> StandingOffer {
        self.outbound.offer
    }

    /// Relationship maturity, `0..=100`.
    pub fn maturity(&self) -> u8 {
        let capped = self.crossings.min(MATURITY_CROSSINGS);
        ((capped as u32 * 100) / MATURITY_CROSSINGS.max(1)) as u8
    }

    /// Cents paid for `units` of border goods landing here.
    ///
    /// Strictly a credit, and it only ever grows with the relationship.
    pub fn arrival_payout_cents(&self, units: u32) -> i64 {
        let units = units.max(1) as i64;
        let base = BORDER_ARRIVAL_CENTS.saturating_mul(units);
        let bonus = base
            .saturating_mul(MATURITY_BONUS_PERCENT)
            .saturating_mul(self.maturity() as i64)
            / 10_000;
        base.saturating_add(bonus)
    }

    /// Take the cached neighbour's word for it, if the manifest is usable.
    ///
    /// Returns `true` when the cache moved on. A refusal is silent and total:
    /// the previous cache keeps supplying, which is §5's "reject the manifest,
    /// fall back to cache. Trade continues."
    pub fn accept_manifest(&mut self, manifest: BorderManifest) -> bool {
        let Some(clean) = manifest.sanitised(self.link) else {
            return false;
        };
        if !clean.supersedes(&self.neighbour) {
            return false;
        }
        self.neighbour = clean;
        true
    }

    /// Publish a departure outward and bump our sequence.
    pub fn publish_departure(&mut self, departure: Departure) {
        self.outbound.push_departure(departure);
        self.outbound.sequence = self.outbound.sequence.saturating_add(1);
    }

    /// Set what we supply and what we want. Player-facing, so it is a command.
    pub fn set_trade(&mut self, offer: GoodKind, request: GoodKind) {
        self.outbound.offer.good = offer;
        self.outbound.request.good = request;
        self.outbound.sequence = self.outbound.sequence.saturating_add(1);
    }

    /// Ticks until the next train is due back, if any is out.
    pub fn next_due_in(&self, now: u64) -> Option<u64> {
        self.transit
            .iter()
            .map(|t| t.due_tick.saturating_sub(now))
            .min()
    }
}

/// Every border on this map. Empty is solo play, and solo play is the default.
#[derive(Debug, Clone, Default, PartialEq, Resource, Serialize, Deserialize)]
pub struct BorderRegistry {
    /// At most one per edge, ascending by [`BorderEdge::index`].
    links: Vec<BorderLink>,
    /// Links that were closed, kept so severing one is never destructive.
    ///
    /// §7: "Replacing a link is always allowed and never destructive — swapping
    /// a neighbour keeps your track, your portal, and your goods." Re-opening an
    /// edge you once had restores its crossings, its maturity and its cached
    /// offer, which is also what makes undo of a close exact.
    closed: Vec<BorderLink>,
    /// The border clock.
    ///
    /// Its own counter rather than a borrowed one, so nothing here depends on
    /// where in the tick anybody else's counter is bumped — which keeps the
    /// module deterministic and keeps its wiring to one `.add_systems` line.
    pub tick: u64,
    /// Map seed, mirrored so echoes regenerate without reaching for the map.
    pub seed: u64,
}

impl BorderRegistry {
    pub fn new(seed: u64) -> Self {
        Self {
            links: Vec::new(),
            closed: Vec::new(),
            tick: 0,
            seed,
        }
    }

    pub fn len(&self) -> usize {
        self.links.len()
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &BorderLink> {
        self.links.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut BorderLink> {
        self.links.iter_mut()
    }

    pub fn get(&self, edge: BorderEdge) -> Option<&BorderLink> {
        self.links.iter().find(|l| l.edge == edge)
    }

    pub fn get_mut(&mut self, edge: BorderEdge) -> Option<&mut BorderLink> {
        self.links.iter_mut().find(|l| l.edge == edge)
    }

    pub fn is_open(&self, edge: BorderEdge) -> bool {
        self.get(edge).is_some()
    }

    /// Link on the border tile, if that tile is a portal.
    pub fn at_tile(&self, tile: TileCoord) -> Option<&BorderLink> {
        self.links.iter().find(|l| l.portal_tile == tile)
    }

    pub fn by_link_id(&self, link: LinkId) -> Option<&BorderLink> {
        self.links.iter().find(|l| l.link == link)
    }

    fn insert(&mut self, link: BorderLink) {
        self.links.retain(|l| l.edge != link.edge);
        self.links.push(link);
        self.links.sort_by_key(|l| l.edge.index());
    }

    /// Move a link into the archive and hand it back.
    fn take(&mut self, edge: BorderEdge) -> Option<BorderLink> {
        let index = self.links.iter().position(|l| l.edge == edge)?;
        let link = self.links.remove(index);
        self.closed.retain(|l| l.edge != edge);
        self.closed.push(link.clone());
        Some(link)
    }

    /// A relationship this edge used to have with the same neighbour, if any.
    fn archived(&mut self, edge: BorderEdge, link: LinkId) -> Option<BorderLink> {
        let index = self
            .closed
            .iter()
            .position(|l| l.edge == edge && l.link == link)?;
        Some(self.closed.remove(index))
    }

    /// Relationships this map has severed, for the panel's history.
    pub fn iter_closed(&self) -> impl Iterator<Item = &BorderLink> {
        self.closed.iter()
    }

    /// Every link a train could still come home through — open ones, and
    /// severed ones that still have stock out.
    ///
    /// Closing a link must not strand rolling stock (§2.1), so a severed link
    /// keeps its transit list and keeps landing trains on schedule.
    pub fn all_mut(&mut self) -> impl Iterator<Item = &mut BorderLink> {
        self.links.iter_mut().chain(self.closed.iter_mut())
    }

    /// Any link with this id, open or severed.
    pub fn any_by_link_id_mut(&mut self, link: LinkId) -> Option<&mut BorderLink> {
        self.links
            .iter_mut()
            .chain(self.closed.iter_mut())
            .find(|l| l.link == link)
    }

    /// Trains beyond the border right now, across every link.
    pub fn trains_in_transit(&self) -> usize {
        self.links
            .iter()
            .chain(self.closed.iter())
            .map(|l| l.transit.len())
            .sum()
    }
}

/// Result of a successful open.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenedBorder {
    pub edge: BorderEdge,
    pub link: LinkId,
    pub portal_tile: TileCoord,
    pub town_name: String,
    pub is_echo: bool,
    pub cost_cents: i64,
}

/// The tile on `edge` where the player's line already reaches the boundary.
///
/// This is what makes the portal a *construction project* rather than a menu
/// toggle: the expensive, slow part is running the line out to the edge, and the
/// button only pays for the door. `None` means no line has got there yet, which
/// is exactly [`BorderError::NoTrack`].
///
/// Scanned in ascending coordinate order so the answer is deterministic, and
/// corners are skipped on east and west to agree with
/// [`edge_for_tile`](super::edge::edge_for_tile), which awards them to north and
/// south.
pub fn railhead_on_edge(
    network: &TrackNetwork,
    terrain: &TrackTerrain,
    edge: BorderEdge,
    layer: u8,
) -> Option<TileCoord> {
    let (w, h) = (terrain.width() as i32, terrain.height() as i32);
    if w < 2 || h < 2 {
        return None;
    }
    let tiles: Vec<TileCoord> = match edge {
        BorderEdge::North => (0..w).map(|x| TileCoord { x, y: h - 1 }).collect(),
        BorderEdge::South => (0..w).map(|x| TileCoord { x, y: 0 }).collect(),
        BorderEdge::East => (1..h - 1).map(|y| TileCoord { x: w - 1, y }).collect(),
        BorderEdge::West => (1..h - 1).map(|y| TileCoord { x: 0, y }).collect(),
    };
    tiles
        .into_iter()
        .find(|tile| network.id_at(*tile, layer).is_some())
}

/// Check every rule for putting a portal on `tile`.
///
/// A portal is the end of a line, so the rules are rules about the line: it must
/// reach the boundary, and the boundary it reaches must be the one being opened.
pub fn validate_border_site(
    registry: &BorderRegistry,
    network: &TrackNetwork,
    terrain: &TrackTerrain,
    tile: TileCoord,
    layer: u8,
    edge: BorderEdge,
) -> Result<(), BorderError> {
    if layer != GROUND_LAYER {
        return Err(BorderError::InvalidLayer);
    }
    let Some(tile_edge) = edge_for_tile(terrain.width(), terrain.height(), tile) else {
        return Err(BorderError::NotOnEdge);
    };
    if tile_edge != edge {
        return Err(BorderError::WrongEdge {
            tile_edge,
            asked: edge,
        });
    }
    if registry.is_open(edge) {
        return Err(BorderError::EdgeAlreadyOpen { edge });
    }
    if network.id_at(tile, layer).is_none() {
        return Err(BorderError::NoTrack);
    }
    Ok(())
}

/// Open a border, debiting [`Money`].
///
/// The neighbour behind it is cached immediately (an echo, generated from the
/// map seed and the edge), so trade can start on the next tick and there is
/// never a pending state.
#[allow(clippy::too_many_arguments)]
pub fn try_open_border(
    registry: &mut BorderRegistry,
    money: &mut Money,
    ledger: &mut MoneyLedger,
    network: &TrackNetwork,
    terrain: &TrackTerrain,
    tile: TileCoord,
    layer: u8,
    edge: BorderEdge,
) -> Result<OpenedBorder, BorderError> {
    validate_border_site(registry, network, terrain, tile, layer, edge)?;

    let cost = BORDER_PORTAL_COST_CENTS;
    ledger
        .try_debit(money, MoneyCategory::Construction, cost)
        .map_err(|_| BorderError::InsufficientFunds {
            need: cost,
            have: money.cents(),
        })?;

    let seed = registry.seed;
    let tick = registry.tick;
    // Re-opening an edge you once had resumes the relationship rather than
    // starting a new one — crossings, maturity and the cached offer survive.
    let link = match registry.archived(edge, echo_link_id(seed, edge)) {
        Some(mut previous) => {
            previous.portal_tile = tile;
            previous.layer = layer;
            previous.paid_cents = cost;
            previous
        }
        None => BorderLink::opened(edge, tile, layer, seed, tick, cost),
    };
    let opened = OpenedBorder {
        edge,
        link: link.link,
        portal_tile: tile,
        town_name: link.town_name().to_string(),
        is_echo: link.is_echo(),
        cost_cents: cost,
    };
    registry.insert(link);
    Ok(opened)
}

/// Close a border, refunding what was spent on it in full.
///
/// §8.1: links are severable at any time, from either side, with no penalty.
/// §7: swapping a neighbour keeps your track, your portal and your goods — so
/// closing removes the link and nothing else.
///
/// Trains already beyond the border are **not** cancelled. The severed link is
/// archived with its transit list intact and keeps landing them on schedule
/// (see [`BorderRegistry::all_mut`]), because §2.1 says nothing may strand
/// stock — including the player's own change of mind.
pub fn try_close_border(
    registry: &mut BorderRegistry,
    money: &mut Money,
    ledger: &mut MoneyLedger,
    edge: BorderEdge,
) -> Result<BorderLink, BorderError> {
    let link = registry
        .take(edge)
        .ok_or(BorderError::EdgeClosed { edge })?;
    ledger.credit(money, MoneyCategory::Construction, link.paid_cents);
    Ok(link)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economy::MoneyLedger;
    use crate::track::try_place_track;

    fn land(w: u32, h: u32) -> TrackTerrain {
        TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 0i8)))
    }

    struct World {
        registry: BorderRegistry,
        money: Money,
        ledger: MoneyLedger,
        network: TrackNetwork,
        terrain: TrackTerrain,
    }

    /// 16×16 of flat land with a line running east from `(8, 8)` to the edge.
    fn world() -> World {
        let terrain = land(16, 16);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(10_000_000);
        let mut ledger = MoneyLedger::default();
        for x in 8..16 {
            try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                TileCoord { x, y: 8 },
                GROUND_LAYER,
            )
            .expect("track");
        }
        World {
            registry: BorderRegistry::new(42),
            // A border is a mid-game commitment costing more than the opening
            // balance, so the fixture is a railway that has already earned.
            money: Money::new(10_000_000),
            ledger: MoneyLedger::default(),
            network,
            terrain,
        }
    }

    fn open(w: &mut World, tile: TileCoord, edge: BorderEdge) -> Result<OpenedBorder, BorderError> {
        try_open_border(
            &mut w.registry,
            &mut w.money,
            &mut w.ledger,
            &w.network,
            &w.terrain,
            tile,
            GROUND_LAYER,
            edge,
        )
    }

    #[test]
    fn opening_a_border_charges_and_caches_a_neighbour_at_once() {
        let mut w = world();
        let before = w.money.cents();

        let opened = open(&mut w, TileCoord { x: 15, y: 8 }, BorderEdge::East).expect("open");

        assert_eq!(w.money.cents(), before - BORDER_PORTAL_COST_CENTS);
        assert_eq!(
            w.ledger.total(MoneyCategory::Construction),
            -BORDER_PORTAL_COST_CENTS,
            "a portal is construction spend like any other"
        );
        assert!(opened.is_echo, "an echo is the default, not a fallback");
        assert!(!opened.town_name.is_empty());

        let link = w.registry.get(BorderEdge::East).expect("link");
        assert_eq!(link.portal_tile, TileCoord { x: 15, y: 8 });
        // The cache is warm from tick zero — there is nothing to wait for.
        assert!(link.their_offer().units_per_period >= 1);
        assert_eq!(link.crossings, 0);
        assert_eq!(link.maturity(), 0);
    }

    #[test]
    fn a_portal_is_a_real_commitment() {
        // Not a toggle: it costs more than the most expensive platform and more
        // than the most expensive train.
        assert!(BORDER_PORTAL_COST_CENTS > crate::stations::INTERCHANGE_COST_CENTS);
        assert!(BORDER_PORTAL_COST_CENTS > crate::trains::TRANSPORT_COST_CENTS);
    }

    #[test]
    fn one_link_per_edge() {
        let mut w = world();
        open(&mut w, TileCoord { x: 15, y: 8 }, BorderEdge::East).expect("first");
        let err = open(&mut w, TileCoord { x: 15, y: 9 }, BorderEdge::East).unwrap_err();
        assert_eq!(
            err,
            BorderError::EdgeAlreadyOpen {
                edge: BorderEdge::East
            }
        );
        assert_eq!(w.registry.len(), 1);
    }

    #[test]
    fn a_border_needs_a_line_that_reaches_it() {
        let mut w = world();
        // On the edge, but no track was ever laid at (15, 2).
        let err = open(&mut w, TileCoord { x: 15, y: 2 }, BorderEdge::East).unwrap_err();
        assert_eq!(err, BorderError::NoTrack);
        assert_eq!(w.money.cents(), 10_000_000, "a refusal must not charge");
    }

    #[test]
    fn a_border_needs_an_edge() {
        let mut w = world();
        let err = open(&mut w, TileCoord { x: 9, y: 8 }, BorderEdge::East).unwrap_err();
        assert_eq!(err, BorderError::NotOnEdge);
    }

    #[test]
    fn the_wrong_edge_names_both_edges() {
        let mut w = world();
        let err = open(&mut w, TileCoord { x: 15, y: 8 }, BorderEdge::North).unwrap_err();
        assert_eq!(
            err,
            BorderError::WrongEdge {
                tile_edge: BorderEdge::East,
                asked: BorderEdge::North,
            }
        );
        assert!(err.reason().contains("east"));
    }

    #[test]
    fn short_funds_refuse_and_name_the_price() {
        let mut w = world();
        w.money = Money::new(BORDER_PORTAL_COST_CENTS - 1);
        let err = open(&mut w, TileCoord { x: 15, y: 8 }, BorderEdge::East).unwrap_err();
        assert_eq!(
            err,
            BorderError::InsufficientFunds {
                need: BORDER_PORTAL_COST_CENTS,
                have: BORDER_PORTAL_COST_CENTS - 1,
            }
        );
        assert!(w.registry.is_empty());
        assert_eq!(w.money.cents(), BORDER_PORTAL_COST_CENTS - 1);
    }

    #[test]
    fn ground_layer_only() {
        let mut w = world();
        let err = try_open_border(
            &mut w.registry,
            &mut w.money,
            &mut w.ledger,
            &w.network,
            &w.terrain,
            TileCoord { x: 15, y: 8 },
            1,
            BorderEdge::East,
        )
        .unwrap_err();
        assert_eq!(err, BorderError::InvalidLayer);
    }

    #[test]
    fn closing_refunds_in_full_and_keeps_the_track() {
        let mut w = world();
        let before = w.money.cents();
        let track_before = w.network.len();
        open(&mut w, TileCoord { x: 15, y: 8 }, BorderEdge::East).expect("open");

        let closed = try_close_border(
            &mut w.registry,
            &mut w.money,
            &mut w.ledger,
            BorderEdge::East,
        )
        .expect("close");

        assert_eq!(closed.edge, BorderEdge::East);
        assert_eq!(w.money.cents(), before, "severing a link has no penalty");
        assert_eq!(w.network.len(), track_before, "your track is yours");
        assert!(w.registry.is_empty());
    }

    #[test]
    fn re_opening_an_edge_resumes_the_relationship() {
        let mut w = world();
        open(&mut w, TileCoord { x: 15, y: 8 }, BorderEdge::East).expect("open");
        {
            let link = w.registry.get_mut(BorderEdge::East).expect("link");
            link.crossings = 9;
            link.received_units = 21;
            link.accept_manifest(echo_manifest(42, BorderEdge::East, 6));
        }
        let matured = w.registry.get(BorderEdge::East).expect("link").maturity();

        try_close_border(
            &mut w.registry,
            &mut w.money,
            &mut w.ledger,
            BorderEdge::East,
        )
        .expect("close");
        open(&mut w, TileCoord { x: 15, y: 8 }, BorderEdge::East).expect("re-open");

        let link = w.registry.get(BorderEdge::East).expect("link");
        assert_eq!(link.crossings, 9, "severing a link is never destructive");
        assert_eq!(link.received_units, 21);
        assert_eq!(link.maturity(), matured);
        assert_eq!(link.neighbour.sequence, 6, "the cache survives too");
        assert_eq!(link.portal_tile, TileCoord { x: 15, y: 8 });
    }

    #[test]
    fn closing_a_closed_edge_says_so() {
        let mut w = world();
        assert_eq!(
            try_close_border(
                &mut w.registry,
                &mut w.money,
                &mut w.ledger,
                BorderEdge::West
            )
            .unwrap_err(),
            BorderError::EdgeClosed {
                edge: BorderEdge::West
            }
        );
    }

    #[test]
    fn four_edges_hold_four_neighbours() {
        let terrain = land(16, 16);
        let mut network = TrackNetwork::new();
        let mut cash = Money::new(10_000_000);
        let mut led = MoneyLedger::default();
        // A cross reaching all four boundaries.
        for x in 0..16 {
            try_place_track(
                &mut network,
                &mut cash,
                &mut led,
                &terrain,
                TileCoord { x, y: 8 },
                GROUND_LAYER,
            )
            .expect("track");
        }
        for y in 0..16 {
            if y == 8 {
                continue;
            }
            try_place_track(
                &mut network,
                &mut cash,
                &mut led,
                &terrain,
                TileCoord { x: 8, y },
                GROUND_LAYER,
            )
            .expect("track");
        }
        let mut w = World {
            registry: BorderRegistry::new(42),
            money: Money::new(10_000_000),
            ledger: MoneyLedger::default(),
            network,
            terrain,
        };

        open(&mut w, TileCoord { x: 8, y: 15 }, BorderEdge::North).expect("north");
        open(&mut w, TileCoord { x: 15, y: 8 }, BorderEdge::East).expect("east");
        open(&mut w, TileCoord { x: 8, y: 0 }, BorderEdge::South).expect("south");
        open(&mut w, TileCoord { x: 0, y: 8 }, BorderEdge::West).expect("west");

        assert_eq!(w.registry.len(), 4);
        let edges: Vec<BorderEdge> = w.registry.iter().map(|l| l.edge).collect();
        assert_eq!(edges, BorderEdge::ALL.to_vec(), "stored in edge order");
        let links: Vec<LinkId> = w.registry.iter().map(|l| l.link).collect();
        for i in 0..links.len() {
            for j in (i + 1)..links.len() {
                assert_ne!(links[i], links[j]);
            }
        }
    }

    #[test]
    fn the_railhead_is_where_the_line_meets_the_edge() {
        let w = world();
        assert_eq!(
            railhead_on_edge(&w.network, &w.terrain, BorderEdge::East, GROUND_LAYER),
            Some(TileCoord { x: 15, y: 8 })
        );
        // No line reaches the other three yet — that is `NoTrack`, said early.
        for edge in [BorderEdge::North, BorderEdge::South, BorderEdge::West] {
            assert_eq!(
                railhead_on_edge(&w.network, &w.terrain, edge, GROUND_LAYER),
                None
            );
        }
    }

    #[test]
    fn maturity_climbs_with_crossings_and_only_ever_pays_more() {
        let mut link = BorderLink::opened(
            BorderEdge::East,
            TileCoord { x: 15, y: 8 },
            GROUND_LAYER,
            42,
            0,
            BORDER_PORTAL_COST_CENTS,
        );
        let green = link.arrival_payout_cents(1);
        assert_eq!(green, BORDER_ARRIVAL_CENTS);

        link.crossings = MATURITY_CROSSINGS;
        assert_eq!(link.maturity(), 100);
        let mature = link.arrival_payout_cents(1);
        assert!(mature > green, "a long relationship trades at better rates");
        assert_eq!(
            mature,
            BORDER_ARRIVAL_CENTS + BORDER_ARRIVAL_CENTS * MATURITY_BONUS_PERCENT / 100
        );

        // Beyond the cap it stops climbing, and it never goes down.
        link.crossings = MATURITY_CROSSINGS * 100;
        assert_eq!(link.maturity(), 100);
        assert_eq!(link.arrival_payout_cents(1), mature);
    }

    #[test]
    fn a_stale_or_wrong_manifest_leaves_the_cache_alone() {
        let mut link = BorderLink::opened(
            BorderEdge::East,
            TileCoord { x: 15, y: 8 },
            GROUND_LAYER,
            42,
            0,
            0,
        );
        let cached = link.neighbour.clone();

        // Wrong link id.
        let mut stranger = echo_manifest(1, BorderEdge::West, 9);
        stranger.link = LinkId(999);
        assert!(!link.accept_manifest(stranger));
        assert_eq!(link.neighbour, cached);

        // Right link, unknown schema.
        let mut future = echo_manifest(42, BorderEdge::East, 9);
        future.schema_version = MANIFEST_SCHEMA_VERSION + 7;
        assert!(!link.accept_manifest(future));
        assert_eq!(link.neighbour, cached);

        // Right link, older sequence.
        let older = echo_manifest(42, BorderEdge::East, 0);
        assert!(!link.accept_manifest(older));
        assert_eq!(link.neighbour, cached);

        // Right link, newer: taken.
        let newer = echo_manifest(42, BorderEdge::East, 4);
        assert!(link.accept_manifest(newer.clone()));
        assert_eq!(link.neighbour, newer);
    }
}
