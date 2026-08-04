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
    bridge_cost_for_span, local_slope, piece_maintenance_weight, tile_build_cost, tile_cost,
    BRIDGE_COST_CENTS, BRIDGE_MAINT_WEIGHT, CHEAP_BRIDGE_SPAN, GROUND_LAYER, MAX_BRIDGE_SPAN,
    MAX_CURVE, MAX_GRADE, MOUNTAIN_HEIGHT_MIN, TRACK_COST_CENTS, TRACK_MAINT_WEIGHT,
};
pub use dir::{
    bearing_deg, bearing_separation_deg, clock_index, clock_separation, dir_from_clock, dir_index,
    intermediate_tiles, is_half_step, length_sq, opposite_dir, step, straddled_dirs, TrackLinks,
    DIR16, DIR8, DIR_COUNT, HALF_STEP_BASE,
};
pub use network::TrackNetwork;
pub use piece::{curve_from_link_dirs, TrackKind, TrackPiece};
pub use place::{
    run_direction, straight_line, try_autofill_track, try_demolish, try_place_path,
    try_place_track, PlacedTrack,
};
pub use rules::{
    grade_to_neighbors_ok, half_step_run_clear, path_bridge_spans_ok, path_grades_ok,
    turnout_divergence_ok, validate_tile_empty, walk_path, PlacementError,
    MIN_TURNOUT_DIVERGENCE_TENTHS,
};
pub use terrain::TrackTerrain;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::TileCoord;
    use crate::economy::MoneyLedger;
    use crate::money::Money;

    fn land_map(w: u32, h: u32) -> TrackTerrain {
        TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 0i8)))
    }

    /// Map cut top to bottom by a water strip `water_w` wide at x=3.., with
    /// three columns of dry land either side.
    ///
    /// Taller than `MAX_BRIDGE_SPAN` on purpose: span is measured on the
    /// *shorter* axis, so a short map would quietly price and permit a strip on
    /// its height rather than on its width.
    fn map_with_water_strip(water_w: u32) -> TrackTerrain {
        let w = water_w + 6;
        let h = MAX_BRIDGE_SPAN + 3;
        TrackTerrain::new(w, h, (0..h).flat_map(move |_y| {
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
        let mut ledger = MoneyLedger::default();
        let start = money.cents();

        let placed = try_place_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            TileCoord { x: 2, y: 3 },
            GROUND_LAYER,
        )
        .unwrap();
        assert_eq!(money.cents(), start - TRACK_COST_CENTS);
        assert_eq!(network.len(), 1);

        try_demolish(&mut network, &mut money, &mut ledger, placed.id).unwrap();
        assert_eq!(money.cents(), start);
        assert!(network.is_empty());
    }

    #[test]
    fn refuse_out_of_bounds_and_non_ground_layer() {
        let terrain = land_map(4, 4);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(50_000);
        let mut ledger = MoneyLedger::default();

        assert_eq!(
            try_place_track(
                &mut network,
                &mut money,
            &mut ledger,
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
            &mut ledger,
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
        let mut ledger = MoneyLedger::default();

        let placed = try_place_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            TileCoord { x: 4, y: 2 },
            GROUND_LAYER,
        )
        .unwrap();
        assert!(placed.piece.is_bridge());
        // The crossing span is the *shorter* axis — how far it is to dry land.
        // This used to read `.max()` and still matched, because every span past
        // 2 billed at the same 20x; the ladder now prices each rung apart, so
        // the two axes are no longer interchangeable.
        let span = terrain
            .water_span_horizontal(TileCoord { x: 4, y: 2 })
            .min(terrain.water_span_vertical(TileCoord { x: 4, y: 2 }));
        assert_eq!(span, 3);
        assert_eq!(money.cents(), 500_000 - bridge_cost_for_span(span));
    }

    /// The premium tier, priced end to end: a wide crossing is buildable, every
    /// deck tile bills at its rung, and the bill is one atomic transaction.
    /// This is the whole point of raising the span limit — a big river is an
    /// expensive answer rather than a wall.
    #[test]
    fn a_wide_bridge_places_at_the_premium_rate() {
        for span in [5u32, 7] {
            let terrain = map_with_water_strip(span);
            let mut network = TrackNetwork::new();
            let mut money = Money::new(50_000_000);
            let mut ledger = MoneyLedger::default();
            let start = money.cents();
            let west = TileCoord { x: 2, y: 4 };
            let east = TileCoord {
                x: 3 + span as i32,
                y: 4,
            };

            let placed = try_autofill_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                west,
                east,
                GROUND_LAYER,
            )
            .unwrap();

            assert_eq!(placed.len() as u32, span + 2);
            let decks = placed
                .iter()
                .filter(|p| p.piece.kind == TrackKind::Bridge)
                .count() as u32;
            assert_eq!(decks, span, "span {span}: one deck tile per water tile");
            let banks = tile_build_cost(&terrain, west).unwrap()
                + tile_build_cost(&terrain, east).unwrap();
            assert_eq!(
                start - money.cents(),
                bridge_cost_for_span(span) * span as i64 + banks,
                "span {span} billed off its rung"
            );
        }
        assert_eq!(bridge_cost_for_span(5), TRACK_COST_CENTS * 42);
        assert_eq!(bridge_cost_for_span(7), TRACK_COST_CENTS * 72);
    }

    #[test]
    fn bridge_rejected_when_water_wider_than_limit() {
        // One past the limit. Was 5, which the ladder now spans at a premium
        // rather than refusing — the refusal moved with the rule.
        let span = MAX_BRIDGE_SPAN + 1;
        let terrain = map_with_water_strip(span);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(50_000_000);
        let mut ledger = MoneyLedger::default();

        let err = try_place_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            TileCoord {
                x: 3 + span as i32 / 2,
                y: 4,
            },
            GROUND_LAYER,
        )
        .unwrap_err();
        assert_eq!(err, PlacementError::BridgeTooLong { span });
        assert_eq!(money.cents(), 50_000_000);
        assert!(network.is_empty());
    }

    #[test]
    fn autofill_straight_and_bridge_path_limit() {
        let terrain = map_with_water_strip(2);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(500_000);
        let mut ledger = MoneyLedger::default();

        // Horizontal line across 2-wide water (land at x=2 and x=5).
        let placed = try_autofill_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            TileCoord { x: 2, y: 2 },
            TileCoord { x: 5, y: 2 },
            GROUND_LAYER,
        )
        .unwrap();
        assert_eq!(placed.len(), 4);
        assert!(network.len() >= 4);

        // Wider water on a fresh map must fail the path run check. One past the
        // span limit, not four: four is a premium span now and lays fine.
        let over = MAX_BRIDGE_SPAN + 1;
        let wide = map_with_water_strip(over);
        let mut network2 = TrackNetwork::new();
        let mut money2 = Money::new(50_000_000);
        let mut ledger2 = MoneyLedger::default();
        let err = try_autofill_track(
            &mut network2,
            &mut money2,
            &mut ledger2,
            &wide,
            TileCoord { x: 2, y: 1 },
            TileCoord {
                x: 3 + over as i32,
                y: 1,
            },
            GROUND_LAYER,
        )
        .unwrap_err();
        assert!(matches!(err, PlacementError::BridgeTooLong { span } if span == over));
        assert_eq!(money2.cents(), 50_000_000);
    }

    #[test]
    fn autofill_rejects_a_run_off_all_sixteen_directions() {
        let terrain = land_map(8, 8);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(50_000);
        let mut ledger = MoneyLedger::default();
        // (3, 1) is neither compass nor knight's move.
        assert_eq!(
            try_autofill_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                TileCoord { x: 0, y: 0 },
                TileCoord { x: 3, y: 1 },
                GROUND_LAYER,
            ),
            Err(PlacementError::NotStraight)
        );
    }

    /// The widening in one test: a knight's move is now a run, and it lays a
    /// *sparse* line whose gaps stay empty so the shallow links can exist.
    #[test]
    fn autofill_lays_a_sparse_half_step_run() {
        let terrain = land_map(12, 12);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(500_000);
        let mut ledger = MoneyLedger::default();

        let placed = try_autofill_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            TileCoord { x: 1, y: 1 },
            TileCoord { x: 5, y: 3 },
            GROUND_LAYER,
        )
        .unwrap();

        // (1,1) → (3,2) → (5,3): three tiles, not five.
        assert_eq!(placed.len(), 3);
        let tiles: Vec<_> = placed.iter().map(|p| p.piece.tile).collect();
        assert_eq!(
            tiles,
            vec![
                TileCoord { x: 1, y: 1 },
                TileCoord { x: 3, y: 2 },
                TileCoord { x: 5, y: 3 },
            ]
        );
        // The tiles the run crosses stay bare.
        for gap in [(2, 1), (2, 2), (4, 2), (4, 3)] {
            assert!(
                network
                    .id_at(
                        TileCoord {
                            x: gap.0,
                            y: gap.1
                        },
                        GROUND_LAYER
                    )
                    .is_none(),
                "half-step run must not fill {gap:?}"
            );
        }
        // And it is genuinely connected end to end.
        let ids: Vec<_> = placed.iter().map(|p| p.id).collect();
        assert_eq!(network.neighbor_ids(ids[0]), vec![ids[1]]);
        assert_eq!(network.neighbor_ids(ids[1]).len(), 2);
        assert!(network.piece(ids[1]).unwrap().links.has_half_step());
    }

    #[test]
    fn autofill_refuses_a_half_step_run_across_existing_track() {
        let terrain = land_map(12, 12);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(500_000);
        let mut ledger = MoneyLedger::default();

        // A tile sitting in the middle of where the shallow run would pass.
        try_place_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            TileCoord { x: 2, y: 1 },
            GROUND_LAYER,
        )
        .unwrap();
        let before = money.cents();

        let err = try_autofill_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            TileCoord { x: 1, y: 1 },
            TileCoord { x: 5, y: 3 },
            GROUND_LAYER,
        )
        .unwrap_err();

        assert_eq!(
            err,
            PlacementError::HalfStepBlocked {
                tile: TileCoord { x: 2, y: 1 }
            }
        );
        assert_eq!(money.cents(), before, "a refused run costs nothing");
    }

    #[test]
    fn a_bent_path_lays_every_leg_and_one_bill() {
        let terrain = map_with_water_strip(2);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(500_000);
        let mut ledger = MoneyLedger::default();

        // East along y=2, over the 2-wide water, then bend north-east: the
        // smart-route commit shape a straight autofill cannot express.
        let path = [
            TileCoord { x: 1, y: 2 },
            TileCoord { x: 2, y: 2 },
            TileCoord { x: 3, y: 2 },
            TileCoord { x: 4, y: 2 },
            TileCoord { x: 5, y: 2 },
            TileCoord { x: 6, y: 1 },
        ];
        let placed = try_place_path(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            &path,
            GROUND_LAYER,
        )
        .unwrap();
        assert_eq!(placed.len(), 6);
        assert_eq!(
            placed
                .iter()
                .filter(|p| p.piece.kind == TrackKind::Bridge)
                .count(),
            2,
            "the water legs land as bridge deck"
        );
    }

    #[test]
    fn a_path_with_a_leg_off_the_sixteen_is_refused_whole() {
        let terrain = land_map(8, 8);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(500_000);
        let mut ledger = MoneyLedger::default();
        let start = money.cents();

        // (3,1) -> (6,2) is off every direction.
        let path = [
            TileCoord { x: 1, y: 1 },
            TileCoord { x: 2, y: 1 },
            TileCoord { x: 3, y: 1 },
            TileCoord { x: 6, y: 2 },
        ];
        let err = try_place_path(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            &path,
            GROUND_LAYER,
        )
        .unwrap_err();
        assert_eq!(err, PlacementError::NotStraight);
        assert_eq!(money.cents(), start, "a refused path costs nothing");
        assert_eq!(network.len(), 0, "and places nothing");
    }

    #[test]
    fn a_path_that_crosses_its_own_half_step_is_refused() {
        let terrain = land_map(12, 12);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(500_000);
        let mut ledger = MoneyLedger::default();

        // The ENE half-step from (2,2) crosses (3,2) and (3,3) — and the path
        // then bends back through (3,2). Placement would sever the link it
        // just paid for, so the whole path must be refused up front.
        let path = [
            TileCoord { x: 2, y: 2 },
            TileCoord { x: 4, y: 3 },
            TileCoord { x: 3, y: 2 },
        ];
        let err = try_place_path(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            &path,
            GROUND_LAYER,
        )
        .unwrap_err();
        assert!(
            matches!(err, PlacementError::HalfStepBlocked { .. }),
            "got {err:?}"
        );
        assert_eq!(network.len(), 0);
    }

    #[test]
    fn a_path_short_of_funds_is_refused_whole() {
        let terrain = land_map(8, 8);
        let mut network = TrackNetwork::new();
        // Two flat tiles' worth, asked to pay for four.
        let mut money = Money::new(TRACK_COST_CENTS * 2);
        let mut ledger = MoneyLedger::default();

        let path = [
            TileCoord { x: 1, y: 1 },
            TileCoord { x: 2, y: 1 },
            TileCoord { x: 3, y: 1 },
            TileCoord { x: 4, y: 1 },
        ];
        let err = try_place_path(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            &path,
            GROUND_LAYER,
        )
        .unwrap_err();
        assert_eq!(err, PlacementError::InsufficientFunds);
        assert_eq!(money.cents(), TRACK_COST_CENTS * 2);
        assert_eq!(network.len(), 0, "all or nothing");
    }

    #[test]
    fn neighbor_graph_links_adjacent_pieces() {
        let terrain = land_map(8, 8);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(50_000);
        let mut ledger = MoneyLedger::default();
        let a = try_place_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            TileCoord { x: 1, y: 1 },
            GROUND_LAYER,
        )
        .unwrap();
        let b = try_place_track(
            &mut network,
            &mut money,
            &mut ledger,
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
