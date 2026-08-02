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

## Test / check

```bash
cargo test --workspace
cargo check --workspace
```

## WASM

Ensure the `wasm32-unknown-unknown` target is installed, then:

```bash
rustup target add wasm32-unknown-unknown
cargo check --target wasm32-unknown-unknown -p rail_town
```

Steam integration is a **future** optional feature flag (`steam` on `rail_town`). It is not enabled by default and must never be required for web builds.

## Shared types (for other slices)

- Player intents: `rail_sim::commands` (`PlaceTrack`, `Demolish`, …)
- Stable IDs: `rail_sim::ids`
- Neighbor backend: `rail_net::{NeighborBackend, NullNeighbor, NeighborService}`
