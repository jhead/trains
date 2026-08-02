# Burn-down — design briefs

Tracks work against [`docs/design/11-roadmap.md`](design/11-roadmap.md). Update as phases land.

## Phase A — Feel

| Item | Status |
| --- | --- |
| Contiguous terrain + palette + integer zoom 1/2/3 | done |
| Interpolated trains (read `progress`, facing) | done |
| Drag-build ghost, cost HUD, loud failures, right-drag demolish | done |
| Undo / redo via command history | done |
| Pixel UI kit + toolbar + status strip | done |
| Track lay / fail sounds | done |

## Phase B — Legibility

| Item | Status |
| --- | --- |
| Selection + Inspector | done |
| Station panel / Peep card / Town Talk | done |
| Overlays + Map View | done |
| Ledger + alerts | done |

## Phase C — The Loop

| Item | Status |
| --- | --- |
| Lines (player-authored) | done |
| Buildable stations + tiers | done |
| Track maintenance + terrain cost + grade limits | done |
| Distinct transit/transport profiles | done |
| New demand outside network | done |
| Congestion: reroute, yield, passing loops, double track | done |

Passing loops and double track needed **no track-graph change** — the tile graph
is 8-connected and auto-links neighbours, so a parallel row of tiles already is
a loop. The gap was that movement followed a fixed path and waited.

## Phase D — Life

| Item | Status |
| --- | --- |
| Terrain art: autotiling, cliffs, chunked composite | done |
| Building tiers on lots, district character, construction/decay as events | done |
| Walking peeps with journeys, households, routines, memory | done |
| Day/night, lit windows, ambient motion | done |
| Full audio: ambience beds, trains, music, mixing | done |

## Phase E — Frame

| Item | Status |
| --- | --- |
| Title, new map, pause, settings | done |
| Save / load | done |
| First-run teaching | done |
| Goals mode | done |

## MP — Neighbour maps

Design: [`docs/design/12-multiplayer.md`](design/12-multiplayer.md). MP-1 requires no networking.

| Item | Status |
| --- | --- |
| MP-1 — portals as construction, border yard, transit, manifests, echo neighbours | done |
| MP-2 — friend codes, blob exchange, cache + reconciliation | pending |
| MP-3 — community pool, relationship maturity | pending |

---

## Playtest fixes (2026-08-02)

| Report | Cause and fix |
| --- | --- |
| Money drains out in ~2 min, then the game is unwinnable | **Bug, not tuning.** Upkeep was charged every tick against a 64 Hz timestep, so every running cost was x64 — 3 idle stations cost ~$345/min. And going broke parked every train, i.e. removed the only income, making bankruptcy terminal. Rates are now per sim-minute via a cent-tick accumulator; trains never park for money; the balance floors at zero and uncollected upkeep is simply not collected. Fares raised to match. |
| Tofu boxes in UI text | The shipped font has no glyph for `.`, `-`, `x`, mood/pause symbols. All string literals are ASCII-only now. **Standing constraint** — brief 03 wants a bitmap pixel font, whose charset will be small. |
| Cannot leave build mode | There was no non-building tool at all. Added **Look** (`V`) as the default, with a toolbar slot, and `Esc` now unwinds one layer at a time before disarming. |
| Way too much water | Composition retargeted to Locomotion/RCT: land 85-92%, sea 0-4% and absent from many maps. |
| Buildings carpet the map | Density falloff was linear to the catchment edge, so every town was the same soft blob. Now a steep power curve with a hard cutoff, and the rural prop carpet (~52% of tiles) is clustered farmsteads (~4%). |
| Buildings too small, no hover | Atlas cell 16x32 -> 24x48, silhouettes redrawn to be separable at a glance. Hover highlight + tooltip added, reusing the click hit test. |
| Peeps walk through water | A* over walkable ground, 4-neighbour so a diagonal cannot cut a river corner. Bridges are walkable; cliffs and mountains are not. |

## Carried debt

Things that work but are known-shallow, with the brief that wants more.

| Item | Note |
| --- | --- |
| **Rail sprite bank must match the realised rose** | Brief 01 §5.2 assumed an even 32-entry bank at 11.25 deg steps. The graph's actual tangents are 0 / 26.57 / 45 / 63.43 deg, which are **not** multiples of 11.25 — the nearest entry is 4.07 deg away, above the `spritebank` plate's 2.81 deg target. Bake the bank at the sixteen realised bearings plus interpolants, never at even steps, or the facet-popping the plate names comes straight back. |
| ~~New Map options re-roll~~ (fixed: real `MapGenOptions`) | `generate_map` takes only `(w, h, seed)`. Terrain/Water/Resources fold into the seed, so every option genuinely changes the world and the readouts are measured from what came out — but "Rugged" does not yet mean rugged. Brief 02 §2.2 wants generated landforms (valleys, ridges with a few passes, plateaus, basins) rather than blobs from octaves. |
| **Save has three hand-written mirrors** | `ServiceScoreSnapshot`/`StationServiceScore`, `ClockSnapshot`/`SimClock`, `BudgetSnapshot`/`PeepBudget` restore with `..Default::default()`. A new field on the source type **compiles and is silently not saved**. Bump `SCHEMA_VERSION` on any blob-shape change. |
| **Gameplay plugins aren't state-gated** | They register systems without a named `SystemSet`, so `main.rs` cannot retro-fit `run_if(in_state(Playing))`. The shell gates by pointer-blocking and input suppression instead, which works. Real gating is one `.in_set(...)` edit per plugin file. |
| **Controls rebinding is stored, not consumed** | Gameplay systems read `KeyCode` literals. The Settings tab says so rather than implying otherwise. There is a real `L` conflict between Line tool and Ledger. |
| **Map View downsamples world sprites** | Now that terrain is textured, nearest-sampling a 4-texel/tile view will alias. Briefs 01 §2.1 and 02 §6 want a purpose-built schematic render, not a downscaled world. |
| **Audio has no settings UI** | `AudioMix` exposes master/music/ambience/effects/UI as public fields ready for sliders; nothing writes them yet. |
| **Panel UI cues aren't wired** | Clips exist behind a public `audio::UiCue` message; each panel owner adds one write. Map View, buttons, alerts and money fire today. |
| **Station demolish refuses when a line calls there** | Brief 04 §4 wants a confirm dialog naming the consequence. Needs `LineRegistry::remove_stop`. |
| **Goods platforms against industries** | Brief 04 §6 last line. `Industry` has no tier. |
| **Peep resources not reset on New Map** | `PeepSpawnState`, `HouseholdRegistry`, `DistrictFlow`, `PeepBudget`, `PeepFocus` aren't publicly re-exported, so the shell can't clear them. Low risk — peeps respawn per station. |
| **Two world-hash helpers** | `atmosphere/hash.rs` and `map/terrain/material.rs` both define one. Dedupe; the terrain one adds rather than XORs, which is better against diagonal moiré. |
