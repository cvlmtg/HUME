//! Synthetic [`DecorationSource`]s for tests that need a pane to contribute
//! display lines or columns the buffer text alone doesn't account for.
//!
//! [`InlineHint`] emits inline inserts. It is parameterised on everything
//! any caller varies and on nothing else, so a test states only what sets it
//! apart (a text string, a byte offset) instead of a `struct Foo; impl
//! DecorationSource for Foo` block that buries it in identical
//! `InlineInsert { .. }` construction. Whole virtual display lines come from
//! `hume_engine::test_doubles::VirtualLineBlock`.
//!
//! [`FormatProbe`] decorates nothing. It registers as an INLINE
//! source purely to be *asked*, because being asked is the observable
//! signature of a format: the display-line map queries every INLINE source
//! exactly once per `format_buffer_line` run and nowhere else. Emitting
//! nothing is what lets it measure the layout an undecorated line produces,
//! which is why it stays a separate double rather than a mode on
//! [`InlineHint`].
//!
//! Lives under `tests/` rather than beside the code it fakes so the `lints/`
//! harness skips it: `collect_source_rs` excludes `tests/` directories, and a
//! test double has no business being scanned as production code. Reachable
//! from the sibling `editor::{cursor,scroll,mouse}::tests` subtrees because
//! `editor::tests` itself is `pub(crate)`.

use std::cell::Cell;
use std::rc::Rc;

use hume_core::Side;
use hume_engine::providers::{
    Decoration, DecorationKinds, DecorationSource, InlineInsert, ProviderSet,
};
use hume_engine::test_doubles::VirtualLineBlock;
use hume_engine::types::ScopeId;

/// An INLINE source emitting one insert on one line, an inlay hint, say.
pub(in crate::editor) struct InlineHint {
    line: hume_rope::line::ContentLine,
    byte_offset: usize,
    text: &'static str,
    scope: ScopeId,
    gate: Option<Rc<Cell<bool>>>,
}

impl InlineHint {
    /// An insert of `text` at `byte_offset` on `line`, unstyled.
    pub(in crate::editor) fn new(line: usize, byte_offset: usize, text: &'static str) -> Self {
        Self {
            line: hume_rope::line::ContentLine::new(line),
            byte_offset,
            text,
            scope: ScopeId(0),
            gate: None,
        }
    }

    /// Carry an already-interned scope, the contract real providers follow,
    /// for a test that asserts on the styling, not just the columns.
    pub(in crate::editor) fn with_scope(mut self, scope: ScopeId) -> Self {
        self.scope = scope;
        self
    }

    /// Emit only while `on` is set, so one registered provider can answer
    /// differently on two frames driven through the same `RenderContext`.
    pub(in crate::editor) fn gated(mut self, on: Rc<Cell<bool>>) -> Self {
        self.gate = Some(on);
        self
    }
}

impl DecorationSource for InlineHint {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::INLINE
    }

    fn decorations_for_line(
        &self,
        line_idx: hume_rope::line::ContentLine,
        out: &mut Vec<Decoration>,
    ) {
        if line_idx != self.line || self.gate.as_ref().is_some_and(|g| !g.get()) {
            return;
        }
        out.push(Decoration::Inline(InlineInsert {
            byte_offset: hume_rope::column::ByteCol::new(self.byte_offset),
            text: self.text.into(),
            scope: self.scope,
        }));
    }
}

/// Counts how many times `line` is formatted, and decorates nothing.
///
/// `DisplayLineMap` queries every registered INLINE-kind source exactly once per
/// `format_buffer_line` run and nowhere else, so being asked is a count of
/// formats that depends on nothing the display-line map reports about
/// itself. Emitting no insert keeps the line's layout the one it would have
/// had unobserved.
pub(in crate::editor) struct FormatProbe {
    line: hume_rope::line::ContentLine,
    formats: Rc<Cell<usize>>,
}

impl FormatProbe {
    pub(in crate::editor) fn new(line: usize, formats: Rc<Cell<usize>>) -> Self {
        Self {
            line: hume_rope::line::ContentLine::new(line),
            formats,
        }
    }
}

impl DecorationSource for FormatProbe {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::INLINE
    }

    fn decorations_for_line(
        &self,
        line_idx: hume_rope::line::ContentLine,
        _out: &mut Vec<Decoration>,
    ) {
        if line_idx == self.line {
            self.formats.set(self.formats.get() + 1);
        }
    }
}

/// No decoration source registered: every line's block reduces to its content
/// rows, which is what a test with virtual-line-unaware expectations needs.
pub(in crate::editor) fn no_providers() -> ProviderSet {
    ProviderSet::new()
}

/// One row before `line`, the shape most virtual-line tests want.
pub(in crate::editor) fn providers_with_before_line(line: usize) -> ProviderSet {
    let mut p = ProviderSet::new();
    p.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        Side::Before,
        hume_rope::line::ContentLine::new(line),
        1,
        "V",
    )));
    p
}
