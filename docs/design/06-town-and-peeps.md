# 06 — Town & Peeps

**The town is the scoreboard, and it must read like a place.** The vision is explicit: the town is the readout, it thickens where trains reach and thins where they don't, and residents stay individually visible and knowable. That is a presentation promise as much as a simulation one — a correct density model that renders as an abstract heat pattern has kept none of it.

---

## 1. Principle

**Growth must be visibly *caused*.** Nothing in this game grows on a hidden counter, and nothing should *look* like it does either. The player should be able to watch a building go up and know exactly which decision of theirs put it there.

The chain has to be legible end to end:

```
  line reaches a place  →  station serves it  →  people can get where they're going
        →  the district thickens  →  more people  →  more demand  →  reach further
```

Every arrow in that chain needs to be visible in the world, not merely true in the model.

---

## 2. The town is made of buildings, not tiles

A tile is not a building. A tile is a **block** containing up to four **lots**, each with a building on it. Density is how many lots are occupied and how tall they've grown, and that resolution is what lets a district read as a place rather than as a bar chart.

### 2.1 Tiers

| Tier | Form | Appears when |
| --- | --- | --- |
| **1 — Cottage** | Single storey, pitched roof, one chimney | Any service at all |
| **2 — Townhouse** | Two storeys, shared walls, small yard | Sustained decent service |
| **3 — Shopfront** | Ground-floor commerce, awning, sign | Good service, near a station |
| **4 — Block** | Three to four storeys, flat roof, courtyard | Excellent service, high demand |

Each tier has four variants and two roof materials, chosen by world hash so a street has rhythm without repetition. Buildings sit on the lot with a small hashed offset and orientation, so a block reads as organic rather than gridded.

### 2.2 Districts have character

Growth is not uniform. What a district becomes depends on what serves it:

- Near a **passenger station** — residential, thickening to shops and blocks.
- Near a **goods facility** — warehouses, yards, workshops.
- Along a **busy corridor** — commercial frontage facing the line.
- **Far from any station** — stays rural: farms, a lane, scattered cottages.

That last one matters as much as the others. The unserved parts of the map must look *deliberately* unserved — quiet countryside, not empty space — so that the contrast with a thriving district reads as a consequence rather than as unfinished content.

---

## 3. Growth and decline are events, not interpolation

The single most important change in this brief: **growth is something the player watches happen.**

**Over days.** Playtest (2026-08-04): *"house growth happens too quickly, within a few in-game minutes. It should be more gradual, e.g. over a few days."* It was worse than that — density approached its target every tick, so a served block filled inside a real second and the whole town was finished before the player let go of the mouse. Growth is denominated in **sim days** now: a lot at the heart of a served town is claimed half a day in, and the block reaches its fourth lot on day five. [17 — Time & Pacing](17-time-and-pacing.md) §5 carries the table and the arithmetic.

The first cottage inside the first day is deliberate. §1 wants growth *visibly caused*, and a consequence the player cannot connect to their decision has not been caused as far as they are concerned — so the **district** is what takes days, while the first hint that it has started is prompt. The sequence below is unaffected: those are real seconds, and they are about how long a small event should hold the eye.

### 3.1 Construction

1. A lot is chosen. A **surveyor's stake** appears — tiny, easy to miss, and the first hint.
2. **Scaffold** goes up, holds for around eight seconds, with a small dust puff and occasional sound.
3. The **building appears** with a two-frame settle. Town Talk may mention it: *"New shop opened on Mill Row."*
4. Windows light that night.

Eight seconds is long enough to notice and short enough not to be tedious. It converts an invisible number crossing a threshold into a small event with a location — which is exactly what "growth is local and caused" needs to feel true.

### 3.2 Decline

Decline is the same sequence backwards, and it must be *legible and gradual*, because the vision promises it is recoverable at any point:

1. **Windows go dark.** The earliest and gentlest signal — a district that stops lighting up at night is in trouble, and the player feels it before they can name it.
2. **Boarded windows**, peeling paint, an overgrown yard.
3. **Occupants leave.** The peep walks out with luggage, and Town Talk names them: *"The Aldertons left Westbrook — 22 minutes to anywhere."* A named departure lands far harder than a decrementing counter.
4. **Derelict**, then eventually cleared to an empty lot with a foundation scar that persists.

At every stage, restoring service reverses it — and reversal is visible too: lights come back on, boards come off. Recovery that the player can *see* is what makes the design's "recoverable at any point" a felt promise rather than a technical one.

---

## 4. Peeps

### 4.1 They travel

The heart of it: **peeps actually make journeys.** A peep who never moves is a mood icon with a name attached, and the "knowable" promise dies there.

```
  home  →  walk to station  →  wait on platform  →  board
        →  ride  →  alight  →  walk to destination  →  spend time  →  return
```

Every stage is visible. A peep walking down a lane to catch the morning train is the game's whole thesis rendered at two pixels tall.

Simulate a bounded set of peeps in full — enough that platforms feel populated and streets have life — and abstract the rest into district-level flow. The abstracted majority still produce demand, service pressure and complaints; they simply don't have sprites. **Which peeps get simulated in full is biased toward wherever the camera is looking**, so the world is always at its most alive exactly where the player is watching.

### 4.2 They are individuals

- **Full names** from a combinatorial pool, so `Mara Aldertone` and `Theo Finch` persist and recur.
- **Households** — peeps live together, share a building, and move together. A family leaving is a bigger event than a person leaving.
- **Routines** — a home, a destination, and a time they habitually travel. Rush hours emerge from the sum of individual routines rather than being imposed by a curve, which means a player can serve them by observing rather than by reading a manual.
- **Memory** — a peep remembers recent journeys, and their patience is shaped by their history. Someone who has had four good commutes tolerates a bad one; someone who has had four bad ones leaves.
- **Portraits** — small, procedural, four body types with palette-drawn variation. Enough that the person in the Inspector is *a* person.

### 4.3 Mood is caused and expressed

Mood comes from accumulated experience — wait times, journey times, whether they got where they were going — and it is expressed in three places:

- **On the sprite** — posture and a small mood tint, readable at a glance on a crowded platform.
- **In Town Talk** — specific, named, numbered.
- **In their decisions** — a frustrated peep gives up and walks, and that shows in the world as somebody trudging away from a platform. Sustained frustration means they move away.

A platform crowded with visibly frustrated people is a diagnostic the player reads without any interface at all. That is the ambient tier of [05 — Inspection & Overlays](05-inspection-and-overlays.md) doing its job.

---

## 5. What growth actually responds to

Growth is driven by **accessibility, not proximity.** A station five tiles away that connects to everywhere is worth far more than one two tiles away that goes nowhere useful. Density should respond to:

| Factor | Effect |
| --- | --- |
| Journey time to places people want to go | The dominant term |
| Service reliability — do trains actually come? | Multiplier. A scheduled call counts at half the weight of a delivery, and the score forgets at one point per sim-minute — corrected (2026-08-04) from per-tick decay, which no schedule on earth could outrun; the multiplier sat at zero everywhere and growth was structurally dead. Arithmetic in `rail_sim/src/stations/service.rs`. |
| Walking distance to a station | Falloff, but secondary to the above |
| Local employment — industries and commerce reachable | Enables higher tiers |
| Terrain suitability | Flat and dry builds; cliffs and marsh don't |

This is what makes *network shape* matter rather than *station count*. A player who rings a town with unconnected halts should see very little growth, and should be able to find out why by asking the panel. The lesson the game wants to teach is that a railway is a network, not a collection of stops.

**Growth is capped by reach.** A district cannot exceed the tier its connectivity supports, no matter how long it sits there. That cap is what converts "the town is thriving" into "I need to build more railway."

---

## 6. The town asks for things

The loop's third rung — *"a new demand appears somewhere you can't yet reach"* — is partly the town's job.

- A district that has grown to its cap **asks for a better station**, with a marker in the world and a Town Talk line.
- A district with no service but real potential **asks for a line**: *"Ridgeline has 40 people and no station."*
- An industry with a full warehouse **asks for collection.**
- Two districts with heavy walking traffic between them **ask for a direct connection.**

These are invitations, never demands. Nothing punishes ignoring them. But they mean the player always has a visible menu of next moves, which is the difference between an open-ended game and an aimless one.

---

## 7. Scale

Town scale, not city scale — deliberately, per the vision. A mature map should hold on the order of a few thousand residents across a handful of towns, at a density where individuals stay visible and a player can plausibly recognise a name.

The moment the town becomes a statistic, the emotional hook is gone. If growth ever threatens that, the answer is to cap density and push expansion outward into new settlements — more places, not bigger numbers.

---

## 8. Acceptance bar

1. A player can look at a district and say whether it is growing or declining, without any interface.
2. Watching a building go up is a small, noticeable pleasure.
3. A named peep can be followed from their front door to their destination.
4. A player who lets service lapse sees people leave, by name, and can bring them back.
5. The unserved parts of the map look intentionally rural, not unfinished.
6. At night, a well-served district is visibly full of light and a neglected one is dark.
