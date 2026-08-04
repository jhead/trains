//! The piece itself (brief S4, and `docs/design/14-music.md`).
//!
//! A generative score is easy to write badly: pick a scale, pick a random note
//! in it, repeat. It is instantly recognisable as such, because real melody is
//! not a sequence of legal pitches -- it is a *motif* that comes back, harmony
//! that goes somewhere, and a register the ear reads as light rather than as
//! weather. Everything in this file exists to make those three things true.
//!
//! # The piece in one paragraph
//!
//! A cue is a small **piece**: four minutes, 4/4 at 88 BPM, in one of four major
//! keys, built as intro - A - B - A' - C - A'' - outro. The intro is a pad
//! establishing the key; A states a three-to-six-note **motif** and answers it;
//! B takes a second motif through a different progression; A' and A'' bring the
//! first motif back transposed, octave-displaced and rhythmically augmented; C
//! breathes, with long notes and a borrowed `bVII` or `iv` for warmth; the outro
//! thins back to the pad and cadences. Four such pieces are composed per world
//! and the cue counter rotates through them, so **no two consecutive cues share
//! a key, a motif or a progression**.
//!
//! # The three claims this file has to earn
//!
//! 1. **Harmony that moves.** [`PALETTE`] is diatonic chord *specifications* and
//!    [`voice_chord`] realises each one as a four-part voicing chosen for least
//!    motion, with parallel fifths and octaves -- against each other and against
//!    the bass -- filtered out rather than hoped against. Every section ends on
//!    a cadence: a tonic preceded by a dominant or a subdominant. There is no
//!    banned dominant, only a preference for `Vsus4` over a bare `V`, because
//!    tension that never resolves is exactly as monotonous as tension that never
//!    arrives.
//! 2. **A motif the ear recognises.** A motif -- three to six [`MotifNote`]s
//!    over a bar or two -- is generated once per piece and then *restated* a
//!    dozen times under [`Variation`]: transposed within the
//!    scale, displaced an octave, augmented by half or doubled, inverted. The
//!    interval signature survives every one of those, which is what makes a
//!    return audible, and the free tail after each statement is regenerated,
//!    which is what keeps a return from being a copy.
//! 3. **Bright, not dark.** The melody lives in C4-A5 on a bell/keys voice with
//!    a soft attack and a long decay, the pad sits under it in C3-C5, and the
//!    bass stays in one octave from C2. Dusk softens and darkens the *reading* --
//!    slower attacks, a lower filter, fewer partials -- and never moves the tune
//!    down into the mud.
//!
//! # Sound
//!
//! Four voices, none of them the same kind of thing:
//!
//! - [`Bell`] -- the melody. Five sine partials from a magic-circle oscillator
//!   (two multiplies and no trigonometry per sample), each with its own decay,
//!   so the top of the tone falls away first the way a struck string does. One
//!   of the five is the fundamental six cents sharp, which is where the shimmer
//!   comes from.
//! - [`PadVoice`] -- the harmony. Two polyBLEP saws a few cents apart through
//!   two poles of low-pass, with a slow attack and a slower release. It is held
//!   rather than struck, and a voice that does not move between two chords is
//!   never retriggered -- the common tone simply stays down.
//! - [`BassVoice`] -- a sine with a touch of its own second partial.
//! - [`KsString`] -- Karplus-Strong, kept from the previous score and demoted to
//!   the arpeggio: a real plucked string is the one thing the other three cannot
//!   imitate, and off-beat plucks are the whole of the rhythmic floor.
//!
//! A single feedback delay of one eighth note sits across the lot at a low mix.
//! It is not a reverb and does not pretend to be; it is the cheapest way to make
//! a small number of notes sound like they are in a place.

use super::clip::SR;
use super::dsp::{lerp, pole_coeff, raised_cosine, Rng};

// -- Theory ---------------------------------------------------

/// The major scale, as semitone offsets from the tonic.
///
/// Ionian rather than a mode: C418's vocabulary is plain major with add9, add6
/// and maj7 colour on top, and colour is what the chord table is for. The one
/// non-diatonic sound in the piece arrives as a *borrowed* chord (`bVII`, `iv`),
/// which is warmth rather than a change of scale.
pub const MAJOR: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];

/// C2, in Hz. Every pitch in the piece is semitones above it.
const C2_HZ: f32 = 65.406_4;

const C3: i32 = 12;
const C4: i32 = 24;
const C5: i32 = 36;
/// A5. The top of the melody: bright, and still two octaves below anything
/// that would read as a whistle.
const A5: i32 = 45;

/// The bass lives in one octave from C2 and never leaves it. A bass that
/// wanders is a bass you notice.
const BASS_LO: i32 = 0;
const BASS_HI: i32 = C3;
/// The pad sits between the bass and the tune, where it can be heard without
/// being listened to.
const PAD_LO: i32 = C3;
const PAD_HI: i32 = C5;
/// The melody. **This range is the single biggest change from the old score**,
/// which sang around D3 and read as gloom no matter what it played.
const MELODY_LO: i32 = C4;
const MELODY_HI: i32 = A5;

/// Nothing is ever plucked below this: the Karplus-Strong delay line is
/// [`RING`] long and a lower string would not fit in it.
const MIN_HZ: f32 = 46.0;

/// 88 BPM. Fast enough to have a pulse, slow enough that the pulse is a walk.
pub const BPM: f32 = 88.0;

/// 4/4. The old score's 6/4 was chosen so that no downbeat would ever land, and
/// it worked: nothing landed at all.
pub const BEATS_PER_BAR: f32 = 4.0;

/// Four bars to a phrase, four phrases to a section.
const BARS_PER_PHRASE: i32 = 4;
const PHRASES_PER_SECTION: i32 = 4;

/// One of the four tonal centres. A piece picks one and stays there; the *world*
/// gets four pieces and therefore four keys.
#[derive(Clone, Copy, Debug)]
pub struct Key {
    /// Read by the WAV tool and by the tests, which name the key they failed
    /// on: a bare index makes a failure unreadable.
    #[allow(dead_code)]
    pub name: &'static str,
    /// Semitones above C2.
    pub root: i32,
}

/// D, G, A, C. All within one octave of C2, so the bass register is the same
/// whichever is drawn.
pub const KEYS: [Key; 4] = [
    Key { name: "D major", root: 2 },
    Key { name: "G major", root: 7 },
    Key { name: "A major", root: 9 },
    Key { name: "C major", root: 0 },
];

/// What a chord is *for*. Cadences are checked against this and nothing else,
/// which is why it is data rather than a comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Tonic,
    Subdominant,
    Dominant,
    /// Colour: the mediant and the two borrowed chords. Neither prepares nor
    /// resolves, so neither may end a section.
    Colour,
}

/// One chord, as a specification rather than as a voicing.
///
/// The old score shipped ten hand-written voicings, which guaranteed good part
/// writing and exactly one progression forever. Here the *tones* are declared
/// and [`voice_chord`] does the part writing, so a piece can use any progression
/// it likes and still be led rather than stacked.
#[derive(Clone, Copy, Debug)]
pub struct ChordSpec {
    /// The chord symbol. Read by the voice-leading tests, which name the pair
    /// they failed on.
    #[allow(dead_code)]
    pub name: &'static str,
    /// Root, as a scale degree of the key.
    pub root: i32,
    /// Chord tones as scale steps above the root: `0` root, `2` third, `4`
    /// fifth, `6` seventh, `1` ninth, `3` eleventh.
    pub tones: [i32; 4],
    /// A scale degree this chord bends and the semitone class it takes;
    /// `(-1, 0)` for the diatonic majority. Modal mixture in one field.
    pub alter: (i32, i32),
    pub role: Role,
}

const I_ADD9: usize = 0;
const I_MAJ7: usize = 1;
const I_SIX: usize = 2;
const II_7: usize = 3;
const III_7: usize = 4;
const IV_MAJ7: usize = 5;
const IV_ADD9: usize = 6;
const V_SUS: usize = 7;
const V_ADD9: usize = 8;
const VI_7: usize = 9;
const VI_9: usize = 10;
const FLAT_VII: usize = 11;
const IV_MINOR: usize = 12;

/// The chord vocabulary. Degrees are `0 I, 1 ii, 2 iii, 3 IV, 4 V, 5 vi, 6 vii`.
///
/// Every tonic and subdominant carries a ninth, a sixth or a major seventh --
/// that added second above the third is the whole of the shimmer this score is
/// after, and a bare triad is the one voicing that never appears.
pub const PALETTE: [ChordSpec; 13] = [
    ChordSpec { name: "Iadd9", root: 0, tones: [0, 2, 4, 1], alter: (-1, 0), role: Role::Tonic },
    ChordSpec { name: "Imaj7", root: 0, tones: [0, 2, 4, 6], alter: (-1, 0), role: Role::Tonic },
    ChordSpec { name: "I6", root: 0, tones: [0, 2, 4, 5], alter: (-1, 0), role: Role::Tonic },
    ChordSpec { name: "ii7", root: 1, tones: [0, 2, 4, 6], alter: (-1, 0), role: Role::Subdominant },
    ChordSpec { name: "iii7", root: 2, tones: [0, 2, 4, 6], alter: (-1, 0), role: Role::Colour },
    ChordSpec { name: "IVmaj7", root: 3, tones: [0, 2, 4, 6], alter: (-1, 0), role: Role::Subdominant },
    ChordSpec { name: "IVadd9", root: 3, tones: [0, 2, 4, 1], alter: (-1, 0), role: Role::Subdominant },
    // The dominant, and the reason there is no "no dominants" rule any more: a
    // sus4 with the ninth on top has all of V's pull and none of its glare, and
    // it resolves to I by two voices moving a step.
    ChordSpec { name: "Vsus4", root: 4, tones: [0, 3, 4, 1], alter: (-1, 0), role: Role::Dominant },
    ChordSpec { name: "Vadd9", root: 4, tones: [0, 2, 4, 1], alter: (-1, 0), role: Role::Dominant },
    ChordSpec { name: "vi7", root: 5, tones: [0, 2, 4, 6], alter: (-1, 0), role: Role::Tonic },
    ChordSpec { name: "vi9", root: 5, tones: [0, 2, 4, 1], alter: (-1, 0), role: Role::Tonic },
    // Borrowed: the flat seventh, which is the sound of major with the corners
    // taken off, and the minor fourth, which is the sound of the light going.
    ChordSpec { name: "bVII", root: 6, tones: [0, 2, 4, 1], alter: (6, 10), role: Role::Colour },
    ChordSpec { name: "iv", root: 3, tones: [0, 2, 4, 1], alter: (5, 8), role: Role::Colour },
];

/// Semitones above the tonic for a scale degree, which may run past an octave,
/// under a chord's alteration.
fn degree_semitone(degree: i32, alter: (i32, i32)) -> i32 {
    let octave = degree.div_euclid(7);
    let class = degree.rem_euclid(7);
    let base = if alter.0 == class { alter.1 } else { MAJOR[class as usize] };
    base + 12 * octave
}

/// The nearest scale degree to a semitone offset from the tonic.
///
/// The inverse of [`degree_semitone`], and well defined because a borrowed note
/// is always exactly one semitone *below* the degree it replaces -- hence the
/// `<=`, which breaks a tie upward and reads a flattened sixth back as a sixth
/// rather than as a fifth. Used by the tests to read a piece back in
/// scale-degree space, where a diatonic transposition is an integer.
#[allow(dead_code)]
fn degree_of(semitone: i32) -> i32 {
    let octave = semitone.div_euclid(12);
    let class = semitone.rem_euclid(12);
    let mut best = 0;
    let mut gap = 99;
    for (d, s) in MAJOR.iter().enumerate() {
        let this = (class - s).abs();
        if this <= gap {
            gap = this;
            best = d as i32;
        }
    }
    best + 7 * octave
}

/// Four-bar progressions, one chord to a bar.
///
/// Functional families, all of them: `I-V-vi-IV`, `I-IV-I-V`, `vi-IV-I-V`,
/// `ii-V` colour. A piece draws one of these as its **home** and the ear learns
/// it, because A, A' and A'' all sit on it.
const HOMES: [[usize; 4]; 6] = [
    [I_ADD9, V_ADD9, VI_7, IV_MAJ7],
    [I_ADD9, IV_MAJ7, I_SIX, V_SUS],
    [VI_7, IV_ADD9, I_ADD9, V_SUS],
    [I_ADD9, III_7, IV_MAJ7, V_SUS],
    [I_ADD9, VI_9, II_7, V_SUS],
    [IV_ADD9, I_SIX, II_7, V_SUS],
];

/// The B section's progression. Chosen to *start* somewhere other than the
/// tonic, so the contrast is heard in the first bar rather than deduced.
const ANSWERS: [[usize; 4]; 6] = [
    [II_7, V_SUS, I_ADD9, VI_7],
    [IV_MAJ7, V_SUS, III_7, VI_7],
    [VI_7, III_7, IV_MAJ7, I_ADD9],
    [II_7, V_ADD9, VI_9, IV_ADD9],
    [IV_ADD9, IV_MAJ7, I_ADD9, III_7],
    [I_ADD9, FLAT_VII, IV_MAJ7, I_SIX],
];

/// The C section: the piece's one breath, and where the borrowed colour lives.
const BREATHS: [[usize; 4]; 4] = [
    [IV_MAJ7, FLAT_VII, I_ADD9, I_ADD9],
    [I_ADD9, IV_MINOR, I_SIX, V_SUS],
    [VI_9, II_7, IV_ADD9, I_SIX],
    [IV_ADD9, I_ADD9, FLAT_VII, IV_MAJ7],
];

/// Cadences. Every one of them ends tonic, and the bar before the tonic is a
/// dominant or a subdominant -- authentic or plagal, never a colour chord
/// pretending to be an ending.
const CADENCES: [[usize; 4]; 4] = [
    [I_ADD9, VI_7, II_7, I_ADD9],
    [IV_MAJ7, I_SIX, V_SUS, I_ADD9],
    [VI_7, II_7, V_SUS, I_ADD9],
    // Plagal, onto a major seventh: it settles without closing the door.
    [I_ADD9, III_7, IV_MAJ7, I_MAJ7],
];

// -- Form -----------------------------------------------------

/// What a section does with the melody.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    /// A statement of a motif, with a variation applied.
    Motif(Variation),
    /// A free line: no motif, and the answer to one.
    Free,
    /// Silence. The intro has no tune at all, and the breath section is mostly
    /// rest -- "long breathing rests between phrases" is a structural decision,
    /// not a gap left by the note chooser.
    Rest,
}

/// How a motif is transformed for one restatement.
///
/// A motif that only ever appears verbatim is a loop; a motif that never
/// appears verbatim is not a motif. These are the four classical answers, and
/// all of them preserve the interval signature (up to sign) that a listener
/// actually recognises.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Variation {
    /// Which of the piece's two motifs.
    motif: u8,
    /// Preferred diatonic transposition, in scale degrees, on top of the
    /// harmony-driven anchor search.
    shift: i8,
    /// Duration multiplier in eighths of a beat: `8` as written, `12` augmented
    /// by half, `16` doubled. An integer so the enum stays `Eq` and the schedule
    /// can be compared in a test.
    stretch: u8,
    /// Octave displacement, in octaves.
    octave: i8,
    /// Every interval negated.
    invert: bool,
}

const fn var(motif: u8, shift: i8, stretch: u8, octave: i8, invert: bool) -> Line {
    Line::Motif(Variation { motif, shift, stretch, octave, invert })
}

/// Where a section's chords come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ground {
    /// The piece's home progression, with phrase 2 left open on the dominant
    /// and phrase 4 replaced by a cadence.
    Home,
    /// The B section's own progression, same treatment.
    Answer,
    /// The breath.
    Breath,
    /// Four bars of the home progression, no cadence: the intro hands over on
    /// the dominant.
    Opening,
    /// Home once, then a cadence.
    Closing,
}

struct SectionSpec {
    #[allow(dead_code)]
    label: &'static str,
    ground: Ground,
    phrases: i32,
    /// One entry per phrase; shorter arrays repeat their last entry.
    lines: [Line; 4],
    /// Section dynamic. The range is narrow on purpose (brief S1).
    level: f32,
    /// Register offset for the melody, in scale degrees.
    lift: i8,
    /// Whether the plucked arpeggio runs under this section.
    arpeggio: bool,
    /// Whether the bass marks the third beat as well as the first.
    walk: bool,
}

/// intro - A - B - A' - C - A'' - outro. Ninety-two bars, four minutes ten.
///
/// Long enough that a three-to-five minute cue does not go round twice, short
/// enough that it is a *piece* rather than a bed with notes in it.
const FORM: [SectionSpec; 7] = [
    SectionSpec {
        label: "intro",
        ground: Ground::Opening,
        phrases: 1,
        lines: [Line::Rest; 4],
        level: 0.72,
        lift: 0,
        arpeggio: false,
        walk: false,
    },
    SectionSpec {
        label: "A",
        ground: Ground::Home,
        phrases: PHRASES_PER_SECTION,
        lines: [
            var(0, 0, 8, 0, false),
            var(0, 2, 8, 0, false),
            var(0, 0, 12, 0, false),
            var(0, 0, 8, 0, false),
        ],
        level: 1.0,
        lift: 0,
        arpeggio: false,
        walk: true,
    },
    SectionSpec {
        label: "B",
        ground: Ground::Answer,
        phrases: PHRASES_PER_SECTION,
        lines: [
            var(1, 0, 8, 0, false),
            var(1, 1, 8, 0, false),
            Line::Free,
            var(1, 0, 8, 0, false),
        ],
        level: 1.0,
        lift: 1,
        arpeggio: true,
        walk: true,
    },
    SectionSpec {
        label: "A'",
        ground: Ground::Home,
        phrases: PHRASES_PER_SECTION,
        lines: [
            var(0, 0, 8, 1, false),
            var(0, 2, 8, 0, false),
            var(0, -1, 8, 0, true),
            var(0, 0, 8, 0, false),
        ],
        level: 1.0,
        lift: 0,
        arpeggio: true,
        walk: true,
    },
    SectionSpec {
        label: "C",
        ground: Ground::Breath,
        phrases: PHRASES_PER_SECTION,
        lines: [
            var(1, 0, 16, 0, false),
            Line::Rest,
            var(0, 0, 16, -1, false),
            Line::Free,
        ],
        level: 0.84,
        lift: -1,
        arpeggio: false,
        walk: false,
    },
    SectionSpec {
        label: "A''",
        ground: Ground::Home,
        phrases: PHRASES_PER_SECTION,
        lines: [
            var(0, 0, 8, 0, false),
            var(0, 2, 8, 0, false),
            var(0, 0, 12, 1, false),
            var(0, 0, 8, 0, false),
        ],
        level: 1.0,
        lift: 0,
        arpeggio: true,
        walk: true,
    },
    SectionSpec {
        label: "outro",
        ground: Ground::Closing,
        phrases: 2,
        lines: [var(0, 0, 16, 0, false), Line::Rest, Line::Rest, Line::Rest],
        level: 0.66,
        lift: 0,
        arpeggio: false,
        walk: false,
    },
];

/// What a phrase's line does with its register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Contour {
    /// Rise to a peak about two thirds through, then fall away.
    Arch,
    Fall,
    Rise,
    /// Stay put and move by inches.
    Hover,
}

impl Contour {
    fn at(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Arch => {
                if t < 0.62 {
                    0.22 + 0.78 * raised_cosine(t / 0.62)
                } else {
                    lerp(1.0, 0.32, raised_cosine((t - 0.62) / 0.38))
                }
            }
            Self::Fall => lerp(0.90, 0.14, raised_cosine(t)),
            Self::Rise => lerp(0.16, 0.92, raised_cosine(t)),
            Self::Hover => 0.40 + 0.18 * raised_cosine((t * 2.0).min(1.0)),
        }
    }

    fn peak_at(self) -> f32 {
        match self {
            Self::Arch => 0.62,
            Self::Fall => 0.0,
            Self::Rise => 1.0,
            Self::Hover => 0.5,
        }
    }
}

/// The four contours, cycled per phrase so a section rises and falls rather
/// than repeating one gesture four times.
const CONTOURS: [Contour; 4] = [Contour::Arch, Contour::Fall, Contour::Rise, Contour::Arch];

// -- The composition ------------------------------------------

/// Which line an event belongs to. Each is a different instrument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Bass,
    /// The sustained harmony.
    Pad,
    /// The tune, on the bell/keys voice.
    Keys,
    /// The plucked arpeggio.
    Pluck,
}

/// One note.
#[derive(Clone, Copy, Debug)]
pub struct Event {
    /// Onset in samples from the top of the piece.
    pub at: u32,
    /// How long the note is held, in samples. Only the pad reads it; the struck
    /// voices have their own decay.
    pub dur: u32,
    /// Semitones above [`C2_HZ`].
    pub pitch: f32,
    pub amp: f32,
    /// Structural importance, `0..=255`. What lets the sparse variant thin the
    /// piece coherently rather than at random: the pad, the bass on the
    /// downbeat and the motif survive; the arpeggio and the walking bass do not.
    pub weight: u8,
    pub part: Part,
}

/// A chord as it is actually played: a bass note and four led upper voices.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct PlacedChord {
    /// Index into [`PALETTE`].
    pub spec: usize,
    /// Absolute semitones above C2.
    pub bass: i32,
    /// Four upper voices, absolute semitones above C2, low to high.
    pub voices: [i32; 4],
    /// Bit `c` set when semitone class `c` *above the tonic* sounds in the
    /// chord. The melody reads this to know what a chord tone is.
    pub classes: u16,
    pub alter: (i32, i32),
    /// Which tier of [`voice_chord`]'s search produced this voicing. `0` is
    /// textbook, `5` means it gave up; the tests assert on the distribution,
    /// which is the only way to know whether the rules are being kept or
    /// quietly abandoned in the corners.
    pub tier: u8,
}

impl PlacedChord {
    #[allow(dead_code)]
    pub fn name(&self) -> &'static str {
        PALETTE[self.spec].name
    }

    #[allow(dead_code)]
    pub fn role(&self) -> Role {
        PALETTE[self.spec].role
    }

    fn is_chord_tone(&self, degree: i32) -> bool {
        let class = degree_semitone(degree, self.alter).rem_euclid(12);
        self.classes & (1 << class) != 0
    }
}

/// A note of a motif, stored relative to the motif's own first note.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotifNote {
    /// Scale degrees above the motif's first note.
    pub degree: i32,
    /// Beats from the start of the phrase.
    pub onset: f32,
    /// Length in beats.
    pub len: f32,
}

/// A composed piece: everything a cue plays, and everything a test reads.
///
/// The diagnostic half -- the key, the bar-by-bar harmony, the motifs, the
/// section spans -- is here so that the theory tests can check what the piece
/// *is* rather than inferring it back out of a note list.
#[allow(dead_code)]
pub struct Piece {
    pub events: Vec<Event>,
    /// Length in samples.
    pub len: u32,
    pub key: usize,
    /// One entry per bar.
    pub chords: Vec<PlacedChord>,
    pub motifs: [Vec<MotifNote>; 2],
    /// `(first bar, bar count)` for each section of [`FORM`].
    pub sections: Vec<(usize, usize)>,
}

fn beat_len() -> f32 {
    SR * 60.0 / BPM
}

fn at_beat(beat: f32) -> u32 {
    (beat.max(0.0) * beat_len()) as u32
}

/// Beats 1 and 3 of a 4/4 bar are the strong ones.
fn is_strong(beat: f32) -> bool {
    let inside = beat.rem_euclid(BEATS_PER_BAR);
    inside < 0.01 || (inside - 2.0).abs() < 0.01
}

// -- Voicing --------------------------------------------------

/// True when two four-part voicings move in parallel fifths or octaves.
fn parallel(from: [i32; 4], to: [i32; 4], from_bass: i32, to_bass: i32) -> bool {
    let perfect = |i: i32| {
        let c = i.rem_euclid(12);
        c == 0 || c == 7
    };
    let mut lines: [(i32, i32); 5] = [(0, 0); 5];
    lines[0] = (from_bass, to_bass);
    for v in 0..4 {
        lines[v + 1] = (from[v], to[v]);
    }
    for a in 0..5 {
        for b in (a + 1)..5 {
            let moved_a = lines[a].1 - lines[a].0;
            let moved_b = lines[b].1 - lines[b].0;
            // Oblique and contrary motion are always fine.
            if moved_a == 0 || moved_b == 0 || moved_a.signum() != moved_b.signum() {
                continue;
            }
            let before = lines[b].0 - lines[a].0;
            let after = lines[b].1 - lines[a].1;
            if perfect(before) && before.rem_euclid(12) == after.rem_euclid(12) {
                return true;
            }
        }
    }
    false
}

/// Every way this chord's four tones can be stacked inside the pad register.
///
/// Enumerated exhaustively -- an octave for each of the four tones, filtered to
/// what fits and what ascends -- rather than as a handful of rotations. It is
/// two hundred and fifty-six iterations of integer arithmetic once per bar at
/// compose time, and it is what turns "no parallel fifths" from a hope into
/// something the generator can actually satisfy: close position alone leaves
/// pairs like `bVII -> Vsus4` with no legal move at all, and then the search
/// has to break a rule to get anywhere.
fn voicings(classes: [i32; 4]) -> Vec<[i32; 4]> {
    let base = classes.map(|c| c.rem_euclid(12));
    let lo_octave = PAD_LO.div_euclid(12) * 12;
    let mut out: Vec<[i32; 4]> = Vec::with_capacity(64);
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    let mut v = [
                        base[0] + lo_octave + 12 * a,
                        base[1] + lo_octave + 12 * b,
                        base[2] + lo_octave + 12 * c,
                        base[3] + lo_octave + 12 * d,
                    ];
                    v.sort_unstable();
                    if v[0] < PAD_LO || v[3] > PAD_HI {
                        continue;
                    }
                    // Strictly ascending: no doubling, no crossed voices.
                    if v.windows(2).any(|w| w[0] >= w[1]) {
                        continue;
                    }
                    if !out.contains(&v) {
                        out.push(v);
                    }
                }
            }
        }
    }
    out
}

/// Realise a chord as four upper voices.
///
/// Part writing by search rather than by hand. The candidates are filtered in
/// tiers -- least motion with no parallels first, then progressively looser --
/// so the good answer is taken when it exists and the piece is still playable
/// when it does not. The tests check the result rather than the intent, which
/// is the only reason a search is allowed to do this job at all.
fn voice_chord(
    classes: [i32; 4],
    bass: i32,
    prev: Option<([i32; 4], i32)>,
    rng: &mut Rng,
) -> (usize, f32, [i32; 4]) {
    let candidates = voicings(classes);
    let shape = |v: [i32; 4]| -> f32 {
        // A voicing wider than a twelfth is an arrangement, not a pad, and one
        // sitting on the bass's own octave is mud.
        let spread = v[3] - v[0];
        let mut cost = if spread > 19 { (spread - 19) as f32 * 2.0 } else { 0.0 };
        cost += ((v[0] - bass) - 10).min(0).abs() as f32 * 1.2;
        cost
    };

    let Some((from, from_bass)) = prev else {
        // The first chord of the piece: sit in the middle of the register, with
        // a hair of seed-dependent taste so four pieces do not all open in the
        // same inversion.
        let centre = ((PAD_LO + PAD_HI) as f32) * 0.5;
        let mut best: Option<([i32; 4], f32)> = None;
        for v in candidates {
            let mean = (v[0] + v[1] + v[2] + v[3]) as f32 * 0.25;
            let cost = shape(v) + (mean - centre).abs() + rng.range(0.0, 3.0);
            if best.is_none_or(|(_, c)| cost < c) {
                best = Some((v, cost));
            }
        }
        return best
            .map(|(v, c)| (0, c, v))
            .unwrap_or((0, 0.0, [PAD_LO, PAD_LO + 4, PAD_LO + 7, PAD_LO + 12]));
    };

    // Tier 0 is the writing this file promises; each one after it gives up one
    // thing, and the tests fail if anything past tier 2 is ever needed. The
    // order matters: a held common tone is worth more than a small move, which
    // is why `iii7 -> IVmaj7` -- where every close voicing makes a parallel
    // fifth between the seventh and the fifth -- ends up on an open voicing
    // with one voice pinned rather than on a tidy stepwise slide.
    for tier in 0..7 {
        let mut best: Option<([i32; 4], f32)> = None;
        for &v in &candidates {
            let moves: [i32; 4] = [
                v[0] - from[0],
                v[1] - from[1],
                v[2] - from[2],
                v[3] - from[3],
            ];
            let worst = moves.iter().map(|m| m.abs()).max().unwrap_or(0);
            let common = moves.iter().filter(|m| **m == 0).count();
            let clashes = parallel(from, v, from_bass, bass);
            let ok = match tier {
                0 => !clashes && worst <= 5 && common >= 1,
                1 => !clashes && worst <= 5,
                2 => !clashes && worst <= 7 && common >= 1,
                3 => !clashes && worst <= 7,
                4 => !clashes && worst <= 10,
                5 => !clashes,
                _ => true,
            };
            if !ok {
                continue;
            }
            let motion: i32 = moves.iter().map(|m| m.abs()).sum();
            // The top voice is the one a listener follows; it may not leap
            // where an inner voice may.
            let cost = shape(v) + motion as f32 + (moves[3].abs() as f32 - 4.0).max(0.0) * 2.5;
            if best.is_none_or(|(_, c)| cost < c) {
                best = Some((v, cost));
            }
        }
        if let Some((v, cost)) = best {
            return (tier, cost, v);
        }
    }
    (6, 0.0, from)
}

/// Where a chord root may sit in the bass octave: one place, or two when the
/// root is low enough that its octave still fits under C3.
///
/// Both are offered to [`voice_chord`] rather than one being picked first,
/// because the bass is a *part*: a parallel fifth between it and an inner voice
/// is the most audible one there is, and choosing the bass before the upper
/// voices regularly leaves the search with no legal answer at all.
fn bass_options(class: i32) -> [i32; 2] {
    let low = class.rem_euclid(12).max(BASS_LO);
    let high = low + 12;
    [low, if high <= BASS_HI { high } else { low }]
}

// -- Melody ---------------------------------------------------

/// Note lengths in beats, and their weights.
const LENGTHS: [(f32, f32); 6] = [
    (0.5, 2.4),
    (1.0, 5.0),
    (1.5, 2.2),
    (2.0, 3.0),
    (3.0, 1.1),
    (4.0, 0.7),
];

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
    1.0
}

/// Weight for an interval of `n` scale degrees.
///
/// Steps outnumber thirds nearly three to one and fourths six to one. This
/// single table is most of the difference between a melody and a random walk in
/// a scale.
fn interval_weight(n: i32) -> f32 {
    match n.abs() {
        0 => 0.30,
        1 => 12.0,
        2 => 2.4,
        3 => 1.30,
        4 => 0.80,
        5 => 0.16,
        6 => 0.03,
        7 => 0.30,
        _ => 0.0,
    }
}

/// State carried from note to note inside a phrase.
struct Walk {
    degree: i32,
    /// The interval that arrived at [`Self::degree`].
    last: i32,
    /// Set when the previous note was a non-chord tone, which must be left by
    /// step in the direction it was approached.
    resolve: i32,
}

#[allow(clippy::too_many_arguments)]
fn pick_note(
    rng: &mut Rng,
    walk: &Walk,
    target: f32,
    chord: &PlacedChord,
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
        let step = degree - walk.degree;
        let mut w = interval_weight(step);
        if w <= 0.0 {
            continue;
        }
        // A passing tone owes a resolution: it may only be left by a step, in
        // the direction it was entered.
        if walk.resolve != 0 && step != walk.resolve {
            continue;
        }
        // A leap of a fourth or more must turn round. A third may be continued:
        // two thirds in a row outline the chord, which is a gesture.
        if walk.last.abs() >= 3 {
            if step != 0 && step.signum() == walk.last.signum() {
                continue;
            }
            if step.abs() == 1 {
                w *= 8.0;
            } else if step.abs() >= 3 {
                w *= 0.20;
            }
        } else if walk.last.abs() == 2 && step.abs() >= 3 {
            w *= 0.30;
        }
        // Pull toward the phrase's contour.
        w *= (-(degree - target.round() as i32).abs() as f32 / 2.6).exp();

        let tone = chord.is_chord_tone(degree);
        if chord_tones_only && !tone {
            continue;
        }
        if strong {
            w *= if tone { 5.0 } else { 0.09 };
        } else if tone {
            w *= 1.35;
        } else if step.abs() != 1 {
            // A non-chord tone that is not approached by step is a wrong note.
            continue;
        }
        // At the top of an arch, favour the ninth and the sixth: the two added
        // tones that make a major chord shimmer rather than merely resolve.
        if near_peak {
            let class = degree.rem_euclid(7);
            if class == 1 || class == 5 {
                w *= 2.1;
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

/// Choose the next melody note. A strong beat takes a chord tone, and that is a
/// rule rather than a weight.
#[allow(clippy::too_many_arguments)]
fn sing(
    rng: &mut Rng,
    walk: &Walk,
    target: f32,
    chord: &PlacedChord,
    strong: bool,
    near_peak: bool,
    lo: i32,
    hi: i32,
) -> i32 {
    if strong {
        if let Some(strict) = pick_note(rng, walk, target, chord, true, near_peak, lo, hi, true) {
            return strict;
        }
    }
    pick_note(rng, walk, target, chord, strong, near_peak, lo, hi, false)
        .unwrap_or_else(|| walk.degree.clamp(lo, hi))
}

/// Nearest chord tone to `degree`, preferring `dir`.
fn snap(degree: i32, chord: &PlacedChord, dir: i32, lo: i32, hi: i32) -> i32 {
    if chord.is_chord_tone(degree) {
        return degree.clamp(lo, hi);
    }
    let order: [i32; 4] = if dir >= 0 { [1, -1, 2, -2] } else { [-1, 1, -2, 2] };
    for offset in order {
        let candidate = degree + offset;
        if candidate >= lo && candidate <= hi && chord.is_chord_tone(candidate) {
            return candidate;
        }
    }
    degree.clamp(lo, hi)
}

// -- The composer ---------------------------------------------

struct Composer {
    rng: Rng,
    events: Vec<Event>,
    key: Key,
}

impl Composer {
    #[allow(clippy::too_many_arguments)]
    fn push(&mut self, at: f32, dur: f32, pitch: f32, amp: f32, weight: u8, part: Part) {
        self.events.push(Event {
            at: at_beat(at),
            dur: at_beat(dur.max(0.05)),
            pitch,
            amp,
            weight,
            part,
        });
    }

    /// Absolute pitch of a melody degree under a chord.
    fn melody_pitch(&self, degree: i32, chord: &PlacedChord) -> f32 {
        (self.key.root + degree_semitone(degree, chord.alter)) as f32
    }

    /// Generate one phrase of melody, four bars long.
    ///
    /// Returns the notes so a phrase can be harvested as a motif.
    #[allow(clippy::too_many_arguments)]
    fn phrase(
        &mut self,
        bars: &[PlacedChord],
        contour: Contour,
        lo: i32,
        hi: i32,
        seed_notes: &[MotifNote],
        anchor: Option<i32>,
        sounding: f32,
    ) -> Vec<MotifNote> {
        let span = BARS_PER_PHRASE as f32 * BEATS_PER_BAR;
        let chord_at = |beat: f32| -> PlacedChord {
            let bar = (beat / BEATS_PER_BAR).floor().max(0.0) as usize;
            bars[bar.min(bars.len() - 1)]
        };

        let mut notes: Vec<MotifNote> = Vec::new();
        let mut beat = 0.0f32;

        // The motif is laid down first, at one transposition, and *not* snapped
        // note by note: snapping would break the interval signature, and the
        // signature is the only thing a listener has to recognise it by. The
        // anchor search has already picked the transposition that puts the most
        // of it on chord tones.
        if !seed_notes.is_empty() {
            let base = anchor.unwrap_or((lo + hi) / 2);
            for note in seed_notes {
                let degree = base + note.degree;
                if degree < lo || degree > hi {
                    continue;
                }
                notes.push(MotifNote { degree, onset: note.onset, len: note.len });
                beat = note.onset + note.len;
            }
        }

        let carried = match notes.as_slice() {
            [.., a, b] => b.degree - a.degree,
            _ => 0,
        };
        let mut walk = Walk {
            degree: notes
                .last()
                .map(|n| n.degree)
                .or(anchor)
                .unwrap_or((lo + hi) / 2),
            last: carried,
            resolve: 0,
        };
        if notes.is_empty() {
            let chord = chord_at(0.0);
            let target = lo as f32 + contour.at(0.0) * (hi - lo) as f32;
            walk.degree = snap(target.round() as i32, &chord, 1, lo, hi);
            let len = draw_length(&mut self.rng, false);
            notes.push(MotifNote { degree: walk.degree, onset: 0.0, len });
            beat = len;
        }

        let peak = contour.peak_at();
        while beat < sounding {
            let t = beat / span;
            let target = lo as f32 + contour.at(t) * (hi - lo) as f32;
            let chord = chord_at(beat);
            let strong = is_strong(beat);
            let last = beat + 2.0 >= sounding;
            let near_peak = (t - peak).abs() < 0.18;

            let degree = if last {
                let want = sing(&mut self.rng, &walk, target, &chord, true, false, lo, hi);
                snap(want, &chord, (want - walk.degree).signum(), lo, hi)
            } else {
                sing(&mut self.rng, &walk, target, &chord, strong, near_peak, lo, hi)
            };

            let step = degree - walk.degree;
            walk.resolve = if chord.is_chord_tone(degree) || step == 0 {
                0
            } else {
                step.signum()
            };
            walk.last = step;
            walk.degree = degree;

            let len = draw_length(&mut self.rng, last);
            notes.push(MotifNote { degree, onset: beat, len });
            beat += len;
        }
        notes
    }

    /// Turn a phrase into events.
    fn lay_melody(
        &mut self,
        notes: &[MotifNote],
        bars: &[PlacedChord],
        phrase_beat: f32,
        stated: usize,
        level: f32,
    ) {
        for (i, note) in notes.iter().enumerate() {
            let bar = (note.onset / BEATS_PER_BAR).floor().max(0.0) as usize;
            let chord = bars[bar.min(bars.len() - 1)];
            let weight: u8 = if i < stated {
                240
            } else if i == 0 || note.len >= 2.0 {
                190
            } else {
                178
            };
            let jitter = self.rng.range(0.92, 1.08);
            let amp = 0.30 * level * jitter * if i == 0 { 1.10 } else { 1.0 };
            let pitch = self.melody_pitch(note.degree, &chord);
            self.push(phrase_beat + note.onset, note.len, pitch, amp, weight, Part::Keys);
        }
    }

    /// The bass and the plucked arpeggio, one bar at a time.
    fn lay_rhythm(&mut self, bar: usize, chord: &PlacedChord, spec: &SectionSpec) {
        let bar_beat = bar as f32 * BEATS_PER_BAR;
        // The downbeat. This is the pulse, and it is the only thing in the
        // piece that is exactly on the grid.
        self.push(
            bar_beat,
            BEATS_PER_BAR,
            chord.bass as f32,
            0.44 * spec.level,
            248,
            Part::Bass,
        );
        if spec.walk {
            // Beat three, a fifth up where the fifth fits under the ceiling of
            // the bass octave and the root again where it does not. Quieter,
            // and one of the two things a thin town loses.
            let fifth = chord.bass + 7;
            let pitch = if fifth <= BASS_HI { fifth } else { chord.bass };
            self.push(
                bar_beat + 2.0,
                2.0,
                pitch as f32,
                0.26 * spec.level,
                135,
                Part::Bass,
            );
        }
        if spec.arpeggio {
            // Off-beats only. On the beat it would be a drum; between the beats
            // it is the floor the tune walks on.
            const FIGURES: [[(f32, usize); 3]; 4] = [
                [(0.5, 1), (1.5, 2), (3.5, 3)],
                [(0.5, 2), (2.5, 1), (3.5, 2)],
                [(1.5, 3), (2.5, 2), (3.5, 1)],
                [(0.5, 0), (1.5, 2), (2.5, 3)],
            ];
            let figure = FIGURES[self.rng.below(FIGURES.len())];
            for (beat, voice) in figure {
                let jitter = self.rng.range(0.85, 1.15);
                self.push(
                    bar_beat + beat,
                    1.0,
                    chord.voices[voice] as f32,
                    0.16 * spec.level * jitter,
                    105,
                    Part::Pluck,
                );
            }
        }
    }

    /// The pad, laid across the whole piece in one pass.
    ///
    /// A voice that does not move between two chords is **not retriggered** --
    /// the common tone simply stays down, which is what a held pad is and what
    /// a chord-by-chord retrigger can never be.
    fn lay_pad(&mut self, chords: &[PlacedChord], levels: &[f32]) {
        for v in 0..4 {
            let mut bar = 0usize;
            while bar < chords.len() {
                let pitch = chords[bar].voices[v];
                let mut end = bar + 1;
                while end < chords.len() && chords[end].voices[v] == pitch {
                    end += 1;
                }
                let bars = (end - bar) as f32;
                // The lowest voice carries a little more, as it does on any real
                // instrument; the spread is under two decibels.
                let voice_amp = lerp(0.115, 0.088, v as f32 / 3.0);
                self.push(
                    bar as f32 * BEATS_PER_BAR,
                    bars * BEATS_PER_BAR,
                    pitch as f32,
                    voice_amp * levels[bar],
                    250,
                    Part::Pad,
                );
                bar = end;
            }
        }
    }
}

/// Apply a variation to a motif.
fn vary(motif: &[MotifNote], v: Variation) -> Vec<MotifNote> {
    let stretch = v.stretch as f32 / 8.0;
    motif
        .iter()
        .map(|n| MotifNote {
            degree: if v.invert { -n.degree } else { n.degree } + v.octave as i32 * 7,
            onset: n.onset * stretch,
            len: n.len * stretch,
        })
        .collect()
}

/// Where to put a motif so that it agrees with the harmony under it.
///
/// Every legal transposition is scored by how many of its strong-beat notes are
/// chord tones, plus a pull toward the phrase's contour and toward the
/// variation's requested shift. This is the step that lets one motif sit over
/// four different progressions without ever being edited note by note.
fn anchor_for(
    motif: &[MotifNote],
    bars: &[PlacedChord],
    lo: i32,
    hi: i32,
    want: f32,
    shift: i32,
) -> Option<i32> {
    let min = motif.iter().map(|n| n.degree).min()?;
    let max = motif.iter().map(|n| n.degree).max()?;
    let mut best: Option<(i32, f32)> = None;
    for anchor in (lo - min)..=(hi - max) {
        let mut score = 0.0f32;
        for note in motif {
            let bar = (note.onset / BEATS_PER_BAR).floor().max(0.0) as usize;
            let chord = bars[bar.min(bars.len() - 1)];
            let degree = anchor + note.degree;
            if chord.is_chord_tone(degree) {
                score += if is_strong(note.onset) { 3.0 } else { 1.0 };
            } else if is_strong(note.onset) {
                score -= 2.0;
            }
        }
        score -= (anchor as f32 - want).abs() * 0.55;
        score -= (anchor - (lo + hi) / 2 - shift).abs() as f32 * 0.20;
        if best.is_none_or(|(_, s)| score > s) {
            best = Some((anchor, score));
        }
    }
    best.map(|(a, _)| a)
}

/// Rhythmic cells a motif may be built on, as `(onset, length)` in beats.
///
/// Four of the six begin off the beat or put a note on the second half of one:
/// gentle syncopation against a bass that is always exactly on the grid is what
/// keeps a calm piece from sounding metronomic.
const CELLS: [&[(f32, f32)]; 6] = [
    &[(0.0, 1.0), (1.0, 1.0), (2.0, 2.0)],
    &[(0.0, 0.5), (0.5, 0.5), (1.0, 1.0), (2.0, 2.0)],
    &[(0.0, 1.0), (1.5, 0.5), (2.0, 1.0), (3.0, 1.0)],
    &[(0.5, 0.5), (1.0, 1.0), (2.0, 1.0), (3.0, 1.0)],
    &[(0.0, 1.0), (1.0, 0.5), (1.5, 0.5), (2.0, 1.0), (3.0, 1.0)],
    &[(0.0, 1.0), (1.5, 0.5), (2.0, 1.0), (3.0, 1.0), (4.0, 2.0)],
];

/// Build a motif over the opening bars of a progression.
fn make_motif(c: &mut Composer, bars: &[PlacedChord], lo: i32, hi: i32, cell: usize) -> Vec<MotifNote> {
    let cell = CELLS[cell % CELLS.len()];
    let contour = if c.rng.chance(0.6) { Contour::Arch } else { Contour::Rise };
    let mid = (lo + hi) / 2;
    let chord_at = |beat: f32| -> PlacedChord {
        let bar = (beat / BEATS_PER_BAR).floor().max(0.0) as usize;
        bars[bar.min(bars.len() - 1)]
    };

    let first = snap(mid + c.rng.below(3) as i32 - 1, &chord_at(cell[0].0), 1, lo, hi);
    let mut walk = Walk { degree: first, last: 0, resolve: 0 };
    let mut out = vec![MotifNote { degree: 0, onset: cell[0].0, len: cell[0].1 }];
    let span = cell[cell.len() - 1].0 + cell[cell.len() - 1].1;

    for &(onset, len) in cell.iter().skip(1) {
        let chord = chord_at(onset);
        let t = onset / span.max(1.0);
        let target = lo as f32 + contour.at(t) * (hi - lo) as f32;
        // A motif is sung, so it stays close to where it started: a shape that
        // needs an octave to state is not a shape anybody hums.
        let (near_lo, near_hi) = ((first - 3).max(lo), (first + 4).min(hi));
        let degree = sing(
            &mut c.rng,
            &walk,
            target,
            &chord,
            is_strong(onset),
            false,
            near_lo,
            near_hi,
        );
        walk.last = degree - walk.degree;
        walk.resolve = 0;
        walk.degree = degree;
        out.push(MotifNote { degree: degree - first, onset, len });
    }
    out
}

/// The chords of one section, one per bar.
fn ground_chords(ground: Ground, home: [usize; 4], answer: [usize; 4], breath: [usize; 4], cadence: [usize; 4]) -> Vec<usize> {
    let half = |p: [usize; 4]| -> [usize; 4] {
        let mut q = p;
        // Leave the second phrase open on the dominant, so the third has
        // somewhere to come back from.
        q[3] = V_SUS;
        q
    };
    match ground {
        // The intro always hands over on the dominant, whatever the home
        // progression happens to end on.
        Ground::Opening => half(home).to_vec(),
        Ground::Home => [home, half(home), home, cadence].concat(),
        Ground::Answer => [answer, half(answer), answer, cadence].concat(),
        Ground::Breath => [breath, breath, half(breath), cadence].concat(),
        Ground::Closing => [home, cadence].concat(),
    }
}

/// Compose one piece. Pure, and deterministic in `(seed, variant)`.
pub fn compose(seed: u64, variant: u32) -> Piece {
    let mut rng = Rng::new(
        seed ^ 0x4d61_6a6f_7200 ^ (variant as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15),
    );

    // The tonal centre is per piece, not per world: the world picks where the
    // rotation starts and the four pieces then take the four keys in order, so
    // no two consecutive cues can land in the same one.
    let offset = Rng::new(seed ^ 0x0074_6f6e_6963).below(KEYS.len());
    let key_index = (variant as usize + offset) % KEYS.len();
    let key = KEYS[key_index];

    let home = HOMES[rng.below(HOMES.len())];
    let answer = ANSWERS[rng.below(ANSWERS.len())];
    let breath = BREATHS[rng.below(BREATHS.len())];
    let cadences = [
        CADENCES[rng.below(CADENCES.len())],
        CADENCES[rng.below(CADENCES.len())],
        CADENCES[rng.below(CADENCES.len())],
    ];

    // -- the harmony, bar by bar, led rather than stacked ------
    let mut plan: Vec<usize> = Vec::new();
    let mut sections: Vec<(usize, usize)> = Vec::new();
    for (s, spec) in FORM.iter().enumerate() {
        let bars = ground_chords(spec.ground, home, answer, breath, cadences[s % cadences.len()]);
        sections.push((plan.len(), bars.len()));
        plan.extend(bars);
    }

    let mut chords: Vec<PlacedChord> = Vec::with_capacity(plan.len());
    let mut prev: Option<([i32; 4], i32)> = None;
    for &index in &plan {
        let spec = PALETTE[index];
        let mut classes_abs = [0i32; 4];
        let mut classes = 0u16;
        for (i, t) in spec.tones.iter().enumerate() {
            let semis = degree_semitone(spec.root + t, spec.alter);
            classes |= 1 << semis.rem_euclid(12);
            classes_abs[i] = (key.root + semis).rem_euclid(12);
        }
        let root_class = key.root + degree_semitone(spec.root, spec.alter);
        // The bass and the upper voices are written together: the best pair
        // wins, ranked by how few rules it had to give up and then by how
        // little everything moved.
        let mut chosen: Option<(usize, f32, i32, [i32; 4])> = None;
        for bass in bass_options(root_class) {
            let (tier, cost, voices) = voice_chord(classes_abs, bass, prev, &mut rng);
            let travel = prev.map(|(_, b)| (bass - b).abs()).unwrap_or(0) as f32 * 0.6;
            let total = cost + travel;
            if chosen.is_none_or(|(t, c, _, _)| (tier, total) < (t, c)) {
                chosen = Some((tier, total, bass, voices));
            }
        }
        let (tier, _, bass, voices) = chosen.expect("bass_options is never empty");
        prev = Some((voices, bass));
        chords.push(PlacedChord {
            spec: index,
            bass,
            voices,
            classes,
            alter: spec.alter,
            tier: tier as u8,
        });
    }

    // -- the register, in scale degrees of this key ------------
    let degree_window = |lo_semis: i32, hi_semis: i32| -> (i32, i32) {
        let mut lo = 0;
        while key.root + degree_semitone(lo, (-1, 0)) < lo_semis {
            lo += 1;
        }
        let mut hi = lo;
        while key.root + degree_semitone(hi + 1, (-1, 0)) <= hi_semis {
            hi += 1;
        }
        (lo, hi)
    };
    let (melody_lo, melody_hi) = degree_window(MELODY_LO, MELODY_HI);

    let mut c = Composer { rng, events: Vec::with_capacity(1024), key };

    // -- the motifs -------------------------------------------
    //
    // One over the home progression and one over the B section's, so each is
    // already at home in the harmony it will spend most of its life over.
    let cell_a = c.rng.below(CELLS.len());
    let motif_a = make_motif(&mut c, &chords[..4], melody_lo, melody_hi, cell_a);
    let answer_start = sections[2].0;
    let cell_b = (cell_a + 1 + c.rng.below(CELLS.len() - 1)) % CELLS.len();
    let motif_b = make_motif(
        &mut c,
        &chords[answer_start..answer_start + 4],
        melody_lo,
        melody_hi,
        cell_b,
    );
    let motifs = [motif_a, motif_b];

    // -- the sections -----------------------------------------
    let mut levels: Vec<f32> = Vec::with_capacity(chords.len());
    for (s, spec) in FORM.iter().enumerate() {
        let (bar0, bars) = sections[s];
        for (bar, chord) in chords.iter().enumerate().skip(bar0).take(bars) {
            levels.push(spec.level);
            c.lay_rhythm(bar, chord, spec);
        }

        for phrase in 0..spec.phrases {
            let start = bar0 + (phrase * BARS_PER_PHRASE) as usize;
            let window = &chords[start..start + BARS_PER_PHRASE as usize];
            let phrase_beat = start as f32 * BEATS_PER_BAR;
            // The breath hovers; everywhere else the four contours cycle, so
            // a section rises and falls rather than repeating one gesture.
            let contour = if spec.ground == Ground::Breath {
                Contour::Hover
            } else {
                CONTOURS[(phrase as usize + s) % CONTOURS.len()]
            };
            let line = spec.lines[(phrase as usize).min(spec.lines.len() - 1)];
            let lo = (melody_lo + spec.lift as i32).max(melody_lo - 2);
            let hi = (melody_hi + spec.lift as i32).min(melody_hi);

            // Sound for most of the phrase, then stop. The rest is the
            // phrasing: a line that never stops is a drone with opinions.
            let drawn = [10.0f32, 11.0, 12.0, 13.0][c.rng.below(4)];

            let (seed_notes, anchor) = match line {
                Line::Rest => continue,
                Line::Free => (Vec::new(), None),
                Line::Motif(v) => {
                    let shaped = vary(&motifs[v.motif as usize % 2], v);
                    let want = lo as f32 + contour.at(0.0) * (hi - lo) as f32;
                    let anchor = anchor_for(&shaped, window, lo, hi, want, v.shift as i32);
                    (shaped, anchor)
                }
            };
            // A statement always gets a free answer after it, however long the
            // motif turned out. Without this, an augmented motif can fill the
            // whole phrase and two statements of the same variation come out
            // note for note identical -- which is a loop, not a return.
            let stated = seed_notes.len();
            let motif_end = seed_notes
                .iter()
                .map(|n| n.onset + n.len)
                .fold(0.0f32, f32::max);
            let sounding = drawn.max(motif_end + 2.0).min(14.0);
            let notes = c.phrase(window, contour, lo, hi, &seed_notes, anchor, sounding);
            c.lay_melody(&notes, window, phrase_beat, stated, spec.level);
        }
    }

    c.lay_pad(&chords, &levels);
    c.events.sort_by_key(|e| e.at);

    let len = at_beat(chords.len() as f32 * BEATS_PER_BAR);
    Piece { events: c.events, len, key: key_index, chords, motifs, sections }
}

// -- Synthesis ------------------------------------------------

/// A magic-circle sine oscillator: two states, two multiplies and two adds per
/// sample, and no trigonometry at all once it is tuned.
///
/// The score runs eighty-odd of these at once on the audio thread. `sin` per
/// partial per sample would be a hundred times the cost for a difference nobody
/// can hear: the recursion's amplitude error is under a tenth of a decibel at
/// every frequency this file asks for.
#[derive(Clone, Copy, Default)]
struct Osc {
    s: f32,
    c: f32,
    eps: f32,
}

impl Osc {
    fn set(&mut self, hz: f32) {
        let hz = hz.clamp(1.0, SR * 0.45);
        self.eps = 2.0 * (core::f32::consts::PI * hz / SR).sin();
        // Starting at zero is what makes every voice in this file begin from
        // silence rather than from a step.
        self.s = 0.0;
        self.c = 1.0;
    }

    #[inline]
    fn step(&mut self) -> f32 {
        self.s += self.eps * self.c;
        self.c -= self.eps * self.s;
        if !self.s.is_finite() || !self.c.is_finite() {
            self.s = 0.0;
            self.c = 0.0;
        }
        self.s
    }
}

/// Partials of the bell/keys voice: `(ratio, amplitude, decay relative to t60)`.
///
/// The top of the tone falls away first, which is what a struck string does and
/// what a sustaining synth pad never does -- it is most of why this reads as an
/// instrument. The fourth partial is two percent sharp, so it beats slowly
/// against the third and the tone glitters instead of sitting still.
const BELL_PARTIALS: [(f32, f32, f32); 4] = [
    (1.0, 1.00, 1.00),
    (2.0, 0.42, 0.55),
    (3.0, 0.17, 0.34),
    (4.02, 0.075, 0.22),
];

/// The detuned twin of the fundamental, in cents and relative amplitude. Six
/// cents is about one and a half beats a second at A4: chorus, not vibrato.
const BELL_DETUNE_CENTS: f32 = 6.0;
const BELL_DETUNE_AMP: f32 = 0.55;

const BELL_OSCS: usize = BELL_PARTIALS.len() + 1;

/// What the partials and the twin sum to at the instant of the strike.
///
/// Divided out so that `amp` means the same thing on the bell as it does on the
/// bass and the pluck. Without it an additive voice is as loud as the number of
/// partials somebody happened to give it, and every balance in the piece would
/// have to be retuned to add a harmonic.
const BELL_SUM: f32 = 1.0 + 0.42 + 0.17 + 0.075 + BELL_DETUNE_AMP;

/// The melody. Additive, struck, and the one voice a listener follows.
struct Bell {
    active: bool,
    osc: [Osc; BELL_OSCS],
    amp: [f32; BELL_OSCS],
    env: [f32; BELL_OSCS],
    dec: [f32; BELL_OSCS],
    age: u32,
    attack: u32,
    life: u32,
    fade: u32,
}

impl Bell {
    fn silent() -> Self {
        Self {
            active: false,
            osc: [Osc::default(); BELL_OSCS],
            amp: [0.0; BELL_OSCS],
            env: [0.0; BELL_OSCS],
            dec: [0.0; BELL_OSCS],
            age: 0,
            attack: 1,
            life: 0,
            fade: 1,
        }
    }

    /// `t60` is the decay of the fundamental to -60 dB, `attack` the length of
    /// the raised-cosine onset, `tilt` how much of the upper partials survives.
    fn strike(&mut self, freq: f32, amp: f32, t60: f32, attack: f32, tilt: f32) {
        let freq = freq.clamp(20.0, SR * 0.44);
        let amp = amp / BELL_SUM;
        for (i, (ratio, level, decay)) in BELL_PARTIALS.iter().enumerate() {
            self.osc[i].set(freq * ratio);
            self.amp[i] = amp * level * if i == 0 { 1.0 } else { tilt.powi(i as i32) };
            self.env[i] = 1.0;
            let tau = (t60 * decay).max(0.05);
            self.dec[i] = (-6.907_755 / (tau * SR)).exp();
        }
        let last = BELL_OSCS - 1;
        self.osc[last].set(freq * (BELL_DETUNE_CENTS / 1200.0).exp2());
        self.amp[last] = amp * BELL_DETUNE_AMP;
        self.env[last] = 1.0;
        self.dec[last] = (-6.907_755 / (t60.max(0.05) * SR)).exp();
        self.age = 0;
        self.attack = ((attack * SR) as u32).max(1);
        self.life = (t60 * SR) as u32;
        self.fade = (0.05 * SR) as u32;
        self.active = true;
    }

    #[inline]
    fn step(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }
        let mut sum = 0.0;
        for i in 0..BELL_OSCS {
            self.env[i] *= self.dec[i];
            sum += self.osc[i].step() * self.env[i] * self.amp[i];
        }
        if self.age < self.attack {
            sum *= raised_cosine(self.age as f32 / self.attack as f32);
        }
        if self.age + self.fade >= self.life {
            let left = self.life.saturating_sub(self.age) as f32 / self.fade as f32;
            sum *= raised_cosine(left);
        }
        self.age += 1;
        if self.age >= self.life || !sum.is_finite() {
            self.active = false;
            return 0.0;
        }
        sum
    }
}

/// Band-limited step for a naive sawtooth. Ten operations at the discontinuity
/// and none anywhere else; without it a pad at C5 folds a hundred partials back
/// down into the melody's own register.
#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// The harmony. Two detuned saws through two poles of low-pass, slow in and
/// slower out.
struct PadVoice {
    active: bool,
    ph_a: f32,
    ph_b: f32,
    inc_a: f32,
    inc_b: f32,
    lp1: f32,
    lp2: f32,
    coeff: f32,
    amp: f32,
    age: u32,
    attack: u32,
    hold: u32,
    release: u32,
}

impl PadVoice {
    fn silent() -> Self {
        Self {
            active: false,
            ph_a: 0.0,
            ph_b: 0.0,
            inc_a: 0.0,
            inc_b: 0.0,
            lp1: 0.0,
            lp2: 0.0,
            coeff: 0.2,
            amp: 0.0,
            age: 0,
            attack: 1,
            hold: 0,
            release: 1,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start(&mut self, freq: f32, amp: f32, hold: f32, attack: f32, release: f32, cutoff: f32, detune: f32) {
        let freq = freq.clamp(20.0, SR * 0.30);
        self.inc_a = freq * (1.0 - detune) / SR;
        self.inc_b = freq * (1.0 + detune) / SR;
        // Two phases a third of a cycle apart, so the pair never starts in
        // phase and the detune is audible from the first millisecond.
        self.ph_a = 0.0;
        self.ph_b = 0.33;
        self.lp1 = 0.0;
        self.lp2 = 0.0;
        self.coeff = pole_coeff(cutoff, SR);
        self.amp = amp;
        self.age = 0;
        self.attack = ((attack * SR) as u32).max(1);
        self.hold = (hold * SR) as u32;
        self.release = ((release * SR) as u32).max(1);
        self.active = true;
    }

    #[inline]
    fn step(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }
        self.ph_a += self.inc_a;
        if self.ph_a >= 1.0 {
            self.ph_a -= 1.0;
        }
        self.ph_b += self.inc_b;
        if self.ph_b >= 1.0 {
            self.ph_b -= 1.0;
        }
        let a = 2.0 * self.ph_a - 1.0 - poly_blep(self.ph_a, self.inc_a);
        let b = 2.0 * self.ph_b - 1.0 - poly_blep(self.ph_b, self.inc_b);
        let raw = (a + b) * 0.5;
        self.lp1 += (raw - self.lp1) * self.coeff;
        self.lp2 += (self.lp1 - self.lp2) * self.coeff;

        let env = if self.age < self.attack {
            raised_cosine(self.age as f32 / self.attack as f32)
        } else if self.age < self.attack + self.hold {
            1.0
        } else {
            let gone = (self.age - self.attack - self.hold) as f32 / self.release as f32;
            raised_cosine(1.0 - gone)
        };
        self.age += 1;
        if self.age >= self.attack + self.hold + self.release || !self.lp2.is_finite() {
            self.active = false;
            self.lp1 = 0.0;
            self.lp2 = 0.0;
            return 0.0;
        }
        self.lp2 * env * self.amp
    }
}

/// The bass: a sine with a touch of its own second partial, which is all the
/// definition a low note needs to be heard through a pad.
struct BassVoice {
    active: bool,
    o1: Osc,
    o2: Osc,
    env: f32,
    dec: f32,
    amp: f32,
    age: u32,
    attack: u32,
    life: u32,
    fade: u32,
}

impl BassVoice {
    fn silent() -> Self {
        Self {
            active: false,
            o1: Osc::default(),
            o2: Osc::default(),
            env: 0.0,
            dec: 0.0,
            amp: 0.0,
            age: 0,
            attack: 1,
            life: 0,
            fade: 1,
        }
    }

    fn strike(&mut self, freq: f32, amp: f32, t60: f32, attack: f32) {
        let freq = freq.clamp(20.0, 500.0);
        self.o1.set(freq);
        self.o2.set(freq * 2.0);
        self.env = 1.0;
        self.dec = (-6.907_755 / (t60.max(0.1) * SR)).exp();
        self.amp = amp;
        self.age = 0;
        self.attack = ((attack * SR) as u32).max(1);
        self.life = (t60 * SR) as u32;
        self.fade = (0.05 * SR) as u32;
        self.active = true;
    }

    #[inline]
    fn step(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }
        self.env *= self.dec;
        // The second partial is normalised away again, so `amp` means the same
        // thing here as it does on the bell.
        let mut out = (self.o1.step() + 0.22 * self.o2.step()) / 1.22 * self.env * self.amp;
        if self.age < self.attack {
            out *= raised_cosine(self.age as f32 / self.attack as f32);
        }
        if self.age + self.fade >= self.life {
            let left = self.life.saturating_sub(self.age) as f32 / self.fade as f32;
            out *= raised_cosine(left);
        }
        self.age += 1;
        if self.age >= self.life || !out.is_finite() {
            self.active = false;
            return 0.0;
        }
        out
    }
}

// -- Karplus-Strong -------------------------------------------

/// Ring size, a power of two so the wrap is a mask.
const RING: usize = 512;
const RING_MASK: usize = RING - 1;

/// How long the excitation burst is, in periods of the note.
///
/// Measured in periods rather than in milliseconds, and that is the point: a
/// delay line is a resonator, so a burst of fixed duration fills a short line
/// many more times than a long one and a bass note comes out ten decibels below
/// a treble note struck with identical force.
const EXCITE_PERIODS: f32 = 1.6;

/// One plucked string: a delay line, a two-tap loop filter, and a loop gain.
///
/// Kept from the previous score and demoted to the arpeggio, where a real
/// physical model is worth its dozen operations a sample: nothing additive
/// sounds like a string, and the off-beat plucks are the only percussive thing
/// in the piece.
struct KsString {
    buf: [f32; RING],
    write: usize,
    delay_int: usize,
    delay_frac: f32,
    filt: f32,
    damp: f32,
    rho: f32,
    age: u32,
    life: u32,
    fade: u32,
    excite_left: u32,
    excite_len: u32,
    excite_amp: f32,
    excite_lp: f32,
    excite_hp: f32,
    excite_a: f32,
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

    #[allow(clippy::too_many_arguments)]
    fn pluck(&mut self, freq: f32, amp: f32, t60: f32, damp: f32, bright_hz: f32, attack: f32, seed: u64) {
        let freq = freq.clamp(MIN_HZ, 4000.0);
        let damp = damp.clamp(0.02, 0.49);
        let delay = (SR / freq - damp).clamp(2.0, (RING - 3) as f32);
        self.delay_int = delay.floor() as usize;
        self.delay_frac = delay - self.delay_int as f32;
        self.damp = damp;
        self.rho = (-6.907_755 / (freq * t60.max(0.2))).exp().clamp(0.0, 0.9999);
        self.age = 0;
        self.life = (t60 * SR) as u32;
        self.fade = (0.04 * SR) as u32;
        self.excite_len = ((delay * EXCITE_PERIODS) as u32).clamp(24, 900);
        self.excite_left = self.excite_len;
        self.excite_amp = amp;
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
        let y = x + (self.filt - x) * self.damp;
        self.filt = x;
        let mut v = y * self.rho;

        if self.excite_left > 0 {
            let k = self.excite_len - self.excite_left;
            let t = k as f32 / self.excite_len as f32;
            // A finger releasing, not a hammer.
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
        out
    }
}

// -- Space ----------------------------------------------------

/// Delay line length, a power of two. 16384 samples is 743 ms at 22.05 kHz,
/// comfortably over the longest tap the tempo can ask for.
const DELAY_RING: usize = 16_384;
const DELAY_MASK: usize = DELAY_RING - 1;

/// One feedback delay, taking the place of a reverb.
///
/// A real reverb is a bank of allpasses and comb filters and costs more than
/// the rest of this file put together. A single eighth-note tap at a third
/// feedback, low-passed so the repeats get darker, does the one thing a sparse
/// piece actually needs: it stops each note ending in a hole.
struct Delay {
    buf: Vec<f32>,
    write: usize,
    taps: usize,
    lp: f32,
}

impl Delay {
    fn new() -> Self {
        // One eighth note, so the repeats fall on the offbeats the arpeggio is
        // already using rather than smearing across them.
        let taps = ((SR * 30.0 / BPM) as usize).clamp(64, DELAY_RING - 2);
        Self { buf: vec![0.0; DELAY_RING], write: 0, taps, lp: 0.0 }
    }

    fn clear(&mut self) {
        self.buf.iter_mut().for_each(|s| *s = 0.0);
        self.write = 0;
        self.lp = 0.0;
    }

    #[inline]
    fn step(&mut self, x: f32, feedback: f32, cut: f32) -> f32 {
        let read = (self.write + DELAY_RING - self.taps) & DELAY_MASK;
        let echo = self.buf[read];
        self.lp += (echo - self.lp) * cut;
        let mut into = x + self.lp * feedback;
        if !into.is_finite() {
            into = 0.0;
            self.lp = 0.0;
        }
        self.buf[self.write] = into;
        self.write = (self.write + 1) & DELAY_MASK;
        echo
    }
}

// -- Playback -------------------------------------------------

/// How many pieces a world gets, and therefore how long it takes a cue to come
/// round again. Four is about seventeen minutes of distinct music, and with
/// three to eight minutes of silence between cues that is the better part of an
/// hour before a player could hear the same piece twice.
const PIECES: usize = 4;

/// Simultaneous voices of each kind.
///
/// Sized from the score rather than guessed at. The melody is one line but its
/// notes ring for three seconds over a one-second harmonic rhythm, so four or
/// five overlap; the pad holds four voices and releases four more under the
/// next chord; the bass has two a bar. Every steal truncates something still
/// sounding, which is both a click and the reason a wash never builds.
const BELLS: usize = 12;
const PADS: usize = 12;
const BASSES: usize = 4;
const STRINGS: usize = 8;

/// How hard the score drives its output before the mixer sees it.
///
/// The one calibration constant in the file. Loudness is decided in the mixer's
/// gain table (see [`super::mixer::gain`]) and every other synthesised sound in
/// the game is normalised to a fixed peak before it gets there; this is the
/// score's equivalent, measured so that the densest passage of the warmest
/// reading peaks near `0.65` -- the same order as a baked clip's normalised
/// `0.85` -- and `gain::MUSIC` therefore means what every other number in that
/// table means.
const DRIVE: f32 = 0.88;

/// Per-part synthesis settings that do not depend on the reading.
struct Timbre {
    t60: f32,
    attack: f32,
}

fn timbre(part: Part) -> Timbre {
    match part {
        // Long enough that a quarter note at 88 BPM is still sounding when the
        // next two arrive, which is what turns a series of notes into a line.
        Part::Keys => Timbre { t60: 3.0, attack: 0.012 },
        Part::Bass => Timbre { t60: 1.9, attack: 0.020 },
        Part::Pluck => Timbre { t60: 2.2, attack: 0.014 },
        Part::Pad => Timbre { t60: 0.0, attack: 0.9 },
    }
}

/// The score, ready to play: four pieces, the voices, and where we are.
pub struct Score {
    pieces: Vec<Piece>,
    piece: usize,
    len: u32,
    pos: u32,
    next: usize,
    bells: Vec<Bell>,
    pads: Vec<PadVoice>,
    basses: Vec<BassVoice>,
    strings: Vec<KsString>,
    delay: Delay,
    cue: u32,
    seed: u64,
    /// Rotating counter used to seed each pluck, so a note's excitation noise is
    /// the same every time the piece comes round.
    struck: u64,
    out_lp: f32,
    dc: f32,
}

impl Score {
    pub fn new(seed: u64) -> Self {
        let pieces: Vec<Piece> = (0..PIECES as u32).map(|v| compose(seed, v)).collect();
        let len = pieces[0].len;
        Self {
            pieces,
            piece: 0,
            len,
            pos: 0,
            next: 0,
            bells: (0..BELLS).map(|_| Bell::silent()).collect(),
            pads: (0..PADS).map(|_| PadVoice::silent()).collect(),
            basses: (0..BASSES).map(|_| BassVoice::silent()).collect(),
            strings: (0..STRINGS).map(|_| KsString::silent()).collect(),
            delay: Delay::new(),
            cue: 0,
            seed,
            struck: 0,
            out_lp: 0.0,
            dc: 0.0,
        }
    }

    /// Length of a piece in seconds. Read by the tests and by the cue director.
    #[allow(dead_code)]
    pub fn secs(&self) -> f32 {
        self.len as f32 / SR
    }

    #[allow(dead_code)]
    pub fn events(&self) -> &[Event] {
        &self.pieces[self.piece].events
    }

    /// Which piece a cue number plays. Consecutive cues are different pieces,
    /// which is the whole of "the same music must not arrive twice running".
    pub fn piece_for(cue: u32) -> usize {
        (cue.max(1) as usize - 1) % PIECES
    }

    fn rewind(&mut self, piece: usize) {
        self.piece = piece.min(self.pieces.len() - 1);
        self.len = self.pieces[self.piece].len;
        self.pos = 0;
        self.next = 0;
        self.struck = 0;
        // Nothing from the last cue may survive into this one. A ringing tail
        // under a new key is the one artefact a listener always notices.
        for bell in self.bells.iter_mut() {
            bell.active = false;
        }
        for pad in self.pads.iter_mut() {
            pad.active = false;
        }
        for bass in self.basses.iter_mut() {
            bass.active = false;
        }
        for string in self.strings.iter_mut() {
            string.active = false;
        }
        self.delay.clear();
        self.out_lp = 0.0;
        self.dc = 0.0;
    }

    /// Take a bell, preferring an idle one and otherwise the most decayed.
    fn take_bell(&mut self) -> usize {
        let mut best = 0usize;
        let mut spent = -1.0f32;
        for (i, v) in self.bells.iter().enumerate() {
            if !v.active {
                return i;
            }
            let done = v.age as f32 / v.life.max(1) as f32;
            if done >= spent {
                spent = done;
                best = i;
            }
        }
        best
    }

    fn take_pad(&mut self) -> usize {
        let mut best = 0usize;
        let mut spent = -1.0f32;
        for (i, v) in self.pads.iter().enumerate() {
            if !v.active {
                return i;
            }
            let total = v.attack + v.hold + v.release;
            let done = v.age as f32 / total.max(1) as f32;
            if done >= spent {
                spent = done;
                best = i;
            }
        }
        best
    }

    fn take_bass(&mut self) -> usize {
        let mut best = 0usize;
        let mut spent = -1.0f32;
        for (i, v) in self.basses.iter().enumerate() {
            if !v.active {
                return i;
            }
            let done = v.age as f32 / v.life.max(1) as f32;
            if done >= spent {
                spent = done;
                best = i;
            }
        }
        best
    }

    fn take_string(&mut self) -> usize {
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

    fn trigger(&mut self, event: Event, warmth: f32, dusk: f32) {
        let t = timbre(event.part);
        let freq = C2_HZ * (event.pitch / 12.0).exp2();
        self.struck = self.struck.wrapping_add(1);
        let seed = self.seed ^ self.struck.wrapping_mul(0x9e37_79b9_7f4a_7c15);

        match event.part {
            Part::Keys => {
                // Warmth opens the upper partials; dusk closes them and slows
                // the onset. Neither changes a note, only the light on it.
                let tilt = lerp(0.55, 1.0, warmth) * lerp(1.0, 0.46, dusk);
                let t60 = t.t60 * lerp(0.92, 1.12, warmth) * lerp(1.0, 1.15, dusk);
                let attack = t.attack * lerp(1.0, 1.8, dusk);
                let slot = self.take_bell();
                self.bells[slot].strike(freq, event.amp, t60, attack, tilt);
            }
            Part::Bass => {
                let t60 = t.t60 * lerp(1.0, 1.2, dusk);
                let slot = self.take_bass();
                self.basses[slot].strike(freq, event.amp, t60, t.attack);
            }
            Part::Pad => {
                let hold = (event.dur as f32 / SR - t.attack).max(0.1);
                let cutoff = lerp(900.0, 2100.0, warmth) * lerp(1.0, 0.42, dusk);
                let attack = t.attack * lerp(1.0, 1.6, dusk);
                let slot = self.take_pad();
                self.pads[slot].start(freq, event.amp, hold, attack, 1.6, cutoff, 0.004);
            }
            Part::Pluck => {
                let damp = (0.26 + (1.0 - warmth) * 0.10 + dusk * 0.11).clamp(0.02, 0.49);
                let bright = 2400.0 * lerp(0.62, 1.0, warmth) * lerp(1.0, 0.75, dusk);
                let slot = self.take_string();
                self.strings[slot].pluck(freq, event.amp, t.t60, damp, bright, t.attack, seed);
            }
        }
    }

    /// One sample.
    ///
    /// `cue` is the director's cue number: `0` is silence, and any change means
    /// "start the next piece from the top". `warmth` is how the network is
    /// doing, `density` how much of the score survives the thinning, `dusk` the
    /// evening reading.
    pub fn step(&mut self, cue: u32, warmth: f32, density: f32, dusk: f32) -> f32 {
        if cue == 0 {
            if self.cue != 0 {
                self.cue = 0;
                self.rewind(self.piece);
            }
            return 0.0;
        }
        if cue != self.cue {
            self.cue = cue;
            self.rewind(Self::piece_for(cue));
        }

        // Everything below the gate is filigree; the pad, the downbeat and the
        // motif are always above it.
        let gate = 235.0 - 215.0 * density.clamp(0.0, 1.0);

        while self.next < self.pieces[self.piece].events.len()
            && self.pieces[self.piece].events[self.next].at <= self.pos
        {
            let event = self.pieces[self.piece].events[self.next];
            self.next += 1;
            if (event.weight as f32) >= gate {
                self.trigger(event, warmth, dusk);
            }
        }
        self.pos += 1;
        if self.pos >= self.len {
            // A cue is shorter than a piece, so this is rare; when it does
            // happen the outro's tonic runs straight into the intro's pad,
            // which is a join rather than a splice.
            self.pos = 0;
            self.next = 0;
            self.struck = 0;
        }

        let mut sum = 0.0;
        for bell in self.bells.iter_mut() {
            sum += bell.step();
        }
        for pad in self.pads.iter_mut() {
            sum += pad.step();
        }
        for bass in self.basses.iter_mut() {
            sum += bass.step();
        }
        for string in self.strings.iter_mut() {
            sum += string.step();
        }
        sum *= DRIVE;

        // The space. Dusk lets the repeats run a little longer and darker,
        // which is most of what "evening" means here.
        let feedback = lerp(0.30, 0.38, dusk);
        let cut = pole_coeff(lerp(2600.0, 1500.0, dusk), SR);
        let echo = self.delay.step(sum, feedback, cut);
        let mixed = sum + echo * lerp(0.22, 0.28, dusk);

        // One shared low-pass for the whole instrument: a body, and the "warmer
        // when thriving" move in one coefficient.
        let cut = lerp(2400.0, 5200.0, warmth) * lerp(1.0, 0.42, dusk);
        let a = pole_coeff(cut, SR);
        self.out_lp += (mixed - self.out_lp) * a;
        // And a 20 Hz high-pass, because a bass sine should not deliver a DC
        // offset to somebody's speaker cone.
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
    /// Seeds every structural claim is checked against. One seed proves a
    /// generator can do a thing once; a spread proves it always does.
    const SEEDS: [u64; 8] = [0, 1, 42, 97, 1234, 90210, 0xdead_beef, 0xffff_ffff_ffff];

    fn pieces(seed: u64) -> Vec<Piece> {
        (0..PIECES as u32).map(|v| compose(seed, v)).collect()
    }

    fn melody(piece: &Piece) -> Vec<&Event> {
        piece.events.iter().filter(|e| e.part == Part::Keys).collect()
    }

    /// A piece's melody read back in scale degrees of its own key, in order.
    fn melody_degrees(piece: &Piece) -> Vec<(i32, f32)> {
        let root = KEYS[piece.key].root;
        melody(piece)
            .iter()
            .map(|e| {
                (
                    degree_of(e.pitch.round() as i32 - root),
                    e.at as f32 / beat_len(),
                )
            })
            .collect()
    }

    // -- harmony -----------------------------------------------

    #[test]
    fn every_chord_is_spelt_from_the_key_it_claims() {
        // A borrowed chord bends exactly one degree by exactly one semitone.
        // Anything else is a wrong note wearing a chord symbol.
        for spec in PALETTE {
            let mut classes = Vec::new();
            for t in spec.tones {
                classes.push(degree_semitone(spec.root + t, spec.alter).rem_euclid(12));
            }
            classes.sort_unstable();
            classes.dedup();
            assert!(
                classes.len() >= 3,
                "{} sounds only {} pitch classes",
                spec.name,
                classes.len()
            );
            if spec.alter.0 >= 0 {
                let natural = MAJOR[spec.alter.0 as usize];
                assert_eq!(
                    natural - spec.alter.1,
                    1,
                    "{} bends degree {} by more than a semitone",
                    spec.name,
                    spec.alter.0
                );
            }
        }
    }

    #[test]
    fn every_tonic_and_subdominant_carries_a_colour_tone() {
        // The added second above the third is the shimmer this score is after,
        // and a bare triad is the one voicing that must never appear.
        for spec in PALETTE {
            if !matches!(spec.role, Role::Tonic | Role::Subdominant) {
                continue;
            }
            let extra = spec.tones.iter().any(|t| !matches!(t, 0 | 2 | 4));
            assert!(extra, "{} is a bare triad", spec.name);
        }
    }

    #[test]
    fn harmony_actually_moves() {
        // The old score sat on one pedal for the whole theme. This one has to
        // go somewhere: distinct roots, real bass motion, and a chord that
        // changes at a rate the ear reads as a progression.
        for seed in SEEDS {
            for (v, piece) in pieces(seed).into_iter().enumerate() {
                let mut roots: Vec<i32> = piece
                    .chords
                    .iter()
                    .map(|c| c.bass.rem_euclid(12))
                    .collect();
                roots.sort_unstable();
                roots.dedup();
                // Measured: five to seven per piece, against the old score's
                // four, three of which were the same pedal D.
                assert!(
                    roots.len() >= 4,
                    "seed {seed} piece {v} uses only {} distinct roots",
                    roots.len()
                );
                let changes = piece
                    .chords
                    .windows(2)
                    .filter(|w| w[0].spec != w[1].spec)
                    .count();
                assert!(
                    changes >= piece.chords.len() / 2,
                    "seed {seed} piece {v} changes chord only {changes} times in {} bars",
                    piece.chords.len()
                );
                let bar = BEATS_PER_BAR * 60.0 / BPM;
                assert!(
                    (2.0..6.0).contains(&bar),
                    "a bar is {bar:.1} s, which is not a harmonic rhythm"
                );
            }
        }
    }

    #[test]
    fn every_section_ends_on_a_cadence() {
        // A section that stops is not a section that ends. The last bar of each
        // one is a tonic, and the bar before it prepares that tonic - authentic
        // or plagal, never a colour chord pretending to be an ending. The intro
        // is the deliberate exception: it hands over on the dominant.
        for seed in SEEDS {
            for (v, piece) in pieces(seed).into_iter().enumerate() {
                for (s, &(bar0, bars)) in piece.sections.iter().enumerate() {
                    let last = piece.chords[bar0 + bars - 1];
                    let before = piece.chords[bar0 + bars - 2];
                    let label = FORM[s].label;
                    if label == "intro" {
                        assert_eq!(
                            last.role(),
                            Role::Dominant,
                            "seed {seed} piece {v}: the intro ends on {} rather than on the dominant",
                            last.name()
                        );
                        continue;
                    }
                    assert_eq!(
                        last.role(),
                        Role::Tonic,
                        "seed {seed} piece {v} section {label} ends on {}",
                        last.name()
                    );
                    assert!(
                        matches!(before.role(), Role::Dominant | Role::Subdominant),
                        "seed {seed} piece {v} section {label}: {} -> {} is not a cadence",
                        before.name(),
                        last.name()
                    );
                }
            }
        }
    }

    #[test]
    fn voices_lead_rather_than_leap() {
        // Two claims, and the difference between them is the honest one.
        //
        // *No voice leaps past a minor seventh* is absolute, and
        // [`the_part_writing_rarely_has_to_give_anything_up`] pins the far
        // tighter thing underneath it: ninety-seven per cent of changes keep
        // every voice inside a fifth. A pad whose inner parts jump is heard as
        // a chord change rather than as part writing.
        //
        // *A common tone is held* is a rate rather than a rule, because it
        // cannot always be had. `iii7 -> IVmaj7` under a bass moving up a step
        // has no voicing that both keeps a common tone and avoids a parallel
        // fifth with the bass; something has to give, and the parallel is the
        // more audible fault of the two.
        let mut held = 0;
        let mut changes = 0;
        for seed in SEEDS {
            for (v, piece) in pieces(seed).into_iter().enumerate() {
                for pair in piece.chords.windows(2) {
                    let (from, to) = (pair[0], pair[1]);
                    if from.spec == to.spec {
                        continue;
                    }
                    let moves: Vec<i32> =
                        (0..4).map(|i| to.voices[i] - from.voices[i]).collect();
                    let worst = moves.iter().map(|m| m.abs()).max().unwrap_or(0);
                    assert!(
                        worst <= 10,
                        "seed {seed} piece {v}: {} -> {} moved a voice {worst} semitones",
                        from.name(),
                        to.name()
                    );
                    changes += 1;
                    if moves.contains(&0) || worst <= 3 {
                        held += 1;
                    }
                }
            }
        }
        assert!(changes > 400, "only {changes} chord changes to judge");
        let rate = held as f32 / changes as f32;
        assert!(
            rate > 0.80,
            "only {:.0}% of chord changes hold a common tone or move by a step or two",
            rate * 100.0
        );
    }

    #[test]
    fn the_part_writing_rarely_has_to_give_anything_up() {
        // [`voice_chord`] searches in tiers and records which one answered, so
        // "the rules are kept" is a distribution rather than an intention. Tier
        // 0 is the textbook answer; anything from tier 4 up has abandoned the
        // motion limit and must stay a rounding error.
        let mut tally = [0usize; 7];
        for seed in SEEDS {
            for piece in pieces(seed) {
                for chord in &piece.chords {
                    tally[chord.tier as usize] += 1;
                }
            }
        }
        let total: usize = tally.iter().sum();
        // Measured: [2821, 56, 33, 24, 10, 0, 0] over eight seeds. Nineteen out
        // of twenty chords are the textbook answer, nothing at all needs a
        // parallel, and the one bar in three hundred that has to leap wide is
        // the price of never writing one.
        assert_eq!(tally[5], 0, "a chord was voiced with a parallel: {tally:?}");
        assert_eq!(tally[6], 0, "the search ran out of voicings entirely: {tally:?}");
        let textbook = (tally[0] + tally[1]) as f32 / total as f32;
        assert!(
            textbook > 0.90,
            "only {:.1}% of chords are led within a fourth: {tally:?}",
            textbook * 100.0
        );
        let tidy = (tally[0] + tally[1] + tally[2] + tally[3]) as f32 / total as f32;
        assert!(
            tidy > 0.97,
            "only {:.1}% of chords keep every voice inside a fifth: {tally:?}",
            tidy * 100.0
        );
    }

    #[test]
    fn no_parallel_fifths_or_octaves_anywhere() {
        // The single most audible sign that chords were stacked rather than
        // led. Checked between every pair of upper voices and between every
        // upper voice and the bass, for every consecutive pair of chords in
        // every piece of every seed - which is a far stronger claim than the
        // old fixed table could make, because the voicings here are searched
        // for rather than written down.
        for seed in SEEDS {
            for (v, piece) in pieces(seed).into_iter().enumerate() {
                for pair in piece.chords.windows(2) {
                    let (from, to) = (pair[0], pair[1]);
                    assert!(
                        !parallel(from.voices, to.voices, from.bass, to.bass),
                        "seed {seed} piece {v}: {} -> {} moves in parallel fifths or octaves",
                        from.name(),
                        to.name()
                    );
                }
            }
        }
    }

    #[test]
    fn the_registers_do_not_overlap_into_mud() {
        for seed in SEEDS {
            for piece in pieces(seed) {
                for chord in &piece.chords {
                    assert!(
                        (BASS_LO..=BASS_HI).contains(&chord.bass),
                        "{} put the bass at {}",
                        chord.name(),
                        chord.bass
                    );
                    assert!(chord.voices[0] >= PAD_LO, "{} dropped the pad into the bass", chord.name());
                    assert!(chord.voices[3] <= PAD_HI, "{} pushed the pad into the tune", chord.name());
                    for w in chord.voices.windows(2) {
                        assert!(w[0] < w[1], "{} has crossed voices", chord.name());
                    }
                }
            }
        }
    }

    // -- melody ------------------------------------------------

    #[test]
    fn register_is_bright() {
        // The playtest complaint, as a number. The old score's melody sat
        // around D3; this one has to have its median at C4 or above, and to
        // spend most of its life between C4 and C6.
        for seed in SEEDS {
            for (v, piece) in pieces(seed).into_iter().enumerate() {
                let mut pitches: Vec<f32> = melody(&piece).iter().map(|e| e.pitch).collect();
                assert!(pitches.len() > 40, "seed {seed} piece {v} has no tune to judge");
                pitches.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let median = pitches[pitches.len() / 2];
                // Measured: the median sits between G4 and C#5 in every piece
                // of every seed. The bar is C4 because that is the claim; the
                // margin is what says the register was designed rather than
                // clamped.
                assert!(
                    median >= C4 as f32,
                    "seed {seed} piece {v}: the median melody note is {median} semitones above C2, below C4"
                );
                let lowest = pitches[0];
                let highest = pitches[pitches.len() - 1];
                assert!(lowest >= (C4 - 3) as f32, "the tune fell to {lowest}");
                assert!(highest <= 48.0, "the tune rose to {highest}, above C6");
                // And it is not one note: a bright drone is still a drone.
                assert!(
                    highest - lowest >= 7.0,
                    "seed {seed} piece {v} spans only {} semitones",
                    highest - lowest
                );
            }
        }
    }

    #[test]
    fn the_melody_steps_far_more_often_than_it_leaps() {
        // Measured in scale degrees, so a "step" is a second and a third is
        // already a leap - a stricter reading than the semitone version this
        // replaces. A uniform draw over a scale gives about a fifth of its
        // intervals as seconds and is instantly recognisable as such.
        // Measured: 72% over eight seeds, worst seed 60%.
        for seed in SEEDS {
            let mut steps = 0;
            let mut leaps = 0;
            for piece in pieces(seed) {
                let notes = melody_degrees(&piece);
                for w in notes.windows(2) {
                    // Skip the gap between phrases: the interval across a rest
                    // is not a melodic interval, it is where the next line
                    // starts.
                    if w[1].1 - w[0].1 > 2.5 {
                        continue;
                    }
                    let d = (w[1].0 - w[0].0).abs();
                    if d == 0 {
                        continue;
                    }
                    if d <= 1 {
                        steps += 1;
                    } else {
                        leaps += 1;
                    }
                }
            }
            let ratio = steps as f32 / (steps + leaps).max(1) as f32;
            assert!(
                ratio > 0.55,
                "seed {seed}: only {:.0}% of the melody moves by step - that is a random walk",
                ratio * 100.0
            );
            assert!(leaps > 0, "a melody with no leap at all has no gesture");
        }
    }

    #[test]
    fn strong_beats_take_chord_tones() {
        for seed in SEEDS {
            for (v, piece) in pieces(seed).into_iter().enumerate() {
                let root = KEYS[piece.key].root;
                let mut on = 0;
                let mut total = 0;
                for event in melody(&piece) {
                    let beat = event.at as f32 / beat_len();
                    // Rounded onsets: `Event::at` is a truncated sample index,
                    // so a note written on beat 8 reads back as 7.99997.
                    let snapped = (beat * 2.0).round() / 2.0;
                    if !is_strong(snapped) {
                        continue;
                    }
                    let bar = (snapped / BEATS_PER_BAR) as usize;
                    let chord = piece.chords[bar.min(piece.chords.len() - 1)];
                    let degree = degree_of(event.pitch.round() as i32 - root);
                    total += 1;
                    if chord.is_chord_tone(degree) {
                        on += 1;
                    }
                }
                assert!(total > 15, "seed {seed} piece {v}: only {total} strong-beat notes");
                // Measured: 97% over eight seeds, worst piece 95%. The gap is
                // motif statements, which are transposed as a whole rather than
                // snapped note by note - a motif whose intervals were edited to
                // fit the chord under it is not the same motif any more.
                let rate = on as f32 / total as f32;
                assert!(
                    rate > 0.90,
                    "seed {seed} piece {v}: only {:.0}% of strong beats land on a chord tone",
                    rate * 100.0
                );
            }
        }
    }

    #[test]
    fn every_phrase_breathes_before_the_next_one() {
        // Silence is part of the phrasing, and C418's is most of the charm.
        // Each four-bar phrase that sings at all must leave at least two beats
        // with no new attack at its end.
        // Windowed in samples rather than in beats: `Event::at` is a truncated
        // sample index, so a note written on beat 16 reads back as 15.99997 and
        // a beat-space comparison files the next phrase's first note under the
        // end of this one.
        for seed in SEEDS {
            for (v, piece) in pieces(seed).into_iter().enumerate() {
                let span = BARS_PER_PHRASE as f32 * BEATS_PER_BAR;
                let phrases = piece.chords.len() / BARS_PER_PHRASE as usize;
                let mut sung = 0;
                for p in 0..phrases {
                    let (lo, hi) = (at_beat(p as f32 * span), at_beat((p + 1) as f32 * span));
                    let last = melody(&piece)
                        .iter()
                        .filter(|e| e.at >= lo && e.at < hi)
                        .map(|e| (e.at - lo) as f32 / beat_len())
                        .fold(f32::MIN, f32::max);
                    if last == f32::MIN {
                        continue;
                    }
                    sung += 1;
                    assert!(
                        span - last >= 2.0,
                        "seed {seed} piece {v} phrase {p} sings to within {:.1} beats of the next",
                        span - last
                    );
                }
                assert!(sung >= 10, "seed {seed} piece {v} only sings in {sung} phrases");
            }
        }
    }

    // -- motif -------------------------------------------------

    /// Interval signature of a run of notes, in scale degrees.
    fn signature(notes: &[MotifNote]) -> Vec<i32> {
        notes.windows(2).map(|w| w[1].degree - w[0].degree).collect()
    }

    #[test]
    fn motif_recurs_recognisably() {
        // The claim the whole rewrite rests on: a listener has to hear material
        // come *back*. A recurrence is counted when a run of consecutive melody
        // notes has the motif's interval signature - which survives diatonic
        // transposition, octave displacement and rhythmic augmentation, and is
        // exactly what an ear recognises a tune by.
        for seed in SEEDS {
            for (v, piece) in pieces(seed).into_iter().enumerate() {
                let notes = melody_degrees(&piece);
                let degrees: Vec<i32> = notes.iter().map(|(d, _)| *d).collect();
                for (m, motif) in piece.motifs.iter().enumerate() {
                    let want = signature(motif);
                    assert!(
                        want.len() >= 2,
                        "seed {seed} piece {v} motif {m} is only {} notes",
                        motif.len()
                    );
                    let hits = degrees
                        .windows(want.len() + 1)
                        .filter(|w| {
                            w.windows(2)
                                .map(|p| p[1] - p[0])
                                .eq(want.iter().copied())
                        })
                        .count();
                    let inverted = degrees
                        .windows(want.len() + 1)
                        .filter(|w| {
                            w.windows(2)
                                .map(|p| p[1] - p[0])
                                .eq(want.iter().map(|i| -i))
                        })
                        .count();
                    // Measured: four to thirty-two exact returns per motif per
                    // piece, mean twelve. Three is the floor, not the target.
                    assert!(
                        hits >= 3,
                        "seed {seed} piece {v}: motif {m} {want:?} returns only {hits} times \
                         ({inverted} inverted) in {} notes",
                        degrees.len()
                    );
                }
            }
        }
    }

    #[test]
    fn a_motif_is_short_and_singable() {
        for seed in SEEDS {
            for piece in pieces(seed) {
                for motif in &piece.motifs {
                    assert!(
                        (3..=6).contains(&motif.len()),
                        "a motif of {} notes is not a motif",
                        motif.len()
                    );
                    let lo = motif.iter().map(|n| n.degree).min().unwrap();
                    let hi = motif.iter().map(|n| n.degree).max().unwrap();
                    assert!(hi - lo <= 7, "a motif spanning {} degrees is not sung", hi - lo);
                    let span = motif
                        .iter()
                        .map(|n| n.onset + n.len)
                        .fold(0.0f32, f32::max);
                    assert!(span <= 2.0 * BEATS_PER_BAR + 0.01, "a motif of {span} beats is a phrase");
                }
            }
        }
    }

    #[test]
    fn a_repeated_phrase_is_rare() {
        // A motif must return; a whole phrase mostly must not. The variation
        // schedule and the freely regenerated tail after every statement are
        // what buy that. Stated as a rate rather than as an absolute, because
        // two short tails over the same harmony *can* coincide and a generator
        // that forbade it would be lying about how it works - but if it starts
        // happening often, the returns have stopped being variations.
        let mut repeats = 0;
        let mut total = 0;
        for seed in SEEDS {
            for (v, piece) in pieces(seed).into_iter().enumerate() {
                let span = BARS_PER_PHRASE as f32 * BEATS_PER_BAR;
                let phrases = piece.chords.len() / BARS_PER_PHRASE as usize;
                let mut seen: Vec<(usize, Vec<(i32, i32)>)> = Vec::new();
                for p in 0..phrases {
                    let (lo, hi) = (at_beat(p as f32 * span), at_beat((p + 1) as f32 * span));
                    let shape: Vec<(i32, i32)> = melody(&piece)
                        .iter()
                        .filter(|e| e.at >= lo && e.at < hi)
                        .map(|e| {
                            (
                                e.pitch.round() as i32,
                                ((e.at - lo) as f32 / beat_len() * 2.0).round() as i32,
                            )
                        })
                        .collect();
                    if shape.len() < 3 {
                        continue;
                    }
                    total += 1;
                    if seen.iter().any(|(_, s)| *s == shape) {
                        repeats += 1;
                    }
                    seen.push((p, shape));
                }
                assert!(
                    seen.len() >= 8,
                    "seed {seed} piece {v} has only {} phrases to compare",
                    seen.len()
                );
            }
        }
        assert!(total > 400, "only {total} phrases to judge");
        let rate = repeats as f32 / total as f32;
        assert!(
            rate < 0.06,
            "{:.0}% of phrases are note-for-note copies of an earlier one",
            rate * 100.0
        );
    }

    // -- pulse and form ----------------------------------------

    #[test]
    fn there_is_a_pulse_and_the_tune_syncopates_against_it() {
        // The bass is exactly on the grid and the melody is not. That contrast
        // is the difference between calm and inert, and it is measurable: every
        // bass note lands on a beat, and a real share of melody notes do not.
        for seed in SEEDS {
            for (v, piece) in pieces(seed).into_iter().enumerate() {
                let mut off_grid = 0;
                let mut total = 0;
                for event in melody(&piece) {
                    let beat = event.at as f32 / beat_len();
                    let snapped = (beat * 2.0).round() / 2.0;
                    total += 1;
                    if (snapped - snapped.round()).abs() > 0.2 {
                        off_grid += 1;
                    }
                }
                let rate = off_grid as f32 / total.max(1) as f32;
                assert!(
                    rate > 0.05,
                    "seed {seed} piece {v}: only {:.0}% of the tune is off the beat",
                    rate * 100.0
                );
                assert!(
                    rate < 0.6,
                    "seed {seed} piece {v}: {:.0}% off the beat is not syncopation, it is drift",
                    rate * 100.0
                );
                for event in piece.events.iter().filter(|e| e.part == Part::Bass) {
                    let beat = event.at as f32 / beat_len();
                    let snapped = (beat * 2.0).round() / 2.0;
                    assert!(
                        (snapped - snapped.round()).abs() < 0.01,
                        "the bass landed off the beat at {beat}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_tempo_and_the_length_are_what_the_director_expects() {
        assert!((80.0..=100.0).contains(&BPM), "the tempo must be a walk");
        assert_eq!(BEATS_PER_BAR, 4.0, "the pulse needs a downbeat");
        let secs = compose(SEED, 0).len as f32 / SR;
        assert!(
            (150.0..=300.0).contains(&secs),
            "a piece is {secs:.0} s - a cue is three to five minutes"
        );
        // Every piece is the same length, because the cue director sizes its
        // envelope from one of them.
        for v in 1..PIECES as u32 {
            assert_eq!(compose(SEED, v).len, compose(SEED, 0).len);
        }
    }

    #[test]
    fn the_form_opens_with_a_pad_and_thins_back_to_one() {
        for seed in SEEDS {
            for (v, piece) in pieces(seed).into_iter().enumerate() {
                let (_, intro_bars) = piece.sections[0];
                let intro_end = at_beat(intro_bars as f32 * BEATS_PER_BAR);
                let opening: Vec<Part> = piece
                    .events
                    .iter()
                    .filter(|e| e.at < intro_end)
                    .map(|e| e.part)
                    .collect();
                assert!(
                    !opening.contains(&Part::Keys),
                    "seed {seed} piece {v}: the tune arrives in the intro"
                );
                assert!(opening.contains(&Part::Pad), "the intro has no pad");

                let (out0, out_bars) = *piece.sections.last().unwrap();
                let out_start = at_beat(out0 as f32 * BEATS_PER_BAR);
                let outro: Vec<Part> = piece
                    .events
                    .iter()
                    .filter(|e| e.at >= out_start)
                    .map(|e| e.part)
                    .collect();
                assert!(
                    !outro.contains(&Part::Pluck),
                    "seed {seed} piece {v}: the arpeggio is still running in the outro"
                );
                assert!(outro.contains(&Part::Pad), "the outro has no pad to end on");
                assert!(out_bars >= 4, "an outro of {out_bars} bars is a stop, not an ending");
            }
        }
    }

    #[test]
    fn thinning_keeps_the_bones_and_drops_the_filigree() {
        let piece = compose(SEED, 0);
        let kept = |density: f32| {
            let gate = 235.0 - 215.0 * density;
            piece
                .events
                .iter()
                .filter(|e| e.weight as f32 >= gate)
                .collect::<Vec<_>>()
        };
        let full = kept(0.95);
        let thin = kept(0.45);
        assert_eq!(full.len(), piece.events.len(), "the warm reading plays everything");
        assert!(
            thin.len() < full.len() * 4 / 5,
            "the sparse reading is barely thinner: {} of {}",
            thin.len(),
            full.len()
        );
        // Whatever is dropped, the piece still has a pad, a bass and a tune.
        for part in [Part::Pad, Part::Bass, Part::Keys] {
            assert!(thin.iter().any(|e| e.part == part), "the thin reading lost {part:?}");
        }
        assert!(
            !thin.iter().any(|e| e.part == Part::Pluck),
            "the arpeggio should be the first thing a thin town loses"
        );
    }

    // -- determinism and variety -------------------------------

    #[test]
    fn determinism() {
        for seed in SEEDS {
            for v in 0..PIECES as u32 {
                let a = compose(seed, v);
                let b = compose(seed, v);
                assert_eq!(a.events.len(), b.events.len());
                for (x, y) in a.events.iter().zip(b.events.iter()) {
                    assert_eq!(x.at, y.at);
                    assert_eq!(x.dur, y.dur);
                    assert_eq!(x.pitch.to_bits(), y.pitch.to_bits());
                    assert_eq!(x.amp.to_bits(), y.amp.to_bits());
                    assert_eq!(x.weight, y.weight);
                }
            }
        }
    }

    #[test]
    fn pieces_differ_across_seeds() {
        // Two worlds must not get the same tune. Compared as event streams, so
        // a piece that differed only in one grace note would still fail.
        let a = compose(SEED, 0);
        for other in [SEED + 1, SEED + 2, 7, 0] {
            let b = compose(other, 0);
            let shared = a
                .events
                .iter()
                .zip(b.events.iter())
                .filter(|(x, y)| x.at == y.at && x.pitch == y.pitch && x.part == y.part)
                .count();
            let overlap = shared as f32 / a.events.len().min(b.events.len()) as f32;
            assert!(
                overlap < 0.5,
                "seeds {SEED} and {other} share {:.0}% of their notes",
                overlap * 100.0
            );
        }
    }

    #[test]
    fn consecutive_cues_differ() {
        // The playtest complaint, structurally: the same music must not arrive
        // twice running. Cues rotate through four pieces, and consecutive ones
        // differ in key, in motif and in progression.
        for seed in SEEDS {
            let all = pieces(seed);
            for cue in 1..=(PIECES as u32 * 2 + 1) {
                let this = &all[Score::piece_for(cue)];
                let next = &all[Score::piece_for(cue + 1)];
                assert_ne!(
                    Score::piece_for(cue),
                    Score::piece_for(cue + 1),
                    "cues {cue} and {} play the same piece",
                    cue + 1
                );
                let same_key = this.key == next.key;
                let same_motif = this.motifs[0] == next.motifs[0];
                assert!(
                    !(same_key && same_motif),
                    "seed {seed}: cues {cue} and {} share both key and motif",
                    cue + 1
                );
            }
            // And across the four, more than one tonal centre is actually used.
            let mut keys: Vec<usize> = all.iter().map(|p| p.key).collect();
            keys.sort_unstable();
            keys.dedup();
            assert!(keys.len() >= 2, "seed {seed} composed all four pieces in one key");
        }
    }

    // -- synthesis ---------------------------------------------

    fn render(secs: f32, warmth: f32, density: f32, dusk: f32) -> Vec<f32> {
        let mut score = Score::new(SEED);
        (0..(secs * SR) as usize)
            .map(|_| score.step(1, warmth, density, dusk))
            .collect()
    }

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|s| (s * s) as f64).sum::<f64>() / buf.len().max(1) as f64).sqrt() as f32
    }

    #[test]
    fn the_bell_holds_its_pitch_and_decays_from_the_top_down() {
        for &want in &[261.63f32, 440.0, 659.26] {
            let mut bell = Bell::silent();
            bell.strike(want, 0.4, 2.5, 0.012, 1.0);
            let n = (SR * 0.5) as usize;
            let buf: Vec<f32> = (0..n).map(|_| bell.step()).collect();
            let body = &buf[(SR * 0.05) as usize..];
            // Autocorrelation around the expected period.
            let period = SR / want;
            let mut best = 0.0f32;
            let mut best_lag = period;
            let lo = (period * 0.8) as usize;
            let hi = ((period * 1.2) as usize).min(body.len() / 3);
            for lag in lo..hi {
                let acc: f32 = (0..body.len() - lag).map(|i| body[i] * body[i + lag]).sum();
                if acc > best {
                    best = acc;
                    best_lag = lag as f32;
                }
            }
            let cents = 1200.0 * ((SR / best_lag) / want).log2();
            assert!(cents.abs() < 35.0, "asked for {want} Hz, got {cents:.0} cents off");
        }

        // The upper partials must fall away faster than the fundamental, which
        // is what makes this a struck instrument rather than an organ. Measured
        // per partial rather than as a treble ratio: a one-pole high-pass leaks
        // most of a 440 Hz fundamental and would report that nothing changed.
        let mut bell = Bell::silent();
        bell.strike(440.0, 0.5, 2.5, 0.012, 1.0);
        let n = (SR * 2.0) as usize;
        let buf: Vec<f32> = (0..n).map(|_| bell.step()).collect();
        let mag = |b: &[f32], hz: f32| -> f64 {
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (i, s) in b.iter().enumerate() {
                let ph = core::f32::consts::TAU * hz * i as f32 / SR;
                re += (*s * ph.cos()) as f64;
                im += (*s * ph.sin()) as f64;
            }
            (re * re + im * im).sqrt() / b.len() as f64
        };
        let ratio = |b: &[f32]| mag(b, 1320.0) / mag(b, 440.0).max(1e-12);
        let early = ratio(&buf[(SR * 0.03) as usize..(SR * 0.20) as usize]);
        let late = ratio(&buf[(SR * 1.10) as usize..(SR * 1.60) as usize]);
        assert!(
            late < early * 0.25,
            "the third partial did not outrun the fundamental: {early:.4} then {late:.4}"
        );
        assert!(early > 0.02, "the third partial was never there: {early:.4}");
    }

    #[test]
    fn the_pad_arrives_over_a_second_and_the_bell_over_milliseconds() {
        // Two genuinely different voices, and the difference is measurable.
        let mut pad = PadVoice::silent();
        pad.start(220.0, 0.2, 2.0, 0.9, 1.6, 1600.0, 0.004);
        let n = (SR * 4.0) as usize;
        let buf: Vec<f32> = (0..n).map(|_| pad.step()).collect();
        assert_eq!(buf[0], 0.0, "a pad must start from silence");
        let window = (0.02 * SR) as usize;
        let env: Vec<f32> = buf.chunks(window).map(rms).collect();
        let loudest = env.iter().fold(0.0f32, |a, s| a.max(*s));
        let half = env.iter().position(|s| *s > loudest * 0.5).unwrap_or(0) as f32 * 0.02;
        assert!(
            (0.25..1.2).contains(&half),
            "the pad reached half power in {half:.2} s"
        );

        let mut bell = Bell::silent();
        bell.strike(440.0, 0.4, 3.0, 0.012, 1.0);
        let n = (SR * 0.3) as usize;
        let buf: Vec<f32> = (0..n).map(|_| bell.step()).collect();
        assert_eq!(buf[0], 0.0, "a bell must start from silence");
        let window = (0.001 * SR) as usize;
        let env: Vec<f32> = buf.chunks(window).map(rms).collect();
        let loudest = env.iter().fold(0.0f32, |a, s| a.max(*s));
        let half = env.iter().position(|s| *s > loudest * 0.5).unwrap_or(0);
        // Milliseconds, not samples: soft enough never to startle on the four
        // hundredth repetition, fast enough to be a struck note.
        assert!((3..40).contains(&half), "the bell reached half power in {half} ms");
    }

    #[test]
    fn the_delay_repeats_and_then_stops() {
        let mut delay = Delay::new();
        let mut out = Vec::new();
        let cut = pole_coeff(2600.0, SR);
        for i in 0..(SR * 3.0) as usize {
            let x = if i < 32 { 1.0 } else { 0.0 };
            out.push(delay.step(x, 0.30, cut));
        }
        let taps = ((SR * 30.0 / BPM) as usize).clamp(64, DELAY_RING - 2);
        assert!(
            (0.30..0.46).contains(&(taps as f32 / SR)),
            "the tap is {:.3} s, outside the 300-450 ms the space wants",
            taps as f32 / SR
        );
        let peak_near = |n: usize| {
            out[n.saturating_sub(64)..(n + 64).min(out.len())]
                .iter()
                .fold(0.0f32, |a, s| a.max(s.abs()))
        };
        assert!(peak_near(taps) > 0.5, "the first repeat never arrived");
        assert!(peak_near(taps * 2) > 0.05, "there is only one repeat");
        assert!(peak_near(taps * 2) < peak_near(taps), "the repeats are not decaying");
        assert!(peak_near(taps * 6) < 0.01, "the delay is still ringing at six repeats");
    }

    #[test]
    fn the_score_never_startles() {
        // Measured as the windowed envelope rather than the raw sample slope: a
        // 440 Hz partial legitimately swings most of its range between two
        // samples at 22 kHz, and what the ear reads as a crack is the envelope
        // arriving at once.
        let buf = render(45.0, 0.9, 0.95, 0.0);
        let window = (0.010 * SR) as usize;
        let peak = buf.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(peak > 0.05 && peak < 1.0, "the score peaked at {peak}");
        let mut worst = 0.0f32;
        let mut previous = 0.0f32;
        for chunk in buf.chunks(window) {
            let level = rms(chunk);
            worst = worst.max((level - previous).abs());
            previous = level;
        }
        assert!(
            worst < peak * 0.42,
            "the envelope jumped by {worst} against a peak of {peak}"
        );
    }

    #[test]
    fn the_score_is_finite_and_bounded_in_every_reading() {
        for (warmth, density, dusk) in [
            (0.05f32, 0.45f32, 0.0f32),
            (1.0, 0.95, 0.0),
            (0.5, 0.6, 1.0),
            (0.95, 1.0, 1.0),
        ] {
            let buf = render(30.0, warmth, density, dusk);
            assert!(buf.iter().all(|s| s.is_finite()), "the score produced a NaN");
            let peak = buf.iter().fold(0.0f32, |a, s| a.max(s.abs()));
            assert!(peak < 0.95, "reading peaked at {peak}");
            assert!(peak > 0.03, "reading was effectively silent at {peak}");
        }
    }

    #[test]
    fn silence_costs_nothing_and_a_new_cue_starts_a_new_piece() {
        let mut score = Score::new(SEED);
        for _ in 0..1000 {
            assert_eq!(score.step(0, 0.8, 0.9, 0.0), 0.0);
        }
        assert_eq!(score.pos, 0, "the piece advanced while silent");
        for _ in 0..(SR as usize * 30) {
            score.step(1, 0.8, 0.9, 0.0);
        }
        assert!(score.pos > 0);
        let first = score.piece;
        score.step(2, 0.8, 0.9, 0.0);
        assert_eq!(score.pos, 1, "a new cue did not start at the top");
        assert_ne!(score.piece, first, "a new cue replayed the same piece");
        // And nothing from the last cue is still ringing under the new key.
        assert!(score.bells.iter().all(|b| !b.active || b.age <= 1));
    }

    #[test]
    fn the_dusk_reading_is_softer_and_darker_without_being_lower() {
        let day = render(28.0, 0.8, 0.9, 0.0);
        let dusk = render(28.0, 0.8, 0.9, 1.0);
        // The fraction of the energy above about 1.5 kHz, via a one-pole
        // high-pass. A ratio rather than a level, so this measures colour and
        // not which reading happens to be louder.
        let treble = |b: &[f32]| {
            let a = pole_coeff(1500.0, SR);
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
        // But it is the same tune in the same octave. The old score dropped the
        // melody an octave at dusk, and a dark low reading is exactly the thing
        // the playtest called bland.
        let mut score = Score::new(SEED);
        let events: Vec<f32> = score.pieces[0]
            .events
            .iter()
            .filter(|e| e.part == Part::Keys)
            .map(|e| e.pitch)
            .collect();
        let _ = score.step(1, 0.5, 0.9, 1.0);
        assert!(
            events.iter().all(|p| *p >= (C4 - 3) as f32),
            "dusk moved the tune below C4"
        );
    }

    #[test]
    fn the_score_is_cheap_enough_for_the_audio_thread() {
        // The generator shares a core with the renderer; a serious FPS
        // regression was just fixed and this must not be the next one. The
        // budget is the one the previous score was held to.
        let mut score = Score::new(SEED);
        let n = (SR * 20.0) as usize;
        let start = std::time::Instant::now();
        let mut sink = 0.0f32;
        for _ in 0..n {
            sink += score.step(1, 0.9, 0.95, 0.0);
        }
        let elapsed = start.elapsed().as_secs_f64();
        assert!(sink.is_finite());
        assert!(elapsed < 4.0, "20 s of score took {elapsed:.2} s to render");
    }

    #[test]
    fn composing_a_world_is_not_a_hitch() {
        // Four pieces are composed when the voice is created, on the ECS thread
        // rather than the audio one - but it still happens inside a frame.
        let start = std::time::Instant::now();
        let score = Score::new(SEED);
        let elapsed = start.elapsed().as_secs_f64();
        assert_eq!(score.pieces.len(), PIECES);
        assert!(elapsed < 0.25, "composing four pieces took {elapsed:.3} s");
    }


    /// Render twenty seconds of one piece to a WAV so a human can listen.
    ///
    /// Not a check - a tool, and the only way anybody finds out whether this
    /// actually sounds like anything:
    ///
    /// ```text
    /// cargo test -p rail_town --lib score::tests::write_a_sample_to_listen_to -- --ignored --nocapture
    /// ```
    ///
    /// `RAIL_TOWN_SCORE_WAV`, `_SECS`, `_SEED`, `_PIECE`, `_WARMTH`, `_DENSITY`
    /// and `_DUSK` override the defaults; the file is never committed.
    #[test]
    #[ignore = "writes a WAV for a human to listen to"]
    fn write_a_sample_to_listen_to() {
        let path = std::env::var("RAIL_TOWN_SCORE_WAV")
            .unwrap_or_else(|_| "/tmp/score_sample.wav".to_string());
        let env = |name: &str, fallback: f32| -> f32 {
            std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(fallback)
        };
        let secs = env("RAIL_TOWN_SCORE_SECS", 20.0);
        let warmth = env("RAIL_TOWN_SCORE_WARMTH", 0.85);
        let density = env("RAIL_TOWN_SCORE_DENSITY", 0.92);
        let dusk = env("RAIL_TOWN_SCORE_DUSK", 0.0);
        let seed: u64 = std::env::var("RAIL_TOWN_SCORE_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(SEED);
        let piece: u32 = std::env::var("RAIL_TOWN_SCORE_PIECE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);

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
            let s = score.step(piece.max(1), warmth, density, dusk).clamp(-1.0, 1.0);
            pcm.extend_from_slice(&((s * 32000.0) as i16).to_le_bytes());
        }
        std::fs::write(&path, pcm).expect("could not write the WAV");
        let key = KEYS[score.pieces[Score::piece_for(piece.max(1))].key].name;
        println!("wrote {path}: {secs} s of seed {seed} piece {piece} in {key}");
    }
}
