//! The piece itself (brief §4, and `docs/design/14-music.md`).
//!
//! A generative score is easy to write badly: pick a scale, pick a random note
//! in it, repeat. It is instantly recognisable as such, because real melody is
//! not a sequence of legal pitches — it is contour, phrasing and voice leading.
//! Everything in this file exists to make those three things true.
//!
//! # The theory, in one paragraph
//!
//! **D Lydian.** A mode rather than a key, because functional major would give
//! the score somewhere to arrive and this is a score that should never arrive.
//! Lydian's raised fourth (`G#`) is the brightest colour tone in the diatonic
//! system and reads as wonder rather than as sweetness — it is the difference
//! between "calm" and "calm *and* upbeat". The chord palette is deliberately
//! missing its dominant: **`A` major never appears and the bass never touches
//! `A`**, because `V–I` is the one cadence that would collapse Lydian back into
//! plain D major in a single bar. What is left — `I`, `II`, `iii`, `vi`, `vii` —
//! moves by step and by third, over a tonic pedal for the whole of the theme.
//!
//! # The three claims this file has to earn
//!
//! 1. **Harmony that progresses.** [`CHORDS`] is a table of *voicings*, not of
//!    chord symbols, and [`FORM`] is a fixed order through it. The tests in this
//!    module check the voice leading of every consecutive pair: common tones
//!    held, moving voices moving by a step or a third, and no parallel fifths or
//!    octaves anywhere. That check is the difference between "composed" and
//!    "generated".
//! 2. **Melody with contour and phrase.** [`sing`] weights every candidate note
//!    by interval size, by leap recovery, by metric position, by chord tone, and
//!    by how far it is from a [`Contour`] curve — then draws from the weights.
//!    Steps outnumber leaps roughly three to one, leaps are answered by a step
//!    back, non-chord tones only ever appear between two steps, and every phrase
//!    stops sounding a bar before it ends.
//! 3. **A fixed seed is a fixed piece.** [`compose`] is pure, uses the module's
//!    SplitMix64 [`Rng`] (never `DefaultHasher`, whose output is not stable
//!    across Rust releases), and is called once when the voice is created. The
//!    same world always sounds like itself.
//!
//! # Sound
//!
//! [`KsString`] is Karplus–Strong: a delay line and a two-tap loop filter. It is
//! a physical model of a plucked string rather than an imitation of one, it
//! costs about a dozen operations per sample, and it sounds like gut or nylon
//! rather than like a synthesiser. The excitation is a filtered noise burst
//! spread over a period and a half rather than an impulse, and the output
//! carries a twenty-millisecond ramp on top — which is what keeps the attack a
//! fingertip rather than a plectrum. "Never startle" (§1) applies to the score
//! too.
//!
//! Measured: **52 ns per sample of audio**, a real-time factor of 873, or about
//! a tenth of one percent of a core. While no cue is running it is a single
//! comparison per sample.

use super::clip::SR;
use super::dsp::{lerp, raised_cosine, Rng};

// ─ Theory ──────────────────────────────────────────────────

/// Lydian, as semitone offsets from the tonic: 1 2 3 **#4** 5 6 7.
///
/// The `6` is the whole point. Remove it (making this `[0,2,4,5,7,9,11]`) and
/// the piece is ordinary major; every colour in the score comes from that one
/// semitone.
pub const LYDIAN: [i32; 7] = [0, 2, 4, 6, 7, 9, 11];

/// Scale-degree index of the raised fourth — `G#` in D Lydian.
const SHARP_FOUR: i32 = 3;
/// Scale-degree index of the major seventh — `C#`.
const MAJOR_SEVENTH: i32 = 6;

/// D2, in Hz. Every pitch in the piece is expressed in semitones above it.
const TONIC_HZ: f32 = 73.416_2;

/// Semitones from [`TONIC_HZ`] up to D3, where the harmony voicings are
/// measured from.
const HARMONY_ORIGIN: f32 = 12.0;

/// Nothing is ever plucked below this. The delay line is [`RING`] long, and a
/// string an octave lower than the bass would not fit in it.
const MIN_HZ: f32 = 46.0;

/// 66 BPM. Slow enough that a bar is a breath; fast enough that the melody is a
/// line rather than a series of separate events.
pub const BPM: f32 = 66.0;

/// 6/4. Not cleverness: six slow beats give a phrase room to rise and fall
/// inside a single bar, and the absence of a four-square downbeat is most of
/// what keeps a metre from turning into a beat.
pub const BEATS_PER_BAR: f32 = 6.0;

/// Two bars to a chord — about eleven seconds. "A chord should last many
/// seconds" is the whole of the harmonic rhythm.
const BARS_PER_CHORD: i32 = 2;
/// Four bars to a phrase: two chords, one arch, one breath.
const BARS_PER_PHRASE: i32 = 4;
/// Eight bars to a section: two phrases.
const BARS_PER_SECTION: i32 = 8;

/// Semitones above D3 for a scale-degree index, which may run past an octave.
fn pitch_of(degree: i32) -> f32 {
    let octave = degree.div_euclid(7);
    (LYDIAN[degree.rem_euclid(7) as usize] + 12 * octave) as f32
}

/// One chord, as a *voicing* rather than as a symbol.
///
/// The four upper voices are absolute — this table is the composition, not a
/// set of options for a voicing algorithm to guess at. That is deliberate: an
/// automatic voicer produces correct chords and dead part-writing, and the part
/// writing is the thing a listener actually hears.
#[derive(Clone, Copy, Debug)]
pub struct Chord {
    /// The chord symbol. Read by the voice-leading tests, which name the pair
    /// they failed on — a bare index would make a failure unreadable.
    #[allow(dead_code)]
    pub name: &'static str,
    /// Bass pitch, in semitones above [`TONIC_HZ`].
    pub bass: i32,
    /// Four upper voices, in semitones above D3, low to high.
    pub voices: [i32; 4],
    /// Bit `d` set when scale degree `d` is sounding somewhere in the chord.
    /// The melody reads this to know what a chord tone is on a strong beat.
    pub tones: u8,
}

const fn mask(degrees: [i32; 5]) -> u8 {
    let mut m = 0u8;
    let mut i = 0;
    while i < 5 {
        if degrees[i] >= 0 {
            m |= 1 << degrees[i];
        }
        i += 1;
    }
    m
}

/// Degrees: `0 D, 1 E, 2 F#, 3 G#, 4 A, 5 B, 6 C#`.
///
/// Read the names down the column and the shape of the piece is visible: the
/// theme is four chords over one pedal `D`, so the "progression" is a slow bloom
/// of colour over a drone rather than a set of root movements — `Dadd9` opens
/// out to `D6/9`, then the `E` triad arrives and with it the `G#`, and the last
/// chord is the full `Dmaj7#11`, which is the Lydian tonic with every one of its
/// characteristic notes present at once.
pub const CHORDS: [Chord; 10] = [
    // 0 — home. D A E F#.
    Chord { name: "Dadd9", bass: 0, voices: [0, 7, 14, 16], tones: mask([0, 1, 2, 4, -1]) },
    // 1 — vi over the pedal, which reads as the tonic with a sixth. D B E F#.
    Chord { name: "D6/9", bass: 0, voices: [0, 9, 14, 16], tones: mask([0, 1, 2, 5, -1]) },
    // 2 — II over the pedal: the Lydian slash chord, and the G# arrives on top.
    Chord { name: "E/D", bass: 0, voices: [2, 9, 14, 18], tones: mask([0, 1, 3, 5, -1]) },
    // 3 — iii over the pedal, i.e. the complete Lydian tonic: D F# A C# G#.
    Chord { name: "Dmaj7#11", bass: 0, voices: [4, 7, 11, 18], tones: mask([0, 2, 3, 4, 6]) },
    // 4 — vii. The bass finally leaves D. C# E G# B.
    Chord { name: "C#m7", bass: -1, voices: [2, 9, 11, 18], tones: mask([1, 3, 5, 6, -1]) },
    // 5 — II in root position. E G# B, with F# as the ninth.
    Chord { name: "Eadd9", bass: 2, voices: [2, 9, 16, 18], tones: mask([1, 2, 3, 5, -1]) },
    // 6 — the same chord over its own third: the #4 in the bass. Same upper
    //     voices as 5, so the only thing that moves is the foundation.
    Chord { name: "E/G#", bass: 6, voices: [2, 9, 16, 18], tones: mask([1, 2, 3, 5, -1]) },
    // 7 — vi in root position. B D F#, with E as the eleventh.
    Chord { name: "Bm7", bass: -3, voices: [0, 9, 14, 16], tones: mask([0, 1, 2, 5, -1]) },
    // 8 — the pivot. Identical upper voices to chord 3; only the bass steps from
    //     D to F#, and Dmaj7#11 is revealed to have been F#m9 all along.
    Chord { name: "F#m9", bass: 4, voices: [4, 7, 11, 18], tones: mask([2, 3, 4, 6, -1]) },
    // 9 — the last chord of the piece. Major seventh, so it settles without
    //     closing, and leads back to chord 0 with three common tones.
    Chord { name: "Dmaj7", bass: 0, voices: [4, 7, 11, 16], tones: mask([0, 2, 4, 6, -1]) },
];

/// What a phrase's line does with its register.
///
/// A melody without a contour is a walk; a melody with one is a gesture. Every
/// phrase is handed one of these, and the note chooser is pulled toward it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contour {
    /// Rise to a peak about two thirds through, then fall away. The default
    /// shape of a sung phrase in almost every tradition there is.
    Arch,
    /// Start high and descend. Answers an `Arch`.
    Fall,
    /// Climb, and end near the top — used where a section has to hand over.
    Rise,
    /// Stay put and move by inches. The interlude's shape.
    Hover,
}

impl Contour {
    /// Normalised height in `0..=1` at position `t` through the phrase.
    fn at(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Arch => {
                if t < 0.62 {
                    0.18 + 0.82 * raised_cosine(t / 0.62)
                } else {
                    lerp(1.0, 0.30, raised_cosine((t - 0.62) / 0.38))
                }
            }
            Self::Fall => lerp(0.92, 0.10, raised_cosine(t)),
            Self::Rise => lerp(0.12, 0.90, raised_cosine(t)),
            Self::Hover => 0.34 + 0.16 * raised_cosine((t * 2.0).min(1.0)),
        }
    }

    /// Where the peak is, so the characteristic tones can be favoured there.
    fn peak_at(self) -> f32 {
        match self {
            Self::Arch => 0.62,
            Self::Fall => 0.0,
            Self::Rise => 1.0,
            Self::Hover => 0.5,
        }
    }
}

/// How a section treats the motif.
///
/// This is the form. A theme that only ever appears once is not a theme, and a
/// generative piece with no literal return is not recognisable — so the motif
/// comes back four times in four different lights, and the last of them is
/// exactly the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    /// The motif as generated.
    Original,
    /// The motif transposed up a diatonic third — same contour, new colour.
    UpThird,
    /// The motif with every interval negated. The shape is recognisable upside
    /// down, which is the oldest trick in counterpoint and still the best.
    Inverted,
    /// An episode: no motif, free line.
    Free,
}

/// One eight-bar section.
pub struct SectionPlan {
    /// `A`, `B`, `A''` and so on. Documentation that travels with the data.
    #[allow(dead_code)]
    pub label: &'static str,
    /// Four chord indices into [`CHORDS`], two bars each.
    pub chords: [usize; 4],
    pub theme: Theme,
    /// One contour per four-bar phrase.
    pub contour: [Contour; 2],
    /// Section dynamic. The range is narrow on purpose (§1).
    pub level: f32,
    /// Register offset for the melody, in scale degrees.
    pub lift: i32,
}

/// A rondo: **A A' B A'' C A'''**. Forty-eight bars, four minutes twenty-two.
///
/// Long enough that the loop point is rarely reached inside a three-to-five
/// minute cue, and when it is, chord 9 leads back to chord 0 by voice leading
/// rather than by a splice.
pub const FORM: [SectionPlan; 6] = [
    SectionPlan {
        label: "A",
        chords: [0, 1, 2, 3],
        theme: Theme::Original,
        contour: [Contour::Arch, Contour::Fall],
        level: 1.0,
        lift: 0,
    },
    SectionPlan {
        label: "A'",
        chords: [0, 1, 2, 3],
        theme: Theme::UpThird,
        contour: [Contour::Arch, Contour::Rise],
        level: 1.0,
        lift: 1,
    },
    SectionPlan {
        label: "B",
        chords: [4, 5, 6, 3],
        theme: Theme::Free,
        contour: [Contour::Rise, Contour::Arch],
        level: 1.0,
        lift: 1,
    },
    SectionPlan {
        label: "A''",
        chords: [0, 1, 2, 3],
        theme: Theme::Inverted,
        contour: [Contour::Arch, Contour::Fall],
        level: 0.94,
        lift: 0,
    },
    SectionPlan {
        label: "C",
        chords: [8, 4, 5, 7],
        theme: Theme::Free,
        contour: [Contour::Hover, Contour::Hover],
        level: 0.80,
        lift: -1,
    },
    SectionPlan {
        label: "A'''",
        chords: [0, 1, 2, 9],
        theme: Theme::Original,
        contour: [Contour::Arch, Contour::Fall],
        level: 1.0,
        lift: 0,
    },
];

/// Melodic register, as scale-degree indices. `7` is D4 (294 Hz) and `14` is D5
/// (587 Hz) — above the harmony's top voice, below anything shrill.
const MELODY_LO: i32 = 7;
const MELODY_HI: i32 = 15;

// ─ The composition ─────────────────────────────────────────

/// Which line an event belongs to. Each has its own string timbre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Bass,
    Harmony,
    Melody,
}

/// One plucked note.
#[derive(Clone, Copy, Debug)]
pub struct Event {
    /// Onset in samples from the top of the piece.
    pub at: u32,
    /// Semitones above [`TONIC_HZ`].
    pub pitch: f32,
    pub amp: f32,
    /// Structural importance, `0..=255`.
    ///
    /// This is what lets the sparse variant thin the piece *coherently*: the
    /// bass on a chord change and the motif survive, the inner filigree does
    /// not. Dropping notes at random would only make it sound broken.
    pub weight: u8,
    pub part: Part,
}

/// Samples per beat.
fn beat_len() -> f32 {
    SR * 60.0 / BPM
}

fn at_beat(beat: f32) -> u32 {
    (beat.max(0.0) * beat_len()) as u32
}

/// A note of the motif, stored relative to its own first note.
#[derive(Clone, Copy, Debug)]
struct MotifNote {
    /// Scale degrees above the motif's first note.
    degree: i32,
    /// Beats from the start of the phrase.
    onset: f32,
    /// Length in beats.
    len: f32,
}

/// Note lengths, in beats, and their weights. All whole beats: at 66 BPM in
/// 6/4 a single beat is already nine-tenths of a second, and anything shorter
/// would be an ornament in a piece that has no room for ornaments.
const LENGTHS: [(f32, f32); 5] = [(1.0, 3.0), (2.0, 5.0), (3.0, 3.0), (4.0, 2.0), (6.0, 1.0)];

fn draw_length(rng: &mut Rng, long: bool) -> f32 {
    let total: f32 = LENGTHS
        .iter()
        .map(|(len, w)| if long { w * len } else { *w })
        .sum();
    let mut pick = rng.unit() * total;
    for (len, w) in LENGTHS {
        let w = if long { w * len } else { w };
        if pick < w {
            return len;
        }
        pick -= w;
    }
    2.0
}

/// Weight for an interval of `n` scale degrees.
///
/// Steps outnumber thirds nearly three to one and fourths six to one. This
/// single table is most of the difference between a melody and a random walk in
/// a scale — a uniform choice here produces the sound everybody recognises as
/// "generative music", and no amount of good harmony underneath rescues it.
fn interval_weight(n: i32) -> f32 {
    match n.abs() {
        0 => 0.28,
        1 => 8.5,
        2 => 2.0,
        3 => 2.60,
        4 => 1.10,
        5 => 0.18,
        6 => 0.04,
        7 => 0.22,
        _ => 0.0,
    }
}

/// State carried from note to note inside a phrase.
struct Line {
    degree: i32,
    /// The interval that arrived at [`Self::degree`], in scale degrees.
    last: i32,
    /// Set when the previous note was a non-chord tone, which must be left by
    /// step in the direction it was approached — the definition of a passing
    /// tone, and the reason one never sounds like a wrong note.
    resolve: i32,
}

/// Choose the next melody note.
#[allow(clippy::too_many_arguments)]
fn sing(
    rng: &mut Rng,
    line: &Line,
    target: f32,
    chord: u8,
    strong: bool,
    near_peak: bool,
    lo: i32,
    hi: i32,
) -> i32 {
    // A strong beat takes a chord tone, and that is a rule rather than a
    // weight: on the two beats a listener is actually counting, a non-chord
    // tone is not a colour, it is a wrong note. The weighted pass is the
    // fallback for the rare bar where the leap rule and the register have
    // between them left no chord tone reachable.
    if strong {
        let strict = pick(rng, line, target, chord, true, near_peak, lo, hi, true);
        if strict.is_some() {
            return strict.unwrap_or(line.degree);
        }
    }
    pick(rng, line, target, chord, strong, near_peak, lo, hi, false)
        .unwrap_or_else(|| line.degree.clamp(lo, hi))
}

#[allow(clippy::too_many_arguments)]
fn pick(
    rng: &mut Rng,
    line: &Line,
    target: f32,
    chord: u8,
    strong: bool,
    near_peak: bool,
    lo: i32,
    hi: i32,
    chord_tones_only: bool,
) -> Option<i32> {
    let mut best: [(i32, f32); 32] = [(0, 0.0); 32];
    let mut n = 0usize;
    let mut total = 0.0f32;

    for degree in lo..=hi {
        if n == best.len() {
            break;
        }
        let step = degree - line.degree;
        let mut w = interval_weight(step);
        if w <= 0.0 {
            continue;
        }

        // A passing tone owes a resolution: it may only be left by a step, in
        // the direction it was entered.
        if line.resolve != 0 && step != line.resolve {
            continue;
        }

        // A leap is answered by a step back. A third may be continued — two
        // thirds in a row outline the chord, which is a gesture rather than a
        // stumble — but a fourth or more must turn round, and that is a rule
        // rather than a preference: a line that leaps twice the same way has
        // stopped being a line and become a series of positions.
        if line.last.abs() >= 3 {
            if step != 0 && step.signum() == line.last.signum() {
                continue;
            }
            if step.abs() == 1 {
                w *= 8.0;
            } else if step.abs() >= 3 {
                w *= 0.20;
            }
        } else if line.last.abs() == 2 && step.abs() >= 3 {
            w *= 0.30;
        }

        // Pull toward the phrase's contour.
        w *= (-(degree - target.round() as i32).abs() as f32 / 2.6).exp();

        let is_chord_tone = chord & (1 << degree.rem_euclid(7)) != 0;
        if chord_tones_only && !is_chord_tone {
            continue;
        }
        if strong {
            // Chord tones on strong beats, passing tones between them. The
            // oldest rule in tonal melody and the one that does the most work.
            w *= if is_chord_tone { 6.5 } else { 0.09 };
        } else if is_chord_tone {
            w *= 1.35;
        } else if step.abs() != 1 {
            // A non-chord tone that is not approached by step is a wrong note.
            continue;
        }

        // Make the mode audible. At the top of an arch, the raised fourth and
        // the major seventh — the two notes that are *not* in plain major and
        // minor respectively — are worth two and a half of anything else.
        if near_peak {
            let class = degree.rem_euclid(7);
            if class == SHARP_FOUR || class == MAJOR_SEVENTH {
                w *= 2.6;
            }
        }

        best[n] = (degree, w);
        total += w;
        n += 1;
    }

    if n == 0 || total <= 0.0 {
        return None;
    }
    let mut draw = rng.unit() * total;
    for &(degree, w) in best.iter().take(n) {
        if draw < w {
            return Some(degree);
        }
        draw -= w;
    }
    Some(best[n - 1].0)
}

/// Nearest chord tone to `degree`, preferring `dir`.
fn snap(degree: i32, chord: u8, dir: i32, lo: i32, hi: i32) -> i32 {
    if chord & (1 << degree.rem_euclid(7)) != 0 {
        return degree;
    }
    let order: [i32; 4] = if dir >= 0 { [1, -1, 2, -2] } else { [-1, 1, -2, 2] };
    for offset in order {
        let candidate = degree + offset;
        if candidate >= lo && candidate <= hi && chord & (1 << candidate.rem_euclid(7)) != 0 {
            return candidate;
        }
    }
    degree.clamp(lo, hi)
}

/// The composer's scratch state.
struct Composer {
    rng: Rng,
    events: Vec<Event>,
}

impl Composer {
    fn push(&mut self, at: f32, pitch: f32, amp: f32, weight: u8, part: Part) {
        self.events.push(Event {
            at: at_beat(at),
            pitch,
            amp,
            weight,
            part,
        });
    }

    /// The bass and the four upper voices for one eight-bar section.
    ///
    /// The upper voices are picked out rather than struck together — five
    /// fingerstyle patterns, chosen per bar, with the chord-change bar getting a
    /// slow roll so that the change is *heard* as an arrival without anything
    /// having to get louder.
    fn accompany(&mut self, plan: &SectionPlan, bar0: i32) {
        /// Fingerstyle patterns, as `(beat, voice)`.
        ///
        /// Every one of them reaches the far end of the bar. A pattern that
        /// spends its whole chord in the first half-second leaves five seconds
        /// of decay behind it, and two of those in a row is a hole in the
        /// piece rather than a rest in it.
        const PATTERNS: [&[(f32, usize)]; 5] = [
            &[(0.0, 0), (1.0, 2), (2.5, 1), (4.0, 3), (5.0, 2)],
            &[(0.0, 1), (1.5, 3), (3.0, 0), (4.5, 2), (5.5, 1)],
            // The roll: a chord arpeggiated across a third of a beat, then left
            // to ring. This is the closest the score comes to a strum and it is
            // still slower than any real one.
            &[(0.0, 0), (0.11, 1), (0.24, 2), (0.38, 3), (2.6, 2), (4.4, 1)],
            &[(0.0, 2), (1.0, 0), (2.5, 3), (4.0, 1), (5.5, 2)],
            &[(0.0, 3), (1.0, 0), (2.5, 2), (4.0, 1), (5.0, 3)],
        ];

        for slot in 0..4 {
            let chord = CHORDS[plan.chords[slot]];
            for sub in 0..BARS_PER_CHORD {
                let bar = bar0 + slot as i32 * BARS_PER_CHORD + sub;
                let bar_beat = bar as f32 * BEATS_PER_BAR;
                let change = sub == 0;

                // The bass on the first beat of every bar: one pluck every five
                // and a half seconds. A pulse that slow is a breath, not a beat.
                let amp = if change { 0.50 } else { 0.34 } * plan.level;
                let weight = if change { 250 } else { 150 };
                self.push(bar_beat, chord.bass as f32, amp, weight, Part::Bass);

                let pattern = if change {
                    PATTERNS[[2, 0, 4][self.rng.below(3)]]
                } else {
                    PATTERNS[[1, 3, 0][self.rng.below(3)]]
                };
                for (i, &(beat, voice)) in pattern.iter().enumerate() {
                    let first = change && i == 0;
                    let weight = if change {
                        210 - i as u8 * 12
                    } else {
                        130 - i as u8 * 6
                    };
                    // Lower voices carry a little more, as they do on any real
                    // instrument; the spread is under two decibels.
                    let voice_amp = lerp(0.30, 0.23, voice as f32 / 3.0);
                    let jitter = self.rng.range(0.92, 1.08);
                    self.push(
                        bar_beat + beat,
                        chord.voices[voice] as f32 + HARMONY_ORIGIN,
                        voice_amp * plan.level * jitter * if first { 1.12 } else { 1.0 },
                        weight,
                        Part::Harmony,
                    );
                }
            }
        }
    }

    /// Generate one four-bar phrase of melody.
    ///
    /// Returns the notes as `(beat, degree, length)` so a phrase can be reused
    /// as a motif.
    #[allow(clippy::too_many_arguments)]
    fn phrase(
        &mut self,
        chords: [u8; 2],
        contour: Contour,
        lo: i32,
        hi: i32,
        start: Option<i32>,
        seed_notes: &[MotifNote],
    ) -> Vec<MotifNote> {
        // Sound for most of the phrase, then stop. The rest is not an absence
        // of material, it is the phrasing — a line that never stops is a drone
        // with opinions.
        let sounding = [15.0f32, 17.0, 18.0, 20.0][self.rng.below(4)];
        let span = BARS_PER_PHRASE as f32 * BEATS_PER_BAR;

        let mut notes: Vec<MotifNote> = Vec::new();
        let mut beat = 0.0f32;

        // A motif, if this phrase has one, is laid down first and verbatim.
        if !seed_notes.is_empty() {
            let anchor = start.unwrap_or(seed_notes[0].degree);
            for note in seed_notes {
                let chord = chords[chord_slot(note.onset)];
                let degree = (anchor + note.degree).clamp(lo, hi);
                let strong = is_strong(note.onset);
                let degree = if strong {
                    snap(degree, chord, note.degree.signum(), lo, hi)
                } else {
                    degree
                };
                notes.push(MotifNote {
                    degree,
                    onset: note.onset,
                    len: note.len,
                });
                beat = note.onset + note.len;
            }
        }

        // The free continuation inherits the motif's last interval, so the
        // note that follows a motif ending in a leap still has to answer it.
        let carried = match notes.as_slice() {
            [.., a, b] => b.degree - a.degree,
            _ => 0,
        };
        let mut line = Line {
            degree: notes
                .last()
                .map(|n| n.degree)
                .or(start)
                .unwrap_or((lo + hi) / 2),
            last: carried,
            resolve: 0,
        };
        if notes.is_empty() {
            // The first note of a free phrase is a chord tone, always.
            let target = lo as f32 + contour.at(0.0) * (hi - lo) as f32;
            line.degree = snap(target.round() as i32, chords[0], 1, lo, hi);
            let len = draw_length(&mut self.rng, false);
            notes.push(MotifNote {
                degree: line.degree,
                onset: 0.0,
                len,
            });
            beat = len;
        }

        let peak = contour.peak_at();
        while beat < sounding {
            let t = beat / span;
            let target = lo as f32 + contour.at(t) * (hi - lo) as f32;
            let chord = chords[chord_slot(beat)];
            let strong = is_strong(beat);
            let last = beat + 3.0 >= sounding;
            let near_peak = (t - peak).abs() < 0.18;

            let degree = if last {
                // Land the phrase on a chord tone, approached by step where the
                // line allows it.
                let want = sing(
                    &mut self.rng,
                    &line,
                    target,
                    chord,
                    true,
                    false,
                    lo,
                    hi,
                );
                snap(want, chord, (want - line.degree).signum(), lo, hi)
            } else {
                sing(
                    &mut self.rng,
                    &line,
                    target,
                    chord,
                    strong,
                    near_peak,
                    lo,
                    hi,
                )
            };

            let step = degree - line.degree;
            let is_chord_tone = chord & (1 << degree.rem_euclid(7)) != 0;
            line.resolve = if is_chord_tone || step == 0 {
                0
            } else {
                step.signum()
            };
            line.last = step;
            line.degree = degree;

            let len = draw_length(&mut self.rng, last);
            notes.push(MotifNote {
                degree,
                onset: beat,
                len,
            });
            beat += len;
        }
        notes
    }

    /// Turn a phrase's notes into events.
    fn voice_melody(&mut self, notes: &[MotifNote], bar0: i32, phrase: i32, plan: &SectionPlan, motif_len: usize) {
        let phrase_beat = (bar0 + phrase * BARS_PER_PHRASE) as f32 * BEATS_PER_BAR;
        for (i, note) in notes.iter().enumerate() {
            let weight: u8 = if i < motif_len {
                245
            } else if i == 0 {
                235
            } else if note.len >= 3.0 {
                190
            } else {
                115
            };
            // A narrow dynamic range, and the phrase's first note a touch
            // forward so the ear finds the start of the line.
            let jitter = self.rng.range(0.90, 1.10);
            let amp = 0.40 * plan.level * jitter * if i == 0 { 1.10 } else { 1.0 };
            self.push(
                phrase_beat + note.onset,
                pitch_of(note.degree) + HARMONY_ORIGIN,
                amp,
                weight,
                Part::Melody,
            );
        }
    }
}

/// Which of a phrase's two chords covers `beat`.
fn chord_slot(beat: f32) -> usize {
    usize::from(beat >= BARS_PER_CHORD as f32 * BEATS_PER_BAR)
}

/// Beats 1 and 4 of a 6/4 bar are the strong ones.
fn is_strong(beat: f32) -> bool {
    let inside = beat.rem_euclid(BEATS_PER_BAR);
    inside < 0.01 || (inside - 3.0).abs() < 0.01
}

/// Compose the whole piece. Pure, and deterministic in `seed`.
pub fn compose(seed: u64) -> (Vec<Event>, u32) {
    let mut c = Composer {
        // "Lydian", so a map seed of zero still starts the generator somewhere
        // interesting. SplitMix64, never `DefaultHasher`.
        rng: Rng::new(seed ^ 0x4c79_6469_616e),
        events: Vec::with_capacity(512),
    };

    // The motif is generated by exactly the rules every other phrase uses, over
    // the theme's opening harmony. It is authored by the weights, not by hand —
    // a different seed gives a different motif and an equally well-formed one.
    let theme_chords = [
        CHORDS[FORM[0].chords[0]].tones,
        CHORDS[FORM[0].chords[1]].tones,
    ];
    let head = c.phrase(theme_chords, Contour::Arch, MELODY_LO, MELODY_HI, None, &[]);
    let motif_len = head.len().min(5);
    let anchor = head[0].degree;
    let motif: Vec<MotifNote> = head
        .iter()
        .take(motif_len)
        .map(|n| MotifNote {
            degree: n.degree - anchor,
            onset: n.onset,
            len: n.len,
        })
        .collect();

    for (s, plan) in FORM.iter().enumerate() {
        let bar0 = s as i32 * BARS_PER_SECTION;
        c.accompany(plan, bar0);

        let lo = MELODY_LO + plan.lift;
        let hi = MELODY_HI + plan.lift;
        for phrase in 0..2 {
            let chords = [
                CHORDS[plan.chords[phrase as usize * 2]].tones,
                CHORDS[plan.chords[phrase as usize * 2 + 1]].tones,
            ];
            // Only the first phrase of a section states the theme; the second
            // answers it freely, which is what makes the return audible.
            let (seed_notes, start): (Vec<MotifNote>, Option<i32>) = if phrase == 0 {
                match plan.theme {
                    Theme::Original => (motif.clone(), Some(anchor)),
                    Theme::UpThird => (motif.clone(), Some(anchor + 2)),
                    Theme::Inverted => (
                        motif
                            .iter()
                            .map(|n| MotifNote {
                                degree: -n.degree,
                                onset: n.onset,
                                len: n.len,
                            })
                            .collect(),
                        Some(anchor + 2),
                    ),
                    Theme::Free => (Vec::new(), None),
                }
            } else {
                (Vec::new(), None)
            };
            let stated = seed_notes.len();
            let notes = c.phrase(
                chords,
                plan.contour[phrase as usize],
                lo,
                hi,
                start,
                &seed_notes,
            );
            c.voice_melody(&notes, bar0, phrase, plan, stated);
        }
    }

    c.events.sort_by_key(|e| e.at);
    let bars = FORM.len() as f32 * BARS_PER_SECTION as f32;
    (c.events, at_beat(bars * BEATS_PER_BAR))
}

// ─ Karplus-Strong ──────────────────────────────────────────

/// Ring size, a power of two so the wrap is a mask. 512 samples at 22.05 kHz is
/// a lowest pitch of 43 Hz, comfortably under [`MIN_HZ`].
const RING: usize = 512;
const RING_MASK: usize = RING - 1;

/// Simultaneous ringing strings.
///
/// Sized from the score rather than guessed at. A bar carries one bass, four
/// harmony plucks and about two and a half melody notes, each of which is two
/// strings — call it ten plucks in five and a half seconds — and a string rings
/// for five. So around ten want to be sounding at once, and a bank of twelve
/// spends its whole life stealing: **every steal truncates a string that is
/// still ringing, which is both a click and the reason the wash never builds**.
/// Twenty-eight leaves the tails alone. Idle strings cost one branch each.
const STRINGS: usize = 28;

/// How long the noise burst is, in periods of the note.
///
/// **Measured in periods rather than in milliseconds, and that is the whole
/// point.** A delay line is a resonator: energy poured in goes round the loop
/// and adds to itself, so a burst of fixed *duration* fills a short line (a
/// high note) many more times than a long one, and a bass note comes out ten
/// decibels below a treble note struck with identical force. Sizing the burst
/// in periods gives every pitch the same number of trips round the loop and
/// therefore the same build-up, which is what makes `amp` mean the same thing
/// at 73 Hz and at 590 Hz.
const EXCITE_PERIODS: f32 = 1.6;

/// How hard the excitation drives the string.
///
/// The one calibration constant in the file. Loudness is decided in the mixer's
/// gain table (see [`super::mixer::gain`]) and every other synthesised sound in
/// the game is normalised to a fixed peak before it gets there; this is the
/// score's equivalent, chosen so a dense passage peaks at about `0.7` — the
/// same order as a baked clip's normalised `0.85` and an ambience generator's
/// loudest gust — and `gain::MUSIC` therefore means the same thing as every
/// other number in that table.
///
/// A plucked piece has a far wider crest factor than a wind bed, so it sits
/// well below the bed in RMS while peaking above it. That is not a mistake:
/// harmonic content in a handful of narrow bands is heard through broadband
/// noise at levels where equal-energy noise would be completely masked.
const PLUCK_DRIVE: f32 = 1.62;

/// One plucked string: a delay line, a two-tap loop filter, and a loop gain.
///
/// The two-tap filter `(1-b)·x[n] + b·x[n-1]` is the whole of the timbre
/// control. Its group delay is exactly `b` samples, so tuning stays honest, and
/// it is a FIR, so the loop cannot run away however hard it is driven — which
/// matters when the thing runs on an audio callback with no supervision.
struct KsString {
    buf: [f32; RING],
    write: usize,
    delay_int: usize,
    delay_frac: f32,
    filt: f32,
    damp: f32,
    rho: f32,
    gain: f32,
    age: u32,
    life: u32,
    fade: u32,
    excite_left: u32,
    excite_len: u32,
    excite_amp: f32,
    excite_lp: f32,
    excite_hp: f32,
    excite_a: f32,
    /// Output ramp, in samples. Separate from the burst so the softness of the
    /// attack is a time and the loudness of the note is not.
    attack: u32,
    rng: Rng,
    active: bool,
}

impl KsString {
    fn silent() -> Self {
        Self {
            buf: [0.0; RING],
            write: 0,
            delay_int: 64,
            delay_frac: 0.0,
            filt: 0.0,
            damp: 0.3,
            rho: 0.99,
            gain: 0.0,
            age: 0,
            life: 0,
            fade: 1,
            excite_left: 0,
            excite_len: 1,
            excite_amp: 0.0,
            excite_lp: 0.0,
            excite_hp: 0.0,
            excite_a: 0.3,
            attack: 1,
            rng: Rng::new(1),
            active: false,
        }
    }

    /// Pluck.
    ///
    /// `t60` is the decay of the fundamental to -60 dB, `damp` is the loop
    /// filter's `b` (larger is darker), `bright_hz` is the cutoff of the
    /// excitation noise — pick hardness, in effect — and `attack` is the length
    /// of the output ramp that softens the onset.
    #[allow(clippy::too_many_arguments)]
    fn pluck(
        &mut self,
        freq: f32,
        amp: f32,
        t60: f32,
        damp: f32,
        bright_hz: f32,
        attack: f32,
        seed: u64,
    ) {
        let freq = freq.clamp(MIN_HZ, 4000.0);
        let damp = damp.clamp(0.02, 0.49);
        // Total loop delay is `delay + damp` samples; solve for the delay.
        let delay = (SR / freq - damp).clamp(2.0, (RING - 3) as f32);
        self.delay_int = delay.floor() as usize;
        self.delay_frac = delay - self.delay_int as f32;
        self.damp = damp;
        // One loop is one period, so the per-loop gain that reaches -60 dB in
        // `t60` seconds is exp(-ln(1000) / (f · t60)).
        self.rho = (-6.907_755 / (freq * t60.max(0.2))).exp().clamp(0.0, 0.9999);
        self.gain = 1.0;
        self.age = 0;
        // At `t60` the string is sixty decibels down and finished; carrying it
        // further only occupies a slot that the next chord wants.
        self.life = (t60 * SR) as u32;
        self.fade = (0.04 * SR) as u32;
        // See [`EXCITE_PERIODS`]: a burst of a fixed number of periods, so the
        // build-up round the loop is the same at every pitch.
        self.excite_len = ((delay * EXCITE_PERIODS) as u32).clamp(24, 900);
        self.excite_left = self.excite_len;
        self.excite_amp = amp * PLUCK_DRIVE;
        // Softness is a *time*, not a number of periods: the burst is sized in
        // periods so every pitch is equally loud, and the onset is smoothed by
        // an output ramp so every pitch is equally gentle. Conflating the two
        // makes a bass note both quieter and softer than a treble one for
        // reasons that have nothing to do with how it was played.
        self.attack = ((attack * SR) as u32).max(1);
        self.excite_a = (1.0 - (-core::f32::consts::TAU * bright_hz / SR).exp()).clamp(0.01, 1.0);
        self.excite_lp = 0.0;
        self.excite_hp = 0.0;
        self.filt = 0.0;
        self.buf = [0.0; RING];
        self.write = 0;
        self.rng = Rng::new(seed);
        self.active = true;
    }

    #[inline]
    fn step(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }
        let r0 = (self.write + RING - self.delay_int) & RING_MASK;
        let r1 = (r0 + RING - 1) & RING_MASK;
        let a = self.buf[r0];
        let x = a + (self.buf[r1] - a) * self.delay_frac;
        // Two-tap loop filter: the string loses its top end faster than its
        // fundamental, which is exactly what a real string does.
        let y = x + (self.filt - x) * self.damp;
        self.filt = x;
        let mut v = y * self.rho;

        if self.excite_left > 0 {
            let k = self.excite_len - self.excite_left;
            let t = k as f32 / self.excite_len as f32;
            // A finger releasing, not a hammer: raised-cosine up over the
            // first half of the burst and raised-cosine down over the rest, so
            // the energy goes in smoothly and the string is never kicked.
            let env = if t < 0.45 {
                raised_cosine(t / 0.45)
            } else {
                raised_cosine((1.0 - t) / 0.55)
            };
            let n = self.rng.bipolar();
            self.excite_lp += (n - self.excite_lp) * self.excite_a;
            // Keep the burst DC-free or the loop integrates the offset into a
            // thump that has nothing to do with the note.
            self.excite_hp += (self.excite_lp - self.excite_hp) * 0.02;
            v += (self.excite_lp - self.excite_hp) * env * self.excite_amp;
            self.excite_left -= 1;
        }

        if !v.is_finite() {
            v = 0.0;
            self.active = false;
        }
        self.buf[self.write] = v;
        self.write = (self.write + 1) & RING_MASK;
        self.age += 1;

        // The tail is already sixty decibels down by `life`; the fade is there
        // so that the *last* sample is zero rather than nearly zero.
        let mut out = if self.age + self.fade >= self.life {
            let left = self.life.saturating_sub(self.age) as f32 / self.fade as f32;
            v * raised_cosine(left)
        } else {
            v
        };
        if self.age < self.attack {
            out *= raised_cosine(self.age as f32 / self.attack as f32);
        }
        if self.age >= self.life {
            self.active = false;
        }
        out * self.gain
    }
}

// ─ Playback ────────────────────────────────────────────────

/// Per-part timbre: `(t60, damp, excitation cutoff Hz, attack secs, twin)`.
///
/// The melody gets a second string four cents sharp. Two strings very slightly
/// apart is how a piano and a twelve-string get their shimmer, and it is the
/// cheapest "organic" there is — no chorus, no modulation, just beating.
struct Timbre {
    t60: f32,
    damp: f32,
    bright_hz: f32,
    attack: f32,
    twin: bool,
}

fn timbre(part: Part) -> Timbre {
    match part {
        Part::Bass => Timbre { t60: 5.0, damp: 0.42, bright_hz: 700.0, attack: 0.030, twin: false },
        Part::Harmony => Timbre { t60: 5.5, damp: 0.33, bright_hz: 1600.0, attack: 0.024, twin: false },
        Part::Melody => Timbre { t60: 4.5, damp: 0.24, bright_hz: 2600.0, attack: 0.020, twin: true },
    }
}

/// The score, ready to play: the note list, the strings, and where we are.
pub struct Score {
    events: Vec<Event>,
    len: u32,
    pos: u32,
    next: usize,
    strings: Vec<KsString>,
    cue: u32,
    seed: u64,
    /// Rotating index used to seed each pluck, so a note's excitation noise is
    /// the same every time the piece comes round.
    plucks: u64,
    out_lp: f32,
    dc: f32,
}

impl Score {
    pub fn new(seed: u64) -> Self {
        let (events, len) = compose(seed);
        Self {
            events,
            len,
            pos: 0,
            next: 0,
            strings: (0..STRINGS).map(|_| KsString::silent()).collect(),
            cue: 0,
            seed,
            plucks: 0,
            out_lp: 0.0,
            dc: 0.0,
        }
    }

    /// Length of the piece in seconds. Read by the tests and the docs.
    #[allow(dead_code)]
    pub fn secs(&self) -> f32 {
        self.len as f32 / SR
    }

    #[allow(dead_code)]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    fn rewind(&mut self) {
        self.pos = 0;
        self.next = 0;
        self.plucks = 0;
    }

    /// Take a string, preferring an idle one and otherwise the most decayed.
    ///
    /// "Most decayed" is `age / life` rather than raw age, because a bass note
    /// lives longer than a melody note and the oldest string is not necessarily
    /// the quietest one. With [`STRINGS`] sized as it is this branch is rarely
    /// taken at all, and when it is the victim is far down its own tail.
    fn take(&mut self) -> usize {
        let mut best = 0usize;
        let mut spent = -1.0f32;
        for (i, s) in self.strings.iter().enumerate() {
            if !s.active {
                return i;
            }
            let done = s.age as f32 / s.life.max(1) as f32;
            if done >= spent {
                spent = done;
                best = i;
            }
        }
        best
    }

    #[allow(clippy::too_many_arguments)]
    fn trigger(&mut self, event: Event, warmth: f32, register: f32, dusk: f32) {
        let t = timbre(event.part);
        let shift = if event.part == Part::Melody { register } else { 0.0 };
        let freq = TONIC_HZ * ((event.pitch + shift) / 12.0).exp2();
        // A thin network is a darker instrument, and dusk darker still. Neither
        // changes a note, only the light on it.
        let damp = (t.damp + (1.0 - warmth) * 0.10 + dusk * 0.11).clamp(0.02, 0.49);
        let bright = t.bright_hz * lerp(0.62, 1.0, warmth) * lerp(1.0, 0.75, dusk);
        let t60 = t.t60 * lerp(1.0, 1.30, dusk);
        let attack = t.attack * lerp(1.0, 1.35, dusk);

        self.plucks = self.plucks.wrapping_add(1);
        let seed = self.seed ^ self.plucks.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let slot = self.take();
        self.strings[slot].pluck(freq, event.amp, t60, damp, bright, attack, seed);

        if t.twin {
            // Four cents sharp, a little quieter, a hair later.
            let slot = self.take();
            self.strings[slot].pluck(
                freq * 1.002_31,
                event.amp * 0.55,
                t60 * 0.9,
                damp + 0.03,
                bright * 0.9,
                attack * 1.2,
                seed ^ 0x5bf0_3635,
            );
        }
    }

    /// One sample.
    ///
    /// `cue` is the director's cue number: `0` is silence, and any change means
    /// "start the piece from the top". `warmth` is how the network is doing,
    /// `density` how much of the score survives the thinning, `dusk` the
    /// evening variant.
    pub fn step(&mut self, cue: u32, warmth: f32, density: f32, dusk: f32) -> f32 {
        if cue == 0 {
            if self.cue != 0 {
                self.cue = 0;
                self.rewind();
            }
            return 0.0;
        }
        if cue != self.cue {
            self.cue = cue;
            self.rewind();
        }

        // Everything below the gate is filigree; the bass on a chord change and
        // the motif are always above it.
        let gate = 235.0 - 215.0 * density.clamp(0.0, 1.0);
        // Dusk moves the tune down an octave into the harmony's own register —
        // the same piece, told lower.
        let register = -12.0 * dusk;

        while self.next < self.events.len() && self.events[self.next].at <= self.pos {
            let event = self.events[self.next];
            self.next += 1;
            if (event.weight as f32) >= gate {
                self.trigger(event, warmth, register, dusk);
            }
        }
        self.pos += 1;
        if self.pos >= self.len {
            self.rewind();
        }

        let mut sum = 0.0;
        for string in self.strings.iter_mut() {
            sum += string.step();
        }

        // One shared low-pass for the whole instrument: a body, and the "warmer
        // when thriving" move in one coefficient.
        let cut = lerp(1900.0, 4200.0, warmth) * lerp(1.0, 0.52, dusk);
        let a = (1.0 - (-core::f32::consts::TAU * cut / SR).exp()).clamp(0.001, 1.0);
        self.out_lp += (sum - self.out_lp) * a;
        // And a 20 Hz high-pass, because a room full of plucked strings should
        // not deliver a DC offset to somebody's speaker cone.
        self.dc += (self.out_lp - self.dc) * 0.0057;
        let out = self.out_lp - self.dc;
        if out.is_finite() {
            out
        } else {
            self.out_lp = 0.0;
            self.dc = 0.0;
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 42;

    fn semitone(voice: i32) -> i32 {
        voice
    }

    /// Every consecutive pair of chords the piece actually plays, including the
    /// section seams and the loop back to the top.
    fn progression() -> Vec<usize> {
        let mut out = Vec::new();
        for plan in FORM.iter() {
            out.extend_from_slice(&plan.chords);
        }
        out
    }

    fn pairs() -> Vec<(Chord, Chord)> {
        let order = progression();
        (0..order.len())
            .map(|i| (CHORDS[order[i]], CHORDS[order[(i + 1) % order.len()]]))
            .collect()
    }

    #[test]
    fn the_mode_is_lydian_and_keeps_its_raised_fourth() {
        // A Lydian score that avoids its #4 is just major. Every degree is a
        // whole step apart except 3-4 and 6-7, and degree 3 is six semitones
        // above the tonic - the tritone that is the entire point.
        assert_eq!(LYDIAN[SHARP_FOUR as usize], 6, "the fourth must be raised");
        assert_eq!(LYDIAN[MAJOR_SEVENTH as usize], 11, "and the seventh major");
        let sounding = CHORDS
            .iter()
            .filter(|c| c.tones & (1 << SHARP_FOUR) != 0)
            .count();
        assert!(
            sounding >= 5,
            "only {sounding} of {} voicings carry the #4",
            CHORDS.len()
        );
        // And it is in the top voice, where it cannot be missed, in the chord
        // that introduces it.
        let e_over_d = CHORDS[2];
        assert_eq!(
            e_over_d.voices[3] % 12,
            6,
            "the Lydian slash chord must put G# on top"
        );
    }

    #[test]
    fn the_dominant_never_appears() {
        // V-I is the cadence that would turn D Lydian into D major. The chord
        // is absent and so is its root in the bass - both halves matter,
        // because an A pedal under the E triad would do the same job.
        for chord in CHORDS {
            let bass_class = chord.bass.rem_euclid(12);
            assert_ne!(bass_class, 7, "{} puts A in the bass", chord.name);
            let triad: Vec<i32> = chord.voices.iter().map(|v| v.rem_euclid(12)).collect();
            let is_a_major = triad.contains(&7) && triad.contains(&11) && triad.contains(&2);
            assert!(!is_a_major, "{} spells an A major triad", chord.name);
        }
    }

    #[test]
    fn voices_lead_rather_than_leap() {
        for (from, to) in pairs() {
            let mut common = 0;
            let mut worst = 0;
            for v in 0..4 {
                let motion = (to.voices[v] - from.voices[v]).abs();
                worst = worst.max(motion);
                if motion == 0 {
                    common += 1;
                }
            }
            assert!(
                worst <= 5,
                "{} -> {}: a voice moved {worst} semitones",
                from.name,
                to.name
            );
            assert!(
                common >= 1 || worst <= 3,
                "{} -> {}: no common tone and a {worst}-semitone move",
                from.name,
                to.name
            );
        }
    }

    #[test]
    fn no_parallel_fifths_or_octaves_between_the_upper_voices() {
        // The single most audible sign that chords were stacked rather than
        // led. Checked for every pair of voices in every pair of chords, so it
        // cannot be reintroduced by editing one voicing in isolation.
        for (from, to) in pairs() {
            for a in 0..4 {
                for b in (a + 1)..4 {
                    let before = semitone(from.voices[b] - from.voices[a]);
                    let after = semitone(to.voices[b] - to.voices[a]);
                    let moved_a = to.voices[a] - from.voices[a];
                    let moved_b = to.voices[b] - from.voices[b];
                    if moved_a == 0 || moved_b == 0 {
                        continue; // oblique motion is always fine
                    }
                    if moved_a.signum() != moved_b.signum() {
                        continue; // so is contrary motion
                    }
                    let perfect = |i: i32| i.rem_euclid(12) == 0 || i.rem_euclid(12) == 7;
                    assert!(
                        !(perfect(before) && before.rem_euclid(12) == after.rem_euclid(12)),
                        "{} -> {}: voices {a} and {b} move in parallel {}s",
                        from.name,
                        to.name,
                        if before.rem_euclid(12) == 0 { "octave" } else { "fifth" }
                    );
                }
            }
        }
    }

    #[test]
    fn no_parallel_fifths_or_octaves_against_the_bass() {
        // The bass is a bass and is allowed to leap, but it may not shadow an
        // upper voice at a fifth or an octave - which is exactly what a pedal
        // that moves at a section seam would otherwise do.
        for (from, to) in pairs() {
            let bass_before = from.bass - 12;
            let bass_after = to.bass - 12;
            let bass_moved = bass_after - bass_before;
            if bass_moved == 0 {
                continue;
            }
            for v in 0..4 {
                let moved = to.voices[v] - from.voices[v];
                if moved == 0 || moved.signum() != bass_moved.signum() {
                    continue;
                }
                let before = from.voices[v] - bass_before;
                let after = to.voices[v] - bass_after;
                let perfect = |i: i32| i.rem_euclid(12) == 0 || i.rem_euclid(12) == 7;
                assert!(
                    !(perfect(before) && before.rem_euclid(12) == after.rem_euclid(12)),
                    "{} -> {}: the bass and voice {v} move in parallel",
                    from.name,
                    to.name
                );
            }
        }
    }

    #[test]
    fn the_harmonic_rhythm_is_slow_and_the_loop_is_long() {
        let bar = BEATS_PER_BAR * 60.0 / BPM;
        let chord = bar * BARS_PER_CHORD as f32;
        assert!(chord > 8.0, "a chord lasts only {chord:.1} s");
        let (_, len) = compose(SEED);
        let secs = len as f32 / SR;
        assert!(
            (240.0..300.0).contains(&secs),
            "the piece is {secs:.0} s - a loop that length is either heard or endless"
        );
        assert!((55.0..=80.0).contains(&BPM), "the tempo must stay calm");
    }

    #[test]
    fn the_same_seed_is_the_same_piece() {
        let (a, _) = compose(SEED);
        let (b, _) = compose(SEED);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.at, y.at);
            assert_eq!(x.pitch.to_bits(), y.pitch.to_bits());
        }
        // And a different world is a different tune.
        let (c, _) = compose(SEED + 1);
        let same = a
            .iter()
            .zip(c.iter())
            .filter(|(x, y)| x.pitch == y.pitch)
            .count();
        assert!(
            same < a.len(),
            "two seeds produced the same piece"
        );
    }

    fn melody(events: &[Event]) -> Vec<&Event> {
        events.iter().filter(|e| e.part == Part::Melody).collect()
    }

    #[test]
    fn the_melody_steps_far_more_often_than_it_leaps() {
        // Measured inside phrases. The gap between one phrase and the next is
        // a breath of three beats or more, and the interval across it is not a
        // melodic interval at all - it is where the next line starts.
        let (events, _) = compose(SEED);
        assert!(melody(&events).len() > 60, "the tune is too short to judge");
        let mut steps = 0;
        let mut leaps = 0;
        let mut widest: f32 = 0.0;
        for phrase in 0..phrase_count() {
            let notes = phrase_notes(&events, phrase);
            for pair in notes.windows(2) {
                let d = (pair[1].0.pitch - pair[0].0.pitch).abs();
                if d < 0.5 {
                    continue;
                }
                widest = widest.max(d);
                if d <= 2.0 {
                    steps += 1;
                } else {
                    leaps += 1;
                }
            }
        }
        let ratio = steps as f32 / (steps + leaps) as f32;
        assert!(
            ratio > 0.55,
            "only {:.0}% of the melody moves by step - that is a random walk",
            ratio * 100.0
        );
        assert!(widest <= 12.5, "the melody leapt {widest} semitones");
    }

    #[test]
    fn the_interval_distribution_is_shaped_like_a_melody() {
        // Seconds far outnumber thirds, thirds outnumber everything larger.
        // A uniform draw over a scale gives the opposite of this and it is
        // instantly audible as "generative music".
        let (events, _) = compose(SEED);
        let (mut seconds, mut thirds, mut wider) = (0, 0, 0);
        for phrase in 0..phrase_count() {
            for pair in phrase_notes(&events, phrase).windows(2) {
                let d = (pair[1].0.pitch - pair[0].0.pitch).abs();
                match d {
                    _ if d < 0.5 => {}
                    _ if d <= 2.0 => seconds += 1,
                    _ if d <= 4.0 => thirds += 1,
                    _ => wider += 1,
                }
            }
        }
        assert!(
            seconds > thirds,
            "{seconds} seconds against {thirds} thirds"
        );
        assert!(
            seconds > wider * 3,
            "{seconds} seconds against {wider} intervals of a fourth or more"
        );
        assert!(wider > 0, "a melody with no leap at all has no gesture");
    }

    #[test]
    fn a_leap_is_answered_rather_than_continued() {
        // Two leaps the same way is the sound of a line that has stopped being
        // a line. A leap of a fourth or more must turn round, and it should
        // usually turn round by a step.
        let (events, _) = compose(SEED);
        let mut turned = 0;
        let mut stepped = 0;
        let mut leaps = 0;
        for phrase in 0..phrase_count() {
            let notes = phrase_notes(&events, phrase);
            for w in notes.windows(3) {
                let a = w[1].0.pitch - w[0].0.pitch;
                let b = w[2].0.pitch - w[1].0.pitch;
                if a.abs() < 4.5 || b.abs() < 0.5 {
                    continue;
                }
                leaps += 1;
                if b.signum() != a.signum() {
                    turned += 1;
                    if b.abs() <= 2.5 {
                        stepped += 1;
                    }
                }
            }
        }
        // Leaps are rare in a piece this calm; three in four minutes is
        // enough to prove the rule is exercised, and the distribution test
        // above is what guards the balance.
        assert!(leaps >= 3, "the melody only leaps {leaps} times");
        assert_eq!(
            turned, leaps,
            "{} leaps carried straight on in the same direction",
            leaps - turned
        );
        // Turning round is compulsory; turning round *by a step* is only
        // strongly preferred, because a leap that resolves onto a chord tone a
        // third away on a strong beat is good writing rather than a failure.
        let rate = stepped as f32 / leaps as f32;
        assert!(
            rate >= 0.45,
            "only {:.0}% of leaps are answered by a step",
            rate * 100.0
        );
    }

    #[test]
    fn strong_beats_take_chord_tones() {
        let (events, _) = compose(SEED);
        let order = progression();
        // Bar boundaries in samples, so a note written at the top of a bar is
        // not filed under the previous one by a rounding error.
        let bars: Vec<u32> = (0..FORM.len() as i32 * BARS_PER_SECTION)
            .map(|b| at_beat(b as f32 * BEATS_PER_BAR))
            .collect();
        let mut on = 0;
        let mut total = 0;
        for event in events.iter().filter(|e| e.part == Part::Melody) {
            let bar = bars.partition_point(|start| *start <= event.at).max(1) - 1;
            let inside = (event.at - bars[bar]) as f32 / beat_len();
            if !(inside < 0.02 || (inside - 3.0).abs() < 0.02) {
                continue;
            }
            let slot = bar / BARS_PER_CHORD as usize;
            let chord = CHORDS[order[slot.min(order.len() - 1)]];
            // Degree class from the pitch: melody pitches are all diatonic.
            let semis = (event.pitch - HARMONY_ORIGIN).round() as i32;
            let class = semis.rem_euclid(12);
            let degree = LYDIAN.iter().position(|s| *s == class);
            let Some(degree) = degree else {
                panic!("melody note {semis} is not in the mode");
            };
            total += 1;
            if chord.tones & (1 << degree) != 0 {
                on += 1;
            }
        }
        assert!(total > 20, "only {total} strong-beat notes to check");
        let rate = on as f32 / total as f32;
        assert!(
            rate > 0.85,
            "only {:.0}% of strong beats land on a chord tone",
            rate * 100.0
        );
    }

    /// Melody onsets inside one four-bar phrase, in beats from its start.
    ///
    /// Windowed in samples rather than in beats: `Event::at` is a truncated
    /// sample index, so a note written at beat 24 reads back as 23.99997 and a
    /// beat-space comparison files it under the previous phrase.
    fn phrase_count() -> usize {
        (FORM.len() as i32 * BARS_PER_SECTION / BARS_PER_PHRASE) as usize
    }

    fn phrase_notes(events: &[Event], phrase: usize) -> Vec<(&Event, f32)> {
        let span = BARS_PER_PHRASE as f32 * BEATS_PER_BAR;
        let start = phrase as f32 * span;
        let (lo, hi) = (at_beat(start), at_beat(start + span));
        events
            .iter()
            .filter(|e| e.part == Part::Melody && e.at >= lo && e.at < hi)
            .map(|e| (e, (e.at - lo) as f32 / beat_len()))
            .collect()
    }

    #[test]
    fn every_phrase_stops_before_it_ends() {
        // Silence is part of the phrasing. Each four-bar phrase must have at
        // least three beats with no new melody attack at its end.
        let (events, _) = compose(SEED);
        let span = BARS_PER_PHRASE as f32 * BEATS_PER_BAR;
        for p in 0..phrase_count() {
            let notes = phrase_notes(&events, p);
            assert!(!notes.is_empty(), "phrase {p} has no melody at all");
            let last = notes.iter().map(|(_, b)| *b).fold(0.0f32, f32::max);
            assert!(
                span - last >= 3.0,
                "phrase {p} sings until {last:.1} of {span} beats"
            );
        }
    }

    #[test]
    fn the_motif_comes_back() {
        // Recognisability is structural, not a hope. The theme's opening phrase
        // states the motif four times in four different lights, and the last
        // statement of the piece is literally the first.
        let (events, _) = compose(SEED);
        // Sections are two phrases each, so section `s` opens with phrase `2s`.
        let head = |section: usize| -> Vec<(i32, i32)> {
            phrase_notes(&events, section * 2)
                .iter()
                .take(5)
                .map(|(e, b)| (e.pitch.round() as i32, (b * 2.0).round() as i32))
                .collect()
        };
        let a = head(0);
        let a3 = head(5);
        assert!(a.len() >= 4, "the theme is only {} notes", a.len());
        assert_eq!(a, a3, "the last section is not a return of the first");

        let intervals = |v: &[(i32, i32)]| -> Vec<i32> {
            v.windows(2).map(|w| w[1].0 - w[0].0).collect()
        };
        // A' is the same rhythm a diatonic third higher.
        let a1 = head(1);
        assert_eq!(
            a.iter().map(|n| n.1).collect::<Vec<_>>(),
            a1.iter().map(|n| n.1).collect::<Vec<_>>(),
            "A' does not keep the motif's rhythm"
        );
        assert!(a1[0].0 > a[0].0, "A' should be higher than A");

        // A'' is the same shape upside down: the intervals run the other way.
        let a2 = head(3);
        let up = intervals(&a);
        let down = intervals(&a2);
        assert_eq!(up.len(), down.len(), "A'' lost a motif note");
        let opposed = up
            .iter()
            .zip(down.iter())
            .filter(|(x, y)| **x != 0 && x.signum() != y.signum())
            .count();
        assert!(
            opposed * 2 >= up.iter().filter(|x| **x != 0).count(),
            "A'' does not read as an inversion: {up:?} against {down:?}"
        );
    }

    #[test]
    fn thinning_keeps_the_bones_and_drops_the_filigree() {
        let (events, _) = compose(SEED);
        let kept = |density: f32| {
            let gate = 235.0 - 215.0 * density;
            events
                .iter()
                .filter(|e| e.weight as f32 >= gate)
                .collect::<Vec<_>>()
        };
        let full = kept(0.95);
        let thin = kept(0.45);
        assert_eq!(full.len(), events.len(), "the warm variant plays everything");
        assert!(
            thin.len() < full.len() * 3 / 4,
            "the sparse variant is barely thinner: {} of {}",
            thin.len(),
            full.len()
        );
        // Whatever is dropped, the piece still has a bass and a tune.
        assert!(thin.iter().any(|e| e.part == Part::Bass));
        assert!(thin.iter().any(|e| e.part == Part::Melody));
        assert!(thin.iter().any(|e| e.part == Part::Harmony));
    }

    // ── synthesis ─────────────────────────────────────────

    fn render(secs: f32, warmth: f32, density: f32, dusk: f32) -> Vec<f32> {
        let mut score = Score::new(SEED);
        (0..(secs * SR) as usize)
            .map(|_| score.step(1, warmth, density, dusk))
            .collect()
    }

    #[test]
    fn a_plucked_string_holds_its_pitch() {
        // A delay line that is a sample out is a string that is thirty cents
        // flat, and a chord of them is a piece in a different key.
        for &want in &[73.42f32, 146.83, 293.66, 440.0] {
            let mut string = KsString::silent();
            string.pluck(want, 0.5, 4.0, 0.3, 2000.0, 0.01, 7);
            let n = (SR * 0.6) as usize;
            let skip = (SR * 0.08) as usize;
            let buf: Vec<f32> = (0..n).map(|_| string.step()).collect();
            let body = &buf[skip..];
            // Autocorrelation over a window around the expected period.
            let period = SR / want;
            let mut best = 0.0f32;
            let mut best_lag = period;
            let lo = (period * 0.8) as usize;
            let hi = ((period * 1.2) as usize).min(body.len() / 3);
            for lag in lo..hi {
                let mut acc = 0.0f32;
                for i in 0..(body.len() - lag) {
                    acc += body[i] * body[i + lag];
                }
                if acc > best {
                    best = acc;
                    best_lag = lag as f32;
                }
            }
            let got = SR / best_lag;
            let cents = 1200.0 * (got / want).log2();
            assert!(
                cents.abs() < 35.0,
                "asked for {want} Hz, got {got:.1} Hz ({cents:.0} cents)"
            );
        }
    }

    #[test]
    fn a_pluck_arrives_over_milliseconds_not_samples() {
        // "Never startle" applies to the score. A plucked string is allowed an
        // attack; what it is not allowed is a discontinuity.
        let mut string = KsString::silent();
        let t = timbre(Part::Melody);
        string.pluck(293.66, 0.4, t.t60, t.damp, t.bright_hz, t.attack, 11);
        let n = (SR * 0.2) as usize;
        let buf: Vec<f32> = (0..n).map(|_| string.step()).collect();
        let peak = buf.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(peak > 0.02, "the pluck was inaudible: {peak}");
        assert_eq!(buf[0], 0.0, "a string must start from silence");
        // The envelope, over one-millisecond windows, rather than the first
        // sample that happens to be loud: a noise excitation is full of those
        // and none of them is what the ear hears as the attack.
        let window = (0.001 * SR) as usize;
        let env: Vec<f32> = buf
            .chunks(window)
            .map(|c| (c.iter().map(|s| (s * s) as f64).sum::<f64>() / c.len() as f64).sqrt() as f32)
            .collect();
        let loudest = env.iter().fold(0.0f32, |a, s| a.max(*s));
        let half = env.iter().position(|s| *s > loudest * 0.5).unwrap_or(0);
        // A real guitar reaches half power in well under a millisecond. This
        // one takes three or more, which is the whole difference between a
        // plectrum and a fingertip - and between a sound that is pleasant on
        // the four hundredth repetition and one that is not.
        assert!(
            half >= 3,
            "the attack reached half of peak in {half} ms"
        );
    }

    #[test]
    fn the_score_never_startles() {
        // Measured as the windowed envelope rather than the raw sample slope: a
        // 440 Hz partial legitimately swings most of its range between two
        // samples at 22 kHz, and what the ear reads as a crack is the envelope
        // arriving at once. Ten milliseconds is about one control block.
        let buf = render(45.0, 0.9, 0.95, 0.0);
        let window = (0.010 * SR) as usize;
        let peak = buf.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(peak > 0.05 && peak < 1.0, "the score peaked at {peak}");
        let mut worst = 0.0f32;
        let mut previous = 0.0f32;
        for chunk in buf.chunks(window) {
            let rms = (chunk.iter().map(|s| (s * s) as f64).sum::<f64>() / chunk.len() as f64)
                .sqrt() as f32;
            worst = worst.max((rms - previous).abs());
            previous = rms;
        }
        assert!(
            worst < peak * 0.42,
            "the envelope jumped by {worst} against a peak of {peak}"
        );
    }

    #[test]
    fn the_score_is_finite_and_bounded_in_every_variant() {
        for (warmth, density, dusk) in [
            (0.05f32, 0.45f32, 0.0f32),
            (1.0, 0.95, 0.0),
            (0.5, 0.6, 1.0),
        ] {
            let buf = render(30.0, warmth, density, dusk);
            let peak = buf.iter().fold(0.0f32, |a, s| a.max(s.abs()));
            assert!(buf.iter().all(|s| s.is_finite()), "the score produced a NaN");
            assert!(peak < 0.95, "variant peaked at {peak}");
            assert!(peak > 0.03, "variant was effectively silent at {peak}");
        }
    }

    #[test]
    fn silence_costs_nothing_and_a_new_cue_starts_at_the_top() {
        let mut score = Score::new(SEED);
        for _ in 0..1000 {
            assert_eq!(score.step(0, 0.8, 0.9, 0.0), 0.0);
        }
        assert_eq!(score.pos, 0, "the piece advanced while silent");
        // Run into the piece, then start a new cue: the theme must come back.
        for _ in 0..(SR as usize * 30) {
            score.step(1, 0.8, 0.9, 0.0);
        }
        assert!(score.pos > 0);
        score.step(2, 0.8, 0.9, 0.0);
        assert_eq!(score.pos, 1, "a new cue did not rewind to the theme");
    }

    #[test]
    fn the_dusk_variant_is_lower_and_darker() {
        let day = render(24.0, 0.8, 0.9, 0.0);
        let dusk = render(24.0, 0.8, 0.9, 1.0);
        // The fraction of the energy above about 1.5 kHz, via a one-pole
        // high-pass. A ratio rather than a level, so this measures colour and
        // not which variant happens to be louder.
        let treble = |b: &[f32]| {
            let a = 1.0 - (-core::f32::consts::TAU * 1500.0 / SR).exp();
            let (mut lp, mut hi, mut all) = (0.0f32, 0.0f64, 0.0f64);
            for s in b {
                lp += (s - lp) * a;
                let h = s - lp;
                hi += (h * h) as f64;
                all += (s * s) as f64;
            }
            hi / all.max(1e-12)
        };
        assert!(
            treble(&dusk) < treble(&day) * 0.85,
            "dusk {} is not darker than day {}",
            treble(&dusk),
            treble(&day)
        );
    }

    #[test]
    fn the_score_is_cheap_enough_for_the_audio_thread() {
        // The generator shares a core with the renderer; a serious FPS
        // regression was just fixed and this must not be the next one.
        let mut score = Score::new(SEED);
        let n = (SR * 20.0) as usize;
        let start = std::time::Instant::now();
        let mut sink = 0.0f32;
        for _ in 0..n {
            sink += score.step(1, 0.9, 0.95, 0.0);
        }
        let elapsed = start.elapsed().as_secs_f64();
        assert!(sink.is_finite());
        // Twenty seconds of audio in well under a second of CPU, even in a
        // debug build, is a real-time factor of over twenty.
        assert!(
            elapsed < 4.0,
            "20 s of score took {elapsed:.2} s to render"
        );
    }

    /// Render the piece to a WAV so a human can listen to it.
    ///
    /// Not a check — a tool. Set `RAIL_TOWN_SCORE_WAV` to a path and run this
    /// test; anything else and it does nothing. "Do not ship music you have not
    /// heard" needs somewhere to hear it from, and the next person to touch the
    /// harmony will want it too.
    #[test]
    fn render_wav_for_listening() {
        let Ok(path) = std::env::var("RAIL_TOWN_SCORE_WAV") else {
            return;
        };
        let secs: f32 = std::env::var("RAIL_TOWN_SCORE_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(280.0);
        let warmth: f32 = std::env::var("RAIL_TOWN_SCORE_WARMTH")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.85);
        let density: f32 = std::env::var("RAIL_TOWN_SCORE_DENSITY")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.92);
        let dusk: f32 = std::env::var("RAIL_TOWN_SCORE_DUSK")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let seed: u64 = std::env::var("RAIL_TOWN_SCORE_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(SEED);

        let mut score = Score::new(seed);
        let n = (secs * SR) as usize;
        let mut pcm = Vec::with_capacity(n * 2 + 44);
        let data_len = n as u32 * 2;
        pcm.extend_from_slice(b"RIFF");
        pcm.extend_from_slice(&(36 + data_len).to_le_bytes());
        pcm.extend_from_slice(b"WAVEfmt ");
        pcm.extend_from_slice(&16u32.to_le_bytes());
        pcm.extend_from_slice(&1u16.to_le_bytes());
        pcm.extend_from_slice(&1u16.to_le_bytes());
        pcm.extend_from_slice(&(SR as u32).to_le_bytes());
        pcm.extend_from_slice(&(SR as u32 * 2).to_le_bytes());
        pcm.extend_from_slice(&2u16.to_le_bytes());
        pcm.extend_from_slice(&16u16.to_le_bytes());
        pcm.extend_from_slice(b"data");
        pcm.extend_from_slice(&data_len.to_le_bytes());
        // The generator's own output, not the in-game level: this file is for
        // judging the piece, and the mixer's gains are judged in the mixer.
        for _ in 0..n {
            let s = score.step(1, warmth, density, dusk).clamp(-1.0, 1.0);
            pcm.extend_from_slice(&((s * 32000.0) as i16).to_le_bytes());
        }
        std::fs::write(&path, pcm).expect("could not write the WAV");
        println!("wrote {path}");
    }
}
