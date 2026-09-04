//! Synthetic [`DecorationSource`]s for tests that need a pane to contribute
//! display rows or columns the buffer text alone doesn't account for.
//!
//! Two of them decorate: one emits whole virtual rows, the other inline
//! inserts. Both are deliberately general — parameterised on everything any
//! caller varies and on nothing else — because the alternative is what this
//! module replaced: eight near-identical `struct Foo; impl DecorationSource
//! for Foo` blocks whose differences (a text string, an anchor side, a row
//! count) were invisible next to twenty lines of identical `VirtualLine
//! { .. }` construction.
//!
//! The third, [`FormatProbe`], decorates nothing. It registers as an INLINE
//! source purely to be *asked*, because being asked is the observable
//! signature of a format: the row map queries every INLINE source exactly
//! once per `format_buffer_line` run and nowhere else. Emitting nothing is
//! what lets it measure the layout an undecorated line produces, which is
//! why it stays a separate double rather than a mode on [`InlineHint`].
//!
//! Lives under `tests/` rather than beside the code it fakes so the `lints/`
//! harness skips it — `collect_source_rs` excludes `tests/` directories, and a
//! test double has no business being scanned as production code. Reachable
//! from the sibling `editor::{cursor,scroll,mouse}::tests` subtrees because
//! `editor::tests` itself is `pub(crate)`.

use std::cell::Cell;
use std::rc::Rc;

use hume_engine::providers::{
    Decoration, DecorationKinds, DecorationSource, InlineInsert, ProviderSet, VirtualLine,
    VirtualLineAnchor,
};
use hume_engine::types::ScopeId;

/// What each of a [`VirtualRows`] block's rows says.
enum RowText {
    /// Every row carries the same text — for tests that only count rows.
    Same(&'static str),
    /// Rows read "1", "2", … — for tests that assert *which* row of a block
    /// landed on a given screen line, which identical text cannot show.
    Ordinal,
}

/// A VIRTUAL_LINE source emitting `count` rows at one anchor, and nothing for
/// any other line.
pub(crate) struct VirtualRows {
    anchor: VirtualLineAnchor,
    count: usize,
    text: RowText,
    calls: Option<Rc<Cell<usize>>>,
}

impl VirtualRows {
    /// `count` rows at `anchor`, every one texted `text`.
    pub(crate) fn uniform(anchor: VirtualLineAnchor, count: usize, text: &'static str) -> Self {
        Self {
            anchor,
            count,
            text: RowText::Same(text),
            calls: None,
        }
    }

    /// `count` rows at `anchor`, texted "1", "2", … so a test can tell which
    /// row of the block it is looking at.
    pub(crate) fn numbered(anchor: VirtualLineAnchor, count: usize) -> Self {
        Self {
            anchor,
            count,
            text: RowText::Ordinal,
            calls: None,
        }
    }

    /// Count queries — the observable proxy for "did the row map treat this
    /// line as already known rather than re-querying it".
    ///
    /// Counts only queries for this double's *own* line. A frame queries every
    /// line it walks, including whichever one the cursor happens to sit on, so
    /// an unnarrowed counter measures the walk rather than the caching under
    /// test.
    pub(crate) fn counting(mut self, calls: Rc<Cell<usize>>) -> Self {
        self.calls = Some(calls);
        self
    }

    fn line(&self) -> usize {
        match self.anchor {
            VirtualLineAnchor::Before(n) | VirtualLineAnchor::After(n) => n,
        }
    }
}

impl DecorationSource for VirtualRows {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }

    fn decorations_for_line(&self, line_idx: usize, out: &mut Vec<Decoration>) {
        if line_idx != self.line() {
            return;
        }
        if let Some(calls) = &self.calls {
            calls.set(calls.get() + 1);
        }
        for i in 0..self.count {
            out.push(Decoration::VirtualLine(VirtualLine {
                anchor: self.anchor,
                // Ignored downstream: `RowMap::block_entry` overwrites a
                // self-reported id with the real one, so there is nothing
                // here worth parameterising.
                provider_id: 0,
                text: match self.text {
                    RowText::Same(t) => t.to_string(),
                    RowText::Ordinal => (i + 1).to_string(),
                },
                segments: Vec::new(),
                base_scope: None,
            }));
        }
    }
}

/// An INLINE source emitting one insert on one line — an inlay hint, say.
pub(crate) struct InlineHint {
    line: usize,
    byte_offset: usize,
    text: &'static str,
    scope: ScopeId,
    gate: Option<Rc<Cell<bool>>>,
}

impl InlineHint {
    /// An insert of `text` at `byte_offset` on `line`, unstyled.
    pub(crate) fn new(line: usize, byte_offset: usize, text: &'static str) -> Self {
        Self {
            line,
            byte_offset,
            text,
            scope: ScopeId(0),
            gate: None,
        }
    }

    /// Carry an already-interned scope, the contract real providers follow —
    /// for a test that asserts on the styling, not just the columns.
    pub(crate) fn with_scope(mut self, scope: ScopeId) -> Self {
        self.scope = scope;
        self
    }

    /// Emit only while `on` is set, so one registered provider can answer
    /// differently on two frames driven through the same `RenderContext`.
    pub(crate) fn gated(mut self, on: Rc<Cell<bool>>) -> Self {
        self.gate = Some(on);
        self
    }
}

impl DecorationSource for InlineHint {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::INLINE
    }

    fn decorations_for_line(&self, line_idx: usize, out: &mut Vec<Decoration>) {
        if line_idx != self.line || self.gate.as_ref().is_some_and(|g| !g.get()) {
            return;
        }
        out.push(Decoration::Inline(InlineInsert {
            byte_offset: self.byte_offset,
            text: self.text.to_string(),
            scope: self.scope,
        }));
    }
}

/// Counts how many times `line` is formatted, and decorates nothing.
///
/// `RowMap` queries every registered INLINE-kind source exactly once per
/// `format_buffer_line` run and nowhere else, so being asked is a count of
/// formats that depends on nothing the row map reports about itself. Emitting
/// no insert keeps the line's layout the one it would have had unobserved.
pub(crate) struct FormatProbe {
    line: usize,
    formats: Rc<Cell<usize>>,
}

impl FormatProbe {
    pub(crate) fn new(line: usize, formats: Rc<Cell<usize>>) -> Self {
        Self { line, formats }
    }
}

impl DecorationSource for FormatProbe {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::INLINE
    }

    fn decorations_for_line(&self, line_idx: usize, _out: &mut Vec<Decoration>) {
        if line_idx == self.line {
            self.formats.set(self.formats.get() + 1);
        }
    }
}

/// No decoration source registered — every line's block reduces to its content
/// rows, which is what a test with virtual-line-unaware expectations needs.
pub(crate) fn no_providers() -> ProviderSet {
    ProviderSet::new()
}

/// One `Before(line)` row, the shape most virtual-line tests want.
pub(crate) fn providers_with_before_line(line: usize) -> ProviderSet {
    let mut p = ProviderSet::new();
    p.add_decoration_source(Box::new(VirtualRows::uniform(
        VirtualLineAnchor::Before(line),
        1,
        "V",
    )));
    p
}
