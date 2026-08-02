//! Track construction costs (cents) and refunds.

/// Cost to place one ground (non-water) track tile: $10.00.
pub const TRACK_COST_CENTS: i64 = 1_000;

/// Cost to place one bridge tile over water: $50.00.
pub const BRIDGE_COST_CENTS: i64 = 5_000;

/// Maximum contiguous water tiles a bridge may span (inclusive).
pub const MAX_BRIDGE_SPAN: u32 = 3;

/// Ground layer index used by MVP placement commands (`PlaceTrack.layer`).
pub const GROUND_LAYER: u8 = 0;

/// Cost for a single tile given whether it needs a bridge.
#[inline]
pub fn tile_cost(is_bridge: bool) -> i64 {
    if is_bridge {
        BRIDGE_COST_CENTS
    } else {
        TRACK_COST_CENTS
    }
}
