//! The status strip — money, net rate, date and time, speed, alert bell.
//!
//! Binding standard: [`docs/design/03-ui-system.md`](../../../docs/design/03-ui-system.md) §6.
//!
//! # Money
//!
//! Whole dollars. The balance is read constantly and, with fares landing in
//! cents, it used to change constantly; a number that ticks is a number that
//! pulls the eye, and this game is calm. The net rate keeps cents only below a
//! dollar a minute, where rounding would erase the distinction between "barely
//! earning" and "not earning".
//!
//! The money field carries a `min_width`, so a balance rolling from `$999` to
//! `$1,000` does not shove the rest of the strip sideways. That is the same job
//! tabular numerals do, done with layout until a bitmap font lands.
//!
//! # The clock
//!
//! Binding standard: [`docs/design/17-time-and-pacing.md`](../../../docs/design/17-time-and-pacing.md) §3.
//!
//! Two fields, and they answer different questions from different sources:
//!
//! - **`Spring 3`** — the date, counted in the **sim's own days**
//!   ([`rail_sim::day_index`]). That is the day the Goals panel deals its
//!   deadlines in (*"by day 4"*) and the day the Peep card counts tenure in
//!   (*"lived here 14 days"*), so it is the only day the game can afford to
//!   have. It used to count wraps of the twelve-minute light cycle instead,
//!   which meant the strip and the panels disagreed by a factor of five and a
//!   third about what a day was — and the strip's day reset to `Spring 1` on
//!   every load, because nothing saved it. The sim tick is saved.
//! - **`Morning`** — which part of the day the light is in, derived from
//!   [`crate::atmosphere::TimeOfDay`], the same `fraction` that drives the tint.
//!   03 §6's rule holds: what the strip says about the light comes from the
//!   light.
//!
//! **There is no `HH:MM`, deliberately** — see [`crate::ui::format`] for why a
//! minute-resolution clock made the railway look absurd.

use bevy::prelude::*;
use rail_sim::{AlertBoard, CommandBuffer, CommandKind, Money, MoneyLedger, SimClock, StationService};

use crate::atmosphere::TimeOfDay;
use crate::palette::{HI, OK, OUTLINE, WARN};
use crate::ui::format::{date_label, money_rate, money_whole, part_of_day};
use crate::ui::health::{actionable_alert_count, alerts_are_bad_news};
use crate::ui::kit::{
    body_font, chrome_button_node, control_border, display_font, micro_font, text_accent,
    text_primary, text_secondary, SPACE_1, SPACE_2, STATUS_ROW_H,
};
use crate::ui::window::{WindowId, WindowManager};

/// Enough width for `$1,000,000` at display size, so the strip never reflows.
const MONEY_MIN_W: f32 = 88.0;

#[derive(Component)]
pub struct StatusMoneyText;

#[derive(Component)]
pub struct StatusRateText;

#[derive(Component)]
pub struct StatusClockText;

#[derive(Component)]
pub struct StatusPhaseText;

#[derive(Component)]
pub struct AlertBellButton;

#[derive(Component)]
pub struct AlertBellText;

#[derive(Component)]
pub struct SpeedButton {
    pub multiplier: u8,
}

/// The in-game calendar — a read of the sim's day counter, nothing more.
///
/// It holds no state of its own on purpose. The day is `tick / TICKS_PER_DAY`,
/// the tick is saved with the world, and a derived readout cannot drift from
/// the thing it is derived from. The previous version counted wraps of the
/// light cycle in a resource of its own, which drifted from the sim's day by
/// construction and reset to `Spring 1` whenever a save was loaded.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct GameCalendar {
    pub day: u32,
}

impl GameCalendar {
    /// Take the day from a sim tick; returns `true` when the date changed.
    pub fn observe_tick(&mut self, tick: u64) -> bool {
        let day = rail_sim::day_index(tick) as u32;
        let changed = day != self.day;
        self.day = day;
        changed
    }

    pub fn label(&self) -> String {
        date_label(self.day)
    }
}

/// Tracks last painted strings so we skip no-op `Text` writes.
#[derive(Resource, Debug, Default)]
pub struct StatusStripCache {
    money: String,
    rate: String,
    clock: String,
    phase: String,
    bell: String,
    rate_cents_per_min: i64,
}

/// Spawn the status row into the top chrome.
pub fn spawn_status_row(parent: &mut ChildSpawnerCommands, starting_cents: i64) {
    parent
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(STATUS_ROW_H),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(SPACE_2),
                padding: UiRect::axes(Val::Px(SPACE_2), Val::Px(0.0)),
                border: UiRect {
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    top: Val::Px(1.0),
                    bottom: Val::Px(1.0),
                },
                border_radius: BorderRadius::ZERO,
                flex_shrink: 0.0,
                ..default()
            },
            BorderColor::all(OUTLINE),
        ))
        .with_children(|strip| {
            strip.spawn((
                StatusMoneyText,
                Text::new(money_whole(starting_cents)),
                display_font(),
                text_accent(),
                Node {
                    min_width: Val::Px(MONEY_MIN_W),
                    ..default()
                },
            ));
            strip.spawn((
                StatusRateText,
                Text::new(money_rate(0)),
                micro_font(),
                text_secondary(),
            ));

            strip.spawn((
                StatusClockText,
                Text::new("Spring 1"),
                body_font(),
                text_primary(),
                Node {
                    margin: UiRect::left(Val::Auto),
                    ..default()
                },
            ));
            strip.spawn((
                StatusPhaseText,
                Text::new("Day"),
                micro_font(),
                text_secondary(),
            ));

            strip
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(1.0),
                    margin: UiRect::left(Val::Auto),
                    ..default()
                })
                .with_children(|seg| {
                    for (label, mult) in [("||", 0u8), ("1x", 1), ("2x", 2), ("3x", 3)] {
                        let (node, bg, border) = chrome_button_node(SPACE_1, 0.0);
                        seg.spawn((Button, SpeedButton { multiplier: mult }, node, bg, border))
                            .with_children(|b| {
                                b.spawn((Text::new(label), micro_font(), text_primary()));
                            });
                    }
                });

            let (node, bg, border) = chrome_button_node(SPACE_1, 0.0);
            strip
                .spawn((Button, AlertBellButton, node, bg, border))
                .with_children(|b| {
                    b.spawn((
                        AlertBellText,
                        Text::new("! 0"),
                        micro_font(),
                        text_secondary(),
                    ));
                });
        });
}

/// Advance the calendar. One resource read, one divide — cheap enough to run
/// every frame, and it has to, because the day can turn over on any frame.
///
/// [`StationService::tick`] is the sim's master tick counter (it is what the
/// save snapshot stores as `service_tick`), and it only advances while the sim
/// is running — so a paused game holds its date, which is what a paused game
/// should do.
pub fn advance_calendar(service: Res<StationService>, mut calendar: ResMut<GameCalendar>) {
    calendar.observe_tick(service.tick);
}

#[allow(clippy::too_many_arguments)]
pub fn update_status_strip(
    money: Res<Money>,
    ledger: Res<MoneyLedger>,
    tod: Res<TimeOfDay>,
    calendar: Res<GameCalendar>,
    board: Res<AlertBoard>,
    service: Res<StationService>,
    mut cache: ResMut<StatusStripCache>,
    mut money_q: Query<&mut Text, With<StatusMoneyText>>,
    mut rate_q: Query<(&mut Text, &mut TextColor), (With<StatusRateText>, Without<StatusMoneyText>)>,
    mut clock_q: Query<
        &mut Text,
        (
            With<StatusClockText>,
            Without<StatusMoneyText>,
            Without<StatusRateText>,
        ),
    >,
    mut phase_q: Query<
        &mut Text,
        (
            With<StatusPhaseText>,
            Without<StatusMoneyText>,
            Without<StatusRateText>,
            Without<StatusClockText>,
        ),
    >,
    mut bell_q: Query<
        (&mut Text, &mut TextColor),
        (
            With<AlertBellText>,
            Without<StatusMoneyText>,
            Without<StatusRateText>,
            Without<StatusClockText>,
            Without<StatusPhaseText>,
        ),
    >,
) {
    let money_str = money_whole(money.cents());
    if money_str != cache.money {
        cache.money = money_str.clone();
        if let Ok(mut text) = money_q.single_mut() {
            *text = Text::new(money_str);
        }
    }

    let rate_cents = ledger.net_rate_cents_per_min();
    if cache.rate_cents_per_min != rate_cents {
        cache.rate_cents_per_min = rate_cents;
    }
    let rate_str = money_rate(rate_cents);
    if rate_str != cache.rate {
        cache.rate = rate_str.clone();
        if let Ok((mut text, mut color)) = rate_q.single_mut() {
            *text = Text::new(rate_str);
            *color = if cache.rate_cents_per_min > 0 {
                TextColor(OK)
            } else if cache.rate_cents_per_min < 0 {
                TextColor(WARN)
            } else {
                text_secondary()
            };
        }
    }

    let clock_str = calendar.label();
    if clock_str != cache.clock {
        cache.clock = clock_str.clone();
        if let Ok(mut text) = clock_q.single_mut() {
            *text = Text::new(clock_str);
        }
    }

    let phase_str = part_of_day(tod.fraction).to_string();
    if phase_str != cache.phase {
        cache.phase = phase_str.clone();
        if let Ok(mut text) = phase_q.single_mut() {
            *text = Text::new(phase_str);
        }
    }

    // The glyph changes with the tone as well as the colour, so the bell is
    // still readable with the colour turned off (03 §4).
    let count = actionable_alert_count(&board, &service);
    let bad = alerts_are_bad_news(&board, &service);
    let glyph = if count == 0 {
        "-"
    } else if bad {
        "!"
    } else {
        "*"
    };
    let bell_str = format!("{glyph} {count}");
    if bell_str != cache.bell {
        cache.bell = bell_str.clone();
        if let Ok((mut text, mut color)) = bell_q.single_mut() {
            *text = Text::new(bell_str);
            *color = if count == 0 {
                text_secondary()
            } else if bad {
                TextColor(WARN)
            } else {
                TextColor(HI)
            };
        }
    }
}

pub fn update_speed_buttons(
    clock: Res<SimClock>,
    mut speed_btns: Query<(&SpeedButton, &Interaction, &mut BorderColor, &Children), With<Button>>,
    mut child_colors: Query<&mut TextColor>,
) {
    let active_speed = if clock.paused {
        0u8
    } else {
        clock.speed_multiplier.max(1)
    };
    for (btn, interaction, mut border, children) in &mut speed_btns {
        let selected = btn.multiplier == active_speed;
        let hovered = matches!(interaction, Interaction::Hovered | Interaction::Pressed);
        let wanted = control_border(selected, hovered);
        if border.top != wanted.top {
            *border = wanted;
        }
        for child in children.iter() {
            if let Ok(mut c) = child_colors.get_mut(child) {
                let colour = if selected {
                    text_accent()
                } else {
                    text_primary()
                };
                if c.0 != colour.0 {
                    *c = colour;
                }
            }
        }
    }
}

pub fn speed_button_clicks(
    interactions: Query<(&Interaction, &SpeedButton), (Changed<Interaction>, With<Button>)>,
    mut buffer: ResMut<CommandBuffer>,
) {
    for (interaction, btn) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if btn.multiplier == 0 {
            buffer.push(CommandKind::pause(true));
        } else {
            buffer.push(CommandKind::set_speed(btn.multiplier));
        }
    }
}

/// The bell opens the Alerts window (03 §6: clicking the count opens the list).
pub fn alert_bell_clicks(
    mut interactions: Query<
        (&Interaction, &mut BorderColor),
        (Changed<Interaction>, With<AlertBellButton>),
    >,
    mut manager: ResMut<WindowManager>,
) {
    for (interaction, mut border) in &mut interactions {
        match interaction {
            Interaction::Pressed => {
                manager.toggle(WindowId::Alerts);
                *border = control_border(true, true);
            }
            Interaction::Hovered => *border = control_border(false, true),
            Interaction::None => *border = control_border(false, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rail_sim::TICKS_PER_DAY;

    #[test]
    fn the_calendar_turns_over_with_the_sim_day() {
        let mut calendar = GameCalendar::default();
        assert!(!calendar.observe_tick(0));
        assert!(!calendar.observe_tick(TICKS_PER_DAY - 1), "still day one");
        assert_eq!(calendar.label(), "Spring 1");
        assert!(calendar.observe_tick(TICKS_PER_DAY), "a sim day has passed");
        assert_eq!(calendar.day, 1);
        assert_eq!(calendar.label(), "Spring 2");
    }

    /// **The date is the same day the rest of the game speaks in.** The Goals
    /// panel says "by day 4" and the Peep card says "lived here 14 days"; both
    /// are `tick / TICKS_PER_DAY`, and so is this.
    #[test]
    fn the_strip_and_the_panels_agree_about_what_day_it_is() {
        let mut calendar = GameCalendar::default();
        for day in [0u64, 1, 7, 40] {
            let tick = day * TICKS_PER_DAY + TICKS_PER_DAY / 3;
            calendar.observe_tick(tick);
            assert_eq!(
                u64::from(calendar.day),
                rail_sim::day_index(tick),
                "the strip must count the sim's own days"
            );
        }
    }

    /// Loading a save restores the tick, so it restores the date. The old
    /// counter lived only in this resource and always reopened at Spring 1.
    #[test]
    fn a_loaded_world_keeps_its_date() {
        let mut calendar = GameCalendar::default();
        calendar.observe_tick(TICKS_PER_DAY * 30);
        assert_eq!(calendar.label(), "Autumn 7");

        let mut reloaded = GameCalendar::default();
        reloaded.observe_tick(TICKS_PER_DAY * 30);
        assert_eq!(reloaded.label(), calendar.label());
    }

    #[test]
    fn a_full_season_of_days_reaches_summer() {
        let mut calendar = GameCalendar::default();
        calendar.observe_tick(TICKS_PER_DAY * 12);
        assert_eq!(calendar.day, 12);
        assert_eq!(calendar.label(), "Summer 1");
    }
}
