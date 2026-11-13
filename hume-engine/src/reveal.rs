//! The lines a pane shows raw: the layout ignores their conceals.

use hume_rope::line::ContentLine;
use hume_rope::offset::InclusiveRange;

use crate::line_set::LineSet;

/// The lines whose conceals a layout leaves out. A conceal is left out when
/// its own line is revealed, or any line of its reveal span is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RevealedLines {
    Lines(LineSet),
    /// Every line, for a layout of the raw text.
    All,
}

impl Default for RevealedLines {
    fn default() -> Self {
        Self::Lines(LineSet::default())
    }
}

impl RevealedLines {
    pub fn reveals(&self, line: ContentLine) -> bool {
        match self {
            Self::Lines(lines) => lines.contains(line),
            Self::All => true,
        }
    }

    /// Whether any line of `span` is revealed.
    pub fn reveals_any(&self, span: InclusiveRange<ContentLine>) -> bool {
        match self {
            Self::Lines(lines) => lines.intersects(span),
            Self::All => true,
        }
    }
}
