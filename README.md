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

- Player intents: `rail_sim::commands` (`PlaceTrack`, `Demolish`, `AutoFillTrack`, …)
- Stable IDs: `rail_sim::ids`
- Track graph: `rail_sim::{TrackNetwork, TrackPiece}` — `at` / `piece` / `neighbor_ids` / `iter`; speed later via `max_grade` + `curve`
- Track costs: `rail_sim::{TRACK_COST_CENTS, BRIDGE_COST_CENTS, MAX_BRIDGE_SPAN, GROUND_LAYER}`
- Map / terrain: `rail_map::{MapGrid, generate_map, Tile, TILE_SIZE, tile_to_world, world_to_tile, Portal}`
- Neighbor backend: `rail_net::{NeighborBackend, NullNeighbor, NeighborService}`
