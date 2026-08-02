# Rail Town

Calm pixel-art railway sandbox. Design brief: [`DESIGN.md`](./DESIGN.md).

## Docs

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
| Left click (Build) | Places the first anchor tile; second click auto-fills a **straight** run (orthogonal or 45° diagonal) to that tile |
| Shift + left click (Build) | Place a single tile (ignores autofill anchor) |
| Esc / right click | Clear pending autofill anchor |
| Left click (Demolish) | Refund and remove track under the cursor |

**Costs:** ground track `$10` (`TRACK_COST_CENTS = 1000`); bridge over water `$50` (`BRIDGE_COST_CENTS = 5000`). Demolish refunds the full amount paid for that tile. Bridges are limited to `MAX_BRIDGE_SPAN = 3` contiguous water tiles.

**Autofill:** two-click anchors (not drag). Non-straight second clicks are rejected by the sim.

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

### HUD

Top-left: **money** (dollars from cents), pause/speed, current tool, short help. Money text flashes when the balance changes. Bottom-left: **complaint feed** (e.g. `Mara waited 11 min at Eastgate`).

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
