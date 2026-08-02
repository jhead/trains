# 11 — Course Correction Roadmap

**How the preceding briefs get sequenced.** Every phase is defined by what a player can perceive at the end of it, not by what has been written. A phase is done when a stranger sitting down at the build agrees it is done.

---

## The sequencing principle

**Feel first, then legibility, then loop, then life, then frame.**

This order is deliberate and it is not the order of technical dependency:

- **Feel** comes first because it is the cheapest large improvement available and because it changes how everything subsequently built is judged — including by the people building it. Working on a game that feels good is a different activity from working on one that doesn't.
- **Legibility** comes second because it is what lets anyone — player or developer — evaluate whether the deeper systems are working. Building the loop before you can see the loop is building blind.
- **The loop** comes third because it is the most valuable and the most expensive, and it benefits from being built on a foundation that can already show its results.
- **Life** and **frame** come last because they are polish and packaging, and both scale with everything before them.

The temptation will be to do the loop first because it is the most important. Resist it. A correct loop that is illegible and feels bad is indistinguishable from a broken one.

---

## Phase A — Feel

*"This looks and feels like a game."*

The smallest set of changes that removes the prototype impression entirely.

- Continuous, interpolated train movement with correct facing and articulated cars.
- Drag-to-build with a live ghost, running cost, and per-tile validity.
- Loud, specific failure feedback in every channel.
- Undo and redo.
- Right-drag demolish from within the build tool.
- The pixel contract enforced — integer zoom only, cursor-anchored, contiguous terrain, no runtime sprite rotation.
- The palette adopted across world and UI.
- The pixel UI kit: bitmap font, square panels, the spacing grid.
- A toolbar with icons, so every verb is discoverable without documentation.
- The status strip: money, rate, date, speed.
- Sound for laying track, and for failing to.

**Done when:** a stranger lays a line of track and smiles. Nothing teleports, nothing is silent, nothing looks like a debug view.

---

## Phase B — Legibility

*"I can see what's happening and why."*

Everything becomes inspectable.

- Selection, and the Inspector for every object type.
- The Station panel with its plain-language cause line.
- The Peep card — name, portrait, history, mood with a reason.
- Town Talk as a living ticker, click-to-fly, deduplicated, with praise as well as complaint.
- Overlays: service, coverage, congestion, gradient, cost, density, profit.
- Map View.
- The ledger, with income and expense by category and a trend.
- Alerts, actionable and non-intrusive.

**Done when:** a player who asks "why is this station bad?" gets a sentence, and a complaint in the ticker is one click from the person who made it.

---

## Phase C — The Loop

*"I always know what I want to build next, and I can't quite afford it."*

The most valuable phase, and the one the previous two exist to support.

- Player-authored **lines**: draw them, name them, colour them, assign trains, set frequency, read the strip diagram.
- **Stations as buildable track**, in tiers, with a live catchment preview.
- **Track maintenance cost**, making overextension real and pruning meaningful.
- **Terrain-differentiated construction cost** with an order-of-magnitude spread.
- **Gradient as a hard limit**, curves costing speed — shortest, cheapest and fastest become three routes.
- **Transit and transport given genuinely different constraint profiles.**
- **New demand appearing outside the served network**, on a felt rhythm, at increasing distance.
- Growth responding to **accessibility rather than proximity**, capped by connectivity.
- Congestion made visible, diagnosable, and solvable with passing loops and double track.

**Done when:** a two-hour session has an arc — the network has a shape that records the player's decisions, and the terrain that was prohibitive in the first minute is worth conquering by the fiftieth.

---

## Phase D — Life

*"This is a place."*

The world stops being a diagram.

- Real terrain art: authored material transitions, cliffs that make elevation readable, water with structure.
- The track sprite bank and authored junctions, at sixteen graph directions.
- Buildings in tiers, on lots, with district character.
- Construction and decay as watchable events.
- Peeps that walk, travel, board, alight, and move away by name.
- Day and night, with windows that light.
- Ambient motion everywhere — water, smoke, foam, crossings.
- The full ambience bed, positional and layered, with the town's own sound.
- Music.

**Done when:** a player leaves the game running to watch it, and can tell with their eyes closed whether the town near the camera is thriving.

---

## Phase E — Frame

*"This is a product."*

- Title screen with the live world behind it.
- New Map with a live preview and honest readouts.
- Pause menu, settings across four tabs, full rebinding.
- Save, load, autosave, without a hitch.
- First-run design: the teaching map, the nudge, the celebrated first payout.
- Accessibility throughout — reduced motion, colour-blind safety, text scale.
- Goals mode as a lens on the sandbox.

**Done when:** launch to laying track takes under fifteen seconds, and nothing in the shell looks like it came from a different game than the world does.

---

## What stays deferred

Deliberately out of scope until the phases above are complete, and none of them are blocked by that:

- **Underground and elevated construction.** The interaction is designed in [04 — Building & Tools](04-building-and-tools.md) §7 so it slots in without re-teaching, but tunnels are a Phase C+ luxury.
- **Neighbour maps and asynchronous multiplayer.** The edge-portal and command seams exist precisely so this stays cheap to add later; nothing in these briefs should compromise them.
- **Signals in full.** Passing loops and double track cover congestion adequately for a first pass; block signalling is the depth extension after.
- **Seasons and weather as mechanics**, beyond the visual and audio layer.

---

## Standing rules for the correction

Four things that apply to every phase and are worth stating once:

1. **The pixel contract and the palette are binding.** Briefs 01 and 03 are not aspirations. Anything that violates them is wrong regardless of how well it works.
2. **Placeholders use the real palette and the real dimensions.** A placeholder that is the wrong size or the wrong colour is not a placeholder; it is a second thing to redo.
3. **Every new player-facing system ships with its feedback.** A mechanic without a readout is not finished, and "we'll add the UI later" is how the current gap was created.
4. **The simulation stays authoritative and fixed-step; presentation stays free to be smooth.** Interpolation, ghosts, tweens and previews are presentation concerns and must never leak into the model.

---

## The one-sentence test

At the end of every phase, ask:

> *Would a person who likes this kind of game keep playing for an hour without being asked to?*

Phase A makes them willing to start. Phase B makes them able to learn. Phase C gives them a reason to continue. Phases D and E make them want to come back.
