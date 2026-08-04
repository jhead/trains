# 17 — Time & Pacing

**Status: binding standard.** Where another brief states a duration, this one states what the unit means. Where the two disagree, fix the other brief.

**The premise:** the world runs six hundred and forty times faster than the room the player is sitting in. Every duration in the game is therefore two durations, and almost every pacing bug in this project's history has been the two being confused for one another — upkeep charged 64× too fast, then collected at 1/640 of its rate, then a clock face that made the railway look ridiculous.

> **Screen time and world time cannot both be literal. Pick which one each readout speaks, and never let a readout speak both.**

---

## 1. The three clocks, and which one each thing uses

| Clock | Length | What it is for |
| --- | --- | --- |
| **Tick** | 1/64 real second at 1× | the unit the sim actually counts. Nothing else is authoritative. |
| **Sim day** | 8,640 ticks = **2 min 15 s real** at 1× | the world's own calendar. Growth, decline, goal deadlines, tenure, wear. |
| **Real minute** | 3,840 ticks | the player's wallet. Every cost and the rate readout. |

A tick advances the world by `SIM_SECONDS_PER_TICK` = 10 sim-seconds. That is where the 640× comes from, and it is deliberately not a number this brief moves: it is the denominator under every per-sim-time claim in the codebase and in briefs 06, 08 and 16.

**Money is billed per real minute and says so.** A railway costs what it costs per minute of the player's life, because that is the only minute a player can feel a burn rate in. The ledger reads `/min` and means the wall clock. See [08 §3](08-economy-and-pressure.md).

**Everything the world does to itself is denominated in sim days.** A town thickens over days. A neglected stop slides over days. A path wears through the grass over days ([16 §2](16-desire-paths.md)). None of those is stated in ticks anywhere it can be helped, because a constant in ticks silently re-paces itself the moment anything else moves — which is exactly how the growth pass came to finish a town in a quarter of a second.

A **season** is 12 sim days — 27 real minutes. A **year** is four seasons, 1 h 48 m real. That is close to *RollerCoaster Tycoon*'s month, which is not a coincidence: a date that turns over every couple of minutes is a session with a shape, and one that turns over every couple of hours is a decoration.

---

## 2. What the player is told, and what they are not

| Readout | Shows | Clock |
| --- | --- | --- |
| Status strip, date | `Spring 3` | sim day |
| Status strip, light | `Morning` | the day tint's own cycle |
| Status strip, rate | `+$829/min` | **real** minute |
| Goals panel | `by day 4`, `2d left` | sim day |
| Peep card, tenure | `lived in Eastgate for 14 days` | sim day |
| Peep card, routine | `leaves about 07:24` | hour of the sim day |
| Nothing, anywhere | `HH:MM` as the current time | — |

---

## 3. The clock face is coarse, on purpose

The owner's report, verbatim:

> *"Trains go between ~10 tiles in ~1 in-game minute at 1x speed which is insanely fast."*

That was an accurate reading of a real number, and the number was on the status strip. The strip showed `HH:MM` derived from the twelve-real-minute light cycle, so one clock-minute went by every half a real second. A transit crossing ten tiles took 0.47 real seconds, and the clock beside it said a minute had passed. The clock was the thing making the claim.

RCT and Locomotion show a month and a year and no hour at all. That is not a limitation of 1994; it is the correct answer to a game whose world runs hundreds of times faster than its player. A coarse clock face cannot be caught lying, because it does not make the claim in the first place.

So: **the strip shows a date and a part of the day, and no minutes.** Six parts — Dawn, Morning, Midday, Afternoon, Evening, Night.

Two rules hold it together:

1. **The date is the sim's own day.** The Goals panel already deals deadlines in `day 4` and the Peep card already counts tenure in days; the strip counting something else meant the game had two days, five and a third apart, both called "day". It now counts `tick / TICKS_PER_DAY` like everything else — and because that tick is in the save file, a loaded world keeps its date instead of reopening on `Spring 1`.
2. **The part of the day is the light's.** [03 §6](03-ui-system.md) requires that what the strip says about the time of day cannot disagree with the tint on the world, and it still cannot: the six parts are a *refinement* of the tint's four phases. Dawn and Night are exactly the tint's; Morning, Midday and Afternoon subdivide the long flat daylight stretch, which has no tint to contradict.

### 3.1 The seam, recorded rather than hidden

The light cycle is twelve real minutes ([01 §3.4](01-art-direction.md)) and a sim day is 2¼, so **the sun goes round once every 5⅓ days of the date.** Nobody can see this — it takes twelve minutes of watching a date that changes five times in the interval to notice — and each length is right for its own reason: twelve minutes is an art decision about how often the world should turn warm, and 2¼ minutes is a demand-and-goals decision that the peep routine and the goal ladder are both built on.

It is still a seam, and the honest fix is one number: set the light cycle to one sim day. That is a change to `atmosphere::DAY_CYCLE_SECS` and to the pacing of every dusk in the game, and it belongs to whoever owns the light. **Recorded here so the next person to touch either number knows the other one exists.**

The peep card's `leaves about 07:24` survives for a different reason: it names an hour of the sim day — the day the date now counts — it describes a *habit* rather than measuring a journey, and it is not permanently on screen, so it never accumulates into a claim about how fast anything is.

---

## 4. How fast a train is

**A transit covers one tile per sim-minute.** Six ticks. That is:

| | Transit | Transport | A peep on foot |
| --- | --- | --- | --- |
| Ticks per flat tile | **6** | 10 | 24 |
| Sim time per tile | 1 min | 1 min 40 s | 4 min |
| Tiles per real second at 1× | **10.7** | 6.4 | 2.7 |

Transit was `3` ticks a tile — 21.3 tiles a real second, a standard 64-tile map crossed in three seconds, and 57 round trips a minute on the opening beat's twenty-tile line. Halving it is the largest real-terms slowdown available, and the thing that sets the ceiling is at the bottom of that table: **a railway has to visibly beat walking.** Transit is now exactly four times a peep's pace. Going slower means slowing the walk first, which is a peep-model change and not this brief's.

Targets at 1×, for the opening beat's ten-tile pair:

| | Before | Now |
| --- | --- | --- |
| Ten tiles, clock | ~1 minute (and it said so) | ten sim-minutes (and it says nothing) |
| Ten tiles, real | 0.47 s | **0.94 s** |
| Round trips a real minute | 57 | **28** |

### 4.1 Fares were re-denominated, not re-balanced

Every cost in this game is charged per *real* minute and a fare is paid per *journey*. Halving the timetable therefore halves income against a cost side that did not move — so the passenger and goods rates per tile both doubled, and the ledger reads what it read before:

| Measured, cold-start harness | Before | Now |
| --- | --- | --- |
| Opening line, income | $1,239/min | $1,218/min |
| Opening line, upkeep | $410/min | $410/min |
| First fare | tick 106 | tick 172 |
| Capital cleared | real minute 7 | **real minute 7** |
| Compact three-stop local | 2.47× costs, paid back minute 13 | **2.47×, minute 13** |

The quadratic distance term, the boarding term's share and the 3:1 goods-to-passenger split are all ratios and all survive untouched. [02 §4.1](02-world-and-terrain.md)'s opening-beat bars are real-minute bars and every one of them still holds with the margin it had.

---

## 5. A town grows over days

The owner's second report:

> *"House growth happens too quickly, within a few in-game minutes. It should be more gradual, e.g. over a few days."*

Growth used to run **every tick** at 4% of the remaining gap. A block reached half its target in seventeen ticks — a quarter of a second — and filled inside one. The player laid a line and the town was finished before they let go of the mouse.

Growth now advances **24 times a sim day**, by 1.25% of the gap each time. That is an exponential approach with a half-life of **2.3 sim days**. For a block at the heart of a fully served town:

| | Sim days | Real minutes at 1× |
| --- | --- | --- |
| First lot — a stake, then a cottage | **0.5** | 1.1 |
| Second lot | 1.3 | 2.9 |
| Third lot | 2.7 | 6.1 |
| Fourth lot, the block full | **5.3** | 12.0 |

A tile out from the platform, where the catchment falloff caps the target at about 0.62, the first cottage lands on day 0.9 and the district behind it fills over the following week.

**The first cottage inside the first day is deliberate and is not a loophole.** [06 §1](06-town-and-peeps.md) wants growth to be *visibly caused*, and a consequence the player cannot connect to their decision has not been caused as far as they are concerned. The **district** is the thing that takes days; the first hint that it has started is prompt. The construction sequence itself — stake, eight seconds of scaffold, settle ([06 §3.1](06-town-and-peeps.md)) — is a *real*-seconds spec about how long a small event should hold the eye, and it is untouched.

Decline runs on the same rate in the same units: a district that loses its service sheds about half its buildings over two and a half sim days, which is [06 §3.2](06-town-and-peeps.md)'s *"legible and gradual"* in the units that promise implies.

### 5.1 What sits upstream of it, and is not this brief's to move

Growth reads station service score, and that score's decay cadence was set deliberately elsewhere and is under owner review. Two things about it are worth recording here, because they are pacing constants and they are stated in **ticks**:

- `SCORE_IDLE_GRACE_TICKS` = 120 — twenty sim-minutes of quiet before a stop starts slipping. At §4's timetable that is **less than one lap** of the opening beat's line, where it used to be about two. The grace period did not change; the lap did.
- `SCORE_DECAY_EVERY_TICKS` = 60 — one point a sim-minute.

Neither was touched here. Both are worth restating in days the next time they are looked at, for the reason at the top of §1.

---

## 6. The whole model on one line each

- A tick is 1/64 of a real second and 10 sim-seconds. The world runs 640×.
- A sim day is 8,640 ticks — 2¼ real minutes at 1×, 45 seconds at 3×.
- The strip shows `Season Day` and a part of the day. Never an hour.
- A transit does a tile a sim-minute: ten tiles in ten sim-minutes and about one real second.
- A house lot takes half a sim day; a full block takes five.
- The opening line pays its capital back in real minute seven.

---

## 7. Acceptance bar

1. No readout in the game states the current time to the minute.
2. A player watching a train can follow it with their eye across a ten-tile hop.
3. A player who connects a town sees a stake go in within a couple of minutes, and a filled-out district only after ten.
4. Nothing on the status strip and nothing in a panel disagrees about what day it is.
5. Every duration constant in the sim is denominated in a sim day or in a real minute, and its doc comment says which.
