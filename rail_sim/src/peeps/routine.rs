//! Routines — a home, a destination, and a time they habitually travel.
//!
//! Brief 06 §4.2: *"Rush hours emerge from the sum of individual routines
//! rather than being imposed by a curve, which means a player can serve them by
//! observing rather than by reading a manual."*
//!
//! There is deliberately **no global demand curve** anywhere in this module.
//! Each peep gets a role, a habitual departure minute derived from that role
//! plus their own hashed jitter, and a length of stay. Whatever shape the
//! morning takes is the sum of those, and nothing else.

use bevy_ecs::prelude::Component;
use serde::{Deserialize, Serialize};

use crate::ids::{StationId, TileCoord};

use super::names::hash64;

/// Sim-minutes in a peep day.
pub const DAY_MINUTES: u32 = 24 * 60;

/// How wide the "it's my time to go" window is, in sim-minutes.
pub const DEPART_WINDOW_MINUTES: u32 = 20;

/// What sort of day this peep keeps. Anchors only — the actual minute is
/// individual.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PeepRole {
    /// Office hours, out early, back late.
    Commuter,
    /// Afternoon shift.
    Shift,
    /// Short mid-morning trip into town.
    Errand,
    /// School / college run.
    Scholar,
    /// Out for the evening.
    Evening,
}

impl PeepRole {
    pub const ALL: [PeepRole; 5] = [
        Self::Commuter,
        Self::Shift,
        Self::Errand,
        Self::Scholar,
        Self::Evening,
    ];

    pub fn from_seed(seed: u64) -> Self {
        // Weighted toward commuters — a town's spine is its commute.
        match hash64(seed ^ 0x1234_5678_9abc_def0) % 10 {
            0..=3 => Self::Commuter,
            4..=5 => Self::Scholar,
            6..=7 => Self::Errand,
            8 => Self::Shift,
            _ => Self::Evening,
        }
    }

    /// Habitual departure anchor, minute-of-day.
    pub fn anchor_minute(self) -> u32 {
        match self {
            Self::Commuter => 7 * 60 + 30,
            Self::Shift => 14 * 60,
            Self::Errand => 10 * 60 + 30,
            Self::Scholar => 8 * 60 + 30,
            Self::Evening => 18 * 60 + 30,
        }
    }

    /// Typical minutes spent at the destination before heading home.
    pub fn stay_minutes(self) -> u32 {
        match self {
            Self::Commuter => 8 * 60,
            Self::Shift => 8 * 60,
            Self::Errand => 90,
            Self::Scholar => 6 * 60,
            Self::Evening => 3 * 60,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Commuter => "commuter",
            Self::Shift => "shift worker",
            Self::Errand => "errands",
            Self::Scholar => "student",
            Self::Evening => "evening out",
        }
    }
}

/// One peep's habitual day.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routine {
    pub role: PeepRole,
    /// Front door.
    pub home: TileCoord,
    /// Station they board at.
    pub home_station: StationId,
    /// Where they are going.
    pub destination: TileCoord,
    /// Station they alight at.
    pub destination_station: StationId,
    /// Habitual departure, minute-of-day.
    pub depart_minute: u32,
    /// How long they stay before heading home, in sim-minutes.
    pub stay_minutes: u32,
}

impl Routine {
    /// Build a routine from a peep seed — role, jitter and stay are all individual.
    pub fn from_seed(
        seed: u64,
        home: TileCoord,
        home_station: StationId,
        destination: TileCoord,
        destination_station: StationId,
    ) -> Self {
        let role = PeepRole::from_seed(seed);
        // ±40 minutes of personal jitter around the role anchor. Two people with
        // the same job still do not leave the house at the same minute.
        let jitter = (hash64(seed ^ 0xa5a5_5a5a_c3c3_3c3c) % 81) as i64 - 40;
        let depart = (role.anchor_minute() as i64 + jitter).rem_euclid(DAY_MINUTES as i64) as u32;
        let stay_jitter = (hash64(seed ^ 0x0f0f_f0f0_1e1e_e1e1) % 61) as i64 - 30;
        let stay = (role.stay_minutes() as i64 + stay_jitter).max(30) as u32;
        Self {
            role,
            home,
            home_station,
            destination,
            destination_station,
            depart_minute: depart,
            stay_minutes: stay,
        }
    }

    /// Habitual return, minute-of-day.
    pub fn return_minute(&self) -> u32 {
        (self.depart_minute + self.stay_minutes) % DAY_MINUTES
    }

    /// True while `minute` sits in this peep's departure window.
    pub fn is_departure_time(&self, minute: u32) -> bool {
        minute_in_window(minute, self.depart_minute, DEPART_WINDOW_MINUTES)
    }

    /// `"07:24"` — the habitual departure, for the Peep card.
    pub fn depart_label(&self) -> String {
        clock_label(self.depart_minute)
    }

    pub fn return_label(&self) -> String {
        clock_label(self.return_minute())
    }

    /// Peep-card line — *"Leaves Eastgate about 07:24, home by 15:40."*
    pub fn describe(&self, home_station_name: &str) -> String {
        format!(
            "{} - leaves {} about {}, home by {}",
            self.role.label(),
            home_station_name,
            self.depart_label(),
            self.return_label()
        )
    }
}

/// `"07:24"` from a minute-of-day.
pub fn clock_label(minute: u32) -> String {
    let m = minute % DAY_MINUTES;
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// True when `minute` lies within `width` minutes after `start`, wrapping midnight.
pub fn minute_in_window(minute: u32, start: u32, width: u32) -> bool {
    let delta = (minute + DAY_MINUTES - (start % DAY_MINUTES)) % DAY_MINUTES;
    delta < width.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routine(seed: u64) -> Routine {
        Routine::from_seed(
            seed,
            TileCoord { x: 3, y: 3 },
            StationId(1),
            TileCoord { x: 20, y: 20 },
            StationId(2),
        )
    }

    #[test]
    fn routines_are_individual_not_a_single_curve() {
        let mut minutes: Vec<u32> = (0..200).map(|s| routine(s).depart_minute).collect();
        minutes.sort_unstable();
        minutes.dedup();
        assert!(
            minutes.len() > 40,
            "departure times collapsed to {} distinct minutes",
            minutes.len()
        );
    }

    #[test]
    fn rush_hours_emerge_from_summed_routines() {
        // No curve is imposed anywhere; the morning peak has to fall out of
        // individual roles + jitter on its own.
        let mut per_hour = [0u32; 24];
        for seed in 0..600u64 {
            per_hour[(routine(seed).depart_minute / 60) as usize] += 1;
        }
        let morning: u32 = per_hour[7..=9].iter().sum();
        let dead_of_night: u32 = per_hour[0..=4].iter().sum();
        assert!(
            morning > dead_of_night * 4,
            "expected an emergent morning peak: morning {morning} vs night {dead_of_night}"
        );
    }

    #[test]
    fn departure_window_wraps_midnight() {
        assert!(minute_in_window(5, DAY_MINUTES - 10, 20));
        assert!(minute_in_window(DAY_MINUTES - 10, DAY_MINUTES - 10, 20));
        assert!(!minute_in_window(100, DAY_MINUTES - 10, 20));
    }

    #[test]
    fn return_follows_the_stay() {
        let r = routine(11);
        let expected = (r.depart_minute + r.stay_minutes) % DAY_MINUTES;
        assert_eq!(r.return_minute(), expected);
        assert!(r.stay_minutes >= 30);
    }

    #[test]
    fn clock_labels_are_two_digit() {
        assert_eq!(clock_label(0), "00:00");
        assert_eq!(clock_label(7 * 60 + 24), "07:24");
        assert_eq!(clock_label(DAY_MINUTES + 61), "01:01");
    }
}
