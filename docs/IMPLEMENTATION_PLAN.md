# Implementation Plan — *Rail Town* MVP

Source of truth for fantasy: [`DESIGN.md`](../DESIGN.md).  
Stack: [`TECH_STACK.md`](./TECH_STACK.md).

## MVP scope

Ship a **playable sandbox** that proves the core loop:

> place track → run trains → earn money → town thickens along service → need to reach further

### In

| Pillar | MVP bar |
| --- | --- |
| Terrain | Seeded heightmap; land / water; simple elevation bands that affect track cost/speed |
| Track | Place / demolish tile-by-tile while sim runs; straight auto-fill between two anchors; full refund on demolish |
| Constraints (light) | No track on deep water without a bridge span (limited length); steep grades slow trains; sharp corners slow trains |
| Layers | **Ground layer only** in UI; `Layer` enum + elevation bit reserved so tunnels/elevated can plug in later |
| Trains | Transit (passengers) + Transport (goods); shared track; simple path follow + station stops |
| Demand | A few industries + residential clusters; passengers want station A→B; goods want industry→industry |
| Money | Fare/delivery income; track build cost; per-train operating cost; cannot go permanently broke (clamp / soft stop building) |
| Town | Local growth near well-served stations; shrinkage when service drops; buildings as sprites/tiles |
| Peeps | Named agents with mood; public complaint feed (“waited N min at Eastgate”) |
| Modes | Sandbox only |
| UX | Camera pan/zoom; build / demolish / train tools; money + complaint HUD; pause / 1x / 3x |
| Platforms | Native run; WASM compile path documented |

### Out (architect for, do not build)

- Neighbor maps / async MP (`rail_net` stub only)
- Goals mode + deadlines
- Network events (landslide, festival, flood) — leave an `EventDirector` stub
- Full underground / elevated editing UI
- Complex signaling / junctions AI beyond simple reservation or wait-at-node
- Steam store upload, achievements, cloud saves
- Polished pixel art (placeholders OK: colored rects / simple tiles)

## Multiplayer seams (do not rewrite later)

1. **Fixed tick sim** — all economy, movement, growth in `FixedUpdate`; render interpolates.
2. **Commands** — every player action becomes a struct in a buffer applied on the tick boundary.
3. **Stable IDs** — `EntityId` / `TrackId` / `StationId` as `u64` (or Bevy entity + remap table on load), never raw pointers in save format.
4. **Map edges** — tiles on map border can hold a `Portal` component (`portal_id`, `facing`). MVP: portals exist but are closed; trains turn back or despawn cargo at edge per design of single-player.
5. **`rail_net`** — trait `NeighborBackend { poll_inbox; send_train; }` with `NullNeighbor`.
6. **Serialization** — `WorldSnapshot` for save/load; same blob shape will later be a neighbor chunk.

## Vertical slices (build order)

Agents may parallelize by crate / slice ownership below. Integrate on `main` frequently.

### Slice 0 — Skeleton
- Cargo workspace, Bevy app window, fixed timestep plugin shell, placeholder camera
- CI-ish: `cargo check` / `cargo test` for all crates
- README with run instructions

### Slice 1 — Map & terrain
- Grid resource (`MapGrid`); height + water; seeded gen
- Draw tiles; camera pan/zoom
- Portal stubs on edges

### Slice 2 — Track & tools
- Place / demolish commands; auto-fill straight
- Cost + full refund; money resource
- Bridge over water (short span limit)

### Slice 3 — Trains & pathing
- Graph from track tiles; pathfind station→station
- Transit + transport entities; speed from grade/curve
- Congestion lite: one train per tile or wait behind

### Slice 4 — Economy & demand
- Stations, industries, cargo / passenger jobs
- Payout on delivery; operating costs per tick
- Soft fail when broke (block paid actions)

### Slice 5 — Town & peeps
- Growth/shrink rings around station service scores
- Spawn named peeps; wait-time tracking; complaint feed UI

### Slice 6 — Juice & share
- Time controls, basic audio-less juice (tweens / flashes)
- WASM build instructions; optional GitHub Pages note
- Save/load snapshot (optional stretch)

## Suggested agent parallelization

| Agent | Owns | Depends on |
| --- | --- | --- |
| A — Skeleton | workspace, `rail_town` boot, schedules, README | — |
| B — Map | `rail_map`, terrain gen, portal stubs, tile draw hooks | A (workspace) |
| C — Track | track components, place/demolish/autofill, money costs | A, map types |
| D — Trains | graph, pathing, transit/transport movement | track graph |
| E — Economy | jobs, payouts, opex | trains + stations |
| F — Town/UI | growth, peeps, complaint HUD, tools HUD | stations + money |

**Integration rule:** prefer merging types into shared modules early (`components.rs`, `commands.rs`). Do not invent parallel entity models.

## Acceptance checklist (MVP done when)

- [ ] New game → seeded map with land/water/elevation
- [ ] Lay track, auto-fill a straight, demolish with full refund
- [ ] Buy/place at least one transit and one transport train
- [ ] Complete a passenger trip and a goods delivery; money changes
- [ ] Served station area grows buildings over time; neglected area stagnates or shrinks
- [ ] Complaint feed shows at least one peep wait complaint
- [ ] Pause and speed controls work
- [ ] `cargo run -p rail_town` works on desktop
- [ ] WASM target documented; `cargo check --target wasm32-unknown-unknown -p rail_town` passes (or equivalent)
- [ ] `rail_net::NullNeighbor` compiled in; portals present on edges

## Decision freedom

Agents choose concrete algorithms (A* vs BFS, growth formulas, tile size, placeholder art) as long as they honor this plan and the multiplayer seams. Prefer boring, readable code over clever frameworks.
