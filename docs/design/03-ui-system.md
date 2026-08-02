# 03 — UI System

**Status: binding standard.** The UI kit is a system, not a set of screens. Anything drawn on top of the world obeys what follows.

---

## 1. Principle

**The world is the interface. Chrome is what's left over.**

Rail Town's UI should feel like instruments beside a landscape, not a dashboard with a landscape in a window. Three commitments follow from that:

1. **Prefer diegetic.** If information can live in the world — a train's smoke, a station's crowd, a district's lit windows, a gleaming railhead — put it there. Panels are for what the world genuinely cannot say.
2. **Nothing is permanently on screen except what is permanently relevant.** Money, time, and the current tool. Everything else appears because the player asked, or because something happened, and then leaves.
3. **The UI is drawn in the same medium as the world.** Same pixel grid, same palette, same integer scale. A crisp pixel landscape under a smooth vector-styled panel with soft shadows and rounded corners is two games sharing a window, and the seam is instantly visible.

---

## 2. The pixel grid applies to UI

| Property | Value |
| --- | --- |
| UI scale | Integer, `2×` or `3×`, chosen from window size; independent of world zoom |
| Base unit | **4 UI texels.** Every dimension, gap and inset is a multiple. |
| Corners | **Square.** No radii. |
| Borders | 1 texel, `outline`, with a 1-texel inner light edge for raised surfaces |
| Shadows | A hard 2-texel offset block in `bg0` at 40%. No blur, ever. |
| Opacity | Panels are opaque. Translucent panels over pixel art muddy both. |

**Spacing scale:** 4 · 8 · 12 · 16 · 24 · 32. Nothing between.

Sub-pixel positioning, fractional font sizes and blurred effects are the three things that most reliably make a pixel game look cheap, and all three are usually inherited by accident from UI framework defaults rather than chosen.

---

## 3. Type

A **bitmap pixel font** in two sizes, rendered at exactly 1× or 2× the UI scale and never at a fractional size.

| Role | Size | Use |
| --- | --- | --- |
| **Display** | 11 texel cap height | money, panel titles, station names |
| **Body** | 7 texel cap height | everything else |
| **Micro** | 5 texel cap height | table columns, tooltips, unit labels |

Line height is 1.5× cap height, rounded to 4. Numerals are **tabular** — money and times must not jitter as digits change. Small caps for section labels; no italics, no faux bold, no letter-spacing tricks.

A general-purpose vector UI font at fractional sizes is the single most common way a pixel-art game's interface ends up looking unfinished, and it is entirely avoidable.

---

## 4. Colour roles

Built from the palette in [01 — Art Direction](01-art-direction.md) §3. The diagnostic triad — `hi`, `warn`, `ok` — belongs to the UI exclusively; the world never draws in it, which is what makes it read instantly.

| Role | Colour |
| --- | --- |
| Panel fill | `bg1` |
| Panel fill, recessed | `bg0` |
| Panel border | `outline` |
| Raised inner edge | `ballastM` |
| Rule / divider | `ballastD` |
| Text primary | `railL` |
| Text secondary | `ballastL` |
| Text disabled | `ballastM` |
| Accent / selection / money | `hi` |
| Positive — income, valid, growth | `ok` |
| Negative — expense, invalid, decline | `warn` |
| Transit line default | `railM` |
| Transport line default | `tieL` |

Colour never carries meaning alone. Every state that uses colour also changes shape, icon or text — for colour-blind players, and because a 2-texel colour difference is invisible at a glance anyway.

---

## 5. Layout

The screen is a frame around the world, and the world is never fully covered.

```
┌────────────────────────────────────────────────────────────┐
│ ▸ money · date · speed                        alerts ▸     │  ← status strip, 24
├────────────────────────────────────────────────────────────┤
│                                                            │
│                                            ┌─────────────┐ │
│                    WORLD                   │  INSPECTOR  │ │  ← 280, on demand
│                                            │             │ │
│                                            └─────────────┘ │
│  ┌──────────────────┐                                      │
│  │  TOWN TALK       │        ┌──────────────────┐          │  ← ticker, transient
│  └──────────────────┘        │  ▣ ▤ ▥ ▦ ▧ ▨ ▩  │          │  ← toolbar, 48
└──────────────────────────────┴──────────────────┴──────────┘
```

| Region | Position | Persistence |
| --- | --- | --- |
| **Status strip** | Top, full width, 24 tall | Always |
| **Toolbar** | Bottom centre, 48 tall | Always |
| **Inspector** | Right, 280 wide | On selection |
| **Town Talk** | Bottom left, 320 wide | Entries appear and expire |
| **Alerts** | Top right, under status | On event |
| **Modals** | Centred, dim the world to 50% | Rare, and always dismissible with `Esc` |

The centre of the screen is never occupied by chrome. If a panel needs to be bigger than the space available, it scrolls — it does not expand into the world.

---

## 6. The status strip

Always visible, minimal, and it never moves.

```
  $12,480    ▲ +$340/min          Spring · Day 14 · 14:20    ▶▶  ⚠ 2
```

- **Money.** Display size, `hi`. On change, the delta floats up beside it in `ok` or `warn` and fades over about a second. The number itself never flashes or scales — a jumping balance is noise, and this number is read constantly.
- **Rate.** Net income per minute, `ok` above zero and `warn` below. This is the number that tells the player whether they are overextended, so it is on screen permanently and not buried in a ledger.
- **Date and time.** The design's pacing promise is written in minutes and hours, and it needs a readout. Season matters if seasons affect demand.
- **Speed.** Pause / 1× / 2× / 3× as a segmented control, clickable, with the active segment in `hi`.
- **Alert count.** Clicking opens the alert list.

---

## 7. The toolbar

The game currently has no discoverable affordances at all, and the toolbar is the fix. Bottom centre, always visible, mouse-operable, with keyboard shortcuts shown on every slot.

```
┌────┬────┬────┬────┬────┬────┬────┐
│ ▤  │ ▨  │ ▣  │ ▥  │ ▦  │ ▧  │ ▩  │
│ 1  │ 2  │ 3  │ 4  │ 5  │ 6  │ 7  │
└────┴────┴────┴────┴────┴────┴────┘
  Track Station Train Line Demolish Overlays Map
```

Each slot is 48×48, holds a 32×32 icon, and shows its number key. The active tool draws with a `hi` inner border and a 2-texel raised edge. Hovering shows a tooltip with the tool's name, its shortcut, and its cost where it has one.

Tools with sub-modes (bridge, tunnel, station tier) expand a small row **above** the slot on selection, rather than opening a separate panel. The player should never lose sight of the world to choose a variant of what they are already doing.

**A player who has never read a document must be able to find every verb in the game from this bar.** That is the bar's entire purpose. Keyboard shortcuts are an accelerator layer on top, never the only way in.

---

## 8. Components

### 8.1 Panel

1-texel `outline` border, `bg1` fill, 12 inset. A title row 20 tall with the title in Display size and a close affordance at the right. A 1-texel `ballastD` rule under the title.

### 8.2 Button

Default 24 tall, 12 horizontal inset. Raised edge, `bg1` fill, `railL` label. Hover lightens the fill one step; press inverts the raised edge and offsets content 1 texel down; disabled drops the label to `ballastM` and removes the raised edge. Destructive buttons carry a `warn` left border, not a `warn` fill — a full red button is louder than anything in this game should be.

### 8.3 Tooltip

Appears after **400 ms** of hover, `bg0` fill with a 1-texel `outline` border, 8 inset, Micro type. Anchored to the element, flipped to stay on screen, and **never covering the tile under the cursor** — which matters enormously while building. Tooltips do not animate in; they appear.

### 8.4 Meter

Used for service scores, capacity, and progress. A 4-texel-tall recessed bar in `bg0` with a filled portion whose colour is derived from value — `warn` below a third, `hi` in the middle band, `ok` above two thirds. Meters always carry a numeral beside them; a bare bar is unreadable at these sizes.

### 8.5 Sparkline

24 × 8, one texel per sample, `ballastL` line with the current value dotted in `hi`. Used for money history and station throughput. A trend answers "is this getting better or worse" far faster than any number.

### 8.6 List row

24 tall, alternating fill between `bg1` and one step darker, with hover raising to `ballastD`. Selected rows take a 2-texel `hi` left border. Rows are clickable targets and behave like buttons.

---

## 9. Motion

Motion is where a pixel UI most easily betrays its grid. Rules:

- **Position tweens move in whole texels.** Anything else resamples.
- **Prefer opacity and reveal over movement.** Panels fade in over 120 ms; they do not slide.
- **Durations:** 120 ms in, 180 ms out, 90 ms for state changes such as hover and press. Nothing exceeds 250 ms.
- **Easing:** a two-step ease-out, quantised. Smooth curves buy nothing when the result is rounded to texels.
- **Nothing loops.** A permanently pulsing element is a permanent distraction, and this game is calm. Attention-getting is a one-shot: an alert flashes twice and then holds a steady state.

---

## 10. Input

### 10.1 Mouse is primary

Every action is reachable with the mouse alone. Keyboard shortcuts accelerate; they never gate.

| Input | Meaning |
| --- | --- |
| Left click | Select, or apply the active tool |
| Left drag | Build, or box-select |
| Right click | Cancel current action, then deselect, then clear tool — in that order |
| Middle drag | Pan |
| Wheel | Zoom to cursor |
| Hover | Highlight and, after a delay, tooltip |
| Double click | Select all of the same kind (e.g. all trains on a line) |

`Esc` unwinds one layer of state per press: cancel a drag, close a panel, deselect, open the pause menu. Never more than one layer per press, and never two things at once.

### 10.2 Shortcuts

| Key | Action |
| --- | --- |
| `1`–`7` | Select tool |
| `Space` | Pause / resume |
| `` ` `` | Cycle speed |
| `M` | Map View |
| `Tab` | Cycle overlay |
| `G` | Toggle tile grid |
| `F` | Follow selection |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Undo / redo |
| `Del` | Demolish selection |
| `Esc` | Unwind |
| `F1` | Controls reference |

All rebindable. The list is shown in Settings and in a `F1` overlay, so no player ever needs external documentation to find a verb.

### 10.3 Accessibility

- **Colour-blind safe:** no state distinguished by colour alone.
- **Text scale:** UI scale is user-selectable independently of world zoom.
- **Reduced motion:** a setting that disables all tweens; everything cuts.
- **Hold-to-repeat** on every incremental control, and full keyboard navigation of panels.
- **No timing-critical input** anywhere in the game. This is a calm builder; nothing should ever require reflex.

---

## 11. Acceptance bar

1. A player who has read nothing can find and use every verb in the game.
2. No text is ever blurry, and no panel edge is ever soft.
3. The world is visible at all times; no panel occupies screen centre.
4. Nothing on screen animates on a loop.
5. Turning the game's colour off entirely leaves every state still distinguishable.
6. A screenshot of the UI and a screenshot of the world look like they come from the same game.
