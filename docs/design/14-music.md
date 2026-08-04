# 14 — Music

**Status: feature design, subordinate to [10 — Audio & Feel](10-audio-and-feel.md).** Where this brief and 10 disagree, 10 wins. "Never startle", "silence is a texture", the narrow dynamic range and the cue schedule in 10 §4 are constraints on everything below, not inputs to be negotiated.

**History.** This is the second design. The first — a four-minute modal rondo in D Lydian, 6/4 at 66 BPM, one plucked-string orchestra, the dominant banned so the piece could never cadence — shipped, was playtested (2026-08-04), and failed. Verdict, verbatim: *"really repetitive and not upbeat and bright. I wanted C418 vibes and it's just bland and boring."* The diagnosis: music that never arrives anywhere is music with nothing to remember, and one key, one motif and one timbre held for a whole session reads as absence, not calm. What was kept and what was retired is itemised in §8; the reference the owner named — **C418** — is the brief now: bright bell tones over quiet pads, singable motifs that return recognisably, real diatonic movement with gentle cadences, and generous rests.

---

**The premise:** the score is generated, not authored, because there is no composer and there are no audio assets. That is a constraint, and constraints are fine — but generated music has a characteristic failure mode, and it is worth naming before anything else:

> **Random notes in a scale is what generative music sounds like when it is done badly, and every listener recognises it inside four bars.** It is not that the notes are wrong. It is that nothing is *going* anywhere: the harmony does not progress, the melody has no shape, and the voices do not lead.

Everything below exists so that the music has harmony that moves, a melody with contour and breath, voice leading that actually leads, and material a listener can recognise coming back.

---

## 1. The music in one paragraph

A world composes **four short pieces** from its seed, and the cue counter rotates through them — so no two consecutive cues share a key, a motif or a progression, and "repetitive" is answered structurally rather than hoped away. Each piece is about four minutes, **4/4 at 88 BPM, in one of D, G, A or C major**, laid out intro–A–B–A′–C–A″–outro. A bell melody in **C4–A5** states a motif and brings it back varied; a pad moves through functional diatonic progressions that **cadence at every section end**; a sine bass keeps the floor; a plucked arpeggio syncopates against the grid. Arrivals are the point now — kept soft, never banned.

## 2. Pieces and form

| Section | Bars | What happens |
| --- | --- | --- |
| Intro | 4 | pad only, establishing the key; hands over **on the dominant**, so A begins as an arrival |
| A | 16 | motif 1 stated and answered over the home progression |
| B | 16 | motif 2 over a contrasting progression |
| A′ | 16 | motif 1 returns — transposed, octave-displaced or augmented |
| C | 16 | the breath: long notes, sparse texture, a borrowed `bVII` or `iv` for warmth |
| A″ | 16 | motif 1 again, differently varied |
| Outro | 8 | thins back to the pad and cadences |

A cue always starts a piece at bar one — the director numbers its cues, and the audio thread reads a cue change as "start the *next* piece from the top" (see §7). The player therefore always begins somewhere they can follow, and never the same way twice running.

## 3. Harmony

- **Functional and diatonic.** Each piece draws a home progression from the I–V–vi–IV / I–IV–I–V / vi–IV–I–V / ii–V families, a contrasting one for B, and a borrowed `bVII` or `iv` in C. Five to seven distinct roots per piece, asserted.
- **Colour on everything.** Tonic and subdominant chords carry add9, add6 or maj7 — no bare triads anywhere. Gentleness comes from the voicing, not from avoiding movement.
- **Cadences at every section end**: a tonic prepared by a dominant or a subdominant. The old dominant ban is gone; in its place, a *preference* — **`Vsus4` over a bare `V`** — so there is release without glare.
- **Voicings are searched, not tabled.** Every octave assignment of a chord inside the pad register (C3–C5) is enumerated and ranked in tiers: least total motion, a held common tone, and no parallel fifths or octaves first — with the bass chosen *jointly* with the upper voices, because a parallel against the bass is the audible one. This is not the "automatic voicer" §8 rejects: the search's objective *is* the part-writing rules, and the measured tally is 96% top-tier with zero parallels anywhere in eight seeds × four pieces.

## 4. Motifs and melody

- **Two motifs per piece** — three to six notes over one or two bars, built on rhythmic cells, most of which start or land off the beat.
- **Restatement is the design.** A motif returns about a dozen times per piece under variation: diatonic transposition, octave displacement, augmentation (×1.5, ×2), inversion. The interval signature survives every variation — that is what makes a return audible.
- **Placement is searched.** Each restatement picks the transposition that maximises strong-beat chord-tone agreement, so the tune is never edited note-by-note to fit the harmony, and the harmony never has to bend to the tune.
- **Fresh tails.** Every statement is followed by at least two beats of newly generated line, which is what keeps a return from being a copy.
- **Melodic discipline, measured**: steps outnumber leaps roughly three to one (counted in scale degrees, so a third is a leap); strong beats take chord tones ~97% of the time; about 29% of melody onsets are off the beat while the bass never leaves the grid — a pulse, and a tune that syncopates against it. Phrases breathe: every phrase is followed by a rest before the next.
- **Register is bright**: melody C4–A5 with its median around G4–C♯5, pad C3–C5, bass one octave from C2. The tune never goes down into the mud — not even at dusk (§6).

## 5. The sound

Four voices, none of them the same kind of thing, plus a delay:

| Voice | Recipe |
| --- | --- |
| **Bell** (melody) | Four sine partials at 1 / 2 / 3 / 4.02×, each with its own decay so the top falls away first, plus the fundamental six cents sharp for shimmer. 12 ms raised-cosine attack — struck, never percussive. Oscillators are magic-circle recursions: no trigonometry per sample. |
| **Pad** (harmony) | Two polyBLEP saws a few cents apart through two poles of low-pass (900–2100 Hz with warmth). Slow attack, slower release; a voice that does not move between two chords is not retriggered — the common tone simply stays down. |
| **Bass** | Sine plus a touch of its own second partial. |
| **Arpeggio** | Karplus–Strong, kept from the first design and demoted from everything to the off-beat plucks — a real string is the one thing the other three cannot imitate. |
| **Delay** | One eighth-note tap (341 ms at 88 BPM), low-passed in the loop, mixed low. This is the space; there is still no reverb. |

Loudness is calibrated to the same `gain::MUSIC` ceiling as the first design (peak ≈ 0.63 at the loudest reading), and the whole instrument stays inside the established DSP budget: the test suite renders minutes of audio in a fraction of a second, and silence costs one comparison per sample.

## 6. Context

Chosen at the start of a cue, while the voice is silent, so no control ever sweeps under a sounding note.

| Signal | Reads | Effect |
| --- | --- | --- |
| **Warmth** | how the network is doing | a more open bell, a brighter pad |
| **Density** | the same | how much survives the thinning: a thin town keeps the pad, the downbeat and the tune; a thriving one gains the walking bass and the arpeggio |
| **Dusk** | the day phase | the same music, **softer and darker in reading** — slower attacks, a lower filter, fewer partials. Never an octave drop: the drop was most of what made the first score read as gloom |

## 7. Determinism and integration

The map's own seed composes the world's four pieces, once, when the music voice is created — the audio callback walks event lists and makes no decisions. Same map, same four pieces, sample-exact, asserted across seeds. A new map rebuilds the voice only ever between cues. Cue numbering (10 §4) is unchanged: a change of cue number means "next piece, bar one"; zero means "stop, and cost nothing until asked again."

To listen without booting the game:

```
cargo test -p rail_town audio::score::tests::write_a_sample_to_listen_to -- --ignored --nocapture
```

writes a short WAV (`RAIL_TOWN_SCORE_WAV`, `_SECS`, `_SEED`, `_PIECE`, `_WARMTH`, `_DENSITY`, `_DUSK` override the defaults). No audio file is ever committed.

## 8. Rejected, and why

**The modal rondo** (the first realisation, in full: D Lydian, 6/4 at 66 BPM, a banned dominant, a hand-written ten-voicing table, one piece per world). Shipped and playtested; the verdict is quoted at the top. Its central wager — "a home but no cadence... somewhere you already are rather than somewhere you are arriving at" — lost: leave-on-able turned out to mean nothing-to-remember. Retired on evidence, which is the best reason there is.

**The banned dominant.** Tension that never resolves is exactly as monotonous as tension that never arrives. Replaced by the `Vsus4` preference: arrivals, kept gentle.

**The hand-written voicing table.** It existed because "an automatic voicer produces correct chords and dead part-writing." That critique stands — and the replacement is not that voicer. The search ranks every voicing *by the part-writing rules themselves* and takes the best tier; the table's craft became the objective function, which scales to four pieces per world where a hand table cannot.

**One piece per world.** However good the piece, the twentieth cue of it is the definition of "really repetitive." Four pieces and a rotation is the structural fix; everything else in this brief is the musical one.

**A reverb.** Still rejected. The eighth-note delay plus long bell decays are the space, at a fraction of the cost.

**Percussion.** Still rejected (10 §4). The metre is felt through where the bass and the off-beat plucks land, which is all a metre has to do.

## 9. Acceptance bar

1. A listener can hum a motif after one cue, and recognises it when it returns.
2. Consecutive cues are audibly different pieces — different key, different tune.
3. Phrase ends land. Cadences read as gentle arrivals, never as fanfare.
4. The register reads bright; dusk reads softer, never gloomier.
5. The same map always plays the same four pieces, sample-exact.
6. The budget holds: well under half a percent of one core, and nothing while silent.
7. The ear is the real test. Every claim above is measurable except "sounds like C418" — that one is settled by playtest, and this brief bends to the next one the way it bent to the last.
