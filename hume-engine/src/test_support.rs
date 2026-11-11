//! Test-only scaffolding shared across this crate's per-module test suites:
//! shorthand for building a `Theme` from a literal list of scope styles,
//! for building a `Grapheme::byte_range` from plain integers, for
//! assembling a `RenderDisplayLine` by hand for a direct `compose_display_line` call,
//! for a char offset literal (`co`), for the cluster at a char offset
//! (`at`), for a painted selection (`painted`) and a pane's selection mirror
//! (`mirror`, `cursor_mirror`), for a `DisplayLineMap` over a fixed
//! `FormatKey` (`map`, or `map_hiding` with closed folds), for a pane's geometry with no header rows
//! (`plain_geo`), and for a body row with the primary head on line 0
//! (`body_row_on_line_0`).

use std::collections::HashMap;

use hume_grid::Rgb;
use ropey::Rope;

use hume_rope::cluster::ClusterStart;
use hume_rope::column::ByteCol;
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::display_lines::line_store::{FormatKey, PaneLineStore};
use crate::display_lines::{DisplayLineMap, RenderDisplayLine};
use crate::folds::HiddenLines;
use crate::pane::{ViewGeometry, WhitespaceConfig, WrapMode};
use crate::pipeline::PaneSize;
use crate::providers::{GutterRow, ProviderSet};
use crate::scope_headers::ScopeHeaderInputs;
use crate::theme::Theme;
use crate::types::{DisplayLine, Grapheme, PaintedSelection, PaintedSelections, ResolvedStyle};

pub(crate) fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

/// The cluster starting at char `n` of `rope`. Panics when `n` is not a
/// cluster start, so a test's literal positions stay the ones it names.
pub(crate) fn at(rope: &Rope, n: usize) -> ClusterStart {
    let start = hume_rope::grapheme::snap_to_cluster(rope.slice(..), co(n))
        .expect("a non-empty text")
        .start();
    assert_eq!(start.offset(), co(n), "char {n} is not a cluster start");
    start
}

/// A selection over `rope` from the cluster at char `anchor` to the one at
/// char `head`, as the host would paint it.
pub(crate) fn painted(rope: &Rope, anchor: usize, head: usize) -> PaintedSelection {
    let (anchor, head) = (at(rope, anchor), at(rope, head));
    PaintedSelection {
        first: anchor.min(head),
        last: anchor.max(head),
        cursor: head,
    }
}

/// A selection mirror over `rope` of `(anchor, head)` char pairs, in cursor
/// order, the one at `primary` primary.
pub(crate) fn mirror(rope: &Rope, sels: &[(usize, usize)], primary: usize) -> PaintedSelections {
    let items = sels
        .iter()
        .map(|&(anchor, head)| painted(rope, anchor, head))
        .collect();
    PaintedSelections::new(items, primary)
}

/// A selection mirror holding one primary cursor on char `n` of `rope`.
pub(crate) fn cursor_mirror(rope: &Rope, n: usize) -> PaintedSelections {
    mirror(rope, &[(n, n)], 0)
}

/// A `DisplayLineMap` over `rope`, built from a fixed `FormatKey` (tag
/// `[0; 3]`, 4-wide tabs, default whitespace). Every test in this crate
/// that needs a map builds one this way, varying only `wrap`/`providers`/
/// `content_width`/`store`.
pub(crate) fn map<'a>(
    rope: &'a Rope,
    wrap: WrapMode,
    providers: &'a ProviderSet,
    content_width: u16,
    store: &'a mut PaneLineStore,
) -> DisplayLineMap<'a> {
    map_hiding(
        rope,
        wrap,
        providers,
        content_width,
        HiddenLines::default(),
        store,
    )
}

/// [`map`] with the lines `hidden` names folded away.
pub(crate) fn map_hiding<'a>(
    rope: &'a Rope,
    wrap: WrapMode,
    providers: &'a ProviderSet,
    content_width: u16,
    hidden: HiddenLines,
    store: &'a mut PaneLineStore,
) -> DisplayLineMap<'a> {
    DisplayLineMap::new(
        rope,
        providers,
        crate::pipeline::ContentWidth::for_test(content_width),
        FormatKey {
            buffer_tag: [0; 3],
            wrap_mode: wrap,
            tab_width: 4,
            whitespace: WhitespaceConfig::default(),
        },
        hidden,
        store,
    )
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

/// Build a `RenderDisplayLine` for a direct `compose_display_line` call.
/// Every test site has no provider-owned background, so `base_scope` is
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

/// The geometry of a pane of `size` with scroll-margin `margin` and no
/// header rows. Panics on a zero-height `size`, which has none.
pub(crate) fn plain_geo(size: PaneSize, margin: usize) -> ViewGeometry<'static> {
    size.geometry(margin, ScopeHeaderInputs::NONE)
        .expect("a nonzero-height pane has a geometry")
}

/// A body row of a pane whose primary head is on line 0.
pub(crate) fn body_row_on_line_0() -> GutterRow {
    GutterRow::Body {
        primary_head_line: hume_rope::line::ContentLine::new(0),
    }
}
