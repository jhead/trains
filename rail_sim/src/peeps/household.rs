//! Households — peeps who share a building and move together.
//!
//! Brief 06 §4.2: *"peeps live together, share a building, and move together.
//! A family leaving is a bigger event than a person leaving."* The registry is
//! the authority on who lives where; peeps carry only a [`HouseholdId`].

use std::collections::HashMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::ids::{StationId, TileCoord};

use super::names::{family_name, family_plural};
use super::PeepId;

/// Stable id for a household (a building's occupants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HouseholdId(pub u64);

/// Smallest / largest number of peeps sharing one home.
pub const HOUSEHOLD_MIN: usize = 1;
pub const HOUSEHOLD_MAX: usize = 3;

/// A family sharing one building.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Household {
    pub id: HouseholdId,
    /// Family name shared by every member.
    pub family: String,
    /// The building tile they live on.
    pub home: TileCoord,
    /// Station this household walks to.
    pub home_station: StationId,
    pub members: Vec<PeepId>,
    /// Sim tick they moved in — feeds *"has lived in Eastgate for 14 days"*.
    pub moved_in_tick: u64,
    /// Packing up. Set once so Town Talk announces the departure a single time.
    pub leaving: bool,
}

impl Household {
    /// Town Talk subject — `"The Aldertons"`.
    pub fn plural(&self) -> String {
        family_plural(&self.family)
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
}

/// Every household in the town, keyed by id.
#[derive(Debug, Clone, Default, Resource)]
pub struct HouseholdRegistry {
    households: HashMap<HouseholdId, Household>,
    next_id: u64,
}

impl HouseholdRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.households.len()
    }

    pub fn is_empty(&self) -> bool {
        self.households.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Household> {
        self.households.values()
    }

    pub fn get(&self, id: HouseholdId) -> Option<&Household> {
        self.households.get(&id)
    }

    pub fn get_mut(&mut self, id: HouseholdId) -> Option<&mut Household> {
        self.households.get_mut(&id)
    }

    /// Households living within the growth ring of a station.
    pub fn at_station(&self, station: StationId) -> impl Iterator<Item = &Household> {
        self.households
            .values()
            .filter(move |h| h.home_station == station)
    }

    /// Create a household on `home`; the family name is drawn from the id.
    pub fn insert(
        &mut self,
        home: TileCoord,
        home_station: StationId,
        moved_in_tick: u64,
    ) -> HouseholdId {
        self.next_id = self.next_id.saturating_add(1);
        let id = HouseholdId(self.next_id);
        self.households.insert(
            id,
            Household {
                id,
                family: family_name(id.0).to_string(),
                home,
                home_station,
                members: Vec::new(),
                moved_in_tick,
                leaving: false,
            },
        );
        id
    }

    pub fn add_member(&mut self, id: HouseholdId, peep: PeepId) {
        if let Some(h) = self.households.get_mut(&id) {
            if !h.members.contains(&peep) {
                h.members.push(peep);
            }
        }
    }

    /// Drop one member; returns `true` when the house is now empty.
    pub fn remove_member(&mut self, id: HouseholdId, peep: PeepId) -> bool {
        let Some(h) = self.households.get_mut(&id) else {
            return false;
        };
        h.members.retain(|p| *p != peep);
        h.members.is_empty()
    }

    /// Remove the whole household (they left town).
    pub fn remove(&mut self, id: HouseholdId) -> Option<Household> {
        self.households.remove(&id)
    }

    /// Total residents across all households.
    pub fn population(&self) -> usize {
        self.households.values().map(|h| h.members.len()).sum()
    }

    /// Next id the registry will hand out — snapshot this so a reloaded town
    /// never reuses a household id.
    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    /// Rebuild a registry from a snapshot.
    ///
    /// [`Household`] is plainly serialisable; the registry itself is not, so
    /// save/load stores the list and calls this rather than reaching inside.
    pub fn restore(households: impl IntoIterator<Item = Household>, next_id: u64) -> Self {
        let mut map = HashMap::new();
        let mut high = next_id;
        for household in households {
            high = high.max(household.id.0);
            map.insert(household.id, household);
        }
        Self {
            households: map,
            next_id: high,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(x: i32, y: i32) -> TileCoord {
        TileCoord { x, y }
    }

    #[test]
    fn members_share_one_home_and_family_name() {
        let mut reg = HouseholdRegistry::new();
        let id = reg.insert(tile(4, 4), StationId(1), 0);
        reg.add_member(id, PeepId(1));
        reg.add_member(id, PeepId(2));

        let h = reg.get(id).unwrap();
        assert_eq!(h.members.len(), 2);
        assert_eq!(h.home, tile(4, 4));
        assert!(!h.family.is_empty());
        assert!(h.plural().starts_with("The "));
    }

    #[test]
    fn removing_last_member_empties_the_house() {
        let mut reg = HouseholdRegistry::new();
        let id = reg.insert(tile(0, 0), StationId(1), 0);
        reg.add_member(id, PeepId(1));
        reg.add_member(id, PeepId(2));
        assert!(!reg.remove_member(id, PeepId(1)));
        assert!(reg.remove_member(id, PeepId(2)));
        assert_eq!(reg.population(), 0);
    }

    #[test]
    fn restore_round_trips_and_never_reuses_an_id() {
        let mut reg = HouseholdRegistry::new();
        let a = reg.insert(tile(1, 1), StationId(1), 7);
        reg.add_member(a, PeepId(1));
        let saved: Vec<Household> = reg.iter().cloned().collect();
        let next = reg.next_id();

        let mut restored = HouseholdRegistry::restore(saved, next);
        assert_eq!(restored.population(), 1);
        assert_eq!(restored.get(a).unwrap().moved_in_tick, 7);
        let fresh = restored.insert(tile(5, 5), StationId(1), 8);
        assert_ne!(fresh, a);
    }

    #[test]
    fn households_index_by_station() {
        let mut reg = HouseholdRegistry::new();
        let a = reg.insert(tile(1, 1), StationId(1), 0);
        reg.insert(tile(9, 9), StationId(2), 0);
        reg.add_member(a, PeepId(1));
        assert_eq!(reg.at_station(StationId(1)).count(), 1);
        assert_eq!(reg.at_station(StationId(2)).count(), 1);
        assert_eq!(reg.at_station(StationId(3)).count(), 0);
    }
}
