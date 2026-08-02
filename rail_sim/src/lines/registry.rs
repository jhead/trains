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

    /// Next stop index when shuttling out-and-back.
    ///
    /// `forward` is mutated when bouncing at an end.
    pub fn next_stop_index(&self, current: usize, forward: &mut bool) -> Option<usize> {
        if self.stops.len() < 2 {
            return None;
        }
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
        format!("{} — {}", first.name, last.name)
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
        assert_eq!(name, "Eastgate — Millhaven");
        let id = lines.create(name, vec![a, b, c]).expect("line");
        let line = lines.get(id).unwrap();
        assert_eq!(line.stops.len(), 3);
        assert_eq!(line.colour, LineColour(0));

        let path = line_path(&network, &stations, &line.stops).expect("connected");
        assert!(path.len() >= 5);
        assert_eq!(path.first(), network.id_at(TileCoord { x: 1, y: 2 }, GROUND_LAYER).as_ref());
        assert_eq!(path.last(), network.id_at(TileCoord { x: 5, y: 2 }, GROUND_LAYER).as_ref());
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
