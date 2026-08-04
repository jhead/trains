# 02 — World & Terrain

**The premise:** the map is the level design. Rail Town ships no hand-authored campaigns, so every generated map has to do the job a designer would otherwise do — pose a routing question with more than one defensible answer, and pose a different one each time.

A map that is merely *varied* is not enough. Noise produces variety for free and it produces almost no interesting decisions. What follows is about generating maps that **argue with the player**.

---

## 1. What a good map does

A map has succeeded if, within the first two minutes, the player can look at it and think all three of these:

1. *"Obviously the line goes through there."*
2. *"…but that's a long way round. What if I cut over the ridge?"*
3. *"I can't afford that yet."*

That is the whole design in miniature: an obvious answer, a tempting expensive answer, and a reason to come back later. Terrain exists to manufacture that triple. Every generator decision below is judged on whether it produces it.

The failure state is a map where the straight line between two points is also the cheapest and the fastest. On such a map terrain is wallpaper, and the player is playing a connect-the-dots game with extra steps.

---

## 2. The shape of a map

### 2.1 Land is the subject; water is punctuation

The world is a **landscape with water in it**, not an ocean with land in it. Target composition for a standard map — revised by playtest (2026-08-02), which found even the original shares read as water-hemmed; the reference is Locomotion/RCT's broad open ground:

| Surface | Share | Role |
| --- | --- | --- |
| Buildable land | **85–92%** | where the game happens |
| Inland water — rivers, lakes | **4–8%** | the primary crossing decision |
| Sea | **0–4%, absent from most maps** | a coastal map's framing, when it rolls at all |
| Impassable rock | **4–8%** | hard walls that force real detours |

Open water carries no gameplay and no information. A map that is 40% flat blue has spent 40% of the player's screen on nothing. Sea is **optional**: there is no edge bias pulling borders underwater, and a map is landlocked unless it rolls a coast. When it does, sea belongs at the edges and in bays that intrude far enough to matter, earning its place by creating peninsulas. On the landlocked majority, **elevation carries the routing puzzle** — ridges with passes and valley corridors matter more, not less, than in the original table.

**Rivers are the best terrain feature in the game** and should be generated deliberately rather than falling out of a height threshold. A river is a continuous line the player must cross somewhere, and choosing *where* to cross is a real decision with cost, distance and future-network consequences. Every standard map gets at least one river system with two to four viable crossing points of differing width — a narrow expensive-detour crossing and a wide cheap-detour crossing is a complete design problem on its own.

### 2.2 Elevation with intent

Height should be organised into **legible landforms**, not a smooth noise field:

- **Valleys** — natural corridors. Cheap to build along, and they define where the obvious route goes.
- **Ridges** — continuous barriers with a small number of passes. A ridge with exactly two passes is a decision; a ridge with twenty is a texture.
- **Plateaus** — flat, buildable, expensive to *reach*. Excellent places to put a demand the player has to work for.
- **Basins** — bowls that are cheap inside and costly to enter or leave.

The generator should place these as *features* and then let noise decorate them, rather than hoping features emerge from octaves. A blurred multi-octave field with a radial bias produces blobs; blobs produce no passes, no corridors, and no decisions.

Playtest (2026-08-04) made this measurable — the failure mode was not too much
elevation but too *frequent* elevation ("constantly fighting terrain... just
mountains and up/down everywhere"). Binding targets for the default style on a
standard map: **at least ~70% of land tiles flat against all eight
neighbours**, hills and mountains gathered into **one to three connected
systems** with clear passes, and a straight 30-tile line between random land
points crossing **on average about two or fewer** band boundaries. (Under one
was the first draft of that number, and it is geometrically unreachable while
§2.1 demands 4–8% rock: bands step one per tile, so the rock alone owes the map
~290 tiles of contour, and by integral geometry a random 30-tile line meets
that much curve ~1.35 times before any other landform exists. The measured
floor is the rock share's, not the generator's.) Rugged may roughly double the
churn; Gentle trims it. Elevation is a feature the player walks up to, never a
texture they wade through.

### 2.3 Legibility is a generation requirement, not a rendering one

If the player cannot see a ridge, the ridge is not part of the puzzle — it is an invisible tax. Terrain must be readable at a glance:

- Elevation resolves into a small number of **discrete bands** with visible steps between them. Continuous height that renders as a soft gradient communicates nothing.
- Steep transitions draw as **cliff faces**, which is what makes a ridge look like a ridge.
- Slope direction is visible in the terrain art, so a valley reads as a valley from directly above.

A player should be able to trace the cheapest route across a map with their finger before laying a single piece of track. If they can't, the generator has failed regardless of how good the heightmap is numerically.

---

## 3. Terrain has to have teeth

Terrain only matters if it changes what the player does. That requires it to touch cost, speed and legality — all three, with real magnitudes.

### 3.1 Build cost varies enormously by terrain

Flat cost per tile is the single change that most completely disables the routing puzzle. The spread between the cheapest and most expensive buildable tile should be roughly **an order of magnitude**, so that going around is genuinely competitive with going through.

| Terrain | Relative cost | Feel |
| --- | --- | --- |
| Flat plains, along contour | **1×** | the default |
| Gentle slope, or cross-contour on plains | 1.5× | mild |
| Hills | 3× | noticeable |
| Steep hillside, cut-and-fill | 6× | you think about it |
| Bridge, per tile, scaling with span | 8–90× | a commitment, then a project |
| Tunnel, per tile | 15× | a project |
| Cliff face, mountain | — | refused |

The point of the spread is that **the cheapest route and the shortest route stop being the same line.** A detour of twelve flat tiles beating four hillside tiles is the moment terrain becomes gameplay.

### 3.2 Gradient is a hard limit, not a soft penalty

Track has a maximum gradient it may climb. Above it, placement is refused. This is what makes ridges into walls with passes rather than speed bumps, and it is what makes the player *read the terrain* instead of dragging a straight line and accepting a small time cost.

Below the limit, gradient still costs: steeper track is slower, and the slowdown must be large enough to feel — the difference between the flat route and the hilly route should be visible in throughput, not buried in rounding.

Climbing therefore becomes a routing craft: follow the contour to gain height gradually, or spend heavily on cut-and-fill to go direct. Both are legitimate. That is the design's "shortest, cheapest and fastest are three different routes" delivered concretely.

### 3.3 Curves cost speed

Tight curves slow trains meaningfully. A straight run is fast; a wiggly cheap route that hugs terrain is slow. This gives the contour-following strategy a real downside and stops it from dominating, which keeps the three-way tension genuinely three-way.

### 3.4 Water crossings are a decision, not a rule

A short bridge is routine. A long bridge is a major expense with a span limit, and beyond that limit the answer is to go around, or to find the narrows. Crossing points should be scarce enough to be worth scouting and plentiful enough that no map has exactly one answer.

Playtest (2026-08-04): *"if we're going to have rivers, we must be able to build bridges."* Binding: any river the generator draws must be bridgeable **somewhere the player is standing** — the span limit covers ordinary trunk widths and then some (spans up to eight: a cheap tier of 8/14/20× for one to three, then 30/42/56/72/90× for four to eight), with narrows remaining the cheap crossing worth scouting. The premium rungs answer the second playtest round's ask for genuinely large spans — a full eight-tile crossing runs 720× base, a project the railway saves toward, not an opening move. A watercourse the limit refuses everywhere is a wall wearing a river costume, and walls are rock's job.

---

## 4. Where the world puts things

Anchor placement is level design, and it deserves as much care as the terrain.

### 4.1 The opening beat

The player's first act must be **short, cheap, and immediately rewarding.** The opening configuration is:

- A **home town** near the map centre, already existing, with one station.
- A **second destination within about eight to twelve tiles**, across terrain that poses a small, legible question — a stream to bridge, a low rise to skirt.
- Everything else on the map is *visible but not yet worth reaching.*

The first line should be affordable within the first minute, connectable within the second, and paying out by the third. That first payout is the moment the game teaches its own loop, and nothing should stand between the player and it.

Scattering the starting anchors to the map's extremes produces the opposite: a long, expensive, unrewarded haul as the first thing the player ever does. Maximum separation is the correct objective for a *late* goal and the worst possible one for an opening.

### 4.2 Placement rules

- Anchors sit on **buildable, sensible ground** — a station on a beach or wedged against a cliff reads as a bug even when it is legal.
- Anchors respect a **minimum distance from the map edge**, so there is always room to build around them.
- Anchor spacing follows a **distribution**, not an extremum: a few close pairs, a few middle-distance, a few far. Uniform spacing and maximal spacing are both wrong; the interesting texture is in the mix.
- Industries are placed **where their resource makes sense** — a sawmill in forest, a quarry against rock, a harbour on a bay when the map has a coast (most do not; see §2.1). Placement that reads as logical makes the world feel authored.

### 4.3 The map grows

Crucially, **the world is not fully populated at generation time.** New settlements and industries appear over the course of a session, and they appear *outside* the currently served network. See [08 — Economy & Pressure](08-economy-and-pressure.md) §4 — this is the mechanism that keeps the loop turning, and terrain generation must reserve space for it: candidate sites identified up front, revealed over time.

---

## 5. Map setup options

Exposed at new-game time, because they change the kind of puzzle the map poses:

| Option | Values | Effect |
| --- | --- | --- |
| **Seed** | any, with a dice roll and a shareable code | reproducibility, and swapping maps with friends |
| **Size** | Small 48² · Standard 64² · Large 96² · Huge 128² | session length |
| **Terrain** | Gentle · Rolling · Rugged | how hard terrain argues |
| **Water** | Sparse · Balanced · Riverlands | how much of the puzzle is crossings |
| **Resources** | Clustered · Scattered | long hauls vs. dense networks |
| **Starting cash** | Lean · Standard · Generous | how much the early game paces |

A seed plus these options fully determines a map. Sharing a code should reproduce someone else's world exactly — that is most of a community feature for very little work, and it makes the generator's quality socially visible, which is a healthy pressure.

---

## 6. Map View

The world is played at a zoom where detail is visible and the whole map is not. **Map View** (`M`) is the strategic read: the entire world rendered schematically — terrain as silhouette and elevation band, water, track, stations, line colours, and live train positions.

It is not a zoomed-out camera. It is a second, purpose-built rendering that answers different questions: *where is my network thin, where is demand I'm not serving, where does the terrain want a line to go.* Clicking anywhere in it flies the world camera there.

The same view hosts the map-wide overlays described in [05 — Inspection & Overlays](05-inspection-and-overlays.md).

---

## 7. Acceptance bar

A generated map is good when:

1. A player can trace the cheapest route between two towns with their finger, before building anything.
2. At least three distinct route choices on the map have no obviously correct answer.
3. The first connection is affordable in the first minute and paying out by the third.
4. No two maps from different seeds pose the same primary question.
5. Removing the terrain entirely would visibly change how the player builds. If it wouldn't, the terrain isn't doing anything.
