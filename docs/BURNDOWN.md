# Burn-down — design briefs

Tracks work against [`docs/design/11-roadmap.md`](design/11-roadmap.md). Update as phases land.

## Phase A — Feel

| Item | Status |
| --- | --- |
| Contiguous terrain + palette + integer zoom 1/2/3 | done |
| Interpolated trains (read `progress`, facing) | done |
| Drag-build ghost, cost HUD, loud failures, right-drag demolish | done |
| Undo / redo via command history | done |
| Pixel UI kit + toolbar + status strip | done |
| Track lay / fail sounds | done |

## Phase B — Legibility

| Item | Status |
| --- | --- |
| Selection + Inspector | done |
| Station panel / Peep card / Town Talk | done |
| Overlays + Map View | done |
| Ledger + alerts | done |

## Phase C — The Loop

| Item | Status |
| --- | --- |
| Lines (player-authored) | done |
| Buildable stations + tiers | done |
| Track maintenance + terrain cost + grade limits | done |
| Distinct transit/transport profiles | done |
| New demand outside network | done |
| Congestion: reroute, yield, passing loops, double track | done |
| Goods platform tier, placed against an industry lot | done |
| Station demolish confirms, then drops the call | done |

Passing loops and double track needed **no track-graph change** — the tile graph
is 8-connected and auto-links neighbours, so a parallel row of tiles already is
a loop. The gap was that movement followed a fixed path and waited.

Demolishing a stop a line calls at now asks (04 §4) and then removes the call
from every line, recording the slots so undo puts them back. A line left with
fewer than two distinct calls is **kept, dormant** rather than deleted: deleting
it would strand its trains on a `LineId` that no longer resolves, and it is the
player's named object. A dormant line hands its trains no next stop, so they
finish the leg they are on and idle.

## Phase D — Life

| Item | Status |
| --- | --- |
| Terrain art: autotiling, cliffs, chunked composite | done |
| Building tiers on lots, district character, construction/decay as events | done |
| Walking peeps with journeys, households, routines, memory | done |
| Day/night, lit windows, ambient motion | done |
| Full audio: ambience beds, trains, music, mixing | done |

## Phase E — Frame

| Item | Status |
| --- | --- |
| Title, new map, pause, settings | done |
| Save / load | done |
| First-run teaching | done |
| Goals mode | done |

## MP — Neighbour maps

Design: [`docs/design/12-multiplayer.md`](design/12-multiplayer.md). MP-1 requires no networking.

| Item | Status |
| --- | --- |
| MP-1 — portals as construction, border yard, transit, manifests, echo neighbours | done |
| MP-2 — friend codes, blob exchange, cache + reconciliation | pending |
| MP-3 — community pool, relationship maturity | pending |

---

## Playtest fixes (2026-08-02)

| Report | Cause and fix |
| --- | --- |
| Money drains out in ~2 min, then the game is unwinnable | **Bug, not tuning.** Upkeep was charged every tick against a 64 Hz timestep, so every running cost was x64 — 3 idle stations cost ~$345/min. And going broke parked every train, i.e. removed the only income, making bankruptcy terminal. Rates are now per sim-minute via a cent-tick accumulator; trains never park for money; the balance floors at zero and uncollected upkeep is simply not collected. Fares raised to match. |
| Tofu boxes in UI text | The shipped font has no glyph for `.`, `-`, `x`, mood/pause symbols. All string literals are ASCII-only now. **Standing constraint** — brief 03 wants a bitmap pixel font, whose charset will be small. |
| Cannot leave build mode | There was no non-building tool at all. Added **Look** (`V`) as the default, with a toolbar slot, and `Esc` now unwinds one layer at a time before disarming. |
| Way too much water | Composition retargeted to Locomotion/RCT: land 85-92%, sea 0-4% and absent from many maps. |
| Buildings carpet the map | Density falloff was linear to the catchment edge, so every town was the same soft blob. Now a steep power curve with a hard cutoff, and the rural prop carpet (~52% of tiles) is clustered farmsteads (~4%). |
| Buildings too small, no hover | Atlas cell 16x32 -> 24x48, silhouettes redrawn to be separable at a glance. Hover highlight + tooltip added, reusing the click hit test. |
| Peeps walk through water | A* over walkable ground, 4-neighbour so a diagonal cannot cut a river corner. Bridges are walkable; cliffs and mountains are not. |

## Input, audio and panel sound (2026-08-02)

| Item | What landed |
| --- | --- |
| Controls rebinding is consumed | `input::KeyBindings` is the live map. Every player-facing verb — tools, station place / upgrade, camera, speed, undo / redo, overlays, Map View, follow, and all six window keys — reads `bindings.just_pressed(&keys, action)` instead of a `KeyCode` literal, and `InputMapPlugin` copies `Settings::controls` in on `PreStartup` and on every change. The menu row draws its shortcuts from the same resource, so a rebind is visible where the verb is. A modifier is now part of a press as well as of a binding, which incidentally stopped `Ctrl+Z` from also resetting the zoom. |
| The `L` conflict | Gone. The Line tool keeps `L` and the **Ledger answers to `K`** (03 §10.2, which already said so while `shell::controls` was the stale copy). The window keys are their own Controls group, and a test asserts the whole default table is conflict-free and that no window key is also a gameplay verb. |
| Audio settings UI | The five bus rows on the Audio tab draw the kit's meter beside their numeral — the pixel-UI slider (03 §8.4), `hi` fill rather than the diagnostic ramp, since a quiet music bus is a preference and not a fault. `mixer::apply_settings` slews the buses toward the stored values every frame, so a change is audible while the panel is still open. |
| Panel UI cues | `audio::UiCue` moved outside the `sfx` gate, and `ui::window::panel_cues` emits the open / close sweep from the one place panel visibility flips — the window manager, plus the Settings overlay, which is the only panel it does not own. At most one cue a frame, and opening wins: a panel replacing another is one opening, not a chord. |

## The arc, the art, and the verbs (2026-08-03)

Three brief-audits (economy/loop, world art, town/shell) drove this batch; every
carried-debt row from the previous table is resolved.

| Item | What landed |
| --- | --- |
| The 640x upkeep hole | `opex.rs`'s "sim-minute" divisor was a real minute, so running costs were collected at 1/640 of their authored rate and overextension could not exist. The time base is honest now (`TICKS_PER_SIM_MINUTE = 6`, documented ratio), and upkeep re-derived against **measured** income from a scripted harness (`rail_sim/tests/economy_arc.rs`): an early network keeps ~73% of gross, 200 dead tiles sink it, pruning restores the rate on screen within a minute, paving the map costs more than any railway earns. |
| Flat fares | `fare = base x (len + len^2/divisor)` on endpoint separation: a 60-tile haul pays ~34x a 4-tile hop, freight steeper. Reaching further is now the lucrative move (08 §2). |
| Demand went silent at minute 31 | Lifetime spawn cap replaced by a pending-board cap of 3; the interval stretches 4 -> 10 min as the count grows, the spacing floor walks outward, and a full map falls back rather than going quiet. Fifteen opportunities across a two-hour session. |
| Goals ended at minute 38 | Deadlines spread to ~106 minutes, quotas derived from measured train throughput (never clearable in half the window), and a resolved board generates a harder successor deterministically. |
| Capital was pocket change | With upkeep real, everything you buy once had become noise (a halt: $40 against ~$950/min net). Track and stations x10, trains x6, the border portal to $15,000 — more than the opening balance, priced like the over-hours commitment 12 §3.1 wants. |
| Smart route | The default drag is an A* over `tile_build_cost` plus a straightness term — follows contours, picks river narrows, rides existing track free, holds last frame's shape against ties. Shift keeps the ray snap, Ctrl single tile, Alt locks the contour. Commits as one atomic `AutoFillPath`, one undo entry. |
| Trains route by time | Hop-count BFS -> Dijkstra over each profile's own `ticks_for_leg`, so the fastest route for a train is a route that exists (02 §3's third leg). Tier dwell is wired: an interchange turns a transit around 3x faster than a halt. |
| Elevation was unreadable | The band ladder now climbs strictly in luminance (L* 23 -> 50) across everything buildable; snow is only the crest on an impassable wall's drawn face, the lit cliff face marks exactly the wall the grade rule refuses, inland water carries three depth steps, and the New Map preview + water glints read the same table. |
| Map View aliased | Purpose-built schematic plate (flat palette fills, contour lips, line-coloured strokes, tier-sized icons, live train dots), baked on change, never resampled. |
| Facing collapsed to four looks | The old `custom_size`-stretch facing was a transform in disguise. A baked 32-entry bank at the sixteen realised bearings plus midpoints; zero bank error on straights; nothing rotates, mirrors or stretches. |
| The town was mute on screen | Town Talk starts open (it is the emotional hook, and the onboarding nudge lands in it); the Peep card composes journey, tenure, routine, household and memory; the town announces finished buildings and asks for bigger stations at capped districts; a departing household's own house is the one that boards up; construction gets dust and a hammer tick; platforms murmur by crowd. |
| The title showed no railway | Boot lays a short demo line and runs one transit behind the menu (09 §2), through the real command path, then hands the player a clean ledger, full cash and zeroed goals on Begin. |
| Congestion was invisible between trains | The overlay reads the crossing-memory window (sustained use holds its tint, one-off movements fade); a blocked train's cause row names the queue's head and is a click target that walks the chain; a blocked **ring** that outlasts every remedy raises a Gridlock alert naming the fix. |
| Plugins were not state-gated | `input::PlayerVerbSet` carries every press-to-verb system and `main.rs` runs it only in `ShellState::Playing`. The world stays alive behind the title; it just is not listening. Input suppression remains as the safety net. |
| Save format | `SCHEMA_VERSION` 4: gen knobs travel with the seed (a save reproduces its world), mirrors destructure exhaustively (a new field is a compile error, and `peep_waiting` was in fact being dropped), `MoneyLedger` counts paid runs, `GoalBoard` carries its generation. |

## Second playtest (2026-08-03): the owner's session, answered

Four worktree branches landed and merged in sequence (builder first — it and
worldgen both re-premise `rail_map`'s crossing tests), full suite green after
each. 1,125 → **1,181 tests.**

| Verdict | What landed |
| --- | --- |
| "I want to build one by one like RCT... intentional, not an afterthought" | The modifier table flipped: a plain drag is a **straight ray** on the sixteen directions — the player picks the angle by pointing, every tile priced on the ghost, illegal tiles refused loudly. Ctrl one tile, Alt contour lock; the A* is **Shift-only** now and leashed to a six-tile corridor around the drawn segment (integer capsule maths) — it may find a narrows, it may not re-plan the journey. Outside the corridor it refuses: *"No buildable route near this line."* |
| "There should be a way to build bigger bridges... naturally cost more" | `MAX_BRIDGE_SPAN` 3 → **8**, per-tile ladder 8/14/20/**30/42/56/72/90×**. Spans 1–3 stay the cheap tier the generator authors narrows against (`CHEAP_BRIDGE_SPAN`); a full eight-span crossing is 720× base — a project, not an opening move. No bridge mode: a straight drag across water is the gesture. `rail_map` scouts only cheap narrows as *crossings*; trunks are premium-bridgeable, so rivers pose a decision without being walls. |
| "Building should always show cost as I'm going" | The cost readout keyed off the mouse-drag, and continuous building spends most of its time between drags — priced ghost, hidden price. Readout and reason chip now key off the **preview** in all four modes, with a fixed-corner fallback when the pointer leaves the window. |
| "Constantly fighting terrain... mountains and up/down everywhere" | The plain is band 0 and landforms stand *on* it: one anchored massif (truncated, sea-standoff), a tableland seated against its inland end, cols cut clean through the crest. Measured on the six repo seeds, Rolling: flat-versus-all-eight-neighbours **56% → 80%** of land, hill systems 2.3 → **1.0** with zero stray spurs, band crossings per 30-tile line 5.5 → **1.9** (the rock share's geometric floor is ~1.4 — derivation now in brief 02). Rivers read the continuous relief so they still carve valleys on a one-band plain. `MapGenOptions::pack()` byte-identical — saves unaffected. |
| "Really repetitive and not upbeat... I wanted C418 vibes" | The score is **four pieces per world** rotated by the cue counter — consecutive cues never share key, motif or progression. Each: 4/4 at 88 BPM in D/G/A/C major, intro–A–B–A′–C–A″–outro, functional diatonic harmony cadencing at every section end (`Vsus4` over bare `V`), two motifs restated under variation with the interval signature intact, melody C4–A5 on a four-partial bell over polyBLEP pad, sine bass, and the old Karplus–Strong demoted to off-beat arpeggio. Dusk softens the reading; it no longer drops an octave. Part-writing is *searched*: 96% textbook tier, zero parallel fifths/octaves anywhere, measured across 8 seeds × 4 pieces. Brief 14 rewritten around what shipped. |
| "With $1,500 left I couldn't place a train, so I'm perma stuck" | The transit/transport verbs pushed `BuyTrain` unconditionally — a failed $3,000 purchase masked the free placement of stock already in the yard. `arm_train_place` asks the yard first (keyboard and menu row share it); buying and entering service now speak in Town Talk (*"Train 3 delivered — click a station to place it"*, *"Train 3 entering service at Eastgate"*); and **SellTrain** refunds full price, requeueing any carried run back onto the JobBoard so demand is never silently deleted. `X` with a train selected asks first, through the existing confirm dialog. |
| "B is a hotkey for both map and build?" | Diagnosed to the file: the shipped table is conflict-free (a test holds it so), but the owner's `settings.ron` carries `controls_MapView: "KeyB"` — written by an automated UI-driving session against the real profile, adopted silently ever after. The adoption path now names any clash in Town Talk (*"B is bound to both Track tool and Map View — fix in Settings > Controls"*); load keeps honouring stored values because the Controls tab deliberately allows-and-flags conflicts. Automated sessions must set `RAIL_TOWN_CONFIG_DIR`. |
| Web keys dead until a click | The canvas takes focus after init, on window focus, and on pointer press. |

Also in this batch: briefs 02/04 caught up to the eight-span ladder and the
crossing target's geometric floor; the `iso-prototype` branch (2:1 dimetric,
presentation-only) awaits the owner's accept/reject.

## Third session live-fire (2026-08-03, evening): the owner played while we fixed

Three defects the owner hit in a live session, each root-caused from his own
screenshots, fixed by parallel agents, merged in sequence. 1,181 → **1,274
tests.**

| Report | Root cause and landing |
| --- | --- |
| "Barely moving anyone, virtually no money" — a correct 20-tile opening line bled ~$410/min against $28.80 of fares | **No constant was wrong; the demand path had three bugs.** The station-pair walk indexed the raw tick, fired every 45 ticks over 3 anchors — gcd says both ends were frozen forever, so the board posted one ordered pair and a 1-in-3 dice roll decided if a player's line ever saw a fare. The 8-slot board had no expiry while the world planted unconnected settlements, silting it shut in two minutes. And station maintenance billed every stop in the registry — $90/min for unreached anchors, rising $30/min per invented village: a tax on doing nothing. Now: the walk mixes, unservable jobs expire, and only railhead-backed stations bill. Measured: the owner's exact scenario goes from bankrupt-by-minute-8 to **surplus from minute one**, every standard seed 2.1–4.0× running costs by minute three — brief 02 §4.1's "paying out by the third" finally true. Zero src constants changed. |
| "Really confused by the Lines mechanic... clicked the train but nothing happens" | A designed-but-never-wired flow: the Line tool switched out of Look mode and never switched back; only Look selects; the assign button fails silently with nothing selected; no dedupe; no delete; the intended click-to-assign helper sat as dead code. Now the tool returns to Look on every exit (Enter *and* Esc), the new line auto-focuses, a fresh train pick assigns to the focused line and says so in Town Talk, the button states its precondition out loud, duplicates (including reversed — an out-and-back is one service) are refused at both layers, rows carry a Remove button through the confirm dialog (X stays demolish: sticky focus would have turned it into a trap), and "Westbroo" got its name back. |
| "Can the prototype run as a mode inside the game?" | Yes — `iso-mode` merged. The projection is a runtime flag at the coords choke point; `I` (or Settings > Display) flips it live via the same presentation rebuild a new world runs. Top-down default is asserted byte-identical; flip costs 0.3 ms into iso, ~16 ms back (the chunk compositor, priced the same as a world swap); Map View works in both; sim-hash asserted unchanged across mid-play flips. Known iso gaps stand: no autotiling (shoreline staircases), top-down sprites read as stickers, and the calm worldgen means most of a stock map is flat — the projection's best argument now only appears where a landform does. |

<<<<<<< HEAD
## Fourth wave (2026-08-04): five agents, five landings, one dead promise found

Five worktree agents ran concurrently against explicit file fences and landed in
dependency order, full suite green between every merge. 1,293 → **1,418 tests.**

| Landing | What it settled |
| --- | --- |
| **Iso inclines** (brief 15, new) | The 2:1 projection is affine in height, so a ramp is the straight screen segment between tile centres and every joint is whole-texel for all 16 directions × 9 grades — asserted as *equality*, not tolerance. Ghost draws the placed piece's own baked cell. Top-down pinned byte-identical by 96 golden hashes. The "~1 px jog" folklore was measured to be sleeper-pitch stutter, not a gap; pitch now fits the link. |
| **Desire paths** (brief 16, new) | Wear counts *footfalls* (not ticks — walking speed must not dig deeper), 64 per step against thresholds 256/640/1024, regrowth 60/sim-day. Trunks outside stations wear first; doorsteps stay clean. **Schema 4 → 5 with a real v4 migration** — a deliberate soften of "other schema ⇒ refuse", because losing live worlds to a cosmetic layer is an absurd trade; the mirror-struct caveat is recorded at the code. Known bias, named in the brief: wear samples full-detail peeps, so paths form where the camera has been. |
| **Coast foam & glints** | Foam applied its edge inset in *screen* space — identical to ground space top-down, 45°-rotated and doubled in iso, so every lip stood a corner off its shore and flips stranded 337/415 of them. Now ground-anchored like everything else. The generic sweep this added — every world sprite resolves to the same tile in both projections — failed on 173/382 sprites pre-fix, and found `game_app` had never registered `AtmospherePlugin`: the whole layer was invisible to every class test until now. |
| **Trains window & refusals** | "Cannot place a freight train" was four silent refusals, not a sprite bug — led by the failed debit on a $4,500 goods train. Refusals now speak in Town Talk; `R` opens rolling stock with Place/Sell/Find; `TrainsParked` repurposed as a quiet "N in yard" count, deliberately not warn-painted. |
| **Stations on the toolbar** | Station arms as a mode with the tier row (prices inline); Upgrade stays on the Inspector because it arms nothing. Catchment preview existed and got pinned by tests instead of rebuilt. **The wave's biggest find: growth was structurally dead** — service score decayed 1/tick after 120 idle ticks, nothing outruns that, every station sat at 0 and `density_target_at` multiplies by score/100. Now decay is 1/sim-minute and a scheduled call banks half an arrival; end-to-end test proves a newly line-served station grows its town. *These two constants are the wave's open veto item.* |

Integration notes: the only textual conflicts were where two agents extended the
same seams (`visuals.rs` bake key, `iso.rs` test compositors) and resolved to
the union — main's trigger-free reconcile kept, grades and projection both in
the staleness check. One semantic collision no fence could catch: the stations
agent widened the toolbar's arm path to carry `StationToolState`, which the
trains window's minimal test app didn't provide; one `init_resource` line. The
pacing agent (in flight) was told mid-run that its growth baselines were
measurements of the broken scorer.

## Time model (2026-08-04): the clock stopped lying, the town slowed down

Two reports, one root cause — **nothing in the game agreed about how long
anything took.** New binding standard, [17 — Time & Pacing](design/17-time-and-pacing.md).

| Report | Root cause and landing |
| --- | --- |
| "Trains go between ~10 tiles in ~1 in-game minute at 1x which is insanely fast" | Both halves were true and they were **two separate faults**. The status strip's `HH:MM` ran on the twelve-real-minute *light* cycle — a clock-minute every half a real second — while the Goals panel and the Peep card counted the 2¼-minute *sim* day, so the game had two days 5⅓ apart, both called "day", and the visible one made every journey look instant. And the train really was doing 21.3 tiles a real second: a 64-tile map crossed in three seconds, 57 round trips a minute on the opening line. Now: **the strip shows `Season Day` and a part of the day, and no minutes at all** (RCT's answer, and the only one that cannot be caught lying), the date is the sim's own day so there is one day in the game — saved with the world, where the old counter reset to Spring 1 on every load — and transit runs **a tile per sim-minute**, 10.7 tiles a real second, exactly 4× a walking peep, which is the floor. |
| "House growth happens too quickly, within a few in-game minutes — it should be over a few days" | The growth pass ran **every tick** at 4% of the gap: half the target in seventeen ticks, full inside one real second. Denominated in the day now — 24 passes a sim day at 1.25% — so a served block takes its first lot half a day in and its fourth on day five (1.1 → 12 real minutes), and a district that loses its service sheds half its buildings over 2½ days. The pass also costs a 360th of what it did. |

Fares and goods payouts **doubled**, which is arithmetic rather than balance:
costs are billed per *real* minute and a fare is paid per journey, so halving
the timetable halves income against a fixed cost side. Measured either side of
the change, the opening line earns $1,239 → $1,218/min and clears its capital in
real minute **seven** both times; the compact local line holds 2.47× its costs
and pays back in minute 13 both times. Every 02 §4.1 bar holds with the margin
it had. 1,418 → **1,426 tests.**

Left deliberately: `SIM_SECONDS_PER_TICK` (every per-sim-time claim in the
codebase and in briefs 06/08/16 hangs off it, and brief 16's day-denominated
wear rates stay true untouched); the eight-second scaffold (06 §3.1 is a
*real*-seconds spec about holding the eye); the service-score decay constants
(set deliberately elsewhere and under owner review — 17 §5.1 records that the
120-tick idle grace is now under one lap of the opening line rather than two).
One seam is recorded rather than hidden: the light cycle is 12 real minutes and
a sim day is 2¼, so the sun goes round once every 5⅓ days of the date — 17 §3.1
names the one-number fix and whose it is.

## Carried debt

Things that work but are known-shallow, with the brief that wants more.

| Item | Note |
| --- | --- |
| **Build preview shows cost, not upkeep** | The cold-start work showed alignment efficiency drives payback (4–16 min spread for the same hop). The ghost prices construction; showing the run's added $/min maintenance next to it would put 08 §3.1's liability in the player's hand at decision time. |
| **Iso mode is a projection, not yet an art style** | No diamond autotiler (material transitions, shorelines, contours), non-terrain sprites are top-down stickers, cliff-occluded picks (~1 tile in 20). Inclines and the water layer have native iso art now; the autotiler and sprite re-draws remain the (large) bill. Per the owner, iso is where build effort goes. |
| **Desire-path wear follows the camera** | Wear samples full-detail peeps and that set is viewport-biased (`PeepFocus`). Deterministic, but the network belongs to your attention, not the town's routines. The fix — depositing wear along abstracted peeps' planned routes — is strictly better and considerably larger (16 §5). |
| **No insert-stop verb** | Extending a line is `RemoveLine` + `CreateLine`. Works, and the station manage flow now makes it visible; a real "add this stop to the line" is the natural next beat. |
| **Single-track rings still deadlock** | The Gridlock alert names it and the fix; the *sim* remedy (a train backing out, or refusing to enter a corridor that cannot pass) is real movement work. Brief 07 §4.3's signals remain the depth extension. |
| **Train capacity / acceleration** | Brief 07 §3 calls acceleration "the defining trait" and capacity is still one job per train for both kinds. The profile table has the seams; neither dimension exists yet. |
| **Brief 13 (shadows) is designed, not built** | The commit that added the brief touched only docs. Phase 1 (shade operator, band hems, fray) is the shippable slice. |
| **Accessibility rows are stored, not wired** | `colour_blind_safe`, `flashes_and_shake`, `hold_repeat_ms` persist and do nothing; `reduced_motion` reaches only the title drift. The palette rework is the L-sized one with real user impact. |
| **Load is a button, not a screen** | `MenuAction::Load` grabs the newest slot. Named slots, thumbnails (`SaveMeta::with_thumbnail` has no caller) and a list screen are designed in 09 §6. |
| **Ledger has no per-line breakdown** | 08 §6 wants per-line / per-station contribution and a projection; the ledger is category-level. `MoneyLedger::record` would need a line tag, available at both payout sites. |
| **Events beyond the stub** | `EventDirector` remains empty. Festival (demand spike) and landslide (temporary closure) are the cheapest two that exercise announce / react / recover (08 §5.2). |
