//! First-run teaching — three light touches, and nothing else.
//!
//! Design brief: [`docs/design/09-shell-and-menus.md`](../../../docs/design/09-shell-and-menus.md) §7,
//! which opens by ruling out almost everything a module like this usually does:
//!
//! > **No tutorial popups. No modal lecture. No forced sequence.**
//!
//! So this module contains no tutorial. The opening map does the teaching
//! ([02 §4.1](../../../docs/design/02-world-and-terrain.md)), and these three
//! touches sit on top of it:
//!
//! | Touch | Where | Module |
//! | --- | --- | --- |
//! | A gentle nudge naming the nearby destination | Town Talk | [`nudge`] |
//! | Contextual hints, once each, never blocking | A chip by the toolbar | [`hints`] |
//! | The first payout celebrated | A warm banner over the balance | [`payout`] |
//!
//! Everything here is dismissible, nothing here is modal, and nothing here ever
//! takes the pointer or the keyboard away from the player. The test the brief
//! sets is *"a player who reads nothing should be laying track within thirty
//! seconds and have earned money within three minutes"* — which is a test of
//! the toolbar and the map, not of this module. These touches make that arc
//! legible; they must never stand in the way of it.
//!
//! # Once each, and once *ever*
//!
//! A hint that returns is worse than no hint at all. "Seen" is per-player, not
//! per-world, so it is persisted beside the settings through
//! [`crate::shell::persist`] rather than into a save. See [`Onboarding`].

mod hints;
mod nudge;
mod payout;

use bevy::prelude::*;

#[allow(unused_imports)] // `HintChip` is the module's marker for other slices.
pub use hints::{ActiveHint, Hint, HintChip};
pub use nudge::OpeningNudge;
pub use payout::FirstPayout;

use crate::shell::persist::{self, KvDoc};

/// File the "seen" flags live in, beside `settings.ron`.
pub const ONBOARDING_FILE: &str = "onboarding.ron";

/// Which light touches this player has already had.
///
/// Deliberately a *player* record, not a world one: somebody who has laid track
/// before does not need to be told how on their fourth map, and a hint that
/// reappears after a reload reads as a bug.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct Onboarding {
    seen: Vec<Hint>,
    /// Set when [`Self::mark_seen`] changed something, so the writer knows.
    dirty: bool,
}

impl Onboarding {
    /// Read the player's record. A missing or unreadable file is a first run,
    /// which is the correct interpretation either way.
    pub fn load() -> Self {
        let Some(doc) = persist::load_doc(ONBOARDING_FILE) else {
            return Self::default();
        };
        Self {
            seen: Hint::ALL
                .iter()
                .copied()
                .filter(|hint| doc.bool(hint.key(), false))
                .collect(),
            dirty: false,
        }
    }

    /// Write the record. Errors are returned rather than panicked on — a
    /// read-only profile must never stop the game, it just means the hints
    /// come back next launch.
    pub fn save(&self) -> std::io::Result<()> {
        let mut doc = KvDoc::new();
        for hint in Hint::ALL {
            doc.set_bool(hint.key(), self.has_seen(*hint));
        }
        persist::save_doc(ONBOARDING_FILE, &doc)
    }

    pub fn has_seen(&self, hint: Hint) -> bool {
        self.seen.contains(&hint)
    }

    /// Record a hint as shown. Returns `true` the first time only, which is
    /// what makes "once each, never repeated" a property of the data rather
    /// than of every caller remembering to check.
    pub fn mark_seen(&mut self, hint: Hint) -> bool {
        if self.has_seen(hint) {
            return false;
        }
        self.seen.push(hint);
        self.dirty = true;
        true
    }

    /// Forget everything. Not reachable from the UI — it exists so a developer
    /// (or a test) can see the first run again without deleting a file by hand.
    #[allow(dead_code)] // Deliberate escape hatch; nothing in the app calls it.
    pub fn reset(&mut self) {
        self.seen.clear();
        self.dirty = true;
    }
}

/// The three light touches of design 09 §7.
pub struct OnboardingPlugin;

impl Plugin for OnboardingPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Onboarding::load())
            .init_resource::<OpeningNudge>()
            .init_resource::<FirstPayout>()
            .add_systems(Startup, hints::setup_hint_chip)
            .add_systems(Startup, payout::setup_payout_banner)
            .add_systems(
                Update,
                (
                    nudge::nudge_toward_the_first_destination,
                    hints::watch_for_hint_moments,
                    hints::paint_hint_chip.after(hints::watch_for_hint_moments),
                    hints::dismiss_hint_chip.after(hints::paint_hint_chip),
                    payout::celebrate_the_first_payout,
                    payout::fade_payout_banner.after(payout::celebrate_the_first_payout),
                    persist_onboarding_on_change,
                ),
            );
    }
}

/// Write the record whenever a hint is first shown, so a crash — or simply
/// quitting without a save — never resurrects a hint the player has had.
fn persist_onboarding_on_change(mut onboarding: ResMut<Onboarding>) {
    if !onboarding.dirty {
        return;
    }
    onboarding.dirty = false;
    if let Err(err) = onboarding.save() {
        warn!("could not write onboarding state: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_player_has_seen_nothing() {
        let onboarding = Onboarding::default();
        for hint in Hint::ALL {
            assert!(!onboarding.has_seen(*hint));
        }
    }

    #[test]
    fn a_hint_fires_once_and_never_again() {
        let mut onboarding = Onboarding::default();
        assert!(onboarding.mark_seen(Hint::Build), "first time shows");
        assert!(
            !onboarding.mark_seen(Hint::Build),
            "and never a second time"
        );
        assert!(onboarding.has_seen(Hint::Build));
        assert!(
            !onboarding.has_seen(Hint::Train),
            "one hint does not consume another"
        );
    }

    #[test]
    fn marking_a_hint_asks_for_a_write_exactly_once() {
        let mut onboarding = Onboarding::default();
        onboarding.mark_seen(Hint::Build);
        assert!(onboarding.dirty);
        onboarding.dirty = false;
        onboarding.mark_seen(Hint::Build);
        assert!(!onboarding.dirty, "a no-op must not rewrite the file");
    }

    #[test]
    fn the_record_survives_a_round_trip_through_the_file_format() {
        let mut onboarding = Onboarding::default();
        onboarding.mark_seen(Hint::Build);
        onboarding.mark_seen(Hint::Ledger);

        let mut doc = KvDoc::new();
        for hint in Hint::ALL {
            doc.set_bool(hint.key(), onboarding.has_seen(*hint));
        }
        let parsed = KvDoc::parse(&doc.to_ron());
        for hint in Hint::ALL {
            assert_eq!(
                parsed.bool(hint.key(), false),
                onboarding.has_seen(*hint),
                "{hint:?} did not survive"
            );
        }
    }

    #[test]
    fn every_hint_has_a_distinct_storage_key() {
        let mut keys: Vec<&str> = Hint::ALL.iter().map(|h| h.key()).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before, "two hints share a key");
    }

    #[test]
    fn resetting_brings_the_first_run_back() {
        let mut onboarding = Onboarding::default();
        for hint in Hint::ALL {
            onboarding.mark_seen(*hint);
        }
        onboarding.reset();
        assert!(Hint::ALL.iter().all(|h| !onboarding.has_seen(*h)));
    }
}
