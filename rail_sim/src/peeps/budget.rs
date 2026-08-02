//! Bounded full simulation, biased toward wherever the camera is looking.
//!
//! Brief 06 §4.1: *"Simulate a bounded set of peeps in full … and abstract the
//! rest into district-level flow. … Which peeps get simulated in full is biased
//! toward wherever the camera is looking, so the world is always at its most
//! alive exactly where the player is watching."*
//!
//! The sim never reads a camera. Presentation writes [`PeepFocus`]; this module
//! only knows about a tile and a radius, which keeps `rail_sim` free of any
//! rendering dependency and keeps the rule testable without a window.

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::ids::TileCoord;
use crate::stations::StationRegistry;

use super::journey::{JourneyStage, PeepPosition};
use super::routine::Routine;
use super::Journey;

/// Hard cap on peeps simulated in full detail (journeys, positions, sprites).
pub const MAX_DETAILED_PEEPS: usize = 64;

/// Radius in tiles used when presentation has not published a viewport yet.
pub const DEFAULT_FOCUS_RADIUS: i32 = 24;

/// Ticks between level-of-detail reshuffles. Cheap, but not every tick — peeps
/// popping between detail levels every frame would churn sprites.
pub const DETAIL_REBALANCE_TICKS: u32 = 30;

/// Region of interest published by the presentation layer.
///
/// `rail_town` sets this from the map camera each frame. Leaving `center` as
/// `None` is legal — the sim then falls back to a stable id ordering, so
/// headless tests and the dedicated server behave deterministically.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeepFocus {
    /// Tile at the centre of the player's view.
    pub center: Option<TileCoord>,
    /// Half-extent of the view in tiles.
    pub radius: i32,
}

impl Default for PeepFocus {
    fn default() -> Self {
        Self {
            center: None,
            radius: DEFAULT_FOCUS_RADIUS,
        }
    }
}

impl PeepFocus {
    /// Publish a viewport (presentation-side helper).
    pub fn look_at(&mut self, center: TileCoord, radius: i32) {
        self.center = Some(center);
        self.radius = radius.max(1);
    }

    pub fn clear(&mut self) {
        self.center = None;
    }

    /// Chebyshev distance from the focus centre, or `None` when unfocused.
    pub fn distance_to(&self, tile: TileCoord) -> Option<i32> {
        let c = self.center?;
        Some((c.x - tile.x).abs().max((c.y - tile.y).abs()))
    }

    /// True when the tile is inside the published viewport.
    pub fn contains(&self, tile: TileCoord) -> bool {
        self.distance_to(tile).is_some_and(|d| d <= self.radius)
    }

    /// Sort key for full-detail selection — lower wins.
    ///
    /// On-screen peeps sort first by distance to the centre; everyone else
    /// sorts behind them but still in distance order, so panning promotes the
    /// people the camera is heading toward before it arrives.
    pub fn priority(&self, tile: TileCoord) -> i64 {
        match self.distance_to(tile) {
            Some(d) if d <= self.radius => d as i64,
            Some(d) => 1_000_000 + d as i64,
            None => 2_000_000,
        }
    }
}

/// The bounded-simulation budget.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeepBudget {
    /// Maximum peeps simulated in full detail.
    pub max_detailed: usize,
    /// Ticks between reshuffles.
    pub rebalance_every: u32,
    ticks: u32,
    /// How many peeps were full-detail after the last reshuffle (read-only).
    pub detailed: usize,
    /// How many peeps are abstracted into district flow (read-only).
    pub abstracted: usize,
}

impl Default for PeepBudget {
    fn default() -> Self {
        Self {
            max_detailed: MAX_DETAILED_PEEPS,
            rebalance_every: DETAIL_REBALANCE_TICKS,
            ticks: 0,
            detailed: 0,
            abstracted: 0,
        }
    }
}

impl PeepBudget {
    /// Total residents the sim is tracking, at either detail level.
    pub fn population(&self) -> usize {
        self.detailed + self.abstracted
    }

    fn due(&mut self) -> bool {
        self.ticks = self.ticks.saturating_add(1);
        if self.ticks >= self.rebalance_every.max(1) {
            self.ticks = 0;
            true
        } else {
            false
        }
    }

    /// Force the next tick to reshuffle (used when population changes).
    pub fn invalidate(&mut self) {
        self.ticks = u32::MAX;
    }
}

/// Level of detail a peep is currently simulated at.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PeepDetail {
    /// Full journey state machine, a real position, and a sprite.
    Full,
    /// Folded into district-level flow — still demands service and complains,
    /// but has no position and draws nothing.
    Abstract,
}

impl Default for PeepDetail {
    fn default() -> Self {
        Self::Abstract
    }
}

impl PeepDetail {
    pub fn is_full(self) -> bool {
        matches!(self, Self::Full)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "simulated",
            Self::Abstract => "abstracted",
        }
    }
}

/// Choose which peeps run in full detail this window.
///
/// Pure ranking helper — takes `(entity_key, tile)` pairs and returns the keys
/// that should be full detail, so the rule can be tested without a `World`.
pub fn select_detailed<K: Copy + Ord>(
    focus: &PeepFocus,
    candidates: &mut Vec<(K, TileCoord)>,
    max_detailed: usize,
) -> Vec<K> {
    candidates.sort_by_key(|(key, tile)| (focus.priority(*tile), *key));
    candidates
        .iter()
        .take(max_detailed)
        .map(|(key, _)| *key)
        .collect()
}

/// Re-rank peeps into [`PeepDetail::Full`] / [`PeepDetail::Abstract`].
///
/// Promotions snap the peep's position to somewhere sensible for their current
/// stage, so a peep who walks on screen never appears mid-air.
pub fn rebalance_peep_detail(
    focus: Res<PeepFocus>,
    stations: Res<StationRegistry>,
    mut budget: ResMut<PeepBudget>,
    mut peeps: Query<(
        Entity,
        &super::Peep,
        &Routine,
        &Journey,
        &mut PeepPosition,
        &mut PeepDetail,
    )>,
) {
    if !budget.due() {
        return;
    }

    let mut candidates: Vec<(Entity, TileCoord)> = peeps
        .iter()
        .map(|(entity, peep, routine, journey, pos, detail)| {
            // Rank on where they actually are when the position is live, else on
            // where their current stage says they must be.
            let tile = if detail.is_full() && journey.stage != JourneyStage::AtHome {
                pos.tile()
            } else {
                journey.anchor_tile(
                    peep.home,
                    routine.destination,
                    stations.get(journey.from_station).map(|s| s.tile),
                )
            };
            (entity, tile)
        })
        .collect();

    let chosen = select_detailed(&focus, &mut candidates, budget.max_detailed);
    let chosen: std::collections::HashSet<Entity> = chosen.into_iter().collect();

    let mut detailed = 0usize;
    let mut abstracted = 0usize;
    for (entity, peep, routine, journey, mut pos, mut detail) in peeps.iter_mut() {
        let want = if chosen.contains(&entity) {
            PeepDetail::Full
        } else {
            PeepDetail::Abstract
        };
        if want.is_full() {
            detailed += 1;
        } else {
            abstracted += 1;
        }
        if *detail == want {
            continue;
        }
        if want.is_full() {
            // Coming back into view — put them where their stage says they are.
            let tile = match journey.stage {
                JourneyStage::WaitingOnPlatform | JourneyStage::Boarding => stations
                    .get(journey.from_station)
                    .map(|s| s.tile)
                    .unwrap_or(peep.home),
                JourneyStage::SpendingTime => routine.destination,
                _ => peep.home,
            };
            *pos = PeepPosition::at_tile(tile, peep.id.0);
        }
        *detail = want;
    }

    budget.detailed = detailed;
    budget.abstracted = abstracted;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    #[test]
    fn focus_biases_selection_toward_the_camera() {
        let mut focus = PeepFocus::default();
        focus.look_at(tile(50, 50), 10);

        let mut candidates = vec![
            (1u64, tile(0, 0)),
            (2, tile(50, 50)),
            (3, tile(52, 49)),
            (4, tile(90, 90)),
            (5, tile(45, 55)),
        ];
        let picked = select_detailed(&focus, &mut candidates, 3);
        assert_eq!(picked, vec![2, 3, 5]);
    }

    #[test]
    fn budget_caps_full_detail() {
        let focus = PeepFocus::default();
        let mut candidates: Vec<(u64, TileCoord)> =
            (0..500).map(|i| (i, tile(i as i32 % 64, 3))).collect();
        let picked = select_detailed(&focus, &mut candidates, MAX_DETAILED_PEEPS);
        assert_eq!(picked.len(), MAX_DETAILED_PEEPS);
    }

    #[test]
    fn unfocused_selection_is_deterministic() {
        let focus = PeepFocus::default();
        let make = || -> Vec<(u64, TileCoord)> {
            (0..20)
                .rev()
                .map(|i| (i, tile(i as i32, i as i32)))
                .collect()
        };
        let mut a = make();
        let mut b = make();
        assert_eq!(
            select_detailed(&focus, &mut a, 5),
            select_detailed(&focus, &mut b, 5)
        );
        assert_eq!(select_detailed(&focus, &mut a, 5), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn peeps_outside_the_viewport_rank_behind_those_inside() {
        let mut focus = PeepFocus::default();
        focus.look_at(tile(10, 10), 4);
        assert!(focus.contains(tile(13, 10)));
        assert!(!focus.contains(tile(20, 10)));
        assert!(focus.priority(tile(13, 10)) < focus.priority(tile(20, 10)));
    }

    #[test]
    fn budget_rebalance_is_periodic() {
        let mut budget = PeepBudget {
            rebalance_every: 3,
            ..Default::default()
        };
        assert!(!budget.due());
        assert!(!budget.due());
        assert!(budget.due());
        assert!(!budget.due());
        budget.invalidate();
        assert!(budget.due());
    }
}
