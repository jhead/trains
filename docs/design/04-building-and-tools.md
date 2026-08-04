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
| `Shift` | **Smart assist** — cheapest legal path, weighted toward straightness, **leashed to six tiles either side** of the straight segment the player pointed along. It finds the narrows and eases a grade; it never wanders off on an itinerary of its own. |

The leash is the whole difference between assistance and autopilot. Six tiles is wide enough to step round a boulder, cross at the narrows, or take a grade at an angle, and far too narrow to decide that the line should really go up the next valley. A search with no corridor produces the route that lost the playtest; a search with this one produces the route the player was already drawing, tidied.

The straight drag is the default because dealing with terrain **is the game**: the player who lays a run into a hillside should feel the refusal, read the ground, and choose — contour round, cut through, or bridge — not have the choice made silently for them. The assist is quality-of-life for a decision already taken.

The proposal must be **stable** — small cursor movements must not cause the route to flip between alternatives. A flickering ghost is unusable, and the failure is worse than it sounds: a wobble in a sixteen-direction snap is not one tile changing its mind, it is the whole run swinging onto a different angle.

The fix is a **detent, not a timer and not frame-to-frame hysteresis**. A half-step ray has to beat the best compass ray by a real margin before it wins, and ties resolve to the shorter run on the lower-indexed direction. That leaves the proposal a pure, deterministic function of the anchor and the cursor tile with a dead band around every compass ray, so it cannot oscillate for a held cursor and cannot depend on how the cursor arrived. Remembering the previous frame would buy the same stillness and cost the property that makes the tool predictable: the same two tiles always propose the same run.

### 2.3 The cost HUD

Follows the cursor at a fixed offset, never covering the tile being pointed at.

```
        ┌──────────────────────────┐
        │  18 tiles   $740         │
        │             - 1 bridge   │
        │  Balance    $9,260       │
        └──────────────────────────┘
```

Tile count, total cost, deck tiles called out as their own line item — a wide crossing is most of the bill and the player should see that is what they are paying for — and the balance the run would leave, in `ok`, or in `warn` when it would go negative, with the whole ghost turning `warn` and the route becoming uncommittable. Demolition uses the same readout with the refund in place of the price.

**"While building" means whenever there is a ghost, not while a button is held.** Those are different windows, and the difference is most of the tool: the Build verb keeps its anchor after every commit, so the ghost follows the cursor between drags, and a continuous-build player spends most of their time in exactly that state. Keying the readout off the drag left them pricing a run they could see and could not cost. It keys off the preview, so if a ghost is on screen its price is on screen — from the first tile, in every modifier mode. If the pointer leaves the window the readout moves to a fixed corner rather than leaving with it; a player whose mouse has wandered has not stopped caring what the run costs.

Cost is the number the player watches while deciding. It updates live, it never lags, and it is never more than a glance away.

Maximum gradient on the route, and the terrain features other than bridges, are wanted here and not there yet.

### 2.4 Bridges are not a mode

**There is no bridge tool.** Drag across water and the tiles over water are deck; the price changes and nothing else does. A mode would make the player answer a question the ground has already answered, and it would put a decision between them and the drag.

What the player is choosing is not *whether* to bridge but *where*, and the price ladder is what makes that a decision. A crossing may span **one to eight** water tiles, and the per-tile rate climbs with the span:

| Span (tiles) | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Cost per tile | 8× | 14× | 20× | 30× | 42× | 56× | 72× | 90× |

The first three rungs are the **cheap tier** — a ford, a stream, a small river, the things a young railway crosses without much thought — and finding one of them is what makes scouting a river worth the walk. Above that a crossing is a premium one: the rate rises *and* there are more tiles to pay it on, so a full eight-span deck runs 720× base. That is a monument the railway saves toward, not a shortcut it takes, and it should read that way in the readout before it is ever committed.

Nine tiles of water is refused, with the span it measured named in the chip. See [02 — World & Terrain](02-world-and-terrain.md) §3.4 for why every river the generator draws has to be crossable somewhere.

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

**`Ctrl+Z` undoes the last build or demolish**, including a whole dragged run as one unit. Redo is `Ctrl+Y`. Fifty levels deep.

Undo is what makes the promise of reversibility true. A player who knows a mistake costs one keystroke experiments freely, and experimenting freely is the entire point of "building is free to experiment with; only commitment costs."

Undo covers construction actions only — it does not rewind the simulation. Building at scale in a running world is safe *because* the actions are reversible, not because the clock is.

---

## 6. Stations are built, not given

The vision says rail is the only thing you build directly, and that the town grows around the service you provide. Both hold — and they require the player to place stations, because otherwise there is nothing new to reach and the loop's third rung has nothing to stand on.

The resolution: **a station is a kind of track.** It is placed with the track tools, on a piece of line, as a platform. The player is still only ever building railway. The town is still only ever a consequence.

| Tier | Platforms | Price | Reach | Character |
| --- | --- | --- | --- | --- |
| **Halt** | 1 short | $400 | 3 | Cheap, slow to board, serves a small catchment |
| **Station** | 2 | $1,200 | 5 | The workhorse |
| **Interchange** | 4 | $4,000 | 8 | Expensive, fast turnaround, wide catchment, lines can meet |
| **Terminus** | 3 stub | $2,600 | 6 | End-of-line, high capacity, no through running |
| **Goods Platform** | 2 | $900 | 1 | Freight, placed against an industry lot |

Freight facilities work the same way: a goods platform placed against an industry. It is the fifth row rather than a separate system — same tool, same line, one extra rule.

### 6.1 Placing one

**Station is a slot on the menu row** ([03](03-ui-system.md) §7), beside Track, with `P` printed on it. Arming it opens the tier row beneath, which names every tier and what it costs — so the choice and its price are both on screen before the click, and neither depends on knowing a key.

Placement shows the **catchment ring** live during the drag, along with how many existing buildings and how much unserved demand fall inside it. Siting a station is then a real decision made with real information, rather than a guess. The chip at the cursor carries the armed tier, its price, its platform count, its reach, and what the balance would be afterwards.

Every refusal speaks, with its rule and its number (§3):

| Condition | Reason chip |
| --- | --- |
| No rails under the cursor | *Platforms need track — lay the line first* |
| Open water | *That tile is water — bridge it first, or build ashore* |
| Another stop within `MIN_STATION_SPACING` | *Too close — 2 tiles, need 3* |
| The straight run is short of the tier's platforms | *Not enough platform — 3 tiles of line, needs 4* |
| A terminus mid-run | *Terminus needs a dead end — the line runs through here* |
| A goods platform touching no lot | *Goods platforms load an industry — none touches this tile* |
| Can't afford it | *Short by $5.00* |

A refusal the **sim** raises — an upgrade whose platforms will not fit, a site something else took between the hover and the click — lands on the same line. The rule is that nothing about a station is ever refused silently.

Opening a station is news: Town Talk says *"Brackwell opened — no line calls there yet"*, which is also the next thing to do about it.

### 6.2 Managing one

Selecting a stop opens the Inspector on it: tier, catchment, platforms and capacity, how many lines call there, its service score and trend. Two verbs live on that card, because they act on that stop and nothing else:

- **Upgrade** — one rung up the Halt → Station → Interchange ladder, labelled with the *difference* in price and what the money buys. Where the upgrade cannot be had the button says why instead of offering — *"Not enough platform — 3 tiles of line, needs 4"*, *"Interchange is the top of the ladder"* — because §3's last line asks a tool to signal an impossible action before the click, not after it. `U` over a stop does the same thing.
- **Demolish** — a full refund (§4), through the confirm that names the consequence: which lines lose a call, and which of them is left with nowhere to run.

### 6.3 And then the town grows

A stop that a line calls at earns a service score, and `town` grows building density inside that stop's catchment in proportion to it. That is the whole loop the vision promises — place a platform, run a train through it, watch a place appear around it — and `rail_town/tests/station_grows_a_town.rs` walks it end to end on a generated map, through the same commands the player's clicks issue, with an unserved control site that must stay open country.

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
