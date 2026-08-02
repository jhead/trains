# 10 — Audio & Feel

**Audio is half of "calm", and it is the half most often left to the end.** A silent building game feels like a tech demo no matter how good it looks. A well-scored one feels like a place.

This brief also covers *feel* — the small responses that make actions land — because feel is where audio, animation and timing meet, and they cannot be designed separately.

---

## 1. Principle

**The soundtrack of Rail Town is a landscape with a railway in it.**

Not a music-forward game. Not a busy one. The dominant sound most of the time should be ambience — wind, water, distant town noise — with the railway punctuating it. Music is an occasional guest.

Three rules that govern everything below:

1. **Never startle.** No sudden loud events, no stingers, no sharp transients at volume. The player may have this open for hours.
2. **Everything is positional.** Sounds come from where they happen in the world, pan with the camera, and attenuate with distance and zoom. This is what makes the map feel like a space rather than a screen.
3. **Silence is a texture, not a bug.** Gaps between musical cues should be minutes long. The ambience carries them.

---

## 2. Ambience

A continuous bed, composed from what the camera is looking at. Layers cross-fade as the player pans, so moving from coast to mountain to town is an audible journey.

| Layer | Character |
| --- | --- |
| **Wind** | Base layer everywhere, thinner and higher at altitude |
| **Water** | Waves at the coast, running water near rivers, both distance-attenuated |
| **Forest** | Birds, leaves; sparse, with long gaps |
| **Town** | Distant murmur, scaling with density — the sound of a place being lived in |
| **Industry** | Machinery, muffled, only while working |
| **Weather** | Rain and its aftermath; seasonal if seasons are in |

**The town layer is the emotional one.** A thriving district should be audibly alive and a declining one audibly quiet, so the player hears growth and decline before they read it. Combined with lit windows at night, that gives the town two ambient channels of feedback that require no interface at all.

Time of day shifts the whole bed: dawn birdsong, daytime activity, evening quiet, night crickets and near-silence.

---

## 3. The railway

The sounds the player causes, and the ones their network makes.

### 3.1 Building

| Action | Sound |
| --- | --- |
| **Track laid** | A single *clack* per tile — ballast and sleeper. Pitch-varied, rate-limited so a long run is a satisfying rhythmic run rather than a burst. **This is the most-heard sound in the game and deserves the most attention.** |
| Bridge placed | Heavier, with a timber creak |
| Tunnel started | A low rumble |
| Station placed | A short constructive chord — the one moment of near-musicality in the build set |
| Demolish | A dull *clank* and settling debris |
| Invalid | A soft low thud. Quiet, unmistakable, never harsh. It will be heard often and must never become irritating. |
| Tool switch | A minimal click |

The track-laying sound is the game's signature. It should be good enough that players lay track idly, for the feel of it. Pitch variation across a small set, slight timing jitter, and a subtle low-end thump per tile.

### 3.2 Trains

- **Motion** — a rolling loop, pitched and filtered by speed, positional. Different timbre for transit and freight; a heavy goods train should be audibly heavy.
- **Starting and stopping** — a chuff or whine on departure, brakes on arrival. These punctuate the map's rhythm.
- **Whistle** — on departure and at crossings. Sparing. A distant whistle across a valley is one of the best sounds available to this game and it is ruined by overuse.
- **Level crossings** — a bell while barriers are down, attenuating quickly with distance.
- **Passing** — a doppler sweep when a train passes near the camera.

### 3.3 The town

- Construction sounds while a building goes up.
- Platform crowd murmur, scaling with how many are waiting — **a busy platform should sound busy**, which turns crowding into an ambient diagnostic.
- Small human sounds around dense districts, sparse and never looping obviously.

---

## 4. Music

Sparse, long, and ambient. Slow harmonic movement, acoustic and analogue in character, no percussion driving anything.

- **First cue enters a minute or two in**, never immediately at launch.
- **Cues run three to five minutes**, then silence for three to eight. Ambience fills the gap.
- **The stack thins during interaction** — laying a long run, the music ducks slightly so the *clacks* have room.
- **Contextual variants**: a warmer palette when the network is thriving, sparser when the town is declining. Never dramatic, never a fail cue.
- **A dusk piece.** The prettiest minute of the day cycle deserves its own music, and players will remember it.

Explicitly avoided: loops short enough to notice, anything that builds to a climax, anything with a strong beat, and any cue that comments on failure.

---

## 5. Interface sound

Quiet, low, soft-edged. UI sound in a calm game should be felt more than heard.

| Element | Sound |
| --- | --- |
| Hover | Nothing, or barely-there |
| Click | A soft low tick |
| Panel open / close | A brief airy sweep |
| Toggle | Two-tone, up for on |
| Money gained | A warm low chime, **rate-limited and aggregated** — a busy network must not become a slot machine |
| Money spent | A softer counterpart |
| Alert | Gentle, two-note, never urgent |
| Milestone | The one genuinely warm moment. Rare enough to stay special. |

The money sound needs care. Income arrives constantly in a working network; unthrottled, it becomes a coin-drop machine and destroys the calm. Aggregate over a window and play once.

---

## 6. Feel

Feel is what makes an action land, and it is where audio, animation and timing combine. The unifying principle: **every action produces a proportionate response in at least two channels.**

| Action | Visual | Audio | Timing |
| --- | --- | --- | --- |
| Tile placed | 2-frame settle, dust puff | *clack* | Immediate |
| Run committed | Staggered settle along the route | Rhythmic run of clacks | ~20 ms stagger |
| Invalid | Ghost turns `warn`, reason chip | Soft thud | Immediate |
| Money changes | Floating delta near the balance | Chime, aggregated | ~1 s fade |
| Train arrives | Brake, dwell, doors | Brakes, crowd shift | Over ~2 s |
| Building completes | Settle, dust, windows | Construction stop | Over ~8 s |
| Selection | 1-texel bright outline | Soft tick | Immediate |
| Alert | Two flashes then steady | Gentle two-note | Twice, then static |

**No screen shake. No hit-stop. No zoom punch. No particles for their own sake.** Those are the vocabulary of a different genre, and every one of them would break the calm this game is built on.

The one exception is the **first payout of a new game**, which gets slightly more than it strictly deserves — a warm chime, a clear floating number, and a Town Talk line. That is the loop closing for the first time and it should be unmistakable.

---

## 7. Mixing

- **Buses**: master, music, ambience, effects, UI, each independently controllable.
- **Ducking**: build sounds duck music slightly; alerts duck ambience slightly. Nothing ducks hard.
- **Dynamic range is narrow.** No sound is dramatically louder than another. The loudest thing in the game is a nearby train whistle, and it is not very loud.
- **Distance attenuation and low-pass** with distance, so far-off sounds are muffled as well as quieter.
- **Zoom affects the mix** — at 1× the world sounds distant and wide; at 3× the player is down among the trains.
- **Voice limits per category**, with the nearest instances winning, so a large network never turns to mush.
- **Mute on focus loss**, by default.

---

## 8. Acceptance bar

1. A player can tell, with their eyes closed, whether the town near the camera is thriving.
2. Laying track is satisfying enough to do idly.
3. Nothing in the game ever startles.
4. A busy network does not sound like a slot machine.
5. Playing for two hours does not produce audio fatigue.
6. Muting the game makes it noticeably worse.
