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

1. Select the Line tool.
2. **Click stations in order.** Each click extends the line, and the route between consecutive stops is drawn along the actual track in the line's colour, so the player sees the real path — including where it doubles back or shares a corridor.
3. If two stops aren't connected, the segment draws `warn` with *"No route — Millhaven is not connected to Eastgate."*
4. `Enter` confirms. The line gets a colour from a distinguishable palette rotation and a name suggested from its endpoints — *"Eastgate — Millhaven"* — which the player can override.
5. Assign trains, set frequency, done.

Editing is the same gesture: drag a stop to reorder, click to insert, right-click to remove. A line can be a there-and-back or a loop.

### 2.2 What a line owns

| Property | Meaning |
| --- | --- |
| **Stops** | Ordered, each with a dwell time |
| **Colour** | Identity everywhere — on the map, in panels, on the trains themselves |
| **Trains** | Assigned vehicles |
| **Frequency** | Target headway; the game reports the achieved one |
| **Direction** | Out-and-back, or a one-way loop |

### 2.3 The Line panel

- Colour, name, and a **schematic strip diagram** in the style of a transit map — stops as nodes with waiting counts and current train positions sliding along it. One glance shows bunching, gaps, and where the crowding is.
- **Frequency**: target versus achieved. When achieved is much worse than target, the panel says why — *"Bunching at Eastgate — dwell exceeds headway"* — because a player who can see a problem but not diagnose it cannot fix it.
- **Economics**: revenue, opex, net, per minute, with a trend.
- **Load**: average occupancy per segment, showing which leg is the busy one.
- Actions: add or remove trains, edit route, reverse, duplicate, delete.

---

## 3. Transit and Transport are genuinely different

The vision calls them sidegrades with distinct constraint profiles. That requires them to actually constrain differently — a price difference is not a profile.

| | **Transit** | **Transport** |
| --- | --- | --- |
| Carries | People | Goods |
| Capacity | Modest per car, many cars | Large per car, fewer cars |
| Acceleration | Brisk | Slow — this is the defining trait |
| Top speed | High | Moderate |
| Gradient tolerance | Good | **Poor** — a hill that transit shrugs off, freight cannot climb loaded |
| Curve tolerance | Good | Poor — long trains need generous radii |
| Dwell at stops | Short, scales with crowd | Long, scales with load |
| Stop pattern | Frequent, short hops | Point to point |
| Operating cost | Lower | Higher |
| Drives | Residential and commercial growth | Cash flow and industry |

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

### 4.4 It should reward slack

The vision wants networks built with slack and redundancy to be rewarded, especially against events. That means an alternative route must be *usable* — trains reroute around a blockage when a path exists. A player who built a second way round should watch it save them, and that moment is the reward for having spent money on something that looked unnecessary.

---

## 5. Trains as objects

- **Named**, either automatically or by the player. A named train that has been running the same line for an hour is a thing the player has a relationship with.
- **Composed** of a locomotive and cars, with the length visible in the world — a long freight train *looks* long, and that is what makes gradient and curve constraints feel physical rather than arithmetic.
- **Aging**: gradual increase in operating cost and slight reliability loss over a long life, creating a gentle reason to reinvest. Never a failure — just a nudge, and never a surprise.
- **Purchasable** from a panel that shows the actual stats being compared, with a placement flow that only permits legal locations and says why when one isn't.

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

- **Lines** decide where trains go.
- **Frequency** decides how often.
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
