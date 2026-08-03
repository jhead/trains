//! Frame-time instrumentation and a repeatable stress scenario.
//!
//! Entirely opt-in and entirely off unless an environment variable asks for it,
//! so a normal `cargo run` pays nothing for its existence. It lives beside the
//! diagnostic overlays because that is what it is: a diagnostic.
//!
//! ```text
//! RAIL_TOWN_PERF=1            log frame-time percentiles + entity counts
//! RAIL_TOWN_PERF_STRESS=1     build a played-in town at boot (stations,
//!                             track, full service) so the numbers reflect a
//!                             real game rather than an empty map
//! RAIL_TOWN_PERF_SECS=20      quit after N seconds (for scripted runs)
//! RAIL_TOWN_PERF_NOVSYNC=1    uncap the frame rate, so the report shows the
//!                             real cost of a frame rather than the refresh
//! ```
//!
//! The stress scenario is deterministic: fixed station tiles on the default
//! 64x64 seed, fixed service scores, fixed speed. Two runs are comparable.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use rail_map::MapGrid;
use rail_sim::stations::StationTier;
use rail_sim::{
    AutoFillTrack, CommandBuffer, CommandKind, PlaceStation, SimClock, StationRegistry,
    StationService, TileCoord, TownDensity, GROUND_LAYER,
};

/// Read once at plugin build; absent means the whole module stays unregistered.
fn flag(name: &str) -> bool {
    std::env::var(name).map(|v| v != "0").unwrap_or(false)
}

// ─ Per-scope timing ────────────────────────────────────
//
// `sample`-style profilers attribute by symbol, which in a Bevy app spreads one
// system's cost across the twenty library functions it called. These scopes
// attribute by *system*, which is the unit a fix is written against.

/// Whether [`scope`] does anything. One relaxed atomic load on the fast path,
/// and the whole facility compiles down to that when perf is off.
static SCOPES_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

type ScopeTable = std::sync::Mutex<std::collections::HashMap<&'static str, (u64, u32)>>;

fn scope_table() -> &'static ScopeTable {
    static TABLE: std::sync::OnceLock<ScopeTable> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Accumulates nanoseconds into the named bucket when it is dropped.
pub(crate) struct ScopeGuard(Option<(&'static str, std::time::Instant)>);

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        let Some((name, started)) = self.0.take() else {
            return;
        };
        let nanos = started.elapsed().as_nanos() as u64;
        if let Ok(mut table) = scope_table().lock() {
            let slot = table.entry(name).or_insert((0, 0));
            slot.0 += nanos;
            slot.1 += 1;
        }
    }
}

/// Time the enclosing block under `name`. Free unless `RAIL_TOWN_PERF` is set.
///
/// ```ignore
/// let _perf = crate::overlays::perf::scope("sync_lit_windows");
/// ```
#[inline]
pub(crate) fn scope(name: &'static str) -> ScopeGuard {
    if SCOPES_ON.load(std::sync::atomic::Ordering::Relaxed) {
        ScopeGuard(Some((name, std::time::Instant::now())))
    } else {
        ScopeGuard(None)
    }
}

/// Drain the scope table into a sorted `(name, ms_per_frame, calls)` list.
fn drain_scopes(frames: u32) -> Vec<(&'static str, f64, u32)> {
    let Ok(mut table) = scope_table().lock() else {
        return Vec::new();
    };
    let mut rows: Vec<(&'static str, f64, u32)> = table
        .iter()
        .map(|(name, (nanos, calls))| {
            (
                *name,
                *nanos as f64 / 1e6 / frames.max(1) as f64,
                *calls,
            )
        })
        .collect();
    table.clear();
    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    rows
}

pub struct PerfPlugin;

impl Plugin for PerfPlugin {
    fn build(&self, app: &mut App) {
        if !flag("RAIL_TOWN_PERF") {
            return;
        }
        SCOPES_ON.store(true, std::sync::atomic::Ordering::Relaxed);
        app.add_plugins(FrameTimeDiagnosticsPlugin::default())
            .init_resource::<PerfWindow>()
            .add_systems(Update, report_frame_time);

        if flag("RAIL_TOWN_PERF_NOVSYNC") {
            app.add_systems(Update, uncap_frame_rate);
        }
        if flag("RAIL_TOWN_PERF_STRESS") {
            app.init_resource::<StressState>()
                .add_systems(Update, drive_stress);
        }
        if let Ok(secs) = std::env::var("RAIL_TOWN_PERF_SECS") {
            if let Ok(secs) = secs.parse::<f32>() {
                app.insert_resource(PerfDeadline(secs))
                    .add_systems(Update, quit_after_deadline);
            }
        }
    }
}

// ─ Frame-time reporting ────────────────────────────────

/// A rolling window of raw frame times, so the report can show a p99 rather
/// than only the smoothed average Bevy keeps.
#[derive(Resource)]
struct PerfWindow {
    samples: Vec<f32>,
    since_report: f32,
    reports: u32,
}

impl Default for PerfWindow {
    fn default() -> Self {
        Self {
            samples: Vec::with_capacity(512),
            since_report: 0.0,
            reports: 0,
        }
    }
}

/// Seconds between reports.
const REPORT_EVERY: f32 = 2.0;
/// Reports to discard before the numbers count — window creation, shader
/// compilation and the first atlas bakes all land in the first second or two.
const WARMUP_REPORTS: u32 = 1;

#[allow(clippy::too_many_arguments)]
fn report_frame_time(
    time: Res<Time<Real>>,
    diagnostics: Res<DiagnosticsStore>,
    mut window: ResMut<PerfWindow>,
    sprites: Query<(), With<Sprite>>,
    entities: Query<()>,
    density: Res<TownDensity>,
    stations: Res<StationRegistry>,
) {
    window.samples.push(time.delta_secs() * 1000.0);
    window.since_report += time.delta_secs();
    if window.since_report < REPORT_EVERY {
        return;
    }
    window.since_report = 0.0;
    window.reports += 1;

    let mut sorted = std::mem::take(&mut window.samples);
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    if n == 0 {
        return;
    }
    let pick = |q: f32| sorted[((n as f32 - 1.0) * q) as usize];
    let mean: f32 = sorted.iter().sum::<f32>() / n as f32;
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);

    let tag = if window.reports <= WARMUP_REPORTS {
        "warmup"
    } else {
        "steady"
    };
    info!(
        "PERF[{tag}] fps {fps:6.1} | frame ms mean {mean:6.2} p50 {:6.2} p95 {:6.2} p99 {:6.2} max {:6.2} \
         | sprites {:5} entities {:5} density_cells {:4} stations {}",
        pick(0.50),
        pick(0.95),
        pick(0.99),
        sorted[n - 1],
        sprites.iter().count(),
        entities.iter().count(),
        density.len(),
        stations.len(),
    );

    for (name, ms, calls) in drain_scopes(n as u32) {
        if ms >= 0.02 {
            info!("PERF[{tag}]   {ms:8.3} ms/frame  {calls:6} calls  {name}");
        }
    }
}

/// Take the refresh-rate cap off, so a measurement reports the cost of the
/// frame instead of the cost of waiting for the display.
fn uncap_frame_rate(mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>) {
    for mut window in windows.iter_mut() {
        if window.present_mode != bevy::window::PresentMode::AutoNoVsync {
            window.present_mode = bevy::window::PresentMode::AutoNoVsync;
        }
    }
}

// ─ Deadline ────────────────────────────────────────────

#[derive(Resource)]
struct PerfDeadline(f32);

fn quit_after_deadline(
    time: Res<Time<Real>>,
    deadline: Res<PerfDeadline>,
    mut exit: MessageWriter<AppExit>,
) {
    if time.elapsed_secs() >= deadline.0 {
        exit.write(AppExit::Success);
    }
}

// ─ Stress scenario ─────────────────────────────────────

/// Station tiles for the scripted town. Chosen inside the default 64x64 map and
/// filtered against terrain at run time, so an unbuildable pick is skipped
/// rather than wedging the scenario.
const STRESS_STATIONS: [(i32, i32); 6] = [
    (14, 14),
    (30, 16),
    (46, 20),
    (18, 34),
    (34, 40),
    (48, 46),
];

#[derive(Resource, Default)]
struct StressState {
    frame: u32,
    placed: bool,
}

/// Place the town, then hold every station at full service so density climbs to
/// its target instead of decaying. This is the state a played-in map reaches;
/// getting there through real train arrivals would take minutes of wall clock.
fn drive_stress(
    mut state: ResMut<StressState>,
    mut buffer: ResMut<CommandBuffer>,
    mut service: ResMut<StationService>,
    mut clock: ResMut<SimClock>,
    stations: Res<StationRegistry>,
    map: Option<Res<MapGrid>>,
) {
    state.frame += 1;
    if state.frame < 4 {
        return;
    }

    if !state.placed {
        let Some(map) = map else { return };
        let mut placed: Vec<TileCoord> = Vec::new();
        for (x, y) in STRESS_STATIONS {
            let tile = TileCoord { x, y };
            let cell = map.tile(tile);
            if cell.water {
                continue;
            }
            buffer.push(CommandKind::PlaceStation(PlaceStation::new(
                tile,
                GROUND_LAYER,
                StationTier::default(),
                None,
            )));
            placed.push(tile);
        }
        // A line through the lot, so the corridor / commercial classification
        // and the track visuals are exercised too.
        for pair in placed.windows(2) {
            buffer.push(CommandKind::AutoFillTrack(AutoFillTrack {
                from: pair[0],
                to: pair[1],
                layer: GROUND_LAYER,
            }));
        }
        clock.paused = false;
        clock.speed_multiplier = 3;
        state.placed = true;
        return;
    }

    // Hold service at full every frame: the growth system reads it directly.
    for station in stations.iter() {
        let score = service.ensure(station.id);
        score.score = 100;
        score.tier = station.tier;
    }
}
