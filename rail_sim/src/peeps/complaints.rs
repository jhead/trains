//! Public complaint feed — short lines like “Mara waited 11 min at Eastgate”.

use std::collections::VecDeque;

use bevy_ecs::prelude::Resource;

/// Max lines kept in the HUD feed (newest first).
pub const MAX_COMPLAINTS: usize = 8;

/// Sim-seconds of waiting before a peep emits a complaint (~11 minutes).
pub const COMPLAINT_WAIT_SECS: u32 = 11 * 60;

/// One public complaint for the HUD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplaintEntry {
    pub peep_name: String,
    pub station_name: String,
    /// Whole minutes waited (at least 1 when emitted).
    pub wait_minutes: u32,
    pub sim_tick: u64,
}

impl ComplaintEntry {
    /// HUD / log line, e.g. `Mara waited 11 min at Eastgate`.
    pub fn display_line(&self) -> String {
        format!(
            "{} waited {} min at {}",
            self.peep_name, self.wait_minutes, self.station_name
        )
    }
}

/// Newest-first public complaint list for the UI.
#[derive(Debug, Clone, Default, Resource)]
pub struct ComplaintFeed {
    entries: VecDeque<ComplaintEntry>,
}

impl ComplaintFeed {
    pub fn push(&mut self, entry: ComplaintEntry) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_keeps_newest_first_and_caps() {
        let mut feed = ComplaintFeed::default();
        for i in 0..12 {
            feed.push(ComplaintEntry {
                peep_name: format!("P{i}"),
                station_name: "Eastgate".into(),
                wait_minutes: i,
                sim_tick: i as u64,
            });
        }
        assert_eq!(feed.len(), MAX_COMPLAINTS);
        assert_eq!(feed.latest_line().unwrap(), "P11 waited 11 min at Eastgate");
    }
}
