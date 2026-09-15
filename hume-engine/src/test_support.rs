//! Test-only scaffolding shared across this crate's per-module test suites:
//! shorthand for building a `Theme` from a literal list of scope styles,
//! for building a `Grapheme::byte_range` from plain integers, for
//! assembling a `RenderDisplayLine` by hand for a direct `compose_display_line` call,
//! for a char offset literal (`co`), for a `DisplayLineMap` over a fixed
//! `FormatKey` (`map`), and for a fake `VIRTUAL_LINE` decoration source
//! (`VirtualLineBlock`) — shared by `display_lines::tests` and
//! `display_lines::scroll::tests`, which are cousins under `display_lines`
//! and so can't see each other's own private test doubles.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use hume_grid::Rgb;
use ropey::Rope;

use hume_rope::column::ByteCol;
use hume_rope::line::ContentLine;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::display_lines::line_store::{FormatKey, PaneLineStore};
use crate::display_lines::{DisplayLineMap, RenderDisplayLine};
use crate::pane::{WhitespaceConfig, WrapMode};
use crate::providers::{
    Decoration, DecorationKinds, DecorationSource, ProviderSet, VirtualLine, VirtualLineAnchor,
};
use crate::theme::Theme;
use crate::types::{DisplayLine, Grapheme, ResolvedStyle};

pub(crate) fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

/// A `DisplayLineMap` over `rope`, built from a fixed `FormatKey` (tag
/// `[0; 3]`, 4-wide tabs, default whitespace) — every test in this crate
/// that needs a map builds one this way, varying only `wrap`/`providers`/
/// `content_width`/`store`.
pub(crate) fn map<'a>(
    rope: &'a Rope,
    wrap: WrapMode,
    providers: &'a ProviderSet,
    content_width: u16,
    store: &'a mut PaneLineStore,
) -> DisplayLineMap<'a> {
    DisplayLineMap::new(
        rope,
        providers,
        content_width,
        FormatKey {
            buffer_tag: [0; 3],
            wrap_mode: wrap,
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
        },
        store,
    )
}

/// What each of a [`VirtualLineBlock`]'s display lines says.
pub(crate) enum BlockText {
    /// Every display line carries the same text — for tests that only count them.
    Same(&'static str),
    /// Display lines read "1", "2", … — for tests that assert *which* one of
    /// a block landed on a given screen line, which identical text cannot show.
    Ordinal,
}

/// A VIRTUAL_LINE source emitting `count` display lines at one anchor, and
/// nothing for any other line.
pub(crate) struct VirtualLineBlock {
    anchor: VirtualLineAnchor,
    count: usize,
    text: BlockText,
    calls: Option<Rc<Cell<usize>>>,
}

impl VirtualLineBlock {
    /// `count` display lines at `anchor`, every one texted `text`.
    pub(crate) fn uniform(anchor: VirtualLineAnchor, count: usize, text: &'static str) -> Self {
        Self {
            anchor,
            count,
            text: BlockText::Same(text),
            calls: None,
        }
    }

    /// `count` display lines at `anchor`, texted "1", "2", … so a test can
    /// tell which one of the block it is looking at.
    pub(crate) fn numbered(anchor: VirtualLineAnchor, count: usize) -> Self {
        Self {
            anchor,
            count,
            text: BlockText::Ordinal,
            calls: None,
        }
    }

    /// Count queries — the observable proxy for "did the display-line map
    /// treat this line as already known rather than re-querying it".
    ///
    /// Counts only queries for this double's *own* line. A frame queries every
    /// line it walks, including whichever one the cursor happens to sit on, so
    /// an unnarrowed counter measures the walk rather than the caching under
    /// test.
    pub(crate) fn counting(mut self, calls: Rc<Cell<usize>>) -> Self {
        self.calls = Some(calls);
        self
    }

    fn line(&self) -> ContentLine {
        match self.anchor {
            VirtualLineAnchor::Before(n) | VirtualLineAnchor::After(n) => n,
        }
    }
}

impl DecorationSource for VirtualLineBlock {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }

    fn decorations_for_line(&self, line_idx: ContentLine, out: &mut Vec<Decoration>) {
        if line_idx != self.line() {
            return;
        }
        if let Some(calls) = &self.calls {
            calls.set(calls.get() + 1);
        }
        for i in 0..self.count {
            out.push(Decoration::VirtualLine(VirtualLine {
                anchor: self.anchor,
                // Ignored downstream: `DisplayLineMap::block_entry` overwrites a
                // self-reported id with the real one, so there is nothing
                // here worth parameterising.
                provider_id: 0,
                text: match self.text {
                    BlockText::Same(t) => t.to_string(),
                    BlockText::Ordinal => (i + 1).to_string(),
                },
                segments: Vec::new(),
                base_scope: None,
            }));
        }
    }
}

pub(crate) fn fg(color: Rgb) -> ResolvedStyle {
    ResolvedStyle {
        fg: Some(color),
        ..Default::default()
    }
}

pub(crate) fn bg(color: Rgb) -> ResolvedStyle {
    ResolvedStyle {
        bg: Some(color),
        ..Default::default()
    }
}

pub(crate) fn theme_with<const N: usize>(entries: [(&'static str, ResolvedStyle); N]) -> Theme {
    Theme::new(HashMap::from(entries), ResolvedStyle::default())
}

pub(crate) fn byte_range(start: usize, end: usize) -> ExclusiveRange<ByteCol> {
    ExclusiveRange::new(ByteCol::new(start), ByteCol::new(end))
}

/// Build a `RenderDisplayLine` for a direct `compose_display_line` call —
/// every test site has no provider-owned background, so `base_scope` is
/// always `None` here rather than a parameter.
pub(crate) fn render_display_line<'m>(
    display_line: &'m DisplayLine,
    graphemes: &'m [Grapheme],
    line_text: &'m str,
    virtual_texts: &'m str,
) -> RenderDisplayLine<'m> {
    RenderDisplayLine {
        display_line,
        graphemes,
        line_text,
        virtual_texts,
        base_scope: None,
    }
}
