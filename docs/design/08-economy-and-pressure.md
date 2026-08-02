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

Two design consequences worth stating:

**Fares scale with journey quality, not just distance.** A passenger who waited two minutes and travelled directly pays full; one who waited twelve and transferred twice pays substantially less and complains. This makes service quality a revenue term rather than a separate abstract score, and it means the player is paid for running a *good* railway rather than merely a large one.

**Long hauls pay disproportionately.** Distance should be worth more than linearly, so that reaching further is genuinely lucrative and the pull outward is economic rather than merely narrative.

---

## 3. Costs, and the overextension trap

### 3.1 Track has a running cost

**This is the mechanism the whole "prune and rebalance" idea depends on.** Every tile of track costs a small amount continuously to maintain. Bridges and tunnels cost several times more. Stations cost more still, scaling with tier.

Without it, network size is free to hold, and a player can pave the entire map with no consequence. The design's central economic tension — *"track that isn't carrying enough starts costing more than it earns"* — cannot occur, and the response it calls for has no trigger.

With it, every piece of railway is a small ongoing bet that it will carry enough traffic to justify itself. That is the whole game in one sentence.

### 3.2 The trap has to be recoverable

Overextension must be a **problem to solve, not a death spiral**:

- The **rate readout** (net income per minute) is permanently on screen, so decline is visible long before it is dangerous.
- The **Profit overlay** shows exactly which track and which lines are underwater.
- **Demolition refunds construction cost in full**, so pruning is always available and never punishing.
- Running out of cash **parks trains rather than destroying anything**. Track stays, stations stay, the town stays. When money recovers, service resumes.

The intended experience is: expand ambitiously, notice the rate going negative, open the Profit overlay, find the branch nobody uses, tear it up, recover. That arc should happen several times per session and feel like competence, not punishment.

### 3.3 Other costs

- **Construction**, scaling hard with terrain — see [02 — World & Terrain](02-world-and-terrain.md) §3.1. This is what makes routing a real decision.
- **Rolling stock**, purchased outright with an ongoing operating cost per train, higher for freight.
- **Station operation**, scaling with tier — an interchange nobody uses is a genuine liability.

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

New demand should appear on a rhythm the player can feel — roughly one meaningful new opportunity every few minutes early on, stretching as the network matures. Each one:

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

### 5.3 Stagnation as the only failure

The vision is unambiguous: a quiet town is the worst outcome, and it stays fixable. That means the game must **notice stagnation and say so**:

- When nothing has grown for a sustained period, Town Talk shifts tone — the world starts asking for things more insistently.
- The Density overlay makes a flat town obvious.
- No game-over, no score, no fail screen. Just a world that has gone quiet, and a visible menu of ways to wake it up.

---

## 6. The ledger

Money needs to be **legible**, or the player cannot reason about any of the above.

A panel, opened from the status strip, showing:

- **Income by source** over the last window — fares, deliveries, bonuses.
- **Expenses by category** — track maintenance, station operation, rolling stock, construction.
- **Net rate**, with a history graph long enough to show a trend across a session.
- **Per line** and **per station** contribution, sortable, so the worst performer is one click away.
- **Projection**: at the current rate, how long until the player can afford what they are looking at.

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
