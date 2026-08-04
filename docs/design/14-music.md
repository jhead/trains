# 14 — Music

**Status: feature design, subordinate to [10 — Audio & Feel](10-audio-and-feel.md).** Where this brief and 10 disagree, 10 wins. "Never startle", "silence is a texture", the narrow dynamic range and the cue schedule in 10 §4 are constraints on everything below, not inputs to be negotiated.

---

## 0. Course correction (2026-08-04) — this supersedes §§1-3 where they conflict

The first realisation of this brief shipped and was playtested. Verdict, verbatim: *"really repetitive and not upbeat and bright. I wanted C418 vibes and it's just bland and boring."*

The diagnosis is that §1's central wager lost. "A home but no cadence... somewhere you already are rather than somewhere you are arriving at" was designed to be leave-on-able, and it turned out that music which never arrives anywhere is music with nothing to remember. One low tonal centre, one plucked timbre, one harmonic colour held for four minutes reads as absence, not calm. The reference the owner named is **C418**: simple bright keys/bell tones with soft attacks and long decays over quiet pads, **singable motifs that return recognisably**, real diatonic movement with cadences at phrase ends (sus/add9 colour keeping them gentle), melody living around C4-C6, a gentle but present pulse, and generous rests between phrases.

What replaces the specifics of §§1-3:

- **Pieces, not a rondo texture.** A cue is a small piece with its own key (drawn from a few bright majors), its own one-or-two motifs, and its own progression; intro / A / B / A' / outro across two to four minutes. Consecutive cues must differ audibly.
- **Motifs over walks.** Melodic material is generated once per piece and *reused with variation* — transposed, augmented, octave-shifted. A listener hears material return; that is the whole difference between a piece and a scale exercise.
- **Harmony moves.** Functional diatonic progressions with cadences at phrase ends. The old dominant ban relaxes to taste: arrivals are the point now, kept soft (sus resolutions) rather than banned.
- **Bright registers, layered voices.** Keys/bell melody (C4-C6), quiet pad harmony, soft sine bass — the single low pluck orchestra is retired as the lead (it may survive as a colour).
- **4/4 around 80-100 BPM** with an unobtrusive rhythmic floor, replacing 6/4 at 66.

What survives unchanged: everything 10 imposes (never startle, silence as texture, narrow dynamics, the cue schedule), determinism from the seed, zero assets and zero new dependencies, the render budget, and §4+ below where they describe motif discipline, voice-leading tests and integration — those apply to the new material as they did to the old.

**The premise:** the score is generated, not authored, because there is no composer and there are no audio assets. That is a constraint, and constraints are fine — but generated music has a characteristic failure mode, and it is worth naming before anything else:

> **Random notes in a scale is what generative music sounds like when it is done badly, and every listener recognises it inside four bars.** It is not that the notes are wrong. It is that nothing is *going* anywhere: the harmony does not progress, the melody has no shape, and the voices do not lead.

Everything in this brief exists so that the piece has harmony that moves, a melody with contour and breath, voice leading that actually leads, and a form you can recognise on hearing it a second time.

---

## 1. The piece in one sentence

**A four-minute-twenty-two rondo in D Lydian at 66 BPM in 6/4, for plucked strings, generated once from the map's seed and played from the top of every cue.**

Three properties, and every decision below is tested against all three:

- **Modal, not functional.** It has a home but no cadence. It should feel like somewhere you already are rather than somewhere you are arriving at.
- **Fixed by seed.** The same world always sounds like itself, down to the sample. Generative and recognisable are not in tension once the seed is fixed and the form is composed.
- **Cheap.** It shares a core with the renderer. A serious FPS regression has already made this game unplayable once and the score will not be the next one.

---

## 2. The mode

**D Lydian: D E F# G# A B C#.**

### 2.1 Why a mode at all

Plain major has a dominant, a leading tone and a cadence, and a cadence is an *arrival*. This score plays for four minutes at a time, twenty times in a session, under a game where nothing much is happening on purpose. A piece that keeps arriving keeps announcing itself. Modal writing gives a tonal centre with no gravitational pull toward it, which is exactly the difference between music you can leave on and music you have to listen to.

Plain minor has the opposite problem: it comments. A minor score under a declining town reads as a judgement, and 10 §4 forbids any cue that comments on failure.

### 2.2 Why Lydian rather than Dorian

Both fit "calm". Only one fits "calm **and** upbeat".

Lydian is major with the fourth raised — one note different from the most familiar scale in the language, and that one note is a tritone above the tonic. The interval is unmistakable and it does not resolve, which is why it reads as *wonder* rather than as sweetness: an open question rather than a statement. Dorian's raised sixth is warmer but it is still a minor mode, and warmth is not the brief. The brief is optimism.

**A Lydian score that avoids its raised fourth is just major**, so the design puts the `G#` where it cannot be missed:

- It is the **top voice** of chords 2, 3 and 4 — the whole of the second half of every statement of the theme.
- Six of the ten voicings contain it.
- The melody is weighted toward it and toward the major seventh **at the peak of every arch**, which is where a listener's attention already is.
- One chord puts it **in the bass** (`E/G#`, the climax of the B section).

### 2.3 The dominant is banned

`A` major never appears as a chord, and the bass never touches `A`.

This is the single most important rule in the piece and it is enforced by a test. `V–I` in D Lydian is `A–D`, which is the perfect authentic cadence of D **major**; play it once and the ear reinterprets every `G#` heard so far as a passing chromatic note. One cadence undoes the mode. So the palette is `I`, `II`, `iii`, `vi`, `vii` — and the diminished chord on the raised fourth is left out as well, since it is unstable in a piece with nothing to resolve to.

`A` is still available as a *melody* note and as a chord tone; it is the fifth of the key and the piece would be hollow without it. It is only the root position triad and the bass note that are forbidden.

---

## 3. The harmony

### 3.1 A pedal, then a bass line

The theme sits on a **D pedal** for all eight of its bars. Over a drone, four different upper structures produce four different chords without the bass moving at all, and the "progression" becomes a slow bloom of colour rather than a set of root movements. That is a real technique with a long history and it is the most efficient way there is to have harmony that changes without harmony that resolves.

The episodes take the pedal away and let the bass walk. That contrast — static theme, moving episode — is the form's main structural signal, and it costs nothing to hear.

### 3.2 The ten voicings

Semitones above D3 for the upper voices, above D2 for the bass. Degrees are `0 D, 1 E, 2 F#, 3 G#, 4 A, 5 B, 6 C#`.

| # | Name | Bass | Upper voices | Sounding degrees | Note |
| --- | --- | --- | --- | --- | --- |
| 0 | `Dadd9` | D | D3 A3 E4 F#4 | D E F# A | home |
| 1 | `D6/9` | D | D3 B3 E4 F#4 | D E F# B | `vi` over the pedal reads as the tonic with a sixth |
| 2 | `E/D` | D | E3 B3 E4 G#4 | D E G# B | **the Lydian slash chord**; `G#` arrives on top |
| 3 | `Dmaj7#11` | D | F#3 A3 C#4 G#4 | D F# G# A C# | the complete Lydian tonic, every characteristic note at once |
| 4 | `C#m7` | C# | E3 B3 C#4 G#4 | C# E G# B | the bass finally leaves D |
| 5 | `Eadd9` | E | E3 B3 F#4 G#4 | E F# G# B | `II` in root position |
| 6 | `E/G#` | G# | E3 B3 F#4 G#4 | E F# G# B | **the raised fourth in the bass**; same upper voices as 5 |
| 7 | `Bm7` | B | D3 B3 E4 F#4 | B D E F# | |
| 8 | `F#m9` | F# | F#3 A3 C#4 G#4 | F# G# A C# | **the pivot** — identical upper voices to 3 |
| 9 | `Dmaj7` | D | F#3 A3 C#4 F#4 | D F# A C# | the last chord; settles without closing |

Two of these are doing something worth pointing at.

**Chord 6 is chord 5 with a different foundation.** Nothing in the upper structure moves; the bass steps from `E` to `G#` and a root-position triad becomes a first inversion with the mode's characteristic note underneath it. It is the loudest the `G#` ever gets and not one voice had to move to do it.

**Chord 8 is chord 3 with a different foundation.** The theme's final chord is `Dmaj7#11`; the interlude opens with exactly those four notes over an `F#` bass, and the chord is revealed to have been `F#m9` all along. A pivot like that is free — it is the same four strings — and it is the most elegant seam in the piece.

### 3.3 Voice leading

**This is the difference between "composed" and "generated", and it is the reason the chords are a hand-written table rather than the output of a voicing algorithm.** An automatic voicer produces correct chords and dead part-writing, and dead part-writing is what a listener actually hears.

Three rules, all checked by tests over **every consecutive pair in the piece including the section seams and the loop back to the top**:

1. **No voice moves more than five semitones**, and most move by one or two.
2. **Every pair either shares a common tone or moves entirely by steps and thirds.** Chords 0→1 share three notes and move one voice by a whole step; 5→6 move nothing at all.
3. **No parallel fifths, octaves or unisons** — between any two of the four upper voices, and between the bass and any upper voice.

That third rule is not pedantry. It is the reason the four voices sound like four independent lines rather than one chord being dragged around, and it caught two real defects during construction: a bass that shadowed an inner voice at a compound fifth across the A→B seam, and a pair of upper voices planing in fifths from `C#m7` to `Eadd9`. Both were fixed by re-voicing, not by relaxing the rule.

### 3.4 Harmonic rhythm

**Two bars to a chord: eleven seconds.** Fast enough that the piece moves, slow enough that a chord is a place rather than an event.

---

## 4. The melody

The composition is generated by weighted choice, note by note. The weights are the design; the seed only decides which of the well-formed melodies you get.

### 4.1 Contour

Every four-bar phrase is handed a shape — **arch**, **fall**, **rise** or **hover** — as a curve over the phrase. Each note is pulled toward the curve's current height. A melody without a contour is a walk; a melody with one is a gesture, and it is the single cheapest thing you can add to make generated notes sound intended.

### 4.2 The weights

Each candidate note is scored, and the score is the sum of the following. Every one of them is a real rule of melodic writing rather than a knob.

| Factor | Effect |
| --- | --- |
| **Interval size** | A second is weighted **8.5**, a third **2.0**, a fourth **2.6**, a fifth **1.1**, a sixth **0.18**, a seventh **0.04**, an octave **0.22**. A uniform draw here is exactly what "generative music" sounds like. |
| **Leap recovery** | After a leap of a fourth or more, continuing in the same direction is **forbidden**, and turning round by a step is multiplied by eight. |
| **Contour** | Weight falls off exponentially with distance from the phrase's current target height. |
| **Metric position** | On beats 1 and 4 of the 6/4 bar, **only chord tones are considered** — a weighted fallback exists for the rare bar where the leap rule has left none reachable. |
| **Passing tones** | Off the strong beats a non-chord tone is allowed, but only if it is *approached* by step, and the next note is then required to *leave* it by a step in the same direction. That is the definition of a passing tone, and it is why one never sounds like a wrong note. |
| **The mode** | At the top of an arch, the raised fourth and the major seventh are multiplied by 2.6. |

The resulting distribution over a whole piece is roughly **69% seconds, 18% thirds, 13% fourths or wider** — which is the shape of a real melody, and is asserted as such.

### 4.3 Phrasing

A phrase is four bars, twenty-four beats. The melody sounds for fifteen to twenty of them and then **stops**. The rest is not an absence of material; it is the phrasing. A line that never stops is a drone with opinions.

Note lengths are drawn from one, two, three, four and six beats — all whole beats, because at 66 BPM a single beat is already nine tenths of a second and this piece has no room for ornaments. Phrase-final notes are weighted long.

---

## 5. The form

**A rondo: A A' B A'' C A'''.** Forty-eight bars, four minutes and twenty-two seconds.

| Section | Chords | The melody |
| --- | --- | --- |
| **A** | 0 1 2 3 (pedal) | the motif, as generated |
| **A'** | 0 1 2 3 (pedal) | the motif, transposed up a diatonic third |
| **B** | 4 5 6 3 (bass walks C# E G# D) | free; the piece's climax on `E/G#` |
| **A''** | 0 1 2 3 (pedal) | the motif, **inverted** — every interval negated |
| **C** | 8 4 5 7 (bass walks F# C# E B) | free, low and sparse |
| **A'''** | 0 1 2 9 (pedal) | the motif, **literally as in A** |

**The motif is generated once, by exactly the same weighted rules every other phrase uses**, over the theme's opening harmony. It is five notes. It opens four of the six sections, and the final statement is bit-for-bit the first — a literal return, which is the oldest way there is to end a piece and still the one that makes it recognisable.

Transposing and inverting a motif can put a note somewhere the harmony does not support it. Where that happens on a strong beat the note is snapped to the nearest chord tone: the shape survives, the harmony wins.

### 5.1 Why four and a half minutes

Cues run three to five minutes (10 §4). A piece this length means **a cue is almost exactly one pass**: the loop point is rarely reached, and when it is, chord 9 leads back to chord 0 with two common tones rather than a splice. Both halves of that are asserted — a much shorter piece would be heard going round, and a much longer one would never reach its own second theme.

---

## 6. The sound

### 6.1 Karplus–Strong

Every note is a **plucked string**, physically modelled: a delay line one period long, a two-tap loop filter, and a loop gain.

This is not a stylistic preference dressed up as engineering. It is the cheapest way there is to get a sound that is genuinely a string rather than an imitation of one — the harmonic series falls out of the delay length, the top end decays faster than the fundamental because the loop filter is a low-pass, and the result has the slight inharmonicity and the uneven decay that make a real instrument sound real. A subtractive synth patch aiming at the same target costs more and still sounds like a synthesiser.

The loop filter is `(1-b)·x[n] + b·x[n-1]`. Its group delay is exactly `b` samples, so tuning stays honest, and it is an FIR, so the loop cannot run away however hard it is driven — which matters on an audio callback with no supervision. `b` is the whole of the timbre control: **0.24 for the melody, 0.33 for the harmony, 0.42 for the bass**, opening or closing further with the contextual variants.

### 6.2 The attack

A real string is excited in under a millisecond. This one is not, because 10 §1 outranks the physics: the excitation is a filtered noise burst **injected over one and a half periods**, and the output carries a raised-cosine ramp of twenty to thirty milliseconds on top. Half of peak arrives at three milliseconds or later. The result is a fingertip rather than a plectrum, and it is the difference between a sound that is pleasant on its four hundredth repetition and one that is not.

**Sizing the burst in periods rather than in milliseconds is load-bearing.** A delay line is a resonator: energy poured in goes round the loop and adds to itself, so a burst of fixed duration fills a short line many more times than a long one, and a bass note comes out ten decibels below a treble note struck with identical force. Getting this wrong was audible as a piece whose loudness tracked its register.

### 6.3 Piano or guitar

Guitar. The melody gets a **second string four cents sharp at 55% amplitude** — the way a piano and a twelve-string get their shimmer, and the cheapest "organic" there is: no chorus, no modulation, just two strings beating against each other. Deliberate inharmonicity (a stiff-string allpass, which is what makes a piano a piano) is left out; the brief said "piano **and/or** guitar" and a nylon-string reading of this material is prettier.

### 6.4 Cost

| | |
| --- | --- |
| Per active string, per sample | two loads, a lerp, a one-multiply loop filter, a gain, a store, a masked increment — **about twelve operations** |
| Concurrent strings | around ten typically, twenty-eight slots |
| Whole instrument, worst case | ~350 operations per sample = **7.7 Mop/s at 22.05 kHz** |
| Measured | **52 ns per sample of audio**, a real-time factor of **873×** — about **0.11% of one core** |
| While silent | **one comparison per sample** — the clock does not advance and no string is active |
| Memory | 28 × 512 floats of delay line plus 371 events: **about 64 KB**, allocated once, on the main thread |

Twenty-eight string slots is sized from the score rather than guessed at: ten plucks in a five-and-a-half second bar, each ringing for five seconds. A bank of twelve spends its whole life stealing, and **every steal truncates a string that is still ringing** — both a click and the reason the wash never builds.

---

## 7. Determinism

The generator is **SplitMix64**, seeded from the map seed through one mixing step. Never `DefaultHasher`, whose output is not stable across Rust releases — the multiplayer module already learned that one.

The whole composition is built **once**, when the music voice is created, into a flat list of about 370 events. The audio thread makes no decisions: it walks the list against a sample clock and plucks strings. That is worth having for three reasons — the piece is testable as data, the audio callback has nothing in it that could branch differently on two machines, and the cost of "composing" is paid once at startup rather than continuously.

A new map rebuilds the voice, and only ever between cues.

---

## 8. Context

Chosen at the start of a cue, while the voice is silent, so no control ever sweeps under a sounding note.

| Signal | Reads | Effect |
| --- | --- | --- |
| **Warmth** | town density around the camera | brighter pick, more open instrument |
| **Density** | the same | how much of the composition survives the thinning |
| **Dusk** | the day phase | the melody an octave lower, longer decays, a much darker instrument |

The thinning is the interesting one. Every event carries a structural weight, and a sparse reading keeps the bass on chord changes, the chord changes themselves and the motif, while dropping the inner filigree and the passing notes. **The declining town gets a sparser reading of the same piece, not a sadder one** — and it stays a piece, because what is dropped is chosen by musical function rather than at random.

---

## 9. Rejected, and why

**A chord-symbol table with an automatic voicer.** Correct chords, dead part-writing. The voicings are the composition.

**Continuous generation under the cue envelope.** Every cue would fade in wherever the piece happened to be, and a piece nobody hears the start of is a piece nobody recognises. Cues are numbered instead, and a change of number is "start at bar one".

**A reverb.** A real one costs memory and a cheap one sounds cheap. Five-to-seven second string decays overlapping each other are the space, and they are free — they are the notes.

**Percussion of any kind, including a soft pulse.** 10 §4 rules it out and it was never tempting. The metre is felt through where notes land, which is all a metre has to do.

**4/4.** Six slow beats give a phrase room to rise and fall inside one bar, and the absence of a four-square downbeat is most of what keeps a metre from turning into a beat. 6/4 is the least clever non-4/4 available and that is exactly why it was chosen.

**A key change.** Distinct and recognisable pull the same way here: one key, one motif, one instrument.

---

## 10. Acceptance bar

1. A listener can hum the opening phrase after two cues.
2. The raised fourth is audible as the character of the piece, not as a passing accident.
3. No cadence anywhere. The piece never sounds like it has finished.
4. Every strong beat lands on a chord tone; every leap turns round.
5. The same map always plays the same piece.
6. A player who leaves the game running for two hours does not notice the loop.
7. The generator costs under half a percent of one core, and nothing while silent.
