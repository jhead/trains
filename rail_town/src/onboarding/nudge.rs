//! The opening nudge — one line in Town Talk, naming somewhere to go.
//!
//! Design 09 §7, touch one:
//!
//! > *"Westbrook is eight tiles east. They'd like a railway."*
//!
//! It goes into the existing feed, in the feed's existing shape, because a
//! separate "first-run message" surface would be the modal lecture the brief
//! rules out. It is a Town Talk line like any other: clickable, locatable,
//! and scrolled away by the next thing the town says.
//!
//! # Which destination
//!
//! Brief 02 §4.1 wants the opening map to *guarantee* a second destination
//! eight to twelve tiles from home, across one small terrain question. Terrain
//! generation does not guarantee that yet — [`seed_stations_and_industries`]
//! places anchors by farthest-point sampling, which is the anti-pattern that
//! brief warns about. So this module does the honest thing available to it: it
//! names **the nearest sensible anchor to home at runtime**, whatever distance
//! that turns out to be, and says the true number of tiles. When generation
//! starts guaranteeing the opening beat, this code needs no change — it will
//! simply start naming a stop that is genuinely eight tiles away.
//!
//! Industries count as destinations. On a map whose stations landed at the
//! extremes, a sawmill six tiles out is a far better first line than a station
//! thirty tiles out, and the player is better served by being pointed at it.
//!
//! [`seed_stations_and_industries`]: rail_sim::seed_stations_and_industries

use bevy::prelude::*;
use rail_sim::ids::TileCoord;
use rail_sim::{
    ComplaintEntry, ComplaintFeed, IndustryRegistry, StationRegistry, StationService, TalkKind,
    TrackNetwork,
};

use crate::shell::ShellState;

/// Whether this world has had its opening line yet.
///
/// Per *world*, not per player: every new map has an opening beat, and a
/// returning player starting their fifth map still wants to know which way to
/// point. It is not persisted for the same reason.
#[derive(Resource, Debug, Default)]
pub struct OpeningNudge {
    /// Seed of the world the nudge was spoken for, so a New Map re-arms it.
    spoken_for: Option<u64>,
}

/// Name the nearest destination, once, on a world nobody has built on yet.
#[allow(clippy::too_many_arguments)]
pub fn nudge_toward_the_first_destination(
    state: Res<State<ShellState>>,
    stations: Res<StationRegistry>,
    industries: Res<IndustryRegistry>,
    network: Res<TrackNetwork>,
    service: Res<StationService>,
    map: Option<Res<rail_map::MapGrid>>,
    mut nudge: ResMut<OpeningNudge>,
    mut talk: ResMut<ComplaintFeed>,
) {
    if *state.get() != ShellState::Playing {
        return;
    }
    let seed = map.map(|m| m.seed).unwrap_or(0);
    if nudge.spoken_for == Some(seed) {
        return;
    }
    // A world with track on it is a world in progress — a loaded save, or a
    // player who has already started. Neither wants to be told where to begin.
    if !network.is_empty() {
        nudge.spoken_for = Some(seed);
        return;
    }
    let Some((home, target)) = opening_pair(&stations, &industries) else {
        return; // Anchors have not been seeded yet; try again next frame.
    };

    nudge.spoken_for = Some(seed);
    talk.push(ComplaintEntry {
        kind: TalkKind::Opportunity,
        // Whole-sentence line with an empty station name — the shape the feed
        // already uses for something the world says rather than a person.
        peep_name: nudge_line(&home, &target),
        station_name: String::new(),
        wait_minutes: 0,
        sim_tick: service.tick,
        peep_id: None,
        station_id: target.station,
        tile: Some(target.tile),
        count: 1,
    });
}

/// One anchor the nudge can talk about.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Anchor {
    name: String,
    tile: TileCoord,
    station: Option<rail_sim::StationId>,
}

/// Home, plus the nearest other anchor to it.
fn opening_pair(
    stations: &StationRegistry,
    industries: &IndustryRegistry,
) -> Option<(Anchor, Anchor)> {
    // Sorted: both registries iterate a `HashMap`, and the line the player
    // reads must not depend on that order.
    let mut stops: Vec<Anchor> = stations
        .iter()
        .map(|s| Anchor {
            name: s.name.clone(),
            tile: s.tile,
            station: Some(s.id),
        })
        .collect();
    stops.sort_by(|a, b| a.name.cmp(&b.name));
    if stops.is_empty() {
        return None;
    }

    // Home is the stop closest to the anchors' centre of gravity — the same
    // stop `seed_stations_and_industries` picks first, derived rather than
    // assumed so it stays right when generation changes.
    let count = stops.len() as i64;
    let centre = TileCoord {
        x: (stops.iter().map(|a| a.tile.x as i64).sum::<i64>() / count) as i32,
        y: (stops.iter().map(|a| a.tile.y as i64).sum::<i64>() / count) as i32,
    };
    let home = stops
        .iter()
        .min_by_key(|a| (chebyshev(a.tile, centre), a.name.clone()))?
        .clone();

    let mut candidates: Vec<Anchor> = stops
        .iter()
        .filter(|a| a.station != home.station)
        .cloned()
        .collect();
    let mut mills: Vec<Anchor> = industries
        .iter()
        .map(|i| Anchor {
            name: i.name.clone(),
            tile: i.tile,
            station: None,
        })
        .collect();
    mills.sort_by(|a, b| a.name.cmp(&b.name));
    candidates.extend(mills);

    let target = candidates
        .into_iter()
        .filter(|a| a.tile != home.tile)
        .min_by_key(|a| (chebyshev(a.tile, home.tile), a.name.clone()))?;
    Some((home, target))
}

/// *"Westbrook is eight tiles east. They'd like a railway."*
fn nudge_line(home: &Anchor, target: &Anchor) -> String {
    let tiles = chebyshev(target.tile, home.tile);
    format!(
        "{} is {} tiles {} of {}. They'd like a railway.",
        target.name,
        tiles,
        compass(home.tile, target.tile),
        home.name
    )
}

fn chebyshev(a: TileCoord, b: TileCoord) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

/// Eight-point compass word for the step from `from` to `to`.
///
/// Map `y` runs up, so a positive `dy` is north. Diagonals only when both axes
/// carry real weight — "north-east" for something two tiles north and twenty
/// east is worse directions than "east".
fn compass(from: TileCoord, to: TileCoord) -> &'static str {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let (ax, ay) = (dx.abs(), dy.abs());
    // A minor axis under half the major one is noise, not a direction.
    let horizontal = ax * 2 > ay;
    let vertical = ay * 2 > ax;
    match (horizontal, vertical) {
        (true, true) => match (dx > 0, dy > 0) {
            (true, true) => "north-east",
            (true, false) => "south-east",
            (false, true) => "north-west",
            (false, false) => "south-west",
        },
        (true, false) => {
            if dx > 0 {
                "east"
            } else {
                "west"
            }
        }
        _ => {
            if dy > 0 {
                "north"
            } else {
                "south"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::GROUND_LAYER;

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    fn anchor(name: &str, x: i32, y: i32) -> Anchor {
        Anchor {
            name: name.into(),
            tile: tile(x, y),
            station: None,
        }
    }

    #[test]
    fn the_line_reads_the_way_the_brief_writes_it() {
        let home = anchor("Eastgate", 20, 20);
        let away = anchor("Westbrook", 28, 20);
        assert_eq!(
            nudge_line(&home, &away),
            "Westbrook is 8 tiles east of Eastgate. They'd like a railway."
        );
    }

    #[test]
    fn the_compass_says_east_not_north_east_for_a_slight_drift() {
        // Directions the player can act on beat directions that are precise.
        assert_eq!(compass(tile(0, 0), tile(20, 2)), "east");
        assert_eq!(compass(tile(0, 0), tile(2, 20)), "north");
        assert_eq!(compass(tile(0, 0), tile(10, 10)), "north-east");
        assert_eq!(compass(tile(0, 0), tile(-10, -10)), "south-west");
        assert_eq!(compass(tile(0, 0), tile(-8, 0)), "west");
        assert_eq!(compass(tile(0, 0), tile(0, -8)), "south");
    }

    #[test]
    fn the_nudge_names_the_nearest_anchor_not_the_farthest() {
        let mut stations = StationRegistry::new();
        stations.insert("Eastgate", tile(20, 20), GROUND_LAYER);
        stations.insert("Faraway", tile(20, 44), GROUND_LAYER);
        stations.insert("Westbrook", tile(29, 20), GROUND_LAYER);

        let (home, target) = opening_pair(&stations, &IndustryRegistry::new()).unwrap();
        assert_eq!(home.name, "Eastgate");
        assert_eq!(target.name, "Westbrook");
        assert!(nudge_line(&home, &target).contains("9 tiles east"));
    }

    #[test]
    fn a_mill_can_be_the_opening_destination_when_it_is_the_closest_thing() {
        // Anchors currently land at the map's extremes (farthest-point
        // sampling), so the nearest *useful* thing is often an industry.
        let mut stations = StationRegistry::new();
        stations.insert("Eastgate", tile(20, 20), GROUND_LAYER);
        stations.insert("Faraway", tile(20, 46), GROUND_LAYER);
        let mut industries = IndustryRegistry::new();
        industries.insert("Pine Sawmill", tile(26, 20), None, None);

        let (home, target) = opening_pair(&stations, &industries).unwrap();
        assert_eq!(home.name, "Eastgate");
        assert_eq!(target.name, "Pine Sawmill");
        assert!(target.station.is_none(), "a mill has no station id to focus");
    }

    #[test]
    fn a_world_with_one_anchor_says_nothing_rather_than_something_wrong() {
        let mut stations = StationRegistry::new();
        stations.insert("Alone", tile(4, 4), GROUND_LAYER);
        assert!(opening_pair(&stations, &IndustryRegistry::new()).is_none());
        assert!(opening_pair(&StationRegistry::new(), &IndustryRegistry::new()).is_none());
    }

    #[test]
    fn the_pair_is_the_same_however_the_registry_iterates() {
        let mut stations = StationRegistry::new();
        for (name, x) in [("Zeta", 30), ("Alpha", 26), ("Mid", 34)] {
            stations.insert(name, tile(x, 20), GROUND_LAYER);
        }
        let first = opening_pair(&stations, &IndustryRegistry::new()).unwrap();
        for _ in 0..16 {
            assert_eq!(
                opening_pair(&stations, &IndustryRegistry::new()).unwrap(),
                first
            );
        }
    }
}
