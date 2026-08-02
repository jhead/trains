//! Undo / redo stack of inverse construction commands.
//!
//! Construction actions (place / demolish / autofill) push their inverses here
//! after a successful apply. Undo / redo replay those inverses through the
//! normal [`crate::CommandBuffer`] path; simulation time is never rewound.

use bevy_ecs::prelude::Resource;

use crate::commands::CommandKind;

/// How deep the undo stack may grow (design brief: ≥ 50).
pub const HISTORY_DEPTH: usize = 50;

/// While replaying history, successful applies accumulate into the opposite stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistoryMode {
    #[default]
    Record,
    /// Undoing — inverses of the replay go onto the redo stack as one entry.
    Undoing,
    /// Redoing — inverses go onto the undo stack as one entry.
    Redoing,
}

/// One undoable / redoable unit (e.g. a single tile or a whole autofill run).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    /// Commands that reverse the original action, applied in order.
    pub inverse: Vec<CommandKind>,
}

/// Inverse-command stacks for construction undo / redo.
#[derive(Debug, Resource)]
pub struct CommandHistory {
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    mode: HistoryMode,
    /// Inverses accumulated while [`HistoryMode::Undoing`] / [`Redoing`].
    batch: Vec<CommandKind>,
    max_depth: usize,
}

impl Default for CommandHistory {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            mode: HistoryMode::Record,
            batch: Vec::new(),
            max_depth: HISTORY_DEPTH,
        }
    }
}

impl CommandHistory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mode(&self) -> HistoryMode {
        self.mode
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// Begin an undo replay: pop one entry and return its inverse commands.
    pub fn begin_undo(&mut self) -> Option<Vec<CommandKind>> {
        let entry = self.undo.pop()?;
        self.mode = HistoryMode::Undoing;
        self.batch.clear();
        Some(entry.inverse)
    }

    /// Begin a redo replay: pop one entry and return its inverse commands.
    pub fn begin_redo(&mut self) -> Option<Vec<CommandKind>> {
        let entry = self.redo.pop()?;
        self.mode = HistoryMode::Redoing;
        self.batch.clear();
        Some(entry.inverse)
    }

    /// Record the inverse of a successful normal (player) construction action.
    ///
    /// Clears the redo stack. No-op while undoing / redoing (use [`push_batch_inverse`]).
    pub fn record_player_action(&mut self, inverse: Vec<CommandKind>) {
        if inverse.is_empty() {
            return;
        }
        if self.mode != HistoryMode::Record {
            return;
        }
        self.redo.clear();
        self.undo.push(HistoryEntry { inverse });
        self.trim();
    }

    /// Accumulate one inverse while undoing / redoing a multi-command entry.
    pub fn push_batch_inverse(&mut self, kind: CommandKind) {
        if matches!(self.mode, HistoryMode::Undoing | HistoryMode::Redoing) {
            self.batch.push(kind);
        }
    }

    /// Finish a replay frame: flush the batch onto the opposite stack and reset mode.
    pub fn finish_replay(&mut self) {
        let batch = std::mem::take(&mut self.batch);
        match self.mode {
            HistoryMode::Undoing => {
                if !batch.is_empty() {
                    self.redo.push(HistoryEntry { inverse: batch });
                    self.trim_redo();
                }
            }
            HistoryMode::Redoing => {
                if !batch.is_empty() {
                    self.undo.push(HistoryEntry { inverse: batch });
                    self.trim();
                }
            }
            HistoryMode::Record => {}
        }
        self.mode = HistoryMode::Record;
    }

    fn trim(&mut self) {
        while self.undo.len() > self.max_depth {
            self.undo.remove(0);
        }
    }

    fn trim_redo(&mut self) {
        while self.redo.len() > self.max_depth {
            self.redo.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Demolish, PlaceTrack};
    use crate::ids::{TileCoord, TrackId};

    #[test]
    fn undo_redo_roundtrip_stacks() {
        let mut h = CommandHistory::new();
        h.record_player_action(vec![CommandKind::Demolish(Demolish {
            track: TrackId(1),
        })]);
        assert_eq!(h.undo_len(), 1);
        assert!(h.can_undo());

        let cmds = h.begin_undo().unwrap();
        assert_eq!(cmds.len(), 1);
        h.push_batch_inverse(CommandKind::PlaceTrack(PlaceTrack {
            tile: TileCoord { x: 0, y: 0 },
            layer: 0,
        }));
        h.finish_replay();
        assert!(h.can_redo());
        assert!(!h.can_undo());

        let redo = h.begin_redo().unwrap();
        assert!(matches!(redo[0], CommandKind::PlaceTrack(_)));
        h.push_batch_inverse(CommandKind::Demolish(Demolish {
            track: TrackId(2),
        }));
        h.finish_replay();
        assert!(h.can_undo());
        assert!(!h.can_redo());
    }

    #[test]
    fn player_action_clears_redo() {
        let mut h = CommandHistory::new();
        h.record_player_action(vec![CommandKind::Demolish(Demolish {
            track: TrackId(1),
        })]);
        let _ = h.begin_undo();
        h.push_batch_inverse(CommandKind::PlaceTrack(PlaceTrack {
            tile: TileCoord { x: 1, y: 1 },
            layer: 0,
        }));
        h.finish_replay();
        assert!(h.can_redo());

        h.record_player_action(vec![CommandKind::Demolish(Demolish {
            track: TrackId(3),
        })]);
        assert!(!h.can_redo());
        assert_eq!(h.undo_len(), 1);
    }
}
