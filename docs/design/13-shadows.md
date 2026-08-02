# 13 — Shadows

**Status: feature design, subordinate to [01 — Art Direction](01-art-direction.md).** Where this brief and 01 disagree, 01 wins. The pixel contract (§2), the palette cap (§3), the time-of-day pass (§3.4) and the composition rules (§6) are constraints on everything below, not inputs to be negotiated.

**The premise:** the world already has a clock, a sun and a set of ramps drawn as though light came from somewhere. What it does not have is *consequence* — nothing casts, nothing receives, and the hour of the day changes the colour of the frame without changing its shape. Shadows are how the world stops being a coloured map and starts being a lit place.

The whole design turns on one question, and it is a gameplay question, not an art question:

> **A shadow that tells the player how tall a ridge is has earned its cost. A shadow that makes them wonder whether that dark patch is low ground has not.**

Everything here is arranged so the first kind happens and the second cannot.

---

## 1. The look in one sentence

**One hard-edged sun that steps through five positions a day, casting short shadows that always have a visible cause, and no shadows at all after dark.**

Three properties, and every proposal below is tested against all three:

- **Hard.** Every shadow edge is a texel edge. No blur, no penumbra, no alpha ramp, no soft falloff. A shadow is a region of the world drawn in a different colour, and the boundary is drawn, not computed at render time. This is [01 §2](01-art-direction.md) and it is not negotiable.
- **Short.** A shadow is a **hem**, never a region. Physically correct low-sun shadows at this density would mat an entire district solid at dawn. Length is capped at three tiles, and the cap is a hard clamp that measurement can tighten but nothing may loosen.
- **Caused.** The near edge of every shadow abuts something the player can see in the same frame — a cliff face, a band step, a building, a bridge, a train. A dark patch of ground with no visible cause is the single failure mode of this feature, and it is the only one that turns a legible map into mud.

---

## 2. The sun

### 2.1 The sun does not rotate continuously

A continuously rotating sun is wrong, and it is worth saying exactly why rather than gesturing at cost.

Terrain art is composited into 16 x 16 tile chunks and baked when data changes ([01 §2.5](01-art-direction.md)). A standard 64² map is sixteen chunks of 512 x 512 texels — about **1.7 ms of compositing and sixteen megabytes of texture upload** for a full-map rebake. Sun direction is terrain data: change it and every chunk is stale. A sun that rotates continuously therefore asks for that rebake **every frame**, which is precisely the shape of the regression that has already made this game unplayable once.

And it buys nothing. At 32 texels per tile with hard edges, a shadow boundary that advances by less than one texel per second is either invisible or it *crawls* — and crawling is the exact artefact the pixel contract's texel snap exists to eliminate. A continuous sun would spend the frame budget to produce the one thing the art direction forbids.

**So the sun is quantised**, and the quantisation is coarse on purpose.

### 2.2 Five states

The sun state is a **pure function of the day cycle's position**, sitting beside the phase and the window fade in the same read model. That is the whole integration: it obeys pause, it obeys the speed multiplier, it survives save and load, and it can never disagree with the tint about what time it is.

| State | Cycle window | Azimuth | Lit edges | Casts toward | Rise per tile | Max length |
| --- | --- | --- | --- | --- | --- | --- |
| **Low East** | 0.00 – 0.14 | E | E | W | 2 | **3 tiles** |
| **High East** | 0.14 – 0.34 | SE | S, E | NW | 4 | **1 tile** |
| **High West** | 0.34 – 0.54 | SW | S, W | NE | 4 | **1 tile** |
| **Low West** | 0.54 – 0.66 | W | W | E | 2 | **3 tiles** |
| **Down** | 0.66 – 1.00 | none | none | none | — | **0** |

**Four azimuths, two length steps, five steps per twelve-minute day.** At 1x that is a step roughly every 100 seconds; at 3x, every 33.

Four decisions inside that table are load-bearing:

**The sun never goes north.** It rises east, crosses south, sets west. The world is drawn from a high angle with a fake front face ([01 §6.1](01-art-direction.md)) — a northern sun would light every face the camera cannot see and shade every face it can. There is no hour at which this game looks better from behind.

**Azimuth and length are not independent.** They are both functions of one clock, and allowing (East, short) or (South, long) would multiply the state count for combinations the sky cannot produce. One state, one look.

**There is no pure-south noon state.** A cast straight north lands behind the drop that made it, where the camera cannot see it. The midday read is instead delivered by the two diagonals at their shortest.

**Three of the five steps land on a tint keyframe.** The steps at 0.14, 0.54 and 0.66 are exactly the moments the full-screen tint begins to move into day, into dusk and into night. A shadow direction changing under a colour change the player is already watching is not a visible event. Only one step — the East-to-West flip at solar noon — lands in flat daylight, and it is deliberately the step where shadows are at their **shortest**, so it is the smallest change of the five: a one-tile hem moves from one diagonal to the other.

### 2.3 What a step costs

A sun step marks every chunk stale. It does **not** rebuild them all in one frame.

| | Terrain edit | Sun step |
| --- | --- | --- |
| Chunks affected | up to 4 | all of them |
| Rebuild policy | immediately, same frame | **one chunk per frame, nearest the camera first** |
| Frame cost | ~0.11 ms x 4 | **~0.11 ms, for as many frames as there are chunks** |
| Wall clock to settle | one frame | 0.27 s on a 64² map, 1.07 s on a 128² |

The shortest gap between two sun steps is 72 seconds at 1x and 24 seconds at 3x. Settling in about one second inside a 24-second window is not a race. The sweep is ordered from the camera outward so that if any of it is perceptible, the perceptible part happens first.

**Total sun cost across a whole day: five sweeps, each about a quarter of a second at roughly a tenth of a millisecond per frame.** That is a rounding error, and it is only a rounding error because there are five steps rather than three hundred.

---

## 3. What a shadow is made of

### 3.1 The operator

> **A shadow is the receiving material's own ramp, stepped down by one. Where there is no step below, the material does not shadow.**

That is the entire colour model, and it has four properties nothing else has:

1. **It is always in palette.** The result is a colour the artist already authored for that material. There is no blend, no multiply, no derived value, no set of 45 colours that quietly becomes 400 at runtime.
2. **It is the right magnitude by construction.** Adjacent steps in these ramps sit at about 70% of each other's luminance. A shadow at 70% of full sun is what shade looks like on a clear day, and it is close to what the night tint's legibility floor allows. The ramps were authored as the same material lit and unlit; this simply uses them that way.
3. **It is a cell selection, not a second pass.** A shadowed tile is the same tile art at a different ramp index. It composites through machinery that already exists, costs no extra layer and adds no per-texel blending.
4. **It solves the track problem for free.** Ballast sits at the bottom of its ramp already, so the operator does nothing to it — a shadow crossing a main line darkens the ground either side and leaves the ballast and the railhead exactly where they were. [01 §3.3](01-art-direction.md) rule 2 says `railS` on `ballastD` is the widest value gap in the palette and is reserved for the railhead; this design cannot violate that rule because the operator has nothing to say about it. **No special case, no exemption list.**

The same reasoning covers station platform stone and the surfaces of bridges. Anything drawn near the bottom of its ramp is already dark and does not need to be darker.

### 3.2 The palette cost: two colours in, two out

The operator needs somewhere to go at the bottom of a ramp. Grass and hill are the materials that cover routable ground, and both bottom out at a step that flat land already uses. They each need one more:

```
─ Shade steps (new) ───────────────────────────────────
grassS    #1f2b1e    below grassD
hillS     #242d16    below hillD
```

The rule that produced those values, which matters more than the values: **a shade step sits at about 70% of the luminance of the step above it, with its hue pulled roughly a tenth of the way toward `outline`.** Cool violet shadows are the palette's stated character ([01 §3.1](01-art-direction.md)); a shadow that stays perfectly on the material's own hue reads as a different material rather than as the same one in shade.

The cap is forty-five and adding one requires deleting one, so two go:

| Deleted | Why it can go |
| --- | --- |
| `sandL #bda87a` | The brightest colour in the terrain set, and [01 §3.3](01-art-direction.md) rule 1 already forbids it on flat ground. Beaches are thin transition lips on maps that are 0–4% sea. The cost is that a beach's sun lip falls back to `sandM` and reads more softly, which on a four-texel lip is not a cost anyone will find. |
| `waterL #335b78` | Water keeps `waterD` and `waterM` for depth banding and `waterF` for shallows and foam. Three values across the small share of the frame water now occupies is enough structure to stop open sea being one flat field. |

**Net palette: still forty-five.**

Every other material shadows without new colours: sand steps `sandM` to `sandD`, rock steps `rockM` to `rockD`, water steps `waterM` to `waterD`, and the darkest step of each simply does not shadow.

### 3.3 The edge

Shadow resolution is **one tile**, which would give tile-square edges and read as a checkerboard. The boundary is broken by a **fray**: a ragged three-to-six texel band of the darker step, stamped on the lit tile's edge facing the shadow, wobbling by one texel along its length and skipping about one column in five.

This is exactly the treatment that already stops a coastline reading as a ruled line rather than as a shore, and it is world-anchored — the wobble hashes on integer world coordinates, so it is nailed to the ground and cannot boil under scroll ([01 §2.4](01-art-direction.md)).

**The fray is the only place dither is allowed.** One texel of a world-anchored 50% Bayer is permitted at the fray's outermost column. Two texels is not, and dither as the *interior* of a shadow is rejected outright — see §11.

### 3.4 One shadow alpha

Where a shadow is drawn as a composited sprite rather than as a ramp step — the contact line under a building, a train's projected silhouette — it is `outline` at **59% alpha**, and that number is a single named constant for the whole game. One shadow colour, one shadow alpha, everywhere. A second value invents a second light source.

---

## 4. What casts, and what receives

Ranked by value per unit of cost. The ranking is also the build order and the reverse of the cut order.

| Rank | Caster | Receives on | Mechanism | Sprites added | Value |
| --- | --- | --- | --- | --- | --- |
| 1 | **Cliff faces and band steps** | ground below them, 1–3 tiles | terrain chunk composite | **0** | **Information.** Length reads drop height. |
| 2 | **Buildings, stations, industries** | ground, and the building behind | building overlay layer | +1 per standing building | Picture. Sells the light direction. |
| 3 | **Bridges** | the water or ground under the deck | track composite | **0** | Picture, and it seats the bridge. |
| 4 | **Trains** | track and ground | projected silhouette sprite | +1 per car | Picture. Sells the weight. |
| 5 | **Trees and rural props** | ground | prop cell | **0** | Picture, free. |
| 6 | **Peeps** | — | **none** | 0 | Rejected. |

**Everything that does not move casts for free**, because its shadow is baked into art that is already being drawn. That is the whole reason this system can exist inside the frame budget.

**What receives:** all terrain, all buildings, bridge decks, prop ground. **What does not:** the railhead, ever (§3.1 makes this automatic); lit windows (§8); anything in the UI, in Map View, or in an overlay.

**Map View has no shadows.** It is a schematic read at four texels per tile ([02 §6](02-world-and-terrain.md)), and shade at that scale is indistinguishable from an elevation band. Adding it would corrupt the one view whose entire job is unambiguous terrain silhouette.

---

## 5. Static and dynamic

The split is the load-bearing performance decision, and the rule is short:

> **A static caster's shadow costs nothing per frame, because it lives in art that is already baked. A moving caster's shadow costs at most one sprite and no new art.**

| Class | Rebakes when | Lives in | Steady per-frame cost |
| --- | --- | --- | --- |
| Terrain onto terrain | sun steps, terrain edits | the terrain chunk composite | **zero** |
| Bridges onto water | sun steps, track edits | the track composite | **zero** |
| Props onto ground | never | the prop's atlas cell | **zero** |
| Buildings onto ground | sun steps, lot phase changes | one overlay sprite per lot, frame chosen from a bank of five | one atlas index write per lot, five times a day |
| Trains onto track | every frame | a second draw of the car's own sprite | one transform write per car — which it already computes |
| Contact shadows | never | the caster's own cell | **zero** |

A moving caster is explicitly forbidden from generating art at runtime, from sampling the height field per frame, or from adding more than one sprite. A train's shadow is its own sprite redrawn at ground depth, offset by whole texels along the sun vector, tinted `outline` at the shadow alpha. That is zero new art for any number of directions, kinds and car types, and it stays correct when the sprite bank grows.

---

## 6. Terrain onto terrain

The expensive one, the atmospheric one, and the only one that pays the player back.

### 6.1 How far a shadow reaches

For each land tile, march toward the sun up to the state's maximum length. The tile is in shade if any tile along the march stands at least `rise x distance` above it, where `rise` is the sun state's height gain per tile.

Terrain is banded: an elevation band is three height units, and adjacent land tiles differ by zero, three or four. That makes the result crisp and countable:

| Drop | Low sun (rise 2, max 3) | High sun (rise 4, max 1) |
| --- | --- | --- |
| Δ3 — a bank | **1 tile** | none |
| Δ4 — the grade limit, a full cliff face | **2 tiles** | **1 tile** |
| Δ6–8 — two steps, a real ridge | **3 tiles** | 1 tile |
| Flat | none | none |

Two consequences fall straight out, and both are gifts:

**Shadow length is a read of drop height.** A face at the grade limit and a three-band ridge draw the same face today; they throw visibly different shadows. The player learns *long shadow means tall ridge means expensive or impossible* without a legend, an overlay or a number. [02 §2.3](02-world-and-terrain.md) asks for exactly this and calls an unreadable ridge "an invisible tax".

**At midday, the only shadows on the map are at the feet of cliffs track cannot cross.** A Δ4 drop is the grade limit — the last delta track may climb. At high sun nothing shallower casts. So for the middle 40% of every day, a shadow on the ground means *the line cannot go that way*, and it says so in the world art rather than in a refusal message. That is a legality read for free, and it is the single best thing in this brief.

### 6.2 The guards

Terrain self-shadowing is the feature most capable of turning a map to mud, so it ships with three hard limits rather than good intentions:

1. **Length is capped at three tiles.** Not a soft falloff, a clamp. A shadow is a hem along the foot of relief; it is never a region, and a region is what makes a player ask whether they are looking at shade or at low ground.
2. **Every shadow's near edge abuts a drawn cliff face or band step.** The cast originates at a break of slope that is itself drawn. There is always a cause in frame.
3. **No more than one quarter of visible land may be in shade at any sun state.** If a map exceeds it — a Rugged preset at low sun is the case to watch — the cast length for that state drops by one until it fits. This is a measured invariant with an automatic remedy, not a note.

### 6.3 The honest answer on legibility

Asked plainly: does terrain self-shadowing help the player, or only the picture?

**It helps, conditionally, and the condition is guard 2.** Shadow length carries information the current art cannot express, and it carries it in the channel the player is already using to read terrain. A ridge that throws a bar of shade into the valley at dawn is more legible than the same ridge drawn flat, not less.

**It hurts the moment a shadow's cause leaves the frame.** Two dark tiles with a visible cliff above them read as shade; the same two tiles with the cliff scrolled off-screen read as lower ground, and the player has to move the camera to disambiguate. This is why the length cap is three and not eight: at three tiles a shadow and its cause are never more than three tiles apart, which at every zoom is comfortably inside one screen.

**The failure case is a soft, wide, causeless gradient** — precisely what [02 §2.3](02-world-and-terrain.md) rejects for elevation itself, and it would be an odd design that fixed the heightmap's legibility and then reintroduced the same problem in the lighting.

There is one place a shadow can actively cost the player something, and it is worth naming: **a station's waiting crowd.** [05](05-inspection-and-overlays.md) makes the platform a first-class ambient read — a crowded platform is supposed to be visible before any panel is opened. Platform stone sits near the bottom of its ramp and so does not shadow (§3.1), and peeps do not receive cast shadows at all. The crowd stays readable at every hour by construction.

---

## 7. Buildings, bridges, trains, props

### 7.1 Buildings cast short, and deliberately not correctly

A three-storey block at dawn physically throws forty texels of shade. A terrace of them, at four lots to a tile, mats an entire district into one dark slab — and district character is the thing town art exists to communicate ([06](06-town-and-peeps.md)).

**So a building's cast is stylised: three texels at high sun, eight at low, never more.** It swings direction with the sun and it lengthens, so the light reads; it never reaches the next lot, so the town does not go black. This is the one place in the brief where physical correctness is traded away outright, and the trade is not close.

The building's **contact shadow** — a one-texel `outline` line under the footprint at the shadow alpha — is a separate thing and is not a sun effect at all. It is ambient occlusion: it is what stops a building floating above the ground it stands on. **It is drawn at every hour, including in full night**, and it is the reason night can dispense with cast shadows entirely without anything appearing to hover.

### 7.2 The building overlay layer

The town already spends one sprite per building at night to draw lit windows over it. Cast shadows and lit windows are near-complements in time, so they share that budget:

- **Daylight:** one overlay sprite per standing building, at ground depth, drawing the cast shadow frame for the current sun state.
- **Night:** one overlay sprite per standing building, at its own lot's depth plus a small lift, drawing the lit-window mask.
- **Dusk and dawn:** both, for about a fifth of the cycle, because a low sun and rising window light genuinely coexist and that overlap is the prettiest moment in the day.

**Steady-state cost of the entire building shadow feature: one sprite per building, which the game already pays after dark.** Two for roughly 18% of the cycle.

### 7.3 Bridges

A bridge deck over water throws the most valuable shadow in the game per texel spent: it is the one cue that says the deck is *above* the water rather than painted on it. It bakes into the track composite alongside the deck itself, costs nothing per frame, and its length follows the same two-step sun rule.

### 7.4 Peeps get a dot, not a shadow

A peep is a handful of texels and there can be several hundred. A cast shadow at that scale is two texels of noise multiplied by the population. **Peeps get a two-texel `outline` contact dot baked into their own frame** — enough to seat them on the ground, free, and immune to the sun.

---

## 8. Night

[01 §3.4](01-art-direction.md) floors night at 65% of a fully lit world. Night is legible, not black. Given that floor:

> **Cast shadows end at sundown. Contact shadows never do.**

Three reasons, in order of weight:

1. **The legibility floor is already spent.** Night applies a 35% blue multiply across the whole frame. A second, local darkening on top of it eats the margin the floor exists to protect, in the state that has least of it.
2. **Nobody would see it.** A moonlight cast would be a shade step under a heavy blue wash — a difference smaller than the wash itself.
3. **It would double the night's cost for that nothing.** A moonlight state means a sixth sun position, two more full-map sweeps per day, and a night that is 34% of the cycle stops being the free third of the day it currently is.

**Night is therefore the cheapest state in the system**: every shadow cell resolves to no shadow, and no sun step occurs for a third of the cycle.

### 8.1 Lit windows

Window light is a second sprite layer and not part of the tint ([01 §3.4](01-art-direction.md)). It interacts with shadow in exactly two places:

**Windows own the overlay sprite after dark.** The same per-lot entity that carried the cast shadow through the day carries the lit-window mask through the night, swapping frame, depth and role at dusk. One sprite, two jobs, never idle.

**A lit window spills onto the ground.** The window mask frame gains a few texels of `winLit` at low alpha at the base of the wall below each ground-floor window. This is free — the mask is already baked and already drawn — and it is the correct answer to "what replaces a cast shadow at night": not a fainter shadow but a small pool of warm light doing the opposite job. A district coming on at nightfall is described as the cheapest emotional payoff in the game; this is the cheapest available extension of it.

The two never fight: the spill is warm and additive, the contact shadow is cool and beneath the wall, and they occupy different texels.

---

## 9. What this does to the cliff art

The world's cliff faces are drawn for a low south-western sun. A sun that moves changes some of that art and — importantly — leaves the rest alone. The distinction is worth stating precisely, because someone will otherwise try to "fix" the part that is already right:

> **A cliff face's *depth* is a fact about the camera. Its *value* is a fact about the sun.**

| Property | Governed by | Moves with the sun? |
| --- | --- | --- |
| South face is by far the deepest | The high camera angle. You see the south face of a drop; you cannot see a north face at all. | **No. Never.** |
| North is a thin dark rim, not a face | Same. It is the shadowed break of slope seen from above. | **No.** |
| Face body and crest take the light steps of the rock ramp | The sun | **Yes** |
| Which edges get a lit sun lip | The sun | **Yes** |
| Which edge gets the contour shadow at its foot | The sun | **Yes** |

Concretely: cliff faces gain a lit and an unlit variant per direction; sun lips, which exist today only for the two south-western edges, gain the other two; and the contour shadow at the foot of a band step becomes the near end of a cast rather than a fixed decoration, with a wide variant for a long cast.

### 9.1 Atlas cost

| Family | Now | With shadows | Δ |
| --- | --- | --- | --- |
| Base tiles | 60 | 60 | — |
| Shade steps (grass, hill) | 0 | 6 | +6 |
| Material transitions | 80 | 80 | — |
| Cliff faces (4 dirs x 2 severities x lit/unlit) | 8 | 16 | +8 |
| Cliff corners | 4 | 4 | — |
| Contour shadows (4 dirs x 2 widths) | 4 | 8 | +4 |
| Sun lips (4 dirs instead of 2) | 40 | 64 | +24 |
| Shadow fray | 0 | 64 | +64 |
| **Total cells** | **196** | **~302** | **+106** |

At 32 texels a cell that is about **1.2 MiB of atlas, up from 0.8**, painted once at boot in a fraction of the budget it already fits inside. There is no runtime art generation anywhere in this design.

---

## 10. The budget

### 10.1 Stated in sprites, not draw calls

Sprites sharing a texture batch, so the true draw-call delta of this entire system is **two** — one batch for building shadow overlays, one for train shadows. That number is not the constraint. The constraint is **per-sprite CPU in extract and prepare**, which scales with sprite count, so the budget is written in sprites.

| Item | Sprites | Steady CPU / frame | Memory |
| --- | --- | --- | --- |
| Terrain and bridge casts | **0** | **0** | +0.4 MiB atlas |
| Building casts | +1 per standing building | one atlas index write per lot, 5x/day | +2.5 MiB atlas |
| Train casts | +1 per car | one transform write per car | 0 |
| Props, peeps, contact shadows | **0** | **0** | 0 |
| Sun step sweep | 0 | **~0.11 ms, for one chunk per frame, 5x/day** | 0 |
| **Total** | **+1 per building, +1 per car** | **< 0.05 ms steady** | **~+3 MiB** |

### 10.2 The hard limits

Three, and they are refusals rather than targets:

1. **No sprite per tile, and no sprite per peep.** Either would put this system's cost on the largest count in the game.
2. **No frame may spend more than 0.25 ms on shadows**, including a sun-step sweep frame.
3. **A scene with shadows on must be indistinguishable from the same scene with shadows off in frame time** — no p99 spike above 1 ms attributable to the sun.

### 10.3 What gets cut first, in order

If it does not fit, cut the picture before cutting the information:

| Order | Cut | What is lost | What is saved |
| --- | --- | --- | --- |
| **1** | Building cast shadows | The town stops showing light direction. It keeps its contact shadows and stays seated on the ground. | ~93% of the added sprite count, and 2.5 MiB |
| **2** | The moving sun — freeze at High West | The daily rhythm. The world keeps terrain relief and directional cliff lighting at the one angle the art was drawn for. | Every rebake, forever, and ~0.8 MiB of atlas |
| **3** | Train shadows | Trains sit on the rails rather than over them. | One sprite per car |
| **4** | Cast length, 3 → 2 → 1 tile | Long dawn and dusk shadows. Drop height still reads, more coarsely. | The shaded area and the march, proportionally |
| **5** | Terrain self-shadowing | Everything. This goes last because it is the only part that pays the player back in information. | The feature |

Cut 1 alone brings the system to **zero added sprites and zero steady per-frame cost**, which is worth noticing: the cheapest useful version of this feature is free at runtime, and the expensive part is the part that only makes it look nice.

---

## 11. Phasing

Three phases. The first ships alone and is most of the picture.

### Phase 1 — "the world has relief"

One fixed sun, in the south-west, where the art already points.

- The shade-step operator and the two palette swaps.
- Terrain cast shadows at the foot of every cliff face and band step: the contour at the foot of a step widens from one texel into a hem, and its width reads the drop.
- The fray at every shadow boundary.
- Contact shadows unchanged, and promoted to one named alpha.

**Zero added sprites, zero per-frame cost, no rebakes, no new systems.** A landscape gains relief and drop height becomes readable, which is the majority of what this brief is for. **Shippable on its own, and worth shipping on its own.**

### Phase 2 — "the sun moves"

- Five sun states derived from the day cycle.
- Cliff faces and sun lips for all four directions.
- One chunk per frame on a sun step, nearest camera first.
- The midday legality read: at high sun, only grade-limit faces cast.

**Still zero added sprites.** Cost is ~0.11 ms per frame for a quarter of a second, five times per twelve minutes.

### Phase 3 — "the town has light on it"

- Building cast shadow frames, one bank of five sun states.
- The building overlay layer, merging the day shadow and the night window light into one sprite per lot.
- Ground spill under lit windows at night.
- Bridge shadows on water.
- Train silhouette shadows.

**+1 sprite per building and per car.** This is the phase that is first to be cut, and it should be built last so that cutting it costs nothing already spent.

### Settings

Shadows appear in the display settings as **Off / Static / Moving**, defaulting to Moving. `Static` is Phase 1's behaviour and is the correct setting for a low-end machine; `Off` keeps only contact shadows, because an object that floats is a bug rather than a preference. This slots into the accessibility work rather than needing its own row — a player who has asked for reduced motion gets `Static` automatically, since a sun that steps is motion the player did not cause.

### Neighbour maps

A neighbour's town, seen across the border yard ([12](12-multiplayer.md)), draws with the **local** sun state. Two halves of one frame lit from two directions would read as a compositing error, and a neighbour map's clock is not a thing this game promises to synchronise.

---

## 11.5 One cost this brief understates

The palette swap in §3 is written as "two colours in, two out", which is true of
the *cap* and not of the work. `sandL` and `waterL` are live in twenty places
across terrain material ramps, the terrain atlas, water shimmer, building art,
peep sprites and the New Map preview. Removing them is a small refactor with a
visual review attached, not a line in `palette.rs`.

That does not change the recommendation — the cap is real and something has to
go — but it should be costed with the phase that needs the new shade, not
assumed free. If the refactor looks unappealing when the time comes, the honest
alternative is to ship Phase 1 using each material's existing bottom step as its
shade and accept that the darkest materials do not shadow, which is already the
rule this brief sets for materials with no step below.

## 12. Rejected, and why

| Option | Verdict |
| --- | --- |
| **A continuously rotating sun** | Asks for a full-map chunk rebake every frame — 1.7 ms and sixteen megabytes on a standard map, which is exactly the regression that has already cost this project its frame rate once. And it buys nothing: a shadow edge advancing sub-texel either does not move or *crawls*, and crawling is what the texel snap exists to forbid. |
| **Soft shadows, penumbra, any blur** | [01 §2](01-art-direction.md), directly. Not available. |
| **Real-time shadow mapping or a lighting shader** | Needs a custom material on every sprite, samples off the texel grid, and produces colours outside the palette. It is also the wrong tool for a world with one light and five states — it solves a general problem this game does not have. |
| **An alpha wash quad per shadow region** | Produces an off-ramp colour at every shadow boundary. Across 45 colours and any number of shadow states, the realised palette becomes hundreds of values — invisible in one screenshot, corrosive over a project. A wash heavy enough to read also collapses a material's dark and mid steps into the same mud. |
| **One dedicated shadow colour, drawn opaque** | `outline` over grass reads as a hole in the ground, not as shade. And no single colour can serve a grass shadow and a sand shadow without one of them looking like a different material. |
| **Ordered dither as the shadow's interior** | Right technique, wrong place. World-anchored Bayer is exactly how this project stops decoration boiling under scroll, but a 25–50% stipple field covering a fifth of the screen is noise, and [01 §1](01-art-direction.md) spends the game's whole contrast budget on track. Kept for one texel at the fray, and nowhere else. |
| **A separate full-resolution shadow layer, one sprite per chunk** | Decouples the shadow bake from the terrain bake, and costs +16 MiB of texture and +16 sprites to avoid a rebake that is 0.11 ms per frame for a quarter of a second, five times per twelve minutes. The rebake is cheaper than avoiding it. |
| **Physically correct cast length** | At dawn a three-storey block throws forty texels. At four lots to a tile, a terrace of them mats the district solid and destroys the density read that town art exists to deliver. Stylised short is the only version that survives the density this game draws at. |
| **Shadows on the railhead** | [01 §3.3](01-art-direction.md) rule 2. `railS` on `ballastD` is the widest value gap in the palette and it belongs to the railhead alone. The shade operator cannot violate this, which is a point in the operator's favour rather than a rule needing enforcement. |
| **Shadows from peeps** | Several hundred casters, three texels each. A contact dot does the same job for nothing. |
| **Moonlight shadows** | A sixth state and two more full-map sweeps per day, to produce a shade step under a 35% blue multiply that the legibility floor will not let anyone see. |
| **A pure-south noon sun** | Casts straight north, behind the drop that made it, where the camera cannot see it. The midday read comes from the two diagonals at their shortest instead. |
| **A northern sun at any hour** | Lights every face the high camera cannot see and shades every face it can. There is no hour at which this world looks better from behind. |
| **Eight or sixteen azimuths** | The track rose is sixteen because junction geometry demands it. Shadows have no such wall — four azimuths already give every edge of a square tile a lit and an unlit state, and each extra azimuth is another full-map sweep per day for a difference measured in one texel of hem. |
| **Shadows in Map View** | Map View is a schematic read at four texels per tile. Shade at that scale is indistinguishable from an elevation band, and it would corrupt the one view whose entire job is unambiguous terrain silhouette. |

---

## 13. Acceptance bar

Shadows have landed when a player who has never seen the game can, from **a single still screenshot at 2x**:

1. Say which way the light is coming from, without being told there is a sun.
2. Tell a tall ridge from a low one by how far its shade reaches.
3. Still trace a rail line across the frame and see where it forks, with a shadow crossing it.
4. Point at a building and say which side is the sunny side.
5. Still tell land from water from hills from mountain with no legend — [01 §9](01-art-direction.md) item 1, unchanged.

And over **one twelve-minute day**:

6. Notice that the shadows have moved, without ever having seen them move.
7. Never see a dark patch of ground whose cause is not visible in the same frame.
8. See the town go dark and come alight without anything appearing to hover.

And **measurably**:

9. At every sun state, on every terrain preset, at most a quarter of visible land is in shade.
10. Frame time with shadows on is indistinguishable from frame time with shadows off: under 0.25 ms in any frame, and no p99 spike above 1 ms attributable to a sun step.
11. Sprite count grows by at most one per standing building and one per train car, and by nothing per tile or per peep.

Item 7 is the one to watch. It is the difference between a lit world and a muddy one, and it is the only item here that no amount of tuning will recover once the length cap is relaxed.
