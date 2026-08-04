# Rail Town — Design Briefs

Detailed direction for what Rail Town *is*. These briefs elaborate the vision in [`DESIGN.md`](../../DESIGN.md) into specific look, feel, interaction and gameplay.

They are written forward-looking. They describe the game, not the build — where a brief states a rule, that rule is the target.

Two exceptions to that, both earned:

- **Where a brief has been overtaken by a playtest, it says so and records the verdict**, including the argument it lost. A rejected design with its reasoning attached is worth more than a deleted one — 01 §8.1 and 14 §8 are the pattern.
- **Where a brief describes something that does not exist yet, it marks it**, because a target that reads as a description makes the docs untrustworthy the first time someone checks.

For where the build actually stands, see [`docs/BURNDOWN.md`](../BURNDOWN.md) — the live ledger, including the carried-debt table. [`docs/PROGRESS-AUDIT.md`](../PROGRESS-AUDIT.md) is an older snapshot and goes stale by design.

---

## The briefs

| # | Brief | Answers |
| --- | --- | --- |
| 01 | [Art Direction](01-art-direction.md) | What does it look like? Pixel contract, palette, track art, camera, composition. |
| 02 | [World & Terrain](02-world-and-terrain.md) | What does a map ask of the player? Landforms, cost, gradient, where things go. |
| 03 | [UI System](03-ui-system.md) | Panels, type, colour roles, layout, components, motion, input. |
| 04 | [Building & Tools](04-building-and-tools.md) | What does it feel like to lay a piece of track? |
| 05 | [Inspection & Overlays](05-inspection-and-overlays.md) | How does the player find out what's going on? |
| 06 | [Town & Peeps](06-town-and-peeps.md) | How does the town read as a place and its residents as people? |
| 07 | [Trains & Lines](07-trains-and-lines.md) | How does the player *operate* a railway? |
| 08 | [Economy & Pressure](08-economy-and-pressure.md) | What keeps the loop turning for an hour? |
| 09 | [Shell & Menus](09-shell-and-menus.md) | Title, new map, settings, saves, first run. |
| 10 | [Audio & Feel](10-audio-and-feel.md) | What does calm sound like? |
| 11 | [Roadmap](11-roadmap.md) | In what order, and how do we know each phase landed? |
| 12 | [Multiplayer: Neighbour Maps](12-multiplayer.md) | How do maps sit next to other players' maps? |
| 13 | [Shadows](13-shadows.md) | How does light fall on the world? Sun quantisation, what casts, what it costs. |
| 14 | [Music](14-music.md) | What does the score actually play? Mode, harmony, melody, form, synthesis. |
| 15 | Isometric track *(forthcoming)* | How does rail draw at 2:1? Cross-section, bank, junctions on the diamond. |

**01 and 03 are binding standards** — the pixel contract, the palette and the UI kit are constraints, not suggestions. 02 and 04–10 are feature design. 11 sequences them. 13, 14 and 15 are subordinate to 01, 10 and 01 respectively: where a brief and its parent disagree, the parent wins.

---

## Reading order

**If you have ten minutes** — read [11 — Roadmap](11-roadmap.md), then [04 — Building & Tools](04-building-and-tools.md). One tells you the shape of the work; the other is the verb the whole game rests on.

**If you're about to draw something** — [01 — Art Direction](01-art-direction.md), all of it, before opening an editor.

**If you're about to build interface** — [03 — UI System](03-ui-system.md), then [05 — Inspection & Overlays](05-inspection-and-overlays.md).

**If you want to understand why the game is the way it is** — [08 — Economy & Pressure](08-economy-and-pressure.md) is where the loop actually lives.

---

## The three things most easily lost

Everything in these briefs serves the vision, but three of its promises are structural — lose any one and the rest stops working:

1. **A new demand appears somewhere you can't yet reach.** Without this the game ends when the player connects what exists. → [08](08-economy-and-pressure.md) §4
2. **Track that isn't carrying enough costs more than it earns.** Without a running cost on the network, overextension cannot happen and pruning has no purpose. → [08](08-economy-and-pressure.md) §3
3. **Shortest, cheapest and fastest are three different routes.** Without terrain that varies cost by an order of magnitude and limits gradient outright, the routing puzzle does not exist. → [02](02-world-and-terrain.md) §3

---

## Related documents

| Document | Role |
| --- | --- |
| [`DESIGN.md`](../../DESIGN.md) | The vision. Source of truth for the fantasy. |
| [`docs/PROGRESS-AUDIT.md`](../PROGRESS-AUDIT.md) | Where the build stands against these briefs. |
| [`docs/RAILGEN_NOTES.md`](../RAILGEN_NOTES.md) | Summary of the *Permanent Way* track-art research. Brief 01 §5 hardens its findings into binding numbers. |
| [`docs/TECH_STACK.md`](../TECH_STACK.md) | Engine, crate split, platform targets. |
| [`docs/IMPLEMENTATION_PLAN.md`](../IMPLEMENTATION_PLAN.md) | The original MVP scope and multiplayer seams. Its acceptance checklist is superseded by the per-brief acceptance bars here. |
