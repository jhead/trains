//! Town Talk — public ambient feed (complaints, praise, and later opportunities).
//!
//! Short, specific lines like “Mara waited 11 min at Eastgate”. The feed is
//! rate-limited, deduplicated by station for complaints, and keeps locate ids
//! so the HUD can fly the camera to the peep / station.

use std::collections::VecDeque;

use bevy_ecs::prelude::Resource;

use crate::ids::{StationId, TileCoord};

use super::PeepId;

/// Max lines kept in the HUD / log (newest first).
pub const MAX_COMPLAINTS: usize = 8;

/// Alias — feed capacity is shared across Town Talk kinds.
pub const MAX_TOWN_TALK: usize = MAX_COMPLAINTS;

/// Sim-seconds of waiting before a peep emits a complaint (~11 minutes).
pub const COMPLAINT_WAIT_SECS: u32 = 11 * 60;

/// Sim ticks within which identical-station complaints merge into one line.
pub const COMPLAINT_DEDUPE_TICKS: u64 = 120;

/// Kind of Town Talk entry (diagnostic + emotional ambient voice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TalkKind {
    Complaint,
    Praise,
    Opportunity,
    Warning,
}

/// One public Town Talk line for the HUD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplaintEntry {
    pub kind: TalkKind,
    pub peep_name: String,
    pub station_name: String,
    /// Whole minutes waited (complaints); `0` for praise / other.
    pub wait_minutes: u32,
    pub sim_tick: u64,
    pub peep_id: Option<PeepId>,
    pub station_id: Option<StationId>,
    pub tile: Option<TileCoord>,
    /// Aggregated voices when complaints about the same station are deduped.
    pub count: u32,
}

/// Preferred name in UI / docs.
pub type TownTalkEntry = ComplaintEntry;

impl ComplaintEntry {
    /// HUD / log line — plain, specific, never cute.
    pub fn display_line(&self) -> String {
        match self.kind {
            TalkKind::Complaint => {
                if self.count > 1 {
                    format!(
                        "{} people are waiting at {}",
                        self.count, self.station_name
                    )
                } else {
                    format!(
                        "{} waited {} min at {}",
                        self.peep_name, self.wait_minutes, self.station_name
                    )
                }
            }
            TalkKind::Praise => format!(
                "{} · smooth ride via {}",
                self.peep_name, self.station_name
            ),
            TalkKind::Opportunity => {
                if self.station_name.is_empty() {
                    self.peep_name.clone()
                } else {
                    format!("{} · {}", self.peep_name, self.station_name)
                }
            }
            TalkKind::Warning => format!(
                "{} · trouble at {}",
                self.peep_name, self.station_name
            ),
        }
    }

    /// Relative age from `now_tick` (sim FixedUpdate ticks ≈ 10 sim-seconds each).
    pub fn age_label(&self, now_tick: u64) -> String {
        let age_ticks = now_tick.saturating_sub(self.sim_tick);
        let age_secs = age_ticks.saturating_mul(10);
        if age_secs < 30 {
            "now".into()
        } else if age_secs < 60 {
            format!("{age_secs}s")
        } else {
            let mins = age_secs / 60;
            format!("{mins}m")
        }
    }

    pub fn is_complaint(&self) -> bool {
        self.kind == TalkKind::Complaint
    }
}

/// Newest-first Town Talk list for the UI.
#[derive(Debug, Clone, Default, Resource)]
pub struct ComplaintFeed {
    entries: VecDeque<ComplaintEntry>,
}

/// Preferred name in UI / docs.
pub type TownTalkFeed = ComplaintFeed;

impl ComplaintFeed {
    pub fn push(&mut self, entry: ComplaintEntry) {
        let mut entry = entry;
        if entry.count == 0 {
            entry.count = 1;
        }

        if entry.kind == TalkKind::Complaint {
            if let Some(station_id) = entry.station_id {
                if let Some(existing) = self.entries.iter_mut().find(|e| {
                    e.kind == TalkKind::Complaint
                        && e.station_id == Some(station_id)
                        && entry.sim_tick.saturating_sub(e.sim_tick) <= COMPLAINT_DEDUPE_TICKS
                }) {
                    existing.count = existing.count.saturating_add(entry.count.max(1));
                    existing.wait_minutes = existing.wait_minutes.max(entry.wait_minutes);
                    existing.sim_tick = entry.sim_tick;
                    // Keep a concrete peep for click-to-locate when possible.
                    if existing.peep_id.is_none() {
                        existing.peep_id = entry.peep_id;
                        existing.peep_name = entry.peep_name;
                    }
                    existing.tile = entry.tile.or(existing.tile);
                    return;
                }
            }
        }

        self.entries.push_front(entry);
        while self.entries.len() > MAX_COMPLAINTS {
            self.entries.pop_back();
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &ComplaintEntry> {
        self.entries.iter()
    }

    pub fn latest_line(&self) -> Option<String> {
        self.entries.front().map(|e| e.display_line())
    }

    pub fn get(&self, index: usize) -> Option<&ComplaintEntry> {
        self.entries.get(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complaint(name: &str, station: &str, mins: u32, tick: u64, sid: u64) -> ComplaintEntry {
        ComplaintEntry {
            kind: TalkKind::Complaint,
            peep_name: name.into(),
            station_name: station.into(),
            wait_minutes: mins,
            sim_tick: tick,
            peep_id: Some(PeepId(1)),
            station_id: Some(StationId(sid)),
            tile: Some(TileCoord { x: 1, y: 2 }),
            count: 1,
        }
    }

    #[test]
    fn feed_keeps_newest_first_and_caps() {
        let mut feed = ComplaintFeed::default();
        for i in 0..12 {
            feed.push(ComplaintEntry {
                kind: TalkKind::Complaint,
                peep_name: format!("P{i}"),
                station_name: "Eastgate".into(),
                wait_minutes: i,
                sim_tick: i as u64 * (COMPLAINT_DEDUPE_TICKS + 1),
                peep_id: None,
                station_id: Some(StationId(i as u64 + 1)),
                tile: None,
                count: 1,
            });
        }
        assert_eq!(feed.len(), MAX_COMPLAINTS);
        assert_eq!(
            feed.latest_line().unwrap(),
            "P11 waited 11 min at Eastgate"
        );
    }

    #[test]
    fn complaints_dedupe_by_station() {
        let mut feed = ComplaintFeed::default();
        feed.push(complaint("Mara", "Eastgate", 11, 10, 1));
        feed.push(complaint("Theo", "Eastgate", 12, 20, 1));
        feed.push(complaint("Nia", "Westbrook", 11, 25, 2));
        assert_eq!(feed.len(), 2);
        let east = feed.iter().find(|e| e.station_name == "Eastgate").unwrap();
        assert_eq!(east.count, 2);
        assert_eq!(east.display_line(), "2 people are waiting at Eastgate");
        assert_eq!(
            feed.latest_line().unwrap(),
            "Nia waited 11 min at Westbrook"
        );
    }

    #[test]
    fn praise_line_is_plain() {
        let e = ComplaintEntry {
            kind: TalkKind::Praise,
            peep_name: "Mara".into(),
            station_name: "Eastgate".into(),
            wait_minutes: 0,
            sim_tick: 1,
            peep_id: Some(PeepId(1)),
            station_id: Some(StationId(1)),
            tile: None,
            count: 1,
        };
        assert_eq!(e.display_line(), "Mara · smooth ride via Eastgate");
    }

    #[test]
    fn age_label_steps() {
        let e = complaint("Mara", "Eastgate", 11, 100, 1);
        assert_eq!(e.age_label(100), "now");
        assert_eq!(e.age_label(104), "40s"); // 4 ticks × 10s
        assert_eq!(e.age_label(112), "2m");
    }
}
