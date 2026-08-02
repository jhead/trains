//! Apply border world-commands after [`crate::apply::apply_commands`].
//!
//! Opening a border is a construction project, so it flows through exactly the
//! path `PlaceStation` and `PlaceTrack` do — buffered as a [`CommandKind`],
//! drained on the fixed tick, validated with a specific reason, charged, and
//! recorded with an inverse for undo. There is no parallel mechanism.
//!
//! # Ordering
//! Register **after** [`crate::apply_commands`] and **before**
//! [`crate::track::apply_track_commands`], for the same reason
//! [`crate::stations::apply`] does: the track handler owns
//! [`CommandHistory::finish_replay`], and running after it would make a border
//! inverse replayed during an undo look like a fresh player action and wipe the
//! redo stack.
//!
//! # Wiring seam
//! [`BorderCommand::from_kind`] and [`BorderCommand::into_kind`] are the only
//! two places that name the border variants of [`CommandKind`], mirroring
//! [`StationCommand`](crate::stations::StationCommand). Everything else —
//! validation, money, history, presentation — is live and tested without them.

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::apply::PendingWorldCommand;
use crate::command_buffer::CommandBuffer;
use crate::commands::CommandKind;
use crate::economy::MoneyLedger;
use crate::history::{CommandHistory, HistoryMode};
use crate::ids::{TileCoord, TrainId};
use crate::money::Money;
use crate::peeps::ComplaintFeed;
use crate::stations::GoodKind;
use crate::track::{TrackNetwork, TrackTerrain};
use crate::trains::Train;

use super::edge::BorderEdge;
use super::link::{try_close_border, try_open_border, BorderError, BorderRegistry};
use super::trade::{say, BorderRun};

/// Open a border portal where a line reaches the map edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenBorder {
    pub tile: TileCoord,
    /// Reserved for tunnels / elevated; ground-only in MVP.
    pub layer: u8,
    pub edge: BorderEdge,
}

/// Sever a link, refunding the portal in full.
///
/// Never destructive: the track stays, the goods stay, and the relationship is
/// archived so re-opening the edge resumes it (§7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseBorder {
    pub edge: BorderEdge,
}

/// Set the standing offer and request published on a link.
///
/// The Trade agreement panel (§9): deliberately simple — what you'll send, what
/// you want.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetBorderTrade {
    pub edge: BorderEdge,
    pub offer: GoodKind,
    pub request: GoodKind,
}

/// Put a train on a border run, or take it off one.
///
/// `edge: None` recalls the train to ordinary work. Not a construction action,
/// so it is not undoable — the same treatment
/// [`AssignTrainToLine`](crate::commands::AssignTrainToLine) gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignTrainToBorder {
    pub train: TrainId,
    pub edge: Option<BorderEdge>,
}

/// Fired when a border changes so presentation, audio and UI can react.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum BorderEdit {
    Opened {
        edge: BorderEdge,
        tile: TileCoord,
        town_name: String,
        is_echo: bool,
    },
    Closed {
        edge: BorderEdge,
        tile: TileCoord,
        town_name: String,
    },
    /// A train left the map through the portal.
    Departed {
        edge: BorderEdge,
        train: TrainId,
        good: Option<GoodKind>,
        units: u32,
    },
    /// A train came back through the portal carrying their goods.
    Arrived {
        edge: BorderEdge,
        train: TrainId,
        good: Option<GoodKind>,
        units: u32,
        paid_cents: i64,
    },
    /// The cached neighbour published something new.
    NeighbourUpdated {
        edge: BorderEdge,
        town_name: String,
        sequence: u64,
    },
    /// Open / close / trade / dispatch rejected, with its reason.
    Failed {
        error: BorderError,
        edge: BorderEdge,
    },
}

/// The border slice of [`CommandKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderCommand {
    Open(OpenBorder),
    Close(CloseBorder),
    SetTrade(SetBorderTrade),
    Assign(AssignTrainToBorder),
}

impl BorderCommand {
    /// Edge the intent is aimed at, for the reason chip / reject flash.
    pub fn edge(&self) -> Option<BorderEdge> {
        match self {
            Self::Open(o) => Some(o.edge),
            Self::Close(c) => Some(c.edge),
            Self::SetTrade(t) => Some(t.edge),
            Self::Assign(a) => a.edge,
        }
    }

    /// **WIRING SEAM** — recognise the border variants of [`CommandKind`].
    ///
    /// Replace the body with:
    /// ```ignore
    /// match kind {
    ///     CommandKind::OpenBorder(o) => Some(Self::Open(*o)),
    ///     CommandKind::CloseBorder(c) => Some(Self::Close(*c)),
    ///     CommandKind::SetBorderTrade(t) => Some(Self::SetTrade(*t)),
    ///     CommandKind::AssignTrainToBorder(a) => Some(Self::Assign(*a)),
    ///     _ => None,
    /// }
    /// ```
    pub fn from_kind(kind: &CommandKind) -> Option<Self> {
        match kind {
            CommandKind::OpenBorder(o) => Some(Self::Open(*o)),
            CommandKind::CloseBorder(c) => Some(Self::Close(*c)),
            CommandKind::SetBorderTrade(t) => Some(Self::SetTrade(*t)),
            CommandKind::AssignTrainToBorder(a) => Some(Self::Assign(*a)),
            _ => None,
        }
    }

    /// **WIRING SEAM** — wrap the intent back into a [`CommandKind`].
    ///
    /// Replace the body with:
    /// ```ignore
    /// Some(match self {
    ///     Self::Open(o) => CommandKind::OpenBorder(o),
    ///     Self::Close(c) => CommandKind::CloseBorder(c),
    ///     Self::SetTrade(t) => CommandKind::SetBorderTrade(t),
    ///     Self::Assign(a) => CommandKind::AssignTrainToBorder(a),
    /// })
    /// ```
    pub fn into_kind(self) -> Option<CommandKind> {
        Some(match self {
            Self::Open(o) => CommandKind::OpenBorder(o),
            Self::Close(c) => CommandKind::CloseBorder(c),
            Self::SetTrade(t) => CommandKind::SetBorderTrade(t),
            Self::Assign(a) => CommandKind::AssignTrainToBorder(a),
        })
    }
}

/// Buffer a border intent the same way the track tools buffer a `PlaceTrack`.
///
/// Presentation calls this instead of building a [`CommandKind`] itself, so the
/// wiring seam stays in one crate.
pub fn push_border_command(buffer: &mut CommandBuffer, command: BorderCommand) {
    if let Some(kind) = command.into_kind() {
        buffer.push(kind);
    }
}

/// Drain [`PendingWorldCommand`] border kinds into the [`BorderRegistry`].
#[allow(clippy::too_many_arguments)]
pub fn apply_border_commands(
    mut pending: MessageReader<PendingWorldCommand>,
    mut registry: ResMut<BorderRegistry>,
    mut money: ResMut<Money>,
    mut ledger: ResMut<MoneyLedger>,
    mut history: ResMut<CommandHistory>,
    mut feed: ResMut<ComplaintFeed>,
    network: Res<TrackNetwork>,
    terrain: Option<Res<TrackTerrain>>,
    trains: Query<(Entity, &Train)>,
    mut commands: Commands,
    mut edits: MessageWriter<BorderEdit>,
) {
    let queued: Vec<BorderCommand> = pending
        .read()
        .filter_map(|msg| BorderCommand::from_kind(&msg.command.kind))
        .collect();
    if queued.is_empty() {
        return;
    }

    let replaying = matches!(
        history.mode(),
        HistoryMode::Undoing | HistoryMode::Redoing
    );

    for command in queued {
        match command {
            BorderCommand::Open(open) => {
                let Some(terrain) = terrain.as_deref() else {
                    // No terrain yet (a world that has not generated): refuse
                    // rather than half-open something.
                    edits.write(BorderEdit::Failed {
                        error: BorderError::NotOnEdge,
                        edge: open.edge,
                    });
                    continue;
                };
                match try_open_border(
                    &mut registry,
                    &mut money,
                    &mut ledger,
                    &network,
                    terrain,
                    open.tile,
                    open.layer,
                    open.edge,
                ) {
                    Ok(opened) => {
                        say(
                            &mut feed,
                            registry.tick,
                            format!(
                                "The {} border is open - {} is over there",
                                opened.edge.label(),
                                opened.town_name
                            ),
                        );
                        edits.write(BorderEdit::Opened {
                            edge: opened.edge,
                            tile: opened.portal_tile,
                            town_name: opened.town_name,
                            is_echo: opened.is_echo,
                        });
                        record(
                            &mut history,
                            replaying,
                            BorderCommand::Close(CloseBorder { edge: open.edge }),
                        );
                    }
                    Err(error) => {
                        edits.write(BorderEdit::Failed {
                            error,
                            edge: open.edge,
                        });
                    }
                }
            }
            BorderCommand::Close(close) => {
                // Stock still beyond the border keeps its schedule: the severed
                // link is archived with its transit list and lands its trains
                // exactly as before (see `trade::advance_border_trade`).
                match try_close_border(&mut registry, &mut money, &mut ledger, close.edge) {
                    Ok(link) => {
                        say(
                            &mut feed,
                            registry.tick,
                            format!("The {} border is closed", link.edge.label()),
                        );
                        edits.write(BorderEdit::Closed {
                            edge: link.edge,
                            tile: link.portal_tile,
                            town_name: link.town_name().to_string(),
                        });
                        record(
                            &mut history,
                            replaying,
                            BorderCommand::Open(OpenBorder {
                                tile: link.portal_tile,
                                layer: link.layer,
                                edge: link.edge,
                            }),
                        );
                    }
                    Err(error) => {
                        edits.write(BorderEdit::Failed {
                            error,
                            edge: close.edge,
                        });
                    }
                }
            }
            BorderCommand::SetTrade(trade) => {
                let Some(link) = registry.get_mut(trade.edge) else {
                    edits.write(BorderEdit::Failed {
                        error: BorderError::EdgeClosed { edge: trade.edge },
                        edge: trade.edge,
                    });
                    continue;
                };
                link.set_trade(trade.offer, trade.request);
                let (edge, name, sequence) =
                    (link.edge, link.town_name().to_string(), link.outbound.sequence);
                edits.write(BorderEdit::NeighbourUpdated {
                    edge,
                    town_name: name,
                    sequence,
                });
            }
            BorderCommand::Assign(assign) => {
                let found = trains.iter().find(|(_, train)| train.id == assign.train);
                let Some((entity, _)) = found else {
                    edits.write(BorderEdit::Failed {
                        error: BorderError::UnknownTrain {
                            train: assign.train,
                        },
                        edge: assign.edge.unwrap_or_default(),
                    });
                    continue;
                };
                match assign.edge {
                    Some(edge) if registry.is_open(edge) => {
                        commands.entity(entity).insert(BorderRun { edge });
                    }
                    Some(edge) => {
                        edits.write(BorderEdit::Failed {
                            error: BorderError::EdgeClosed { edge },
                            edge,
                        });
                    }
                    None => {
                        commands.entity(entity).remove::<BorderRun>();
                    }
                }
            }
        }
    }
}

/// Push an inverse onto the replay batch or record it as a player action.
///
/// [`CommandHistory::finish_replay`] stays the track handler's job — see the
/// module docs for why this system must run before it.
fn record(history: &mut CommandHistory, replaying: bool, inverse: BorderCommand) {
    let Some(kind) = inverse.into_kind() else {
        // Wiring seam still open — nothing to undo yet.
        return;
    };
    if replaying {
        history.push_batch_inverse(kind);
    } else {
        history.record_player_action(vec![kind]);
    }
}
