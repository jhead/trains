# Rail Town

Calm pixel-art railway sandbox. Design brief: [`DESIGN.md`](./DESIGN.md).

## Docs

- [`docs/design/`](./docs/design/README.md) — **the design briefs**: art direction, world, UI, building, inspection, town, lines, economy, shell, audio, roadmap
- [`docs/PROGRESS-AUDIT.md`](./docs/PROGRESS-AUDIT.md) — where the build stands against those briefs
- [`docs/TECH_STACK.md`](./docs/TECH_STACK.md) — Bevy ECS, Steam + web targets, crate split
- [`docs/IMPLEMENTATION_PLAN.md`](./docs/IMPLEMENTATION_PLAN.md) — MVP scope, MP seams, slices

## Workspace

| Crate | Role |
| --- | --- |
| `rail_town` | Bevy binary — window, camera, input, UI |
| `rail_sim` | Sim library — commands, IDs, fixed-tick systems (no rendering) |
| `rail_map` | Map / terrain library |
| `rail_net` | Neighbor exchange stub (`NeighborBackend` + `NullNeighbor`) |

## Run (native)

```bash
cargo run -p rail_town
```

Default map: **64×64**, seed **42** (`rail_map::DEFAULT_MAP_*` / `MapPlugin::default()`).

### Map controls

| Input | Action |
| --- | --- |
| WASD / arrow keys | Pan camera |
| Mouse scroll | Zoom in / out |

### Track tools

| Input | Action |
| --- | --- |
| `B` | Build tool |
| `X` | Demolish tool |
| Left press–drag–release (Build) | Live ghost of an ortho/45° run; release commits (`PlaceTrack` / `AutoFillTrack`). Endpoint stays as continuous-build anchor |
| `Shift` while dragging | Exact straight only (no snap) — off-axis shows a reason chip |
| `Ctrl` while dragging | Single tile under the cursor |
| Right-drag (Build or Demolish) | Demolish along the path with full refund preview |
| `Esc` | Clear continuous-build anchor |
| Left-drag (Demolish tool) | Same demolish path as right-drag |

While dragging: **cost HUD** (tile count, cost, balance-after) follows the cursor; invalid tiles tint `warn`; rejects show a plain-language **reason chip** and flash the offending tiles (never silent).

**Costs:** ground track `$10` (`TRACK_COST_CENTS = 1000`); bridge over water `$50` (`BRIDGE_COST_CENTS = 5000`). Demolish refunds the full amount paid for that tile. Bridges are limited to `MAX_BRIDGE_SPAN = 3` contiguous water tiles.

**Path proposal (Phase A):** default snaps to nearest ortho/45° ray. Smart A* contour routing is Phase C.

### Stations & industries (auto-seeded)

At startup the sim places **3 named stations** (Eastgate, Westbrook, Millhaven) and **2 industries** (Pine Sawmill produces lumber; Harbor Mill consumes it) on spaced land tiles. Connect them with track — there is no click-to-place station tool in MVP.

| Marker | Meaning |
| --- | --- |
| Red square | Station |
| Amber square | Producer industry |
| Purple square | Consumer industry |

### Train tools

| Input | Action |
| --- | --- |
| `T` | Buy a **transit** (passenger) train ($500) and enter place mode |
| `G` | Buy a **transport** (goods) train ($750) and enter place mode |
| Left click (place mode) | Place the bought train at a station that has track on or adjacent to its tile |
| `B` / `X` | Leave train place mode and return to track tools |

Blue rectangle = transit; amber = transport. Passenger fares (`$5`) and goods deliveries (`$20`) credit money; each train pays soft opex (`$0.10`/tick) and parks if broke (never deleted).

### Time controls

| Input | Action |
| --- | --- |
| Space | Toggle pause |
| `1` / `2` / `3` | Set sim speed (1x / 2x / 3x); unpauses |

### HUD & toolbar

- **Status strip** (top): money (`hi`), approximate net $/min, clickable speed segments (pause / 1× / 2× / 3×), active tool.
- **Toolbar** (bottom centre): Track (`B`), Demolish (`X`), Transit (`T`), Transport (`G`) — mouse-reachable; keyboard still works.
- **Complaint feed** (bottom-left): e.g. `Mara waited 11 min at Eastgate`.
- **Undo / redo:** `Ctrl/Cmd+Z` undoes the last track place / demolish / autofill run; `Ctrl/Cmd+Shift+Z` or `Ctrl/Cmd+Y` redoes. Construction only — sim time is not rewound.
- **SFX (native, `sfx` feature):** short procedural *clack* on successful track place; soft *thud* on rejection. Disable with `--no-default-features` for wasm-safe builds.

### Town & peeps

- Building density rings grow around stations when [`StationService`](rail_sim/src/stations/service.rs) scores are high, and shrink when service decays.
- Named peeps wait at stations; poor service makes wait accumulate faster and emits public complaints.

**Service-score contract** (trains / economy write, town / peeps read):

| API | Role |
| --- | --- |
| `StationService::record_arrival(id)` | Bump score on delivery |
| `StationService::set_waiting(id, n)` | Waiting passengers gently lower score |
| `StationService::tick_decay()` | Idle stations lose score over time |
| `StationServiceScore::score` | `0..=100` quality used as growth target |

## Test / check

```bash
cargo test --workspace
cargo check --workspace
```

## Web / WASM (share & iterate)

One command — builds `rail_town` for the browser and serves it (Bevy CLI):

```bash
./scripts/web --open
```

| Flag | Effect |
| --- | --- |
| *(none)* | Serve at http://127.0.0.1:4000 |
| `--open` | Open the default browser |
| `--release` | Release/web-optimized build (slower compile, better runtime) |
| `-- --port 8080` | Extra args to `bevy run web` (host/port/etc.) |

First run installs the Bevy CLI if missing (`cli-v0.1.0-alpha.2`) and ensures the `wasm32-unknown-unknown` target. Equivalent manual command:

```bash
bevy run -p rail_town --yes web --open
```

Typecheck only (no serve):

```bash
cargo check --target wasm32-unknown-unknown -p rail_town
```

Steam integration is a **future** optional feature flag (`steam` on `rail_town`). It is not enabled by default and must never be required for web builds.

## Shared types (for other slices)

- Player intents: `rail_sim::commands` (`PlaceTrack`, `Demolish`, `AutoFillTrack`, `BuyTrain`, `PlaceTrain`, …)
- Stable IDs: `rail_sim::ids`
- Track graph: `rail_sim::{TrackNetwork, TrackPiece}` — `at` / `piece` / `neighbor_ids` / `iter`; speed via `max_grade` + `curve`
- Track costs: `rail_sim::{TRACK_COST_CENTS, BRIDGE_COST_CENTS, MAX_BRIDGE_SPAN, GROUND_LAYER}`
- Pathfinding: `rail_sim::find_path` (BFS on track graph)
- Stations / industries / service: `rail_sim::{StationRegistry, IndustryRegistry, StationService, StationServiceScore, seed_stations_and_industries}`
- Trains: `rail_sim::{Train, TrainLocation, TrainCargo, TrainYard, JobBoard}`
- Town / peeps: `rail_sim::{TownDensity, ComplaintFeed, Peep, Mood}`
- Map / terrain: `rail_map::{MapGrid, generate_map, Tile, TILE_SIZE, tile_to_world, world_to_tile, Portal}`
- Neighbor backend: `rail_net::{NeighborBackend, NullNeighbor, NeighborService}`
