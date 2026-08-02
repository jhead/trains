# 03 — UI System

**Status: binding standard.** The UI kit is a system, not a set of screens. Anything drawn on top of the world obeys what follows.

---

## 1. Principle

**The world is the interface. Chrome is what's left over.**

Rail Town's UI should feel like instruments beside a landscape, not a dashboard with a landscape in a window. Three commitments follow from that:

1. **Prefer diegetic.** If information can live in the world — a train's smoke, a station's crowd, a district's lit windows, a gleaming railhead — put it there. Panels are for what the world genuinely cannot say.
2. **Nothing is permanently on screen except what is permanently relevant.** Money, time, network health, and the current tool. Everything else appears because the player asked, or because something happened, and then leaves.
3. **The UI is drawn in the same medium as the world.** Same pixel grid, same palette, same integer scale. A crisp pixel landscape under a smooth vector-styled panel with soft shadows and rounded corners is two games sharing a window, and the seam is instantly visible.
4. **The interface is a window system, not a HUD.** The reference is a management sim — RollerCoaster Tycoon, Locomotion. A reading the player wants is a window they opened, that they can move, stack and close. Panels do not live in fixed corners because a designer put them there.

---

## 2. The pixel grid applies to UI

| Property | Value |
| --- | --- |
| UI scale | **Whole numbers only**, `1×`–`4×`; `Auto` picks from window size. Independent of world zoom. |
| Base unit | **4 UI texels.** Every dimension, gap and inset is a multiple. |
| Corners | **Square.** No radii. |
| Borders | 1 texel, `outline`, with a 1-texel inner light edge for raised surfaces |
| Shadows | A hard 2-texel offset block in `bg0` at 40%. No blur, ever. |
| Opacity | Panels are opaque. Translucent panels over pixel art muddy both. |

**Spacing scale:** 4 · 8 · 12 · 16 · 24 · 32. Nothing between.

Sub-pixel positioning, fractional font sizes and blurred effects are the three things that most reliably make a pixel game look cheap, and all three are usually inherited by accident from UI framework defaults rather than chosen.

**The scale ladder has no half steps, and this is not fussiness.** The smallest thing the UI draws is a 1-texel border. At `1.25×` that border is 1.25 px, and the renderer resamples it — one border ends up two pixels wide and grey, the next is one pixel and dark, and the panel looks like a mistake. A whole-number scale times whole-number metrics is always a whole number of pixels. If the ladder ever feels too coarse, the fix is to change the metrics, not to add `1.5×`.

**Default is `Auto`, which resolves to `1×` on the shipping window.** Anything larger reads as a UI built for a different screen.

### 2.1 The three coordinate spaces

Bevy folds `UiScale` into the UI's scale factor, so three different spaces are in play and they are easy to confuse:

| Space | Where it comes from | Conversion |
| --- | --- | --- |
| **UI px** | what you write in `Val::Px` | — |
| **Logical window px** | `Window::cursor_position()` | `ui_px × ui_scale` |
| **Physical px** | `ComputedNode`, a UI node's `GlobalTransform` | `ui_px × ui_scale × window.scale_factor` |

**Anything that turns a cursor position into a `Val::Px` must divide by `UiScale` first.** Use `ui::kit::cursor_to_ui`. Skipping it places the element `ui_scale` times too far right and down — which is invisible at `1×` and off the screen at `2×`. A tooltip or panel landing far from the pointer is this bug until proven otherwise.

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

**The screen is one block of chrome along the top, and windows the player opened. Nothing else is fixed.**

```
┌──────────────────────────────────────────────────────────────────────────┐
│ Look Track Demolish Line Transit Transport │ Network Town Talk Ledger     │  ← menu row, 24
│ Alerts Goals Neighbours Map Overlay │ Settings                            │
├──────────────────────────────────────────────────────────────────────────┤
│ $12,480  +$340/min           Spring 14  17:57 Dusk        ‖ 1x 2x 3x  ! 2 │  ← status strip, 20
├──────────────────────────────────────────────────────────────────────────┤
│ NET 62 ▰▰▰▱▱  1 blocked 2 unserved │ Westbrook 18 ▰▱▱ │ Eastgate 44 ▰▰▱   │  ← health strip, 20
├──────────────────────────────────────────────────────────────────────────┤
│  ┌─ Town Talk ─────── x ┐                                                 │
│  │                      │                       WORLD                     │
│  └──────────────────────┘                                                 │
│                                        ┌─ Inspector ───── x ┐             │
│                                        │                    │             │
│                                        └────────────────────┘             │
└──────────────────────────────────────────────────────────────────────────┘
```

### 5.1 The permanent block

Three rows, always on screen, in one node at the top. Nothing else is permanent.

| Row | Height | Contents |
| --- | --- | --- |
| **Menu row** | 24 | Build verbs, then window buttons, then Settings |
| **Status strip** | 20 | Money, net rate, date and time, speed, alert bell (§6) |
| **Health strip** | 20 | Network score, what is wrong, worst stations first (§6.1) |

The menu row **wraps** rather than overflowing, so no verb is ever pushed off the edge of a narrow window. Everything anchored beneath the block measures from `kit::STATUS_H`.

### 5.2 The menu row

Two groups, divided by a rule, and the split carries meaning:

- **Verbs, left.** Look · Track · Demolish · Line · Transit · Transport. These are *modes*: they arm the pointer, so they are one click with no intermediate step, and the armed one draws with a `hi` border. §7's rule is unchanged — a player who has read nothing can find every verb in the game here.
- **Windows, right.** Network · Town Talk · Ledger · Alerts · Goals · Neighbours. These are *readings*. Each button toggles a window; an open one draws with a `hi` border. Nothing in this group changes what a click on the world does, which is why it is a separate group.
- **Then** Map View and Overlay, which change how the world is drawn rather than what a click does, and Settings, which hands off to the shell.

There is **no bottom toolbar**. Two permanent bars competing for the eye is one too many.

### 5.3 Windows

Everything that is not the top block is a window: draggable by its title bar, closable by its close box or `Esc`, stacked in the order they were raised, and **remembering their position** — including across a panel rebuilding its own body.

| Window | Opens | Default |
| --- | --- | --- |
| **Network** | `Network` button, `H`, or the `NET` chip | closed |
| **Town Talk** | `Town Talk` button, `Y` | **closed** |
| **Ledger** | `Ledger` button, `K` | closed |
| **Alerts** | `Alerts` button, `C`, or the status strip's bell | closed |
| **Goals** | `Goals` button, `O` | opens once, the first time a map has goals |
| **Neighbours** | `Neighbours` button, `N` | closed |
| **Inspector** | selecting something in the world | follows the selection |

Town Talk is closed by default. Flavour is by definition not permanently relevant, and a ticker running in the corner of every screenshot is chatter the player did not ask for.

A window is one component — `ui::UiWindow` — on a panel root. Position, open state, stacking and the title bar all belong to the window manager; a panel author writes their content and nothing about layout.

**Windows never trap the player.** `Esc` closes the top window and stops there (§10.1). Modals remain centred, dim the world to 50%, and are rare.

The centre of the screen is never occupied by default. If a panel needs to be bigger than the space available, it scrolls — it does not expand into the world.

---

## 6. The status strip

Always visible, minimal, and it never moves.

```
  $12,480  +$340/min            Spring 14  17:57 Dusk        ‖ 1x 2x 3x   ! 2
```

- **Money. Whole dollars.** Fares land in cents, so a balance shown to the cent changes every second or two; a number that ticks pulls the eye, and this one is read constantly. It is Display size, `hi`, with a `min_width` so rolling from `$999` to `$1,000` does not shove the strip sideways.
- **Rate.** Net income per minute, `ok` above zero and `warn` below. **Whole dollars, except below $1/min**, where the cents are the only thing distinguishing "barely earning" from "not earning". This is the number that says whether the player is overextended, so it is permanent and not buried in a ledger.
- **Date and time.** `Season Day` and `HH:MM`, plus the **phase name** — `Dawn` / `Day` / `Dusk` / `Night`. Derived from `atmosphere::TimeOfDay::fraction`, the same value that drives the day tint, so the readout can never disagree with the light. Without it the world turns warm for no visible reason, which reads as a rendering fault rather than as evening.
- **Speed.** Pause / 1× / 2× / 3× as a segmented control, clickable, with the active segment in `hi`.
- **Alert bell.** Count of *actionable* alerts (§6.1). Clicking opens the Alerts window. `-` when there is nothing, `*` in `hi` when the news is only opportunity, `!` in `warn` when something is actually wrong — the glyph carries the tone as well as the colour.

---

## 6.1 The health strip

**Network health is a readout, not a notification.** It is the third permanently-visible thing on screen, next to money and time, because it is the third thing the player needs to know at all times. Alerts are for things that *changed*; this is for the state of the railway, which the player consults constantly and should never have to go looking for.

```
  NET 62 ▰▰▰▱▱   1 blocked  2 unserved   │ Westbrook 18 ▰▱▱ │ Eastgate 44 ▰▰▱
```

- **Network score** — the mean across stations that have actually been served, with a meter. `-` when nothing has run yet, never `0`; a railway with no trains has no score, and printing zero reads as failure.
- **What is wrong, in words** — blocked trains, parked trains, unserved demand, stations still waiting. `warn` only when something genuinely needs the player; an unfinished network is not a failing one.
- **Worst stations first**, as small meters with a numeral beside them, each **clickable to fly there**. Stations still awaiting their first train sort *last* and read `awaiting service` rather than a score — they are a to-do, not a problem.

The `NET` chip opens the **Network window**, which is the same list without a limit.

**A station that has never been served does not raise an alert.** "Westbrook service low (0)" on the opening frame is not a degradation, it is a station waiting for its first train, and the player already knows because they just built it. Brief 05 §7 requires an alert to be actionable; this one was neither actionable nor news. It now appears in the health strip as a neutral fact, in the place the player is already looking.

---

## 7. The verb group

*(This was a 48×48 bottom-centre toolbar. It is now the left-hand group of the menu row — §5.2. The purpose below is unchanged; only the position and the size are.)*

Every slot carries its **name and its shortcut key**, side by side, at Micro size. Names rather than icons: there is no icon set yet, and a labelled button a player can read beats a glyph they have to guess. The armed verb draws with a `hi` border.

Slots are sized by their label, not padded out to a fixed square — a 48×48 slot at `2×` was 96 physical pixels, which is most of a phone screen's width for one button.

Tools with sub-modes (bridge, tunnel, station tier) expand a small row **beneath** the slot on selection, rather than opening a separate panel. The player should never lose sight of the world to choose a variant of what they are already doing.

**A player who has never read a document must be able to find every verb in the game from this row.** That is its entire purpose. Keyboard shortcuts are an accelerator layer on top, never the only way in.

---

## 8. Components

### 8.1 Window

1-texel `outline` border, `bg1` fill, square corners. A title bar 16 tall in `bg0` — the title at Micro size in `hi`, a close box at the right — which is also the drag handle. The focused window's title bar reads a step darker than the rest, so the stack has a visible top.

The title bar is added by the window system, not by the panel. A panel that writes its own title row will end up with two.

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

`Esc` unwinds one layer of state per press. Never more than one layer per press, and never two things at once. The order is:

1. **Close the top window**, if any are open.
2. Cancel a build drag or anchor.
3. Disarm the tool, then clear the selection.
4. Open the pause menu.

Step 1 runs in `PreUpdate` (`ui::WindowEscSet`) and **consumes the key press** when it acts, so nothing further down the list also fires. `ShellPlugin` orders its own `Esc` handling after that set; a new `Esc` consumer must do the same or it will double-unwind.

### 10.2 Shortcuts

| Key | Action |
| --- | --- |
| `V` `B` `X` `L` `T` `G` | Look / Track / Demolish / Line / Transit / Transport |
| `H` `Y` `K` `C` `O` `N` | Network / Town Talk / Ledger / Alerts / Goals / Neighbours |
| `Space` | Pause / resume |
| `1` `2` `3` | Speed |
| `M` | Map View |
| `Tab` | Cycle overlay |
| `F` | Follow selection |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Undo / redo |
| `Esc` | Unwind |

Window keys avoid every key a gameplay verb already owns; a test asserts it. `L` belongs to the Line tool, so the Ledger answers to `K` — the two used to share `L`, and the Controls tab was right to flag it.

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
3. The world is visible at all times; no panel occupies screen centre by default.
4. Nothing on screen animates on a loop.
5. Turning the game's colour off entirely leaves every state still distinguishable.
6. A screenshot of the UI and a screenshot of the world look like they come from the same game.
7. **`Esc` always gets the player out**, one layer per press, and never opens the pause menu while a window is still open.
8. **Every window can be moved, closed, and reopened where it was left.**
9. **A new game opens with nothing wrong.** No alert on the opening frame describes a state the player has not had a chance to change.
10. **No permanently-visible number ticks.** Money is whole dollars; anything that changes every second belongs in a window.
