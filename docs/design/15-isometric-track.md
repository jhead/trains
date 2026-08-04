# 15 — Isometric Track

**Status: feature design, subordinate to [01 — Art Direction](01-art-direction.md) and [04 — Building & Tools](04-building-and-tools.md).** Where this brief and 01 disagree, 01 wins: the pixel contract (§2), the cross-section line weights (§5.3) and the "direction is a different sprite, never a rotation" rule (§2.2) are constraints here, not inputs.

**The premise:** the isometric view gives every tile an elevation and draws terraces to prove it. Track does not yet know. Two tiles one height apart carry a railway that stops 4 px short of the ground and starts again 4 px higher — a step in the middle of a rail, which is the one thing a railway may never have.

The whole brief turns on one sentence, and it is the owner's:

> **Looking at two tiles with 1 height difference, I would expect the track to show a connected incline/decline between them.**

Everything below exists to make that true, and to make "connected" a thing a test can measure rather than a thing a person squints at.

---

## 1. The look in one sentence

**A railway lies on the ground. Where the ground steps, the railway ramps — and the ramp is drawn between the two tile centres, not between the two terraces.**

Terrain in isometric is deliberately terraced ([02 §2.3](02-world-and-terrain.md): elevation resolves into discrete bands with visible steps, because a soft gradient communicates nothing). Track is the opposite: it is the one thing in the world that must be *continuous*, because a train runs along it and the player's eye follows it for a hundred tiles looking for where it goes.

Those two facts do not fight. They compose, and the composition is the whole look:

- The **ground** is a staircase. It reads its own height by counting terraces.
- The **railway** is a smooth polyline through the tile centres. It reads its own gradient by not being level.
- Where they differ, the railway wins locally and pays for it in **earthworks** — an embankment where it runs above its own tile, a cutting where it runs below.

That is what a railway crossing terraced ground looks like, and it is why a real railway is legible in a landscape: it is the only line in the frame that refuses to follow the contour exactly.

---

## 2. The geometry, and why it is not negotiable

### 2.1 The projection is affine in height

`rail_map::coords` puts a tile at:

```text
sx = gx - gy
sy = (gx + gy) / 2 + h · ISO_LIFT       ISO_LIFT = 4
```

This is **linear in all three of `gx`, `gy`, `h`**. The consequence is the single most useful fact in this brief:

> The projection of a straight line in `(gx, gy, h)` is a straight line on screen.

So a ramp between two tile centres is not a curve to be approximated, an art asset to be nudged, or a special case. It is the straight screen segment from `tile_to_world(a)` to `tile_to_world(b)`, and drawing it correctly is drawing a straight line.

### 2.2 The midpoint always lands on a texel

Each of the two pieces draws its own half of the leg, meeting at the link midpoint. For a direction step `(dx, dy)` the half-link screen offset is:

```text
(16 · (dx − dy),  8 · (dx + dy))  +  (0, 2 · Δh)
```

Every one of the sixteen `DIR16` steps has integer `dx, dy`, and `Δh` is an integer, so **the midpoint is always a whole number of texels from both tile centres**. Neither half has to round it, so neither half can round it differently. The joint is exact by construction rather than by tolerance — which is what lets §5 assert equality rather than "within a pixel".

### 2.3 Which legs may climb

All sixteen. This is not a simplification; it is what the sim already permits, and the drawing has to cover exactly what placement allows or the player sees track they cannot build or builds track they cannot see.

| Rule | Where | What it permits |
| --- | --- | --- |
| `grade_to_neighbors_ok` | every **linked** direction, all 16 | `|Δh| ≤ MAX_GRADE` (4) |
| `path_grades_ok` | every consecutive pair on a route | `|Δh| ≤ MAX_GRADE` (4) |
| both | any leg touching **water** | grade check skipped entirely |

Two things fall out of the table:

- **Half-steps climb too.** A knight's move is measured endpoint to endpoint, so a `(2, 1)` leg may rise 4 over its √5 tiles exactly as a `(1, 0)` leg may rise 4 over its one. The drawing gets no say.
- **A leg to water may exceed 4.** The projection reads water at its *surface* (height 0), not its bed, so a bank at height 5 beside a river draws a 20 px drop onto the deck. The grade code therefore carries a real `i8` per direction and clamps nothing.

The height a leg reads is `rail_map::tile_height`, the same field `tile_to_world` lifts by — never `TrackPiece::height`, which is the raw terrain height and disagrees with the projection over water. Reading the field the projection reads is what makes the endpoints *equal* instead of nearly equal.

---

## 3. What an incline is made of

### 3.1 The ramp

Every painter walks a leg as `along · t + across · s`, in ground units projected onto the screen. The ramp adds one term:

```text
lift(t) = (t / reach) · Δh · ISO_LIFT / 2
```

added to screen y, for every texel of the leg — bed, sleepers, rail bodies, railheads, polish. Because the lift depends only on `t`, the whole cross-section at a given point along the run rises together: the sleepers stay square to the rails and the bed stays 8 either side. The track surface is a ruled plane, which is what a ramp is.

At `t = 0` the lift is zero (the tile centre, where `tile_to_world` already put the sprite). At `t = reach` it is `Δh · 2` px — exactly half the step — and the far piece's own leg arrives at the same point from the other side. **One expression, applied in one place, and every layer is in register because they all walk through it.**

### 3.2 Sleeper spacing on the slope

Sleepers keep their **ground-plane** pitch on a ramp. They are not re-spaced along the true incline.

This is a deliberate refusal, not an oversight. "True incline length" is not a quantity this projection has: height enters only as screen pixels (`ISO_LIFT`), never as a ground-plane distance, so there is no honest ratio to take a hypotenuse with. Inventing one would buy a sub-texel correction at `MAX_GRADE` and would cost the phase lock in §4, which is worth far more.

### 3.3 The embankment

A climbing leg runs above its own tile's surface — by up to `Δh · 2` px at the boundary, 2 px for the single step this brief is named after and 8 px at the steepest legal climb. Left alone, the bed floats and a sliver of ground shows under it.

So the ascending half of every leg draws a **skirt**: every bed texel fills straight down to where the same texel would have sat unlifted. The fill is `ballastD` with an `outline` texel on its bottom row — the game's one shadow key, used exactly as the terrain cliff faces use it ([13 §3.1](13-shadows.md)). The bed's own `ballastL` sun edge stays on top, so the bank reads as a lit crest over a shaded flank at any height from two texels upward.

**The descending half gets nothing**, and that is the correct asymmetry. A leg running below its own tile's surface is a *cutting*: the track simply draws over the ground it is notched into, because track sorts above terrain within a row. Drawing a skirt there would be drawing earth in mid-air.

Composite the two halves of a one-step climb and you get the picture the brief opened with: a small embankment on the low tile, a shallow cutting in the high one, and a rail that crosses the terrace at half its height without a break.

### 3.4 The rail on a 2:1 staircase

One ground unit across a leg projects to **1.118 screen texels**, and the run itself is a 2:1 staircase. A three-texel cross-section — `railD` shadow, `railL` head, `railM` body, measured across the ground — therefore cannot survive: the head painted half a step further along the run rounds onto the very texel the flank wanted, and the flanks come back as speckle rather than as line.

The discipline that does survive is stated in screen texels, not ground units:

| | Top-down | Isometric |
| --- | --- | --- |
| Railhead | 1 texel `railL` at gauge | 1 texel `railL` at gauge |
| Rail body | `railD` / `railM`, ±1 **across the ground** | 1 texel `railD`, one step **across the screen** |

The isometric shadow is offset by one whole texel in the screen direction perpendicular to the run, on the side away from the light — right for a leg that climbs the screen, down for one that crosses it. It is a single, unambiguous texel per rail per sample, so it cannot alias, and it keeps [01 §5.3](01-art-direction.md)'s claim that a rail has a shadow side while dropping the half of the cross-section the lattice cannot hold.

**Two texels of rail, and both of them are lines.** That is the bar: a 1-unit-wide rail is readable on a 2:1 staircase when every texel it owns is chosen in screen space and painted exactly once.

---

## 4. Connected, measurably

The word has to mean something a test can fail. It means this:

> **For every legal leg between adjacent track tiles `a` and `b`, the drawn centreline of the run is the straight screen segment from `tile_to_world(a)` to `tile_to_world(b)`, correct to within half a texel at every point, and the sleeper pitch is constant across the boundary.**

Four clauses, each independently checkable:

1. **No gap.** Sampling the segment end to end, every sample lands on an opaque texel of one of the two cells.
2. **No jog.** The two half-legs' centrelines end on the *same texel* — the midpoint of §2.2, asserted as equality, not as a tolerance.
3. **No lift discontinuity.** The drawn centreline's screen y is affine along the run: one slope, through the boundary, with no step at it.
4. **No stutter.** Consecutive sleepers are the same distance apart everywhere on a straight run, including across the tile boundary.

Clause 4 is the one the flat view was already failing. Each cell bakes independently and starts its sleeper ladder at its own tile centre with a pitch of 4, but a diagonal link is 45.25 texels long and a half-step link is 71.55 — neither a whole number of sleepers. The measured result is a pitch that runs 4, 4, 4 and then **6 to 7 texels at every boundary**: a visible stutter down every diagonal run in the game.

**The fix is to fit the pitch to the link, not to the tile.** For a leg in direction `d`:

```text
n     = round(link_length(d) / 4)      whole sleepers per link
pitch = link_length(d) / n
```

| Leg | Link length | Sleepers | Pitch |
| --- | --- | --- | --- |
| Orthogonal | 32.00 | 8 | 4.000 |
| Diagonal | 45.25 | 11 | 4.114 |
| Half-step | 71.55 | 18 | 3.975 |

Anchored at the tile centre, both halves of a link now compute the same ladder from opposite ends, and the ladder is even across the whole run. Where `n` is even the two halves paint one coincident sleeper at the boundary — the same texels, so it is invisible. Where `n` is odd the boundary falls cleanly between two sleepers.

The pitch never moves more than 3% off the brief's 4, which is under half a texel over a whole link and far under the cost of the stutter it removes.

**This applies in isometric only.** Top-down has the identical latent stutter and is deliberately left alone: it is the shipping view, this brief is the isometric one, and its cells are pinned byte-for-byte by a golden test so the transplant can be made later as its own decision.

One thing clause 4 does **not** cover: the `tieM`/`tieD` alternation is anchored per cell, so across an odd-`n` joint two adjacent sleepers can take the same tone. Those two browns are four values apart on a ballast bed. It is decoration noise, well below the [02 §2.3](02-world-and-terrain.md) legibility bar, and pinning it would mean dropping the world-hashed variant from the tie colour to buy nothing.

---

## 5. The ghost climbs too

[04 §2.2](04-building-and-tools.md) is already explicit: *"It is the **actual** track art at 55% opacity tinted `hi`, not an abstract line — the player sees what they will get."*

That contract earns its keep the moment gradient exists. Deciding whether to climb is the most consequential thing the build tool asks — it costs more, it caps train speed, and it is the difference between a route that works and one that is a scar. A player deciding it while looking at a flat bar has been given the cost and denied the picture.

So in isometric, a ghost tile that would place track draws **the cell the placed piece will draw**, ramps and all, tinted and translucent. Same bake, same bank, same key: ghost and placed art are the same asset, so they cannot drift.

The links it draws are the links the piece *will have* — existing neighbours plus the rest of the proposed route — derived through the same rule the network uses (a compass step always links; a half-step links only while both tiles it crosses are clear), evaluated against occupancy that counts the route as already built.

Top-down keeps the bar it has. That is not a compromise for isometric's benefit: from above there is no ramp to show, the bar is what shipped, and this brief has no business moving it.

---

## 6. What this costs

**Bake count.** A cell is keyed on its link mask, and now also on the height delta of each linked leg. Flat track has an all-zero grade key, so a level network bakes exactly the vocabulary it baked before — the cache only widens where the ground actually moves, and it widens by the number of *distinct climbing configurations the player has built*, not by anything combinatorial.

**Bake cost.** Unchanged per cell. The skirt adds a short vertical fill under bed texels on ascending legs only; the ramp is one multiply-add inside a walk that was already running.

**Frame cost.** Zero. Nothing here runs per frame: cells are baked on edit and cached for the session, and a full track rebuild — a load, a new map, a projection flip — stays one bake per new key plus one sprite per piece, which is the same shape and the same millisecond class as the flip costs today.

**Palette.** No new colours. `ballastD` and `outline` are both already in the world.

---

## 7. The implementation choice, and the one it beat

Two ways to draw a ramp:

**A. Per-leg ramp painting, lift interpolated along the leg.** The leg's height delta joins the bake key; every painter adds `lift(t)` to screen y. *(Chosen.)*

**B. Baked incline sprite variants per climbing bearing.** Author or generate a distinct sprite for each (bearing, grade) and select it the way direction is already selected.

B is the more orthodox pixel-art answer, and on a fixed-camera game with hand-drawn track it would probably be right. It loses here on four counts:

- **It is the same cache with a bigger vocabulary.** A junction is a composite of legs, so an inclined leg still has to composite with flat legs in one cell. B does not replace the key — it grows it by exactly the same axis A does, and then adds an asset pipeline on top.
- **It cannot be checked.** A's ramp is derived from `tile_to_world`, so "the endpoints are the tile centres" is a theorem the test asserts as equality. B's ramp is drawn by hand or by a generator with its own idea of the slope, and the best a test can do is measure the drawn art against a tolerance and hope.
- **It multiplies by nine, not by two.** `MAX_GRADE` is 4 and water legs exceed it, so "climbing bearing" is 16 bearings × ~11 deltas of authored art before junctions are considered.
- **It cannot ride the seam.** A already inherits every future change to the cross-section for free, because it is a term inside the existing walk. B forks the art: every line-weight change has to be made twice, and the flat and inclined versions drift.

**What A gives up** is the thing B is genuinely better at: an artist's hand on the ramp. B could taper the ballast into the bank, foreshorten the sleepers as the grade steepens, and put a different tie spacing on a 4-step climb than on a 1-step. A gets a mathematically exact ramp with a procedurally uniform cross-section, and it will always look slightly more *drafted* than *drawn*.

That is the right trade for this game at this stage. The bar is "connected", the projection makes connected provable, and a provable ramp today beats a prettier one that the tests can only squint at. If the railway later earns hand-drawn art, A's skirt and lift are the specification that art would be drawn against.

---

## 8. Acceptance bar

1. Two adjacent track tiles one height apart show one continuous rail from centre to centre, in all sixteen directions, up and down.
2. The joint is the same texel from both sides — asserted as equality, not tolerance.
3. Sleeper pitch is constant along a straight run, through every tile boundary, in every direction.
4. The build ghost shows the ramp before the player commits to climbing it.
5. Top-down is byte-identical. Not "looks the same" — byte-identical, and pinned.
6. A flat network bakes the same number of cells it baked before.

---

## 9. Deliberately not in this brief

- **Half-step legs ignore the ground under them.** A knight's move crosses two tiles that carry no track, and the ramp is drawn straight from endpoint to endpoint regardless of what those two tiles are doing. The sim grades it the same way, so the drawing and the rule agree — but a shallow link over a bump will pass through the bump. Fixing it means either grading the intermediates (a sim change) or bending the ramp (giving up the affine argument in §2.1). Neither belongs here.
- **Tunnels and cuttings as terrain edits.** A cutting here is a drawing, not an excavation: the ground is unchanged and the track is simply below it. Real earthworks are a terrain-modification feature and a different brief.
- **Grade in the cross-section.** A steep leg draws the same rail as a level one. Sleepers packed tighter on a climb, or a heavier bed under one, is legibility the game may want later once gradient is something the player tunes rather than accepts.
- **The top-down sleeper stutter.** Real, measured, identical in cause, and out of scope by mandate. §4 is the fix when someone decides to spend it.
