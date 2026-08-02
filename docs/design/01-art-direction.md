# 01 — Art Direction

**Status: binding standard.** Sections 2 (pixel contract), 3 (palette) and 4 (camera) are constraints, not suggestions — they define what the game is allowed to look like. The rest is direction with room to argue.

**Evidence base:** the *Permanent Way* feasibility gallery (`~/dev/railgen`) — six interactive plates testing whether pixel-art railroad track can be generated at arbitrary angles without losing the aesthetic. Its measurements settle four questions that would otherwise be taste arguments, and they are cited inline.

---

## 1. The look in one sentence

**A cool-shadowed, low-saturation pixel town seen from a fixed high angle, where the only bright things in the world are the rails, the lit windows, and the money.**

Three adjectives, and every asset is tested against all three:

- **Quiet.** Low saturation, narrow value range within each material, no hue competes for attention. Contrast is a budget, and it is spent on track and on things the player must notice. Background terrain never spends it.
- **Drawn.** Every pixel looks placed by a hand. No filtering, no anti-aliasing, no gradients, no runtime rotation, no fractional scale. A shape that cannot be drawn cleanly at this resolution is not in the game.
- **Slow.** Everything that moves, moves gently. Water shimmers over a second, smoke rises over three, a building goes up over eight. Nothing pops, flashes, or bounces.

The reference axis is *Rail Route* and *Mini Metro* for network legibility, crossed with *Rise to Ruins* and *Songs of Syx* for town density, rendered with Factorio's discipline about direction count.

---

## 2. The pixel contract

Five rules. Four of the six *Permanent Way* plates exist because breaking one of them is cheap to do and expensive to discover late.

### 2.1 One texel is one screen pixel times a whole number

The world renders at a fixed logical resolution and scales up by an integer factor. There is no fractional zoom.

| Property | Value |
| --- | --- |
| Source tile | **32 × 32 texels** |
| Zoom factors | **1× · 2× · 3×**, nothing between and nothing outside |
| Default | 2× |
| Sampling | Nearest-neighbour, no MSAA, no mipmaps |
| Camera translation | Rounded to whole world texels every frame, after integration |

The `crawl` plate is why this is a contract:

> Snapping the camera to whole world pixels takes the shimmer from double-digit percentages of the frame to essentially zero, and it costs one rounding call. The catch is that it constrains the rest of the engine: sub-pixel smooth scrolling, fractional zoom and free camera rotation are all off the table, so this is an architecture decision, not a rendering tweak.

The trade is taken deliberately. **We give up:** smooth camera glide, pinch-zoom, camera rotation, free-rotating sprites. **We get:** a world that is perfectly still when the camera is still and perfectly clean when it moves.

Zooming out is not how the player sees the whole map. At 1× a 64×64 map does not fit on screen, and that is correct — sampling below one texel per screen pixel destroys the art. To read the whole network the player opens **Map View** (`M`), a separate schematic render at 4 texels per tile showing terrain silhouette, track, stations and line colours. It is a better strategic read than a zoomed-out world *and* it sidesteps the sampling problem entirely. See [05 — Inspection & Overlays](05-inspection-and-overlays.md) §6.

### 2.2 Sprites never rotate at runtime

Direction is expressed by choosing a different sprite, never by transforming one. A rotated sprite resamples, and resampled pixel art is mush. This single rule is what makes the direction-count decision in §5 load-bearing rather than academic.

### 2.3 Terrain is contiguous

Tiles meet exactly, edge to edge, with no gap and no seam. A visible tile grid is a debug affordance: it belongs behind a toggle (`G`), drawn as a 1-texel `outline` line at 25% alpha over the world — never baked into the terrain itself.

A world with gaps between its tiles reads as a spreadsheet with colour in the cells. Contiguity is what turns 4,096 tiles into a landscape.

### 2.4 Decoration noise is anchored to the world

Any procedural per-pixel variation — ballast speckle, grass tufts, sand grain, dither, foam — hashes on integer world coordinates. Never on screen position, never on time.

The `downsample` plate's finding is blunt: screen-anchored Floyd–Steinberg *boils* across the entire surface under scroll, while ordered Bayer anchored to world pixel coordinates fixes it almost completely. The same hash discipline applies to every speckled material in the game, not just dithered ones.

### 2.5 Art is baked when data changes, never per frame

Track sprite selection, autotile mask resolution and building composition happen on edit, in response to the world changing. The unit of rebuild is a **16 × 16 tile chunk**, and a chunk composites to a single drawn sprite.

The `spritebank` plate measures a full rotation-bank rebake at under a millisecond, so even a pathological full-map rebuild is affordable. Dirty-chunking is not about the cost of baking; it is about never doing per-tile work in a per-frame loop.

---

## 3. Palette

### 3.1 The spine

*Permanent Way* established a coherent 19-colour palette that all six plates were drawn against, and it has a specific and good character: **cool violet-grey shadows, warm brown sleepers, cold steel rails.** The track art we want comes from that world. Adopt it verbatim as the core rather than deriving a palette independently and then forcing the track to fit.

```
─ Ground / structure ──────────────────────────────────
bg0       #12111a    darkest — night, voids, map-view ground
bg1       #1b1a26    panel fill, deep shadow
outline   #241f2e    the only outline colour in the game

─ Ballast (cool violet-grey) ──────────────────────────
ballastD  #3b3546    ballastM  #57505f    ballastL  #776e77

─ Sleepers (warm brown) ───────────────────────────────
tieD      #40291c    tieM      #5c3b26    tieL      #7d5436

─ Rail (cold steel; railS is the polished head) ───────
railD     #4a4f5c    railM     #7f8899
railL     #b9c2cf    railS     #e8eef5

─ Grass ───────────────────────────────────────────────
grassD    #2a3d24    grassM    #3f5a30    grassL    #5c7a3a

─ Diagnostic only — NEVER in world art ────────────────
hi        #f2c14e    highlight / selection / money
warn      #e8624a    invalid / negative / alert
ok        #6fd08c    valid / positive / confirm
```

The gallery's rule about the last three carries over and gets stronger: **`hi`, `warn` and `ok` appear only in UI, overlays, ghosts and selection. Nothing in the world is ever drawn in them.** That is what makes a highlight read instantly — it is a colour the world is incapable of producing.

### 3.2 Extension ramps

The gallery only needed track and grass. The game needs terrain, water and buildings. Extend in the same key — violet-leaning shadows, desaturated mids, warm highlights — and hold every ramp to **three steps**, so material identity survives at 1×.

```
─ Water ───────────────────────────────────────────────
waterD    #16283d    waterM    #22405c
waterL    #335b78    waterF    #5d8ea3   (foam / shallows only)

─ Beach & bare earth ──────────────────────────────────
sandD     #6b5a3e    sandM     #937d55    sandL     #bda87a

─ Hills (grass ramp shifted ochre) ────────────────────
hillD     #34401f    hillM     #4c5a2a    hillL     #6a7638

─ Mountain rock (shares the ballast hue family) ───────
rockD     #3d3944    rockM     #565260    rockL     #7b7684
snow      #cfd2dd    (only above the top elevation band)

─ Buildings — plaster ─────────────────────────────────
plasterD  #6d5f4e    plasterM  #97846b    plasterL  #c0ab8c

─ Buildings — timber ──────────────────────────────────
woodD     #3a2a1d    woodM     #5a4029    woodL     #7d5a38
                                          (woodL = tieL, deliberately:
                                           it is the same lumber)

─ Roofs ───────────────────────────────────────────────
roofTileD  #4a2622   roofTileM  #6e3a30   roofTileL  #8f4e3e
roofSlateD #2c313c   roofSlateM #414855   roofSlateL #5b6371

─ Windows ─────────────────────────────────────────────
winDark   #2a2f3a    winLit    #f2d98a
                     (the warmest colour in the world art,
                      and it only exists at night)
```

**Total world palette: 45 colours.** That is the cap. Adding one requires deleting one.

### 3.3 Colour discipline

1. **Terrain lives in the bottom two-thirds of every ramp.** The light step of a ramp is for slope-facing tiles only, never for flat ground. Flat terrain that is too bright competes with track and loses the frame.
2. **Track is the highest-contrast object in the world.** `railS` on `ballastD` is the widest value gap in the palette and it is reserved for the railhead. This is precisely why laid track pops out of the landscape at any zoom, and why nothing else may use that gap.
3. **Saturation ceiling for terrain is roughly 35%.** A landscape at full saturation has no hierarchy — nothing can recede, so nothing can stand out. Calm is a saturation decision before it is anything else.
4. **One accent owns the screen at a time.** While a build ghost is showing, it owns `hi`; selection falls back to a 1-texel `railS` outline for the duration.

### 3.4 Time of day

A single full-screen tint pass driven by the sim clock, on a twelve-minute cycle at normal speed. Never darker than 65% — night is legible, not black.

| Phase | Tint | Character |
| --- | --- | --- |
| Dawn | `#c08a5a` @ 18% overlay | long, warm, low contrast |
| Day | none | flat and neutral |
| Dusk | `#b06a4e` @ 22% overlay | warm and saturated, the prettiest ten seconds |
| Night | `#1b2340` @ 35% multiply | cool, quiet, windows on |

Window lighting is not part of the tint — it is a second sprite layer that fades up over about forty seconds at dusk. A well-served district lighting up at nightfall is the cheapest emotional payoff in the game, and it makes density read as *life* rather than as a number.

---

## 4. Camera

| Property | Behaviour |
| --- | --- |
| Projection | Orthographic, high angle, with a fake front face (§6.1) |
| Zoom | 1× / 2× / 3× via wheel, `+` / `-`; `Z` returns to 2× |
| Zoom anchor | **The cursor.** The world point under the pointer stays under the pointer, then the result rounds to texels. |
| Keyboard pan | WASD and arrows, rounded per frame |
| Drag pan | Middle-drag, or `Space` + left-drag, 1:1 with the cursor |
| Edge pan | Off by default, available as a setting |
| Bounds | Soft-clamped so a map edge can reach screen centre and no further |
| Follow | Selecting a train offers `F`; the camera tracks its tile |
| Fly-to | Jumping to a place — from Town Talk, search, or an alert — is a **cut with a two-frame dip**, not a slide. Sliding a texel-snapped camera at speed still shimmers, because the contents move sub-texel relative to each other. A cut costs nothing and reads as deliberate. |

---

## 5. Track art and the direction-count decision

This is the most consequential art decision in the project, and *Permanent Way* settles it with measurements instead of taste.

### 5.1 What the plates found

| Plate | Finding |
| --- | --- |
| `anglebudget` | A 32px rail has **806** bitmap-distinct rasterizations, **440** that survive a run-regularity test, and **70** pure ratios. The legal fraction *falls* as rails get longer — 83% at 8px down to 31% at 96px — because length gives irregularity room to show. Conclusion: *"the ceiling is set by convention, not resolution, and it lands in the same range as a 64-entry sprite bank."* |
| `spritebank` | Baking N clean-run rotations and stamping the nearest along a spline is **the shippable technique** — the sprites stay hand-clean and only the *choice* of rotation is quantized. N=8 is the named failure case: *"the ties skew and pop between facets as the tangent sweeps (11.25° of error)."* N=32 caps bank error at **2.81°**, N=64 at **1.41°**, and clean-run snapping itself contributes about 2.8°, *"so past 64 you are paying memory for nothing."* |
| `junction` | Turnouts generated from a single angle parameter work between roughly **10° and 30°**. Below 10° they die permanently — the flangeway that defines a frog is **half a pixel** at any sane gauge. Art cost scales brutally: **N=16 is 6 artist-days, N=32 is 25, N=64 is a quarter of a year** of one artist drawing nothing but junctions. *"Both walls close at N≈32"* — at N=64 the 5.6° step is already below the pixel floor, so the extra sprites rasterize to the same mush. |
| `sdf` | Elegant and shader-portable; breaks hard when curve radius drops below the ballast half-width, where the inner offset self-intersects and the medial axis shows as a crease. |
| `downsample` | Works and is trivially authorable, and *"reads as 3D pretending to be pixel art."* |

### 5.2 The decision

> **Sixteen directions for the track graph and every junction. A thirty-two entry sprite bank for the rail runs between them.**

The two numbers differ on purpose, because each sits at a different wall:

- **Junctions are the expensive, brittle thing.** Their art cost scales super-linearly and they hit a hard pixel floor at 10°. Sixteen directions gives a 22.5° step — comfortably above the floor, every junction angle drawable, about six artist-days. Factorio ships sixteen for exactly this reason.
- **Plain track is cheap and sensitive.** There is no combinatorial explosion: a bank is one axis of N sprites, rebakeable in under a millisecond. Thirty-two buys 2.81° of error, already at parity with what clean-run snapping contributes on its own.

So the graph is sixteen-directional, and the curves stamped between graph nodes come from a thirty-two entry bank. That is what lets a long sweeping curve read as drawn art rather than as a row of facets.

**Eight directions is explicitly the failure case** the `spritebank` plate is named after, and it is not available to us.

A sixteen-direction graph on a square tile grid means a piece can link to a neighbour two tiles along one axis and one along the other. That is a genuine increase in the richness of the routing geometry, and it is a real cost in adjacency and pathfinding that should be budgeted as such rather than bolted on. **Minimum turnout divergence is one direction step**; anything shallower is refused at placement time, because it cannot be drawn.

### 5.3 Track cross-section

Ported from the plates' measured constants and scaled to a 32-texel tile. These numbers *are* the line weights.

| Element | Texels | Colour |
| --- | --- | --- |
| Ballast bed, half-width | 8 | `ballastD` base, `ballastM` speckle at ~18% coverage (world-hashed), `ballastL` on the sun edge |
| Rail gauge, centre to centre | 8 | — |
| Rail body, half-width | 1 | `railD` shadow side, `railM` body |
| Railhead | 1 texel at rail top | `railL`, brightening to `railS` on the stretch a train most recently crossed and decaying over about four seconds |
| Sleeper spacing | 4 | — |
| Sleeper length | 14 | `tieD` / `tieM`, with `tieL` on one in five, world-hashed |
| Bridge deck | replaces ballast | `woodD` / `woodM` planking, 2-texel piers descending into `waterD` |

The polished-railhead decay is close to free and it is one of the most satisfying details available to us: a busy main line visibly gleams, a branch nobody runs goes dull. That is the network's usage written directly into the world art, with no overlay and no numbers.

---

## 6. World composition

### 6.1 Projection and depth

Top-down with a **fake front face**: a building's tile footprint is its ground plan, and it draws upward past the tile boundary by its height. This buys a legible skyline without the authoring and coordinate costs of true isometric.

Depth is one rule: things further south draw in front. Layer bands run terrain → terrain decals → track → track decals → buildings → peeps → trains → weather → time-of-day tint, with everything from buildings up sorted by tile row within its band. A train drawing *over* the buildings north of it and *under* the buildings south of it does most of the work of selling the third dimension on its own.

### 6.2 Terrain reads as terrain

Three requirements, in priority order:

1. **Material boundaries are drawn, not blended.** Every boundary — water to beach, beach to grass, grass to hills, hills to mountain — gets an authored transition set resolved from a 4-bit neighbour mask, plus inner corners. A coastline is a *line*, with foam and a sand lip, not a colour change.
2. **Elevation is legible at a glance.** Where the height delta between neighbours is steep, the terrain draws a dedicated cliff face — a solid banded strip in the rock ramp — rather than a gradient. Cliffs are what turn a heightmap into a landscape the player can *route around*, and without them the elevation data may as well not exist. If the player cannot see the ridge, the ridge is not part of the puzzle.
3. **Large expanses do not tile visibly.** Each material carries three flat variants selected by world hash, and water carries depth bands so that open sea has structure instead of being a single flat field.

### 6.3 Ambient motion

Calm is not still. A still world reads as broken. Everything below is cheap and world-anchored.

| Element | Motion |
| --- | --- |
| Water | two-frame shimmer, ~1.2s, phase from world hash so it never pulses in unison |
| Coast foam | three-frame, ~2.4s, on water adjacent to land |
| Chimney smoke | four-frame plume, ~3s, on occupied buildings, gated on density |
| Industry stacks | six-frame, ~2s, only while the industry has recently shipped |
| Train smoke | emitted per tile crossed, drifting, five-second life |
| Level crossings | barriers lower over ~0.8s when a train is within three tiles |
| Station flags | two-frame, ~1.6s |
| Trees | **still, deliberately.** Everything swaying at once is noise, not calm. |

---

## 7. Asset manifest

What has to be drawn, in dependency order. Day estimates assume the `junction` plate's rate of three finished, shaded, aligned pieces per artist-day for complex work.

| Set | Contents | Est. |
| --- | --- | --- |
| Terrain base | 5 materials × 3 variants | 2d |
| Terrain transitions | 4 boundaries × 20 pieces | 5d |
| Cliffs | 8 faces + 4 corners | 2d |
| Track bank | 32 rotations × ballast / sleepers / rails | 4d |
| Junctions | 16-direction turnouts and crossings | 6d |
| Bridges | 3 span types across a direction subset | 3d |
| Stations | 4 tiers × platform / building / canopy | 4d |
| Trains | 2 kinds × 16 directions × loco and 2 car types | 6d |
| Buildings | 4 tiers × 4 variants × 2 roof materials | 8d |
| Industries | 6 types | 3d |
| Peeps | 4 body types × 4 directions × 2-frame walk | 3d |
| Effects | smoke, foam, dust, sparks, construction scaffold | 2d |
| UI kit | font, panel frames, ~40 icons | 5d |
| **Total** | | **~53 artist-days** |

Every one of these is placeholder-able as flat rectangles in the meantime — but a useful placeholder **uses the real palette and the real dimensions**, so that swapping in final art is a texture change and never a layout change. A placeholder that is the wrong size or the wrong colour is not a placeholder; it is a second thing to redo.

---

## 8. Rejected, and why

| Option | Verdict |
| --- | --- |
| Runtime supersample → downsample → palette-quantize | Reads as 3D pretending to be pixel art: line weight drifts between one and two pixels, edges go muddy, orphan pixels litter the frame. World-anchored ordered dither fixes the boiling but not the mush. Keep the technique for smoke and water only. |
| SDF track banded into palette indices | Beautiful, shader-portable, and it self-intersects below the ballast half-width radius with a visible medial-axis crease. Revisit only if track art ever goes truly continuous-angle. |
| Eight directions | The named failure mode of the `spritebank` plate. |
| Sixty-four directions | Both walls close at ≈32. The 5.6° step is below the junction pixel floor, and the art is a quarter of a year, spent drawing pairs that rasterize identically. |
| True isometric | Doubles every sprite's authoring cost and breaks the square-tile track graph, for no gain the fake front face doesn't already deliver. |
| Fractional zoom, smooth camera glide | The `crawl` plate. An architecture decision, already taken. |
| Unlimited palette | The cap is the reason forty-five colours read as one world. |

---

## 9. Acceptance bar

The art direction has landed when a player who has never seen the game can, from a single still screenshot at 2×:

1. Tell land from water from hills from mountain, with no legend.
2. Trace a rail line across the frame and see where it forks.
3. Point at the busiest part of town.
4. Tell a passenger train from a goods train.
5. Say what time of day it is.

And over ten seconds of panning:

6. Nothing shimmers, crawls, or boils.
7. At least three things are moving that the player did not cause.
