//! The confirm dialog — one modal, for actions that cost the player something
//! they cannot see from the cursor.
//!
//! Binding standard: [`docs/design/03-ui-system.md`](../../../docs/design/03-ui-system.md),
//! and [04 — Building & Tools](../../../docs/design/04-building-and-tools.md) §4:
//!
//! > Removing track that a train is currently on, or that is the only route
//! > serving a station, asks for confirmation and **names the consequence**.
//!
//! That is the whole rule this module exists to keep. A confirm that says
//! "Are you sure?" is a speed bump; a confirm that says *"Riverside Loop and 1
//! other line stop here"* is information the player did not have, which is why
//! [`ConfirmPrompt::body`] is written by the caller who knows the consequence
//! rather than by anything in here.
//!
//! # Shape
//!
//! - **One at a time.** [`ConfirmDialog`] holds at most one prompt. Asking
//!   again replaces it, so a dialog can never stack on a dialog.
//! - **Cancel is the default.** The cursor starts on Cancel, `Esc` cancels, and
//!   `Enter` activates whatever is selected — so the safe answer is the one a
//!   player gets for pressing anything reflexively.
//! - **Modal.** A full-screen [`WorldClickBlocker`] sits under the panel, so
//!   `UiBlocksWorld` reads `true` and no build tool fires through it.
//! - **`Esc` first.** The key handler runs at the head of
//!   [`WindowEscSet`](super::WindowEscSet) and consumes the press, ahead of the
//!   window stack and the pause menu — 03 §10.1's one layer per press.
//!
//! The dialog does not perform the action. It writes [`ConfirmAccepted`] and the
//! tool that asked does the work, because only that tool knows how to turn it
//! into a command.

use bevy::prelude::*;
use rail_sim::StationId;

use crate::palette::{BG0, WARN};
use crate::ui::kit::{
    body_font, chrome_button_node, control_border, micro_font, panel_node, text_accent,
    text_primary, text_secondary, WorldClickBlocker, SPACE_1, SPACE_2, SPACE_3, TITLE_BAR_H,
};

/// Above every window (`100 + depth`) and every shell screen (`100`).
const CONFIRM_Z: i32 = 400;

/// What the player is being asked to agree to.
///
/// One variant per confirmable action. The dialog only carries it; the tool
/// that raised the prompt reads it back out of [`ConfirmAccepted`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    /// Lift a station that lines still call at (04 §4).
    DemolishStation(StationId),
}

/// A question on screen: what is at stake, and what saying yes is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmPrompt {
    pub title: String,
    /// The consequence, in plain language. May run to two lines.
    pub body: String,
    /// Label on the destructive button — a verb, never "OK".
    pub confirm: String,
    pub action: ConfirmAction,
}

/// Which button the keyboard is on. Cancel is always where it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfirmChoice {
    #[default]
    Cancel,
    Confirm,
}

/// The one open prompt, if any.
#[derive(Resource, Debug, Default)]
pub struct ConfirmDialog {
    prompt: Option<ConfirmPrompt>,
    choice: ConfirmChoice,
}

impl ConfirmDialog {
    /// Put a question on screen, replacing any question already there.
    pub fn ask(&mut self, prompt: ConfirmPrompt) {
        self.prompt = Some(prompt);
        self.choice = ConfirmChoice::default();
    }

    pub fn is_open(&self) -> bool {
        self.prompt.is_some()
    }

    pub fn prompt(&self) -> Option<&ConfirmPrompt> {
        self.prompt.as_ref()
    }

    pub fn choice(&self) -> ConfirmChoice {
        self.choice
    }

    /// Close without acting. The safe exit, and the one `Esc` takes.
    pub fn cancel(&mut self) {
        if self.prompt.is_some() {
            self.prompt = None;
            self.choice = ConfirmChoice::default();
        }
    }

    /// Close and hand the action back to whoever asked.
    fn accept(&mut self) -> Option<ConfirmAction> {
        let action = self.prompt.take().map(|p| p.action);
        self.choice = ConfirmChoice::default();
        action
    }
}

/// The player said yes. Written once, on the frame the dialog closes.
#[derive(Message, Debug, Clone, Copy)]
pub struct ConfirmAccepted(pub ConfirmAction);

/// Root of the modal (scrim + panel).
#[derive(Component, Debug, Clone, Copy)]
pub struct ConfirmRoot;

/// A button in the dialog.
#[derive(Component, Debug, Clone, Copy)]
pub struct ConfirmButton(pub ConfirmChoice);

/// Build the modal when a prompt appears, tear it down when one closes.
///
/// Runs off `ConfirmDialog` change detection, so an idle frame does nothing at
/// all — and a prompt that is replaced rebuilds with the new text.
pub fn sync_confirm_dialog(
    mut commands: Commands,
    dialog: Res<ConfirmDialog>,
    roots: Query<Entity, With<ConfirmRoot>>,
) {
    if !dialog.is_changed() {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
    let Some(prompt) = dialog.prompt() else {
        return;
    };

    commands
        .spawn((
            ConfirmRoot,
            WorldClickBlocker,
            Interaction::default(),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            // 03 §5: a modal dims what is behind it rather than hiding it.
            BackgroundColor(BG0.with_alpha(0.5)),
            ZIndex(CONFIRM_Z),
        ))
        .with_children(|screen| {
            let (node, bg, border) = panel_node(Node {
                width: Val::Px(280.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(SPACE_2),
                padding: UiRect::all(Val::Px(SPACE_3)),
                ..default()
            });
            screen.spawn((node, bg, border)).with_children(|panel| {
                panel
                    .spawn(Node {
                        height: Val::Px(TITLE_BAR_H),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|bar| {
                        bar.spawn((
                            Text::new(prompt.title.clone()),
                            body_font(),
                            text_accent(),
                        ));
                    });
                panel.spawn((
                    Text::new(prompt.body.clone()),
                    body_font(),
                    text_primary(),
                ));
                panel
                    .spawn(Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(SPACE_2),
                        justify_content: JustifyContent::FlexEnd,
                        ..default()
                    })
                    .with_children(|row| {
                        // Cancel sits first, and holds the cursor: the safe
                        // answer is the one a reflex press gives you.
                        spawn_choice(row, ConfirmChoice::Cancel, "Cancel");
                        spawn_choice(row, ConfirmChoice::Confirm, &prompt.confirm);
                    });
                panel.spawn((
                    Text::new("Esc cancels   Enter picks"),
                    micro_font(),
                    text_secondary(),
                ));
            });
        });
}

fn spawn_choice(parent: &mut ChildSpawnerCommands, choice: ConfirmChoice, label: &str) {
    let (node, bg, border) = chrome_button_node(SPACE_2, SPACE_1);
    parent
        .spawn((Button, ConfirmButton(choice), node, bg, border))
        .with_children(|b| {
            b.spawn((
                Text::new(label.to_string()),
                body_font(),
                if choice == ConfirmChoice::Confirm {
                    TextColor(WARN)
                } else {
                    text_primary()
                },
            ));
        });
}

/// Paint the cursor: the selected button takes the `hi` edge.
///
/// Colour never carries the state alone — the destructive verb is also the only
/// label in `warn`, and the hint line spells out both keys.
pub fn paint_confirm_dialog(
    dialog: Res<ConfirmDialog>,
    mut buttons: Query<(&ConfirmButton, &Interaction, &mut BorderColor)>,
) {
    for (button, interaction, mut border) in &mut buttons {
        let hovered = matches!(interaction, Interaction::Hovered | Interaction::Pressed);
        *border = control_border(dialog.choice() == button.0, hovered);
    }
}

/// `Esc` cancels, `Enter` takes the selection, arrows / `Tab` move it.
///
/// Runs at the head of [`WindowEscSet`](super::WindowEscSet) and clears the key
/// it used, so a press that answered the dialog never also closes a window or
/// opens the pause menu.
pub fn confirm_dialog_keys(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut dialog: ResMut<ConfirmDialog>,
    mut accepted: MessageWriter<ConfirmAccepted>,
) {
    if !dialog.is_open() {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        dialog.cancel();
        keys.clear_just_pressed(KeyCode::Escape);
        return;
    }
    for key in [
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
        KeyCode::Tab,
        KeyCode::ArrowUp,
        KeyCode::ArrowDown,
    ] {
        if keys.just_pressed(key) {
            dialog.choice = match dialog.choice {
                ConfirmChoice::Cancel => ConfirmChoice::Confirm,
                ConfirmChoice::Confirm => ConfirmChoice::Cancel,
            };
            keys.clear_just_pressed(key);
        }
    }
    for key in [KeyCode::Enter, KeyCode::NumpadEnter] {
        if !keys.just_pressed(key) {
            continue;
        }
        keys.clear_just_pressed(key);
        match dialog.choice {
            ConfirmChoice::Cancel => dialog.cancel(),
            ConfirmChoice::Confirm => {
                if let Some(action) = dialog.accept() {
                    accepted.write(ConfirmAccepted(action));
                }
            }
        }
    }
}

/// Clicks on the two buttons.
///
/// `ConfirmButton` only ever sits on a [`Button`], so the filter is the change
/// detection alone.
pub fn confirm_dialog_clicks(
    interactions: Query<(&Interaction, &ConfirmButton), Changed<Interaction>>,
    mut dialog: ResMut<ConfirmDialog>,
    mut accepted: MessageWriter<ConfirmAccepted>,
) {
    for (interaction, button) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match button.0 {
            ConfirmChoice::Cancel => dialog.cancel(),
            ConfirmChoice::Confirm => {
                if let Some(action) = dialog.accept() {
                    accepted.write(ConfirmAccepted(action));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt() -> ConfirmPrompt {
        ConfirmPrompt {
            title: "Demolish station".into(),
            body: "Riverside Loop stops here. Demolish and drop the stop?".into(),
            confirm: "Demolish".into(),
            action: ConfirmAction::DemolishStation(StationId(1)),
        }
    }

    #[test]
    fn a_prompt_opens_on_cancel() {
        let mut dialog = ConfirmDialog::default();
        assert!(!dialog.is_open());
        dialog.ask(prompt());
        assert!(dialog.is_open());
        assert_eq!(
            dialog.choice(),
            ConfirmChoice::Cancel,
            "the safe answer holds the cursor"
        );
    }

    #[test]
    fn cancelling_closes_without_an_action() {
        let mut dialog = ConfirmDialog::default();
        dialog.ask(prompt());
        dialog.cancel();
        assert!(!dialog.is_open());
        assert!(dialog.prompt().is_none());
    }

    #[test]
    fn accepting_hands_back_the_action_once() {
        let mut dialog = ConfirmDialog::default();
        dialog.ask(prompt());
        assert_eq!(
            dialog.accept(),
            Some(ConfirmAction::DemolishStation(StationId(1)))
        );
        assert!(!dialog.is_open());
        assert_eq!(dialog.accept(), None, "a closed dialog acts no further");
    }

    #[test]
    fn asking_again_replaces_the_question_and_resets_the_cursor() {
        let mut dialog = ConfirmDialog::default();
        dialog.ask(prompt());
        dialog.choice = ConfirmChoice::Confirm;
        dialog.ask(ConfirmPrompt {
            action: ConfirmAction::DemolishStation(StationId(2)),
            ..prompt()
        });
        assert_eq!(dialog.choice(), ConfirmChoice::Cancel);
        assert_eq!(
            dialog.prompt().map(|p| p.action),
            Some(ConfirmAction::DemolishStation(StationId(2))),
            "one dialog at a time, and it is the newest question"
        );
    }

    /// The bitmap font has a small charset (see the playtest note in
    /// `docs/BURNDOWN.md`): anything outside ASCII draws as tofu.
    #[test]
    fn the_dialog_furniture_is_ascii() {
        for text in ["Cancel", "Esc cancels   Enter picks"] {
            assert!(text.is_ascii(), "{text} would draw as tofu");
        }
    }
}
