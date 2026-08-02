//! Buffered player intent applied on the fixed-tick boundary.

use bevy_ecs::prelude::Resource;

use crate::commands::{CommandKind, SimCommand};

/// Queue of [`SimCommand`]s waiting for the next FixedUpdate drain.
///
/// Input / UI push here; [`crate::apply::apply_commands`] drains and applies.
#[derive(Debug, Default, Resource)]
pub struct CommandBuffer {
    pending: Vec<SimCommand>,
    /// Next sequence number to assign (`1` on first push; `0` means unassigned).
    next_sequence: u64,
}

impl CommandBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of commands waiting to be applied.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Push a command kind, assign a monotonic sequence, return that sequence.
    pub fn push(&mut self, kind: CommandKind) -> u64 {
        self.next_sequence = self.next_sequence.saturating_add(1);
        let sequence = self.next_sequence;
        self.pending.push(SimCommand { sequence, kind });
        sequence
    }

    /// Peek at pending commands without draining.
    pub fn pending(&self) -> &[SimCommand] {
        &self.pending
    }

    /// Take all pending commands in push order (already sequenced).
    pub fn drain(&mut self) -> Vec<SimCommand> {
        core::mem::take(&mut self.pending)
    }

    /// Monotonic counter of the last assigned sequence (0 if none pushed yet).
    pub fn last_sequence(&self) -> u64 {
        self.next_sequence
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Pause, SetSpeed};

    #[test]
    fn push_assigns_monotonic_sequences() {
        let mut buf = CommandBuffer::new();
        let a = buf.push(CommandKind::Pause(Pause { paused: true }));
        let b = buf.push(CommandKind::SetSpeed(SetSpeed { multiplier: 3 }));
        let c = buf.push(CommandKind::Pause(Pause { paused: false }));

        assert_eq!((a, b, c), (1, 2, 3));
        assert_eq!(buf.len(), 3);

        let drained = buf.drain();
        assert!(buf.is_empty());
        assert_eq!(
            drained.iter().map(|c| c.sequence).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(matches!(drained[0].kind, CommandKind::Pause(_)));
        assert!(matches!(drained[1].kind, CommandKind::SetSpeed(_)));

        // Sequences keep climbing across drains.
        let d = buf.push(CommandKind::SetSpeed(SetSpeed { multiplier: 1 }));
        assert_eq!(d, 4);
    }
}
