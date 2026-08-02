//! Top status strip — money, rate, speed, active tool.
//!
//! Always visible. Text nodes are only rewritten when the displayed string changes
//! (avoids rebuilding unchanged HUD text every frame).

use bevy::prelude::*;
use rail_sim::{CommandBuffer, CommandKind, Money, MoneyLedger, SimClock};

use crate::lines::LineToolState;
use crate::palette::{BALLAST_L, BG1, HI, OK, OUTLINE, WARN};
use crate::track::{BuildTool, TrackToolState};
use crate::trains::{TrainPlaceKind, TrainToolState};
use crate::ui::kit::{
    body_font, display_font, text_accent, text_primary, text_secondary, FONT_BODY, SPACE_2,
    SPACE_3, STATUS_H,
};
use crate::ui::ledger::spawn_ledger_toggle;

#[derive(Component)]
pub struct StatusStripRoot;

#[derive(Component)]
pub struct StatusMoneyText;

#[derive(Component)]
pub struct StatusRateText;

#[derive(Component)]
pub struct StatusToolText;

#[derive(Component)]
pub struct SpeedButton {
    pub multiplier: u8,
}

/// Tracks last painted strings so we skip no-op Text writes.
#[derive(Resource, Debug, Default)]
pub struct StatusStripCache {
    money: String,
    rate: String,
    tool: String,
    rate_cents_per_min: i64,
}

pub fn setup_status_strip(mut commands: Commands, money: Res<Money>) {
    let money_str = format_money(money.cents());
    commands.insert_resource(StatusStripCache {
        money: money_str.clone(),
        rate: "$0/min".into(),
        tool: "Build".into(),
        rate_cents_per_min: 0,
    });

    commands
        .spawn((
            StatusStripRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                height: Val::Px(STATUS_H),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(SPACE_3),
                padding: UiRect::axes(Val::Px(SPACE_3), Val::Px(4.0)),
                border: UiRect {
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    top: Val::Px(0.0),
                    bottom: Val::Px(1.0),
                },
                border_radius: BorderRadius::ZERO,
                ..default()
            },
            BackgroundColor(BG1),
            BorderColor::all(OUTLINE),
            ZIndex(10),
        ))
        .with_children(|parent| {
            parent.spawn((
                StatusMoneyText,
                Text::new(money_str),
                display_font(),
                text_accent(),
            ));
            parent.spawn((
                StatusRateText,
                Text::new("$0/min"),
                body_font(),
                text_secondary(),
            ));
            parent.spawn((Text::new("·"), body_font(), text_secondary()));
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(2.0),
                    height: Val::Px(STATUS_H - 8.0),
                    ..default()
                })
                .with_children(|seg| {
                    for (label, mult) in [("❚❚", 0u8), ("1×", 1), ("2×", 2), ("3×", 3)] {
                        seg.spawn((
                            Button,
                            SpeedButton { multiplier: mult },
                            Node {
                                padding: UiRect::axes(Val::Px(SPACE_2), Val::Px(2.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                border_radius: BorderRadius::ZERO,
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(BG1),
                            BorderColor::all(OUTLINE),
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new(label),
                                TextFont::from_font_size(FONT_BODY),
                                text_primary(),
                            ));
                        });
                    }
                });
            parent.spawn((Text::new("·"), body_font(), text_secondary()));
            parent.spawn((
                StatusToolText,
                Text::new("Build"),
                body_font(),
                text_primary(),
            ));
            spawn_ledger_toggle(parent);
        });
}

pub fn update_status_strip(
    money: Res<Money>,
    ledger: Res<MoneyLedger>,
    clock: Res<SimClock>,
    tools: Res<TrackToolState>,
    train_tools: Option<Res<TrainToolState>>,
    line_tools: Option<Res<LineToolState>>,
    mut cache: ResMut<StatusStripCache>,
    mut money_q: Query<
        &mut Text,
        (
            With<StatusMoneyText>,
            Without<StatusRateText>,
            Without<StatusToolText>,
        ),
    >,
    mut rate_q: Query<
        &mut Text,
        (
            With<StatusRateText>,
            Without<StatusMoneyText>,
            Without<StatusToolText>,
        ),
    >,
    mut rate_color: Query<&mut TextColor, With<StatusRateText>>,
    mut tool_q: Query<
        &mut Text,
        (
            With<StatusToolText>,
            Without<StatusMoneyText>,
            Without<StatusRateText>,
        ),
    >,
    mut speed_btns: Query<(&SpeedButton, &Interaction, &mut BorderColor, &Children), With<Button>>,
    mut child_colors: Query<&mut TextColor, Without<StatusRateText>>,
) {
    cache.rate_cents_per_min = ledger.net_rate_cents_per_min();

    let money_str = format_money(money.cents());
    if money_str != cache.money {
        cache.money = money_str.clone();
        if let Ok(mut text) = money_q.single_mut() {
            *text = Text::new(money_str);
        }
    }

    let rate_str = format_rate(cache.rate_cents_per_min);
    if rate_str != cache.rate {
        cache.rate = rate_str.clone();
        if let Ok(mut text) = rate_q.single_mut() {
            *text = Text::new(rate_str);
        }
        if let Ok(mut color) = rate_color.single_mut() {
            *color = if cache.rate_cents_per_min > 0 {
                TextColor(OK)
            } else if cache.rate_cents_per_min < 0 {
                TextColor(WARN)
            } else {
                text_secondary()
            };
        }
    }

    let placing = train_tools.as_ref().is_some_and(|t| t.place_mode);
    let place_kind = train_tools.as_ref().map(|t| t.kind);
    let line_active = line_tools.as_ref().is_some_and(|l| l.active);
    let tool_str = tool_label(tools.tool, placing, place_kind, line_active).to_string();
    if tool_str != cache.tool {
        cache.tool = tool_str.clone();
        if let Ok(mut text) = tool_q.single_mut() {
            *text = Text::new(tool_str);
        }
    }

    let active_speed = if clock.paused {
        0u8
    } else {
        clock.speed_multiplier.max(1)
    };
    for (btn, interaction, mut border, children) in &mut speed_btns {
        let selected = btn.multiplier == active_speed;
        *border = if selected {
            BorderColor::all(HI)
        } else if matches!(interaction, Interaction::Hovered) {
            BorderColor::all(BALLAST_L)
        } else {
            BorderColor::all(OUTLINE)
        };
        for child in children.iter() {
            if let Ok(mut c) = child_colors.get_mut(child) {
                *c = if selected {
                    text_accent()
                } else {
                    text_primary()
                };
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

fn format_money(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    let dollars = abs / 100;
    let rem = abs % 100;
    format!("{sign}${dollars}.{rem:02}")
}

fn format_rate(cents_per_min: i64) -> String {
    if cents_per_min == 0 {
        return "$0/min".into();
    }
    let sign = if cents_per_min > 0 { "+" } else { "-" };
    let abs = cents_per_min.unsigned_abs();
    let dollars = abs / 100;
    let rem = abs % 100;
    if rem == 0 {
        format!("{sign}${dollars}/min")
    } else {
        format!("{sign}${dollars}.{rem:02}/min")
    }
}

fn tool_label(
    tool: BuildTool,
    placing: bool,
    kind: Option<TrainPlaceKind>,
    line_active: bool,
) -> &'static str {
    if line_active {
        return "Line";
    }
    if placing {
        return match kind.unwrap_or_default() {
            TrainPlaceKind::Transit => "Transit",
            TrainPlaceKind::Transport => "Transport",
        };
    }
    match tool {
        BuildTool::Build => "Build",
        BuildTool::Demolish => "Demolish",
    }
}

#[cfg(test)]
mod tests {
    use super::{format_money, format_rate};

    #[test]
    fn money_formats_cents_as_dollars() {
        assert_eq!(format_money(1_000_000), "$10000.00");
        assert_eq!(format_money(1050), "$10.50");
        assert_eq!(format_money(0), "$0.00");
    }

    #[test]
    fn rate_formats_signed() {
        assert_eq!(format_rate(0), "$0/min");
        assert_eq!(format_rate(34_000), "+$340/min");
        assert_eq!(format_rate(-500), "-$5/min");
    }
}
