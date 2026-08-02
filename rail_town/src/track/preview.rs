//! Pure cost / validity preview for build and demolish ghosts.

use rail_sim::ids::TileCoord;
use rail_sim::{
    path_bridge_spans_ok, tile_cost, validate_tile_empty, Money, PlacementError, TrackNetwork,
    TrackTerrain, GROUND_LAYER, MAX_BRIDGE_SPAN,
};

use super::propose::{propose_path, PathMode, ProposedPath};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileGhostKind {
    /// New track that would be placed.
    Place { cost_cents: i64, is_bridge: bool },
    /// Already has track — skipped by autofill.
    Existing,
    /// Illegal for a placeable reason.
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhostTile {
    pub tile: TileCoord,
    pub kind: TileGhostKind,
    pub valid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectInfo {
    pub message: String,
    pub tiles: Vec<TileCoord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPreview {
    pub tiles: Vec<GhostTile>,
    pub new_tile_count: u32,
    pub bridge_count: u32,
    pub total_cost_cents: i64,
    pub balance_after_cents: i64,
    pub can_commit: bool,
    pub reject: Option<RejectInfo>,
    pub endpoint: TileCoord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemolishPreview {
    pub tiles: Vec<TileCoord>,
    pub refund_cents: i64,
    pub balance_after_cents: i64,
    pub track_count: u32,
    pub can_commit: bool,
    pub reject: Option<RejectInfo>,
    pub endpoint: TileCoord,
}

/// Preview a build path from anchor → cursor.
pub fn preview_build(
    network: &TrackNetwork,
    terrain: &TrackTerrain,
    money: &Money,
    from: TileCoord,
    to: TileCoord,
    mode: PathMode,
) -> BuildPreview {
    let proposed = propose_path(from, to, mode);
    preview_build_proposed(network, terrain, money, proposed)
}

fn preview_build_proposed(
    network: &TrackNetwork,
    terrain: &TrackTerrain,
    money: &Money,
    proposed: ProposedPath,
) -> BuildPreview {
    let balance = money.cents();
    if proposed.not_straight {
        return BuildPreview {
            tiles: proposed
                .tiles
                .iter()
                .map(|&tile| GhostTile {
                    tile,
                    kind: TileGhostKind::Invalid,
                    valid: false,
                })
                .collect(),
            new_tile_count: 0,
            bridge_count: 0,
            total_cost_cents: 0,
            balance_after_cents: balance,
            can_commit: false,
            reject: Some(RejectInfo {
                message: "Need a straight or 45° run".into(),
                tiles: vec![proposed.endpoint],
            }),
            endpoint: proposed.endpoint,
        };
    }

    let mut reject: Option<RejectInfo> = None;
    if let Err(err) = path_bridge_spans_ok(terrain, &proposed.tiles) {
        reject = Some(RejectInfo {
            message: placement_reason(err, 0, balance),
            tiles: water_run_tiles(terrain, &proposed.tiles),
        });
    }

    let mut ghosts = Vec::with_capacity(proposed.tiles.len());
    let mut new_tile_count = 0u32;
    let mut bridge_count = 0u32;
    let mut total_cost = 0i64;

    for &tile in &proposed.tiles {
        if !terrain.contains(tile) {
            ghosts.push(GhostTile {
                tile,
                kind: TileGhostKind::Invalid,
                valid: false,
            });
            if reject.is_none() {
                reject = Some(RejectInfo {
                    message: placement_reason(PlacementError::OutOfBounds, 0, balance),
                    tiles: vec![tile],
                });
            }
            continue;
        }

        if network.id_at(tile, GROUND_LAYER).is_some() {
            ghosts.push(GhostTile {
                tile,
                kind: TileGhostKind::Existing,
                valid: true,
            });
            continue;
        }

        match validate_tile_empty(network, terrain, tile, GROUND_LAYER) {
            Ok(is_bridge) => {
                let cost = tile_cost(is_bridge);
                total_cost += cost;
                new_tile_count += 1;
                if is_bridge {
                    bridge_count += 1;
                }
                ghosts.push(GhostTile {
                    tile,
                    kind: TileGhostKind::Place {
                        cost_cents: cost,
                        is_bridge,
                    },
                    valid: true,
                });
            }
            Err(err) => {
                ghosts.push(GhostTile {
                    tile,
                    kind: TileGhostKind::Invalid,
                    valid: false,
                });
                if reject.is_none() {
                    reject = Some(RejectInfo {
                        message: placement_reason(err, 0, balance),
                        tiles: vec![tile],
                    });
                }
            }
        }
    }

    if reject.is_none() && new_tile_count == 0 {
        // Entire path already occupied — still a useful loud signal on commit.
        let occupied: Vec<_> = proposed
            .tiles
            .iter()
            .copied()
            .filter(|t| network.id_at(*t, GROUND_LAYER).is_some())
            .collect();
        if !occupied.is_empty() {
            reject = Some(RejectInfo {
                message: placement_reason(PlacementError::AlreadyOccupied, 0, balance),
                tiles: occupied,
            });
        }
    }

    let balance_after = balance - total_cost;
    if reject.is_none() && total_cost > balance {
        let short = total_cost - balance;
        reject = Some(RejectInfo {
            message: format!("Short by {}", format_dollars(short)),
            tiles: ghosts
                .iter()
                .filter(|g| matches!(g.kind, TileGhostKind::Place { .. }))
                .map(|g| g.tile)
                .collect(),
        });
    }

    // Mark placeable tiles invalid when the whole route cannot commit.
    if reject.is_some() {
        for g in &mut ghosts {
            if matches!(g.kind, TileGhostKind::Place { .. }) {
                g.valid = false;
            }
        }
    }

    let can_commit = reject.is_none() && new_tile_count > 0;
    BuildPreview {
        tiles: ghosts,
        new_tile_count,
        bridge_count,
        total_cost_cents: total_cost,
        balance_after_cents: balance_after,
        can_commit,
        reject,
        endpoint: proposed.endpoint,
    }
}

/// Preview demolish along a snapped ortho/45° path.
pub fn preview_demolish(
    network: &TrackNetwork,
    money: &Money,
    from: TileCoord,
    to: TileCoord,
) -> DemolishPreview {
    let proposed = propose_path(from, to, PathMode::Autofill);
    let mut tiles = Vec::new();
    let mut refund = 0i64;
    let mut track_count = 0u32;

    for &tile in &proposed.tiles {
        if let Some(id) = network.id_at(tile, GROUND_LAYER) {
            if let Some(piece) = network.piece(id) {
                refund += piece.paid_cents;
                track_count += 1;
                tiles.push(tile);
            }
        }
    }

    let reject = if track_count == 0 {
        Some(RejectInfo {
            message: "Nothing to demolish".into(),
            tiles: vec![proposed.endpoint],
        })
    } else {
        None
    };

    DemolishPreview {
        tiles,
        refund_cents: refund,
        balance_after_cents: money.cents() + refund,
        track_count,
        can_commit: reject.is_none(),
        reject,
        endpoint: proposed.endpoint,
    }
}

/// Plain-language reason for a [`PlacementError`], with numbers where rules have them.
pub fn placement_reason(err: PlacementError, total_cost: i64, balance: i64) -> String {
    match err {
        PlacementError::OutOfBounds => "Map edge".into(),
        PlacementError::NotStraight => "Need a straight or 45° run".into(),
        PlacementError::InvalidLayer => "Can't build on that layer".into(),
        PlacementError::AlreadyOccupied => "Track already here".into(),
        PlacementError::BridgeTooLong { span } => {
            format!("Span too wide — {span} tiles, max {MAX_BRIDGE_SPAN}")
        }
        PlacementError::InsufficientFunds => {
            let short = (total_cost - balance).max(0);
            format!("Short by {}", format_dollars(short))
        }
        PlacementError::UnknownTrack => "Nothing to demolish".into(),
    }
}

pub fn format_dollars(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    let dollars = abs / 100;
    let rem = abs % 100;
    format!("{sign}${dollars}.{rem:02}")
}

fn water_run_tiles(terrain: &TrackTerrain, path: &[TileCoord]) -> Vec<TileCoord> {
    let mut best: Vec<TileCoord> = Vec::new();
    let mut run: Vec<TileCoord> = Vec::new();
    for &tile in path {
        if terrain.contains(tile) && terrain.is_water(tile) {
            run.push(tile);
            if run.len() > best.len() {
                best = run.clone();
            }
        } else {
            run.clear();
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::TRACK_COST_CENTS;

    fn land(w: u32, h: u32) -> TrackTerrain {
        TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 0i8)))
    }

    fn water_strip(water_w: u32) -> TrackTerrain {
        let w = 10u32;
        let h = 5u32;
        TrackTerrain::new(
            w,
            h,
            (0..h).flat_map(move |_y| {
                (0..w).map(move |x| {
                    let water = x >= 3 && x < 3 + water_w;
                    (water, if water { -2 } else { 1 })
                })
            }),
        )
    }

    #[test]
    fn cost_preview_sums_new_tiles() {
        let terrain = land(8, 8);
        let network = TrackNetwork::new();
        let money = Money::new(50_000);
        let preview = preview_build(
            &network,
            &terrain,
            &money,
            TileCoord { x: 1, y: 1 },
            TileCoord { x: 4, y: 1 },
            PathMode::Autofill,
        );
        assert_eq!(preview.new_tile_count, 4);
        assert_eq!(preview.total_cost_cents, 4 * TRACK_COST_CENTS);
        assert!(preview.can_commit);
        assert!(preview.reject.is_none());
        assert_eq!(
            preview.balance_after_cents,
            50_000 - 4 * TRACK_COST_CENTS
        );
    }

    #[test]
    fn bridge_too_long_names_span() {
        let terrain = water_strip(4);
        let network = TrackNetwork::new();
        let money = Money::new(500_000);
        let preview = preview_build(
            &network,
            &terrain,
            &money,
            TileCoord { x: 2, y: 1 },
            TileCoord { x: 7, y: 1 },
            PathMode::Autofill,
        );
        assert!(!preview.can_commit);
        let msg = preview.reject.unwrap().message;
        assert!(msg.contains("Span too wide"), "{msg}");
        assert!(msg.contains("max 3"), "{msg}");
    }

    #[test]
    fn insufficient_funds_names_shortfall() {
        let terrain = land(8, 8);
        let network = TrackNetwork::new();
        let money = Money::new(1_500); // $15 — one tile short of two
        let preview = preview_build(
            &network,
            &terrain,
            &money,
            TileCoord { x: 0, y: 0 },
            TileCoord { x: 1, y: 0 },
            PathMode::Autofill,
        );
        assert!(!preview.can_commit);
        assert_eq!(
            preview.reject.as_ref().map(|r| r.message.as_str()),
            Some("Short by $5.00")
        );
    }

    #[test]
    fn demolish_preview_sums_refunds() {
        let terrain = land(8, 8);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(50_000);
        let a = rail_sim::track::try_place_track(
            &mut network,
            &mut money,
            &terrain,
            TileCoord { x: 2, y: 2 },
            GROUND_LAYER,
        )
        .unwrap();
        let _b = rail_sim::track::try_place_track(
            &mut network,
            &mut money,
            &terrain,
            TileCoord { x: 3, y: 2 },
            GROUND_LAYER,
        )
        .unwrap();
        let paid = a.piece.paid_cents * 2;
        let preview = preview_demolish(
            &network,
            &money,
            TileCoord { x: 2, y: 2 },
            TileCoord { x: 3, y: 2 },
        );
        assert_eq!(preview.track_count, 2);
        assert_eq!(preview.refund_cents, paid);
        assert!(preview.can_commit);
    }
}
