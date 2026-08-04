# Progress Audit — 2026-08-01

**Purpose:** an honest snapshot of what exists against [`DESIGN.md`](../DESIGN.md), and a diagnosis of *why* the build reads as an unpolished prototype.

This is the **only** document in the repo that discusses current state. The design briefs in [`docs/design/`](design/README.md) are written forward-looking and deliberately say nothing about any particular implementation — they describe the game we are building. This document says where we stand relative to them, and it goes stale by design.

---

## 1. Verdict in one paragraph

The simulation is in decent shape. The game is not. `rail_sim` has a real spine — fixed-tick command buffer, track graph, BFS pathing, jobs, payouts, service scores, density rings, a complaint feed, all under test. `rail_town` — every part the player sees and touches — is a debug harness: forty-one hundred flat coloured rectangles, keyboard-only tools, no hover feedback, no inspection, no interpolation, no menus, no audio. On top of that, three load-bearing promises from the vision are **structurally absent**, not merely rough: nothing new ever appears to reach for, track has no running cost so overextension cannot bite, and shortest / cheapest / fastest resolve to the same route on every map. Those three are the loop. Fixing the pixels without fixing them produces a pretty thing with no reason to keep playing.

---

## 2. Where each pillar actually stands

Legend: **Built** — works and is defensible · **Thin** — exists but does not deliver the promise · **Absent** — not implemented · **Deferred** — correctly out of MVP scope.

### 2.1 The Loop

| Vision rung | Status | Detail |
| --- | --- | --- |
| **Seconds** — place a piece of track | Thin | Works, but the interaction is two discrete clicks with a hidden anchor, straight-only, no ghost, no cost preview, and silent rejection on failure. Nothing about it is "fast, reversible, immediately legible." |
| **A minute** — a train completes a run and pays out | Built | `spawn_demand_jobs → assign_jobs → advance_trains → resolve_deliveries` closes the loop and credits `Money`. |
| **Ten minutes** — a served area thickens; **a new demand appears somewhere you can't yet reach** | **Absent** | The first half exists (`TownDensity`). The second half does not exist at all. `seed_stations_and_industries` places 3 stations and 2 industries once, at startup, and the world never adds another. `spawn_demand_jobs` cycles A→B pairs over that fixed set forever. |
| **An hour** — the map is threaded, the town has a shape | Absent | With five fixed anchors on a 64×64 map there is nothing to thread. The session has no long arc. |

**This is the single most important finding.** The loop's third rung is what converts a toy into a game, and it is missing. See [08 — Economy & Pressure](design/08-economy-and-pressure.md) §4.

### 2.2 Track and Terrain

| Promise | Status | Detail |
| --- | --- | --- |
| Track laid piece by piece while the world runs | Built | `PlaceTrack` / `AutoFillTrack` apply on the tick boundary; sim keeps running. |
| Terrain generates the puzzle | Thin | `rail_map::gen` produces a genuinely nice 3-octave heightmap with continent bias and blur. The *sim* then ignores almost all of it. |
| **Gradient limits, curve radius, tunnels, bridge spans keep shortest / cheapest / fastest as three different routes** | **Thin — promise unmet** | Every land tile costs a flat `TRACK_COST_CENTS` ($10) regardless of terrain. Grade adds `+1 tick` per unit and curve `+curve/32` ticks against a `BASE_TICKS` of 4 (`movement.rs:75`). No grade *limit* — you may lay track up a cliff. No tunnels. So the cheapest route is the shortest route is very nearly the fastest route, always. The central routing puzzle does not exist. |
| Underground / elevated via a layer view | Deferred | `GROUND_LAYER` const and a `layer: u8` field are reserved. Correct for MVP. |
| Long straights auto-filled | Built | Straight and 45° only, which is the right MVP scope. |
| Demolition refunds in full | Built | Paid amount is stored per tile and refunded exactly. |

### 2.3 The Town and Its People

| Promise | Status | Detail |
| --- | --- | --- |
| Residents individually visible, named, **knowable** | Thin | `PEEPS_PER_STATION = 2` gives six peeps on the whole map. They are named from a 12-entry pool. They never move, never board a train, never arrive anywhere. They are 9-pixel dots pinned near a station tile, offset by `id % 3`, accumulating wait forever. Nothing can be clicked. "Knowable" is unmet. |
| Peeps have moods and voice them publicly | Thin | `Mood` and `ComplaintFeed` exist and the line text is good ("Mara waited 11 min at Eastgate"). Presentation is a static five-line text blob in a translucent box, redrawn every frame, with no entry animation, no timestamps, no click-to-locate, no history. |
| Growth is local and caused | Built (model) / Thin (readout) | `density_target_at` is a clean Chebyshev falloff off `StationServiceScore`. But it renders as **one small brown rectangle per tile** that grows from 4.8px to 22px. It reads as a debug heatmap, not as a town. |
| When service degrades, people leave and buildings empty | Thin | Density shrinks. Buildings pop out of existence below 0.08 with no decay stage. Peeps never leave — the population is a constant six. |

### 2.4 Trains

| Promise | Status | Detail |
| --- | --- | --- |
| Two types as **sidegrades with distinct constraint profiles** | Thin | `TrainKind::{Transit, Transport}` differ in exactly two ways: purchase price ($500 / $750) and which `JobKind` they will accept. Identical speed, identical capacity (one job), identical dwell (none), identical opex. There is no profile to trade off. |
| Both share one track system; mixing is a strategy and a mistake | Thin | They do share track, and `TileOccupancy` blocks tile-sharing — but with 3 stations and a handful of trains, congestion never occurs, so the "legitimate mistake" never lands. |

### 2.5 Money

| Promise | Status | Detail |
| --- | --- | --- |
| Income from throughput | Built | Distance-scaled fares with a boarding component, plus deliveries. |
| Costs from construction **and from operating expenses that scale with how much network you're running** | Built | Per-kind train opex (`TrainProfile::opex_cents_per_real_min`) and per-piece track maintenance (`MAINT_CENTS_PER_WEIGHT_PER_REAL_MIN`, bridges weighted heavier), both billed per real minute. Network size now costs to hold. |
| Overextension inverts it; the response is to prune and rebalance | Built | Idle track and stock bleed maintenance and opex every real minute; the second playtest's ledger showed exactly this inversion, and demolition refunds make pruning the real response. |
| Money paces expansion, never ends the game | Built | Soft-park on insufficient funds, never delete. Good, keep it. |

### 2.6 Pressure

| Promise | Status |
| --- | --- |
| Congestion is the standing puzzle | Thin — mechanism present (`TileOccupancy`), never bites, and is completely invisible to the player. |
| Events target the network | Deferred — `event_director.rs` is a 9-line stub. Correct for MVP. |
| Stagnation is the only failure | Absent — nothing in the game names, shows, or warns about stagnation. |

### 2.7 Modes & Neighbors

| Promise | Status |
| --- | --- |
| Sandbox | Built (implicitly — the game boots straight into it). |
| Goals mode | Deferred. |
| Neighbors | Deferred — `NullNeighbor` compiled in, portals generated closed on every border tile. The seam is real and correct. |

### 2.8 MVP acceptance checklist (from `IMPLEMENTATION_PLAN.md`)

- [x] New game → seeded map with land/water/elevation
- [x] Lay track, auto-fill a straight, demolish with full refund
- [x] Buy/place at least one transit and one transport train
- [x] Complete a passenger trip and a goods delivery; money changes
- [x] Served station area grows buildings over time
- [x] Complaint feed shows a peep wait complaint
- [x] Pause and speed controls work
- [x] `cargo run -p rail_town` works
- [x] WASM path documented (`./scripts/web`)
- [x] `NullNeighbor` compiled in; portals present on edges

**The MVP checklist passes. That is precisely the problem.** It was a checklist of mechanisms, not of experience — it never asked whether laying track *feels* good, whether the town *reads* as a town, or whether the player is ever given a reason to build the next line. The briefs that follow replace it with acceptance bars written in terms of what the player perceives.

---

## 2.9 What the first screen actually shows

Evidence: `docs/screenshots/mvp-native-sm.png` — the default map (seed 42, 64×64) as the player first sees it.

Measured across seeds 42, 1, 2, 7, 99 and 1234 at 64×64:

| Measurement | Result |
| --- | --- |
| Land fraction | 53–71% (seed 42: **57%**) |
| Anchors reachable from each other, allowing 3-tile bridges | **5 of 5, every seed** |
| Anchors within 2 tiles of a map edge | **~4 of 5, every seed** |
| Starting cash | $10,000 |

Two things to record, one of which corrects an assumption worth stating so nobody re-derives it:

**Connectivity is fine.** The archipelago read of the screenshot is misleading — the flat, uniform blue over-weights visually. Land is a comfortable majority and every seeded anchor is mutually reachable within the 3-tile bridge limit on every seed tested. There is no unplayable-map bug.

**Anchor placement is not fine.** `pick_spaced` is greedy farthest-point sampling over the whole land set, which by construction drives anchors to the extremities. In practice that means stations at `(4,2)`, `(61,2)`, `(61,61)`, `(2,2)` — literal corner tiles — with the three starting stations at maximum mutual separation. The player's opening move is therefore a ~40-tile haul from one corner of the map to another, across whatever terrain happens to intervene, before anything at all pays out. That is the worst possible first beat for a game whose stated seconds-level promise is "fast, reversible, immediately legible."

And visually, from the same screenshot:

- **The 1-texel tile gap dominates the frame.** The map does not read as terrain; it reads as graph paper with colour in the cells. This is the single loudest visual problem and it is one line of code.
- **Saturation is far above the stated "calm."** The greens and blues are near-primary. Nothing recedes, so nothing can stand out.
- **Water is one flat field.** No coastline, no depth reading, no shore detail — it is 43% of the frame carrying zero information.
- **Elevation is invisible.** Mountains render as soft grey blobs. The heightmap that the routing puzzle is supposed to be built on cannot be read at all.
- **The anchors — the only things the player is meant to act on — are ~17px squares** that disappear against the terrain. Three of the five are hard to find without knowing where to look.

---

## 3. Diagnosis — the seven reasons it feels like a prototype

Ranked by how much each one costs per unit of effort to fix.

### 3.1 Trains teleport

`sync_train_sprites` assigns `transform.translation.x = wx` from the train's *current tile*. `TrainLocation.progress` — the exact 0..n sub-tile position, already computed by the sim — is never read by the renderer. At `BASE_TICKS = 4` and 64Hz fixed tick, a train snaps a full 32px every ~62ms.

Nothing else on this list damages the impression of quality more, and nothing else is cheaper to fix. The data is already there. See [07 — Trains & Lines](design/07-trains-and-lines.md) §6.

### 3.2 There is no art, and no art *policy*

Every visual in the game is `Sprite::from_color(...)` — terrain, track, stations, industries, trains, buildings, peeps. Beyond "no sprites yet," three decisions actively fight the stated pixel-art look:

- **`spawn_map_tiles` draws tiles at `TILE_SIZE - 1.0`**, leaving a deliberate 1px gap "so the grid reads clearly." That gap is a debug affordance. It makes the world read as a spreadsheet.
- **`ZOOM_STEPS = [0.25, 0.5, 1.0, 1.5, 2.0, 4.0]`** includes 1.5, a non-integer scale that guarantees the exact pixel crawl `RAILGEN_NOTES.md` warns about, and 2.0/4.0 which sample *below* one screen pixel per texel.
- **The UI uses Bevy's default font at sizes 22 / 16 / 13 with a 4px `border_radius`.** That is a modern-web look bolted onto a pixel game. The two idioms cannot share a screen.

`docs/RAILGEN_NOTES.md` already contains the right policies (integer camera, bake-on-edit, world-anchored noise, min curve radius). None of them are enforced anywhere in the code. See [01 — Art Direction](design/01-art-direction.md).

### 3.3 The player is given no feedback whatsoever

Trace a single failed action. The player is in Build mode, clicks a water tile 6 tiles from shore. `validate_tile_empty` returns `BridgeTooLong { span: 6 }`. That error is **discarded**. No sound, no flash, no message, no cursor change. From the player's chair, the game ignored the click.

The same is true of: insufficient funds, occupied tile, out of bounds, non-straight autofill, placing a train where there's no track, and buying a train you can't afford. Every failure mode in the game is silent. And there is no *positive* feedback either — no hover highlight, no ghost of what you're about to build, no running cost while you drag, no confirmation that a tile landed.

### 3.4 Nothing can be clicked

There is no selection, no inspector, no tooltip, no context menu. The player cannot ask *which station is that*, *why is it doing badly*, *where is that train going*, *who is Mara*, *how much am I spending*. The complaint feed is the entire diagnostic surface of the game, and it is five lines of static text.

For a design whose stated emotional hook is that residents are "individually visible, named, and knowable," having no way to look at one is a direct contradiction. See [05 — Inspection & Overlays](design/05-inspection-and-overlays.md).

### 3.5 Interaction is a keyboard cheat-sheet

`B`, `X`, `T`, `G`, `Space`, `1`/`2`/`3`, Shift-click, right-click-to-cancel, two-click anchors. There is no toolbar, no button, no icon, no discoverable affordance of any kind. The README is the tutorial. A player who launches the binary without reading the repo cannot find out that trains exist.

The interaction *model* is also wrong for the verb. Laying track is a dragging motion in every game in this lineage, and in the hand. Two clicks with an invisible anchor is neither.

### 3.6 The world is inert

64×64 = 4096 static tile sprites, spawned once, never touched. No water animation, no smoke, no day/night, no weather, no birds, no level crossings, no ambient motion, and — because `bevy` is pulled in with `default-features = false, features = ["2d"]` — **no audio subsystem compiled in at all**. "Calm" is not the same as "still." A calm game is full of slow, quiet motion; a still one looks broken.

### 3.7 There is no game shell

No title screen, no new-game flow, no seed entry, no pause menu, no settings, no save, no load, no quit. The binary opens directly onto seed 42 and can only be closed by killing the window. Nothing frames the experience as a product.

---

## 4. What is genuinely good and must not be thrown away

The correction is a rewrite of `rail_town`, not of `rail_sim`. Preserve:

1. **The command architecture.** `CommandBuffer` → `apply_commands` → `PendingWorldCommand` on the tick boundary is exactly right, and it is what makes undo, replay, and eventual multiplayer cheap. Every new interaction in these briefs emits commands and nothing else.
2. **The fixed-tick / presentation split.** `SimSet::{ApplyCommands, Advance}` with `run_if(sim_is_running)`, and commands applying *even while paused* so you can build during pause. Keep.
3. **Stable IDs.** `TrackId` / `StationId` / `TrainId` / `PeepId` as `u64` newtypes. Every panel, selection, and save format in these briefs keys off them.
4. **`StationService` as a published contract.** A single `0..=100` score that economy writes and town/peeps read is a clean seam, and the overlays in brief 04 are just new readers of it.
5. **Terrain generation.** `rail_map::gen` is better than what the game does with it. The fix is downstream.
6. **Soft-fail economics.** Park trains, never delete; clamp, never game-over. This is load-bearing for "money never ends the game."
7. **The MP seams.** Closed portals on every border tile, `NullNeighbor`, serializable snapshots. Cheap to keep, expensive to retrofit.
8. **Test discipline.** Determinism tests on gen, threshold tests on growth and complaints. Extend this to the new systems.

---

## 5. Known code-level defects found during the audit

Not design issues — file these as work items.

| Location | Issue |
| --- | --- |
| `rail_sim/src/trains/movement.rs:34,71` | `moves` vec is built and then discarded via `let _ = moves;`. Dead. |
| `rail_town/src/trains/visuals.rs:30` | `sprites.iter_mut().find(...)` inside the per-train loop — O(trains × sprites) every frame. |
| `rail_town/src/track/visuals.rs:52` | Full sprite-table scan per removal message. |
| `rail_town/src/town/buildings.rs` | Allocates a `HashSet` and iterates all density cells every frame; no change detection. |
| `rail_town/src/ui/hud.rs:101` | Rebuilds every `Text` component each frame regardless of change. |
| `rail_sim/src/stations/service.rs:52` | `set_waiting` applies `saturating_sub` to the score every call, so score decays as a side effect of a read-shaped update. Waiting pressure should be computed into the score, not repeatedly subtracted. |
| `rail_sim/src/clock.rs:41` | `speed_multiplier: u8` is free-form but `SimSpeed` only models 1× and 3×, while the HUD advertises `1/2/3`. Three sources of truth for speed. |
| `rail_sim/src/stations/seed.rs:55` | `let _ = i;` — leftover. |
| `rail_town/src/map/spawn.rs` | 4096 individually-spawned sprites with no chunking; will not survive a larger map. |

---

## 6. Where the correction is written down

The design briefs in [`docs/design/`](design/README.md) describe the game we are building. They are the target this audit measures against.

The three findings above that are **structural rather than cosmetic** map to specific briefs:

| Finding | Where it is addressed |
| --- | --- |
| Nothing new ever appears to reach for | [08 — Economy & Pressure](design/08-economy-and-pressure.md) §4 |
| Network size is free to hold, so overextension cannot bite | [08 — Economy & Pressure](design/08-economy-and-pressure.md) §3 |
| Shortest, cheapest and fastest are the same route | [02 — World & Terrain](design/02-world-and-terrain.md) §3 |

The prototype-feel diagnoses in §3 are addressed by [01 — Art Direction](design/01-art-direction.md), [03 — UI System](design/03-ui-system.md), [04 — Building & Tools](design/04-building-and-tools.md) and [05 — Inspection & Overlays](design/05-inspection-and-overlays.md).

Sequencing is in [11 — Roadmap](design/11-roadmap.md). The MVP acceptance checklist in §2.8 is superseded by the per-brief acceptance bars.
