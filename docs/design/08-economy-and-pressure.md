# 08 — Economy & Pressure

**This brief carries the loop.** Art and interaction determine whether the first ten minutes are pleasant. This determines whether there is a tenth hour.

The vision names three economic promises, and each is a mechanism that has to exist for the loop to turn:

> - *A new demand appears somewhere you can't yet reach.*
> - *Overextension inverts it: track that isn't carrying enough starts costing more than it earns.*
> - *Stagnation is the only failure.*

---

## 1. Principle

**Money paces expansion. Demand pulls it. Neither ever ends the game.**

The economy exists to answer one question continuously: *what should I build next?* If the answer is ever "nothing, I'm fine", the game has stopped. If it is ever "nothing, I'm ruined", the game has broken its own promise.

---

## 2. Income

Income comes from throughput — moving people and goods. The shape matters more than the numbers:

| Source | Character |
| --- | --- |
| **Passenger fares** | Small, frequent, scaling with distance and *journey quality* |
| **Goods delivery** | Large, lumpy, scaling with distance and commodity value |
| **Sustained service bonus** | A modest premium for reliability — rewards maintaining, not just expanding |

Three design consequences worth stating:

**Long hauls pay disproportionately.** Distance is worth more than linearly, so that reaching further is genuinely lucrative and the pull outward is economic rather than merely narrative.

A flat fare says the exact opposite, and it is worth recording why, because it was shipped once. If a four-tile hop and a sixty-tile haul pay the same, the hop wins on every axis — less track to lay, less to maintain, more runs a minute — and the optimal railway is a tram circling one square. Every other system in the game pulls outward; a flat fare pulled back harder than any of them pushed.

So a payout carries a squared term as well as a linear one, steeper for goods than for passengers, and the surplus at distance is what makes an expensive alignment worth costing out. **The distance paid on is the Chebyshev separation of the two endpoints, not the length of the route the train took.** Paying for route length would pay for winding track — a player could earn more by building worse — and would bill the empty repositioning leg as revenue. Endpoint separation pays for *reaching further* and leaves a direct alignment strictly better than a rambling one: same fare, less track, less upkeep, more runs an hour.

**Fares should also scale with journey quality, and do not yet.** A passenger who waited two minutes and travelled directly should pay full; one who waited twelve and transferred twice should pay substantially less and complain. That would make service quality a revenue term rather than a separate abstract score, and it is the missing half of being paid for a *good* railway rather than merely a large one.

---

## 3. Costs, and the overextension trap

### 3.1 Track has a running cost

**This is the mechanism the whole "prune and rebalance" idea depends on.** Every tile of track costs a small amount continuously to maintain. Bridges and tunnels cost several times more. Stations cost more still, scaling with tier.

Without it, network size is free to hold, and a player can pave the entire map with no consequence. The design's central economic tension — *"track that isn't carrying enough starts costing more than it earns"* — cannot occur, and the response it calls for has no trigger.

With it, every piece of railway is a small ongoing bet that it will carry enough traffic to justify itself. That is the whole game in one sentence.

**Every running cost is per _real_ minute — the clock the player is sitting at.** The world runs 640× faster than the wall, and the two minutes are easy to confuse in a way that is not cosmetic: read as sim-minutes, every authored rate collects at 1/640 of its stated value, upkeep lands at about 3% of gross income, one train pays for five thousand tiles of dead track, and this section's trap cannot occur at all. That is not hypothetical — it shipped, and it is why the status strip, the ledger and every constant behind them are stated in the minutes the player is living in.

**Only stations the railway actually reaches are billed.** A stop with no railhead under it is not a stop the railway is maintaining; it is a town on the map. The distinction is load-bearing because two kinds of station arrive without the player building anything — the opening anchors, and the settlement the world plants every few minutes for the rest of the session, *unconnected by definition*, since being unconnected is what makes it an opportunity. Billing those was a slow, invisible tax on doing nothing: a fresh world opened owing upkeep on three towns the player had not reached, and every marker the world put down added more, forever, whether or not it was ever served. §3.3's liability is *"an interchange nobody uses"* — something the player chose and paid for — not a village the world invented. Anything the player builds is always billed: a stop refuses to be placed without track under it.

### 3.2 The trap has to be recoverable

Overextension must be a **problem to solve, not a death spiral**:

- The **rate readout** (net income per minute) is permanently on screen, so decline is visible long before it is dangerous.
- The **Profit overlay** shows exactly which track and which lines are underwater. *(Designed, not built — the three overlays that ship are service, congestion and density. Until it exists the rate readout is doing this job alone, which is the weakest link in the arc below.)*
- **Demolition refunds construction cost in full**, so pruning is always available and never punishing. The same is true of rolling stock: a train sells back for what it cost.
- Running out of cash **destroys nothing and stops nothing**. The balance floors at zero, unpaid upkeep is simply not collected, and **the trains keep running**. Track stays, stations stay, the town stays; only paid construction is blocked.

**Trains are explicitly not parked for lack of money, and this brief used to say they were.** Trains are the only source of income, so parking them at zero makes bankruptcy permanent — the player can never earn their way out, and *"money paces expansion, it never ends the game"* becomes false at exactly the moment it matters. Leaving them running makes recovery automatic, and turns "prune and rebalance, not start over" into something the player can act on rather than advice they can only read. Debt would be a second way to lose, and there is not one.

The intended experience is: expand ambitiously, notice the rate going negative, find the branch nobody uses, tear it up, recover. That arc should happen several times per session and feel like competence, not punishment.

### 3.3 Other costs

- **Construction**, scaling hard with terrain — see [02 — World & Terrain](02-world-and-terrain.md) §3.1. This is what makes routing a real decision.
- **Rolling stock**, purchased outright with an ongoing operating cost per train, higher for freight. Idle stock is therefore a slow leak, which is the point.
- **Station operation**, scaling with tier — an interchange nobody uses is a genuine liability. Only stops with a railhead under them are billed (§3.1).

**Capital sits an order of magnitude above the small change, deliberately.** A tile of plain track and a train are bought in *units of decision*, not units of currency, and the scale was lifted so that a purchase reads as a commitment against the balance rather than as a rounding error against it — currently $100 a tile of flat track and $3,000 for a transit, against $10,000 of opening cash on Standard (half that on Lean, double on Generous).

**Treat every figure in this brief as tunable and the relationships as not.** The numbers move whenever the economy is measured against a running sim, and they should; what must not move is that a first line is affordable in the first minute, a second train is a decision, and an eight-tile crossing is a project. When prose and code disagree about a figure, the code is right and the prose is stale.

---

## 4. The world creates new demand

**This is the missing rung**, and it is the single most important mechanism in this brief. The design's ten-minute promise is *"a new demand appears somewhere you can't yet reach."* Without it, a player who connects everything available has finished, and there is no hour-long arc.

### 4.1 Sources of new demand

| Source | Behaviour |
| --- | --- |
| **New settlements** | Appear over time at sites the generator reserved, preferentially *outside* the currently served network. They start small, and they grow much faster once connected. |
| **New industries** | Open where their resource is, often deliberately awkward — up a valley, across a river, past a ridge. |
| **Growth caps** | A district that has grown to the limit its connectivity supports asks for better service. |
| **Derived demand** | A served industry creates demand for its inputs and outputs, chaining outward. Connecting a sawmill creates a reason to reach a forest and a reason to reach a builder. |
| **Population pressure** | A thriving town generates journeys to places it currently cannot reach. |

### 4.2 The rhythm

New demand should appear on a rhythm the player can feel — roughly one meaningful new opportunity every few minutes early on, stretching as the network matures. Early on the player has nothing to do and wants the world to speak up; an hour in they have a railway to run and a fresh marker every four minutes is nagging, so the gap widens with each opportunity and then stops widening. A ceiling, not an end: §4 is explicit that a player who has connected everything must never be finished, so the world keeps asking — just less often.

**The cap is on the board, not on the session.** At most **three** unconnected opportunities may stand open at once; connect one and the next is free to appear. A lifetime cap was the earlier design and it was the wrong shape entirely — it was the whole session's supply, so the world fell permanently silent about half an hour in, which is precisely the missing rung this section exists to install. Three unanswered markers is a menu; a dozen is wallpaper, and a player who is ignoring the world should not drown in reminders that they are ignoring it.

Each opportunity:

1. **Announces itself** in Town Talk with a location and a reason.
2. **Is visible** on the map, and in Map View is unmissable.
3. **Is reachable but not trivially** — far enough to require a real decision, close enough to be tempting.
4. **Is optional.** Nothing punishes ignoring it. But the pull should be strong enough that ignoring it feels like a choice.

### 4.3 The pull outward

Crucially, new demand should appear at **increasing distance** as the network grows. Early opportunities are a few tiles away; later ones are across the map, past terrain that was previously prohibitive. This is what makes tunnels, long bridges, and expensive alignments become worthwhile *over the course of a session* — the terrain that was impossible at minute five is the interesting problem at minute fifty.

That progression is the game's difficulty curve, and it comes from the world rather than from a multiplier.

---

## 5. Pressure

### 5.1 Congestion

The standing puzzle — covered in [07 — Trains & Lines](07-trains-and-lines.md) §4. Economically, congestion shows up as revenue that stops scaling with the trains you add, which is the signal to fix the corridor rather than buy more stock. That lesson should be learnable from the numbers alone.

### 5.2 Events

Events target the network, never the town, and they exist to reward slack and redundancy.

| Event | Effect | Rewards |
| --- | --- | --- |
| **Landslide** | Closes a section, needs clearing | Alternative routes |
| **Flood** | Takes out a bridge or low-lying track | Not putting everything in one valley |
| **Festival** | Demand spike at one station | Spare capacity |
| **New industry opens** | Opportunity, off the network | Cash reserves |
| **Harsh winter** | Slower trains, higher costs, for a season | Margin |

Rules for events: they are **announced with time to react** where possible, they are **always recoverable**, they **never destroy the town**, and they **never cascade** into an unrecoverable state. An event should provoke an interesting hour, not end a session.

Frequency scales with network maturity — a small network is left alone, because the early game's job is teaching, not testing.

**None of this table exists yet.** The director that would run it is a stub. Festival (a demand spike at one station) and landslide (a temporary closure) are the cheapest two, and they are the two worth doing first because between them they exercise the whole announce → react → recover shape that the rest of the table is variations on.

### 5.3 Stagnation as the only failure

The vision is unambiguous: a quiet town is the worst outcome, and it stays fixable. That means the game must **notice stagnation and say so**:

- When nothing has grown for a sustained period, Town Talk shifts tone — the world starts asking for things more insistently.
- The Density overlay makes a flat town obvious.
- No game-over, no score, no fail screen. Just a world that has gone quiet, and a visible menu of ways to wake it up.

---

## 6. The ledger

Money needs to be **legible**, or the player cannot reason about any of the above.

A window, opened from the menu row's `Ledger` button or `K`, showing:

- **Income by source** over the last window — fares, deliveries, bonuses.
- **Expenses by category** — track maintenance, station operation, rolling stock, construction.
- **Net rate**, with a history graph long enough to show a trend across a session.
- **Per line** and **per station** contribution, sortable, so the worst performer is one click away. *(Not built — the ledger records categories, and money is not tagged with the line that earned it. It is the same gap as [07](07-trains-and-lines.md) §2.3's economics row, and §9's acceptance bar 5 is not currently met.)*
- **Projection**: at the current rate, how long until the player can afford what they are looking at. *(Not built.)*

That last one is quietly important. A player deciding whether to commit to an expensive tunnel wants to know *"how long do I have to wait?"*, and answering it turns a vague reluctance into a plan.

---

## 7. Pacing targets

Rough shape of a session, as a design intention rather than a tuning table:

| Time | Player state |
| --- | --- |
| 0–2 min | First line connected, first payout received |
| 2–10 min | Three or four stations, first goods route, first profit |
| 10–25 min | First congestion problem, first pruning decision, first new settlement appears |
| 25–60 min | Multiple lines, deliberate express and local routing, first expensive terrain commitment |
| 1 hr+ | A network with shape and history, reaching places that were prohibitive at the start |

Real minutes throughout, and the first row is the only one that is binding rather than intended: [02](02-world-and-terrain.md) §4.1 pins the opening beat and it is measured against a running sim rather than trusted.

The test of the whole economy: **at every point on that timeline, the player should have a next thing they want to build and a reason they can't quite do it yet.** That gap is where the game lives.

---

## 8. Goals mode

The same systems with objectives and deadlines, per the vision. Objectives are drawn from what the sandbox already produces — population thresholds, delivery quotas, connect these places by this date, keep this district served for this long.

Deadlines make the pacing bite, and that is the only difference. No separate systems, no special rules. A goal is a lens on the sandbox, which means every improvement to the sandbox improves goals mode for free.

---

## 9. Acceptance bar

1. A player who paves the whole map goes broke and can see exactly why.
2. Pruning an unprofitable branch visibly restores the rate, within a minute.
3. At any moment in a session, there is a visible opportunity the player wants and cannot yet afford.
4. Running out of money is annoying and recoverable, never terminal.
5. A player can find their worst-performing line in under ten seconds.
6. The terrain that was impossible in the first minute is worth conquering by the fiftieth.
