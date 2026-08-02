# 05 — Inspection & Overlays

**The problem this brief solves:** a game whose emotional hook is that residents are "individually visible, named, and knowable" must let the player *look at one*. More broadly — a simulation the player cannot interrogate is a simulation the player cannot learn, and a system they cannot learn is one they cannot get better at.

---

## 1. Principle

**Everything in the world can be clicked, and clicking it explains it.**

Three tiers of interrogation, each one cheaper than the last:

| Tier | Cost to player | Answers |
| --- | --- | --- |
| **Ambient** | Free — it's just there | Is this district thriving? Is that line busy? |
| **Hover** | A moment | What is this? What's its headline number? |
| **Select** | A click | Why is it doing that, and what can I do? |

Most questions should be answered at the ambient tier by the world itself — a crowded platform, dark windows, a gleaming main line, smoke from a working mill. The panels exist for the questions the world genuinely cannot answer.

---

## 2. Selection

Clicking any world object selects it: track, station, train, building, peep, industry. Selection is single by default, additive with `Shift`, and a drag with the select tool boxes a region.

The selected object takes a **1-texel `railS` outline** — the brightest value in the palette, and a shape change rather than a colour wash, so it is unmistakable without being loud. The Inspector opens on the right.

`Esc` or clicking empty ground deselects. Selection survives camera movement, pause, and speed changes. `F` follows the selection.

---

## 3. The Inspector

280 texels wide, right side, opening over the world without covering the centre. Every inspector shares one anatomy:

```
┌────────────────────────────┐
│ ▣  EASTGATE          ✕     │   identity — icon, name, close
│ Station · Tier 2           │   type line
├────────────────────────────┤
│  Service      ████████░░ 78│   the headline number, always first
│  ▁▂▃▅▆▇▇▆                  │   trend
├────────────────────────────┤
│  the body — varies by type │
├────────────────────────────┤
│  [Upgrade]  [Rename]  [⌫]  │   actions
└────────────────────────────┘
```

**The headline number comes first, with its trend.** Whether a thing is getting better or worse is the question a player actually has, and a sparkline answers it faster than any table.

### 3.1 Station

The most important panel in the game.

- **Service score** with trend, and — critically — a **plain-language cause**: *"Falling: 14 people waiting more than 8 minutes."* A bare score teaches nothing. A score with a reason teaches the player how the simulation works.
- **Waiting** — a live list of named peeps with wait times, worst first, colour-coded by mood. Clicking one selects that peep. This is where "knowable" is delivered concretely: the complaint in the ticker and the person on the platform are the same clickable individual.
- **Lines calling here**, with their colours, frequencies, and next arrival.
- **Throughput** — passengers and goods over the last few minutes, as a sparkline.
- **Catchment** — a toggle that draws the ring on the map, with population inside and how much of it is unserved.
- **Actions** — upgrade tier, rename, demolish.

### 3.2 Train

- Kind, name, and current line with its colour.
- **Cargo** — what it's carrying, from where, to where, and how full.
- **Route** — the remaining stops, with the next one highlighted and an ETA.
- **Status** — running, loading, or **blocked**, and if blocked, *by what*, with a link to select the blocker. Congestion the player cannot diagnose is congestion the player cannot fix.
- **Economics** — revenue this run, opex per minute, and lifetime profit. This is where a player discovers a train is losing money.
- Actions: reassign line, reverse, sell.

### 3.3 Peep

The panel that earns the design's emotional hook.

- **Name and a small procedural portrait.** Four body types, a handful of hair and clothing colours from the palette — enough that any two peeps on screen look different.
- **Mood**, with the reason: *"Frustrated — waited 11 minutes at Eastgate, third time this week."*
- **Home**, **work**, and where they are going right now, each clickable to fly there.
- **History** — their last few journeys with times, and their last few complaints.
- The line that makes it land: *"Mara has lived in Eastgate for 14 days."*

A player who selects a peep, reads that they have been waiting for their commute for eleven minutes for the third time, and then goes and adds a train to that line, has experienced the entire game in one gesture.

### 3.4 Building

Type and tier, occupants (clickable), which station serves it and how far away, and its growth state — *"Growing — good service"*, *"Declining — nearest station 9 tiles away"*. Buildings are the readout of the whole simulation, so they must be able to explain themselves.

### 3.5 Track

Length of the selected run, gradient, curve radius, resulting speed limit, build cost and current maintenance cost, plus how many trains crossed it recently. This is where a player learns that their cheap wiggly route is slow, and where a disused branch reveals itself as a liability.

### 3.6 Industry

What it produces or consumes, current stock, production rate, whether it is starved or backed up, and which stations serve it. An industry with a full warehouse and no collection is a business opportunity, and it should be able to say so.

---

## 4. Town Talk

The complaint feed reimagined as the game's ambient voice — its diagnostic layer *and* its emotional hook, exactly as the vision asks.

**Form.** Bottom left, up to four entries visible. An entry slides in over 120 ms, holds for about twelve seconds, then fades. Each entry carries a mood icon, the peep's name in `railL`, and their line in `ballastL`.

```
  ☹  Mara  ·  waited 11 min at Eastgate
  ☺  Theo  ·  new shop opened on Mill Row
  ☹  Nia   ·  gave up waiting, walked
  ◆  Ridgeline quarry is looking for a rail link
```

**Behaviour.**

- **Clicking an entry flies the camera** to the peep or place, and selects it. The feed is a navigation surface, not decoration — this is what converts a complaint into an action.
- **Entries are typed and prioritised**: complaints, celebrations, opportunities, and warnings. When several compete, the feed shows the most actionable.
- **Rate-limited and deduplicated.** Six people complaining about the same platform is one entry that says *"6 people are waiting at Eastgate."* A wall of near-identical lines is noise, and noise gets ignored.
- **Praise as well as complaint.** A feed that only ever nags is exhausting. When a district grows, when a line hits a milestone, when someone's commute gets better — say so. The vision's failure state is a quiet town, so a *lively* feed should feel like success.
- **`` ` `` opens the full log**, searchable and filterable by type and by station, with timestamps.

Voice: plain, specific, never cute. *"Mara waited 11 min at Eastgate"* — a name, a number, a place. Never *"Uh oh! Someone's unhappy!"*

---

## 5. Overlays

`Tab` cycles; each also has a direct key. An overlay tints the world and puts a legend bottom right. The world stays readable underneath — overlays inform, they don't replace.

| Overlay | Shows | Answers |
| --- | --- | --- |
| **Service** | Per-station score, as a catchment gradient | Where is service bad? |
| **Coverage** | Catchment rings, unserved buildings hatched | Who am I missing? |
| **Congestion** | Track tinted by utilisation; blocked trains pulse | Where is my network jammed? |
| **Gradient** | Terrain tinted by slope; impassable hatched | Where can track physically go? |
| **Cost** | Terrain tinted by build cost | Where is cheap to build? |
| **Density** | Building density heat | Where is the town? |
| **Profit** | Lines and stations tinted by net contribution | What is losing money? |

**Cost** and **Gradient** are the two that teach terrain, and they should be available directly from the Build tool — reading the land before committing is a skill the game wants to develop, and it needs to be one keystroke away while building.

**Profit** is the overlay that makes overextension visible. A network sprawling with a lot of `warn`-tinted track is a picture worth more than any balance sheet.

---

## 6. Map View

`M` opens the whole world, schematic, at a scale where the entire network fits.

- Terrain as silhouette and elevation band; water; impassable rock.
- Track drawn thin, with lines in their colours and live train positions as moving dots.
- Stations as labelled nodes sized by throughput.
- Unserved demand marked, which makes the next expansion obvious at a glance.
- All the overlays from §5 render here too, and at map scale they're often more useful.
- Click anywhere to fly there; drag to box-zoom.

This is the strategic read, and it exists partly so the world camera never has to zoom out past its art's resolution.

---

## 7. Alerts

Alerts are for things needing attention that the player is not currently looking at. They stack top-right, under the status strip.

| Alert | Trigger |
| --- | --- |
| Line blocked | A train has been stuck beyond a threshold |
| Station overwhelmed | Sustained waiting over capacity |
| Train unprofitable | A train has lost money over a sustained window |
| District declining | Density falling in a served area |
| New opportunity | A new settlement or industry has appeared |
| Cash low | Balance below a few minutes of operating cost |

Each is one line, clickable to fly to the cause, dismissible individually or all at once. **Alerts never pause the game and never steal focus.** At most three visible; the rest collapse into a counter.

Every alert must be *actionable*. If the player can do nothing about it, it is not an alert — it is Town Talk.

---

## 8. Acceptance bar

1. Every visible object can be clicked and explains itself.
2. A player who asks "why is this station bad?" gets a sentence, not a number.
3. A complaint in Town Talk is one click from the person who made it and one more from the place they're standing.
4. Congestion is visible without opening a panel.
5. A player can find their least profitable line in under ten seconds.
6. Nothing in the world is a mystery that the interface refuses to discuss.
