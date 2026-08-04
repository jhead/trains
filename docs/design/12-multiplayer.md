# 12 — Multiplayer: Neighbour Maps

**The promise, from the vision:**

> Maps sit next to other players' maps. Tunnels at the map edge connect to a neighbor's network; trains cross over carrying goods and come back with theirs. Contributions are additive — a neighbor's line makes your town better, and a neighbor's absence never blocks it. This works asynchronously, so a single-player town is still connected to somebody.

Four sentences, and two of them are hard constraints that determine the entire architecture. This brief takes them literally.

---

## 1. What this is, and what it is not

**It is not multiplayer in the usual sense.** There is no shared world, no lobby, no session, no co-op, no competition, no synchronous play. Two people are never in the same simulation at the same time.

**It is a trade relationship between two private worlds.** You build your railway. Your neighbour builds theirs. Neither of you can touch the other's map. The only thing that ever crosses the border is a train.

The nearest reference points are Animal Crossing's island visits and Death Stranding's asynchronous infrastructure — a persistent sense that somebody else is out there, delivered without either player having to be present.

Three design consequences follow immediately, and each one saves an enormous amount of engineering:

| Because | We never need |
| --- | --- |
| Nothing crosses but manifests, applied as ordinary commands on your own tick | Lockstep, rollback, prediction, or determinism across machines |
| Neither player can affect the other's world state | Authority arbitration, anti-cheat on simulation, or server-side game logic |
| Everything is asynchronous by construction | Latency budgets, connection quality handling, or netcode in the hot path |

The fixed-tick command architecture and the map-edge portals were designed as seams for exactly this. They stay seams: a border manifest arriving from a neighbour becomes a command in your buffer, indistinguishable from one you issued yourself.

---

## 2. The two hard constraints

Everything below is downstream of these. When a design question is ambiguous, resolve it by whichever answer protects them.

### 2.1 A neighbour's absence never blocks you

Your neighbour may be offline for a week, may never launch the game again, may have deleted their map, may be on an incompatible version. **In every one of those cases your game continues exactly as before, and trade continues.** Not degraded-with-a-warning — continues.

This rules out any design where you wait on them for anything. No pending states, no "awaiting neighbour", no blocked trains sitting at a portal. If a train reaches the border, it leaves. Something comes back.

### 2.2 Contributions are strictly additive

A neighbour can only ever make your game better or leave it unchanged. There is no mechanism by which they make it worse. Concretely, a neighbour can never:

- occupy, block, or demolish a tile on your map
- take, spend, or cost you money
- create congestion on your track
- reduce a service score, a growth rate, or a payout
- see anything you have not explicitly published
- send you anything you did not ask for

The worst possible neighbour is a silent one, and a silent one is identical to solo play. This is what makes the whole feature safe to ship without moderation infrastructure, and it is not negotiable in service of any gameplay idea.

---

## 3. Borders

### 3.1 Edges and links

A map has four edges. Each edge can host **one border link**. A link is a paired connection to exactly one neighbour, so a fully-connected map has four neighbours — enough to feel populated, few enough to stay comprehensible.

An edge with no link is closed: the map boundary, as it is in solo play today.

Opening an edge is a **construction project**, and it should feel like one. The player builds a line to the edge and then pays to open the portal — a tunnel mouth or a bridge abutment at the boundary, expensive enough to be a real commitment competing with domestic expansion. It is the most distant, least immediately useful thing you can build, and it should pay off over hours.

### 3.2 The Border Yard

You never see your neighbour's map. You see the **Border Yard**: a strip of world beyond your edge, past the portal, showing their side of the connection.

- Their track approaching from the far side.
- Their trains arriving and departing.
- The silhouette of their town on the horizon — enough to read as a place, not enough to be their map.
- A sign with their town's name.
- What they are currently offering to trade.

The yard is rendered from the small amount of border data they publish, not from their world. This is what delivers *"there is somebody over there"* without syncing a world, and it sidesteps the privacy question entirely: nothing appears in your yard that they did not publish for exactly this purpose.

At night, their town's lights are on across the border. That single detail will do more for the fantasy than any amount of interface.

---

## 4. What crosses

### 4.1 The unit of exchange

Not world state. Not chunks. A small, versioned **Border Manifest**:

| Field | Meaning |
| --- | --- |
| Link identity and schema version | Routing and compatibility |
| Departures | What left your map through this portal, and when |
| Standing offer | What you will supply, in what quantity, per period |
| Standing request | What you would like to receive |
| Published presence | Town name, a headline stat or two, the border silhouette |
| Sequence number | For ordering and idempotent replay |

Kilobytes, not megabytes. It contains nothing that could reconstruct your map, which is both a privacy property and the reason exchange stays cheap enough to run over trivial infrastructure.

### 4.2 The trade loop

1. A train on your map is assigned to a **border line** and runs to the portal.
2. It enters the portal and leaves your simulation, entering **transit**.
3. Its cargo is added to your outbound manifest.
4. Some time later — minutes, or days — a train arrives from the portal carrying goods from their standing offer, and runs into your network as ordinary freight.

The asymmetry of time is the design's friend rather than its problem. You send lumber and, tomorrow, ore comes back. A border route is a slow, patient thing, and that suits the game's temperament exactly.

### 4.3 Why you would care

Trade has to be genuinely worth the expense, or the border is decoration.

**Each map lacks something.** Terrain generation should guarantee that every map is missing one or two commodities its industries want. That scarcity is the entire economic case for a neighbour, and it must be built into world generation rather than bolted on — see [02 — World & Terrain](02-world-and-terrain.md).

**Border goods are worth more.** What comes across is what you cannot produce, so it unlocks industry chains that are otherwise closed to you.

**Relationships mature.** A link that has been running a long time trades at better rates. This rewards the long, patient investment the feature is built around, and it gives a returning player something that grew while they were away.

---

## 5. Asynchrony: how trade continues with nobody there

This is the mechanism that makes constraint 2.1 true, and it is the cleverest part of the design.

**The neighbour's standing offer is cached locally.** When you last heard from them, they published what they supply. That offer persists. Your game generates their return trains from the cached offer, on your own tick, with no network involved.

So:

- Neighbour online and active → exchange is live, offers update, everything is current.
- Neighbour offline for a week → their last standing offer keeps supplying you. Trade continues.
- Neighbour never returns → the link quietly becomes an **echo** (§6). Trade continues.
- Version mismatch or corrupt data → reject the manifest, fall back to cache. Trade continues.

Reconciliation on reconnect resolves any drift, and **it always resolves in the player's favour.** Nothing is ever clawed back, no goods vanish retroactively, no balance is corrected downward. A conservative over-delivery is vastly cheaper than the trust damage of taking something away.

There is no state in which the player is waiting.

---

## 6. Echo neighbours — how solo play is "still connected to somebody"

The vision insists a single-player town is connected. Taken seriously, that means **the feature must work with no network at all.**

An **echo neighbour** is a persistent, named, simulated trading partner behind a border link. It has a town name, a border silhouette, a standing offer, a trading rhythm, and it grows slowly over time. It behaves exactly like a real neighbour, because from your side a real neighbour is only ever a manifest anyway.

Echoes come from either:

- **A generated partner**, seeded deterministically from your map seed and the edge, so it is stable and reproducible.
- **A donated snapshot** — the published border data of a real player's map, anonymised and opted into. Somebody else's actual railway, running as your neighbour, without either of you being online.

Echoes are not a fallback or a consolation. **They are the default experience**, and a real linked friend is the upgrade. Designing it this way means:

- The entire feature ships and is fully playable with zero networking.
- Solo players get the complete fantasy.
- There is no empty-lobby problem, ever.
- Real links are strictly additive on top — which is the same guarantee the vision demands of neighbours themselves.

An echo is always honestly labelled in the interface. Not deceptive, just present.

---

## 7. Pairing

| Method | How | Priority |
| --- | --- | --- |
| **Echo** | Automatic on opening an edge | Default |
| **Friend code** | Short shareable code; both players attach it to a chosen edge | Primary real link |
| **Community pool** | Opt-in matching with a stranger's published border | Later |

Friend codes are the real target: two people who know each other, deliberately connecting their towns, is the version of this that has emotional weight. The community pool is a scale feature and can wait.

Replacing a link is always allowed and never destructive — swapping a neighbour keeps your track, your portal, and your goods.

---

## 8. Infrastructure

**Deliberately, aggressively dumb.**

- A blob store keyed by link identity. Put a manifest, get a manifest.
- **No authoritative server. No game logic server-side.** The server cannot validate a railway and should not try.
- Offline-first: everything reads from local cache, and the network is an opportunistic refresh.
- Self-hostable, and the game is fully playable with the endpoint unset.
- Exchange is polled on a slow, jittered interval measured in minutes. Nothing is real-time, so nothing needs to be.

Because a neighbour can only add to your world, a malicious manifest's blast radius is "you receive goods you didn't expect." Clamp quantities to sane bounds, reject unknown schema versions, and the threat model is close to empty. That is a direct dividend of constraint 2.2 and it is worth protecting.

### 8.1 Safety

- **No free-text chat.** None. Town names come from a curated generator or are filtered, and there is no other channel. This removes an entire category of moderation burden that a game this size cannot carry.
- Nothing is published without explicit action — opening a link is consent, and it is revocable.
- Published data is border-only and cannot reconstruct a map.
- Links are severable at any time, from either side, with no penalty.

---

## 9. Interface

**The Neighbours panel** — your four edges, each showing its link (echo or real), the partner's town name, what flows each way, the relationship's maturity, and a link/unlink action.

**The Border Yard view** — fly to an edge and watch the exchange happen. Trains arriving from somewhere you cannot go.

**The Trade agreement panel** — set your standing offer and request per link. Deliberately simple: what you'll send, what you want.

**Town Talk carries the border.** *"A train from Ashcombe brought ore."* *"Ashcombe is asking for lumber."* The feed is already the game's ambient voice ([05 — Inspection & Overlays](05-inspection-and-overlays.md) §4), and border events belong in it rather than in a separate notification system.

---

## 10. Phasing

Each phase is independently shippable and independently valuable — a property worth more than any individual feature here.

| Phase | Contents | Network needed | State |
| --- | --- | --- | --- |
| **MP-1 — Borders & Echoes** | Portal opening as construction, border yard, transit, manifests, standing offers, generated echo neighbours, Neighbours panel | **None** | **shipped** |
| **MP-2 — Friend links** | Friend codes, blob exchange, cache and reconciliation, presence, donated snapshots | Yes | deferred |
| **MP-3 — Community** | Opt-in stranger pool, relationship maturity, border-scale trade economy | Yes | deferred |

**MP-1 delivered most of the fantasy and required no networking whatsoever** — which was the whole argument for building it first, and it held. Every neighbour in the game today is an echo: a pure function of the map's seed and the edge it stands behind, regenerated rather than stored, so nothing about it can drift from what it says it is. Nothing anywhere in the build talks to another machine.

MP-2 is where the infrastructure starts, and it is deferred rather than dropped: it wants somewhere to put a blob, and that is the only thing standing between the current build and a real friend on the other side of a portal. If it never happened, MP-1 would still be a good feature — which is the correct shape for a risky system, and it is now demonstrated rather than argued.

---

## 11. Rejected, and why

| Option | Verdict |
| --- | --- |
| Real-time co-op on one map | Requires lockstep or rollback, an authoritative server, and anti-cheat. Contradicts the calm, asynchronous temperament of the whole game. |
| Competitive play — rival companies, contested routes | Directly violates the additive constraint. A neighbour who can hurt you is a different game. |
| Syncing world chunks across the border | Enormous bandwidth and privacy cost for a view the player barely uses. The published silhouette gives 90% of the feeling for 1% of the data. |
| Blocking on the neighbour for anything | Violates the absence constraint. There is no acceptable version of the player waiting. |
| Free-text chat | Moderation burden a project this size cannot carry, for a feature the design doesn't need. |
| Server-authoritative simulation | Nothing needs arbitrating, because nobody can affect anyone else's world. |
| Requiring a real neighbour to open an edge | Empty-lobby problem on day one, and it breaks the "still connected to somebody" promise for solo players. |

---

## 12. Acceptance bar

1. A player with no internet connection can open a border, trade with a neighbour, and never know the difference.
2. Deleting your neighbour's save changes nothing about your game.
3. A neighbour cannot, by any action, make your game worse.
4. A returning player finds goods that arrived while they were away.
5. Opening a border feels like a major construction project that pays off over hours.
6. Standing at the border at night, you can see somebody else's lights.
