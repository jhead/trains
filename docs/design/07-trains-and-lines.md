# 07 — Trains & Lines

**The gap this brief closes:** the player builds a railway and then has no way to *operate* one. Track is only half a network — the other half is deciding what runs where, how often, and what happens when two trains want the same rail. Without that, trains are autonomous agents the player watches, and the strategy layer the design promises has nowhere to live.

---

## 1. Principle

**The player builds the railway and runs the railway.** Track is the geography; lines are the decisions.

A train assigned automatically to whatever demand exists is a delivery drone. A train assigned by the player to a named route, at a frequency they chose, competing for track with their other trains, is a railway. The second is the game.

---

## 2. Lines

A **line** is a named, coloured, ordered sequence of stations that trains are assigned to run. It is the central object of the operations layer, and it is the thing the player actually thinks about once track is down.

### 2.1 Creating one

Line creation is a drawing gesture, matching the build tool's feel:

1. **`L`** selects the Line tool.
2. **Click stations in order.** Each click extends the line, and the route between consecutive stops is drawn along the actual track in the line's colour, so the player sees the real path — including where it doubles back or shares a corridor. Right-click takes the last stop back off the draft.
3. If two stops aren't connected, the segment draws `warn` with *"No route — Millhaven is not connected to Eastgate."*
4. **`Enter`** confirms. The line gets a colour from a distinguishable palette rotation and a name suggested from its endpoints — *"Eastgate — Millhaven"* — which the player can override. Town Talk says the line opened and what to do next.
5. **Crew it.** The tool puts itself down, hands the pointer back to Look, and focuses the new line; clicking a train in the world then assigns it to that line, and the assignment is announced. A focused row says so in words, because a mode the player is in and cannot see is a mode they will be surprised by; `Esc` leaves it.

Every exit from the tool — `Enter`, `Esc`, or another verb taking the pointer — hands the world back clickable. Confirming a line used to leave the pointer parked in a half-state, so the player's next click on a train did nothing at all, with no message: the line they had just drawn could not be crewed, which is the one thing they drew it for.

**A duplicate is refused, and an exact reverse is a duplicate.** An out-and-back is one service, not two, and a player who draws the return leg as its own line gets told so rather than getting a second row that quietly competes with the first for the same trains.

Removal is per-row from the Lines panel and goes through the confirm dialog, because a line is a decision several other decisions hang off.

Editing in place — drag a stop to reorder, click to insert — is designed and not built. A line can be a there-and-back or a loop.

### 2.2 What a line owns

| Property | Meaning | Built? |
| --- | --- | --- |
| **Name** | Suggested from the endpoints, overridable | yes |
| **Stops** | Ordered | yes |
| **Colour** | Identity everywhere — on the map, in panels, on the trains themselves | yes |
| **Trains** | Assigned vehicles | yes |
| **Direction** | Out-and-back, or a one-way loop | yes |
| **Frequency** | Target headway; the game reports the achieved one | **no** |

A line survives losing its route: demolish the far end and it goes **dormant** rather than being deleted. It is the player's named object, its trains still point at an id that resolves, and putting the stop back makes it run again. Deleting it would strand every train assigned to it on a line that no longer exists — the tidier behaviour is the destructive one.

Dwell is currently a property of the train kind rather than of the stop, which is the same debt as §3's last two rows.

### 2.3 The Line panel

- Colour, name, and a **schematic strip diagram** in the style of a transit map — stops as nodes with waiting counts and current train positions sliding along it. One glance shows bunching, gaps, and where the crowding is.
- **Frequency**: target versus achieved. When achieved is much worse than target, the panel says why — *"Bunching at Eastgate — dwell exceeds headway"* — because a player who can see a problem but not diagnose it cannot fix it. *(Not built; §2.2.)*
- **Economics**: revenue, opex, net, per minute, with a trend. *(Not built — the ledger is category-level, and money is not tagged with the line that earned it. This is the one thing on this list a player asks for by name; see [08](08-economy-and-pressure.md) §6.)*
- **Load**: average occupancy per segment, showing which leg is the busy one. *(Waits on capacity — §3.)*
- Actions: crew from the world by clicking a train, remove a line through the confirm dialog. Edit route, reverse and duplicate are designed, not built.

**Nothing in this panel fails quietly.** It once had a single control and a single response to being pressed with nothing selected: none. A playtester pressed it, nothing happened, and there was no way to tell whether the game had heard them. Brief 04 §3's rule is not a build-tool rule — every refusal anywhere writes a sentence where the player is already looking.

---

## 3. Transit and Transport are genuinely different

The vision calls them sidegrades with distinct constraint profiles. That requires them to actually constrain differently — a price difference is not a profile.

| | **Transit** | **Transport** | Built? |
| --- | --- | --- | --- |
| Carries | People | Goods | yes |
| Top speed | High | Moderate | yes |
| Gradient tolerance | Good | **Poor** — a hill that transit shrugs off, freight cannot climb loaded | yes |
| Curve tolerance | Good | Poor — long trains need generous radii | yes |
| Dwell at stops | Short | Long | yes |
| Operating cost | Lower | Higher | yes |
| Stop pattern | Frequent, short hops | Point to point | emergent |
| Drives | Residential and commercial growth | Cash flow and industry | emergent |
| **Capacity** | Up to three cars, a load each | One wagon — see §3.4 | **yes** |
| **Acceleration** | Brisk | Slow — this is meant to be the defining trait | **no** |

**Capacity is built and acceleration is not.** A train is no longer one job: a transit couples up to three cars, each carrying one more load, and the consist costs it time on the road and time at the platform. What is still missing is acceleration — a train crosses tiles at a rate and does not build up to it — so the "defining trait" claim in the last row is still a claim rather than a behaviour. The profile has the seam for it (`base_ticks` is a flat rate, with no ramp in front of it) and the consist penalty rides on that same field, which is the honest half-measure: a longer freight train is slower everywhere rather than slower *away from a stop*, which is where the difference should really bite.

### 3.1 What a car does

**Every car past the first carries one more load, and makes the train a tick slower per tile and slower to board.** That is the whole model, and it is one sentence because a player has to be able to recite it.

The numbers live in `TrainProfile` next to the ones they modify:

| | Transit | Transport |
| --- | --- | --- |
| Ticks a flat tile, one car | 6 | 10 |
| …per extra car | **+1** | +2 |
| Dwell, one car | 4 | 12 |
| …per extra car | **+2** | +6 |
| Longest consist | **3 cars** | 1 wagon |

A two-car transit therefore runs at 6/7 of a single car's speed and boards half again as slowly; a three-car at 3/4 speed and double the dwell. The platform grade scales the dwell it is given, so a long consist is exactly what makes an Interchange worth its price — **the station tier is a cost on length, never a cap on it**. Tying the cap to tiers was considered and rejected: a free-roaming train has no fixed set of stops, so a tier-derived limit would be a number the player cannot see, predict, or plan around, and it would change under them when they demolished a halt three towns away.

### 3.2 Where a queue comes from

A car is only worth having when there is a queue for it, and the queue is real demand that used to be discarded. A pair of stops used to hold **one** open job: a second traveller wanting the same trip while one was already posted was dropped. Peep departures now stack instead, three deep — `MAX_PENDING_PER_PAIR`, the same number as the longest transit, so no carriage exists that the board can never fill. The station-pair walk is deliberately left alone at one per pair: that is a synthetic heartbeat so a new line has something to carry, and letting it stack would mint fares out of the spawn interval.

A train boards **one working** and fills as many cars as that pair's queue can supply. Whatever it cannot lift stays posted for the next train, which is what makes a second train and a second carriage genuinely different purchases.

### 3.3 What a car costs, and when it beats a second train

A car is **half a train**: `$1,500` against a transit's `$3,000`. Selling returns the whole consist, so lengthening a train is as reversible as laying track.

Measured on a ten-tile double-track corridor (`rail_sim/tests/consist_capacity.rs`), per real minute:

| | one car | **+ a car** | + a second train |
| --- | --- | --- | --- |
| Thin line — one working per pair | $1,370 | **$1,250** (−$120) | $1,500 (+$130) |
| Busy line — a queue three deep | $826 | **$1,696** (+$870) | $1,990 (+$1,164) |
| Payback, thin | — | **never** | 23 min |
| Payback, busy | — | **1.7 min** | 2.6 min |

**On the opening beat the car is a loss and the second train is not.** That is the design: the first car is not the correct opening move, and it is not gated by a price or a tech tree — it is gated by there being nobody on the platform for it. The lever turns rational the first time a stop's queue is deeper than one carriage, which the pair walk alone never produces; it takes a district generating repeat departures, and [17 §5](17-time-and-pacing.md) puts a district's growth at days rather than minutes.

Once the queue is there the car is the cheaper way to lift it — half the capital, faster payback — and the second train is still the more *flexible* one: it serves another pair, and it keeps running when the first is held. Neither dominates, which is the point.

### 3.4 Why freight runs one wagon

`max_cars` is `1` for Transport, and that is a statement about the world rather than about the locomotive. A car pays when there is a queue, and freight has no queue to have: an industry produces and consumes a good with no stockpile behind it, so the board carries exactly one working per producer→consumer pair however long the train takes to come back. A second wagon would be permanently empty and permanently slowing the train down, sold at a price the player could never earn back — a trap with a price tag.

So the seam is filled in and the number is one. Give an industry a stock level and this becomes two lines: raise the cap, and let goods jobs stack the way passenger jobs do. Until then the Trains window does not offer the verb on a goods train, and the sim says why if the command arrives anyway.

The consequences are the interesting part, and they should be discoverable by playing rather than by reading:

- **A route that's fine for passengers may be unusable for freight.** Terrain that transit ignores stops a loaded goods train, which means the same map poses two different routing problems and the player may need two different alignments.
- **Freight's slow acceleration makes frequent stops catastrophic**, which naturally pushes goods onto express alignments and passengers onto local ones.
- **Mixing them on one corridor is exactly the legitimate strategy and legitimate mistake** the vision describes: it works when traffic is light and produces vicious congestion when it isn't, because the slow accelerator holds up everything behind it.

---

## 4. Congestion is the standing puzzle

Congestion only functions as a puzzle if it is **visible, diagnosable and solvable**. All three are required; the vision explicitly wants it to be "a design problem rather than a disaster."

### 4.1 Visible

- A blocked train shows a **stop indicator** and its smoke goes idle.
- Track under sustained heavy use tints in the **Congestion overlay**, and its railhead visibly gleams in the world art.
- A queue of waiting trains is drawn as what it is: a queue.

### 4.2 Diagnosable

Selecting a blocked train says what is blocking it and offers to select that. Following the chain to its head takes seconds. Congestion the player cannot trace is congestion they can only respond to by superstition.

### 4.3 Solvable

The player needs real tools, or congestion is just a cap:

- **Passing loops** — a second track beside a single line so trains can pass. The classic, cheap, and it teaches the concept.
- **Double track** — expensive, permanent, the real answer to a busy corridor.
- **Signals** — divide a line into blocks so several trains can follow at spacing rather than one-at-a-time. This is the deepest tool and should be introduced last, but its absence caps how interesting a network can get.
- **Better junctions** — grade separation to stop crossing moves fouling each other.
- **Rescheduling** — fewer, longer trains instead of many short ones.

Each of these is a distinct strategy with a distinct cost, which is what makes a congested corridor an interesting problem rather than a fail state.

**Today only the first two exist, and there is one shape they do not cover.** A single-track ring can deadlock outright — trains meet nose to nose with nowhere to pass, and nothing in the sim breaks the tie. An alert names the situation and names the fix, which is the honest minimum, but *naming* a deadlock is not *resolving* one. The real remedy is movement work: a train that backs out, or one that refuses to enter a corridor it cannot leave. Signals are the depth extension on top of that, not a substitute for it.

### 4.4 It should reward slack

The vision wants networks built with slack and redundancy to be rewarded, especially against events. That means an alternative route must be *usable* — trains reroute around a blockage when a path exists. A player who built a second way round should watch it save them, and that moment is the reward for having spent money on something that looked unnecessary.

---

## 5. Trains as objects

- **Named**, either automatically or by the player. A named train that has been running the same line for an hour is a thing the player has a relationship with.
- **Composed** of a locomotive and cars, with the length visible in the world — a long freight train *looks* long, and that is what makes gradient and curve constraints feel physical rather than arithmetic.
- **Aging**: gradual increase in operating cost and slight reliability loss over a long life, creating a gentle reason to reinvest. Never a failure — just a nudge, and never a surprise. *(Designed, not built.)*
- **Bought and placed with one verb.** `T` and `G` arm placement for transit and goods; a click on a station tile puts a train down. The verb asks the **yard first** and only reaches for the bank when there is nothing there to place, which is why the slots read *place / buy* rather than *buy*. Pressing it used to purchase every time, so a player holding an unplaced train and less than its price in the bank got a failed purchase — reading as *"I can't place a train, I'm stuck"* — while a free placement was already armed and a train of theirs was sitting invisibly in the yard. A verb that is honest at any balance is worth more than one that is simple.
- **Sold** with `X` on a selected train, after a confirm, for the **full purchase price**. Anything it was carrying goes back on the job board rather than vanishing with it. Rolling stock is a reversible decision for the same reason track is: the game wants the player experimenting with the shape of their network, and an irreversible purchase is a reason to stop.
- **Never deleted by the world.** A train left standing where its track used to be is recalled to the yard, not destroyed. The player paid for it.
- Purchase, entering service, and sale all say so in Town Talk — a purchase as an opportunity (there is now something to do), a placement as praise (the thing got done).

---

## 6. Movement must be continuous

**Trains move smoothly between tiles, always.** A train that snaps from tile to tile reads as broken software regardless of how good everything around it is, and it is the loudest possible signal that a game is a prototype.

The requirements:

- Position interpolates continuously along the track between graph nodes, using sub-tile progress.
- Facing comes from the direction of travel and selects a sprite from the sixteen-direction bank — never a runtime rotation.
- Cars follow the locomotive along the path it took, so a train articulates correctly through curves. A train bending around a curve is one of the most satisfying things in this genre and it comes almost free once cars follow a path history.
- Acceleration and braking are visible: a train eases out of a station and slows into one.
- Smoke is emitted along the path, drifting and dissipating.

Rendering interpolates between simulation ticks. The simulation stays fixed-step and authoritative; the presentation is free to be smooth, and must be.

---

## 7. Automation, but the player's automation

The player should not micromanage individual journeys — they set up a system and watch it run. But it must be *their* system:

- **Assignment is the switch between the two behaviours.** A train on a line patrols its stops and prefers work that lies along it. A train on no line free-roams the job board and takes whatever is nearest. Both are useful — the free-roamer is what makes the first ten minutes work before any line exists — and choosing between them is the player's first operational decision.
- **Lines** decide where trains go.
- **Frequency** decides how often. *(Designed, not built — a line has no headway yet, and the number of trains on it is the only lever.)*
- Passengers choose among the lines the player has provided, including transfers, and their choices are visible in the data. Someone taking a two-transfer journey because the direct link doesn't exist is a design signal the player can act on.
- **Goods** flow along the routes the player set up, with sensible defaults so a freight line "just works" once drawn.

The right feeling: the player is a network planner, not a dispatcher. Every hour of play should involve a few high-leverage decisions rather than a stream of small ones.

---

## 8. Acceptance bar

1. Trains move smoothly, articulate through curves, and never teleport.
2. A player can draw a line through four stations in under fifteen seconds.
3. A blocked train can be traced to its cause in under ten seconds.
4. Transit and transport demonstrably want different routes across the same terrain.
5. A congested corridor has at least three distinct viable fixes.
6. A player can tell which of their lines is losing money without doing arithmetic.
