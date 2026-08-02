//! Contextual hints — a small chip by the toolbar, once each, never blocking.
//!
//! Design 09 §7 gives the shape and the one worked example:
//!
//! > The first time the player selects the Build tool, a small non-modal chip
//! > near the toolbar: *"Drag to lay track."* It appears once, never returns,
//! > and never blocks anything.
//!
//! Three rules are load-bearing and all three are enforced here rather than
//! trusted to callers:
//!
//! 1. **Once each.** A hint is marked seen the moment it is *shown*, not when
//!    it is dismissed, so a player who ignores one still never sees it twice.
//! 2. **Never blocking.** The chip body carries no `Button` and no
//!    `WorldClickBlocker`, so a click through it reaches the world exactly as
//!    if the chip were not there. Only the `×` is a control.
//! 3. **Contextual.** Each hint waits for the moment it answers. Nothing here
//!    fires on a timer or in a sequence — there is no sequence.
//!
//! One hint is on screen at a time. If two moments coincide, the earlier one in
//! [`Hint::ALL`] wins and the other waits for its next chance, which keeps the
//! corner of the screen quiet.

use bevy::prelude::*;
use rail_sim::{track_for_station, MoneyLedger, StationRegistry, TrackNetwork, TrainYard};

use crate::palette::{BALLAST_L, BG1, HI, OUTLINE, RAIL_L};
use crate::shell::ShellState;
use crate::track::{BuildTool, TrackToolState};
use crate::ui::kit::{micro_font, SPACE_1, SPACE_2, TOOL_SLOT};

use super::Onboarding;

/// Seconds a chip stays up before retiring itself. Long enough to read twice,
/// short enough that an ignored hint does not become furniture.
const CHIP_SECONDS: f32 = 9.0;

/// Chip sits directly above the toolbar, clear of it.
const CHIP_BOTTOM: f32 = SPACE_2 + TOOL_SLOT + SPACE_2;

/// The contextual hints this game has. Adding one is an enum variant, a key, a
/// line of text, and one arm in [`moment_for`] — and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    /// The brief's own example.
    Build,
    /// Track joins two stops but nothing is running on it.
    Train,
    /// The railway is losing money and the player may not know where to look.
    Ledger,
}

impl Hint {
    pub const ALL: &'static [Self] = &[Self::Build, Self::Train, Self::Ledger];

    /// Storage key in `onboarding.ron`. Stable — renaming one re-shows a hint.
    pub fn key(self) -> &'static str {
        match self {
            Self::Build => "hint_build",
            Self::Train => "hint_train",
            Self::Ledger => "hint_ledger",
        }
    }

    /// The whole hint. One short sentence, no exclamation, no second sentence.
    pub fn text(self) -> &'static str {
        match self {
            Self::Build => "Drag to lay track.",
            // Rails *at* two stops, not necessarily joined: the check is per
            // stop, and a hint must never claim more than it knows.
            Self::Train => "Rails at two stops. T buys a train.",
            Self::Ledger => "Earning less than it costs. L for why.",
        }
    }
}

/// Marker on the chip root.
#[derive(Component)]
pub struct HintChip;

/// Marker on the chip's sentence.
#[derive(Component)]
pub(crate) struct HintChipText;

/// Marker on the dismiss control.
#[derive(Component)]
pub(crate) struct HintChipDismiss;

/// The chip currently up, if any.
#[derive(Resource, Debug, Default)]
pub struct ActiveHint {
    hint: Option<Hint>,
    seconds_left: f32,
}

/// Spawn the chip once, hidden. It is reused rather than respawned so a hint
/// never costs a UI rebuild mid-drag.
pub fn setup_hint_chip(mut commands: Commands) {
    commands.init_resource::<ActiveHint>();
    commands
        .spawn((
            HintChip,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(CHIP_BOTTOM),
                left: Val::Percent(50.0),
                // Centred by pulling back half the chip's own width, the same
                // way the toolbar centres itself.
                margin: UiRect::left(Val::Px(-110.0)),
                width: Val::Px(220.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(SPACE_2),
                padding: UiRect::axes(Val::Px(SPACE_2), Val::Px(SPACE_1)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::ZERO,
                display: Display::None,
                ..default()
            },
            BackgroundColor(BG1),
            BorderColor::all(OUTLINE),
            ZIndex(9),
        ))
        .with_children(|chip| {
            chip.spawn((
                HintChipText,
                Text::new(""),
                micro_font(),
                TextColor(RAIL_L),
                Node {
                    flex_grow: 1.0,
                    ..default()
                },
            ));
            // The only control on the chip, so the rest of it stays click-through.
            chip.spawn((
                Button,
                HintChipDismiss,
                Node {
                    padding: UiRect::axes(Val::Px(SPACE_1), Val::Px(0.0)),
                    border_radius: BorderRadius::ZERO,
                    ..default()
                },
                BackgroundColor(BG1),
            ))
            .with_children(|b| {
                b.spawn((Text::new("x"), micro_font(), TextColor(BALLAST_L)));
            });
        });
}

/// Raise the first hint whose moment has arrived and that the player has not
/// already had.
#[allow(clippy::too_many_arguments)]
pub fn watch_for_hint_moments(
    state: Res<State<ShellState>>,
    time: Res<Time>,
    tools: Res<TrackToolState>,
    network: Res<TrackNetwork>,
    stations: Res<StationRegistry>,
    ledger: Res<MoneyLedger>,
    yard: Res<TrainYard>,
    trains: Query<(), With<rail_sim::Train>>,
    mut onboarding: ResMut<Onboarding>,
    mut active: ResMut<ActiveHint>,
) {
    // Retire whatever is up, on real time — hints are interface, not world, so
    // they do not freeze when the sim is paused.
    if active.hint.is_some() {
        active.seconds_left -= time.delta_secs();
        if active.seconds_left <= 0.0 {
            active.hint = None;
        }
        return;
    }
    if *state.get() != ShellState::Playing {
        return;
    }

    let running = trains.iter().count() + yard.unplaced().len();
    let served = stations
        .iter()
        .filter(|s| track_for_station(&network, s.tile, s.layer).is_some())
        .count();

    for hint in Hint::ALL.iter().copied() {
        if onboarding.has_seen(hint) || !moment_for(hint, &tools, &network, &ledger, served, running)
        {
            continue;
        }
        // Marked on *show*, so an ignored hint is still spent.
        onboarding.mark_seen(hint);
        active.hint = Some(hint);
        active.seconds_left = CHIP_SECONDS;
        return;
    }
}

/// Has this hint's moment arrived?
///
/// Each condition is the question the hint answers, and nothing else. A hint
/// with no moment would be a tutorial step, which this game does not have.
fn moment_for(
    hint: Hint,
    tools: &TrackToolState,
    network: &TrackNetwork,
    ledger: &MoneyLedger,
    served_stations: usize,
    running_stock: usize,
) -> bool {
    match hint {
        // The Build tool is where a new game starts, so its moment is the first
        // frame of play with nothing built yet — which is exactly when "drag to
        // lay track" is the sentence the player needs.
        Hint::Build => tools.tool == BuildTool::Build && network.is_empty(),
        // Rails reach two stops and nothing is running on them.
        Hint::Train => served_stations >= 2 && running_stock == 0,
        // A railway that *earns* and still loses money. Requiring income first
        // is what keeps this off the screen during the opening minute, when the
        // rate is negative purely because the player just paid for some track.
        Hint::Ledger => {
            !network.is_empty()
                && ledger.session_income() > 0
                && ledger.net_rate_cents_per_min() < 0
        }
    }
}

/// Show or hide the chip to match [`ActiveHint`].
pub fn paint_hint_chip(
    active: Res<ActiveHint>,
    mut chip: Query<&mut Node, With<HintChip>>,
    mut text: Query<&mut Text, With<HintChipText>>,
) {
    let Ok(mut node) = chip.single_mut() else {
        return;
    };
    let wanted = if active.hint.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    if node.display != wanted {
        node.display = wanted;
    }
    if let (Some(hint), Ok(mut text)) = (active.hint, text.single_mut()) {
        let line = hint.text();
        if text.as_str() != line {
            *text = Text::new(line);
        }
    }
}

/// The `×` puts the chip away immediately, and paints its own hover.
pub fn dismiss_hint_chip(
    mut buttons: Query<(&Interaction, &mut BackgroundColor), With<HintChipDismiss>>,
    mut active: ResMut<ActiveHint>,
) {
    for (interaction, mut bg) in &mut buttons {
        if *interaction == Interaction::Pressed {
            active.hint = None;
        }
        let wanted = if matches!(interaction, Interaction::Hovered | Interaction::Pressed) {
            HI.with_alpha(0.2)
        } else {
            BG1
        };
        if bg.0 != wanted {
            bg.0 = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(tool: BuildTool) -> TrackToolState {
        TrackToolState {
            tool,
            ..TrackToolState::default()
        }
    }

    /// A railway that has earned something and is still underwater.
    fn losing() -> MoneyLedger {
        let mut ledger = MoneyLedger::default();
        ledger.record(rail_sim::MoneyCategory::Fares, 500);
        ledger.record(rail_sim::MoneyCategory::TrackMaintenance, -10_000);
        ledger.on_sim_secs(rail_sim::LEDGER_SAMPLE_SIM_SECS);
        ledger
    }

    /// Money going out and none coming in — the opening minute.
    fn only_spending() -> MoneyLedger {
        let mut ledger = MoneyLedger::default();
        ledger.record(rail_sim::MoneyCategory::Construction, -12_000);
        ledger.on_sim_secs(rail_sim::LEDGER_SAMPLE_SIM_SECS);
        ledger
    }

    fn some_track() -> TrackNetwork {
        let terrain = rail_sim::TrackTerrain::new(8, 8, (0..64).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut money = rail_sim::Money::new(500_000);
        let mut ledger = MoneyLedger::default();
        rail_sim::track::try_place_track(
            &mut network,
            &mut money,
            &mut ledger,
            &terrain,
            rail_sim::TileCoord { x: 1, y: 1 },
            rail_sim::GROUND_LAYER,
        )
        .unwrap();
        network
    }

    #[test]
    fn the_build_hint_waits_for_a_world_with_nothing_in_it() {
        let empty = TrackNetwork::new();
        assert!(moment_for(
            Hint::Build,
            &tools(BuildTool::Build),
            &empty,
            &MoneyLedger::default(),
            0,
            0
        ));
        assert!(
            !moment_for(
                Hint::Build,
                &tools(BuildTool::Demolish),
                &empty,
                &MoneyLedger::default(),
                0,
                0
            ),
            "it is the Build tool's hint"
        );
        assert!(
            !moment_for(
                Hint::Build,
                &tools(BuildTool::Build),
                &some_track(),
                &MoneyLedger::default(),
                0,
                0
            ),
            "somebody who has already laid track does not need telling"
        );
    }

    #[test]
    fn the_train_hint_waits_until_there_is_something_to_run_on() {
        let network = some_track();
        let ledger = MoneyLedger::default();
        assert!(!moment_for(Hint::Train, &tools(BuildTool::Build), &network, &ledger, 1, 0));
        assert!(moment_for(Hint::Train, &tools(BuildTool::Build), &network, &ledger, 2, 0));
        assert!(
            !moment_for(Hint::Train, &tools(BuildTool::Build), &network, &ledger, 2, 1),
            "a player who already bought a train knows how"
        );
    }

    #[test]
    fn the_ledger_hint_waits_for_an_actual_loss_on_an_actual_railway() {
        assert!(!moment_for(
            Hint::Ledger,
            &tools(BuildTool::Build),
            &TrackNetwork::new(),
            &losing(),
            0,
            0
        ));
        assert!(moment_for(
            Hint::Ledger,
            &tools(BuildTool::Build),
            &some_track(),
            &losing(),
            0,
            0
        ));
        assert!(
            !moment_for(
                Hint::Ledger,
                &tools(BuildTool::Build),
                &some_track(),
                &MoneyLedger::default(),
                0,
                0
            ),
            "breaking even is not a problem to point at"
        );
        assert!(
            !moment_for(
                Hint::Ledger,
                &tools(BuildTool::Build),
                &some_track(),
                &only_spending(),
                0,
                0
            ),
            "paying for the first stretch of track is not a warning sign"
        );
    }

    #[test]
    fn every_hint_is_one_short_sentence() {
        for hint in Hint::ALL {
            let text = hint.text();
            assert!(text.len() <= 48, "{hint:?} is a lecture: {text:?}");
            assert!(text.ends_with('.'), "{hint:?} is not a sentence");
            assert!(!text.contains('!'), "{hint:?} shouts");
        }
    }

    #[test]
    fn the_chip_sits_clear_of_the_toolbar() {
        // Overlapping the toolbar would cover the very buttons it points at.
        let chip = CHIP_BOTTOM;
        let toolbar_top = SPACE_2 + TOOL_SLOT;
        assert!(chip >= toolbar_top, "{chip} overlaps the toolbar");
    }
}
