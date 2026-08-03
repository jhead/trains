//! Player-authored lines — named coloured ordered station routes.
//!
//! See `docs/design/07-trains-and-lines.md` §2.

mod apply;
mod registry;

pub use apply::apply_line_commands;
pub use registry::{
    line_colour_rgba, line_path, suggest_line_name, Line, LineColour, LineDirection, LineRegistry,
    LineStopSlot, RemovedStops, LINE_PALETTE,
};
