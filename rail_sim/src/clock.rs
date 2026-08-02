//! Sim pause / speed. Presentation syncs Bevy `Time<Virtual>` from this resource;
//! domain systems gate work with [`sim_is_running`].

use bevy_ecs::prelude::Resource;

use crate::commands::{Pause, SetSpeed};

/// Primary UX speeds for MVP (2x is allowed via [`SetSpeed`] as well).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimSpeed {
    /// Real-time sim steps.
    X1,
    /// Triple-speed sim steps.
    X3,
}

impl SimSpeed {
    pub fn multiplier(self) -> u8 {
        match self {
            Self::X1 => 1,
            Self::X3 => 3,
        }
    }

    pub fn from_multiplier(multiplier: u8) -> Self {
        if multiplier >= 3 {
            Self::X3
        } else {
            Self::X1
        }
    }
}

/// Authoritative pause + speed for the simulation.
///
/// - [`apply_commands`](crate::apply::apply_commands) always runs (build while paused).
/// - Systems that advance the world should use `.run_if(sim_is_running)`.
/// - `rail_town` syncs Bevy virtual time speed from [`Self::relative_speed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Resource)]
pub struct SimClock {
    pub paused: bool,
    /// Speed multiplier while unpaused (`1` or `3` for MVP UX; other values ok).
    pub speed_multiplier: u8,
}

impl Default for SimClock {
    fn default() -> Self {
        Self {
            paused: false,
            speed_multiplier: 1,
        }
    }
}

impl SimClock {
    pub fn is_running(&self) -> bool {
        !self.paused
    }

    pub fn speed(&self) -> SimSpeed {
        SimSpeed::from_multiplier(self.speed_multiplier)
    }

    /// Relative speed for Bevy `Time<Virtual>` while the sim is running.
    ///
    /// When paused this still returns the stored multiplier; the bridge should
    /// keep virtual time at `1.0` so FixedUpdate continues to drain commands.
    pub fn relative_speed(&self) -> f32 {
        let m = self.speed_multiplier.max(1) as f32;
        m
    }

    pub fn apply_pause(&mut self, pause: Pause) {
        self.paused = pause.paused;
    }

    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }

    pub fn apply_set_speed(&mut self, set: SetSpeed) {
        // Multiplier 0 is treated as pause via SetSpeed for convenience.
        if set.multiplier == 0 {
            self.paused = true;
            return;
        }
        self.speed_multiplier = set.multiplier;
        self.paused = false;
    }
}

/// Run condition: sim world advancement (trains, growth, opex) should proceed.
pub fn sim_is_running(clock: bevy_ecs::prelude::Res<SimClock>) -> bool {
    clock.is_running()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_speed_unpauses_and_stores_multiplier() {
        let mut clock = SimClock {
            paused: true,
            speed_multiplier: 1,
        };
        clock.apply_set_speed(SetSpeed { multiplier: 3 });
        assert!(!clock.paused);
        assert_eq!(clock.speed_multiplier, 3);
        assert_eq!(clock.speed(), SimSpeed::X3);
    }

    #[test]
    fn pause_command_toggles_running_flag() {
        let mut clock = SimClock::default();
        clock.apply_pause(Pause { paused: true });
        assert!(!clock.is_running());
        clock.apply_pause(Pause { paused: false });
        assert!(clock.is_running());
    }
}
