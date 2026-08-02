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
//! [`crate::atmosphere::TimeOfDay`] drives the day tint. Before this strip
//! existed the player would watch the world turn warm with no way to know why.
//! Season, day, time and the phase name are all derived from the same
//! `fraction`, so the readout can never disagree with the light.

use bevy::prelude::*;
use rail_sim::{AlertBoard, CommandBuffer, CommandKind, Money, MoneyLedger, SimClock, StationService};

use crate::atmosphere::TimeOfDay;
use crate::palette::{HI, OK, OUTLINE, WARN};
use crate::ui::format::{clock_label, date_label, money_rate, money_whole};
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

/// The in-game calendar.
///
/// [`TimeOfDay`] is a position inside one day and nothing more, so the day
/// counter lives here: it advances when the cycle fraction wraps. That keeps the
/// atmosphere module free of a calendar it has no use for, and keeps the two
/// from ever drifting apart, because there is only one clock.
#[derive(Resource, Debug, Clone, Copy)]
pub struct GameCalendar {
    pub day: u32,
    last_fraction: f32,
}

impl Default for GameCalendar {
    fn default() -> Self {
        Self {
            day: 0,
            last_fraction: 0.0,
        }
    }
}

impl GameCalendar {
    /// Feed the current cycle position; returns `true` on a new day.
    pub fn observe(&mut self, fraction: f32) -> bool {
        // The cycle only ever moves forward, so a fall means it wrapped past
        // first light. The half-cycle guard keeps a load or a rewind from
        // counting as a day.
        let wrapped = fraction + 0.5 < self.last_fraction;
        self.last_fraction = fraction;
        if wrapped {
            self.day = self.day.saturating_add(1);
        }
        wrapped
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
                Text::new("Spring 1  05:00"),
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

/// Advance the calendar. One resource read, one compare — cheap enough to run
/// every frame, and it has to, because the wrap can happen on any frame.
pub fn advance_calendar(tod: Res<TimeOfDay>, mut calendar: ResMut<GameCalendar>) {
    if !tod.is_changed() {
        return;
    }
    calendar.observe(tod.fraction);
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

    let clock_str = format!("{}  {}", calendar.label(), clock_label(tod.fraction));
    if clock_str != cache.clock {
        cache.clock = clock_str.clone();
        if let Ok(mut text) = clock_q.single_mut() {
            *text = Text::new(clock_str);
        }
    }

    let phase_str = tod.phase.label().to_string();
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

    #[test]
    fn the_calendar_advances_when_the_cycle_wraps() {
        let mut calendar = GameCalendar::default();
        assert!(!calendar.observe(0.20));
        assert!(!calendar.observe(0.99));
        assert!(calendar.observe(0.01), "past first light is a new day");
        assert_eq!(calendar.day, 1);
        assert_eq!(calendar.label(), "Spring 2");
    }

    #[test]
    fn a_small_step_backwards_is_not_a_new_day() {
        // Loading a save can move the clock back; that is not a day passing.
        let mut calendar = GameCalendar::default();
        calendar.observe(0.60);
        assert!(!calendar.observe(0.55));
        assert_eq!(calendar.day, 0);
    }

    #[test]
    fn a_full_season_of_days_reaches_summer() {
        let mut calendar = GameCalendar::default();
        for _ in 0..12 {
            calendar.observe(0.99);
            calendar.observe(0.01);
        }
        assert_eq!(calendar.day, 12);
        assert_eq!(calendar.label(), "Summer 1");
    }
}
