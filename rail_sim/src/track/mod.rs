//! Track network, placement rules, and FixedUpdate apply.

mod apply;
mod cost;
mod dir;
mod network;
mod piece;
mod place;
mod rules;
mod terrain;

pub use apply::{apply_track_commands, TrackEdit};
pub use cost::{
    tile_cost, BRIDGE_COST_CENTS, GROUND_LAYER, MAX_BRIDGE_SPAN, TRACK_COST_CENTS,
};
pub use dir::{dir_index, opposite_dir, step, TrackLinks, DIR8};
pub use network::TrackNetwork;
pub use piece::{curve_from_link_dirs, TrackKind, TrackPiece};
pub use place::{straight_line, try_autofill_track, try_demolish, try_place_track, PlacedTrack};
pub use rules::PlacementError;
pub use terrain::TrackTerrain;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::TileCoord;
    use crate::money::Money;

    fn land_map(w: u32, h: u32) -> TrackTerrain {
        TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 0i8)))
    }

    /// 8×3 map with a vertical water strip of width `water_w` at x=3..
    fn map_with_water_strip(water_w: u32) -> TrackTerrain {
        let w = 10u32;
        let h = 5u32;
        TrackTerrain::new(w, h, (0..h).flat_map(|_y| {
            (0..w).map(move |x| {
                let water = x >= 3 && x < 3 + water_w;
                (water, if water { -2 } else { 1 })
            })
        }))
    }

    #[test]
    fn place_and_demolish_refunds_full() {
        let terrain = land_map(8, 8);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(50_000);
        let start = money.cents();

        let placed = try_place_track(
            &mut network,
            &mut money,
            &terrain,
            TileCoord { x: 2, y: 3 },
            GROUND_LAYER,
        )
        .unwrap();
        assert_eq!(money.cents(), start - TRACK_COST_CENTS);
        assert_eq!(network.len(), 1);

        try_demolish(&mut network, &mut money, placed.id).unwrap();
        assert_eq!(money.cents(), start);
        assert!(network.is_empty());
    }

    #[test]
    fn refuse_out_of_bounds_and_non_ground_layer() {
        let terrain = land_map(4, 4);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(50_000);

        assert_eq!(
            try_place_track(
                &mut network,
                &mut money,
                &terrain,
                TileCoord { x: 10, y: 0 },
                GROUND_LAYER,
            ),
            Err(PlacementError::OutOfBounds)
        );
        assert_eq!(
            try_place_track(
                &mut network,
                &mut money,
                &terrain,
                TileCoord { x: 1, y: 1 },
                1,
            ),
            Err(PlacementError::InvalidLayer)
        );
        assert_eq!(money.cents(), 50_000);
    }

    #[test]
    fn bridge_allowed_within_span_limit() {
        // Water width 3 → min axis span 3 ≤ MAX_BRIDGE_SPAN.
        let terrain = map_with_water_strip(3);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(500_000);

        let placed = try_place_track(
            &mut network,
            &mut money,
            &terrain,
            TileCoord { x: 4, y: 2 },
            GROUND_LAYER,
        )
        .unwrap();
        assert!(placed.piece.is_bridge());
        assert_eq!(money.cents(), 500_000 - BRIDGE_COST_CENTS);
    }

    #[test]
    fn bridge_rejected_when_water_wider_than_limit() {
        // Water width 5 → min(h,v) = 5 > 3.
        let terrain = map_with_water_strip(5);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(500_000);

        let err = try_place_track(
            &mut network,
            &mut money,
            &terrain,
            TileCoord { x: 5, y: 2 },
            GROUND_LAYER,
        )
        .unwrap_err();
        assert!(matches!(err, PlacementError::BridgeTooLong { .. }));
        assert_eq!(money.cents(), 500_000);
        assert!(network.is_empty());
    }

    #[test]
    fn autofill_straight_and_bridge_path_limit() {
        let terrain = map_with_water_strip(2);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(500_000);

        // Horizontal line across 2-wide water (land at x=2 and x=5).
        let placed = try_autofill_track(
            &mut network,
            &mut money,
            &terrain,
            TileCoord { x: 2, y: 2 },
            TileCoord { x: 5, y: 2 },
            GROUND_LAYER,
        )
        .unwrap();
        assert_eq!(placed.len(), 4);
        assert!(network.len() >= 4);

        // Wider water (4) on a fresh map must fail the path run check.
        let wide = map_with_water_strip(4);
        let mut network2 = TrackNetwork::new();
        let mut money2 = Money::new(500_000);
        let err = try_autofill_track(
            &mut network2,
            &mut money2,
            &wide,
            TileCoord { x: 2, y: 1 },
            TileCoord { x: 7, y: 1 },
            GROUND_LAYER,
        )
        .unwrap_err();
        assert!(matches!(err, PlacementError::BridgeTooLong { span: 4 }));
        assert_eq!(money2.cents(), 500_000);
    }

    #[test]
    fn autofill_rejects_non_straight() {
        let terrain = land_map(8, 8);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(50_000);
        assert_eq!(
            try_autofill_track(
                &mut network,
                &mut money,
                &terrain,
                TileCoord { x: 0, y: 0 },
                TileCoord { x: 2, y: 1 },
                GROUND_LAYER,
            ),
            Err(PlacementError::NotStraight)
        );
    }

    #[test]
    fn neighbor_graph_links_adjacent_pieces() {
        let terrain = land_map(8, 8);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(50_000);
        let a = try_place_track(
            &mut network,
            &mut money,
            &terrain,
            TileCoord { x: 1, y: 1 },
            GROUND_LAYER,
        )
        .unwrap();
        let b = try_place_track(
            &mut network,
            &mut money,
            &terrain,
            TileCoord { x: 2, y: 1 },
            GROUND_LAYER,
        )
        .unwrap();
        let n = network.neighbor_ids(a.id);
        assert!(n.contains(&b.id));
        let piece_a = network.piece(a.id).unwrap();
        assert!(piece_a.links.count() >= 1);
    }
}
