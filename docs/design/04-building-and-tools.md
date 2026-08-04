# 04 — Building & Tools

**This is the brief that matters most.** Laying track is the game's only real verb and the player performs it thousands of times per session. Everything else can be adequate; this has to be *delicious*.

The vision's seconds-level promise is "fast, reversible, immediately legible." All three words are requirements.

---

## 1. The feel we are chasing

Laying a piece of track should feel like **drawing with a confident pen**. You put the nib down, you pull, the line follows your hand and snaps to something sensible, and when you lift, it's there — with a small satisfying sound and a number that ticks down.

The three sensations, in order of importance:

1. **The line follows your hand.** Zero perceptible latency between cursor and ghost. The proposed route updates every frame.
2. **The game is thinking with you.** The route it proposes is the one a sensible engineer would draw. When you drag across a river it finds the narrows; when you drag up a hillside it suggests the contour.
3. **Commitment is a moment.** Release is a distinct, felt event — sound, a small settle animation, a cost deducted. Building is free to *explore* and a decision to *commit*.

Two-click placement with an invisible anchor is the opposite of all three. Dragging is how this verb works in every game in this lineage and in the hand.

---

## 2. The build interaction

### 2.1 The loop

```
  hover            press           drag              release
    │                │               │                  │
    ▼                ▼               ▼                  ▼
 tile highlight   anchor set    live ghost route    commit + sound
 + terrain chip   + cost HUD    + running cost      + settle anim
                                + validity per tile
```

**Hover.** The tile under the cursor takes a 1-texel `hi` outline. A small chip near the cursor shows terrain and elevation — `Hills · 7m` — and, if track can't go there, why not. This is on at all times in Build mode, and it is how the player learns to read terrain without a tutorial.

**Press.** The anchor is set and marked with a filled `hi` corner bracket. The cost HUD appears near the cursor.

**Drag.** The ghost route from anchor to cursor redraws every frame. It is the *actual* track art at 55% opacity tinted `hi`, not an abstract line — the player sees what they will get, including how the curve will resolve. Per-tile validity is drawn on the ghost itself.

**Release.** The route commits. Each tile settles with a 2-frame drop, staggered ~20 ms along the route so a long run reads as *laying* rather than appearing. One `clack` per tile, pitch-varied, rate-limited to about eight per second so a fifty-tile run is a satisfying run of sound rather than a machine gun.

**Continuous building.** After release, the endpoint becomes the new anchor and the player can immediately drag onward. Building a long route is one fluid press-drag-release-drag-release chain without ever returning to a menu. Right-click or `Esc` drops the anchor.

### 2.2 Route proposal

Dragging means **the run goes exactly where the player points** — corrected by playtest (2026-08-04), which found the routed default made "decisions that don't make any sense": building is intentional, RCT-style, and terrain is dealt with deliberately rather than optimised away on the player's behalf. Assistance exists, opted into, never assumed.

| Modifier | Proposal |
| --- | --- |
| *(none)* | **Straight** — direct line on one of the sixteen directions, terrain be damned. The player picks the angle by pointing and the length by dragging; every tile shows its own validity and cost, and an illegal tile refuses loudly. |
| `Ctrl` | **Single tile** — exactly the tile under the cursor. The one-piece-at-a-time verb. |
| `Alt` | **Contour lock** — hold current elevation, refusing anything that would climb. |
| `Shift` | **Smart assist** — cheapest legal path, weighted toward straightness, held to a corridor a few tiles either side of where the player pointed. It finds the narrows and eases a grade; it never wanders off on an itinerary of its own. |

The straight drag is the default because dealing with terrain **is the game**: the player who lays a run into a hillside should feel the refusal, read the ground, and choose — contour round, cut through, or bridge — not have the choice made silently for them. The assist is quality-of-life for a decision already taken.

The assist's proposal must be **stable** — small cursor movements must not cause the route to flip between equal-cost alternatives. Hysteresis on route selection; prefer the previous frame's shape when costs are within a few percent. A flickering ghost is unusable.

### 2.3 The cost HUD

Follows the cursor at a fixed offset, never covering the tile being pointed at.

```
        ┌──────────────────────┐
        │  18 tiles     $740   │
        │  1 bridge     ▲ 2.1% │
        │  ────────────────    │
        │  Balance     $9,260  │
        └──────────────────────┘
```

Tile count, total cost, notable features (bridges, tunnels, cuttings), and maximum gradient on the route. Balance-after in `ok`, or in `warn` when it would go negative — with the whole ghost turning `warn` and the route becoming uncommittable.

Cost is the number the player watches while deciding. It updates live, it never lags, and it is never more than a glance away from the cursor.

---

## 3. Failure must be loud

Every rejection is communicated in three channels at once — visual on the offending tile, textual in the reason chip, and audible with a soft low thud. Silent rejection is the single most alienating thing an interface can do, because the player cannot distinguish "not allowed" from "broken."

| Condition | Ghost | Reason chip |
| --- | --- | --- |
| Too steep | Offending tiles `warn`, cliff face highlighted | *Too steep — 8% exceeds 4% limit* |
| Water too wide | Span drawn `warn` with the width called out | *Span too wide — 9 tiles, max 8* |
| Curve too tight | The tight corner pulses `warn` | *Curve too tight here* |
| Occupied | Existing track flashes | *Track already here* |
| Can't afford | Whole ghost `warn`, cost in `warn` | *Short by $240* |
| Off map | Route clipped at the boundary | *Map edge* |
| Turnout too shallow | Junction marked `warn` | *Junction angle too shallow* |

The reason chip sits next to the cost HUD and reads as plain language, never as an error code. Where a rule has a number, the message states both the value and the limit — that is how the player learns the rule rather than merely bouncing off it.

Where an action is *impossible from the outset*, the tool signals it before the drag: the cursor changes and the tool slot dims. Prevention beats rejection.

---

## 4. Demolition

Demolition is a first-class verb because the vision makes building free to experiment with. It refunds in full.

- **`Del` or the demolish tool.** Drag to demolish a run; the ghost shows what will go, tinted `warn`, with the refund total.
- **Right-drag** demolishes from within the Build tool, so correcting a mistake never requires switching tools. This is the single biggest quality-of-life affordance in the whole build loop.
- Demolished track lifts with a 2-frame animation and a dull *clank*, leaving a scar decal that fades over about thirty seconds. The world remembers, briefly, that you changed your mind.
- Removing track that a train is currently on, or that is the only route serving a station, asks for confirmation and names the consequence: *"This will strand 2 trains and cut Millhaven off."*

## 5. Undo

**`Ctrl+Z` undoes the last build or demolish**, including a whole dragged run as one unit. Redo with `Ctrl+Shift+Z`. At least fifty levels deep.

Undo is what makes the promise of reversibility true. A player who knows a mistake costs one keystroke experiments freely, and experimenting freely is the entire point of "building is free to experiment with; only commitment costs."

Undo covers construction actions only — it does not rewind the simulation. Building at scale in a running world is safe *because* the actions are reversible, not because the clock is.

---

## 6. Stations are built, not given

The vision says rail is the only thing you build directly, and that the town grows around the service you provide. Both hold — and they require the player to place stations, because otherwise there is nothing new to reach and the loop's third rung has nothing to stand on.

The resolution: **a station is a kind of track.** It is placed with the track tools, on a piece of line, as a platform. The player is still only ever building railway. The town is still only ever a consequence.

| Tier | Platforms | Character |
| --- | --- | --- |
| **Halt** | 1 short | Cheap, slow to board, serves a small catchment |
| **Station** | 2 | The workhorse |
| **Interchange** | 4 | Expensive, fast turnaround, wide catchment, lines can meet |
| **Terminus** | 3 stub | End-of-line, high capacity, no through running |

Placement shows the **catchment ring** live during the drag, along with how many existing buildings and how much unserved demand fall inside it. Siting a station is then a real decision made with real information, rather than a guess.

Freight facilities work the same way: a goods platform placed against an industry.

---

## 7. Layers, reserved

Tunnels and elevated track are not in the first pass, but the interaction is designed now so it can slot in without re-teaching the player:

- `PgUp` / `PgDn` change the working layer. The world dims the layers you are not on and draws them as ghosts.
- Grade separation is proposed automatically when a route would cross existing track, with the cost shown, so the player learns that crossings have options before tunnels formally exist.

---

## 8. Building while the world runs

Building never pauses the game, and the game never pauses to let you build. But the player may pause and keep building, and the world will apply everything the instant it resumes. Planning during a pause is a legitimate and supported style.

At faster speeds, build interaction stays at full responsiveness — the ghost, cost and validity are presentation, and they run at frame rate regardless of how fast the world is ticking.

---

## 9. Acceptance bar

1. Laying a twenty-tile line is one continuous gesture, and it feels good enough to do idly.
2. Every rejected action tells the player why, in words, with the number.
3. The proposed route is the one an experienced player would have drawn, at least four times in five.
4. A mistake costs one keystroke to erase.
5. A new player discovers drag-to-build without being told.
6. The ghost never flickers between alternatives while the cursor is still.
