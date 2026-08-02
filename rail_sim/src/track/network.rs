//! Authoritative track index for placement and train pathing.
//!
//! # The sixteen-direction graph
//!
//! Links come from [`DIR16`]: eight compass steps to tile-adjacent neighbours,
//! plus eight knight's-move half-steps that reach a tile two along one axis and
//! one along the other (brief 01 §5.2). A linked neighbour is therefore no
//! longer always tile-adjacent.
//!
//! ## Tiles a half-step passes over
//!
//! A knight's move from tile centre to tile centre crosses exactly two other
//! tiles — `(0,0) → (2,1)` runs over `(1,0)` and `(1,1)`, a quarter of its
//! length in each. The ballast of such a link genuinely lies on top of them.
//!
//! **The rule: a half-step link exists only while both tiles it passes over are
//! free of track on that layer.** The link is refused, not the build. Build on
//! one of those tiles later and the shortcut simply stops existing; the two
//! tiles are now themselves track and the network routes through them by
//! ordinary compass steps, which is the same path on the ground.
//!
//! It is stated this way round rather than "the link occupies the tiles"
//! because:
//!
//! - It stores nothing. The predicate is a pure function of tile occupancy, so
//!   there is no reservation state to persist, to keep in sync, or to leak when
//!   a piece is removed.
//! - It is symmetric by construction — both endpoints compute the same two
//!   tiles ([`intermediate_tiles`]) — so the graph cannot go half-linked.
//! - It never refuses a build. Reserving tiles would make a stretch of empty
//!   ground silently unbuildable because of a link two tiles away, which is the
//!   kind of rule players cannot see and cannot learn.
//! - It suppresses exactly the spurious links. A half-step's two tiles are the
//!   tiles of the two compass steps it straddles, so anywhere track is already
//!   dense — parallel running lines, a passing loop, a filled junction — every
//!   half-step is refused automatically. Half-steps only appear where there
//!   really is a shallow jump across bare ground.
//!
//! That last property also fixes the junction geometry: a node can never hold a
//! half-step and either compass step beside it, so the tightest turnout the grid
//! can produce is two rose steps. See [`turnout_divergence_ok`].
//!
//! ## Relinking
//!
//! Link sets are *derived*, never patched. When a tile changes, every piece
//! whose link set can change is recomputed from scratch: the tile itself and its
//! sixteen [`DIR16`] neighbours. That covers direct neighbours (they gain or
//! lose a link to the edited tile) and the endpoints of any half-step that
//! passes over it — both of those are compass neighbours of the edited tile, so
//! both are inside the same set.
//!
//! # For the trains slice
//! - [`TrackNetwork::piece`] / [`TrackNetwork::at`] — look up a node
//! - [`TrackNetwork::neighbor_ids`] — 16-dir graph edges via [`TrackPiece::links`]
//! - [`TrackNetwork::iter`] — scan the whole graph
//! - Read [`TrackPiece::max_grade`] / [`TrackPiece::curve`] for speed modifiers
//!
//! [`turnout_divergence_ok`]: super::rules::turnout_divergence_ok

use std::collections::HashMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::ids::{TileCoord, TrackId};

use super::dir::{dir_index, intermediate_tiles, opposite_dir, step, TrackLinks, DIR_COUNT};
use super::piece::{curve_from_link_dirs, TrackPiece};

/// All laid track, keyed by stable [`TrackId`] and by tile+layer.
#[derive(Debug, Clone, Default, PartialEq, Resource, Serialize, Deserialize)]
pub struct TrackNetwork {
    pieces: HashMap<TrackId, TrackPiece>,
    /// `(x, y, layer)` → id
    at_tile: HashMap<(i32, i32, u8), TrackId>,
    next_id: u64,
}

impl TrackNetwork {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.pieces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &TrackPiece> {
        self.pieces.values()
    }

    pub fn piece(&self, id: TrackId) -> Option<&TrackPiece> {
        self.pieces.get(&id)
    }

    pub fn piece_mut(&mut self, id: TrackId) -> Option<&mut TrackPiece> {
        self.pieces.get_mut(&id)
    }

    pub fn at(&self, tile: TileCoord, layer: u8) -> Option<&TrackPiece> {
        let id = *self.at_tile.get(&(tile.x, tile.y, layer))?;
        self.pieces.get(&id)
    }

    pub fn id_at(&self, tile: TileCoord, layer: u8) -> Option<TrackId> {
        self.at_tile.get(&(tile.x, tile.y, layer)).copied()
    }

    /// Neighbor track ids for graph traversal (only dirs marked in `links`).
    ///
    /// Half-step neighbours are two tiles away on one axis and one on the other;
    /// callers that walk this graph get them for free and do not need to know.
    pub fn neighbor_ids(&self, id: TrackId) -> Vec<TrackId> {
        let Some(piece) = self.pieces.get(&id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for i in 0..DIR_COUNT {
            if !piece.links.has(i) {
                continue;
            }
            let n = step(piece.tile, i);
            if let Some(nid) = self.id_at(n, piece.layer) {
                out.push(nid);
            }
        }
        out
    }

    /// Whether a link from `tile` in `dir` may exist, ignoring whether the far
    /// end has track.
    ///
    /// Compass steps always may. A half-step may only while both tiles it
    /// passes over are free of track — see the module docs.
    pub fn link_clear(&self, tile: TileCoord, layer: u8, dir: usize) -> bool {
        match intermediate_tiles(tile, dir) {
            None => true,
            Some(mids) => mids
                .iter()
                .all(|m| !self.at_tile.contains_key(&(m.x, m.y, layer))),
        }
    }

    /// Tiles a half-step from `tile` would pass over that already carry track.
    ///
    /// Empty for compass steps and for clear half-steps. Placement uses this to
    /// explain a refused shallow run rather than charging for one that cannot
    /// link up.
    pub fn blocked_intermediates(&self, tile: TileCoord, layer: u8, dir: usize) -> Vec<TileCoord> {
        match intermediate_tiles(tile, dir) {
            None => Vec::new(),
            Some(mids) => mids
                .into_iter()
                .filter(|m| self.at_tile.contains_key(&(m.x, m.y, layer)))
                .collect(),
        }
    }

    /// The link set a piece at `tile` would have, derived purely from occupancy.
    ///
    /// Pure: safe to call for a tile that has no piece yet, which is how
    /// placement validation previews the junction a build would create.
    pub fn links_for(&self, tile: TileCoord, layer: u8) -> TrackLinks {
        let mut links = TrackLinks::empty();
        for i in 0..DIR_COUNT {
            let n = step(tile, i);
            if self.at_tile.contains_key(&(n.x, n.y, layer)) && self.link_clear(tile, layer, i) {
                links.set(i);
            }
        }
        links
    }

    pub(crate) fn alloc_id(&mut self) -> TrackId {
        self.next_id = self.next_id.saturating_add(1);
        TrackId(self.next_id)
    }

    /// Insert a new piece and refresh links/grade/curve across its neighbourhood.
    pub(crate) fn insert_piece(&mut self, piece: TrackPiece) -> TrackId {
        let id = piece.id;
        let tile = piece.tile;
        let layer = piece.layer;
        self.at_tile.insert((tile.x, tile.y, layer), id);
        self.pieces.insert(id, piece);
        self.relink_around(tile, layer);
        id
    }

    /// Remove by id; refund amount is the caller's job. Returns the removed piece.
    pub(crate) fn remove_piece(&mut self, id: TrackId) -> Option<TrackPiece> {
        let piece = self.pieces.remove(&id)?;
        self.at_tile
            .remove(&(piece.tile.x, piece.tile.y, piece.layer));
        // Removing a tile can *create* half-steps that used to run across it, as
        // well as dropping its own links, so the whole neighbourhood is redone.
        self.relink_around(piece.tile, piece.layer);
        Some(piece)
    }

    /// Every piece whose link set can change when `tile` gains or loses track.
    ///
    /// The tile itself, plus its sixteen [`DIR16`](super::dir::DIR16)
    /// neighbours. Endpoints of a half-step passing over `tile` are compass
    /// neighbours of it, so they are already in this set.
    fn affected_tiles(tile: TileCoord) -> Vec<TileCoord> {
        let mut out = Vec::with_capacity(DIR_COUNT + 1);
        out.push(tile);
        for i in 0..DIR_COUNT {
            out.push(step(tile, i));
        }
        out
    }

    /// Recompute links, then grade and curve, for `tile` and its neighbourhood.
    ///
    /// Two passes because a piece's metrics read its own (new) link set. The
    /// link predicate is symmetric, so deriving every piece independently gives
    /// a consistent graph with no reciprocal-bit patching.
    pub(crate) fn relink_around(&mut self, tile: TileCoord, layer: u8) {
        let affected = Self::affected_tiles(tile);
        for &t in &affected {
            self.recompute_links(t, layer);
        }
        for &t in &affected {
            self.refresh_metrics(t, layer);
        }
    }

    fn recompute_links(&mut self, tile: TileCoord, layer: u8) {
        let Some(id) = self.id_at(tile, layer) else {
            return;
        };
        let links = self.links_for(tile, layer);
        if let Some(p) = self.pieces.get_mut(&id) {
            p.links = links;
        }
    }

    /// Recompute `max_grade` and `curve` from the piece's current link set.
    ///
    /// Grade stays the raw absolute height delta to a linked neighbour, exactly
    /// as on the eight-direction graph. A half-step covers √5 tiles, so the same
    /// delta is really a *gentler* climb than over an orthogonal step — holding
    /// it to the same number is the conservative direction, and it keeps
    /// [`MAX_GRADE`](super::cost::MAX_GRADE) and the train profiles meaning what
    /// they meant.
    fn refresh_metrics(&mut self, tile: TileCoord, layer: u8) {
        let Some(id) = self.id_at(tile, layer) else {
            return;
        };
        let our_height = self.pieces.get(&id).map(|p| p.height).unwrap_or(0);
        let links = self.pieces.get(&id).map(|p| p.links).unwrap_or_default();
        let mut link_dirs = Vec::new();
        let mut max_grade = 0u8;
        for i in 0..DIR_COUNT {
            if !links.has(i) {
                continue;
            }
            link_dirs.push(i);
            let n = step(tile, i);
            if let Some(np) = self.at(n, layer) {
                let dh = (np.height as i16 - our_height as i16).unsigned_abs() as u8;
                max_grade = max_grade.max(dh);
            }
        }
        let curve = curve_from_link_dirs(&link_dirs);
        if let Some(p) = self.pieces.get_mut(&id) {
            p.max_grade = max_grade;
            p.curve = curve;
        }
    }

    /// Clear the link bits between `from` and `toward`, then refresh metrics.
    ///
    /// Advisory only — the next [`Self::relink_around`] re-derives them. Kept for
    /// callers that want a one-off cut without touching occupancy.
    #[allow(dead_code)]
    pub(crate) fn clear_link_toward(&mut self, from: TileCoord, layer: u8, toward: TileCoord) {
        let Some(id) = self.id_at(from, layer) else {
            return;
        };
        if let Some(i) = dir_index(from, toward) {
            if let Some(p) = self.pieces.get_mut(&id) {
                p.links.clear(i);
            }
            if let Some(nid) = self.id_at(toward, layer) {
                if let Some(np) = self.pieces.get_mut(&nid) {
                    np.links.clear(opposite_dir(i));
                }
            }
            self.refresh_metrics(from, layer);
            self.refresh_metrics(toward, layer);
        }
    }

    /// Every link has a matching link back, and no link is stale.
    #[cfg(test)]
    pub(crate) fn is_symmetric(&self) -> bool {
        self.pieces.values().all(|p| {
            (0..DIR_COUNT).all(|i| {
                let n = step(p.tile, i);
                let back = self
                    .at(n, p.layer)
                    .is_some_and(|np| np.links.has(opposite_dir(i)));
                p.links.has(i) == back
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economy::MoneyLedger;
    use crate::money::Money;
    use crate::track::dir::{is_half_step, DIR16};
    use crate::track::{try_demolish, try_place_track, TrackTerrain, GROUND_LAYER};

    fn land(w: u32, h: u32) -> TrackTerrain {
        TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 0i8)))
    }

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    struct Yard {
        network: TrackNetwork,
        terrain: TrackTerrain,
        money: Money,
        ledger: MoneyLedger,
    }

    impl Yard {
        fn new(w: u32, h: u32) -> Self {
            Self {
                network: TrackNetwork::new(),
                terrain: land(w, h),
                money: Money::new(50_000_000),
                ledger: MoneyLedger::default(),
            }
        }

        fn lay(&mut self, x: i32, y: i32) -> TrackId {
            try_place_track(
                &mut self.network,
                &mut self.money,
                &mut self.ledger,
                &self.terrain,
                tile(x, y),
                GROUND_LAYER,
            )
            .expect("place")
            .id
        }

        fn pull(&mut self, id: TrackId) {
            try_demolish(&mut self.network, &mut self.money, &mut self.ledger, id)
                .expect("demolish");
        }

        fn links_at(&self, x: i32, y: i32) -> TrackLinks {
            self.network.at(tile(x, y), GROUND_LAYER).unwrap().links
        }
    }

    #[test]
    fn a_knights_move_pair_links_over_bare_ground() {
        let mut yard = Yard::new(12, 12);
        let a = yard.lay(2, 2);
        let b = yard.lay(4, 3);

        assert_eq!(yard.network.neighbor_ids(a), vec![b]);
        assert_eq!(yard.network.neighbor_ids(b), vec![a]);
        assert!(yard.links_at(2, 2).has_half_step());
        assert!(yard.network.is_symmetric());
        // ENE from (2,2).
        assert_eq!(DIR16[9], (2, 1));
        assert!(yard.links_at(2, 2).has(9));
        assert!(yard.links_at(4, 3).has(opposite_dir(9)));
    }

    /// The rule, both ways round: occupy a crossed tile and the shortcut goes.
    #[test]
    fn building_across_a_half_step_breaks_it_and_demolishing_restores_it() {
        let mut yard = Yard::new(12, 12);
        let a = yard.lay(2, 2);
        let b = yard.lay(4, 3);
        assert_eq!(yard.network.neighbor_ids(a), vec![b]);

        // (3, 2) is one of the two tiles the shallow link runs over.
        let blocker = yard.lay(3, 2);
        assert!(
            !yard.links_at(2, 2).has(9),
            "half-step must drop when its ground is built on"
        );
        assert!(!yard.links_at(4, 3).has(opposite_dir(9)));
        assert!(yard.network.is_symmetric(), "no stale reciprocal bit");
        // The route is still there, now as two ordinary steps.
        assert!(yard.network.neighbor_ids(a).contains(&blocker));
        assert!(yard.network.neighbor_ids(b).contains(&blocker));

        yard.pull(blocker);
        assert!(
            yard.links_at(2, 2).has(9),
            "removing the obstruction restores the shallow link"
        );
        assert!(yard.network.is_symmetric());
    }

    #[test]
    fn a_half_step_needs_both_crossed_tiles_clear() {
        // (2,2) → (4,3) crosses (3,2) and (3,3). Either one is enough to stop it.
        for blocker in [(3, 2), (3, 3)] {
            let mut yard = Yard::new(12, 12);
            yard.lay(2, 2);
            yard.lay(4, 3);
            yard.lay(blocker.0, blocker.1);
            assert!(
                !yard.links_at(2, 2).has(9),
                "{blocker:?} should block the half-step"
            );
            assert!(yard.network.is_symmetric());
        }
    }

    /// Density suppresses half-steps on its own — the property that keeps them
    /// from appearing all over a double-track corridor.
    #[test]
    fn parallel_running_lines_grow_no_half_steps() {
        let mut yard = Yard::new(16, 8);
        for x in 1..=8 {
            yard.lay(x, 3);
        }
        for x in 1..=8 {
            yard.lay(x, 4);
        }
        for x in 1..=8 {
            for y in [3, 4] {
                assert!(
                    !yard.links_at(x, y).has_half_step(),
                    "({x},{y}) grew a spurious shallow link"
                );
            }
        }
        assert!(yard.network.is_symmetric());
    }

    /// A shallow line reads as one continuous run of half-steps.
    #[test]
    fn a_shallow_staircase_chains_end_to_end() {
        let mut yard = Yard::new(16, 12);
        let ids: Vec<TrackId> = (0..4).map(|i| yard.lay(1 + 2 * i, 1 + i)).collect();
        for (i, &id) in ids.iter().enumerate() {
            let want = match i {
                0 | 3 => 1,
                _ => 2,
            };
            assert_eq!(
                yard.network.neighbor_ids(id).len(),
                want,
                "piece {i} of the shallow run"
            );
        }
        // Every link along the run is a half-step, and the middles read straight.
        for &id in &ids[1..3] {
            let piece = yard.network.piece(id).unwrap();
            assert!(piece.links.dirs().all(is_half_step));
            assert_eq!(piece.curve, 0, "a shallow run is not a curve");
        }
        assert!(yard.network.is_symmetric());
    }

    /// The geometric guarantee the junction rule leans on: no node can hold a
    /// half-step next to a compass step, so the tightest turnout is two steps.
    #[test]
    fn no_node_can_hold_two_legs_one_rose_step_apart() {
        use crate::track::dir::clock_separation;
        let mut yard = Yard::new(24, 24);
        // A messy yard: a line, a branch, a couple of isolated shallow partners.
        for x in 3..=10 {
            yard.lay(x, 8);
        }
        for y in 9..=12 {
            yard.lay(6, y);
        }
        for (x, y) in [(12, 9), (14, 10), (16, 11), (5, 14), (7, 15), (2, 5)] {
            yard.lay(x, y);
        }
        assert!(yard.network.is_symmetric());

        for piece in yard.network.iter() {
            let dirs: Vec<usize> = piece.links.dirs().collect();
            for i in 0..dirs.len() {
                for j in (i + 1)..dirs.len() {
                    assert!(
                        clock_separation(dirs[i], dirs[j]) >= 2,
                        "{:?} holds legs {} and {} one rose step apart",
                        piece.tile,
                        dirs[i],
                        dirs[j]
                    );
                }
            }
        }
    }

    #[test]
    fn removing_a_piece_leaves_no_dangling_links() {
        let mut yard = Yard::new(16, 16);
        let ids: Vec<TrackId> = (1..=6).map(|x| yard.lay(x, 5)).collect();
        let shallow = yard.lay(3, 7); // knight partner of (1,6)? no — of (4,6)/(2,6)
        yard.pull(ids[2]);
        assert!(yard.network.is_symmetric());
        yard.pull(shallow);
        assert!(yard.network.is_symmetric());
        for &id in &ids {
            if let Some(p) = yard.network.piece(id) {
                for d in p.links.dirs() {
                    assert!(
                        yard.network.id_at(step(p.tile, d), p.layer).is_some(),
                        "link {d} from {:?} points at nothing",
                        p.tile
                    );
                }
            }
        }
    }

    #[test]
    fn links_for_previews_a_tile_that_is_not_built_yet() {
        let mut yard = Yard::new(12, 12);
        yard.lay(4, 3);
        let preview = yard.network.links_for(tile(2, 2), GROUND_LAYER);
        assert!(preview.has(9), "(2,2) would link ENE to (4,3)");
        assert!(yard.network.id_at(tile(2, 2), GROUND_LAYER).is_none());

        let built = yard.lay(2, 2);
        assert_eq!(yard.network.piece(built).unwrap().links, preview);
    }

    #[test]
    fn blocked_intermediates_names_what_is_in_the_way() {
        let mut yard = Yard::new(12, 12);
        yard.lay(3, 2);
        let blocked = yard
            .network
            .blocked_intermediates(tile(2, 2), GROUND_LAYER, 9);
        assert_eq!(blocked, vec![tile(3, 2)]);
        assert!(!yard.network.link_clear(tile(2, 2), GROUND_LAYER, 9));
        // Compass steps cross nothing and are always clear.
        assert!(yard.network.link_clear(tile(2, 2), GROUND_LAYER, 2));
        assert!(yard
            .network
            .blocked_intermediates(tile(2, 2), GROUND_LAYER, 2)
            .is_empty());
    }
}
