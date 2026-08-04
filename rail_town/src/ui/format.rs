//! Shared readouts: money, rates, and the in-game calendar clock.
//!
//! # Money (03 §6)
//!
//! The balance is **whole dollars**. A balance that ticks in cents is read
//! constantly and changes constantly, and the two together make a calm builder
//! feel like a countdown. Cents survive in exactly one place — a net rate under
//! a dollar a minute, where rounding to zero would be a lie.
//!
//! # Clock (03 §6, and [17 — Time & Pacing](../../../docs/design/17-time-and-pacing.md) §3)
//!
//! **The strip shows a date and a part of the day. It does not show `HH:MM`,
//! and it must not.**
//!
//! It used to. The label was minute-resolution and it was driven by the light
//! cycle, which is twelve real minutes long — so a clock minute went by every
//! half a real second, and a transit crossing ten tiles appeared to do it in
//! about one. The owner's report was exactly that arithmetic: *"trains go
//! between ~10 tiles in ~1 in-game minute at 1x speed which is insanely
//! fast."* Nothing was wrong with the train. The **claim** was wrong, and it
//! was a claim nobody needed the game to make.
//!
//! RCT and Locomotion show a month and a year for the same reason: screen time
//! and world time cannot both be literal, and a coarse clock face is the one
//! that never lies. Brief 17 §3 is the whole argument.
//!
//! ## Where each half comes from
//!
//! | Field | Source | Why |
//! | --- | --- | --- |
//! | `Spring 3` | the sim's own day, [`rail_sim::day_index`] | it is the day the Goals panel and the Peep card already count |
//! | `Morning` | [`crate::atmosphere::TimeOfDay::fraction`] | 03 §6: the readout can never disagree with the light |
//!
//! The part-of-day boundaries are a **refinement** of the tint's own phases —
//! `Dawn` and `Night` are exactly the tint's, and `Morning` / `Midday` /
//! `Afternoon` subdivide the flat untinted middle, where there is no light
//! change to contradict. So the word on the strip and the colour on the world
//! agree by construction, which is the rule 03 §6 was written to protect.

/// Days in one season.
pub const DAYS_PER_SEASON: u32 = 12;

/// Cycle positions where each named part of the day begins.
///
/// `0.0` is first light. The first, fifth and sixth boundaries are the
/// atmosphere module's own `DAY_START` / `DUSK_START` / `NIGHT_START`; the two
/// in between split the long flat daylight stretch, which carries no tint and
/// so cannot be contradicted. `part_of_day_agrees_with_the_light` holds the
/// pairing.
const PART_STARTS: [(f32, &str); 6] = [
    (0.00, "Dawn"),
    (0.14, "Morning"),
    (0.32, "Midday"),
    (0.44, "Afternoon"),
    (0.54, "Evening"),
    (0.66, "Night"),
];

/// Seasons, in order. Season affects nothing yet; the label is here to give the
/// date a shape a bare counter has not got — `Autumn 4` says how long the
/// session has been going in a way `day 40` does not.
///
/// On the sim day ([`crate::ui::status_strip`]) a season is twelve days, about
/// **27 real minutes**, and a year is four of them. Counted against the light
/// cycle, as it was, a season took two and a half hours and nobody ever saw
/// autumn.
pub const SEASONS: [&str; 4] = ["Spring", "Summer", "Autumn", "Winter"];

/// Whole dollars with thousands separators — `-$1,204`.
pub fn money_whole(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    // Truncate toward zero: a balance never rounds up into money you don't have.
    let dollars = cents.unsigned_abs() / 100;
    format!("{sign}${}", group_thousands(dollars))
}

/// Money with cents, for ledger detail where the exact figure is the point.
pub fn money_exact(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    format!("{sign}${}.{:02}", group_thousands(abs / 100), abs % 100)
}

/// Signed money with cents — `+$12.40`.
pub fn money_signed_exact(cents: i64) -> String {
    if cents == 0 {
        return "$0.00".into();
    }
    let sign = if cents > 0 { "+" } else { "-" };
    format!("{sign}{}", money_exact(cents.abs()))
}

/// Net rate per minute.
///
/// Whole dollars, except under a dollar a minute where cents are the only thing
/// that distinguishes "barely earning" from "earning nothing" (03 §6).
pub fn money_rate(cents_per_min: i64) -> String {
    if cents_per_min == 0 {
        return "$0/min".into();
    }
    let sign = if cents_per_min > 0 { "+" } else { "-" };
    let abs = cents_per_min.unsigned_abs();
    if abs < 100 {
        format!("{sign}$0.{:02}/min", abs % 100)
    } else {
        format!("{sign}${}/min", group_thousands(abs / 100))
    }
}

fn group_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// `Morning` — which part of the day the light is in.
///
/// Deliberately the whole of what the strip says about time. See the module
/// docs, and brief 17 §3 for the argument.
pub fn part_of_day(fraction: f32) -> &'static str {
    let f = fraction.rem_euclid(1.0);
    let mut label = PART_STARTS[PART_STARTS.len() - 1].1;
    for &(start, name) in PART_STARTS.iter() {
        if f >= start {
            label = name;
        }
    }
    label
}

/// `Spring 3` — season and day-within-season, from a day index counted from
/// the first day of play.
pub fn date_label(day: u32) -> String {
    let season = SEASONS[((day / DAYS_PER_SEASON) % SEASONS.len() as u32) as usize];
    format!("{season} {}", day % DAYS_PER_SEASON + 1)
}

/// Two-letter tag for how far the meter has to shrink before names collide.
pub fn abbreviate(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_string();
    }
    name.chars().take(max.saturating_sub(1)).collect::<String>() + "."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_balance_is_whole_dollars() {
        // The playtest note: a balance ticking in cents reads as stressful.
        assert_eq!(money_whole(1_248_099), "$12,480");
        assert_eq!(money_whole(1_248_000), "$12,480");
        assert_eq!(money_whole(0), "$0");
        assert_eq!(money_whole(99), "$0");
        assert_eq!(money_whole(-150_050), "-$1,500");
    }

    #[test]
    fn thousands_are_grouped_at_every_magnitude() {
        assert_eq!(money_whole(100), "$1");
        assert_eq!(money_whole(100_000), "$1,000");
        assert_eq!(money_whole(100_000_000), "$1,000,000");
    }

    #[test]
    fn the_rate_keeps_cents_only_below_a_dollar() {
        assert_eq!(money_rate(0), "$0/min");
        assert_eq!(money_rate(34_000), "+$340/min");
        assert_eq!(money_rate(-500), "-$5/min");
        assert_eq!(money_rate(-550), "-$5/min");
        assert_eq!(money_rate(40), "+$0.40/min");
        assert_eq!(money_rate(-7), "-$0.07/min");
    }

    #[test]
    fn the_day_reads_as_parts_and_never_as_minutes() {
        assert_eq!(part_of_day(0.0), "Dawn");
        assert_eq!(part_of_day(0.20), "Morning");
        assert_eq!(part_of_day(0.35), "Midday");
        assert_eq!(part_of_day(0.50), "Afternoon");
        assert_eq!(part_of_day(0.60), "Evening");
        assert_eq!(part_of_day(0.90), "Night");
        // Wraps rather than clamping — the cycle has a seam and the strip is
        // read across it.
        assert_eq!(part_of_day(1.0), "Dawn");
        assert_eq!(part_of_day(-0.01), "Night");
        assert_eq!(part_of_day(1.9), "Night");
    }

    /// **The rule 03 §6 exists to protect.** The word on the strip is derived
    /// from the same `fraction` as the tint, and its boundaries refine the
    /// tint's own — so the strip can never say "Morning" over a dusk-lit world.
    #[test]
    fn part_of_day_agrees_with_the_light() {
        use crate::atmosphere::{DayPhase, TimeOfDay};
        for step in 0..2000 {
            let f = step as f32 / 2000.0;
            let phase = TimeOfDay::at(f).phase;
            let expected = match phase {
                DayPhase::Dawn => &["Dawn"][..],
                DayPhase::Day => &["Morning", "Midday", "Afternoon"][..],
                DayPhase::Dusk => &["Evening"][..],
                DayPhase::Night => &["Night"][..],
            };
            let part = part_of_day(f);
            assert!(
                expected.contains(&part),
                "at {f} the world is tinted {phase:?} and the strip says {part}"
            );
        }
    }

    #[test]
    fn every_part_of_the_day_gets_a_turn() {
        let mut seen: Vec<&str> = (0..2000)
            .map(|s| part_of_day(s as f32 / 2000.0))
            .collect();
        seen.dedup();
        assert_eq!(
            seen,
            vec!["Dawn", "Morning", "Midday", "Afternoon", "Evening", "Night"],
            "the parts should run in order, once each, across a cycle"
        );
    }

    #[test]
    fn the_date_walks_seasons() {
        assert_eq!(date_label(0), "Spring 1");
        assert_eq!(date_label(11), "Spring 12");
        assert_eq!(date_label(12), "Summer 1");
        assert_eq!(date_label(36), "Winter 1");
        assert_eq!(date_label(48), "Spring 1");
    }

    #[test]
    fn abbreviation_only_bites_when_it_has_to() {
        assert_eq!(abbreviate("Westbrook", 12), "Westbrook");
        assert_eq!(abbreviate("Westbrook", 6), "Westb.");
    }
}
