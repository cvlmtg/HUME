//! Decoration sources for tests here and in crates that enable the
//! `test-util` feature.

use std::cell::Cell;
use std::rc::Rc;

use hume_core::Side;
use hume_rope::line::ContentLine;
use hume_rope::spanned::SpannedText;

use crate::providers::{Decoration, DecorationKinds, DecorationSource, VirtualLine};

/// What each of a [`VirtualLineBlock`]'s display lines says.
enum BlockText {
    /// Every display line carries the same text, for tests that only count them.
    Same(&'static str),
    /// Display lines read "1", "2", …, for tests that assert *which* one of
    /// a block landed on a given screen line, which identical text cannot show.
    Ordinal,
}

/// A VIRTUAL_LINE source emitting `count` display lines on one side of one
/// line, and nothing for any other line.
pub struct VirtualLineBlock {
    side: Side,
    line: ContentLine,
    count: usize,
    text: BlockText,
    calls: Option<Rc<Cell<usize>>>,
}

impl VirtualLineBlock {
    /// `count` display lines on `side` of `line`, every one texted `text`.
    pub fn uniform(side: Side, line: ContentLine, count: usize, text: &'static str) -> Self {
        Self {
            side,
            line,
            count,
            text: BlockText::Same(text),
            calls: None,
        }
    }

    /// `count` display lines on `side` of `line`, texted "1", "2", … so a
    /// test can tell which one of the block it is looking at.
    pub fn numbered(side: Side, line: ContentLine, count: usize) -> Self {
        Self {
            side,
            line,
            count,
            text: BlockText::Ordinal,
            calls: None,
        }
    }

    /// Count queries: the observable proxy for "did the display-line map
    /// treat this line as already known rather than re-querying it".
    ///
    /// Counts only queries for this double's *own* line. A frame queries every
    /// line it walks, including whichever one the cursor happens to sit on, so
    /// an unnarrowed counter measures the walk rather than the caching under
    /// test.
    pub fn counting(mut self, calls: Rc<Cell<usize>>) -> Self {
        self.calls = Some(calls);
        self
    }
}

impl DecorationSource for VirtualLineBlock {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }

    fn decorations_for_line(&self, line_idx: ContentLine, out: &mut Vec<Decoration>) {
        if line_idx != self.line {
            return;
        }
        if let Some(calls) = &self.calls {
            calls.set(calls.get() + 1);
        }
        for i in 0..self.count {
            out.push(Decoration::VirtualLine(VirtualLine {
                line: self.line,
                side: self.side,
                // Ignored downstream: `DisplayLineMap::block_entry` overwrites a
                // self-reported id with the real one, so there is nothing
                // here worth parameterising.
                provider_id: 0,
                content: SpannedText::plain(match self.text {
                    BlockText::Same(t) => t.to_string(),
                    BlockText::Ordinal => (i + 1).to_string(),
                }),
                base_scope: None,
            }));
        }
    }
}
