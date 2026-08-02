//! Authoritative track index for placement and (later) train pathing.

use std::collections::HashMap;

use bevy_ecs::prelude::Resource;

use crate::ids::{TileCoord, TrackId};

use super::dir::{dir_index, opposite_dir, step, DIR8};
use super::piece::{curve_from_link_dirs, TrackPiece};

/// All laid track, keyed by stable [`TrackId`] and by tile+layer.
///
/// # For the trains slice
/// - [`TrackNetwork::piece`] / [`TrackNetwork::at`] — look up a node
/// - [`TrackNetwork::neighbor_ids`] — 8-dir graph edges via [`TrackPiece::links`]
/// - [`TrackNetwork::iter`] — scan the whole graph
/// - Read [`TrackPiece::max_grade`] / [`TrackPiece::curve`] for speed modifiers
#[derive(Debug, Clone, Default, Resource)]
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
    pub fn neighbor_ids(&self, id: TrackId) -> Vec<TrackId> {
        let Some(piece) = self.pieces.get(&id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (i, _) in DIR8.iter().enumerate() {
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

    pub(crate) fn alloc_id(&mut self) -> TrackId {
        self.next_id = self.next_id.saturating_add(1);
        TrackId(self.next_id)
    }

    /// Insert a new piece and refresh links/grade/curve with neighbors.
    pub(crate) fn insert_piece(&mut self, piece: TrackPiece) -> TrackId {
        let id = piece.id;
        let tile = piece.tile;
        let layer = piece.layer;
        self.at_tile.insert((tile.x, tile.y, layer), id);
        self.pieces.insert(id, piece);
        self.relink_tile(tile, layer);
        // Relink may have updated the piece; ensure we return the id.
        let _ = self.pieces.get(&id);
        id
    }

    /// Remove by id; refund amount is the caller's job. Returns the removed piece.
    pub(crate) fn remove_piece(&mut self, id: TrackId) -> Option<TrackPiece> {
        let piece = self.pieces.remove(&id)?;
        self.at_tile
            .remove(&(piece.tile.x, piece.tile.y, piece.layer));
        // Clear neighbor links that pointed here, then refresh their grade/curve.
        let neighbors: Vec<(TileCoord, u8)> = DIR8
            .iter()
            .enumerate()
            .filter_map(|(i, _)| {
                let n = step(piece.tile, i);
                self.id_at(n, piece.layer).map(|_| (n, piece.layer))
            })
            .collect();
        for (n_tile, layer) in neighbors {
            self.relink_tile(n_tile, layer);
        }
        Some(piece)
    }

    /// Recompute links, max_grade, and curve for the piece at `tile` (if any)
    /// and ensure symmetric neighbor bits.
    pub(crate) fn relink_tile(&mut self, tile: TileCoord, layer: u8) {
        let Some(id) = self.id_at(tile, layer) else {
            return;
        };

        // First pass: set our links based on who exists.
        let mut links = super::dir::TrackLinks::empty();
        let mut link_dirs = Vec::new();
        let mut max_grade = 0u8;
        let our_height = self.pieces.get(&id).map(|p| p.height).unwrap_or(0);

        for (i, _) in DIR8.iter().enumerate() {
            let n = step(tile, i);
            if let Some(nid) = self.id_at(n, layer) {
                links.set(i);
                link_dirs.push(i);
                if let Some(np) = self.pieces.get(&nid) {
                    let dist = if DIR8[i].0 != 0 && DIR8[i].1 != 0 {
                        // Diagonal: approximate grade with Δh (same units as ortho for MVP).
                        1
                    } else {
                        1
                    };
                    let _ = dist;
                    let dh = (np.height as i16 - our_height as i16).unsigned_abs() as u8;
                    max_grade = max_grade.max(dh);
                }
                // Ensure neighbor has reciprocal link.
                let opp = opposite_dir(i);
                if let Some(np) = self.pieces.get_mut(&nid) {
                    if !np.links.has(opp) {
                        np.links.set(opp);
                    }
                }
            }
        }

        let curve = curve_from_link_dirs(&link_dirs);
        if let Some(p) = self.pieces.get_mut(&id) {
            p.links = links;
            p.max_grade = max_grade;
            p.curve = curve;
        }

        // Refresh neighbor grade/curve (their max_grade may include us).
        for (i, _) in DIR8.iter().enumerate() {
            if !links.has(i) {
                continue;
            }
            let n = step(tile, i);
            self.refresh_metrics(n, layer);
        }
    }

    fn refresh_metrics(&mut self, tile: TileCoord, layer: u8) {
        let Some(id) = self.id_at(tile, layer) else {
            return;
        };
        let our_height = self.pieces.get(&id).map(|p| p.height).unwrap_or(0);
        let links = self.pieces.get(&id).map(|p| p.links).unwrap_or_default();
        let mut link_dirs = Vec::new();
        let mut max_grade = 0u8;
        for (i, _) in DIR8.iter().enumerate() {
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

    /// Clear reciprocal link bits when a neighbor was removed (handled via relink).
    #[allow(dead_code)]
    pub(crate) fn clear_link_toward(&mut self, from: TileCoord, layer: u8, toward: TileCoord) {
        let Some(id) = self.id_at(from, layer) else {
            return;
        };
        if let Some(i) = dir_index(from, toward) {
            if let Some(p) = self.pieces.get_mut(&id) {
                p.links.clear(i);
            }
            self.refresh_metrics(from, layer);
        }
    }
}
