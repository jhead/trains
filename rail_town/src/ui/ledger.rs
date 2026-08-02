//! Ledger panel — income vs expense by category + recent net trend.
//!
//! Toggle with `L` or the status-strip Ledger button.

use bevy::prelude::*;
use rail_sim::{MoneyCategory, MoneyLedger};

use crate::palette::{BALLAST_L, BG1, HI, OUTLINE, RAIL_L};
use crate::ui::kit::{
    body_font, micro_font, panel_node, text_accent, text_primary, text_secondary, FONT_BODY,
    SPACE_2, SPACE_3, STATUS_H,
};

#[derive(Resource, Debug, Default)]
pub struct LedgerPanelState {
    pub open: bool,
}

#[derive(Component)]
pub struct LedgerPanelRoot;

#[derive(Component)]
pub struct LedgerBodyText;

#[derive(Component)]
pub struct LedgerToggleButton;

#[derive(Resource, Debug, Default)]
pub(crate) struct LedgerUiCache {
    body: String,
}

pub fn setup_ledger_ui(mut commands: Commands) {
    commands.init_resource::<LedgerPanelState>();
    commands.insert_resource(LedgerUiCache::default());

    let (node, bg, border) = panel_node(Node {
        position_type: PositionType::Absolute,
        top: Val::Px(STATUS_H + SPACE_2),
        left: Val::Px(SPACE_3),
        width: Val::Px(280.0),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(SPACE_2),
        padding: UiRect::all(Val::Px(SPACE_2)),
        display: Display::None,
        ..default()
    });

    commands
        .spawn((
            LedgerPanelRoot,
            node,
            bg,
            border,
            ZIndex(12),
        ))
        .with_children(|panel| {
            panel
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    width: Val::Percent(100.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((Text::new("Ledger"), body_font(), text_accent()));
                    row.spawn((Text::new("L"), micro_font(), text_secondary()));
                });
            panel.spawn((
                LedgerBodyText,
                Text::new("No activity yet."),
                body_font(),
                text_primary(),
            ));
        });
}

pub fn ledger_toggle_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<LedgerPanelState>,
    buttons: Query<&Interaction, (Changed<Interaction>, With<LedgerToggleButton>)>,
) {
    let mut toggle = keys.just_pressed(KeyCode::KeyL);
    for interaction in &buttons {
        if *interaction == Interaction::Pressed {
            toggle = true;
        }
    }
    if keys.just_pressed(KeyCode::Escape) && state.open {
        state.open = false;
        return;
    }
    if toggle {
        state.open = !state.open;
    }
}

pub fn update_ledger_panel(
    state: Res<LedgerPanelState>,
    ledger: Res<MoneyLedger>,
    mut cache: ResMut<LedgerUiCache>,
    mut root_q: Query<&mut Node, With<LedgerPanelRoot>>,
    mut body_q: Query<&mut Text, With<LedgerBodyText>>,
) {
    if let Ok(mut node) = root_q.single_mut() {
        node.display = if state.open {
            Display::Flex
        } else {
            Display::None
        };
    }
    if !state.open {
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
            format_cents(session),
            format_cents(recent)
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
            format_cents(session),
            format_cents(recent)
        ));
    }
    let net = ledger.session_income() - ledger.session_expense();
    lines.push(format!(
        "Net session    {}   rate {}",
        format_signed_cents(net),
        format_rate(ledger.net_rate_cents_per_min())
    ));
    lines.push(format!("Trend {}", sparkline(ledger)));
    lines.join("\n")
}

fn sparkline(ledger: &MoneyLedger) -> String {
    let vals: Vec<i64> = ledger.history_nets().collect();
    if vals.is_empty() {
        return "·".into();
    }
    let max_abs = vals.iter().map(|v| v.unsigned_abs()).max().unwrap_or(1).max(1);
    const BARS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    vals.iter()
        .map(|v| {
            if *v == 0 {
                '·'
            } else {
                let idx = ((*v).unsigned_abs() * (BARS.len() as u64 - 1) / max_abs) as usize;
                BARS[idx.min(BARS.len() - 1)]
            }
        })
        .collect()
}

fn format_cents(cents: i64) -> String {
    let abs = cents.unsigned_abs();
    let dollars = abs / 100;
    let rem = abs % 100;
    format!("${dollars}.{rem:02}")
}

fn format_signed_cents(cents: i64) -> String {
    if cents == 0 {
        return "$0.00".into();
    }
    let sign = if cents > 0 { "+" } else { "-" };
    format!("{sign}{}", format_cents(cents.abs()))
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

/// Status-strip control that opens the ledger.
pub fn spawn_ledger_toggle(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            Button,
            LedgerToggleButton,
            Node {
                padding: UiRect::axes(Val::Px(SPACE_2), Val::Px(2.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::ZERO,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                margin: UiRect::left(Val::Auto),
                ..default()
            },
            BackgroundColor(BG1),
            BorderColor::all(OUTLINE),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new("Ledger L"),
                TextFont::from_font_size(FONT_BODY),
                TextColor(RAIL_L),
            ));
        });
}

pub fn update_ledger_toggle_visual(
    state: Res<LedgerPanelState>,
    mut q: Query<(&Interaction, &mut BorderColor), With<LedgerToggleButton>>,
) {
    for (interaction, mut border) in &mut q {
        *border = if state.open {
            BorderColor::all(HI)
        } else if matches!(*interaction, Interaction::Hovered) {
            BorderColor::all(BALLAST_L)
        } else {
            BorderColor::all(OUTLINE)
        };
    }
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
}
