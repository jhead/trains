# 09 — Shell & Menus

**The shell is what makes it a product rather than a build.** It is also the player's first thirty seconds, which disproportionately sets expectations for everything after.

---

## 1. Principle

**The shell is calm, quick, and made of the same material as the game.** No splash logos to sit through, no loading bars for things that load instantly, no menu that takes more clicks than it needs. A player should be able to go from launching the game to laying track in under fifteen seconds, and to resume a saved game in under five.

Menus use the same pixel kit as the in-game UI ([03 — UI System](03-ui-system.md)) — same font, same palette, same grid, same square corners. A polished title screen in a different visual language than the game is a promise the game then breaks.

---

## 2. Title screen

**The world is the background.** A generated map runs live behind the menu, at a slow drift, with trains moving on a small pre-built network. It is not a video and not a still — it is the actual game, playing itself quietly.

This does more work than any amount of key art. It shows the player what the game is, in motion, before they have clicked anything. It also means the title screen looks better every time the game does.

```
                    R A I L   T O W N

                      Continue
                      New Map
                      Load
                      Settings
                      Quit

                                          v0.1 · seed 84213
```

- **Continue** is first and pre-selected — it is what a returning player wants, every time.
- The seed of the background map is shown, and it can be played directly. A player who likes what they see can just start there.
- No animation on the menu itself beyond a subtle selection highlight. The motion in the frame is the world's.

---

## 3. New Map

One screen, no wizard. Options on the left, a **live preview of the generated map on the right** that regenerates as options change.

```
┌─────────────────────────┬──────────────────────────┐
│  Seed    [ 84213 ] 🎲   │                          │
│  Size    Small ▪ Std ▫  │      map preview         │
│  Terrain Gentle ▫ Roll ▪│      (live, schematic)   │
│  Water   Sparse ▫ Bal ▪ │                          │
│  Cash    Lean ▫ Std ▪   │   land 74%  towns 4      │
│  Mode    Sandbox ▪ Goals│   rivers 2  passes 5     │
├─────────────────────────┴──────────────────────────┤
│                              [ Back ]  [ Begin ]   │
└────────────────────────────────────────────────────┘
```

The preview is the point. Choosing a map should be a small pleasure — rolling the dice a few times until one looks interesting is a legitimate and enjoyable way to start, and it also puts the terrain generator's quality on display at the exact moment it matters.

Alongside the preview, a few honest readouts of what the map contains — land share, number of towns, rivers, mountain passes — so the player can pick a map that poses the kind of problem they're in the mood for.

Options are described in [02 — World & Terrain](02-world-and-terrain.md) §5. Seeds are shareable as a short code that encodes both seed and settings.

---

## 4. In-game menu

`Esc` from play dims the world to 50% and centres a small panel. It does **not** hide the world — the player should still see what they were doing.

```
              ─ Paused ─

               Resume
               Save
               Load
               Settings
               Quit to Title

           Rail Town · Day 14
```

The world pauses. `Esc` again resumes. Nothing here takes more than one click to reach.

---

## 5. Settings

Four tabs, applied live wherever possible — a setting that requires a restart to see is a setting the player cannot evaluate.

**Display** — window mode, resolution, UI scale (1×–3×), world zoom default, vsync, frame cap, tile grid, edge pan.

**Audio** — master, music, ambience, effects, UI, each with a live preview tone. Mute-on-focus-loss. See [10 — Audio & Feel](10-audio-and-feel.md).

**Gameplay** — autosave interval, tooltip delay, confirm destructive actions, show cost while building, pause on alert (default off), Town Talk verbosity.

**Controls** — full rebindable list, grouped by context, with conflict detection and a reset. The same list is available in-game on `F1`.

**Accessibility** lives across these rather than being quarantined: reduced motion, colour-blind-safe palette variants, text scale, hold-to-repeat timing, and the option to disable all screen shake and flashes.

---

## 6. Save and load

- **Autosave** on an interval and on quit, into a rotating set of slots, never blocking play.
- **Manual saves**, named, with a thumbnail of the map, the date, elapsed time and headline stats.
- **Quick save / quick load** on function keys.
- Saving happens **without interrupting the simulation**, and it must never produce a hitch. A calm game that stutters every three minutes is not calm.

A save is a complete snapshot of the world — map, network, lines, trains, town, peeps with their names and histories. Peep names and histories persisting across a save is what makes the town feel like a continuous place rather than a re-rolled state.

---

## 7. First run

**No tutorial popups. No modal lecture. No forced sequence.**

Instead, the opening map is designed to teach ([02 — World & Terrain](02-world-and-terrain.md) §4.1): a home town, one destination close by, one small terrain question between them. The game teaches by being legible, and by three light touches:

1. **A gentle nudge in Town Talk:** *"Westbrook is eight tiles east. They'd like a railway."*
2. **Contextual hints, once each.** The first time the player selects the Build tool, a small non-modal chip near the toolbar: *"Drag to lay track."* It appears once, never returns, and never blocks anything.
3. **The first payout is celebrated** — a clear, warm moment when the first train completes its first run. That is the loop closing for the first time, and it should feel like something.

Everything else is discovered. A player who wants explicit instruction has `F1`; a player who doesn't is never interrupted.

The test: **a player who reads nothing should be laying track within thirty seconds and have earned money within three minutes.**

---

## 8. Around the edges

- **Screenshot key** that hides all UI, for a game that will be pretty enough to want photographed.
- **Credits**, reachable from the title, listing everyone including tool and asset authors.
- **A visible build version and map seed**, so a player reporting a problem can say which world they were in.
- **Graceful window resize** at any moment, with UI scale re-derived and the world re-framed.

---

## 9. Acceptance bar

1. Launch to laying track in under fifteen seconds.
2. Launch to resuming a save in under five.
3. The title screen shows the actual game, in motion, before any click.
4. Every setting applies live, or says plainly that it doesn't.
5. A player who reads nothing earns their first money within three minutes.
6. Nothing in the shell looks like it came from a different game than the world does.
