//! The trade loop, transit, and the asynchrony machinery behind it.
//!
//! `12-multiplayer.md` §4.2, in four steps:
//!
//! 1. A train is assigned to a border run and routes to the portal.
//! 2. It enters the portal and **leaves the simulation**, entering transit.
//! 3. Its cargo is added to the outbound manifest.
//! 4. Some time later a train arrives from the portal carrying goods from their
//!    standing offer, and runs into the network as ordinary freight.
//!
//! # The two constraints, in code
//!
//! **§2.1 — a neighbour's absence never blocks you.** Step 2 despawns the train
//! and writes down its return tick *at that moment*, from the cached offer
//! already in hand. There is no inbox to poll, no acknowledgement to wait for,
//! and no state a train can sit in called "pending". [`advance_border_trade`]
//! does not read anything remote, because in MP-1 there is nothing remote to
//! read — and that is the point: MP-2 only adds a second way for a manifest to
//! reach the cache.
//!
//! **§2.2 — contributions are strictly additive.** The train that comes back is
//! *your own train*, returning to the railhead it left from. Nothing the
//! neighbour does ever spawns rolling stock on your track, so a border can never
//! create congestion, never occupy a tile, and never block a route. Their trains
//! exist only in the Border Yard, beyond the map edge, where there are no tiles
//! to take. Every cent that crosses is a credit.
//!
//! # The cache is the mechanism (§5)
//!
//! [`refresh_cached_neighbour`] is where an offer enters the world. In MP-1 it
//! is generated locally from the map seed; in MP-2 a network task will call
//! [`BorderLink::accept_manifest`] with bytes from a blob store. Return trains
//! are generated from whatever is in the cache, on our own tick, either way —
//! so "the neighbour went offline for a week" and "the neighbour is a generated
//! echo" are the same code path, and neither has a waiting state.

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::economy::{MoneyCategory, MoneyLedger};
use crate::ids::{TileCoord, TrackId, TrainId};
use crate::money::Money;
use crate::peeps::{ComplaintEntry, ComplaintFeed, TalkKind};
use crate::stations::{GoodKind, IndustryId, IndustryRegistry};
use crate::track::TrackNetwork;
use crate::trains::{find_path_for_kind, track_for_station, Train, TrainCargo, TrainLocation};

use super::apply::BorderEdit;
use super::echo::{echo_manifest, growth_steps};
use super::edge::BorderEdge;
use super::link::{
    BorderLink, BorderRegistry, TransitTrain, BORDER_CROSSING_TICKS, BORDER_TALK_COOLDOWN_TICKS,
};
use super::manifest::{Departure, LinkId};

/// Cargo that came from beyond the border has no producer on this map.
///
/// [`IndustryRegistry`] hands out ids from 1, so 0 can never name a site here.
/// Lookups of it return [`None`], which is exactly the behaviour every consumer
/// of `TrainCargo::Goods::from` already handles.
pub const BEYOND_THE_BORDER: IndustryId = IndustryId(0);

/// A train working a border run.
///
/// The train keeps doing ordinary work — it will take a domestic job and deliver
/// it — but whenever it is free it heads for the portal. Wearing this marker is
/// the whole of "assigned to a border line".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Component, Serialize, Deserialize)]
pub struct BorderRun {
    pub edge: BorderEdge,
}

/// Push one plain sentence into Town Talk.
///
/// §9: "Town Talk carries the border." Border events belong in the feed the game
/// already speaks with, not in a notification system of their own. The line
/// carries the portal tile so click-to-locate flies to the border.
pub(crate) fn say(feed: &mut ComplaintFeed, tick: u64, sentence: impl Into<String>) {
    say_at(feed, tick, None, sentence);
}

pub(crate) fn say_at(
    feed: &mut ComplaintFeed,
    tick: u64,
    tile: Option<TileCoord>,
    sentence: impl Into<String>,
) {
    feed.push(ComplaintEntry {
        kind: TalkKind::Opportunity,
        peep_name: sentence.into(),
        station_name: String::new(),
        wait_minutes: 0,
        sim_tick: tick,
        peep_id: None,
        station_id: None,
        tile,
        count: 1,
    });
}

/// Bring a transit train back and set it running as ordinary freight.
///
/// If some industry here consumes what they sent and a route exists, the train
/// runs to it exactly as a domestic goods working would and is paid again on
/// arrival. If nothing consumes it, the goods are still landed and still paid
/// for at the portal — the train simply becomes free stock. Absence of a
/// consumer degrades the payoff, never the loop.
#[allow(clippy::too_many_arguments)]
pub fn land_transit_train(
    commands: &mut Commands,
    link: &BorderLink,
    transit: &TransitTrain,
    money: &mut Money,
    ledger: &mut MoneyLedger,
    network: &TrackNetwork,
    industries: &IndustryRegistry,
) -> (i64, GoodKind, u32) {
    let offer = link.their_offer();
    let units = offer.units_per_period.max(1);
    let paid = link.arrival_payout_cents(units);
    ledger.credit(money, MoneyCategory::Deliveries, paid);

    // The railhead it left from may have been demolished, or belong to a world
    // that has since been replaced. Pay for the goods either way -- they landed
    // at the portal and the neighbour held up their end -- but do not put stock
    // down on track that is not there. `advance_trains` would recall it to the
    // yard a tick later; skipping the spawn means it never stands on the grass
    // in the first place.
    if network.piece(transit.home).is_none() {
        return (paid, offer.good, units);
    }

    let mut location = TrainLocation::at_track(transit.home);
    let mut cargo = TrainCargo::Empty;
    if let Some(consumer) = industries.consumer_of(offer.good) {
        if let Some(dest) = track_for_station(network, consumer.tile, link.layer) {
            if let Some(path) = find_path_for_kind(network, transit.home, dest, transit.kind) {
                location.set_path(path);
                cargo = TrainCargo::Goods {
                    kind: offer.good,
                    from: BEYOND_THE_BORDER,
                    to: consumer.id,
                };
            }
        }
    }

    commands.spawn((
        Train {
            id: transit.train,
            kind: transit.kind,
        },
        location,
        cargo,
        BorderRun { edge: link.edge },
    ));
    (paid, offer.good, units)
}

/// Refresh a link's cached neighbour from the echo generator.
///
/// Returns `true` when the cache moved on. In MP-2 the same [`BorderLink`] can
/// be updated instead by a network task calling
/// [`BorderLink::accept_manifest`] — this function is the local source, not the
/// only one, which is why it checks [`BorderLink::is_echo`] first.
pub fn refresh_cached_neighbour(link: &mut BorderLink, seed: u64, tick: u64) -> bool {
    if !link.is_echo() {
        // A real neighbour's cache is written by whatever last heard from them,
        // and is never overwritten by a generated one.
        return false;
    }
    let growth = u64::from(growth_steps(tick.saturating_sub(link.opened_tick)));
    if growth <= link.neighbour.sequence {
        return false;
    }
    let manifest = echo_manifest(seed, link.edge, growth as u32);
    link.accept_manifest(manifest)
}

/// Advance every border: the cache, the arrivals, and the departures.
///
/// # Ordering
/// Runs in [`SimSet::Advance`](crate::SimSet) **before**
/// [`assign_jobs`](crate::economy::assign_jobs), so a train that reached the
/// portal on the previous tick leaves through it before the job board can hand
/// it domestic work.
#[allow(clippy::too_many_arguments)]
pub fn advance_border_trade(
    mut registry: ResMut<BorderRegistry>,
    mut money: ResMut<Money>,
    mut ledger: ResMut<MoneyLedger>,
    mut feed: ResMut<ComplaintFeed>,
    network: Res<TrackNetwork>,
    industries: Res<IndustryRegistry>,
    mut q: Query<(Entity, &Train, &mut TrainLocation, &mut TrainCargo, &BorderRun)>,
    mut commands: Commands,
    mut edits: MessageWriter<BorderEdit>,
) {
    // The tick is a clock, not an edit.
    //
    // Bumping it through change detection marked `BorderRegistry` changed on
    // every sim tick, including solo play with no border ever opened — and a
    // consumer that reasonably read `is_changed()` as "a link moved" then did
    // its work every frame forever. (Presentation's portal mirror did exactly
    // that, and it cost two thirds of the frame.) Advance the clock without
    // announcing it, and announce only once there is real border work.
    let clock = registry.bypass_change_detection();
    clock.tick = clock.tick.saturating_add(1);
    let tick = clock.tick;
    let seed = clock.seed;
    // Solo play with no border ever opened costs one increment a tick.
    if clock.is_empty() && clock.trains_in_transit() == 0 {
        return;
    }
    // Past here the links are walked mutably, so report the change honestly.
    registry.set_changed();

    // ── Their side: the cache, their rhythm, and what they are asking for ──
    for link in registry.iter_mut() {
        if refresh_cached_neighbour(link, seed, tick) {
            edits.write(BorderEdit::NeighbourUpdated {
                edge: link.edge,
                town_name: link.town_name().to_string(),
                sequence: link.neighbour.sequence,
            });
        }
        let period = link.their_offer().period_ticks.max(1);
        link.their_phase = (link.their_phase + 1) % period;
        // On the beat of their rhythm, and no more often than the feed allows,
        // they say what they want. It is a hint, never a demand: nothing here
        // changes a score, a payout or a tile.
        if link.their_phase == 0
            && tick.saturating_sub(link.last_spoke_tick) >= BORDER_TALK_COOLDOWN_TICKS
        {
            link.last_spoke_tick = tick;
            let sentence = format!(
                "{} is asking for {}",
                link.town_name(),
                link.their_request().good.label()
            );
            let tile = link.portal_tile;
            say_at(&mut feed, tick, Some(tile), sentence);
        }
    }

    // ── Arrivals: whatever is due, comes back ──
    //
    // Severed links are walked too. A player who closes a border while stock is
    // out gets their trains home on schedule rather than losing them.
    let mut landings: Vec<(LinkId, BorderLink, TransitTrain)> = Vec::new();
    for link in registry.all_mut() {
        if link.transit.iter().all(|t| t.due_tick > tick) {
            continue;
        }
        let due: Vec<TransitTrain> = link
            .transit
            .iter()
            .copied()
            .filter(|t| t.due_tick <= tick)
            .collect();
        link.transit.retain(|t| t.due_tick > tick);
        for transit in due {
            landings.push((link.link, link.clone(), transit));
        }
    }
    landings.sort_by_key(|(_, link, transit)| (link.edge.index(), transit.train.0));

    for (link_id, link, transit) in landings {
        let (paid, good, units) = land_transit_train(
            &mut commands,
            &link,
            &transit,
            &mut money,
            &mut ledger,
            &network,
            &industries,
        );
        if let Some(live) = registry.any_by_link_id_mut(link_id) {
            live.crossings = live.crossings.saturating_add(1);
            live.received_units = live.received_units.saturating_add(units);
            live.last_spoke_tick = tick;
        }
        say_at(
            &mut feed,
            tick,
            Some(link.portal_tile),
            format!("A train from {} brought {}", link.town_name(), good.label()),
        );
        edits.write(BorderEdit::Arrived {
            edge: link.edge,
            train: transit.train,
            good: Some(good),
            units,
            paid_cents: paid,
        });
    }

    // ── Departures: a train that reaches the border leaves ──
    let mut runs: Vec<(Entity, TrainId)> = q
        .iter()
        .map(|(entity, train, _, _, _)| (entity, train.id))
        .collect();
    runs.sort_unstable_by_key(|(_, id)| id.0);

    for (entity, _) in runs {
        let Ok((_, train, mut loc, cargo, run)) = q.get_mut(entity) else {
            continue;
        };
        let Some(link) = registry.get(run.edge) else {
            // The link was closed under them. The train keeps its stock and its
            // position and goes back to ordinary work — nothing is stranded.
            commands.entity(entity).remove::<BorderRun>();
            continue;
        };
        let Some(portal_track) = network.id_at(link.portal_tile, link.layer) else {
            // The player lifted the railhead. Nothing to do but wait for track,
            // and the train is free for domestic work in the meantime.
            continue;
        };

        if loc.parked || loc.dwell_remaining > 0 {
            continue;
        }

        if loc.track == portal_track && loc.at_destination() {
            let offer = link.our_offer();
            let (good, units) = match *cargo {
                TrainCargo::Goods { kind, .. } => (Some(kind), 1),
                // Empty stock is loaded from the yard with whatever we publish.
                _ => (Some(offer.good), offer.units_per_period.max(1)),
            };
            let transit = TransitTrain {
                train: train.id,
                kind: train.kind,
                sent_tick: tick,
                due_tick: tick.saturating_add(BORDER_CROSSING_TICKS),
                home: portal_track,
                carried: good,
                units,
            };
            let (edge, tile, name) = (link.edge, link.portal_tile, link.town_name().to_string());
            if let Some(live) = registry.get_mut(edge) {
                live.transit.push(transit);
                live.sent_units = live.sent_units.saturating_add(units);
                live.publish_departure(Departure {
                    tick,
                    good,
                    units,
                });
            }
            // It leaves the simulation: no entity, no tile, no blocker.
            commands.entity(entity).despawn();
            if let Some(good) = good {
                say_at(
                    &mut feed,
                    tick,
                    Some(tile),
                    format!("A train left for {} with {}", name, good.label()),
                );
            }
            edits.write(BorderEdit::Departed {
                edge,
                train: train.id,
                good,
                units,
            });
            continue;
        }

        // Free and idle: head for the border. A train carrying a domestic job
        // finishes it first — a border run adds work, it never cancels any.
        if cargo.is_empty() && loc.at_destination() && loc.track != portal_track {
            if let Some(path) = find_path_for_kind(&network, loc.track, portal_track, train.kind) {
                loc.set_path(path);
            }
        }
    }
}

/// Track a portal sits on, if the railhead is still there.
pub fn portal_track(network: &TrackNetwork, link: &BorderLink) -> Option<TrackId> {
    network.id_at(link.portal_tile, link.layer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::border::edge::BorderEdge;
    use crate::border::link::{try_open_border, BORDER_ARRIVAL_CENTS};
    use crate::border::BorderPlugin;
    use crate::commands::{Pause, TrainKind};
    use crate::ids::TileCoord;
    use crate::track::{try_place_track, TrackTerrain, GROUND_LAYER};
    use crate::trains::TileOccupancy;
    use crate::{SimClock, SimPlugin};
    use bevy_app::{App, FixedUpdate};

    const W: u32 = 16;
    const H: u32 = 16;
    /// Ticks for a goods train to run the eleven tiles from the railhead at
    /// `(4, 8)` out to the portal at `(15, 8)`, with headroom.
    const RUN_TO_PORTAL_TICKS: u32 = 80;

    /// A flat world with a line running east from `(4, 8)` to the east edge,
    /// and the east border already open.
    fn world_with_border() -> App {
        let mut app = App::new();
        app.add_plugins(SimPlugin);
        // `SimPlugin` adds `BorderPlugin` once `lib.rs` is wired; before that it
        // does not. Adding it only when missing keeps these tests honest in both
        // states rather than passing for the wrong reason in one of them.
        if !app.is_plugin_added::<BorderPlugin>() {
            app.add_plugins(BorderPlugin);
        }
        let terrain = TrackTerrain::new(W, H, (0..W * H).map(|_| (false, 0i8)));

        let mut network = TrackNetwork::new();
        let mut money = Money::new(100_000_000);
        let mut ledger = MoneyLedger::default();
        for x in 4..W as i32 {
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

        let mut registry = BorderRegistry::new(42);
        let mut open_money = Money::new(10_000_000);
        let mut open_ledger = MoneyLedger::default();
        try_open_border(
            &mut registry,
            &mut open_money,
            &mut open_ledger,
            &network,
            &terrain,
            TileCoord { x: 15, y: 8 },
            GROUND_LAYER,
            BorderEdge::East,
        )
        .expect("open");

        // Somewhere for border goods to go: §4.2 step 4 is that what arrives
        // "runs into your network as ordinary freight", which needs a consumer.
        let wanted = registry
            .get(BorderEdge::East)
            .expect("link")
            .their_offer()
            .good;
        let mut industries = IndustryRegistry::new();
        industries.insert("Mill", TileCoord { x: 5, y: 8 }, None, Some(wanted));

        app.insert_resource(terrain);
        app.insert_resource(network);
        app.insert_resource(registry);
        app.insert_resource(industries);
        // Funded, so upkeep never becomes the reason a train stands still —
        // these tests are about the border, not about bankruptcy.
        app.insert_resource(Money::new(10_000_000));
        app.insert_resource(MoneyLedger::default());
        app
    }

    fn spawn_border_train(app: &mut App, id: u64) -> TrainId {
        let start = {
            let network = app.world().resource::<TrackNetwork>();
            network
                .id_at(TileCoord { x: 4, y: 8 }, GROUND_LAYER)
                .expect("railhead")
        };
        let id = TrainId(id);
        app.world_mut().spawn((
            Train {
                id,
                kind: TrainKind::Transport,
            },
            TrainLocation::at_track(start),
            TrainCargo::Empty,
            BorderRun {
                edge: BorderEdge::East,
            },
        ));
        id
    }

    fn run(app: &mut App, ticks: u32) {
        for _ in 0..ticks {
            app.world_mut().run_schedule(FixedUpdate);
        }
    }

    fn trains_on_map(app: &mut App) -> Vec<TrainId> {
        let mut q = app.world_mut().query::<&Train>();
        let mut ids: Vec<TrainId> = q.iter(app.world()).map(|t| t.id).collect();
        ids.sort_by_key(|id| id.0);
        ids
    }

    /// Cents the border has paid into the treasury, clean of upkeep noise.
    fn border_income(app: &App) -> i64 {
        app.world()
            .resource::<MoneyLedger>()
            .total(MoneyCategory::Deliveries)
    }

    fn link_of(app: &App) -> &BorderLink {
        app.world()
            .resource::<BorderRegistry>()
            .get(BorderEdge::East)
            .expect("east link")
    }

    /// The whole loop, with nobody on the other side: a train reaches the
    /// border, leaves, and something comes back. This is §2.1 as a test.
    #[test]
    fn a_train_that_reaches_the_border_leaves_and_something_comes_back() {
        let mut app = world_with_border();
        let id = spawn_border_train(&mut app, 1);

        // Long enough to run the line: eleven tiles at five ticks a tile.
        run(&mut app, RUN_TO_PORTAL_TICKS);
        assert!(
            trains_on_map(&mut app).is_empty(),
            "the train must actually leave the simulation"
        );
        let link = link_of(&app);
        assert_eq!(link.transit.len(), 1, "it is in transit, not waiting");
        assert_eq!(link.sent_units, link.our_offer().units_per_period.max(1));
        assert_eq!(link.outbound.departures.len(), 1, "its cargo was published");

        run(&mut app, BORDER_CROSSING_TICKS as u32 + 24);
        assert_eq!(
            trains_on_map(&mut app),
            vec![id],
            "the same train comes back, with nobody having been asked"
        );
        let link = link_of(&app);
        assert!(link.transit.is_empty());
        assert_eq!(link.crossings, 1);
        assert!(link.received_units >= 1);
        assert!(
            border_income(&app) >= BORDER_ARRIVAL_CENTS,
            "border goods pay on arrival"
        );
    }

    /// §2.1 again, from the other direction: the loop keeps turning forever
    /// without any inbox, any acknowledgement, or any remote anything.
    #[test]
    fn trade_continues_with_nobody_there() {
        let mut app = world_with_border();
        spawn_border_train(&mut app, 1);

        run(&mut app, (BORDER_CROSSING_TICKS as u32 + 60) * 3);

        let link = link_of(&app);
        assert!(
            link.crossings >= 2,
            "the route should keep running, got {} crossings",
            link.crossings
        );
        assert!(border_income(&app) > 0);
        // And it never got stuck in a pending state.
        assert!(link.transit.len() <= 1);
    }

    /// §2.2 — the worst possible neighbour is a silent one, and a silent one is
    /// identical to solo play. Two identical worlds, one with a border open and
    /// no train ever sent, must be indistinguishable in every player-visible
    /// number.
    #[test]
    fn a_silent_neighbour_changes_nothing() {
        let mut solo = world_with_border();
        {
            // Close the border again so this world has no neighbour at all.
            let mut registry = solo.world_mut().resource_mut::<BorderRegistry>();
            let mut money = Money::new(0);
            let mut ledger = MoneyLedger::default();
            crate::border::link::try_close_border(
                &mut registry,
                &mut money,
                &mut ledger,
                BorderEdge::East,
            )
            .expect("close");
        }
        let mut linked = world_with_border();

        run(&mut solo, 600);
        run(&mut linked, 600);

        // Money, track, occupancy and rolling stock are all identical: the
        // neighbour has had six hundred ticks and has changed nothing.
        assert_eq!(
            linked.world().resource::<Money>().cents(),
            solo.world().resource::<Money>().cents(),
            "a neighbour may not move the treasury on their own"
        );
        assert_eq!(
            linked.world().resource::<TrackNetwork>().len(),
            solo.world().resource::<TrackNetwork>().len(),
            "a neighbour may not occupy or demolish a tile"
        );
        assert_eq!(
            linked.world().resource::<TileOccupancy>().by_track.len(),
            solo.world().resource::<TileOccupancy>().by_track.len(),
            "a neighbour may not create congestion"
        );
        assert_eq!(
            trains_on_map(&mut linked),
            trains_on_map(&mut solo),
            "a neighbour may not put rolling stock on your track"
        );
    }

    /// A neighbour that never says anything again still supplies, because the
    /// return was decided from the cache at departure.
    #[test]
    fn a_frozen_cache_still_supplies() {
        let mut app = world_with_border();
        spawn_border_train(&mut app, 1);
        run(&mut app, RUN_TO_PORTAL_TICKS);

        // Freeze the cache: pretend we will never hear from them again.
        {
            let mut registry = app.world_mut().resource_mut::<BorderRegistry>();
            let link = registry.get_mut(BorderEdge::East).expect("link");
            link.neighbour.sequence = u64::MAX;
        }
        let before = border_income(&app);

        run(&mut app, BORDER_CROSSING_TICKS as u32 + 24);

        assert_eq!(trains_on_map(&mut app).len(), 1, "it still came home");
        assert!(
            border_income(&app) > before,
            "a stale offer keeps supplying"
        );
    }

    /// Closing a link while stock is out must not strand it.
    #[test]
    fn severing_a_link_still_brings_the_trains_home() {
        let mut app = world_with_border();
        let id = spawn_border_train(&mut app, 1);
        run(&mut app, RUN_TO_PORTAL_TICKS);
        assert!(trains_on_map(&mut app).is_empty(), "it is out there");

        {
            let mut registry = app.world_mut().resource_mut::<BorderRegistry>();
            assert_eq!(registry.get(BorderEdge::East).expect("link").transit.len(), 1);
            let mut money = Money::new(0);
            let mut ledger = MoneyLedger::default();
            crate::border::link::try_close_border(
                &mut registry,
                &mut money,
                &mut ledger,
                BorderEdge::East,
            )
            .expect("close");
            assert!(registry.get(BorderEdge::East).is_none());
            assert_eq!(registry.trains_in_transit(), 1, "still due home");
        }

        run(&mut app, BORDER_CROSSING_TICKS as u32 + 24);

        assert_eq!(
            trains_on_map(&mut app),
            vec![id],
            "a train that is out is never stranded by anything, including a
             player who changed their mind"
        );
        assert_eq!(app.world().resource::<BorderRegistry>().trains_in_transit(), 0);
    }

    /// The border must never hand a train to the job board while it is standing
    /// at the portal, or it would never leave.
    #[test]
    fn a_train_at_the_portal_leaves_before_the_job_board_sees_it() {
        let mut app = world_with_border();
        // Unpause is the default; make it explicit so Advance definitely runs.
        app.world_mut()
            .resource_mut::<SimClock>()
            .apply_pause(Pause { paused: false });
        spawn_border_train(&mut app, 1);

        run(&mut app, RUN_TO_PORTAL_TICKS);
        assert!(trains_on_map(&mut app).is_empty());
        assert_eq!(link_of(&app).transit.len(), 1);
    }

    /// Determinism: the same world, run twice, trades identically.
    #[test]
    fn the_border_is_deterministic() {
        let sample = |ticks: u32| {
            let mut app = world_with_border();
            spawn_border_train(&mut app, 1);
            spawn_border_train(&mut app, 2);
            run(&mut app, ticks);
            let link = link_of(&app);
            (
                link.crossings,
                link.sent_units,
                link.received_units,
                link.transit.len(),
                border_income(&app),
            )
        };
        let a = sample(500);
        let b = sample(500);
        assert_eq!(a, b);
    }
}
