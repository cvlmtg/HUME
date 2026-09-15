use std::cell::Cell;
use std::rc::Rc;

use ropey::Rope;

use super::*;
use crate::pane::{WhitespaceConfig, WrapMode};
use crate::providers::DecorationSource;
use crate::test_support::{VirtualLineBlock, co};
use crate::types::{CellContent, ScopeId};
use hume_rope::column::{BufferLineCol, DisplayLineCol};
use hume_rope::line::{ContentLine, RopeyLine};
use hume_rope::lines::content_line_count;
use hume_rope::offset::{CharOffset, ExclusiveRange};

mod block;
mod bounded_format;
mod buffer_line_col;
mod locate;
mod render;
mod shared_store;
mod stepping;

fn dc(n: u32) -> DisplayLineCol {
    DisplayLineCol::new(n)
}

fn ldc(n: u32) -> BufferLineCol {
    BufferLineCol::new(n)
}

fn ex(start: usize, end: usize) -> ExclusiveRange<CharOffset> {
    ExclusiveRange::new(co(start), co(end))
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn ws() -> WhitespaceConfig {
    WhitespaceConfig::default()
}

/// [`crate::test_support::map`] at this suite's fixed 80-column width —
/// every test in this tree that doesn't specifically vary width uses this.
fn map<'a>(
    rope: &'a Rope,
    wrap: WrapMode,
    providers: &'a ProviderSet,
    store: &'a mut PaneLineStore,
) -> DisplayLineMap<'a> {
    crate::test_support::map(rope, wrap, providers, 80, store)
}

/// One inline insert on `line`, counting how often it is queried — the only
/// observable proxy for "did the map run the formatter".
struct CountingInsert {
    line: ContentLine,
    byte_offset: usize,
    text: &'static str,
    calls: Rc<Cell<usize>>,
}

impl DecorationSource for CountingInsert {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::INLINE
    }
    fn decorations_for_line(&self, line_idx: ContentLine, out: &mut Vec<Decoration>) {
        self.calls.set(self.calls.get() + 1);
        if line_idx == self.line {
            out.push(Decoration::Inline(InlineInsert {
                byte_offset: hume_rope::column::ByteCol::new(self.byte_offset),
                text: self.text.to_string(),
                scope: ScopeId(0),
            }));
        }
    }
}

fn with_counting_insert(
    line: ContentLine,
    byte_offset: usize,
    text: &'static str,
) -> (ProviderSet, Rc<Cell<usize>>) {
    let calls = Rc::new(Cell::new(0));
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(CountingInsert {
        line,
        byte_offset,
        text,
        calls: Rc::clone(&calls),
    }));
    (providers, calls)
}

/// Reconstruct the text a rendered display line puts on screen, cell by cell. Derived
/// from the `Grapheme`/`CellContent` contract rather than from anything
/// `DisplayLineMap` computed, so it is an independent check of the render accessors.
fn display_line_text(r: &RenderDisplayLine<'_>) -> String {
    r.graphemes[r.display_line.graphemes.clone()]
        .iter()
        .filter_map(|g| match g.content {
            CellContent::Virtual { start, len }
            | CellContent::Whitespace { start, len }
            | CellContent::Placeholder { start, len } => {
                let start = start as usize;
                Some(r.virtual_texts[start..start + len as usize].to_string())
            }
            // No arena entry — blank across its whole reserved width, same
            // as `render::compose_display_line`'s `TabFill` arm draws it on screen.
            CellContent::TabFill => Some(" ".repeat(g.width as usize)),
            CellContent::Grapheme => {
                Some(r.line_text[g.byte_range.start.index()..g.byte_range.end.index()].to_string())
            }
            CellContent::WidthContinuation | CellContent::Empty => None,
        })
        .collect()
}
