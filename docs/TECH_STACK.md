# Tech Stack — *Rail Town*

## Goals that drove the choice

1. **ECS-first simulation** — track, trains, peeps, and districts are data; systems own the loop.
2. **Steam-ready desktop** — native Windows / macOS / Linux builds with a clear Steamworks path later.
3. **Shareable web build** — same codebase → WASM so friends can play from a link (itch.io, GitHub Pages, etc.).
4. **Multiplayer later without a rewrite** — sim is deterministic, serializable, and isolated from presentation; edge portals are first-class stubs.

## Decision

| Layer | Choice | Why |
| --- | --- | --- |
| Language | **Rust** | One binary for desktop + WASM; strong typing for sim invariants |
| Engine | **Bevy 0.18** | Native ECS, 2D + UI, mature WASM (`web` feature), `bevy-steamworks` aligned to 0.18 |
| Rendering | **Bevy 2D** (pixel-art orthographic) | Calm top-down / slight-iso look; cheap to iterate |
| Sim clock | **Fixed timestep** (`FixedUpdate`) | Determinism + future lockstep / replay |
| Persistence | **serde + bincode** (RON for debug maps) | Save/load + future neighbor map exchange |
| RNG | **`rand` + seedable `StdRng`** | Reproducible terrain / events |
| Audio (later) | Bevy audio | Not MVP |
| Steam (later) | `bevy-steamworks` | Optional feature flag; never required for web |
| Hosting web | Static WASM + JS glue (`wasm-bindgen`, trunk or `bevy run web`) | Friend-share loop |

**Pinned for MVP:** Bevy **0.18.x** (not 0.19 yet) so Steamworks and ecosystem crates stay compatible when we flip the Steam feature on.

## Workspace layout

```
rail_town/          # binary — Bevy app, input, rendering, UI
rail_sim/           # library — pure ECS-friendly sim types & systems (no windowing)
rail_map/           # library — terrain gen, tiles, layers, edge portal stubs
rail_net/           # library — *stub* neighbor exchange API (no networking yet)
```

Rules:

- `rail_sim` and `rail_map` must not depend on `bevy_render` / windowing. They may use `bevy_ecs` + `bevy_app` schedules, or stay engine-agnostic with plain structs + Bevy wrappers in `rail_town`. Prefer **Bevy ECS in `rail_sim`** so we don't dual-model entities.
- `rail_net` exposes traits/types for edge handoff (`NeighborLink`, `CargoManifest`, async inbox). MVP implements an **in-process null neighbor** that never blocks single-player.
- Player intent flows as **commands** (`PlaceTrack`, `Demolish`, `BuyTrain`, …). Later, the same commands can be networked or replayed.

## Platforms

| Target | How | MVP |
| --- | --- | --- |
| Native (dev) | `cargo run -p rail_town` | Yes |
| Web | `trunk serve` or Bevy web runner → WASM | Scaffold + CI note; must compile |
| Steam | Native release + `bevy-steamworks` feature | Stub feature only |

## Explicit non-choices

- **Unity / Godot** — viable, but weaker “share a WASM build from the same ECS sim” story and more rewrite risk for our MP boundary.
- **TypeScript + bitECS** — great for web, awkward Steam path and weaker long-session sim tooling.
- **3D mesh world** — out of scope; pixel-art 2D matches the brief.

## Multiplayer readiness (no MP in MVP)

See `IMPLEMENTATION_PLAN.md` § Multiplayer seams. Summary: fixed tick, command buffer, serializable map chunks, edge portals as entities, `rail_net` null backend.
