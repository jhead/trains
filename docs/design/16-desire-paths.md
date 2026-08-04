# 16 — Desire Paths

**Status: feature design, subordinate to [01 — Art Direction](01-art-direction.md) and [06 — Town & Peeps](06-town-and-peeps.md).** Where this brief and 01 disagree, 01 wins — the pixel contract (§2), the palette cap (§3) and the value ladder that makes elevation readable (§3.3, and [02 §2.3](02-world-and-terrain.md)) are constraints on everything below, not inputs to be negotiated.

**The premise:** the town already has habits. Residents leave the same front door at the same hour, walk the same lane to the same platform, and come back the same way ([06 §4.2](06-town-and-peeps.md)). None of that leaves a mark. The ground the whole town crosses twice a day looks exactly like the ground nobody has ever stood on.

A desire path is the ground admitting it has been used.

> **The player should be able to look at a field and tell, without a single number, where the town walks.**

That is the entire feature. It is a *readout*, not a mechanic — and §6 is about keeping it that way.

---

## 1. The look in one sentence

**Ground that people repeatedly cross wears through the grass into bare earth, in three visible steps, and grasses back over when they stop.**

Three properties, and every decision below is tested against all three:

- **Earned.** One person walking somewhere once leaves nothing. A path is the *residue of a habit*, and the threshold to the first visible step is deliberately several days of one commuter's traffic. A world where a single stroll paints a line is a world where the ground is noise.
- **Warm, not bright.** The path shifts **hue** and barely moves **value**. This is not an aesthetic preference; it is a hard constraint, and §3.1 explains why breaking it would silently lie about elevation.
- **Slow to forget.** Grass regrows at a fraction of the rate it is worn away. A route abandoned this morning is still faintly legible a fortnight later. The world's memory is the point — "transient" here means *decades of tile-time*, not *seconds*.

---

## 2. What wear is made of

### 2.1 The unit is a footfall, not a frame and not a tick

**One footfall = one walkable tile entered by a peep who is walking.**

Not a tick of standing on it (a peep occupies a tile for [`WALK_TICKS_PER_TILE`] = 24 ticks, which would make wear a function of walking *speed*), and emphatically not a rendered frame. A peep crossing a six-tile lane deposits exactly six footfalls, once, whatever the speed multiplier is set to and whatever the frame rate is.

That definition falls out of the sim's own movement model: a walk is a route over walkable tiles, advanced one waypoint at a time on the fixed tick (`rail_sim::peeps::walk`). Wear observes the tile a walking peep stands on and records a footfall when it changes. Nothing in the wear pass reads a camera, a transform, a delta time or a float.

### 2.2 The numbers

Wear is a `u16` per tile, saturating, and every rate below is an integer.

| Constant | Value | In words |
| --- | --- | --- |
| `WEAR_PER_FOOTFALL` | 64 | What one crossing deposits |
| `WEAR_MAX` | 1200 | Saturation — about 19 crossings of headroom above Bare |
| `WEAR_FAINT` | 256 | Level 1 threshold — 4 footfalls |
| `WEAR_WORN` | 640 | Level 2 threshold — 10 footfalls |
| `WEAR_BARE` | 1024 | Level 3 threshold — 16 footfalls |
| `WEAR_RELEASE` | 32 | Hysteresis band below each threshold (§4.3) |
| `REGROWTH_INTERVAL_TICKS` | 288 | 48 sim-minutes — 30 regrowth steps per sim-day |
| `REGROWTH_PER_STEP` | 2 | **60 per sim-day** |

The time base these sit on is fixed and worth stating once: 64 Hz `FixedUpdate`, `SIM_SECONDS_PER_TICK` = 10, so **`TICKS_PER_DAY` = 8,640 and one sim-day is 2.25 real minutes**.

### 2.3 What those numbers do

A tile crossed `C` times per sim-day nets `64C − 60` per day.

| Traffic | Net / sim-day | To Faint | To Worn | To Bare |
| --- | --- | --- | --- | --- |
| One crossing a day | +4 | never, in practice | — | — |
| **One commuter** (out and back) | +68 | **3.8 days** | 9.4 days | 15.1 days |
| Two commuters | +196 | 1.3 days | 3.3 days | 5.2 days |
| **A station approach** (five commuters) | +580 | 0.4 days | 1.1 days | **1.8 days** |
| A trunk lane (ten commuters) | +1220 | 0.2 days | 0.5 days | 0.8 days |

And the two bars the feature was asked to clear:

- **One stroll never paints.** A single crossing is 64, which is a quarter of the Faint threshold. Three separate strollers across a season still never reach it, because 60/day of regrowth eats a stray footfall inside a single sim-day.
- **A commuting route reads within a few sim-days.** A lone commuter's lane is visible on day four; a lane shared by a district's worth of commuters is visible before the first day is out and fully bare by the second.

Note the shape this produces, because it is the thing that makes it look right rather than merely work: **the trunk wears first and deepest, the capillaries stay faint.** Every resident of a district converges on the same few tiles outside the station, so those tiles saturate while the individual garden paths that feed them hover around Faint. That is exactly what a real desire-path network looks like from above, and it is a free consequence of counting footfalls rather than routes.

### 2.4 Forgetting

Regrowth runs on a tick schedule — every 288 ticks, subtract 2 from every worn tile — never per frame and never in floating point.

From full saturation, with all traffic stopped:

| | Sim-days | Real minutes |
| --- | --- | --- |
| Drops out of Bare | 2.9 | 6.6 |
| Drops out of Worn | 9.3 | 21 |
| Drops out of Faint | 15.7 | 35 |
| Clean ground again | 20.0 | 45 |

The asymmetry is deliberate and is the whole emotional content of the feature. A path you stop using **loses its deepest read quickly** — the bare earth greens over within a few minutes of play, so the world visibly responds to a rerouted line. But **the ghost lingers**: a faint scar sits in the grass for the better part of an hour of play, long after the station that caused it was demolished. The world remembers where the town used to walk.

---

## 3. What it looks like

### 3.1 The value rule — the one that must not be broken

Brief 01 §3.3 and [02 §2.3](02-world-and-terrain.md) between them establish that **lightness carries elevation**. The realised band ladder climbs 23.5 · 35.2 · 36.0 · 47.4 L\*, roughly 11 L\* per legible band, and the game's readability rests on it: a player traces a cheap route across a map by reading value.

A bare-earth path drawn the obvious way — a dusty ochre on dark grass — lands at 39 L\* on ground that fills at 23.5. That is **a band and a half of apparent elevation, painted onto flat ground, by a feature that has nothing whatever to do with height.** It would put a phantom ridge along every lane in town.

So:

> **A path may shift hue as far as it likes. Its value must stay within 7 L\* of the ground it lies on** — under two-thirds of the ~11.5 L\* the ladder spends on a legible band, and never enough to promote a tile a whole band.

The palette makes this mostly easy, which is a happy accident worth recording. The `TIE` ramp is very nearly a warm twin of the grass and hill ramps rung for rung:

| Ground fill | Band | L\* | Path fill | L\* | Δ |
| --- | --- | --- | --- | --- | --- |
| `GRASS_D` | 0 | 23.5 | `TIE_M` | 28.2 | +4.7 |
| `GRASS_M` | 1 | 35.2 | `TIE_L` | 39.5 | +4.3 |
| `HILL_M` | 2 | 36.0 | `TIE_L` | 39.5 | +3.5 |
| `HILL_L` | 3 | 47.4 | `SAND_M` | 53.5 | +6.1 |

The last rung is the tight one, and hiding it would be worse than admitting it: **the palette holds no warm tone within 6 L\* of `HILL_L`.** `SAND_M` at +6.1 is the closest that exists — a little over half a band step, under two-thirds of one. It is accepted for three reasons. The high hill band is a minority of a calm map ([02 §2.3](02-world-and-terrain.md): elevation is "a few deliberate features", not texture). A peep pays `WALK_CLIMB_COST` to climb, so routes prefer the flat and paths up there are rare to begin with. And the hue swing green → sand is far too large for the mark to read as anything but a change of *material*. The two bands that carry most of every map stay inside 5 L\*, and are held to that by a tighter test.

Better still, the pairing collapses to a pure function of the ground's own fill shade — the four realised walkable fills use shades 0, 1, 1, 2 — so there is no material table at all:

```
PATH_FILL = [TIE_M, TIE_L, SAND_M]     // indexed by the ground's fill shade
PATH_DUST = [TIE_L, SAND_M, SAND_L]    // the sparse light speckle, one rung up
```

**The path is a fourth ramp, and it climbs with the ground it lies on.**

### 3.2 Where a path can be drawn at all

Only where there is grass to wear away.

- **Grass and Hill** — the two materials a peep can actually walk on below the mountain band ([`WALK_MAX_HEIGHT`] = `MOUNTAIN_HEIGHT_MIN` = 14). These get paths.
- **Sand** — beach is already bare earth. There is nothing to wear barer, and a brown patch on a brown beach is invisible work.
- **Rock** — the mountain band is impassable on foot, so no footfall can ever land there.
- **Water** — impassable, except on a bridge deck, which is a built structure and not ground. **A bridge never wears.** Wooden decking with a mud path worn into it is not a thing, and the deck art belongs to the track slice.

Wear is *accumulated* on any walkable tile, but *drawn* only on Grass and Hill. That separation matters: a tile that is terraformed from beach to grassland later will show the path its traffic has already earned.

### 3.3 The three steps

Wear is quantised to four visual states. Coverage grows and the edge stays ragged; the tone does not change between levels, because a path that got *lighter* as it deepened would climb the value ladder it is forbidden to touch.

| Level | Wear | Centre | Over the tile | Reads as |
| --- | --- | --- | --- | --- |
| **Clean** | < 256 | 0% | 0% | Nothing is drawn. Not a faint tint, not an alpha wash — nothing. |
| **Faint** | 256 | 40% | ~30% | Scattered earth showing through thinning grass. At 1× you notice it on a run of tiles, not on one. |
| **Worn** | 640 | 80% | ~60% | A broken earth ribbon with grass surviving at the edges. Unmistakably a path. |
| **Bare** | 1024 | 100% | ~76% | Trodden earth, tufts clinging at the rim, sparse dust speckle. |

Coverage over the whole tile is lower than at the centre because the rim thins, and that gap is deliberate: **even at its deepest a path is never a repainted tile**, which is what keeps it ground rather than a road.

Two rules govern the mask itself:

**The coverage is world-anchored and thins toward the tile's rim.** Which texels are earth is chosen by the same world hash the terrain's own grain uses ([01 §6.2.3]) — the mark belongs to the ground, not to the screen — and biased toward the tile's centre, so a chain of worn tiles reads as one continuous ribbon threading through tile centres rather than as a row of independent blobs. The bias does the work of adjacency joining at none of its cost.

The norm that measures "toward the rim" has to be **the shape of the tile**, and this is worth recording because the first attempt got it wrong and the picture said so. A radial falloff barely bites inside a 64 × 32 diamond at all — the diamond fills only half its bounding box, so the corners where a radial term is strongest lie outside the tile entirely — and a run of Bare tiles came out as a chain of hard-edged diamonds, reading as *tiles* rather than as a path. The norm is Manhattan instead: `|dx| + |dy| = 1` is exactly the rim of a 2:1 diamond, and the edge midpoint of a square. One norm, both geometries, full strength at each tile's own boundary.

**The edge is never a straight line.** Coverage is a scatter, so the boundary between path and grass is ragged by construction. This is the pixel-art way to draw a soft edge without an alpha ramp, and it satisfies 01 §2's hard-edge rule: every texel is either grass or earth, and the softness is in the *distribution*, not in the blending.

Four mask variants per level, world-hashed, so a long lane does not repeat its scatter tile after tile.

### 3.4 Both projections

**Top-down** ([`chunk`](../../rail_town/src/map/terrain/chunk.rs)) composites the path as one more alpha-tested overlay cell, stamped after the autotile layers. It is a 32 × 32 cell like every other, from the same baked atlas, and it costs a chunk re-composite — which is exactly why the level quantisation exists (§4.3).

**Isometric** ([`iso`](../../rail_town/src/map/terrain/iso.rs)) draws the path as a **separate diamond sprite** over the tile's own, from the same atlas every terrain sprite samples, so it batches into the same draw call. A separate sprite rather than a re-baked tile cell for one reason: the isometric renderer redraws the entire map when terrain changes, and a wear level changing on one tile must not cost a full-map respawn. One sprite appears, changes its atlas rect, or vanishes.

Neither renderer draws a path on a cliff **face**. A face is a vertical wall; a path lies on the ground.

---

## 4. What it costs

### 4.1 The budget

| Pass | When | Work | Budget | Measured |
| --- | --- | --- | --- | --- |
| Footfall accumulation | every sim tick | one tile compare per walking peep, capped by `MAX_DETAILED_PEEPS` = 64 | < 500 µs / tick | **4.4 µs** |
| Regrowth | every 288 ticks | one decrement per *worn* tile, in sorted index order | < 4 ms / step | **136 µs** over 6,400 worn tiles |
| Presentation | every frame | drain a transition list; usually empty | zero when nothing changed | **zero** |

Measured on an M4 Max in release, by `a_busy_towns_wear_pass_costs_microseconds`. The budgets are set an order of magnitude above the measurements on purpose: they exist to catch somebody scanning the whole map every tick or re-sorting the worn set into a fresh allocation, not to benchmark a laptop under load. Regrowth runs once every 288 ticks, so its amortised cost is **under half a microsecond a tick**.

The two things those numbers rest on: wear is accumulated per *walking peep* rather than per tile, and regrowth walks a sorted index of the tiles that carry any wear rather than the map. Neither cost scales with map size.

The third row is the one with history behind it. This repo has already paid once for a renderer keyed on "did anybody write this resource" rather than "did the picture change" — the border slice's portal mirror cost a full-map re-composite and a sixteen-megabyte texture upload *every frame* ([`chunk::TerrainDirty`](../../rail_town/src/map/terrain/chunk.rs) carries the scar tissue). Wear changes on some tile almost every tick in a living town. Keyed naively, this feature would reintroduce that exact regression, at a level of continuous churn no amount of profiling would let anyone ship.

### 4.2 So presentation never sees wear

Presentation is not allowed to read the wear number at all. The sim publishes **level transitions** — "tile (14, 22) went from Worn to Bare" — and nothing else crosses the boundary. A tile whose wear climbs from 700 to 701 produces no event, no dirty flag, no work of any kind.

### 4.3 Quantisation, with hysteresis

A level rises **exactly at its threshold**: 256, 640, 1024. No fudge, no smoothing, no interpolation.

A level falls only once wear drops `WEAR_RELEASE` = 32 **below** the threshold it entered at. Without that band, a tile parked on a boundary would flip level every time a footfall landed and every time regrowth ticked it back — a re-composite each way, forever, on every boundary tile in town.

With it, falling out of a level costs 16 regrowth steps, which is 4,608 ticks, which is **half a sim-day**. A boundary tile therefore transitions at most a couple of times a sim-day — roughly one chunk re-composite per real minute across a whole town's worth of boundary tiles. That is not a budget item.

The asymmetry is honest and worth stating plainly: **wear is crisp, forgetting is sticky.** It is the same asymmetry §2.4 gives the rates, expressed one layer up.

---

## 5. Where wear comes from, and what that means

Wear is accumulated from the movement of peeps the sim is running **in full detail** — the bounded set of at most 64 that [06 §4.1](06-town-and-peeps.md) biases toward wherever the camera is looking. The abstracted majority have no position at all; they are district-level flow, and there is no tile for them to tread on.

This has a consequence that should be named rather than discovered:

> **Paths form where the player has been watching.**

Two honest readings of that, and both are true. It is a *limitation*: a district you never visit will not develop the path network its traffic deserves, and two players with identical worlds who looked at different corners will not have identical ground. And it is a *feature*: the ground accumulates a record of where the game was actually played, which is a rather lovely thing for a world to do, and paths appear exactly where the player is positioned to notice them appear.

It does not compromise determinism in the sense that matters. Wear derives entirely from sim-side state on the fixed tick — tile coordinates, integer counters, a tick-scheduled decay, sorted iteration throughout. Given the same inputs, including the same viewport stream, the same world reproduces exactly; and headless (no viewport published) the detail set falls back to a stable id ordering, so tests and any dedicated server are fully deterministic. What it means is that the viewport is *an input to the sim*, which was already true of moods, complaints and Town Talk before this feature existed.

**Future work, deliberately not in v1:** deposit wear along the *planned route* of abstracted peeps' journeys at departure, amortised through the existing per-tick route budget. That would make the path network a property of the town's routines rather than of the player's attention. It is a strictly better model and a considerably larger one — it needs route planning for peeps that currently have no position, and it wants its own answer to what a route costs when nobody is looking.

---

## 6. What this deliberately does not do

**v1 paths are mechanically inert paint.** Every one of the following is a deliberate omission, not an oversight:

- **No speed bonus.** Walking a bare path is exactly as fast as walking through grass. `WALK_STEP_COST` does not know paths exist.
- **No routing influence.** The walk pathfinder does not prefer a worn tile. There is no cost discount, no tiebreak, no attraction term.
- **No feedback loop.** Consequently there is none of the runaway the two above would create together, where the first route to wear in becomes the route everyone takes forever.
- **No economy, no maintenance, no mood.** A path costs nothing, earns nothing, and nobody is happier for walking on one.
- **No build interaction.** Paths do not block, discount or influence track and station placement, and laying track over a worn tile neither erases the wear nor is affected by it.
- **No player agency.** Paths cannot be drawn, cleared, paved or preserved. The only way to make one is to have somewhere people want to go.

The reason for the whole list is one line: **an inert path can only ever be information, and information cannot be exploited.** The moment a path is worth 5% walk speed, the player's relationship to it changes from *reading the town* to *farming the town*, and a feature about noticing quietly becomes a feature about optimising. This game's centre of gravity is the first thing.

### 6.1 The hooks, if they are ever wanted

Named here so that a later slice inherits a design rather than inventing one:

- **Paving.** A worn tile is a natural site for a player-built footpath — the town shows you where it wants a path, you spend money making it permanent. This is the strongest of the hooks because it keeps the *reading* intact and adds a verb on top of it.
- **A district's legibility.** Path density around a station is a real, already-computed signal of how well that catchment is served on foot. It could feed an overlay ([05](05-inspection-and-overlays.md)) without ever touching the sim.
- **Erosion on a slope.** A worn tile on steep ground could wear differently — a rut rather than a track. Art-only, and cheap.
- **Speed, but only on paved paths.** If a walking bonus is ever wanted, it should attach to the *built* thing in the first hook and never to the worn one, so the feedback loop runs through a player decision instead of around it.

---

## 7. Saves

Persistent paths are most of the point. A world that forgot its habits every time it was loaded would be a world with no memory at all, which is the opposite of what this feature is for. So wear is **in the save**, and the schema goes to **5**.

The blob is sparse — `(tile index, wear)` pairs, sorted ascending by index, for non-zero tiles only. A town with two thousand worn tiles costs about 12 KB. Sorted by construction, so the bytes are deterministic.

**v4 saves keep loading.** The repo's discipline is that a save from another schema fails outright, and that discipline is correct in general and wrong for this specific change, because the owner has live v4 worlds mid-playtest and losing them to a cosmetic feature would be an absurd trade. The migration is genuinely clean here for a structural reason: `paths` is a new field on the **top-level** `WorldSnapshot` and nothing nested changes shape, so reading a v4 payload needs a mirror of one struct rather than of the whole tree. A v4 file decodes through that mirror, gains an empty wear map, and comes back as a v5 world — its ground unmarked, which is exactly the truth about it.

The envelope accepts version 4 or 5 and refuses everything else, including anything from the future. Both directions are tested, and a v4 fixture is built and decoded in the test suite rather than asserted about in prose.

---

## 8. Acceptance

The feature has landed when all of the following are true.

1. **A scripted commuter wears a path.** One peep walking a fixed route twice a sim-day drives its tiles to Faint inside four sim-days, deterministically, to the exact wear value.
2. **A single trip leaves nothing.** One crossing never reaches any level, and is gone inside a sim-day.
3. **Regrowth returns the ground to clean** over the horizon in §2.4, and a saturated path drops out of Bare before it drops out of Faint.
4. **Levels transition exactly at their thresholds** on the way up, and exactly `WEAR_RELEASE` below them on the way down.
5. **A worn tile draws in both projections**, verified from above and in isometric.
6. **Nothing re-composites when nothing changes level** — counted, not asserted. A hundred ticks of heavy traffic that crosses no threshold must produce zero chunk rebuilds.
7. **A v4 save loads**, and a v5 save round-trips its wear exactly.
8. **The value rule holds** — every path fill is within 7 L\* of the ground it lies on (5 on the lowland bands), measured in L\* rather than eyeballed, and the bound is checked against the band ladder it protects.

---

## 9. Open questions

- **Should regrowth pause at night?** Nobody walks at 3 a.m., so every path currently loses ground overnight and regains it in the morning rush. That is probably correct and even pleasant — paths would breathe with the day — but it has not been observed at length.
- **Does the rim falloff survive a diagonal lane?** Routes are four-neighbourhood, so a diagonal lane is a staircase, and a staircase of centre-weighted diamonds may read as beads rather than a ribbon. The headless picture of a straight lane looks right; a staircase has not been judged. If it beads, the fix is a 4-bit adjacency mask on the path cell — the deferred adjacency work in §3.3 arriving after all.
- **Is the rim thin enough where a path meets grass, and dense enough where it meets more path?** One falloff has to serve both, because a per-tile mask cannot tell the two apart. The current setting is a compromise picked by eye against a straight lane.
- **How much of the map is worn in a mature town?** If the answer is "most of it near stations", the Bare level may need to be rarer, either by raising its threshold or by capping how much of a district can saturate.
