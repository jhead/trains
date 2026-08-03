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
    /// A ring of trains each blocked by the next — nothing will ever move
    /// without more railway. 07 §4.1: congestion must be visible; a silent
    /// permanent stall is the one failure the player cannot diagnose.
    Gridlock,
}

impl AlertKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::StationServiceLow => "Service low",
            Self::StationOverwhelmed => "Station overwhelmed",
            Self::TrainsParked => "Trains parked",
            Self::CashLow => "Cash low",
            Self::NewDemand => "New demand",
            Self::Gridlock => "Gridlock",
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
    /// Keyed on the smallest train id in the ring, so one ring is one alert.
    Gridlock(crate::ids::TrainId),
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
#[allow(clippy::too_many_arguments)]
pub fn refresh_alerts(
    mut board: ResMut<AlertBoard>,
    money: Res<Money>,
    stations: Res<StationRegistry>,
    service: Res<StationService>,
    network: Res<TrackNetwork>,
    demand: Res<DemandSpawner>,
    // Optional so a partial world — a save-restore harness, a headless
    // embedder — can still refresh the rest of the board.
    occupancy: Option<Res<crate::trains::TileOccupancy>>,
    watch: Option<ResMut<GridlockWatch>>,
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
                    format!("New settlement: {} - not yet served", opp.name),
                    AlertFocus::Station(id),
                );
            }
            DemandOpportunityKind::Industry(id) => {
                let key = AlertKey::NewIndustry(id);
                active.push(key);
                board.upsert(
                    key,
                    AlertKind::NewDemand,
                    format!("New industry: {} - not yet served", opp.name),
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
                "Can't afford opex - {parked_count} train{} parked",
                if parked_count == 1 { "" } else { "s" }
            ),
            parked_focus,
        );
    }

    // Per *real* minute, which is what [`ALERT_CASH_LOW_MINUTES`] counts and
    // what the player is measuring their patience in. This used to multiply a
    // per-minute rate by six — the ticks in a sim-minute — which was neither
    // unit and put the warning six times too late.
    let opex_per_real_min: i64 = trains
        .iter()
        .filter(|(_, loc)| !loc.parked)
        .map(|(train, _)| TrainProfile::for_kind(train.kind).opex_cents_per_real_min)
        .sum();
    // Keep TRAIN_OPEX_CENTS referenced for API stability / docs.
    let _ = TRAIN_OPEX_CENTS;
    if opex_per_real_min > 0 {
        let reserve = opex_per_real_min.saturating_mul(ALERT_CASH_LOW_MINUTES);
        if money.cents() < reserve {
            let key = AlertKey::CashLow;
            active.push(key);
            board.upsert(
                key,
                AlertKind::CashLow,
                format!(
                    "Cash low - under {ALERT_CASH_LOW_MINUTES} min of opex"
                ),
                AlertFocus::None,
            );
        }
    }

    // A ring of trains each blocked by the next is the one congestion state
    // that never resolves itself — free-roamers on plain single track have no
    // passing loop to yield into. The reroute and yield machinery keeps
    // trying, so a ring that *can* break does; only one that has held for
    // [`GRIDLOCK_ALERT_TICKS`] is worth interrupting the player about.
    if let (Some(occupancy), Some(mut watch)) = (occupancy, watch) {
        gridlock_pass(
            &mut board,
            &mut active,
            &occupancy,
            &mut watch,
            &network,
            &trains,
        );
    }

    board.retain_keys(&active);
}

/// The gridlock half of [`refresh_alerts`]; see the comment at its call site.
fn gridlock_pass(
    board: &mut AlertBoard,
    active: &mut Vec<AlertKey>,
    occupancy: &crate::trains::TileOccupancy,
    watch: &mut GridlockWatch,
    network: &TrackNetwork,
    trains: &Query<(&crate::trains::Train, &TrainLocation)>,
) {
    for canonical in gridlock_rings(occupancy) {
        let now = occupancy.tick;
        let seen = *watch.since.entry(canonical).or_insert(now);
        // A new world restarts the tick count; a stale entry from the old one
        // must not fire instantly.
        let seen = seen.min(now);
        watch.since.insert(canonical, seen);
        if now.saturating_sub(seen) >= GRIDLOCK_ALERT_TICKS {
            let key = AlertKey::Gridlock(canonical);
            active.push(key);
            let focus = trains
                .iter()
                .find(|(t, _)| t.id == canonical)
                .and_then(|(_, loc)| network.piece(loc.track))
                .map(|p| AlertFocus::Tile(p.tile))
                .unwrap_or(AlertFocus::Train(canonical));
            board.upsert(
                key,
                AlertKind::Gridlock,
                "Trains gridlocked - a passing loop or double track frees them".into(),
                focus,
            );
        }
    }
    let rings: std::collections::HashSet<TrainId> =
        gridlock_rings(occupancy).into_iter().collect();
    watch.since.retain(|id, _| rings.contains(id));
}

/// Ticks a blocked ring must persist before the alert fires — ten real
/// seconds, long past any yield cooldown.
pub const GRIDLOCK_ALERT_TICKS: u64 = 640;

/// First-seen tick per blocked ring, keyed by the ring's smallest train id.
#[derive(Debug, Default, Resource)]
pub struct GridlockWatch {
    since: std::collections::HashMap<TrainId, u64>,
}

/// Every ring in the blocked-by graph, as its smallest member, ascending.
///
/// Walks each blocked train's chain with a visited set; a walk that returns
/// to a train already on its own path found a cycle. Sorted iteration keeps
/// the result deterministic.
fn gridlock_rings(occupancy: &crate::trains::TileOccupancy) -> Vec<TrainId> {
    let mut starts: Vec<TrainId> = occupancy.blocked_by.keys().copied().collect();
    starts.sort_unstable_by_key(|t| t.0);
    let mut rings = Vec::new();
    for start in starts {
        let mut path = Vec::new();
        let mut cur = start;
        loop {
            if path.contains(&cur) {
                // Cycle found; canonicalise on its smallest member so every
                // entry point into the same ring reports the same ring.
                let ring_start = path.iter().position(|&t| t == cur).unwrap_or(0);
                let canonical = path[ring_start..].iter().min_by_key(|t| t.0).copied();
                if let Some(c) = canonical {
                    if !rings.contains(&c) {
                        rings.push(c);
                    }
                }
                break;
            }
            path.push(cur);
            match occupancy.blocked_by.get(&cur) {
                Some(&next) => cur = next,
                None => break,
            }
            if path.len() > 128 {
                break;
            }
        }
    }
    rings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::StationId;

    /// The ring detector: mutual blocks are a ring, chains that end are not,
    /// and every entry point into one ring reports the same canonical id.
    #[test]
    fn a_blocked_ring_is_found_once_and_a_mere_queue_is_not() {
        let mut occupancy = crate::trains::TileOccupancy::default();
        // Ring: 3 -> 5 -> 9 -> 3. Queue into it: 12 -> 3. Plain chain: 20 -> 21.
        occupancy.blocked_by.insert(TrainId(3), TrainId(5));
        occupancy.blocked_by.insert(TrainId(5), TrainId(9));
        occupancy.blocked_by.insert(TrainId(9), TrainId(3));
        occupancy.blocked_by.insert(TrainId(12), TrainId(3));
        occupancy.blocked_by.insert(TrainId(20), TrainId(21));

        assert_eq!(gridlock_rings(&occupancy), vec![TrainId(3)]);
    }

    /// The alert waits out the persistence window, then fires; a ring that
    /// breaks first never interrupts anyone.
    #[test]
    fn a_gridlock_alert_waits_and_a_broken_ring_never_fires() {
        use bevy_app::App;
        use bevy_ecs::prelude::IntoScheduleConfigs;

        let mut app = App::new();
        app.init_resource::<AlertBoard>()
            .init_resource::<GridlockWatch>()
            .init_resource::<Money>()
            .init_resource::<StationRegistry>()
            .init_resource::<StationService>()
            .init_resource::<TrackNetwork>()
            .init_resource::<DemandSpawner>()
            .init_resource::<crate::trains::TileOccupancy>()
            .add_systems(bevy_app::Update, refresh_alerts.into_configs());

        let ring = |occ: &mut crate::trains::TileOccupancy| {
            occ.blocked_by.insert(TrainId(1), TrainId(2));
            occ.blocked_by.insert(TrainId(2), TrainId(1));
        };

        ring(&mut app.world_mut().resource_mut::<crate::trains::TileOccupancy>());
        app.update();
        assert!(
            app.world().resource::<AlertBoard>().is_empty(),
            "a fresh ring must not fire instantly"
        );

        // Still ringed after the window: fire.
        {
            let mut occ = app
                .world_mut()
                .resource_mut::<crate::trains::TileOccupancy>();
            occ.tick += GRIDLOCK_ALERT_TICKS;
        }
        app.update();
        let board = app.world().resource::<AlertBoard>();
        assert_eq!(board.len(), 1);
        assert_eq!(board.iter().next().unwrap().kind, AlertKind::Gridlock);

        // Ring breaks: the alert clears and the watch forgets.
        app.world_mut()
            .resource_mut::<crate::trains::TileOccupancy>()
            .blocked_by
            .clear();
        app.update();
        assert!(app.world().resource::<AlertBoard>().is_empty());
        // A new ring must wait the full window again.
        ring(&mut app.world_mut().resource_mut::<crate::trains::TileOccupancy>());
        app.update();
        assert!(app.world().resource::<AlertBoard>().is_empty());
    }

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
