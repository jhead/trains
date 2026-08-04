//! Line data and registry.

use std::collections::HashMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::ids::{LineId, StationId, TrackId, TrainId};
use crate::stations::StationRegistry;
use crate::track::TrackNetwork;
use crate::trains::{find_path, track_for_station};

/// Distinguishable line colours (RGB 0–255). Rotated on create.
pub const LINE_PALETTE: &[[u8; 3]] = &[
    [0xe8, 0x62, 0x4a], // warm coral
    [0x6f, 0xd0, 0x8c], // green
    [0xf2, 0xc1, 0x4e], // amber
    [0x5d, 0x8e, 0xa3], // steel blue
    [0xc0, 0xab, 0x8c], // plaster
    [0x8f, 0x4e, 0x3e], // roof tile
    [0xb9, 0xc2, 0xcf], // rail light
    [0x7d, 0x54, 0x36], // tie
];

/// Palette index into [`LINE_PALETTE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LineColour(pub u8);

impl LineColour {
    pub fn from_index(i: usize) -> Self {
        Self((i % LINE_PALETTE.len()) as u8)
    }

    pub fn rgba(self) -> [u8; 3] {
        LINE_PALETTE[self.0 as usize % LINE_PALETTE.len()]
    }
}

/// Out-and-back shuttle (MVP). Loops deferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LineDirection {
    #[default]
    OutAndBack,
}

/// Where a stop sat in a line's sequence.
///
/// Recorded when a station is demolished out from under a line so undo can put
/// the call back exactly where it was — see [`LineRegistry::restore_stop`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineStopSlot {
    pub line: LineId,
    /// Index the stop occupied before it was removed.
    pub index: usize,
}

/// What [`LineRegistry::remove_stop`] took out.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RemovedStops {
    /// Indices the station occupied, ascending. A station may be called at more
    /// than once (an out-and-back through a hub), and all of them go.
    pub indices: Vec<usize>,
    /// The line is left [dormant](Line::is_dormant) — kept, but not running.
    pub dormant: bool,
}

/// A named coloured ordered sequence of stations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub id: LineId,
    pub name: String,
    pub colour: LineColour,
    pub stops: Vec<StationId>,
    pub trains: Vec<TrainId>,
    pub direction: LineDirection,
}

impl Line {
    pub fn contains_station(&self, id: StationId) -> bool {
        self.stops.contains(&id)
    }

    pub fn stop_index(&self, id: StationId) -> Option<usize> {
        self.stops.iter().position(|s| *s == id)
    }

    /// `true` when the line has nowhere left to run.
    ///
    /// Fewer than two calls, or every call at the same stop — which is what an
    /// out-and-back is left with once the far end is demolished. A dormant line
    /// is **kept, not deleted**: it is the player's named object, its trains
    /// still point at an id that resolves, and putting the stop back (undo, or
    /// editing the route) makes it run again. Deleting it would strand every
    /// assigned train on a line id that no longer exists.
    pub fn is_dormant(&self) -> bool {
        self.stops.len() < 2 || self.stops.iter().all(|s| *s == self.stops[0])
    }

    /// Next stop index when shuttling out-and-back.
    ///
    /// `forward` is mutated when bouncing at an end. `current` is clamped into
    /// range first: a stop demolished from the middle of the sequence shifts
    /// every index after it, and a train still holding the old one must bounce
    /// off the end of the shorter line rather than index past it.
    pub fn next_stop_index(&self, current: usize, forward: &mut bool) -> Option<usize> {
        if self.is_dormant() {
            return None;
        }
        let current = current.min(self.stops.len() - 1);
        if *forward {
            if current + 1 >= self.stops.len() {
                *forward = false;
                Some(current.saturating_sub(1))
            } else {
                Some(current + 1)
            }
        } else if current == 0 {
            *forward = true;
            Some(1)
        } else {
            Some(current - 1)
        }
    }
}

/// All player lines.
#[derive(Debug, Clone, Default, PartialEq, Resource, Serialize, Deserialize)]
pub struct LineRegistry {
    lines: HashMap<LineId, Line>,
    next_id: u64,
    next_colour: usize,
}

impl LineRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Line> {
        self.lines.values()
    }

    pub fn get(&self, id: LineId) -> Option<&Line> {
        self.lines.get(&id)
    }

    pub fn get_mut(&mut self, id: LineId) -> Option<&mut Line> {
        self.lines.get_mut(&id)
    }

    /// Create a line from an ordered stop list (≥2). Colour rotates.
    pub fn create(&mut self, name: String, stops: Vec<StationId>) -> Option<LineId> {
        if stops.len() < 2 {
            return None;
        }
        self.next_id = self.next_id.saturating_add(1);
        let id = LineId(self.next_id);
        let colour = LineColour::from_index(self.next_colour);
        self.next_colour = self.next_colour.saturating_add(1);
        self.lines.insert(
            id,
            Line {
                id,
                name,
                colour,
                stops,
                trains: Vec::new(),
                direction: LineDirection::OutAndBack,
            },
        );
        Some(id)
    }

    pub fn remove(&mut self, id: LineId) -> Option<Line> {
        self.lines.remove(&id)
    }

    /// The existing line that already runs `stops`, if there is one.
    ///
    /// # A reversed route is the same service
    ///
    /// Every line in the MVP is a [`LineDirection::OutAndBack`] shuttle: a train
    /// on `A - B` runs to B, turns, and runs back to A. So `[A, B]` and `[B, A]`
    /// are not two services, they are one service written down from either end —
    /// the same trains calling at the same platforms in the same order. Creating
    /// both gives the player two rows, two colours and two names for one piece of
    /// railway, which is exactly what the playtest produced: *"Westbrook -
    /// Eastgate"* and *"Eastgate - Westbrook"* side by side, indistinguishable in
    /// the world.
    ///
    /// This must be revisited if one-way loops ever land ([`LineDirection`] has
    /// room for them): on a loop, direction is the whole service.
    ///
    /// The lowest [`LineId`] wins so the answer — and the warning naming it — is
    /// the same on every run, whatever order the `HashMap` yields.
    pub fn duplicate_of(&self, stops: &[StationId]) -> Option<LineId> {
        if stops.len() < 2 {
            return None;
        }
        let reversed: Vec<StationId> = stops.iter().rev().copied().collect();
        self.lines
            .values()
            .filter(|line| line.stops == stops || line.stops == reversed)
            .map(|line| line.id)
            .min_by_key(|id| id.0)
    }

    /// Every line that calls at `station`, in [`LineId`] order.
    ///
    /// Sorted because the registry is a `HashMap`: the demolish path records an
    /// undo payload from this and the confirm dialog names the lines from it,
    /// and neither may vary run to run.
    pub fn lines_calling_at(&self, station: StationId) -> Vec<LineId> {
        let mut ids: Vec<LineId> = self
            .lines
            .values()
            .filter(|line| line.contains_station(station))
            .map(|line| line.id)
            .collect();
        ids.sort_unstable_by_key(|id| id.0);
        ids
    }

    /// Drop every call at `station` from `line`.
    ///
    /// `None` when the line is unknown or never called there. The stop list is
    /// only ever shortened — nothing is renumbered or collapsed — so the
    /// returned indices put the route back exactly as it was
    /// ([`Self::restore_stop`]). A line left with fewer than two distinct calls
    /// is reported [`RemovedStops::dormant`] and kept; see [`Line::is_dormant`].
    pub fn remove_stop(&mut self, line: LineId, station: StationId) -> Option<RemovedStops> {
        let line = self.lines.get_mut(&line)?;
        let indices: Vec<usize> = line
            .stops
            .iter()
            .enumerate()
            .filter(|(_, stop)| **stop == station)
            .map(|(index, _)| index)
            .collect();
        if indices.is_empty() {
            return None;
        }
        line.stops.retain(|stop| *stop != station);
        Some(RemovedStops {
            dormant: line.is_dormant(),
            indices,
        })
    }

    /// Splice `station` back in at `index` (clamped to the end of the route).
    ///
    /// The inverse of [`Self::remove_stop`]: restoring the recorded indices in
    /// ascending order rebuilds the original sequence.
    pub fn restore_stop(&mut self, line: LineId, index: usize, station: StationId) -> bool {
        let Some(line) = self.lines.get_mut(&line) else {
            return false;
        };
        line.stops.insert(index.min(line.stops.len()), station);
        true
    }

    pub fn assign_train(&mut self, line: LineId, train: TrainId) -> bool {
        // Drop from any previous line.
        for l in self.lines.values_mut() {
            l.trains.retain(|t| *t != train);
        }
        let Some(l) = self.lines.get_mut(&line) else {
            return false;
        };
        if !l.trains.contains(&train) {
            l.trains.push(train);
        }
        true
    }

    pub fn unassign_train(&mut self, train: TrainId) {
        for l in self.lines.values_mut() {
            l.trains.retain(|t| *t != train);
        }
    }

    pub fn line_for_train(&self, train: TrainId) -> Option<&Line> {
        self.lines.values().find(|l| l.trains.contains(&train))
    }
}

/// RGB for a colour index (presentation helper).
pub fn line_colour_rgba(colour: LineColour) -> [u8; 3] {
    colour.rgba()
}

/// Suggest `"Eastgate — Millhaven"` from endpoint station names.
pub fn suggest_line_name(stations: &StationRegistry, stops: &[StationId]) -> String {
    let Some(first) = stops.first().and_then(|id| stations.get(*id)) else {
        return "New Line".into();
    };
    let Some(last) = stops.last().and_then(|id| stations.get(*id)) else {
        return first.name.clone();
    };
    if first.id == last.id {
        first.name.clone()
    } else {
        format!("{} - {}", first.name, last.name)
    }
}

/// Track path visiting consecutive stops (joined). `None` if any segment disconnects.
pub fn line_path(
    network: &TrackNetwork,
    stations: &StationRegistry,
    stops: &[StationId],
) -> Option<Vec<TrackId>> {
    if stops.len() < 2 {
        return None;
    }
    let mut full: Vec<TrackId> = Vec::new();
    for w in stops.windows(2) {
        let a = stations.get(w[0])?;
        let b = stations.get(w[1])?;
        let from = track_for_station(network, a.tile, a.layer)?;
        let to = track_for_station(network, b.tile, b.layer)?;
        let leg = find_path(network, from, to)?;
        if full.is_empty() {
            full = leg;
        } else if full.last() == leg.first() {
            full.extend(leg.into_iter().skip(1));
        } else {
            full.extend(leg);
        }
    }
    Some(full)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economy::MoneyLedger;
    use crate::ids::TileCoord;
    use crate::money::Money;
    use crate::track::{try_place_track, TrackTerrain, GROUND_LAYER};

    fn land(w: u32, h: u32) -> TrackTerrain {
        TrackTerrain::new(w, h, (0..w * h).map(|_| (false, 0i8)))
    }

    #[test]
    fn create_line_and_path_between_stops() {
        let terrain = land(8, 8);
        let mut network = TrackNetwork::new();
        let mut money = Money::new(500_000);
        let mut ledger = MoneyLedger::default();
        for x in 1..=5 {
            try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                TileCoord { x, y: 2 },
                GROUND_LAYER,
            )
            .unwrap();
        }
        let mut stations = StationRegistry::new();
        let a = stations.insert("Eastgate", TileCoord { x: 1, y: 2 }, GROUND_LAYER);
        let b = stations.insert("Mid", TileCoord { x: 3, y: 2 }, GROUND_LAYER);
        let c = stations.insert("Millhaven", TileCoord { x: 5, y: 2 }, GROUND_LAYER);

        let mut lines = LineRegistry::new();
        let name = suggest_line_name(&stations, &[a, b, c]);
        assert_eq!(name, "Eastgate - Millhaven");
        let id = lines.create(name, vec![a, b, c]).expect("line");
        let line = lines.get(id).unwrap();
        assert_eq!(line.stops.len(), 3);
        assert_eq!(line.colour, LineColour(0));

        let path = line_path(&network, &stations, &line.stops).expect("connected");
        assert!(path.len() >= 5);
        assert_eq!(path.first(), network.id_at(TileCoord { x: 1, y: 2 }, GROUND_LAYER).as_ref());
        assert_eq!(path.last(), network.id_at(TileCoord { x: 5, y: 2 }, GROUND_LAYER).as_ref());
    }

    /// Three stops in, one demolished, two left: the line runs on.
    #[test]
    fn remove_stop_takes_the_call_out_and_leaves_the_line_running() {
        let mut lines = LineRegistry::new();
        let (a, b, c) = (StationId(1), StationId(2), StationId(3));
        let id = lines.create("Riverside Loop".into(), vec![a, b, c]).unwrap();

        let removed = lines.remove_stop(id, b).expect("b was a call");
        assert_eq!(removed.indices, vec![1]);
        assert!(!removed.dormant, "two stops still make a route");
        assert_eq!(lines.get(id).unwrap().stops, vec![a, c]);
    }

    #[test]
    fn remove_stop_drops_every_call_at_the_same_station() {
        // An out-and-back through a hub calls there twice; both go.
        let mut lines = LineRegistry::new();
        let (a, b, c) = (StationId(1), StationId(2), StationId(3));
        let id = lines.create("Hub Shuttle".into(), vec![a, b, c, b]).unwrap();

        let removed = lines.remove_stop(id, b).expect("b was a call");
        assert_eq!(removed.indices, vec![1, 3]);
        assert_eq!(lines.get(id).unwrap().stops, vec![a, c]);
    }

    #[test]
    fn remove_stop_reports_nothing_for_a_station_the_line_never_called_at() {
        let mut lines = LineRegistry::new();
        let id = lines
            .create("Coast".into(), vec![StationId(1), StationId(2)])
            .unwrap();
        assert_eq!(lines.remove_stop(id, StationId(9)), None);
        assert_eq!(lines.remove_stop(LineId(99), StationId(1)), None);
        assert_eq!(lines.get(id).unwrap().stops.len(), 2);
    }

    /// The degenerate case: the line is **kept, dormant**, never deleted.
    #[test]
    fn a_line_left_under_two_stops_goes_dormant_rather_than_being_deleted() {
        let mut lines = LineRegistry::new();
        let (a, b) = (StationId(1), StationId(2));
        let id = lines.create("Eastgate - Millhaven".into(), vec![a, b]).unwrap();

        let removed = lines.remove_stop(id, b).expect("b was a call");
        assert!(removed.dormant);
        assert!(lines.get(id).is_some(), "the player's line still exists");
        assert!(lines.get(id).unwrap().is_dormant());
        assert_eq!(lines.len(), 1);
    }

    /// An out-and-back reduced to the same stop twice is going nowhere either.
    #[test]
    fn a_route_calling_only_at_one_station_is_dormant() {
        let mut lines = LineRegistry::new();
        let (a, b) = (StationId(1), StationId(2));
        let id = lines.create("There and Back".into(), vec![a, b, a]).unwrap();

        let removed = lines.remove_stop(id, b).expect("b was a call");
        assert!(removed.dormant, "A - A is not a route");
        assert_eq!(lines.get(id).unwrap().stops, vec![a, a]);
        let mut forward = true;
        assert_eq!(
            lines.get(id).unwrap().next_stop_index(0, &mut forward),
            None,
            "a dormant line hands its trains nowhere to go"
        );
    }

    #[test]
    fn restoring_the_recorded_indices_rebuilds_the_original_route() {
        let mut lines = LineRegistry::new();
        let (a, b, c) = (StationId(1), StationId(2), StationId(3));
        let id = lines.create("Hub Shuttle".into(), vec![a, b, c, b]).unwrap();
        let before = lines.get(id).unwrap().stops.clone();

        let removed = lines.remove_stop(id, b).expect("b was a call");
        for index in removed.indices {
            assert!(lines.restore_stop(id, index, b));
        }

        assert_eq!(lines.get(id).unwrap().stops, before);
        assert!(!lines.get(id).unwrap().is_dormant());
    }

    #[test]
    fn a_route_the_registry_already_holds_is_found_from_either_end() {
        let mut lines = LineRegistry::new();
        let (a, b, c) = (StationId(1), StationId(2), StationId(3));
        let id = lines.create("Eastgate - Westbrook".into(), vec![a, b]).unwrap();

        assert_eq!(lines.duplicate_of(&[a, b]), Some(id), "the same route");
        assert_eq!(
            lines.duplicate_of(&[b, a]),
            Some(id),
            "an out-and-back shuttle is one service, written from either end"
        );
        assert_eq!(lines.duplicate_of(&[a, c]), None, "a different route");
        assert_eq!(lines.duplicate_of(&[a, b, c]), None, "a longer route");
        assert_eq!(lines.duplicate_of(&[a]), None, "not a route at all");
        assert_eq!(lines.duplicate_of(&[]), None);
    }

    #[test]
    fn a_longer_route_is_matched_stop_for_stop() {
        let mut lines = LineRegistry::new();
        let (a, b, c) = (StationId(1), StationId(2), StationId(3));
        let id = lines.create("Cross Town".into(), vec![a, b, c]).unwrap();

        assert_eq!(lines.duplicate_of(&[a, b, c]), Some(id));
        assert_eq!(lines.duplicate_of(&[c, b, a]), Some(id));
        assert_eq!(
            lines.duplicate_of(&[a, c, b]),
            None,
            "the same stops in a different order call in a different order"
        );
    }

    /// The registry is a `HashMap`, and the warning names the line it found.
    #[test]
    fn the_duplicate_reported_is_the_oldest_one_every_time() {
        let mut lines = LineRegistry::new();
        let (a, b) = (StationId(1), StationId(2));
        let first = lines.create("First".into(), vec![a, b]).unwrap();
        // Two copies can only exist in a world saved before the check landed —
        // the answer still has to be stable.
        let _second = lines.create("Second".into(), vec![b, a]).unwrap();

        for _ in 0..16 {
            assert_eq!(lines.duplicate_of(&[a, b]), Some(first));
        }
    }

    #[test]
    fn lines_calling_at_a_stop_come_back_in_id_order() {
        let mut lines = LineRegistry::new();
        let (a, b, c) = (StationId(1), StationId(2), StationId(3));
        let first = lines.create("First".into(), vec![a, b]).unwrap();
        let second = lines.create("Second".into(), vec![c, b]).unwrap();
        let _elsewhere = lines.create("Elsewhere".into(), vec![a, c]).unwrap();

        assert_eq!(lines.lines_calling_at(b), vec![first, second]);
        assert!(lines.lines_calling_at(StationId(9)).is_empty());
    }

    /// A stop removed from the middle shifts every index after it; a train
    /// still holding the old one must bounce, not index past the end.
    #[test]
    fn a_stale_stop_index_clamps_instead_of_running_off_the_end() {
        let line = Line {
            id: LineId(1),
            name: "Test".into(),
            colour: LineColour(0),
            stops: vec![StationId(1), StationId(2)],
            trains: vec![],
            direction: LineDirection::OutAndBack,
        };
        let mut forward = false;
        let next = line.next_stop_index(7, &mut forward).expect("a stop to aim at");
        assert!(next < line.stops.len(), "index {next} is off the route");
    }

    #[test]
    fn out_and_back_bounces() {
        let line = Line {
            id: LineId(1),
            name: "Test".into(),
            colour: LineColour(0),
            stops: vec![StationId(1), StationId(2), StationId(3)],
            trains: vec![],
            direction: LineDirection::OutAndBack,
        };
        let mut forward = true;
        assert_eq!(line.next_stop_index(0, &mut forward), Some(1));
        assert!(forward);
        assert_eq!(line.next_stop_index(1, &mut forward), Some(2));
        assert_eq!(line.next_stop_index(2, &mut forward), Some(1));
        assert!(!forward);
        assert_eq!(line.next_stop_index(1, &mut forward), Some(0));
        assert_eq!(line.next_stop_index(0, &mut forward), Some(1));
        assert!(forward);
    }
}
