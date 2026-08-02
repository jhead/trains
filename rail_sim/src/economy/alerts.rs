//! Actionable, non-modal alerts for things needing attention off-screen.

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::demand::{DemandOpportunityKind, DemandSpawner};
use crate::ids::{StationId, TileCoord, TrainId};
use crate::money::Money;
use crate::stations::{IndustryId, StationRegistry, StationService};
use crate::track::TrackNetwork;
use crate::trains::{TrainLocation, TrainProfile};

use super::opex::TRAIN_OPEX_CENTS;

/// Service score at or below this raises a “service low” alert.
pub const ALERT_SERVICE_LOW_SCORE: u8 = 30;
/// Waiting passengers at or above this raises “station overwhelmed”.
pub const ALERT_WAITING_OVERWHELMED: u32 = 8;
/// Cash below this many minutes of active-train opex → “cash low”.
pub const ALERT_CASH_LOW_MINUTES: i64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AlertKind {
    StationServiceLow,
    StationOverwhelmed,
    TrainsParked,
    CashLow,
    /// New settlement / industry still outside the rail network.
    NewDemand,
}

impl AlertKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::StationServiceLow => "Service low",
            Self::StationOverwhelmed => "Station overwhelmed",
            Self::TrainsParked => "Trains parked",
            Self::CashLow => "Cash low",
            Self::NewDemand => "New demand",
        }
    }
}

/// Where clicking an alert should fly the camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertFocus {
    Tile(TileCoord),
    Station(StationId),
    Train(TrainId),
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    pub id: u64,
    pub kind: AlertKind,
    pub message: String,
    pub focus: AlertFocus,
    /// Stable key for dedupe / refresh (not the display id).
    pub key: AlertKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AlertKey {
    StationService(StationId),
    StationWaiting(StationId),
    TrainsParked,
    CashLow,
    NewSettlement(StationId),
    NewIndustry(IndustryId),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Resource, Serialize, Deserialize)]
pub struct AlertBoard {
    alerts: Vec<Alert>,
    next_id: u64,
    dismissed: Vec<AlertKey>,
}

impl AlertBoard {
    pub fn iter(&self) -> impl Iterator<Item = &Alert> {
        self.alerts.iter()
    }

    pub fn len(&self) -> usize {
        self.alerts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.alerts.is_empty()
    }

    pub fn dismiss(&mut self, id: u64) {
        if let Some(pos) = self.alerts.iter().position(|a| a.id == id) {
            let key = self.alerts[pos].key;
            self.alerts.remove(pos);
            if !self.dismissed.contains(&key) {
                self.dismissed.push(key);
            }
        }
    }

    pub fn dismiss_all(&mut self) {
        for a in self.alerts.drain(..) {
            if !self.dismissed.contains(&a.key) {
                self.dismissed.push(a.key);
            }
        }
    }

    pub fn clear_dismissed(&mut self) {
        self.dismissed.clear();
    }

    fn is_dismissed(&self, key: AlertKey) -> bool {
        self.dismissed.contains(&key)
    }

    fn upsert(&mut self, key: AlertKey, kind: AlertKind, message: String, focus: AlertFocus) {
        if self.is_dismissed(key) {
            return;
        }
        if let Some(existing) = self.alerts.iter_mut().find(|a| a.key == key) {
            existing.message = message;
            existing.focus = focus;
            existing.kind = kind;
            return;
        }
        self.next_id = self.next_id.saturating_add(1);
        self.alerts.push(Alert {
            id: self.next_id,
            kind,
            message,
            focus,
            key,
        });
    }

    fn retain_keys(&mut self, active: &[AlertKey]) {
        self.alerts.retain(|a| active.contains(&a.key));
        // Allow re-fire once the condition clears.
        self.dismissed.retain(|k| active.contains(k));
    }
}

/// Rebuild active alerts from sim state (idempotent, non-modal).
pub fn refresh_alerts(
    mut board: ResMut<AlertBoard>,
    money: Res<Money>,
    stations: Res<StationRegistry>,
    service: Res<StationService>,
    network: Res<TrackNetwork>,
    demand: Res<DemandSpawner>,
    trains: Query<(&crate::trains::Train, &TrainLocation)>,
) {
    let mut active: Vec<AlertKey> = Vec::new();

    for opp in &demand.open {
        match opp.kind {
            DemandOpportunityKind::Settlement(id) => {
                let key = AlertKey::NewSettlement(id);
                active.push(key);
                board.upsert(
                    key,
                    AlertKind::NewDemand,
                    format!("New settlement: {} — not yet served", opp.name),
                    AlertFocus::Station(id),
                );
            }
            DemandOpportunityKind::Industry(id) => {
                let key = AlertKey::NewIndustry(id);
                active.push(key);
                board.upsert(
                    key,
                    AlertKind::NewDemand,
                    format!("New industry: {} — not yet served", opp.name),
                    AlertFocus::Tile(opp.tile),
                );
            }
        }
    }

    for station in stations.iter() {
        let score = service.score(station.id);
        if score.score <= ALERT_SERVICE_LOW_SCORE {
            let key = AlertKey::StationService(station.id);
            active.push(key);
            board.upsert(
                key,
                AlertKind::StationServiceLow,
                format!(
                    "{} service low ({})",
                    station.name, score.score
                ),
                AlertFocus::Station(station.id),
            );
        }
        if score.waiting_passengers >= ALERT_WAITING_OVERWHELMED {
            let key = AlertKey::StationWaiting(station.id);
            active.push(key);
            board.upsert(
                key,
                AlertKind::StationOverwhelmed,
                format!(
                    "{} overwhelmed ({} waiting)",
                    station.name, score.waiting_passengers
                ),
                AlertFocus::Station(station.id),
            );
        }
    }

    let mut parked_focus = AlertFocus::None;
    let mut parked_count = 0u32;
    for (train, loc) in trains.iter() {
        if loc.parked {
            parked_count += 1;
            if matches!(parked_focus, AlertFocus::None) {
                if let Some(piece) = network.piece(loc.track) {
                    parked_focus = AlertFocus::Tile(piece.tile);
                } else {
                    parked_focus = AlertFocus::Train(train.id);
                }
            }
        }
    }
    if parked_count > 0 {
        let key = AlertKey::TrainsParked;
        active.push(key);
        board.upsert(
            key,
            AlertKind::TrainsParked,
            format!(
                "Can't afford opex — {parked_count} train{} parked",
                if parked_count == 1 { "" } else { "s" }
            ),
            parked_focus,
        );
    }

    let opex_per_min: i64 = trains
        .iter()
        .filter(|(_, loc)| !loc.parked)
        .map(|(train, _)| TrainProfile::for_kind(train.kind).opex_cents.saturating_mul(6))
        .sum();
    // Keep TRAIN_OPEX_CENTS referenced for API stability / docs.
    let _ = TRAIN_OPEX_CENTS;
    if opex_per_min > 0 {
        let reserve = opex_per_min.saturating_mul(ALERT_CASH_LOW_MINUTES);
        if money.cents() < reserve {
            let key = AlertKey::CashLow;
            active.push(key);
            board.upsert(
                key,
                AlertKind::CashLow,
                format!(
                    "Cash low — under {ALERT_CASH_LOW_MINUTES} min of opex"
                ),
                AlertFocus::None,
            );
        }
    }

    board.retain_keys(&active);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::StationId;

    #[test]
    fn dismiss_suppresses_until_cleared() {
        let mut board = AlertBoard::default();
        let key = AlertKey::CashLow;
        board.upsert(
            key,
            AlertKind::CashLow,
            "Cash low".into(),
            AlertFocus::None,
        );
        assert_eq!(board.len(), 1);
        let id = board.iter().next().unwrap().id;
        board.dismiss(id);
        assert!(board.is_empty());
        board.upsert(
            key,
            AlertKind::CashLow,
            "Cash low".into(),
            AlertFocus::None,
        );
        assert!(board.is_empty(), "dismissed keys stay quiet while active");
        board.retain_keys(&[]); // condition cleared
        board.upsert(
            key,
            AlertKind::CashLow,
            "Cash low".into(),
            AlertFocus::None,
        );
        assert_eq!(board.len(), 1, "may re-fire after condition clears");
    }

    #[test]
    fn upsert_updates_same_key() {
        let mut board = AlertBoard::default();
        let sid = StationId(1);
        let key = AlertKey::StationService(sid);
        board.upsert(
            key,
            AlertKind::StationServiceLow,
            "A".into(),
            AlertFocus::Station(sid),
        );
        board.upsert(
            key,
            AlertKind::StationServiceLow,
            "B".into(),
            AlertFocus::Tile(TileCoord { x: 1, y: 2 }),
        );
        assert_eq!(board.len(), 1);
        assert_eq!(board.iter().next().unwrap().message, "B");
    }
}
