//! Ledger window — income vs expense by category, plus the recent net trend.
//!
//! Opens from the menu row or `K`. It used to answer to `L`, which the Line
//! tool also claims; the Controls tab flagged that as a conflict and it was a
//! real one, so the window moved and the verb kept the letter.
//!
//! The ledger is the one place cents still belong: it is a detail view opened
//! deliberately, not a number the player watches out of the corner of their eye
//! (03 §6).

use bevy::prelude::*;
use rail_sim::{MoneyCategory, MoneyLedger};

use crate::ui::format::{money_exact, money_rate, money_signed_exact};
use crate::ui::kit::{micro_font, text_primary};
use crate::ui::window::{window_root, WindowId, WindowManager};

#[derive(Component)]
pub struct LedgerPanelRoot;

#[derive(Component)]
pub struct LedgerBodyText;

#[derive(Resource, Debug, Default)]
pub(crate) struct LedgerUiCache {
    body: String,
}

pub fn setup_ledger_ui(mut commands: Commands) {
    commands.insert_resource(LedgerUiCache::default());
    commands
        .spawn((LedgerPanelRoot, window_root(WindowId::Ledger, 268.0)))
        .with_children(|panel| {
            panel.spawn((
                LedgerBodyText,
                Text::new("No activity yet."),
                micro_font(),
                text_primary(),
            ));
        });
}

pub fn update_ledger_panel(
    manager: Res<WindowManager>,
    ledger: Res<MoneyLedger>,
    mut cache: ResMut<LedgerUiCache>,
    mut body_q: Query<&mut Text, With<LedgerBodyText>>,
) {
    if !manager.is_open(WindowId::Ledger) {
        return;
    }
    let body = format_ledger_body(&ledger);
    if body == cache.body {
        return;
    }
    cache.body = body.clone();
    if let Ok(mut text) = body_q.single_mut() {
        *text = Text::new(body);
    }
}

fn format_ledger_body(ledger: &MoneyLedger) -> String {
    let mut lines = Vec::new();
    lines.push("Income".to_string());
    for cat in MoneyCategory::ALL {
        if !cat.is_income() {
            continue;
        }
        let session = ledger.total(cat).max(0);
        let recent = ledger.recent(cat).max(0);
        lines.push(format!(
            "  {:<14} {}  (~{})",
            cat.label(),
            money_exact(session),
            money_exact(recent)
        ));
    }
    lines.push("Expense".to_string());
    for cat in MoneyCategory::ALL {
        if cat.is_income() {
            continue;
        }
        let session = (-ledger.total(cat)).max(0);
        let recent = (-ledger.recent(cat)).max(0);
        lines.push(format!(
            "  {:<14} {}  (~{})",
            cat.label(),
            money_exact(session),
            money_exact(recent)
        ));
    }
    let net = ledger.session_income() - ledger.session_expense();
    lines.push(format!(
        "Net session    {}   rate {}",
        money_signed_exact(net),
        money_rate(ledger.net_rate_cents_per_min())
    ));
    lines.push(format!("{} {}", trend_label(ledger), sparkline(ledger)));
    lines.join("\n")
}

/// "Trend (last 7 min)" — the window the sparkline actually covers.
///
/// One sample is one real minute (`rail_sim::LEDGER_SAMPLE_SIM_SECS`), so the
/// count of completed samples *is* the number of minutes drawn. Stating it is
/// not decoration: design 08 §6 wants "a history graph long enough to show a
/// trend across a session", and a player cannot tell whether they are looking
/// at a session or a second without being told which.
fn trend_label(ledger: &MoneyLedger) -> String {
    match ledger.history_len() {
        0 => "Trend".to_string(),
        1 => "Trend (last min)".to_string(),
        n => format!("Trend (last {n} min)"),
    }
}

/// Height ramp for the trend line, low to high.
///
/// **ASCII only.** The shipped font has no block-drawing glyphs, so the old
/// `▁▂▃▄▅▆▇█` ramp rendered as a row of tofu boxes in game — a chart that told
/// the player nothing except that something was broken. Every glyph the UI draws
/// has to exist in the font it is drawn with (03 §3).
const RAMP: &[u8] = b".:-=+*#@";

/// Net income per sample, as a trend line.
///
/// Scaled between the run's own minimum and maximum rather than by absolute
/// magnitude. The old version ranked samples by `abs()`, which drew a heavy loss
/// and a heavy profit at exactly the same height — precisely backwards for the
/// one question a trend line exists to answer.
fn sparkline(ledger: &MoneyLedger) -> String {
    let vals: Vec<i64> = ledger.history_nets().collect();
    if vals.is_empty() {
        return "-".into();
    }
    let min = *vals.iter().min().unwrap_or(&0);
    let max = *vals.iter().max().unwrap_or(&0);
    let mid = RAMP[RAMP.len() / 2] as char;
    if min == max {
        // Flat is flat, at whatever level. Drawing it along the bottom would
        // imply a decline that did not happen.
        return std::iter::repeat_n(mid, vals.len()).collect();
    }
    let span = (max - min) as i128;
    let top = RAMP.len() as i128 - 1;
    vals.iter()
        .map(|v| {
            let idx = (((*v - min) as i128 * top) / span).clamp(0, top) as usize;
            RAMP[idx] as char
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::LEDGER_SAMPLE_SIM_SECS;

    #[test]
    fn ledger_body_lists_categories() {
        let mut ledger = MoneyLedger::default();
        ledger.record(MoneyCategory::Fares, 500);
        ledger.record(MoneyCategory::TrainOpex, -10);
        ledger.on_sim_secs(LEDGER_SAMPLE_SIM_SECS);
        let body = format_ledger_body(&ledger);
        assert!(body.contains("Fares"));
        assert!(body.contains("Train opex"));
        assert!(body.contains("Trend"));
    }

    #[test]
    fn every_glyph_the_ledger_draws_is_ascii() {
        // 03 §3: the shipped font has no block-drawing characters, and a glyph
        // it cannot render becomes a tofu box on screen.
        let mut ledger = MoneyLedger::default();
        for (i, cents) in [500i64, -200, 900, 0, -1_400].iter().enumerate() {
            ledger.record(MoneyCategory::Fares, *cents);
            ledger.on_sim_secs(LEDGER_SAMPLE_SIM_SECS);
            let _ = i;
        }
        let body = format_ledger_body(&ledger);
        assert!(
            body.is_ascii(),
            "non-ASCII glyph in the ledger: {:?}",
            body.chars().filter(|c| !c.is_ascii()).collect::<Vec<_>>()
        );
        assert!(RAMP.is_ascii());
    }

    #[test]
    fn the_trend_ranks_by_value_and_not_by_magnitude() {
        // The old ramp used abs(), which drew a big loss and a big profit at
        // the same height — backwards for the only question it answers.
        let mut ledger = MoneyLedger::default();
        for cents in [-1_000i64, 0, 1_000] {
            ledger.record(MoneyCategory::Fares, cents);
            ledger.on_sim_secs(LEDGER_SAMPLE_SIM_SECS);
        }
        let line = sparkline(&ledger);
        let chars: Vec<char> = line.chars().collect();
        assert_eq!(chars.len(), 3, "{line}");
        let rank = |c: char| RAMP.iter().position(|r| *r as char == c).unwrap();
        assert!(
            rank(chars[0]) < rank(chars[1]) && rank(chars[1]) < rank(chars[2]),
            "a rising run must rise: {line}"
        );
    }

    #[test]
    fn a_flat_history_draws_flat_rather_than_low() {
        let mut ledger = MoneyLedger::default();
        for _ in 0..4 {
            ledger.on_sim_secs(LEDGER_SAMPLE_SIM_SECS);
        }
        let line = sparkline(&ledger);
        assert!(!line.is_empty());
        assert_eq!(
            line.chars().collect::<std::collections::HashSet<_>>().len(),
            1,
            "a flat run has one level: {line}"
        );
    }

    #[test]
    fn an_empty_history_says_so() {
        assert_eq!(sparkline(&MoneyLedger::default()), "-");
    }

    /// The trend has to say how much time it covers, and be right about it.
    ///
    /// It used to cover 24 samples of 30 *sim*-seconds — three ticks each, so
    /// 1.1 real seconds in total — while sitting under a panel that talks about
    /// a session. A sample is now a real minute.
    #[test]
    fn the_trend_states_the_window_it_actually_covers() {
        let mut ledger = MoneyLedger::default();
        assert_eq!(trend_label(&ledger), "Trend");

        ledger.on_sim_secs(LEDGER_SAMPLE_SIM_SECS);
        assert_eq!(trend_label(&ledger), "Trend (last min)");

        for _ in 0..6 {
            ledger.on_sim_secs(LEDGER_SAMPLE_SIM_SECS);
        }
        assert_eq!(trend_label(&ledger), "Trend (last 7 min)");

        // And a sample really is a real minute of ticks.
        assert_eq!(
            LEDGER_SAMPLE_SIM_SECS / rail_sim::SIM_SECONDS_PER_TICK,
            3_840
        );
    }

    #[test]
    fn the_detail_view_keeps_its_cents() {
        // Whole dollars are for the strip; a ledger that hides cents cannot be
        // reconciled against the balance it explains.
        let mut ledger = MoneyLedger::default();
        ledger.record(MoneyCategory::Fares, 1_234);
        let body = format_ledger_body(&ledger);
        assert!(body.contains("$12.34"), "{body}");
    }
}
