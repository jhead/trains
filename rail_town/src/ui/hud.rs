//! Top-left HUD: money, pause/speed, tool mode, short help.

use bevy::prelude::*;
use rail_sim::{Money, SimClock};

use crate::track::{BuildTool, TrackToolState};
use crate::trains::{TrainPlaceKind, TrainToolState};

#[derive(Component)]
pub struct HudRoot;

#[derive(Component)]
pub struct HudMoneyText;

#[derive(Component)]
pub struct HudClockText;

#[derive(Component)]
pub struct HudToolText;

#[derive(Component)]
pub struct HudHelpText;

/// Flash money text briefly when the balance changes.
#[derive(Resource, Debug, Clone, Copy)]
pub(crate) struct MoneyFlash {
    last_cents: i64,
    frames_left: u8,
}

pub fn setup_hud(mut commands: Commands, money: Res<Money>) {
    commands.insert_resource(MoneyFlash {
        last_cents: money.cents(),
        frames_left: 0,
    });

    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(10.0),
                left: Val::Px(12.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.0),
                padding: UiRect::all(Val::Px(8.0)),
                border_radius: BorderRadius::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.06, 0.08, 0.72)),
        ))
        .with_children(|parent| {
            parent.spawn((
                HudMoneyText,
                Text::new(format_money(money.cents())),
                TextFont::from_font_size(22.0),
                TextColor(Color::srgb(0.92, 0.95, 0.78)),
            ));
            parent.spawn((
                HudClockText,
                Text::new("Speed 1x"),
                TextFont::from_font_size(16.0),
                TextColor(Color::srgb(0.85, 0.88, 0.92)),
            ));
            parent.spawn((
                HudToolText,
                Text::new("Tool: Build"),
                TextFont::from_font_size(16.0),
                TextColor(Color::srgb(0.75, 0.82, 0.95)),
            ));
            parent.spawn((
                HudHelpText,
                Text::new(help_line(BuildTool::Build, false)),
                TextFont::from_font_size(13.0),
                TextColor(Color::srgb(0.65, 0.68, 0.72)),
            ));
        });
}

pub fn update_hud(
    money: Res<Money>,
    clock: Res<SimClock>,
    tools: Res<TrackToolState>,
    train_tools: Option<Res<TrainToolState>>,
    mut flash: ResMut<MoneyFlash>,
    mut money_q: Query<&mut Text, (With<HudMoneyText>, Without<HudClockText>, Without<HudToolText>, Without<HudHelpText>)>,
    mut clock_q: Query<&mut Text, (With<HudClockText>, Without<HudMoneyText>, Without<HudToolText>, Without<HudHelpText>)>,
    mut tool_q: Query<&mut Text, (With<HudToolText>, Without<HudMoneyText>, Without<HudClockText>, Without<HudHelpText>)>,
    mut help_q: Query<&mut Text, (With<HudHelpText>, Without<HudMoneyText>, Without<HudClockText>, Without<HudToolText>)>,
    mut money_color: Query<&mut TextColor, With<HudMoneyText>>,
) {
    let cents = money.cents();
    if cents != flash.last_cents {
        flash.last_cents = cents;
        flash.frames_left = 18;
    }
    if flash.frames_left > 0 {
        flash.frames_left -= 1;
    }

    if let Ok(mut text) = money_q.single_mut() {
        *text = Text::new(format_money(cents));
    }
    if let Ok(mut color) = money_color.single_mut() {
        *color = if flash.frames_left > 0 {
            TextColor(Color::srgb(1.0, 0.92, 0.45))
        } else {
            TextColor(Color::srgb(0.92, 0.95, 0.78))
        };
    }

    let placing = train_tools.as_ref().is_some_and(|t| t.place_mode);
    let place_kind = train_tools.as_ref().map(|t| t.kind);

    if let Ok(mut text) = clock_q.single_mut() {
        *text = Text::new(format_clock(*clock));
    }
    if let Ok(mut text) = tool_q.single_mut() {
        *text = Text::new(format!(
            "Tool: {}",
            tool_label(tools.tool, placing, place_kind)
        ));
    }
    if let Ok(mut text) = help_q.single_mut() {
        *text = Text::new(help_line(tools.tool, placing));
    }
}

fn format_money(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    let dollars = abs / 100;
    let rem = abs % 100;
    format!("{sign}${dollars}.{rem:02}")
}

fn format_clock(clock: SimClock) -> String {
    if clock.paused {
        "Paused".to_string()
    } else {
        format!("Speed {}x", clock.speed_multiplier.max(1))
    }
}

fn tool_label(
    tool: BuildTool,
    placing: bool,
    kind: Option<TrainPlaceKind>,
) -> &'static str {
    if placing {
        return match kind.unwrap_or_default() {
            TrainPlaceKind::Transit => "Place transit",
            TrainPlaceKind::Transport => "Place transport",
        };
    }
    match tool {
        BuildTool::Build => "Build",
        BuildTool::Demolish => "Demolish",
    }
}

fn help_line(tool: BuildTool, placing: bool) -> String {
    if placing {
        return "Click station to place · B/X back to track · Space pause".into();
    }
    match tool {
        BuildTool::Build => {
            "B build · X demolish · T/G trains · Space pause · 1/3 speed".into()
        }
        BuildTool::Demolish => {
            "X demolish · B build · T/G trains · Space pause · click to refund".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format_money;

    #[test]
    fn money_formats_cents_as_dollars() {
        assert_eq!(format_money(1_000_000), "$10000.00");
        assert_eq!(format_money(1050), "$10.50");
        assert_eq!(format_money(0), "$0.00");
    }
}
