//! Plain-language cause lines for the inspector (stations, peeps).

use rail_sim::Mood;

/// Inputs for a station service cause sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StationCauseInput {
    pub score: u8,
    /// Current score minus a recent sample (negative = falling).
    pub score_delta: i16,
    pub waiting_passengers: u32,
    /// Peeps at this station with wait ≥ threshold minutes.
    pub long_wait_count: u32,
    /// Threshold used for the long-wait clause (minutes).
    pub long_wait_minutes: u32,
}

/// Plain-language cause for a station's service headline.
///
/// Examples:
/// - `"Falling: 14 people waiting more than 8 minutes."`
/// - `"Rising: recent deliveries are lifting service."`
/// - `"Steady: service holding."`
pub fn station_cause_line(input: StationCauseInput) -> String {
    let falling = input.score_delta < 0
        || input.waiting_passengers > 0
        || input.long_wait_count > 0
        || input.score < 40;
    let rising = input.score_delta > 0 && input.waiting_passengers == 0 && input.long_wait_count == 0;

    if falling {
        if input.long_wait_count > 0 {
            let n = input.long_wait_count;
            let m = input.long_wait_minutes.max(1);
            let people = if n == 1 { "person" } else { "people" };
            return format!("Falling: {n} {people} waiting more than {m} minutes.");
        }
        if input.waiting_passengers > 0 {
            let n = input.waiting_passengers;
            let people = if n == 1 { "person" } else { "people" };
            return format!("Falling: {n} {people} waiting for a train.");
        }
        if input.score_delta < 0 {
            return "Falling: service drifting without arrivals.".into();
        }
        return "Falling: platforms need more service.".into();
    }

    if rising {
        return "Rising: recent deliveries are lifting service.".into();
    }

    if input.score >= 70 {
        "Steady: service holding.".into()
    } else {
        "Quiet: waiting for trains.".into()
    }
}

/// Mood + reason line for a peep card.
pub fn peep_mood_line(mood: Mood, wait_secs: u32, station_name: &str) -> String {
    let mins = (wait_secs / 60).max(1);
    match mood {
        Mood::Frustrated => {
            format!("Frustrated - waited {mins} min at {station_name}.")
        }
        Mood::Uneasy => {
            format!("Uneasy - waiting at {station_name} ({mins} min).")
        }
        Mood::Content => {
            if wait_secs < 60 {
                "Content - commute is fine.".into()
            } else {
                format!("Content - at {station_name}.")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falling_uses_long_wait_clause() {
        let line = station_cause_line(StationCauseInput {
            score: 42,
            score_delta: -3,
            waiting_passengers: 14,
            long_wait_count: 14,
            long_wait_minutes: 8,
        });
        assert_eq!(
            line,
            "Falling: 14 people waiting more than 8 minutes."
        );
    }

    #[test]
    fn falling_singular_person() {
        let line = station_cause_line(StationCauseInput {
            score: 30,
            score_delta: 0,
            waiting_passengers: 1,
            long_wait_count: 1,
            long_wait_minutes: 8,
        });
        assert_eq!(line, "Falling: 1 person waiting more than 8 minutes.");
    }

    #[test]
    fn rising_when_score_up_and_clear() {
        let line = station_cause_line(StationCauseInput {
            score: 80,
            score_delta: 5,
            waiting_passengers: 0,
            long_wait_count: 0,
            long_wait_minutes: 8,
        });
        assert_eq!(line, "Rising: recent deliveries are lifting service.");
    }

    #[test]
    fn steady_when_flat_and_good() {
        let line = station_cause_line(StationCauseInput {
            score: 78,
            score_delta: 0,
            waiting_passengers: 0,
            long_wait_count: 0,
            long_wait_minutes: 8,
        });
        assert_eq!(line, "Steady: service holding.");
    }

    #[test]
    fn peep_frustrated_names_station() {
        let line = peep_mood_line(Mood::Frustrated, 11 * 60, "Eastgate");
        assert_eq!(line, "Frustrated - waited 11 min at Eastgate.");
    }
}
