//! The first payout, celebrated — the loop closing for the first time.
//!
//! Design 09 §7, touch three: *"a clear, warm moment when the first train
//! completes its first run … it should feel like something."* Brief 10 §6 says
//! what that moment is made of — *"a warm chime, a clear floating number, and a
//! Town Talk line"* — and is equally clear about what it is **not**: no screen
//! shake, no hit-stop, no zoom punch, no particles.
//!
//! # Division of labour with audio
//!
//! `audio::ui_sound` already plays the milestone clip on the session's first
//! positive money window, and that is the right owner for it. This module adds
//! the two channels the brief asks for that were missing — the number and the
//! Town Talk line — and deliberately does **not** write a second
//! [`UiCue::Milestone`](crate::audio::UiCue), which would double the one sound
//! in the game that is supposed to happen once.
//!
//! # Once per world, not once per player
//!
//! Unlike a hint, this is not knowledge the player carries between maps: it is
//! the first payout *of this railway*, and every new railway earns one. So it
//! is armed by the ledger itself — a world whose session income is still zero
//! has not had it — which also means a loaded save never re-celebrates a
//! payout it banked hours ago.

use bevy::prelude::*;
use rail_sim::{ComplaintEntry, ComplaintFeed, MoneyLedger, StationService, TalkKind};

use crate::palette::{BG1, HI, OK};
use crate::shell::ShellState;
use crate::ui::kit::{body_font, display_font, SPACE_1, SPACE_2, SPACE_3, STATUS_H};

/// How long the banner stays up in total.
const BANNER_SECONDS: f32 = 4.5;
/// How much of that is held at full strength before it starts to fade.
const BANNER_HOLD_SECONDS: f32 = 2.5;

/// Session-income watch, and the banner's countdown.
#[derive(Resource, Debug, Default)]
pub struct FirstPayout {
    /// `false` until the first observation, so a loaded save is read rather
    /// than reacted to.
    seeded: bool,
    last_income_cents: i64,
    seconds_left: f32,
    amount_cents: i64,
}

impl FirstPayout {
    /// `true` while the banner is on screen. Read by anything that wants to
    /// stay out of the moment's way — nothing does yet, and that is fine.
    #[allow(dead_code)]
    pub fn is_showing(&self) -> bool {
        self.seconds_left > 0.0
    }
}

#[derive(Component)]
pub(crate) struct PayoutBanner;

#[derive(Component)]
pub(crate) struct PayoutBannerTitle;

#[derive(Component)]
pub(crate) struct PayoutBannerAmount;

/// Spawn the banner once, hidden.
pub fn setup_payout_banner(mut commands: Commands) {
    commands
        .spawn((
            PayoutBanner,
            Node {
                position_type: PositionType::Absolute,
                // Directly under the balance in the status strip — brief 10 §6
                // wants the number *near the money*, not in the middle of the
                // screen where it would cover the world.
                top: Val::Px(STATUS_H + SPACE_1),
                left: Val::Px(SPACE_3),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(SPACE_1),
                padding: UiRect::axes(Val::Px(SPACE_2), Val::Px(SPACE_1)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::ZERO,
                display: Display::None,
                ..default()
            },
            BackgroundColor(BG1),
            BorderColor::all(HI),
            // Above the ledger panel: this is a moment, and it lasts seconds.
            ZIndex(14),
        ))
        .with_children(|banner| {
            banner.spawn((
                PayoutBannerTitle,
                Text::new("First payout"),
                display_font(),
                TextColor(HI),
            ));
            banner.spawn((
                PayoutBannerAmount,
                Text::new(""),
                body_font(),
                TextColor(OK),
            ));
        });
}

/// Watch session income and light the banner the first time it moves.
pub fn celebrate_the_first_payout(
    state: Res<State<ShellState>>,
    ledger: Res<MoneyLedger>,
    service: Res<StationService>,
    mut payout: ResMut<FirstPayout>,
    mut talk: ResMut<ComplaintFeed>,
) {
    if *state.get() != ShellState::Playing {
        return;
    }
    let income = ledger.session_income();

    // First look at this world: record, react to nothing. A restored save walks
    // in with income already banked and must not replay its opening moment.
    if !payout.seeded {
        payout.seeded = true;
        payout.last_income_cents = income;
        return;
    }
    // A new map resets the ledger, which re-arms this by itself.
    if income <= 0 {
        payout.last_income_cents = income;
        return;
    }
    if payout.last_income_cents > 0 {
        payout.last_income_cents = income;
        return;
    }

    payout.last_income_cents = income;
    payout.amount_cents = income;
    payout.seconds_left = BANNER_SECONDS;

    talk.push(ComplaintEntry {
        kind: TalkKind::Praise,
        peep_name: format!(
            "First payout - {}. The line is working.",
            format_cents(income)
        ),
        // Empty station name: the feed renders praise as "name · via station",
        // so a whole-sentence line needs the station slot left alone.
        station_name: String::new(),
        wait_minutes: 0,
        sim_tick: service.tick,
        peep_id: None,
        station_id: None,
        tile: None,
        count: 1,
    });
}

/// Count the banner down and fade it out. No motion, no scale, no particles —
/// brief 10 §6 rules all three out; the warmth is in the colour and the wait.
pub fn fade_payout_banner(
    time: Res<Time>,
    mut payout: ResMut<FirstPayout>,
    mut banner: Query<(&mut Node, &mut BackgroundColor, &mut BorderColor), With<PayoutBanner>>,
    mut title: Query<
        &mut TextColor,
        (With<PayoutBannerTitle>, Without<PayoutBannerAmount>),
    >,
    mut amount: Query<
        (&mut Text, &mut TextColor),
        (With<PayoutBannerAmount>, Without<PayoutBannerTitle>),
    >,
) {
    let Ok((mut node, mut bg, mut border)) = banner.single_mut() else {
        return;
    };
    if payout.seconds_left <= 0.0 {
        if node.display != Display::None {
            node.display = Display::None;
        }
        return;
    }
    payout.seconds_left = (payout.seconds_left - time.delta_secs()).max(0.0);
    node.display = Display::Flex;

    let alpha = banner_alpha(payout.seconds_left);
    bg.0 = BG1.with_alpha(alpha);
    *border = BorderColor::all(HI.with_alpha(alpha));
    if let Ok(mut colour) = title.single_mut() {
        colour.0 = HI.with_alpha(alpha);
    }
    if let Ok((mut text, mut colour)) = amount.single_mut() {
        let line = format!("+{} earned", format_cents(payout.amount_cents));
        if text.as_str() != line {
            *text = Text::new(line);
        }
        colour.0 = OK.with_alpha(alpha);
    }
}

/// Full strength for the hold, then a linear fade to nothing.
fn banner_alpha(seconds_left: f32) -> f32 {
    let fade = (BANNER_SECONDS - BANNER_HOLD_SECONDS).max(f32::EPSILON);
    (seconds_left / fade).clamp(0.0, 1.0)
}

fn format_cents(cents: i64) -> String {
    let abs = cents.unsigned_abs();
    format!("${}.{:02}", abs / 100, abs % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_banner_holds_before_it_fades() {
        assert_eq!(banner_alpha(BANNER_SECONDS), 1.0);
        assert_eq!(banner_alpha(BANNER_HOLD_SECONDS), 1.0, "still held");
        assert!(banner_alpha(BANNER_SECONDS - BANNER_HOLD_SECONDS) >= 0.99);
        assert!(banner_alpha(1.0) < 1.0, "fading by now");
        assert_eq!(banner_alpha(0.0), 0.0);
    }

    #[test]
    fn the_moment_lasts_long_enough_to_notice_and_not_long_enough_to_annoy() {
        let total = BANNER_SECONDS;
        let hold = BANNER_HOLD_SECONDS;
        assert!(total >= 3.0, "a warm moment, not a flash");
        assert!(total <= 8.0, "and then it gets out of the way");
        assert!(hold < total, "there has to be a fade to fade");
    }

    #[test]
    fn money_reads_as_dollars_and_cents() {
        assert_eq!(format_cents(500), "$5.00");
        assert_eq!(format_cents(2_000), "$20.00");
        assert_eq!(format_cents(1_234), "$12.34");
    }

    #[test]
    fn a_fresh_world_is_armed_and_a_loaded_one_is_not() {
        // The ledger is the arming mechanism: zero income means the loop has
        // not closed yet, whatever else is true of the world.
        let mut fresh = MoneyLedger::default();
        assert_eq!(fresh.session_income(), 0);
        fresh.record(rail_sim::MoneyCategory::Fares, 500);
        assert_eq!(fresh.session_income(), 500);

        // Construction spend must not look like income and pre-arm anything.
        let mut spending = MoneyLedger::default();
        spending.record(rail_sim::MoneyCategory::Construction, -12_000);
        assert_eq!(spending.session_income(), 0);
    }

    #[test]
    fn the_banner_is_not_showing_until_something_is_earned() {
        let payout = FirstPayout::default();
        assert!(!payout.is_showing());
    }
}
