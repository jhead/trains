//! Apply CreateLine / RemoveLine / AssignTrainToLine from [`PendingWorldCommand`].
//!
//! # The sim is the last word on duplicates
//!
//! [`crate::commands::CreateLine`] arrives from a tool that has already checked
//! the registry and warned the player — but the check that *counts* is here,
//! because this is the one place every source of the command funnels through
//! (tool, panel, a replayed command log, a future network peer). A UI that
//! forgets to look still cannot mint a second copy of a line the player already
//! has. See [`LineRegistry::duplicate_of`] for what "already has" means.
//!
//! Everything the player does to a line says so in Town Talk, in the same
//! whole-sentence shape [`crate::trains::apply`] uses: a line that appears, a
//! train that joins one, and a line that is taken away are all things the
//! player did and must be able to see land.

use bevy_ecs::prelude::*;

use crate::apply::PendingWorldCommand;
use crate::commands::CommandKind;
use crate::ids::TileCoord;
use crate::peeps::{ComplaintEntry, ComplaintFeed, TalkKind};
use crate::stations::{StationRegistry, StationService};
use crate::trains::TrainOnLine;

use super::registry::{suggest_line_name, LineRegistry};

/// Push one whole sentence into Town Talk.
///
/// Mirrors [`crate::trains::apply`]'s helper exactly: the sentence goes in
/// `peep_name` and `station_name` is left empty, which is how
/// [`ComplaintEntry::display_line`] knows the entry carries its own sentence.
/// `station_id` stays `None` so a line's news never counts as *the town speaking
/// about a stop* and silences a district's own ([`ComplaintFeed::town_spoke_recently`]).
fn say(feed: &mut ComplaintFeed, kind: TalkKind, tick: u64, tile: Option<TileCoord>, line: String) {
    feed.push(ComplaintEntry {
        kind,
        peep_name: line,
        station_name: String::new(),
        wait_minutes: 0,
        sim_tick: tick,
        peep_id: None,
        station_id: None,
        tile,
        count: 1,
    });
}

/// Drain line-related pending commands.
#[allow(clippy::too_many_arguments)]
pub fn apply_line_commands(
    mut pending: MessageReader<PendingWorldCommand>,
    mut lines: ResMut<LineRegistry>,
    mut talk: ResMut<ComplaintFeed>,
    stations: Res<StationRegistry>,
    service: Res<StationService>,
    mut trains: Query<(Entity, &crate::trains::Train, Option<&mut TrainOnLine>)>,
    mut commands: Commands,
) {
    let tick = service.tick;
    for msg in pending.read() {
        match &msg.command.kind {
            CommandKind::CreateLine(c) => {
                if c.stops.len() < 2 {
                    continue;
                }
                // Validate stations exist.
                if c.stops.iter().any(|s| stations.get(*s).is_none()) {
                    continue;
                }
                // The player already has this railway under another name.
                if lines.duplicate_of(&c.stops).is_some() {
                    continue;
                }
                let name = c
                    .name
                    .clone()
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| suggest_line_name(&stations, &c.stops));
                let first_stop = c.stops.first().and_then(|s| stations.get(*s)).map(|s| s.tile);
                if lines.create(name.clone(), c.stops.clone()).is_some() {
                    say(
                        &mut talk,
                        TalkKind::Opportunity,
                        tick,
                        first_stop,
                        format!("Line {name} opened - click a train to put it on the route"),
                    );
                }
            }
            // A line is the player's object, so removing one is theirs to
            // command — but its trains are not thrown away with it. They lose
            // the assignment and go back to free-roaming the job board, which is
            // what the confirm dialog promised.
            CommandKind::RemoveLine(r) => {
                let Some(line) = lines.remove(r.line) else {
                    continue;
                };
                let mut released = 0u32;
                for (entity, train, on_line) in trains.iter_mut() {
                    let Some(on) = on_line else {
                        continue;
                    };
                    if on.line != r.line {
                        continue;
                    }
                    let _ = train;
                    commands.entity(entity).remove::<TrainOnLine>();
                    released += 1;
                }
                let crew = match released {
                    0 => String::new(),
                    1 => " - 1 train takes any job now".into(),
                    n => format!(" - {n} trains take any job now"),
                };
                say(
                    &mut talk,
                    TalkKind::Opportunity,
                    tick,
                    None,
                    format!("Line {} removed{crew}", line.name),
                );
            }
            CommandKind::AssignTrainToLine(a) => {
                if !lines.assign_train(a.line, a.train) {
                    continue;
                }
                let mut found = false;
                for (entity, train, on_line) in trains.iter_mut() {
                    if train.id != a.train {
                        continue;
                    }
                    found = true;
                    if let Some(mut on) = on_line {
                        on.line = a.line;
                        on.next_stop = 0;
                        on.forward = true;
                    } else {
                        commands.entity(entity).insert(TrainOnLine {
                            line: a.line,
                            next_stop: 0,
                            forward: true,
                        });
                    }
                    break;
                }
                if !found {
                    // Train not spawned yet — assignment lives on the registry;
                    // component is attached when / if the train exists.
                    let _ = found;
                }
                // Said whether or not the train is on the map yet: the yard is
                // invisible, and an assignment the player cannot see landing is
                // the failure this whole flow was reported for.
                if let Some(line) = lines.get(a.line) {
                    say(
                        &mut talk,
                        TalkKind::Praise,
                        tick,
                        None,
                        format!("Train {} assigned to {}", a.train.0, line.name),
                    );
                }
            }
            CommandKind::UnassignTrain(u) => {
                lines.unassign_train(u.train);
                for (entity, train, on_line) in trains.iter_mut() {
                    if train.id == u.train && on_line.is_some() {
                        commands.entity(entity).remove::<TrainOnLine>();
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::{App, FixedUpdate};

    use crate::command_buffer::CommandBuffer;
    use crate::commands::{AssignTrainToLine, CreateLine, RemoveLine};
    use crate::economy::MoneyLedger;
    use crate::ids::{LineId, StationId, TrainId};
    use crate::money::Money;
    use crate::save::{decode_save, encode_save, SaveMeta, WorldSnapshot};
    use crate::track::{try_place_track, TrackNetwork, TrackTerrain, GROUND_LAYER};
    use crate::trains::{Train, TrainCargo, TrainLocation};
    use crate::{SimClock, SimPlugin, TrainKind};

    /// A paused world with one east-west run and two stops on it.
    ///
    /// Paused because these tests are about the command pass; a moving train
    /// would keep changing the answer.
    fn world() -> (App, StationId, StationId) {
        let mut app = App::new();
        app.add_plugins(SimPlugin);
        app.world_mut().resource_mut::<SimClock>().paused = true;

        let terrain = TrackTerrain::new(32, 32, (0..32 * 32).map(|_| (false, 0i8)));
        let mut network = TrackNetwork::new();
        let mut money = Money::new(10_000_000);
        let mut ledger = MoneyLedger::default();
        for x in 2..=12 {
            try_place_track(
                &mut network,
                &mut money,
                &mut ledger,
                &terrain,
                TileCoord { x, y: 8 },
                GROUND_LAYER,
            )
            .expect("track");
        }
        let mut stations = StationRegistry::new();
        let east = stations.insert("Eastgate", TileCoord { x: 3, y: 8 }, GROUND_LAYER);
        let west = stations.insert("Westbrook", TileCoord { x: 11, y: 8 }, GROUND_LAYER);

        let w = app.world_mut();
        w.insert_resource(network);
        w.insert_resource(stations);
        w.insert_resource(crate::WorldAnchorsSeeded(true));
        (app, east, west)
    }

    fn push(app: &mut App, kind: CommandKind) {
        app.world_mut().resource_mut::<CommandBuffer>().push(kind);
        app.world_mut().run_schedule(FixedUpdate);
    }

    fn create(stops: Vec<StationId>) -> CommandKind {
        CommandKind::CreateLine(CreateLine { name: None, stops })
    }

    fn talk_lines(app: &App) -> Vec<String> {
        app.world()
            .resource::<ComplaintFeed>()
            .iter()
            .map(|e| e.display_line())
            .collect()
    }

    fn line_names(app: &App) -> Vec<String> {
        let lines = app.world().resource::<LineRegistry>();
        let mut names: Vec<_> = lines.iter().map(|l| (l.id.0, l.name.clone())).collect();
        names.sort();
        names.into_iter().map(|(_, name)| name).collect()
    }

    /// Spawn a placed train and put it on `line`.
    fn crew(app: &mut App, id: TrainId, line: LineId) {
        let track = app
            .world()
            .resource::<TrackNetwork>()
            .id_at(TileCoord { x: 3, y: 8 }, GROUND_LAYER)
            .expect("track under Eastgate");
        app.world_mut().spawn((
            Train {
                id,
                kind: TrainKind::Transit,
            },
            TrainLocation::at_track(track),
            TrainCargo::Empty,
        ));
        push(
            app,
            CommandKind::AssignTrainToLine(AssignTrainToLine { train: id, line }),
        );
    }

    fn on_line(app: &mut App, id: TrainId) -> Option<LineId> {
        app.world_mut()
            .query::<(&Train, &TrainOnLine)>()
            .iter(app.world())
            .find(|(t, _)| t.id == id)
            .map(|(_, on)| on.line)
    }

    /// **The playtest bug.** The tool gave no feedback, the player pressed Enter
    /// again, and the town ended up with three names for one piece of railway.
    #[test]
    fn the_same_route_twice_is_one_line() {
        let (mut app, east, west) = world();
        push(&mut app, create(vec![east, west]));
        push(&mut app, create(vec![east, west]));
        push(&mut app, create(vec![east, west]));

        assert_eq!(
            line_names(&app),
            vec!["Eastgate - Westbrook"],
            "three presses of Enter are still one railway"
        );
    }

    /// …and drawing it from the other end is the same railway too: every line
    /// is an out-and-back shuttle, so `A - B` and `B - A` are one service.
    #[test]
    fn the_same_route_drawn_backwards_is_the_same_line() {
        let (mut app, east, west) = world();
        push(&mut app, create(vec![east, west]));
        push(&mut app, create(vec![west, east]));

        assert_eq!(line_names(&app), vec!["Eastgate - Westbrook"]);
    }

    #[test]
    fn a_genuinely_different_route_still_opens() {
        let (mut app, east, west) = world();
        let mid = app
            .world_mut()
            .resource_mut::<StationRegistry>()
            .insert("Millhaven", TileCoord { x: 7, y: 8 }, GROUND_LAYER);

        push(&mut app, create(vec![east, west]));
        push(&mut app, create(vec![east, mid]));
        push(&mut app, create(vec![east, mid, west]));

        assert_eq!(
            line_names(&app),
            vec![
                "Eastgate - Westbrook",
                "Eastgate - Millhaven",
                "Eastgate - Westbrook",
            ],
            "a different stop sequence is a different service"
        );
    }

    #[test]
    fn a_new_line_announces_itself_and_says_what_to_do_next() {
        let (mut app, east, west) = world();
        push(&mut app, create(vec![east, west]));

        let talk = talk_lines(&app);
        assert!(
            talk.iter()
                .any(|l| l == "Line Eastgate - Westbrook opened - click a train to put it on the route"),
            "a created line has to land somewhere the player can see: {talk:?}"
        );
        assert!(talk.iter().all(|l| l.is_ascii()), "{talk:?}");
    }

    /// A duplicate is refused, and refusing it says nothing new — the tool has
    /// already warned. What must not happen is a second "opened" line.
    #[test]
    fn a_refused_duplicate_does_not_announce_a_line_that_was_not_opened() {
        let (mut app, east, west) = world();
        push(&mut app, create(vec![east, west]));
        push(&mut app, create(vec![west, east]));

        let opened = talk_lines(&app)
            .into_iter()
            .filter(|l| l.contains("opened"))
            .count();
        assert_eq!(opened, 1, "one railway, one announcement");
    }

    #[test]
    fn assigning_a_train_says_which_line_it_went_on() {
        let (mut app, east, west) = world();
        push(&mut app, create(vec![east, west]));
        crew(&mut app, TrainId(3), LineId(1));

        let talk = talk_lines(&app);
        assert_eq!(
            talk.first().map(String::as_str),
            Some("Train 3 assigned to Eastgate - Westbrook"),
            "the newest line is the assignment: {talk:?}"
        );
        assert_eq!(on_line(&mut app, TrainId(3)), Some(LineId(1)));
    }

    /// Removal is a first-class verb: the line goes, the trains stay.
    #[test]
    fn removing_a_line_frees_its_trains_rather_than_stranding_them() {
        let (mut app, east, west) = world();
        push(&mut app, create(vec![east, west]));
        crew(&mut app, TrainId(3), LineId(1));
        crew(&mut app, TrainId(4), LineId(1));

        push(&mut app, CommandKind::RemoveLine(RemoveLine { line: LineId(1) }));

        assert!(
            app.world().resource::<LineRegistry>().is_empty(),
            "the line is gone from the registry"
        );
        assert_eq!(on_line(&mut app, TrainId(3)), None, "no stale TrainOnLine");
        assert_eq!(on_line(&mut app, TrainId(4)), None);
        assert_eq!(
            app.world_mut().query::<&Train>().iter(app.world()).count(),
            2,
            "the trains are the player's stock, not the line's"
        );

        let talk = talk_lines(&app);
        assert_eq!(
            talk.first().map(String::as_str),
            Some("Line Eastgate - Westbrook removed - 2 trains take any job now"),
            "the removal names what became of the trains: {talk:?}"
        );
    }

    #[test]
    fn removing_a_line_with_no_trains_says_so_plainly() {
        let (mut app, east, west) = world();
        push(&mut app, create(vec![east, west]));
        push(&mut app, CommandKind::RemoveLine(RemoveLine { line: LineId(1) }));

        let talk = talk_lines(&app);
        assert_eq!(
            talk.first().map(String::as_str),
            Some("Line Eastgate - Westbrook removed")
        );
    }

    #[test]
    fn removing_a_line_that_is_already_gone_does_nothing() {
        let (mut app, east, west) = world();
        push(&mut app, create(vec![east, west]));
        push(&mut app, CommandKind::RemoveLine(RemoveLine { line: LineId(1) }));
        let before = talk_lines(&app).len();

        push(&mut app, CommandKind::RemoveLine(RemoveLine { line: LineId(9) }));
        assert_eq!(talk_lines(&app).len(), before, "no news, no crash");
    }

    /// Removing a line then reusing the route is allowed — the duplicate check
    /// asks about lines that *exist*, not about lines that once did.
    #[test]
    fn a_route_can_be_drawn_again_once_its_line_is_removed() {
        let (mut app, east, west) = world();
        push(&mut app, create(vec![east, west]));
        push(&mut app, CommandKind::RemoveLine(RemoveLine { line: LineId(1) }));
        push(&mut app, create(vec![east, west]));

        assert_eq!(line_names(&app), vec!["Eastgate - Westbrook"]);
    }

    /// A world that has had a line removed saves and loads clean — the registry
    /// is serialized whole, so a hole in the id space must not trip restore.
    #[test]
    fn a_world_with_a_removed_line_round_trips_through_a_save() {
        let (mut app, east, west) = world();
        push(&mut app, create(vec![east, west]));
        let mid = app
            .world_mut()
            .resource_mut::<StationRegistry>()
            .insert("Millhaven", TileCoord { x: 7, y: 8 }, GROUND_LAYER);
        push(&mut app, create(vec![east, mid]));
        crew(&mut app, TrainId(3), LineId(2));
        // Take the *first* line out, leaving a gap below the surviving id.
        push(&mut app, CommandKind::RemoveLine(RemoveLine { line: LineId(1) }));

        let snapshot = WorldSnapshot::capture(app.world());
        let meta = SaveMeta::from_snapshot(&snapshot, "lines round trip");
        let bytes = encode_save(&meta, &snapshot).expect("encode");
        let (_, decoded) = decode_save(&bytes).expect("decode");
        assert_eq!(decoded.lines, snapshot.lines, "the registry survives the trip");

        let mut fresh = App::new();
        fresh.add_plugins(SimPlugin);
        let report = decoded.restore(fresh.world_mut());
        assert!(report.is_clean(), "restore warnings: {:?}", report.warnings);

        let lines = fresh.world().resource::<LineRegistry>();
        assert_eq!(lines.len(), 1);
        assert!(lines.get(LineId(1)).is_none(), "the removed line stays gone");
        let kept = lines.get(LineId(2)).expect("the surviving line");
        assert_eq!(kept.name, "Eastgate - Millhaven");
        assert_eq!(kept.trains, vec![TrainId(3)], "its crew came back with it");
    }

    /// Same commands, same world, twice — the sim may not disagree with itself.
    #[test]
    fn the_line_pass_is_deterministic() {
        let run = || {
            let (mut app, east, west) = world();
            let mid = app
                .world_mut()
                .resource_mut::<StationRegistry>()
                .insert("Millhaven", TileCoord { x: 7, y: 8 }, GROUND_LAYER);
            push(&mut app, create(vec![east, west]));
            push(&mut app, create(vec![west, east]));
            push(&mut app, create(vec![east, mid]));
            push(&mut app, create(vec![mid, east]));
            crew(&mut app, TrainId(3), LineId(2));
            push(&mut app, CommandKind::RemoveLine(RemoveLine { line: LineId(1) }));
            (line_names(&app), talk_lines(&app))
        };
        assert_eq!(run(), run());
    }
}
