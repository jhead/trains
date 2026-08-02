//! Track piece data and light grade / curve constraints for later train speed.

use bevy_ecs::prelude::Component;
use serde::{Deserialize, Serialize};

use crate::ids::{TileCoord, TrackId};

use super::dir::TrackLinks;

/// Whether the piece sits on land or spans water.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackKind {
    Ground,
    Bridge,
}

/// One laid track tile (sim state; presentation mirrors this).
#[derive(Debug, Clone, PartialEq, Eq, Component, Serialize, Deserialize)]
pub struct TrackPiece {
    pub id: TrackId,
    pub tile: TileCoord,
    /// MVP: always [`super::cost::GROUND_LAYER`].
    pub layer: u8,
    pub kind: TrackKind,
    /// Terrain height cached at placement (for grade).
    pub height: i8,
    /// Which of 8 neighbors currently have track.
    pub links: TrackLinks,
    /// Max absolute height delta to a linked neighbor (0 = flat).
    /// Trains later: slow when this exceeds a threshold.
    pub max_grade: u8,
    /// Turn sharpness at this node (0 = straight / dead-end, higher = sharper).
    /// Derived from link directions; trains later: slow on high values.
    pub curve: u8,
    /// Cents charged when placed — full refund on demolish.
    pub paid_cents: i64,
}

impl TrackPiece {
    pub fn is_bridge(&self) -> bool {
        self.kind == TrackKind::Bridge
    }
}

/// Curve penalty from the set of link direction indices (0–7).
///
/// Dead-end / isolated → 0. Two opposite dirs → 0 (straight). Sharpest turn
/// among link pairs maps to 45°≈32, 90°≈64, 135°≈96.
pub fn curve_from_link_dirs(dirs: &[usize]) -> u8 {
    if dirs.len() < 2 {
        return 0;
    }
    let mut sharpest = 0u8;
    for i in 0..dirs.len() {
        for j in (i + 1)..dirs.len() {
            let a = dirs[i] as i8;
            let b = dirs[j] as i8;
            let mut d = (a - b).unsigned_abs() as u8;
            if d > 4 {
                d = 8 - d;
            }
            // d=4 means opposite (straight through); treat as 0 turn.
            let turn = if d == 4 { 0 } else { d };
            sharpest = sharpest.max(turn);
        }
    }
    sharpest.saturating_mul(32)
}
