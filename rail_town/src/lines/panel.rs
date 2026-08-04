//! Left-side lines panel — list, strip diagram, crew, and the remove verb.
//!
//! # Nothing here fails quietly
//!
//! This panel used to have exactly one interactive control ("Assign selected
//! train") and exactly one response to being pressed with nothing selected:
//! none. A playtester pressed it, nothing happened, and there was no way to
//! tell whether the game had heard them. Brief 04's rule is that failure must
//! be loud, so every refusal in this file writes a sentence to
//! [`LinesFeedback`], which the panel draws where the player is already
//! looking.
//!
//! # Focus is a mode, and it says so
//!
//! A focused row is not decoration: while a line is focused, clicking a train
//! in the world puts that train on that line
//! ([`assign_clicked_train_to_focused_line`]). That was designed and left
//! unwired, so the interaction existed only as a doc comment. The focused row
//! now states the mode in words, and `Esc` leaves it.

use bevy::prelude::*;
use rail_sim::{
    line_colour_rgba, AssignTrainToLine, CommandBuffer, CommandKind, LineId, LineRegistry,
    RemoveLine, StationRegistry, Train, TrainId,
};

use crate::inspect::{Selectable, Selection};
use crate::palette::{BALLAST_L, BALLAST_M, BG1, HI, OUTLINE, RAIL_L};
use crate::ui::kit::{
    body_font, micro_font, text_primary, text_secondary, text_warn, SPACE_2, SPACE_3, STATUS_H,
};
use crate::ui::{ConfirmAccepted, ConfirmAction, ConfirmDialog, ConfirmPrompt};

use super::tools::LineToolState;

/// Longest station name drawn in a stop strip before it is cut short.
///
/// Was `8`, which turned *Westbrook* into *"Westbroo"* — a stop the player
/// could not name back to the game. The panel is 260px of `micro_font`, so a
/// strip still has to end somewhere; it ends late enough that every name the
/// generator produces survives intact, and says so with an ellipsis when it
/// does not.
const MAX_STOP_NAME: usize = 18;

/// Assigned trains named in a row before the rest become a count.
const MAX_NAMED_TRAINS: usize = 4;

/// Seconds a panel message stays up.
const FEEDBACK_TTL_SECS: f32 = 4.0;

#[derive(Component)]
pub struct LinesPanelRoot;

#[derive(Component)]
pub struct LinesPanelBody;

#[derive(Component)]
pub struct LineRowButton {
    pub line: LineId,
}

#[derive(Component)]
pub struct AssignTrainButton {
    pub line: LineId,
}

#[derive(Component)]
pub struct RemoveLineButton {
    pub line: LineId,
}

#[derive(Resource, Debug, Default)]
pub(crate) struct LinesPanelCache {
    fingerprint: String,
}

/// The line the panel is pointed at — and the crewing mode that implies.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct FocusedLine(pub Option<LineId>);

/// The last thing the Lines panel has to say to the player.
///
/// Every refusal in this module goes through here rather than through a silent
/// `continue`. It is a panel-local channel on purpose: the build cursor's chip
/// belongs to the track tools and follows the pointer, whereas these messages
/// answer a click on this panel, and this panel is where the player is looking.
#[derive(Resource, Debug, Default)]
pub struct LinesFeedback {
    message: Option<String>,
    warn: bool,
    ttl: f32,
}

impl LinesFeedback {
    /// Report something that happened.
    pub fn say(&mut self, message: impl Into<String>) {
        self.set(message.into(), false);
    }

    /// Report something that did **not** happen, and why.
    pub fn refuse(&mut self, message: impl Into<String>) {
        self.set(message.into(), true);
    }

    fn set(&mut self, message: String, warn: bool) {
        self.message = Some(message);
        self.warn = warn;
        self.ttl = FEEDBACK_TTL_SECS;
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn is_warning(&self) -> bool {
        self.warn
    }

    pub fn clear(&mut self) {
        self.message = None;
        self.warn = false;
        self.ttl = 0.0;
    }
}

/// Age out the panel message so a stale answer never reads as a fresh one.
pub fn tick_lines_feedback(time: Res<Time>, mut feedback: ResMut<LinesFeedback>) {
    if feedback.message.is_none() {
        return;
    }
    feedback.ttl -= time.delta_secs();
    if feedback.ttl <= 0.0 {
        feedback.clear();
    }
}

pub fn setup_lines_panel(mut commands: Commands) {
    commands.insert_resource(LinesPanelCache::default());
    commands.insert_resource(FocusedLine::default());
    commands.insert_resource(LinesFeedback::default());
    commands
        .spawn((
            LinesPanelRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(STATUS_H + SPACE_2),
                left: Val::Px(SPACE_3),
                width: Val::Px(260.0),
                max_height: Val::Px(320.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                padding: UiRect::all(Val::Px(SPACE_2)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::ZERO,
                ..default()
            },
            BackgroundColor(BG1),
            BorderColor::all(OUTLINE),
            ZIndex(11),
        ))
        .with_children(|root| {
            root.spawn((Text::new("Lines"), body_font(), text_primary()));
            root.spawn((
                Text::new("L - click stations - Enter"),
                micro_font(),
                text_secondary(),
            ));
            root.spawn((
                LinesPanelBody,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    width: Val::Percent(100.0),
                    ..default()
                },
            ));
        });
}

#[allow(clippy::too_many_arguments)]
pub fn update_lines_panel(
    lines: Res<LineRegistry>,
    stations: Res<StationRegistry>,
    line_tool: Res<LineToolState>,
    focused: Res<FocusedLine>,
    feedback: Res<LinesFeedback>,
    selection: Res<Selection>,
    mut cache: ResMut<LinesPanelCache>,
    mut commands: Commands,
    body_q: Query<Entity, With<LinesPanelBody>>,
    children_q: Query<&Children, With<LinesPanelBody>>,
) {
    let selected_train = match selection.0 {
        Some(Selectable::Train(id)) => Some(id),
        _ => None,
    };
    let mut fp = format!(
        "d:{}:w:{:?}:f:{:?}:m:{:?}:s:{:?}:",
        line_tool.draft_stops.len(),
        line_tool.warn,
        focused.0,
        feedback.message(),
        selected_train,
    );
    for line in lines.iter() {
        fp.push_str(&format!(
            "{}:{}:{}:{:?};",
            line.id.0, line.name, line.stops.len(), line.trains
        ));
    }
    if fp == cache.fingerprint {
        return;
    }
    cache.fingerprint = fp;

    let Ok(body) = body_q.single() else {
        return;
    };
    if let Ok(children) = children_q.get(body) {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    commands.entity(body).with_children(|body| {
        // The panel's own answer to the last thing it was asked, first, so a
        // refusal is the thing the eye lands on.
        if let Some(message) = feedback.message() {
            body.spawn((
                Text::new(message.to_string()),
                micro_font(),
                if feedback.is_warning() {
                    text_warn()
                } else {
                    text_primary()
                },
            ));
        }

        // Draft strip
        if line_tool.active || !line_tool.draft_stops.is_empty() {
            body.spawn((
                Text::new(if line_tool.active {
                    "Draft (Enter to confirm)"
                } else {
                    "Draft"
                }),
                micro_font(),
                text_secondary(),
            ));
            let strip = draft_strip(&stations, &line_tool.draft_stops);
            body.spawn((Text::new(strip), body_font(), text_primary()));
            if let Some(warn) = &line_tool.warn {
                body.spawn((Text::new(warn.clone()), micro_font(), text_warn()));
            }
        }

        let mut sorted: Vec<_> = lines.iter().collect();
        sorted.sort_by_key(|l| l.id.0);

        if sorted.is_empty() && !line_tool.active {
            body.spawn((Text::new("No lines yet"), micro_font(), text_secondary()));
            return;
        }

        for line in sorted {
            let selected = focused.0 == Some(line.id);
            let rgb = line_colour_rgba(line.colour);
            let colour = Color::srgb(
                rgb[0] as f32 / 255.0,
                rgb[1] as f32 / 255.0,
                rgb[2] as f32 / 255.0,
            );
            let strip = stop_strip(&stations, &line.stops);
            let crew = crew_line(&line.trains);
            body.spawn((
                Button,
                LineRowButton { line: line.id },
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    padding: UiRect::all(Val::Px(4.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::ZERO,
                    width: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(BG1),
                BorderColor::all(if selected { HI } else { BALLAST_M }),
            ))
            .with_children(|row| {
                row.spawn((
                    Text::new(format!("o {}", line.name)),
                    body_font(),
                    TextColor(colour),
                ));
                row.spawn((Text::new(strip), micro_font(), TextColor(RAIL_L)));
                row.spawn((Text::new(crew), micro_font(), text_secondary()));
                // The focused row states the mode it puts the world into. This
                // is the affordance the click-to-assign interaction was missing
                // for as long as it sat behind `#[allow(dead_code)]`.
                if selected {
                    row.spawn((
                        Text::new("Click a train in the world to assign it"),
                        micro_font(),
                        text_primary(),
                    ));
                }
                row.spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(4.0),
                        ..default()
                    },
                    BackgroundColor(BG1),
                ))
                .with_children(|controls| {
                    // The button says what it will do with what is actually
                    // selected, so its precondition is readable before the
                    // click rather than only after it.
                    let label = match selected_train {
                        Some(id) => format!("Assign Train {}", id.0),
                        None => "Assign train (none picked)".to_string(),
                    };
                    spawn_row_button(controls, AssignTrainButton { line: line.id }, &label);
                    spawn_row_button(controls, RemoveLineButton { line: line.id }, "Remove");
                });
            });
        }
    });
}

fn spawn_row_button<C: Component>(parent: &mut ChildSpawnerCommands, marker: C, label: &str) {
    parent
        .spawn((
            Button,
            marker,
            Node {
                padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::ZERO,
                ..default()
            },
            BackgroundColor(BG1),
            BorderColor::all(BALLAST_L),
        ))
        .with_children(|b| {
            b.spawn((Text::new(label.to_string()), micro_font(), text_secondary()));
        });
}

pub fn line_row_clicks(
    interactions: Query<(&Interaction, &LineRowButton), (Changed<Interaction>, With<Button>)>,
    lines: Res<LineRegistry>,
    mut focused: ResMut<FocusedLine>,
    mut feedback: ResMut<LinesFeedback>,
) {
    for (interaction, btn) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // Clicking the focused row again puts the crewing mode down, so the
        // mode has a way out that is not a keyboard shortcut.
        if focused.0 == Some(btn.line) {
            focused.0 = None;
            feedback.clear();
            continue;
        }
        focused.0 = Some(btn.line);
        let name = line_name(&lines, btn.line);
        feedback.say(format!("{name} focused - click a train to assign it"));
    }
}

/// The panel's own assign button.
///
/// Its failure mode is the one the playtest found: pressed with no train
/// selected it did nothing at all. Now every path out of it says something.
pub fn assign_train_clicks(
    interactions: Query<(&Interaction, &AssignTrainButton), (Changed<Interaction>, With<Button>)>,
    selection: Res<Selection>,
    lines: Res<LineRegistry>,
    trains: Query<&Train>,
    mut buffer: ResMut<CommandBuffer>,
    mut focused: ResMut<FocusedLine>,
    mut feedback: ResMut<LinesFeedback>,
) {
    for (interaction, btn) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // Pressing a row's button is also pointing the panel at that row.
        focused.0 = Some(btn.line);
        let Some(Selectable::Train(id)) = selection.0 else {
            feedback.refuse("No train picked - press V for Look, then click a train");
            continue;
        };
        if !trains.iter().any(|t| t.id == id) {
            feedback.refuse(format!("Train {} is not on the map yet", id.0));
            continue;
        }
        assign(&mut buffer, &mut feedback, &lines, id, btn.line);
    }
}

/// **The interaction that was designed and never wired.**
///
/// With a line focused in the panel, a world click that picks a train assigns
/// it to that line. The selection gate in [`crate::inspect::selection`] is what
/// makes this safe to hang off `Selection`: only the Look tool selects, so an
/// armed build verb still owns its own world clicks and this can never steal
/// one from it.
///
/// It fires on a **fresh pick**, never on steady state. Focusing a row while
/// some train happens to be selected must not quietly crew the line — the
/// player asked to look at the line, not to staff it. The row's own button is
/// the verb for that case.
pub fn assign_clicked_train_to_focused_line(
    selection: Res<Selection>,
    focused: Res<FocusedLine>,
    lines: Res<LineRegistry>,
    mut buffer: ResMut<CommandBuffer>,
    mut feedback: ResMut<LinesFeedback>,
    mut last: Local<Option<(LineId, TrainId)>>,
) {
    let Some(line) = focused.0 else {
        *last = None;
        return;
    };
    let Some(Selectable::Train(train)) = selection.0 else {
        *last = None;
        return;
    };
    if !selection.is_changed() {
        return;
    }
    // One assignment per pick. The registry answers this too, but only from the
    // next tick — the command has not been applied yet on the frame of the
    // click, and the same pick must not queue a second one.
    if *last == Some((line, train)) {
        return;
    }
    *last = Some((line, train));
    assign(&mut buffer, &mut feedback, &lines, train, line);
}

/// Buffer an assignment, or say why there was nothing to do.
///
/// The sim announces the assignment in Town Talk; this is the panel echoing it
/// where the player's eye already is.
fn assign(
    buffer: &mut CommandBuffer,
    feedback: &mut LinesFeedback,
    lines: &LineRegistry,
    train: TrainId,
    line: LineId,
) {
    let name = line_name(lines, line);
    if lines.get(line).is_none() {
        feedback.refuse("That line is gone");
        return;
    }
    if lines
        .get(line)
        .is_some_and(|l| l.trains.contains(&train))
    {
        feedback.say(format!("Train {} is already on {name}", train.0));
        return;
    }
    buffer.push(CommandKind::AssignTrainToLine(AssignTrainToLine {
        train,
        line,
    }));
    feedback.say(format!("Train {} assigned to {name}", train.0));
}

/// The per-row `Remove` affordance, and the only way to delete a line.
///
/// # Why this is a button and not the `X` key
///
/// `X` is the Demolish verb, and the obvious design — *`X` with a line focused
/// removes it* — reads well and plays badly. Focus here is sticky: confirming a
/// new line focuses it, and it stays focused while the player goes back to
/// building. `X` would then stop arming the demolish tool and start asking about
/// a line the player had forgotten was selected, in the middle of laying track.
/// The train sale gets away with sharing `X`
/// ([`crate::trains::sell_selected_train_input`]) because a selected train is a
/// deliberate, momentary state with the Inspector open on it; a focused line is
/// not.
///
/// So removal is a control on the row it removes, which is also where the player
/// is looking when they decide a line was a mistake. The consequence is still
/// named first, through the one confirm dialog, as 04 §4 requires.
pub fn remove_line_clicks(
    interactions: Query<(&Interaction, &RemoveLineButton), (Changed<Interaction>, With<Button>)>,
    lines: Res<LineRegistry>,
    mut focused: ResMut<FocusedLine>,
    mut confirm: ResMut<ConfirmDialog>,
) {
    for (interaction, btn) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        focused.0 = Some(btn.line);
        ask_remove(&mut confirm, &lines, btn.line);
    }
}

fn ask_remove(confirm: &mut ConfirmDialog, lines: &LineRegistry, id: LineId) {
    let Some(line) = lines.get(id) else {
        return;
    };
    // Name the consequence, not the act: what the player cannot see from here is
    // what becomes of the trains.
    let crew = match line.trains.len() {
        0 => "It has no trains.".to_string(),
        1 => "Its train keeps running and takes any job.".to_string(),
        n => format!("Its {n} trains keep running and take any job."),
    };
    confirm.ask(ConfirmPrompt {
        title: "Remove line".into(),
        body: format!("Remove line {}? {crew}", line.name),
        confirm: "Remove".into(),
        action: ConfirmAction::RemoveLine(id),
    });
}

/// Carry out a removal the player agreed to in the dialog.
pub fn apply_confirmed_remove_line(
    mut accepted: MessageReader<ConfirmAccepted>,
    lines: Res<LineRegistry>,
    mut buffer: ResMut<CommandBuffer>,
    mut focused: ResMut<FocusedLine>,
    mut feedback: ResMut<LinesFeedback>,
) {
    for ConfirmAccepted(action) in accepted.read() {
        let ConfirmAction::RemoveLine(id) = action else {
            continue;
        };
        let name = line_name(&lines, *id);
        buffer.push(CommandKind::RemoveLine(RemoveLine { line: *id }));
        // The panel must not sit pointed at a line that is on its way out.
        if focused.0 == Some(*id) {
            focused.0 = None;
        }
        feedback.say(format!("{name} removed"));
    }
}

fn line_name(lines: &LineRegistry, id: LineId) -> String {
    lines
        .get(id)
        .map(|l| l.name.clone())
        .unwrap_or_else(|| format!("Line {}", id.0))
}

/// "Trains 3, 5" — the crew, by name, not a bare count.
///
/// A row that says *"2 train(s)"* tells the player a number they already knew
/// and withholds the one thing they need to check their work: *which* trains.
pub(super) fn crew_line(trains: &[TrainId]) -> String {
    if trains.is_empty() {
        return "No trains yet".into();
    }
    let mut ids: Vec<u64> = trains.iter().map(|t| t.0).collect();
    ids.sort_unstable();
    let named: Vec<String> = ids
        .iter()
        .take(MAX_NAMED_TRAINS)
        .map(|id| id.to_string())
        .collect();
    let rest = ids.len().saturating_sub(named.len());
    let head = if named.len() == 1 { "Train" } else { "Trains" };
    if rest == 0 {
        format!("{head} {}", named.join(", "))
    } else {
        format!("{head} {} +{rest} more", named.join(", "))
    }
}

/// One station name, whole unless it is genuinely too long to draw.
pub(super) fn stop_name(stations: &StationRegistry, id: rail_sim::StationId) -> String {
    let name = stations.get(id).map(|s| s.name.as_str()).unwrap_or("?");
    if name.chars().count() <= MAX_STOP_NAME {
        return name.to_string();
    }
    let kept: String = name.chars().take(MAX_STOP_NAME - 3).collect();
    format!("{kept}...")
}

pub(super) fn stop_strip(stations: &StationRegistry, stops: &[rail_sim::StationId]) -> String {
    if stops.is_empty() {
        return "-".into();
    }
    stops
        .iter()
        .map(|id| stop_name(stations, *id))
        .collect::<Vec<_>>()
        .join(" - ")
}

fn draft_strip(stations: &StationRegistry, stops: &[rail_sim::StationId]) -> String {
    if stops.is_empty() {
        return "(click stations)".into();
    }
    stop_strip(stations, stops)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rail_sim::ids::TileCoord;
    use rail_sim::GROUND_LAYER;

    fn stations() -> StationRegistry {
        let mut stations = StationRegistry::new();
        stations.insert("Westbrook", TileCoord { x: 1, y: 1 }, GROUND_LAYER);
        stations.insert("Eastgate", TileCoord { x: 5, y: 1 }, GROUND_LAYER);
        stations
    }

    /// **The playtest bug.** `take(8)` rendered *Westbrook* as *"Westbroo"* —
    /// a stop the player could not name back to the game.
    #[test]
    fn a_stop_strip_names_its_stops_in_full() {
        let stations = stations();
        let strip = stop_strip(&stations, &[rail_sim::StationId(1), rail_sim::StationId(2)]);
        assert_eq!(strip, "Westbrook - Eastgate");
    }

    #[test]
    fn a_name_too_long_to_draw_is_cut_with_an_ellipsis_not_in_silence() {
        let mut stations = StationRegistry::new();
        let id = stations.insert(
            "Kirkbride Junction Interchange",
            TileCoord { x: 1, y: 1 },
            GROUND_LAYER,
        );
        let name = stop_name(&stations, id);
        assert_eq!(name, "Kirkbride Junct...");
        assert!(name.ends_with("..."), "a cut name has to look cut");
        assert!(name.chars().count() <= MAX_STOP_NAME);
        assert!(name.is_ascii(), "the shipped font draws non-ASCII as tofu");
    }

    #[test]
    fn an_empty_strip_says_so() {
        assert_eq!(stop_strip(&stations(), &[]), "-");
        assert_eq!(draft_strip(&stations(), &[]), "(click stations)");
    }

    /// A row that says "2 train(s)" withholds the only fact worth having.
    #[test]
    fn a_row_names_the_trains_working_it() {
        assert_eq!(crew_line(&[]), "No trains yet");
        assert_eq!(crew_line(&[TrainId(3)]), "Train 3");
        assert_eq!(crew_line(&[TrainId(5), TrainId(3)]), "Trains 3, 5");
        assert_eq!(
            crew_line(&[TrainId(6), TrainId(1), TrainId(4), TrainId(2), TrainId(9)]),
            "Trains 1, 2, 4, 6 +1 more",
            "a long crew still fits the row"
        );
    }

    /// A registry holding one two-stop line, and the resources the panel's
    /// interaction systems read.
    fn app() -> (App, LineId) {
        let mut lines = LineRegistry::new();
        let id = lines
            .create("Eastgate - Westbrook".into(), vec![
                rail_sim::StationId(1),
                rail_sim::StationId(2),
            ])
            .expect("line");

        let mut app = App::new();
        app.init_resource::<Selection>()
            .init_resource::<CommandBuffer>()
            .init_resource::<FocusedLine>()
            .init_resource::<LinesFeedback>()
            .init_resource::<ConfirmDialog>()
            .add_message::<ConfirmAccepted>()
            .insert_resource(lines);
        (app, id)
    }

    fn message(app: &App) -> Option<String> {
        app.world()
            .resource::<LinesFeedback>()
            .message()
            .map(str::to_string)
    }

    fn pending(app: &App) -> Vec<CommandKind> {
        app.world()
            .resource::<CommandBuffer>()
            .pending()
            .iter()
            .map(|c| c.kind.clone())
            .collect()
    }

    /// **The silent button.** Pressed with nothing selected it did literally
    /// nothing — in exactly the state the broken Line flow left the player in.
    #[test]
    fn the_assign_button_says_why_it_did_nothing() {
        let (mut app, id) = app();
        app.add_systems(Update, assign_train_clicks);
        app.world_mut()
            .spawn((Button, AssignTrainButton { line: id }, Interaction::Pressed));

        app.update();

        let said = message(&app).expect("a refusal has to be visible");
        assert_eq!(said, "No train picked - press V for Look, then click a train");
        assert!(said.is_ascii());
        assert!(
            app.world().resource::<LinesFeedback>().is_warning(),
            "a refusal reads as one"
        );
        assert!(pending(&app).is_empty(), "and nothing was ordered");
        assert_eq!(
            app.world().resource::<FocusedLine>().0,
            Some(id),
            "pressing a row's button still points the panel at that row"
        );
    }

    #[test]
    fn the_assign_button_assigns_the_selected_train() {
        let (mut app, id) = app();
        app.add_systems(Update, assign_train_clicks);
        app.world_mut().spawn(Train {
            id: TrainId(3),
            kind: rail_sim::TrainKind::Transit,
        });
        app.world_mut()
            .resource_mut::<Selection>()
            .set(Selectable::Train(TrainId(3)));
        app.world_mut()
            .spawn((Button, AssignTrainButton { line: id }, Interaction::Pressed));

        app.update();

        assert!(matches!(
            pending(&app).first(),
            Some(CommandKind::AssignTrainToLine(a)) if a.train == TrainId(3) && a.line == id
        ));
        assert_eq!(
            message(&app).as_deref(),
            Some("Train 3 assigned to Eastgate - Westbrook")
        );
    }

    /// A train the player picked out of a save but that is not on the map is
    /// still a refusal, and still says so.
    #[test]
    fn assigning_a_train_that_is_not_on_the_map_says_so() {
        let (mut app, id) = app();
        app.add_systems(Update, assign_train_clicks);
        app.world_mut()
            .resource_mut::<Selection>()
            .set(Selectable::Train(TrainId(7)));
        app.world_mut()
            .spawn((Button, AssignTrainButton { line: id }, Interaction::Pressed));

        app.update();

        assert_eq!(
            message(&app).as_deref(),
            Some("Train 7 is not on the map yet")
        );
        assert!(pending(&app).is_empty());
    }

    /// **The interaction that was dead code.** A line focused, a train clicked,
    /// and the train joins the line.
    #[test]
    fn clicking_a_train_with_a_line_focused_assigns_it() {
        let (mut app, id) = app();
        app.add_systems(Update, assign_clicked_train_to_focused_line);
        app.world_mut().resource_mut::<FocusedLine>().0 = Some(id);
        app.world_mut()
            .resource_mut::<Selection>()
            .set(Selectable::Train(TrainId(3)));

        app.update();

        assert!(matches!(
            pending(&app).first(),
            Some(CommandKind::AssignTrainToLine(a)) if a.train == TrainId(3) && a.line == id
        ));
        assert_eq!(
            message(&app).as_deref(),
            Some("Train 3 assigned to Eastgate - Westbrook"),
            "the panel echoes what the sim is about to announce"
        );
    }

    #[test]
    fn clicking_a_train_with_no_line_focused_assigns_nothing() {
        let (mut app, _) = app();
        app.add_systems(Update, assign_clicked_train_to_focused_line);
        app.world_mut()
            .resource_mut::<Selection>()
            .set(Selectable::Train(TrainId(3)));

        app.update();

        assert!(pending(&app).is_empty(), "focus is what arms this");
        assert_eq!(message(&app), None);
    }

    /// Focusing a row must not quietly crew the line with whatever train
    /// happened to be selected — the player asked to look at it, not staff it.
    #[test]
    fn focusing_a_line_does_not_assign_a_train_that_was_already_selected() {
        let (mut app, id) = app();
        app.add_systems(Update, assign_clicked_train_to_focused_line);
        app.world_mut()
            .resource_mut::<Selection>()
            .set(Selectable::Train(TrainId(3)));
        app.update();
        assert!(pending(&app).is_empty(), "no line focused yet");

        // Focus arrives on a later frame, with the selection untouched.
        app.world_mut().resource_mut::<FocusedLine>().0 = Some(id);
        app.update();

        assert!(
            pending(&app).is_empty(),
            "a focus change is not a pick, and must not act like one"
        );
    }

    /// The same pick must not queue a second assignment while the first is
    /// still on its way to the sim.
    #[test]
    fn one_pick_is_one_assignment() {
        let (mut app, id) = app();
        app.add_systems(Update, assign_clicked_train_to_focused_line);
        app.world_mut().resource_mut::<FocusedLine>().0 = Some(id);
        app.world_mut()
            .resource_mut::<Selection>()
            .set(Selectable::Train(TrainId(3)));

        app.update();
        app.update();
        app.update();

        assert_eq!(pending(&app).len(), 1);
    }

    #[test]
    fn a_train_already_on_the_line_is_told_so_rather_than_reassigned() {
        let (mut app, id) = app();
        app.world_mut()
            .resource_mut::<LineRegistry>()
            .assign_train(id, TrainId(3));
        app.add_systems(Update, assign_clicked_train_to_focused_line);
        app.world_mut().resource_mut::<FocusedLine>().0 = Some(id);
        app.world_mut()
            .resource_mut::<Selection>()
            .set(Selectable::Train(TrainId(3)));

        app.update();

        assert!(pending(&app).is_empty());
        assert_eq!(
            message(&app).as_deref(),
            Some("Train 3 is already on Eastgate - Westbrook")
        );
    }

    /// 04 §4: a removal with a consequence names it. What the player cannot see
    /// from this panel is what becomes of the trains.
    #[test]
    fn removing_a_line_asks_first_and_names_what_happens_to_its_trains() {
        let (mut app, id) = app();
        {
            let mut lines = app.world_mut().resource_mut::<LineRegistry>();
            lines.assign_train(id, TrainId(3));
            lines.assign_train(id, TrainId(5));
        }
        app.add_systems(Update, remove_line_clicks);
        app.world_mut()
            .spawn((Button, RemoveLineButton { line: id }, Interaction::Pressed));

        app.update();

        let dialog = app.world().resource::<ConfirmDialog>();
        let prompt = dialog.prompt().expect("the dialog asks first");
        assert_eq!(
            prompt.body,
            "Remove line Eastgate - Westbrook? Its 2 trains keep running and take any job."
        );
        assert_eq!(prompt.confirm, "Remove", "the button is the verb");
        assert_eq!(prompt.action, ConfirmAction::RemoveLine(id));
        assert!(prompt.body.is_ascii() && prompt.title.is_ascii());
        assert!(
            pending(&app).is_empty(),
            "asking is not doing — the dialog performs nothing"
        );
    }

    #[test]
    fn a_line_with_no_trains_says_that_instead() {
        let (mut app, id) = app();
        app.add_systems(Update, remove_line_clicks);
        app.world_mut()
            .spawn((Button, RemoveLineButton { line: id }, Interaction::Pressed));

        app.update();

        assert_eq!(
            app.world()
                .resource::<ConfirmDialog>()
                .prompt()
                .map(|p| p.body.clone()),
            Some("Remove line Eastgate - Westbrook? It has no trains.".into())
        );
    }

    #[test]
    fn saying_yes_buffers_the_removal_and_lets_the_focus_go() {
        let (mut app, id) = app();
        app.add_systems(Update, apply_confirmed_remove_line);
        app.world_mut().resource_mut::<FocusedLine>().0 = Some(id);

        app.update();
        assert!(pending(&app).is_empty(), "nothing agreed to yet");

        app.world_mut()
            .write_message(ConfirmAccepted(ConfirmAction::RemoveLine(id)));
        app.update();

        assert!(matches!(
            pending(&app).first(),
            Some(CommandKind::RemoveLine(r)) if r.line == id
        ));
        assert_eq!(
            app.world().resource::<FocusedLine>().0,
            None,
            "the panel does not stay pointed at a line that is leaving"
        );
        assert_eq!(
            message(&app).as_deref(),
            Some("Eastgate - Westbrook removed")
        );
    }

    /// The other confirmable actions are not ours.
    #[test]
    fn a_train_sale_agreement_is_not_a_line_removal() {
        let (mut app, _) = app();
        app.add_systems(Update, apply_confirmed_remove_line);

        app.world_mut()
            .write_message(ConfirmAccepted(ConfirmAction::SellTrain(TrainId(3))));
        app.update();

        assert!(pending(&app).is_empty());
    }

    #[test]
    fn clicking_the_focused_row_again_puts_the_crewing_mode_down() {
        let (mut app, id) = app();
        app.add_systems(Update, line_row_clicks);
        let row = app
            .world_mut()
            .spawn((Button, LineRowButton { line: id }, Interaction::Pressed))
            .id();

        app.update();
        assert_eq!(app.world().resource::<FocusedLine>().0, Some(id));
        assert_eq!(
            message(&app).as_deref(),
            Some("Eastgate - Westbrook focused - click a train to assign it"),
            "focus is a mode, so it announces itself"
        );

        // A second press on the same row is the way back out. The system reads
        // `Changed<Interaction>`, so the press has to be a fresh one.
        *app.world_mut().get_mut::<Interaction>(row).expect("the row") = Interaction::Pressed;
        app.update();
        assert_eq!(app.world().resource::<FocusedLine>().0, None);
        assert_eq!(message(&app), None);
    }

    #[test]
    fn every_message_this_panel_can_show_is_ascii() {
        let mut feedback = LinesFeedback::default();
        feedback.refuse("No train picked - press V for Look, then click a train");
        let message = feedback.message().expect("a refusal").to_string();
        assert!(message.is_ascii(), "{message} would draw as tofu");
        assert!(feedback.is_warning());
        feedback.say("Train 3 assigned to Eastgate - Westbrook");
        assert!(!feedback.is_warning(), "news is not a warning");
    }
}
