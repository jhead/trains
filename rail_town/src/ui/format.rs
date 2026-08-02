//! Shared readouts: money, rates, and the in-game calendar clock.
//!
//! # Money (03 §6)
//!
//! The balance is **whole dollars**. A balance that ticks in cents is read
//! constantly and changes constantly, and the two together make a calm builder
//! feel like a countdown. Cents survive in exactly one place — a net rate under
//! a dollar a minute, where rounding to zero would be a lie.
//!
//! # Clock (03 §6)
//!
//! [`crate::atmosphere::TimeOfDay`] owns the day cycle and exposes a `fraction`
//! where `0.0` is first light. That is the only clock in the game, so the
//! readout is derived from it rather than counted separately — the warm dusk
//! cast and the time on the strip can never disagree.
//!
//! `fraction` is mapped onto a 24-hour day with first light at [`FIRST_LIGHT_MINUTES`],
//! which puts the atmosphere module's own phase boundaries at plausible clock
//! times (day from ~08:20, dusk from ~17:55, night from ~20:50).

/// Clock time at cycle position `0.0`, in minutes past midnight.
pub const FIRST_LIGHT_MINUTES: u32 = 5 * 60;

/// Days in one season.
pub const DAYS_PER_SEASON: u32 = 12;

/// Seasons, in order. Season affects nothing yet; the label is here because the
/// player needs to know why the light changed, and a bare day counter does not
/// say that (03 §6).
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

/// `HH:MM` for a cycle position.
pub fn clock_label(fraction: f32) -> String {
    let minutes = minute_of_day(fraction);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Minutes past midnight for a cycle position.
pub fn minute_of_day(fraction: f32) -> u32 {
    let f = fraction.rem_euclid(1.0);
    let raw = (f * 1440.0) as u32 + FIRST_LIGHT_MINUTES;
    raw % 1440
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
    fn first_light_reads_as_early_morning() {
        assert_eq!(clock_label(0.0), "05:00");
        // The atmosphere module's own phase boundaries land at plausible times,
        // which is the whole point: the warm cast now has a clock beside it.
        assert_eq!(clock_label(0.14), "08:21");
        assert_eq!(clock_label(0.54), "17:57");
        assert_eq!(clock_label(0.66), "20:50");
    }

    #[test]
    fn the_clock_wraps_rather_than_running_past_midnight() {
        assert_eq!(clock_label(0.834), "01:00");
        assert_eq!(clock_label(1.0), "05:00");
        assert_eq!(clock_label(-0.01), "04:45");
        for step in 0..1000 {
            let minutes = minute_of_day(step as f32 / 1000.0);
            assert!(minutes < 1440);
        }
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
