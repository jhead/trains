//! Borders — MP-1 of `docs/design/12-multiplayer.md`, and **no networking at all**.
//!
//! A map has four edges. Run a line to one, pay to open a portal, and there is
//! somebody on the other side: a named town with a skyline, a standing offer and
//! a trading rhythm. Send a train through and, half a sim-hour later, it comes
//! back carrying what you cannot produce.
//!
//! Every part of that works offline, forever, with no account, no server and no
//! second player — which is the phasing the brief asks for (§10): *"MP-1
//! delivers most of the fantasy and requires no networking whatsoever."*
//!
//! # Layout
//!
//! | Module | What it owns |
//! | --- | --- |
//! | [`edge`] | the four edges and which one a tile is on |
//! | [`manifest`] | [`BorderManifest`] — the unit of exchange (§4.1) |
//! | [`echo`] | deterministic neighbours from map seed + edge (§6) |
//! | [`link`] | opening a border as a construction project (§3.1) |
//! | [`apply`] | the [`CommandKind`](crate::commands::CommandKind) seam (§4.2) |
//! | [`trade`] | transit, returns, the cache, Town Talk (§4.2, §5) |
//!
//! # The two constraints
//!
//! Everything here is downstream of §2, so both are load-bearing rather than
//! aspirational, and both are tested directly:
//!
//! **A neighbour's absence never blocks you.** There is no state in which the
//! player waits. A train that reaches the portal leaves the simulation on that
//! tick and its return tick is written down there and then, from an offer
//! already cached locally. Nothing in this module reads anything remote —
//! `trade::advance_border_trade` has no I/O in it at all — so "offline",
//! "deleted their save" and "generated echo" are one code path.
//!
//! **Contributions are strictly additive.** The train that comes back is your
//! own, returning to the railhead it left from; a neighbour never spawns stock,
//! never occupies a tile, never creates congestion and never reduces a score.
//! The only money that crosses is a credit. The portal has a one-off build cost
//! and no upkeep, and closing refunds it in full.
//!
//! # Wiring
//!
//! [`BorderPlugin`] registers the resource, the message and both systems with
//! their ordering, so `SimPlugin` needs one line. The command variants are the
//! usual `from_kind` / `into_kind` seam in [`apply`] — until `commands.rs`
//! grows them, the command path is inert and everything else is live.

pub mod apply;
pub mod echo;
pub mod edge;
pub mod link;
pub mod manifest;
pub mod trade;

pub use apply::{
    apply_border_commands, push_border_command, AssignTrainToBorder, BorderCommand, BorderEdit,
    CloseBorder, OpenBorder, SetBorderTrade,
};
pub use echo::{
    echo_headline, echo_link_id, echo_manifest, echo_offer, echo_offer_good, echo_period_ticks,
    echo_request, echo_request_good, echo_silhouette, echo_town_name, growth_steps,
    ECHO_GROWTH_TICKS,
};
pub use edge::{edge_for_tile, BorderEdge};
pub use link::{
    railhead_on_edge, try_close_border, try_open_border, validate_border_site, BorderError,
    BorderLink, BorderRegistry, OpenedBorder, TransitTrain, BORDER_ARRIVAL_CENTS,
    BORDER_CROSSING_TICKS, BORDER_PORTAL_COST_CENTS, BORDER_TALK_COOLDOWN_TICKS,
    MATURITY_BONUS_PERCENT, MATURITY_CROSSINGS,
};
pub use manifest::{
    BorderManifest, Departure, HeadlineStat, LinkId, Presence, PresenceSource, Silhouette,
    StandingOffer, StandingRequest, MANIFEST_SCHEMA_VERSION, MAX_DEPARTURES, MAX_TOWN_NAME_LEN,
    MAX_UNITS, SILHOUETTE_ROOFS,
};
pub use trade::{
    advance_border_trade, land_transit_train, portal_track, refresh_cached_neighbour,
    BorderRun, BEYOND_THE_BORDER,
};

use bevy_app::{App, FixedUpdate, Plugin};
use bevy_ecs::schedule::IntoScheduleConfigs;

/// Registers the border registry, edits, and both fixed-tick systems.
///
/// # Ordering, and why
///
/// - [`apply_border_commands`] runs after
///   [`apply_commands`](crate::apply_commands) and **before**
///   [`apply_track_commands`](crate::track::apply_track_commands), which owns
///   `CommandHistory::finish_replay`. Same rule, same reason, as
///   [`apply_station_commands`](crate::stations::apply_station_commands).
/// - [`advance_border_trade`] runs in [`SimSet::Advance`](crate::SimSet)
///   **before** [`assign_jobs`](crate::economy::assign_jobs), so a train that
///   reached the portal last tick leaves through it before the job board can
///   hand it domestic work instead.
pub struct BorderPlugin;

impl Plugin for BorderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BorderRegistry>()
            .add_message::<BorderEdit>()
            .add_systems(
                FixedUpdate,
                apply_border_commands
                    .after(crate::apply_commands)
                    .before(crate::track::apply_track_commands)
                    .in_set(crate::SimSet::ApplyCommands),
            )
            .add_systems(
                FixedUpdate,
                advance_border_trade
                    .before(crate::economy::assign_jobs)
                    .in_set(crate::SimSet::Advance)
                    .run_if(crate::sim_is_running),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::TileCoord;
    use crate::money::Money;
    use crate::track::{try_place_track, TrackNetwork, TrackTerrain, GROUND_LAYER};
    use crate::economy::MoneyLedger;

    /// A world with every border open is byte-comparable to solo play in every
    /// number the player can see, until the player themselves sends a train.
    ///
    /// This is §2.2's "the worst possible neighbour is a silent one, and a
    /// silent one is identical to solo play", checked on the data rather than
    /// on behaviour.
    #[test]
    fn an_open_border_takes_nothing_from_the_world() {
        let terrain = TrackTerrain::new(16, 16, (0..16 * 16).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut money = Money::new(10_000_000);
        let mut ledger = MoneyLedger::default();
        for x in 0..16 {
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
        let track_before = network.len();

        let mut registry = BorderRegistry::new(42);
        let mut cash = Money::new(10_000_000);
        let mut led = MoneyLedger::default();
        try_open_border(
            &mut registry,
            &mut cash,
            &mut led,
            &network,
            &terrain,
            TileCoord { x: 15, y: 8 },
            GROUND_LAYER,
            BorderEdge::East,
        )
        .expect("east");
        try_open_border(
            &mut registry,
            &mut cash,
            &mut led,
            &network,
            &terrain,
            TileCoord { x: 0, y: 8 },
            GROUND_LAYER,
            BorderEdge::West,
        )
        .expect("west");

        // The network is untouched: no tile was taken, none was demolished.
        assert_eq!(network.len(), track_before);
        // The only money that moved is the player's own construction spend.
        assert_eq!(
            cash.cents(),
            10_000_000 - 2 * BORDER_PORTAL_COST_CENTS,
            "a portal is the only debit, and the player chose it"
        );
        assert_eq!(led.total(crate::economy::MoneyCategory::Deliveries), 0);
        // Nothing is out, nothing is pending, nothing is owed.
        assert_eq!(registry.trains_in_transit(), 0);
        for link in registry.iter() {
            assert!(link.transit.is_empty());
            assert!(link.is_echo(), "and it says so");
        }
    }
}
