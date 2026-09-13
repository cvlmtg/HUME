use std::cell::Cell;
use std::rc::Rc;

use ropey::Rope;

use super::*;
use crate::pane::{WhitespaceConfig, WrapMode};
use crate::providers::{DecorationSource, VirtualLine};
use crate::types::{CellContent, ScopeId};
use hume_rope::column::{BufferLineCol, DisplayLineCol};
use hume_rope::line::{ContentLine, RopeyLine};
use hume_rope::lines::content_line_count;
use hume_rope::offset::{CharOffset, ExclusiveRange};

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

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

fn map<'a>(
    rope: &'a Rope,
    wrap: WrapMode,
    providers: &'a ProviderSet,
    store: &'a mut PaneLineStore,
) -> DisplayLineMap<'a> {
    DisplayLineMap::new(
        rope,
        providers,
        80,
        FormatKey {
            buffer_tag: [0; 3],
            wrap_mode: wrap,
            tab_width: 4,
            whitespace: ws(),
        },
        store,
    )
}

/// How many graphemes the formatter actually emitted for `line` — an oracle
/// over `format.rs`'s output, read from the store after the map that filled
/// it is gone.
fn stored_graphemes(store: &PaneLineStore, line: ContentLine) -> usize {
    store
        .find(line)
        .map_or(0, |i| store.entry(i).format.graphemes.len())
}

/// Whether `line` has an entry that was never formatted — the state a no-wrap
/// line sits in when only its block shape was ever asked for.
fn is_unformatted(store: &PaneLineStore, line: ContentLine) -> bool {
    store
        .find(line)
        .is_some_and(|i| store.entry(i).format.extent.is_none())
}

/// Emits `count` identical display lines at one fixed anchor. Self-reports
/// `provider_id: 0` so the id-stamping test has something wrong to correct.
struct FixedAnchor {
    anchor: VirtualLineAnchor,
    count: usize,
    text: &'static str,
    calls: Option<Rc<Cell<usize>>>,
}

impl FixedAnchor {
    fn new(anchor: VirtualLineAnchor, count: usize) -> Self {
        Self::texted(anchor, count, "V")
    }

    fn texted(anchor: VirtualLineAnchor, count: usize, text: &'static str) -> Self {
        Self {
            anchor,
            count,
            text,
            calls: None,
        }
    }

    /// Count *every* `decorations_for_line` call, whatever line it asks about
    /// — the only observable proxy for whether `block_entry` treated a line as
    /// already known rather than re-querying providers for it.
    fn counting(mut self, calls: Rc<Cell<usize>>) -> Self {
        self.calls = Some(calls);
        self
    }
}

impl DecorationSource for FixedAnchor {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(&self, line_idx: ContentLine, out: &mut Vec<Decoration>) {
        if let Some(calls) = &self.calls {
            calls.set(calls.get() + 1);
        }
        let line = match self.anchor {
            VirtualLineAnchor::Before(n) | VirtualLineAnchor::After(n) => n,
        };
        if line_idx == line {
            for _ in 0..self.count {
                out.push(Decoration::VirtualLine(VirtualLine {
                    anchor: self.anchor,
                    provider_id: 0,
                    text: self.text.to_string(),
                    segments: Vec::new(),
                    base_scope: None,
                }));
            }
        }
    }
}

/// A VIRTUAL_LINE-kind source that never emits anything — registered only to
/// consume a `ProviderId` so the next provider's real id is not 0.
struct NoVirtualLines;

impl DecorationSource for NoVirtualLines {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(&self, _line_idx: ContentLine, _out: &mut Vec<Decoration>) {}
}

/// A call-counting source emitting one `Before(0)` display line, so the entry whose
/// re-query the counter is watching has a non-trivial block shape.
fn with_counting_anchor() -> (ProviderSet, Rc<Cell<usize>>) {
    let calls = Rc::new(Cell::new(0));
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(
        FixedAnchor::new(VirtualLineAnchor::Before(ContentLine::new(0)), 1)
            .counting(Rc::clone(&calls)),
    ));
    (providers, calls)
}

/// A LINE_BG-kind source that counts every `decorations_for_line` call —
/// used to prove the layout stage (`block`/`ensure_formatted`) never queries a
/// kind it has no use for, even when the same line is both counted and
/// formatted.
struct CountingLineBg(Rc<Cell<usize>>);

impl DecorationSource for CountingLineBg {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::LINE_BG
    }
    fn decorations_for_line(&self, _line_idx: ContentLine, _out: &mut Vec<Decoration>) {
        self.0.set(self.0.get() + 1);
    }
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

/// Walk the whole document forward from its first display line.
fn walk_forward(dlm: &mut DisplayLineMap<'_>) -> Vec<DisplayLinePos> {
    let mut positions = vec![DisplayLinePos::default()];
    while let Some(next) = dlm.next(*positions.last().expect("seeded above")) {
        positions.push(next);
    }
    positions
}

// ---------------------------------------------------------------------------
// block()
// ---------------------------------------------------------------------------

#[test]
fn block_without_providers_is_content_only() {
    let rope = Rope::from_str("hello\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.block(ContentLine::new(0)),
        BlockBreakdown {
            before: 0,
            content: 1,
            after: 0,
        }
    );
}

#[test]
fn block_counts_one_content_display_line_per_wrap_display_line() {
    // "abcdefgh" is 8 columns wrapped at 4 → "abcd" / "efgh" = 2 display lines, and
    // "efgh" exactly fills its display line, so the trailing '\n's own sentinel wraps
    // onto a third display line rather than landing past the pane's edge.
    let rope = Rope::from_str("abcdefgh\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(dlm.block(ContentLine::new(0)).content, 3);
}

#[test]
fn block_counts_before_and_after_virtual_lines() {
    // Line 5 of "a\nb\nc\nd\ne\nf\n" gets 2 Before display lines and 1 After display line from
    // two separate providers, on top of its own single unwrapped content display line.
    let rope = Rope::from_str("a\nb\nc\nd\ne\nf\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::Before(ContentLine::new(5)),
        2,
    )));
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::After(ContentLine::new(5)),
        1,
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    let block = dlm.block(ContentLine::new(5));
    assert_eq!(
        block,
        BlockBreakdown {
            before: 2,
            content: 1,
            after: 1,
        }
    );
    assert_eq!(block.total(), 4);
}

#[test]
fn block_ignores_virtual_lines_anchored_to_other_lines() {
    let rope = Rope::from_str("a\nb\nc\nd\ne\nf\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::Before(ContentLine::new(2)),
        3,
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(dlm.block(ContentLine::new(5)).before, 0);
    assert_eq!(dlm.block(ContentLine::new(5)).after, 0);
}

#[test]
fn layout_stage_never_queries_a_paint_only_kind() {
    // block() drives both the VIRTUAL_LINE query and, under wrapping,
    // ensure_formatted()'s INLINE query — a LINE_BG-kind source (paint-only)
    // must be invisible to both.
    let calls = Rc::new(Cell::new(0));
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(CountingLineBg(Rc::clone(&calls))));
    let rope = Rope::from_str("abcdef\n");
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    dlm.block(ContentLine::new(0));
    assert_eq!(
        calls.get(),
        0,
        "block() must never query a LINE_BG-only source"
    );
}

#[test]
fn block_counts_inline_inserts_toward_wrapping() {
    // The bug this fixes: an inlay hint participates in wrapping, so a line
    // that fits without it can need two display lines with it, and display line counting that
    // ignores inserts disagrees with what the renderer emits.
    //
    // "abcdef" is 6 columns; wrapped at 8 that is one display line. A 4-column insert
    // at the head of the line makes 10 columns, which is two.
    let rope = Rope::from_str("abcdef\n");
    let wrap = WrapMode::Soft { width: 8 };

    let bare = ProviderSet::new();
    let mut s = PaneLineStore::new();
    assert_eq!(
        map(&rope, wrap, &bare, &mut s)
            .block(ContentLine::new(0))
            .content,
        1,
        "6 columns fit on one display line at width 8"
    );

    let (providers, _calls) = with_counting_insert(ContentLine::new(0), 0, "hint");
    let mut s = PaneLineStore::new();
    assert_eq!(
        map(&rope, wrap, &providers, &mut s)
            .block(ContentLine::new(0))
            .content,
        2,
        "4 columns of inlay hint push the line's 6 columns past width 8"
    );
}

#[test]
fn no_wrap_block_counts_without_running_the_formatter() {
    // `WrapMode::None` is always one content display line, so counting must not format
    // — that is what keeps a display line query O(1) instead of O(line length) on a
    // minified line megabytes wide. Querying decorations is what formatting
    // does first, so a zero call count is the observable proxy.
    let rope = Rope::from_str("abcdef\n");
    let (providers, calls) = with_counting_insert(ContentLine::new(0), 0, "hint");
    let mut s = PaneLineStore::new();

    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);
    assert_eq!(dlm.block(ContentLine::new(0)).content, 1);
    assert_eq!(calls.get(), 0, "no-wrap counting must not format");

    // The same query while wrapping has to format, because the display line count
    // genuinely depends on the content.
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 8 }, &providers, &mut s);
    dlm.block(ContentLine::new(0));
    assert!(
        calls.get() > 0,
        "wrapping must format to count display lines"
    );
}

// ---------------------------------------------------------------------------
// slot() / clamp() / last_line()
// ---------------------------------------------------------------------------

/// Line 0 wrapped into 2 content display lines, with 2 Before display lines and 1 After display line:
/// a 5-display-line block whose every display line slot is hand-known.
fn mixed_block_providers() -> ProviderSet {
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        2,
    )));
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::After(ContentLine::new(0)),
        1,
    )));
    providers
}

#[test]
fn slot_classifies_every_display_line_of_a_mixed_block() {
    // "abcdefgh\n" at width 4 supplies 3 content display lines, not 2: "efgh" exactly
    // fills the wrap width, so the trailing '\n's own sentinel wraps onto a
    // display line of its own (see `content_display_line_char_bounds_scopes_to_one_wrap_display_line`).
    let rope = Rope::from_str("abcdefgh\n");
    let providers = mixed_block_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(dlm.block(ContentLine::new(0)).total(), 6);
    let slots: Vec<BlockSlot> = (0..6)
        .map(|slot_idx| dlm.slot(DisplayLinePos::new(ContentLine::new(0), slot_idx)))
        .collect();
    assert_eq!(
        slots,
        vec![
            BlockSlot::Before(0),
            BlockSlot::Before(1),
            BlockSlot::Content(0),
            BlockSlot::Content(1),
            BlockSlot::Content(2),
            BlockSlot::After(0),
        ]
    );
}

#[test]
fn clamp_pulls_line_and_slot_into_the_document() {
    let rope = Rope::from_str("abcdefgh\n");
    let providers = mixed_block_providers();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(
        dlm.clamp(DisplayLinePos::new(ContentLine::new(0), 99)),
        DisplayLinePos::new(ContentLine::new(0), 5),
        "display line clamps to the block's last display line"
    );
    assert_eq!(
        dlm.clamp(DisplayLinePos::new(ContentLine::new(99), 99)),
        DisplayLinePos::new(ContentLine::new(0), 5),
        "line clamps to the last real line, then display line to its block"
    );
    assert_eq!(
        dlm.clamp(DisplayLinePos::new(ContentLine::new(0), 2)),
        DisplayLinePos::new(ContentLine::new(0), 2),
        "an address already inside the document is untouched"
    );
}

#[test]
fn last_line_excludes_the_phantom_trailing_line() {
    // Every buffer ends with a structural '\n', so ropey's `len_lines()`
    // reports one extra empty line past the content; `last_line` must not
    // count it as a real line.
    let rope = Rope::from_str("a\nb\nc\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(dlm.last_line(), ContentLine::new(2));
}

#[test]
fn clamp_reaches_the_documents_very_last_display_line() {
    // "a\nb\nc\n" has 3 real lines; line 2 carries 1 After display line, so the
    // document's last display line is that virtual display line at (2, 1). `clamp` is the
    // documented way to reach it (DisplayLineMap has no dedicated accessor).
    let rope = Rope::from_str("a\nb\nc\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::After(ContentLine::new(2)),
        1,
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.clamp(DisplayLinePos::new(
            ContentLine::new(usize::MAX),
            usize::MAX
        )),
        DisplayLinePos::new(ContentLine::new(2), 1)
    );
}

// ---------------------------------------------------------------------------
// Stepping
// ---------------------------------------------------------------------------

/// "a\nb\nc\n" with 2 Before display lines on line 1 and 1 After display line on line 2. Hand-
/// written display line list, in document order:
///
/// ```text
///   (0,0)  line 0 content
///   (1,0)  line 1 Before #0
///   (1,1)  line 1 Before #1
///   (1,2)  line 1 content
///   (2,0)  line 2 content
///   (2,1)  line 2 After #0
/// ```
fn three_line_doc() -> (Rope, ProviderSet, Vec<DisplayLinePos>) {
    let rope = Rope::from_str("a\nb\nc\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::Before(ContentLine::new(1)),
        2,
    )));
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::After(ContentLine::new(2)),
        1,
    )));
    let expected = vec![
        DisplayLinePos::new(ContentLine::new(0), 0),
        DisplayLinePos::new(ContentLine::new(1), 0),
        DisplayLinePos::new(ContentLine::new(1), 1),
        DisplayLinePos::new(ContentLine::new(1), 2),
        DisplayLinePos::new(ContentLine::new(2), 0),
        DisplayLinePos::new(ContentLine::new(2), 1),
    ];
    (rope, providers, expected)
}

#[test]
fn next_walks_the_documents_display_lines_in_order() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(walk_forward(&mut dlm), expected);
}

#[test]
fn prev_is_the_inverse_of_next() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    let mut backward = vec![*expected.last().expect("non-empty")];
    while let Some(prev) = dlm.prev(*backward.last().expect("seeded above")) {
        backward.push(prev);
    }
    backward.reverse();
    assert_eq!(backward, expected);
}

#[test]
fn next_and_prev_stop_exactly_at_the_documents_edges() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    let first = expected[0];
    let last = *expected.last().expect("non-empty");
    assert_eq!(
        dlm.prev(first),
        None,
        "no display line precedes the document's first display line"
    );
    assert_eq!(
        dlm.next(last),
        None,
        "no display line follows the document's last display line"
    );
}

#[test]
fn advance_matches_repeated_stepping_and_saturates_at_both_ends() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    for (i, &from) in expected.iter().enumerate() {
        for (j, &to) in expected.iter().enumerate() {
            let delta = j as isize - i as isize;
            assert_eq!(
                dlm.advance(from, delta),
                to,
                "advance({from:?}, {delta}) should reach {to:?}"
            );
        }
    }

    let first = expected[0];
    let last = *expected.last().expect("non-empty");
    assert_eq!(
        dlm.advance(first, -10),
        first,
        "saturates at the first display line"
    );
    assert_eq!(
        dlm.advance(last, 10),
        last,
        "saturates at the last display line"
    );
}

#[test]
fn advance_counted_reports_how_far_it_actually_stepped() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    // In bounds: the count matches the requested delta exactly.
    for (i, &from) in expected.iter().enumerate() {
        for (j, &to) in expected.iter().enumerate() {
            let delta = j as isize - i as isize;
            let (pos, taken) = dlm.advance_counted(from, delta);
            assert_eq!(pos, to, "advance_counted({from:?}, {delta}) position");
            assert_eq!(
                taken,
                delta.unsigned_abs(),
                "advance_counted({from:?}, {delta}) count"
            );
        }
    }

    // Past either edge: the document stops the walk short, so the count is
    // less than what was asked for.
    let first = expected[0];
    let last = *expected.last().expect("non-empty");
    assert_eq!(
        dlm.advance_counted(first, -10),
        (first, 0),
        "no display lines exist before the first display line"
    );
    assert_eq!(
        dlm.advance_counted(last, 10),
        (last, 0),
        "no display lines exist past the last display line"
    );

    // The documented "count is also the distance back" property
    // `scroll::scroll_back_from` relies on: having stepped `n` display lines backward
    // from `pos`, `distance` from the landing spot back to `pos` is that
    // same `n`.
    let pos = expected[4];
    let (landed, taken) = dlm.advance_counted(pos, -3);
    assert_eq!(dlm.distance(landed, pos, expected.len()), Some(taken));
}

#[test]
fn distance_counts_display_lines_forward_and_rejects_backward_or_distant() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    for (i, &from) in expected.iter().enumerate() {
        for (j, &to) in expected.iter().enumerate().skip(i) {
            assert_eq!(
                dlm.distance(from, to, 16),
                Some(j - i),
                "distance({from:?} → {to:?})"
            );
        }
    }

    assert_eq!(
        dlm.distance(expected[3], expected[1], 16),
        None,
        "a display line behind the start is not reachable forward"
    );
    assert_eq!(
        dlm.distance(expected[0], expected[5], 2),
        None,
        "5 display lines away is beyond a cap of 2"
    );
    assert_eq!(
        dlm.distance(expected[0], expected[2], 2),
        Some(2),
        "exactly at the cap still resolves"
    );
}

#[test]
fn fits_in_counts_virtual_lines_toward_the_height() {
    // The hand-written list above is 6 display lines: 3 content + 3 virtual.
    let (rope, providers, expected) = three_line_doc();
    assert_eq!(expected.len(), 6);
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert!(dlm.fits_in(6), "6 display lines fit in 6");
    assert!(!dlm.fits_in(5), "6 display lines do not fit in 5");
    assert!(
        !dlm.fits_in(3),
        "counting content display lines alone would wrongly fit 3 lines in 3 display lines"
    );
}

#[test]
fn fits_in_zero_height_never_fits() {
    // Every document has at least one display line (even a single empty line), so a
    // zero-height viewport can never fit it — regardless of how short the
    // document is.
    let rope = Rope::from_str("x\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert!(!dlm.fits_in(0));
}

#[test]
fn degenerate_single_empty_line_document() {
    // A bare "\n" is the smallest possible buffer under the invariant: one
    // real (empty) line plus the structural trailing newline.
    let rope = Rope::from_str("\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(dlm.last_line(), ContentLine::new(0));
    assert_eq!(
        dlm.clamp(DisplayLinePos::new(ContentLine::new(99), 99)),
        DisplayLinePos::new(ContentLine::new(0), 0),
        "the only display line in a one-line document is (0, 0)"
    );
    assert!(
        dlm.fits_in(1),
        "the document's single display line fits a height of 1"
    );
    assert!(!dlm.fits_in(0));
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "content_width >= 1")]
fn new_panics_on_zero_content_width() {
    let rope = Rope::from_str("x\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    DisplayLineMap::new(
        &rope,
        &providers,
        0,
        FormatKey {
            buffer_tag: [0; 3],
            wrap_mode: WrapMode::None,
            tab_width: 4,
            whitespace: ws(),
        },
        &mut s,
    );
}

#[test]
fn wrap_width_one_emits_one_grapheme_per_display_line_without_hanging() {
    // One grapheme per display line at wrap width 1, plus a 5th display line for the trailing
    // '\n's own sentinel: every display line at width 1 exactly fills, so the
    // sentinel always wraps onto a display line of its own here.
    let rope = Rope::from_str("abcd\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = DisplayLineMap::new(
        &rope,
        &providers,
        80,
        FormatKey {
            buffer_tag: [0; 3],
            wrap_mode: WrapMode::Soft { width: 1 },
            tab_width: 4,
            whitespace: ws(),
        },
        &mut s,
    );

    let breakdown = dlm.block(ContentLine::new(0));
    assert_eq!(breakdown.content, 5);
}

// ---------------------------------------------------------------------------
// locate() / char_at()
// ---------------------------------------------------------------------------

#[test]
fn locate_returns_the_wrap_display_line_and_column_of_a_char() {
    // "abcdefgh\n" at width 4: display line 0 holds chars 0..3, display line 1 holds 4..7.
    let rope = Rope::from_str("abcdefgh\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(
        dlm.locate(co(0)),
        (DisplayLinePos::new(ContentLine::new(0), 0), dc(0)),
        "'a' — display line 0, column 0"
    );
    assert_eq!(
        dlm.locate(co(2)),
        (DisplayLinePos::new(ContentLine::new(0), 0), dc(2)),
        "'c' — display line 0, column 2"
    );
    assert_eq!(
        dlm.locate(co(4)),
        (DisplayLinePos::new(ContentLine::new(0), 1), dc(0)),
        "'e' — display line 1, column 0"
    );
    assert_eq!(
        dlm.locate(co(5)),
        (DisplayLinePos::new(ContentLine::new(0), 1), dc(1)),
        "'f' — display line 1, column 1"
    );
}

#[test]
fn locate_offsets_the_display_line_by_the_lines_before_block() {
    // Same line, now with 2 Before display lines above it: 'f' keeps its column but
    // its block display line shifts from 1 to 3.
    let rope = Rope::from_str("abcdefgh\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        2,
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(
        dlm.locate(co(5)),
        (DisplayLinePos::new(ContentLine::new(0), 3), dc(1))
    );
}

#[test]
fn locate_skips_a_mid_line_inline_insert_sharing_the_real_graphemes_offset() {
    // "ab\n", no wrap: an inline insert "XY" (an inlay hint, say) spliced in
    // right before 'b' shares 'b's char_offset (1). `locate` must resolve to
    // the real grapheme's column — 'a' at 0, the insert's own two cells at 1
    // and 2, 'b' at 3 — not the insert's column, matching what
    // `style::resolve_grapheme_display_col` already guarantees for selection styling.
    let rope = Rope::from_str("ab\n");
    let (providers, _calls) = with_counting_insert(ContentLine::new(0), 1, "XY");
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.locate(co(1)),
        (DisplayLinePos::new(ContentLine::new(0), 0), dc(3))
    );
}

#[test]
fn char_at_cell_lands_on_the_eol_sentinel_past_the_text() {
    // "hi\n": h at column 0, i at column 1, and the end-of-line sentinel at
    // column 2 standing for the '\n' (char 2) — a real cursor position in
    // HUME's inclusive selection model, so a click out to the right lands
    // there rather than back on 'i'.
    let rope = Rope::from_str("hi\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.char_at(
            DisplayLinePos::new(ContentLine::new(0), 0),
            dc(99),
            DisplayColTarget::Cell
        ),
        co(2)
    );
}

#[test]
fn char_at_nearest_content_stays_off_the_eol_sentinel() {
    // The same display line asked the other question: a sticky column past the end of
    // the text resolves to the last real character, never the '\n'.
    let rope = Rope::from_str("hi\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.char_at(
            DisplayLinePos::new(ContentLine::new(0), 0),
            dc(99),
            DisplayColTarget::NearestContent
        ),
        co(1)
    );
}

#[test]
fn locate_resolves_the_eol_sentinel_of_an_exactly_full_wrapped_display_line() {
    // "abcde\n" at wrap width 5 fits exactly — no room left on display line 0 for the
    // sentinel, so `format.rs` wraps it onto display line 1's own column 0.
    // The cursor addressing this same head position must resolve to that
    // same display line/column, not to display line 0's one-past-the-end column (which would
    // land the cursor on the seam past the pane's right edge).
    let rope = Rope::from_str("abcde\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 5 }, &providers, &mut s);

    assert_eq!(
        dlm.locate(co(5)),
        (DisplayLinePos::new(ContentLine::new(0), 1), dc(0))
    );
}

#[test]
fn char_at_nearest_content_stays_off_the_newline_indicator() {
    // Same scenario as the sibling test above, but with the newline
    // indicator (`whitespace-newline`) enabled: `format.rs` pushes it at the
    // same column and char_offset as the EOL sentinel, so a sticky column
    // past the end of the text must still land on the last real character —
    // not the indicator cell, which `Whitespace`'s tab/space-glyph cases make
    // ineligible for a blanket exclusion.
    let rope = Rope::from_str("hi\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut whitespace = ws();
    whitespace.newline = true;
    let mut dlm = DisplayLineMap::new(
        &rope,
        &providers,
        80,
        FormatKey {
            buffer_tag: [0; 3],
            wrap_mode: WrapMode::None,
            tab_width: 4,
            whitespace,
        },
        &mut s,
    );

    assert_eq!(
        dlm.char_at(
            DisplayLinePos::new(ContentLine::new(0), 0),
            dc(99),
            DisplayColTarget::NearestContent
        ),
        co(1),
        "sticky column must land on 'i', not the newline indicator"
    );
}

#[test]
fn char_at_nearest_content_skips_a_trailing_inline_insert() {
    // "hi\n" plus a trailing insert "ZZZ" (an end-of-line diagnostic summary,
    // say) appended after the text. Both the EOL sentinel and the insert's
    // cells sit at the '\n' char offset — a sticky column past all of them
    // must still land on the last *real* character ('i'), not the insert or
    // the newline the way an unfiltered nearest-column search would (it
    // would prefer the insert's own trailing cell, being visually closer).
    let rope = Rope::from_str("hi\n");
    let (providers, _calls) = with_counting_insert(ContentLine::new(0), 2, "ZZZ");
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.char_at(
            DisplayLinePos::new(ContentLine::new(0), 0),
            dc(10),
            DisplayColTarget::NearestContent
        ),
        co(1),
        "sticky column must land on 'i', not the trailing insert or the newline"
    );
}

#[test]
fn char_at_nearest_content_falls_back_to_the_sentinel_on_an_empty_line() {
    // An empty line's only cell *is* the sentinel, so it has to answer.
    let rope = Rope::from_str("\nx\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.char_at(
            DisplayLinePos::new(ContentLine::new(0), 0),
            dc(5),
            DisplayColTarget::NearestContent
        ),
        co(0)
    );
}

#[test]
fn char_at_resolves_a_column_inside_a_wide_cell_differently_per_policy() {
    // "\tx\n" at tab width 4: the tab spans columns 0..3, 'x' sits at column
    // 4. Column 3 is inside the tab's expanse.
    let rope = Rope::from_str("\tx\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.char_at(
            DisplayLinePos::new(ContentLine::new(0), 0),
            dc(3),
            DisplayColTarget::Cell
        ),
        co(0),
        "a click at column 3 hit the tab, so it selects the tab"
    );
    assert_eq!(
        dlm.char_at(
            DisplayLinePos::new(ContentLine::new(0), 0),
            dc(3),
            DisplayColTarget::NearestContent
        ),
        co(1),
        "a sticky column of 3 is nearer 'x' at column 4 than the tab at 0"
    );
}

#[test]
fn char_at_cell_on_the_right_half_of_a_wide_grapheme_selects_the_grapheme() {
    // "中x\n": '中' spans display cols 0-1 (its own cell at col 0, width 2)
    // plus a separate WidthContinuation entry at col 1 (width 0, sharing
    // '中's char_offset). A click at col 1 — the glyph's right half — must
    // resolve to '中' via its own cell's span (0..2), not skip past it to
    // the WidthContinuation entry or fall through to 'x'.
    let rope = Rope::from_str("中x\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.char_at(
            DisplayLinePos::new(ContentLine::new(0), 0),
            dc(1),
            DisplayColTarget::Cell
        ),
        co(0),
        "clicking the wide glyph's right half must select the glyph itself"
    );
}

#[test]
fn char_at_cell_inside_a_placeholder_selects_the_placeholder() {
    // "a\u{200b}b\n": 'a' at col 0, the zero-width space's `<200b>`
    // placeholder spans cols 1-6 (width 6), 'b' at col 7. A click anywhere
    // in the placeholder's span — not just its first cell — must resolve to
    // the character it stands in for, the same as clicking any other
    // multi-column cell.
    let rope = Rope::from_str("a\u{200b}b\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.char_at(
            DisplayLinePos::new(ContentLine::new(0), 0),
            dc(3),
            DisplayColTarget::Cell
        ),
        co(1),
        "a click inside the placeholder's span must select the char it stands in for"
    );
}

#[test]
fn char_at_nearest_content_prefers_real_content_over_a_width_continuation_tie() {
    // "中x\n": '中' is CJK (width 2, columns 0-1); its WidthContinuation cell
    // sits at column 2, sharing '中's char_offset, and 'x' also starts at
    // column 2. A sticky column of 2 ties between the continuation cell and
    // 'x' — the continuation must not win the tie: it would silently answer
    // '中's char_offset instead of 'x's, landing a vertical move one glyph
    // too far left whenever the sticky column matches the cell right after a
    // wide grapheme.
    let rope = Rope::from_str("中x\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.char_at(
            DisplayLinePos::new(ContentLine::new(0), 0),
            dc(2),
            DisplayColTarget::NearestContent
        ),
        co(1),
        "sticky column 2 must land on 'x' (char 1), not '中' via its continuation cell"
    );
}

#[test]
fn char_at_on_a_virtual_display_line_clamps_to_the_lines_own_content() {
    // A virtual display line carries no buffer position, so an address on one resolves
    // against the nearest content display line of the line it is anchored to.
    let rope = Rope::from_str("a\nb\nc\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::Before(ContentLine::new(1)),
        1,
    )));
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::After(ContentLine::new(2)),
        1,
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.char_at(
            DisplayLinePos::new(ContentLine::new(1), 0),
            dc(0),
            DisplayColTarget::Cell
        ),
        co(hume_rope::lines::line_start_char(&rope, RopeyLine::new(1)).index()),
        "the Before display line resolves to line 1's first content display line"
    );
    assert_eq!(
        dlm.char_at(
            DisplayLinePos::new(ContentLine::new(2), 1),
            dc(0),
            DisplayColTarget::Cell
        ),
        co(hume_rope::lines::line_start_char(&rope, RopeyLine::new(2)).index()),
        "the After display line resolves to line 2's last content display line"
    );
}

// ---------------------------------------------------------------------------
// content_display_line_char_bounds()
// ---------------------------------------------------------------------------

#[test]
fn content_display_line_char_bounds_scopes_to_one_wrap_display_line() {
    // "abcdefgh\n" at width 4: display line 0 covers chars 0..4, display line 1 covers 4..8.
    // Row 1 ("efgh") exactly fills the wrap width, so the trailing '\n's own
    // sentinel wraps onto a display line of its own (char 8, the '\n' itself) instead
    // of being folded into display line 1's bounds.
    let rope = Rope::from_str("abcdefgh\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(
        dlm.content_display_line_char_bounds(DisplayLinePos::new(ContentLine::new(0), 0)),
        Some(ex(0, 4))
    );
    assert_eq!(
        dlm.content_display_line_char_bounds(DisplayLinePos::new(ContentLine::new(0), 1)),
        Some(ex(4, 8))
    );
    assert_eq!(
        dlm.content_display_line_char_bounds(DisplayLinePos::new(ContentLine::new(0), 2)),
        Some(ex(8, 9)),
        "the wrapped sentinel display line covers just the '\\n' itself"
    );
}

#[test]
fn content_display_line_char_bounds_rejects_a_virtual_display_line() {
    let rope = Rope::from_str("abcdefgh\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        1,
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(
        dlm.content_display_line_char_bounds(DisplayLinePos::new(ContentLine::new(0), 0)),
        None,
        "display line 0 is the Before display line, not content"
    );
    assert_eq!(
        dlm.content_display_line_char_bounds(DisplayLinePos::new(ContentLine::new(0), 1)),
        Some(ex(0, 4)),
        "display line 1 is the line's first content display line"
    );
}

// ---------------------------------------------------------------------------
// Render accessors
// ---------------------------------------------------------------------------

#[test]
fn render_display_line_yields_a_content_lines_wrap_display_lines() {
    let rope = Rope::from_str("abcdefgh\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    let dl0 = dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));
    assert_eq!(
        dl0.display_line.kind,
        crate::types::DisplayLineKind::LineStart {
            line_idx: RopeyLine::new(0)
        }
    );
    assert_eq!(display_line_text(&dl0), "abcd");

    let dl1 = dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 1));
    assert_eq!(
        dl1.display_line.kind,
        crate::types::DisplayLineKind::Wrap {
            line_idx: RopeyLine::new(0),
            wrap_index: 1,
        }
    );
    assert_eq!(display_line_text(&dl1), "efgh");
}

#[test]
fn render_display_line_segments_a_virtual_lines_text() {
    let rope = Rope::from_str("hi\n");
    let mut providers = ProviderSet::new();
    // Consume id 0 so the emitting provider's real id is 1 — it self-reports
    // 0, which must be overwritten.
    providers.add_decoration_source(Box::new(NoVirtualLines));
    providers.add_decoration_source(Box::new(FixedAnchor::texted(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        1,
        "deleted line",
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    let virtual_line = dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));
    assert_eq!(
        virtual_line.display_line.kind,
        crate::types::DisplayLineKind::Virtual {
            provider_id: 1,
            anchor_line: RopeyLine::new(0),
        },
        "the registry's id replaces the provider's self-reported one"
    );
    assert_eq!(display_line_text(&virtual_line), "deleted line");
}

#[test]
fn render_display_line_expands_a_tab_in_a_virtual_lines_text() {
    // A virtual display line must be tab-aware exactly like a real buffer line — this
    // is what lets `set-virtual-lines!` accept a literal `\t` in `'text`
    // instead of requiring the caller to expand it by hand (previously the
    // git-diff plugin's job, and the source of its column-counting bug).
    let rope = Rope::from_str("hi\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(NoVirtualLines));
    providers.add_decoration_source(Box::new(FixedAnchor::texted(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        1,
        "\tx",
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s); // tab_width == 4, see `map`

    let virtual_line = dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));
    let cells = &virtual_line.graphemes[virtual_line.display_line.graphemes.clone()];
    assert_eq!(cells.len(), 2, "one cell for the tab, one for 'x'");
    assert_eq!(cells[0].display_col, dc(0));
    assert_eq!(
        cells[0].width, 4,
        "tab at display_col 0, tab_width 4 -> full stop"
    );
    assert!(
        matches!(cells[0].content, CellContent::TabFill),
        "a tab renders as TabFill, matching a real buffer line's tab with its indicator off"
    );
    assert_eq!(
        cells[1].display_col,
        dc(4),
        "'x' lands right after the tab stop"
    );
}

#[test]
fn render_display_line_wide_cjk_before_tab_in_a_virtual_lines_text_shifts_the_stop() {
    // A wide CJK grapheme before a tab must shift the tab's stop by its full
    // 2-column width, matching a real buffer line — the exact case
    // `git-diff/render.scm` used to get wrong when it counted one Steel char
    // (not one display column) per preceding character.
    let rope = Rope::from_str("hi\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(NoVirtualLines));
    providers.add_decoration_source(Box::new(FixedAnchor::texted(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        1,
        "\u{6F22}\tx",
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s); // tab_width == 4, see `map`

    let virtual_line = dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));
    let cells = &virtual_line.graphemes[virtual_line.display_line.graphemes.clone()];
    // 漢(w2) + its WidthContinuation, then the tab (tab_advance(2, 4) == 2,
    // so it also occupies 2 columns and gets its own WidthContinuation —
    // same as any width-2 cell, tab or not), then 'x'.
    assert_eq!(cells.len(), 5);
    assert_eq!(cells[0].display_col, dc(0));
    assert_eq!(cells[0].width, 2);
    assert!(matches!(cells[1].content, CellContent::WidthContinuation));
    assert_eq!(
        cells[2].display_col,
        dc(2),
        "tab starts right after the wide char"
    );
    assert_eq!(cells[2].width, 2, "tab_advance(2, 4) == 2");
    assert!(matches!(cells[3].content, CellContent::WidthContinuation));
    assert_eq!(cells[4].display_col, dc(4), "'x' lands at column 4, not 3");
}

#[test]
fn h_window_clips_an_unwrapped_display_lines_graphemes_without_changing_its_display_line_count() {
    // The render path's bound on arbitrarily long unwrapped lines: only the
    // window's columns are emitted, but the line is still one display line.
    let rope = Rope::from_str("abcdefghij\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s).with_h_window(Some(dc(2)..dc(5)));

    assert_eq!(
        dlm.block(ContentLine::new(0)).content,
        1,
        "clipping never adds or drops display lines"
    );
    assert_eq!(
        display_line_text(&dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0))),
        "cde",
        "columns 2..5 of the line, and nothing else"
    );
}

#[test]
fn render_display_line_formats_a_line_once_however_many_display_lines_are_drawn() {
    // Both wrap display lines of one line come from a single format pass.
    let rope = Rope::from_str("abcdef\n");
    let (providers, calls) = with_counting_insert(ContentLine::new(0), 0, "hint");
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 8 }, &providers, &mut s);

    assert_eq!(dlm.block(ContentLine::new(0)).content, 2);
    let after_count = calls.get();
    dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));
    dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 1));
    assert_eq!(
        calls.get(),
        after_count,
        "rendering display lines of an already-counted line must not re-format it"
    );
}

#[test]
fn render_display_line_does_not_reformat_a_line_because_of_its_virtual_lines() {
    // A Before display line is laid out and rendered before its line's content display lines —
    // it must not disturb the already-formatted content display line/grapheme/arena
    // state that follows it in the same block.
    let rope = Rope::from_str("abcdef\n");
    let (mut providers, calls) = with_counting_insert(ContentLine::new(0), 0, "hint");
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        1,
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 8 }, &providers, &mut s);

    assert_eq!(dlm.block(ContentLine::new(0)).content, 2);
    let after_count = calls.get();
    dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0)); // the Before display line
    dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 1)); // content display line 0
    dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 2)); // content display line 1
    assert_eq!(
        calls.get(),
        after_count,
        "rendering a line's Before display line must not force its content display lines to re-format"
    );
}

#[test]
fn render_display_line_yields_correct_content_display_lines_after_a_virtual_display_line() {
    // A virtual display line's layout must not disturb the content display lines that follow
    // it in the same block: they must still come back correct.
    let rope = Rope::from_str("abcdefgh\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::texted(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        1,
        "V",
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(
        display_line_text(&dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0))),
        "V"
    );
    assert_eq!(
        display_line_text(&dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 1))),
        "abcd"
    );
    assert_eq!(
        display_line_text(&dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), 2))),
        "efgh"
    );
}

// ---------------------------------------------------------------------------
// Bounded formatting
// ---------------------------------------------------------------------------

/// One unwrapped line far longer than any query needs to scan. Pure ASCII, so
/// column == char offset and the expected answers are plain arithmetic.
fn long_unwrapped_line() -> Rope {
    Rope::from_str(&("a".repeat(70_000) + "\n"))
}

#[test]
fn locate_formats_only_as_far_as_the_target_offset() {
    let rope = long_unwrapped_line();
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.locate(co(5)).1,
        dc(5),
        "pure ASCII: column equals char offset"
    );

    // Dropping the map releases its borrow of the store, letting the test
    // read what the formatter actually emitted — an oracle over `format.rs`'s
    // output rather than anything `DisplayLineMap` reports about itself.
    drop(dlm);
    assert_eq!(
        stored_graphemes(&s, ContentLine::new(0)),
        6,
        "the target grapheme and the five before it, not all 70k"
    );
}

#[test]
fn char_at_formats_only_as_far_as_the_target_column() {
    let rope = long_unwrapped_line();
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.char_at(
            DisplayLinePos::new(ContentLine::new(0), 0),
            dc(5),
            DisplayColTarget::Cell
        ),
        co(5)
    );

    drop(dlm);
    assert_eq!(
        stored_graphemes(&s, ContentLine::new(0)),
        7,
        "through the first cell past column 5, not all 70k"
    );
}

#[test]
fn a_wider_offset_on_a_cached_line_reformats() {
    // The narrower scan left everything past its target unformatted, so the
    // cached extent must not be treated as answering the wider one.
    let rope = long_unwrapped_line();
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(dlm.locate(co(3)).1, dc(3));
    assert_eq!(dlm.locate(co(50)).1, dc(50), "the second query must rescan");
}

#[test]
fn a_column_query_after_an_offset_query_reformats() {
    // A byte-bounded scan says nothing about how far the columns reached, so
    // the two bound kinds can never satisfy each other.
    let rope = long_unwrapped_line();
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    let (pos, _) = dlm.locate(co(3));
    assert_eq!(dlm.char_at(pos, dc(40), DisplayColTarget::Cell), co(40));
}

#[test]
fn locate_display_line_answers_without_formatting_in_no_wrap() {
    let rope = long_unwrapped_line();
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        2,
    )));
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.locate_display_line(co(5)),
        DisplayLinePos::new(ContentLine::new(0), 2),
        "the line's own display line sits after the two Before display lines above it"
    );

    // The entry exists — `block` built its shape — but carries no format at
    // all, which is exactly the claim: the display line came from the breakdown.
    drop(dlm);
    assert!(
        is_unformatted(&s, ContentLine::new(0)),
        "a no-wrap display line comes from the block breakdown, with no formatting"
    );
}

#[test]
fn locate_display_line_agrees_with_locate_in_both_wrap_modes() {
    // `locate` is the oracle here, which is not circular: the claim *is* that
    // the two agree, and `locate`'s own answers are pinned independently by
    // the `locate_*` tests above.
    //
    // "a\n\nébc\n" covers an empty line, a line with Before display lines above it, a
    // multi-byte grapheme, and the phantom line past the last `\n`.
    let rope = Rope::from_str("a\n\nébc\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(
        VirtualLineAnchor::Before(ContentLine::new(1)),
        2,
    )));

    for wrap in [WrapMode::None, WrapMode::Soft { width: 2 }] {
        let mut s = PaneLineStore::new();
        let mut dlm = map(&rope, wrap, &providers, &mut s);
        // Inclusive upper bound is defensive — the buffer invariant keeps a
        // cursor at `head < len_chars()`.
        for offset in 0..=rope.len_chars() {
            // `locate_display_line` first, so it has to be right without a previous
            // `locate` having warmed the scratch.
            let pos = dlm.locate_display_line(co(offset));
            let via_locate = dlm.locate(co(offset)).0;
            assert_eq!(pos, via_locate, "{wrap:?}, offset {offset}");
        }
    }
}

// ---------------------------------------------------------------------------
// buffer_line_col / char_at_buffer_line_col
// ---------------------------------------------------------------------------

#[test]
fn line_display_col_matches_locate_column_in_no_wrap() {
    // No-wrap: a line is exactly one display line, so the buffer-line-relative
    // column and `locate`'s display-line-relative one must agree everywhere — the invariant
    // `BufferLineCol::as_display_line_unwrapped` relies on to treat the two origins
    // as interchangeable there. `locate` is the oracle, pinned independently
    // by the `locate_*` tests above.
    let rope = Rope::from_str("hello\tworld\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    for offset in 0..=rope.len_chars() {
        let expected = dlm.locate(co(offset)).1;
        assert_eq!(
            dlm.buffer_line_col(co(offset)).get(),
            expected.get(),
            "offset {offset}"
        );
    }
}

#[test]
fn buffer_line_col_accumulates_across_a_wrap_display_line() {
    // "永永永永\n": 4 CJK graphemes, each exactly 2 columns wide regardless of
    // position (unlike a tab), wrapped 2-per-display-line at width 4. The line-relative
    // column is then just 2x the char offset — an oracle independent of the
    // wrap point, which this asserts crosses the display-line boundary (offsets 2, 3).
    let rope = Rope::from_str("永永永永\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    // Sanity: the display-line boundary actually falls where the arithmetic assumes.
    assert_eq!(
        dlm.locate(co(2)).0.slot,
        1,
        "third character starts display line 1"
    );

    for offset in 0..4 {
        assert_eq!(
            dlm.buffer_line_col(co(offset)),
            ldc(offset as u32 * 2),
            "offset {offset}"
        );
    }
}

#[test]
fn line_display_col_excludes_wrap_indent() {
    // "    ab cd\n": 4 leading spaces register one tab-stop of indent
    // (`indent_depth` truncates to whole tab-stops at tab_width 4), so
    // `WrapMode::Indent` opens the continuation display line 4 columns in. Every
    // grapheme here is exactly one cell wide with no tabs past the leading
    // run, so a position's TRUE line-relative column is trivially its own
    // char offset — an oracle independent of both `DisplayLineMap` and exactly where
    // the line wraps.
    let rope = Rope::from_str("    ab cd\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Indent { width: 7 }, &providers, &mut s);

    for offset in 0..rope.len_chars() {
        assert_eq!(
            dlm.buffer_line_col(co(offset)),
            ldc(offset as u32),
            "offset {offset}"
        );
    }

    // Sanity: the line really did wrap, and the last position really is on
    // the indented continuation display line, where the display-line-relative
    // column (`locate`) disagrees with the buffer-line-relative one — proving
    // `buffer_line_col` isn't just forwarding `locate`'s answer verbatim.
    let last = rope.len_chars() - 1;
    let (pos, display_line_col) = dlm.locate(co(last));
    assert!(
        pos.slot > 0,
        "the line must actually wrap for this test to mean anything"
    );
    assert_ne!(
        display_line_col,
        dc(last as u32),
        "display-line-relative column must differ from the buffer-line-relative one on an indented display line"
    );
}

#[test]
fn line_display_col_counts_a_preceding_inline_insert() {
    // "ab\n" with a 2-cell inline insert ("XY", an inlay hint say) spliced in
    // right before 'b': the insert occupies columns 1..3, so 'b's
    // line-relative column is 3, not its char offset (1) — exactly the
    // quantity the rope-only mirror this API replaces (`place_display_column`)
    // could never see, since inline inserts live only in the decoration layer
    // `DisplayLineMap` formats through.
    let rope = Rope::from_str("ab\n");
    let (providers, _calls) = with_counting_insert(ContentLine::new(0), 1, "XY");
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.buffer_line_col(co(0)),
        ldc(0),
        "'a' precedes the insert"
    );
    assert_eq!(
        dlm.buffer_line_col(co(1)),
        ldc(3),
        "'b' is pushed right by the insert's 2 cells"
    );
}

#[test]
fn char_at_line_display_col_round_trips_with_line_display_col() {
    // Up to, not through, the line's own terminating '\n': `NearestContent`
    // deliberately never lands there on a non-empty line (see
    // `char_at_nearest_content_stays_off_the_eol_sentinel`), so round-tripping
    // *that* offset's column intentionally clamps back to 'd' rather than
    // returning 9 — not a round trip to test.
    let rope = Rope::from_str("    ab cd\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::Indent { width: 7 }, &providers, &mut s);

    for offset in 0..rope.len_chars() - 1 {
        let col = dlm.buffer_line_col(co(offset));
        assert_eq!(
            dlm.char_at_buffer_line_col(ContentLine::new(0), col, DisplayColTarget::NearestContent),
            co(offset),
            "offset {offset}, col {col:?}"
        );
    }
}

#[test]
fn char_at_line_display_col_clamps_to_last_char_on_a_shorter_line() {
    // Line 1 ("ab") is shorter than the column target (5) carried over from a
    // longer line — clamps to the last real character rather than landing on
    // the '\n', matching `NearestContent`'s own EOL-exclusion (the "9j onto a
    // shorter line" rule the retired `place_display_column` used to encode
    // by hand).
    let rope = Rope::from_str("hello\nab\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.char_at_buffer_line_col(
            ContentLine::new(1),
            ldc(5),
            DisplayColTarget::NearestContent
        ),
        co(hume_rope::lines::line_start_char(&rope, RopeyLine::new(1)).index() + 1),
        "clamps to 'b', not the '\\n'"
    );
}

#[test]
fn char_at_line_display_col_lands_on_newline_for_an_empty_line() {
    // An empty line's only cell *is* the sentinel, so it has to answer.
    let rope = Rope::from_str("\nx\n");
    let providers = ProviderSet::new();
    let mut s = PaneLineStore::new();
    let mut dlm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        dlm.char_at_buffer_line_col(
            ContentLine::new(0),
            ldc(5),
            DisplayColTarget::NearestContent
        ),
        co(0)
    );
}

#[test]
fn char_at_line_display_col_matches_char_at_in_no_wrap() {
    // No-wrap: buffer-line-relative and display-line-relative columns coincide,
    // so `char_at_buffer_line_col` must agree with `char_at` (display line 0)
    // everywhere — `char_at` is the oracle here, pinned independently by the
    // `char_at_*` tests above. Covers, via that agreement rather than by
    // duplicating hardcoded expectations, the same tab/CJK/width-boundary
    // cases the retired `hume_rope::lines::place_display_column`'s test
    // suite once pinned by hand: a tab or wide grapheme before the target,
    // and the boundary exactly at a line's display width.
    let ropes = [
        Rope::from_str("hello\nworld\n"),
        Rope::from_str("hi\nhello\n"),
        Rope::from_str("\tworld\nhi\n"),
        Rope::from_str("\u{6F22}bc\nhi\n"),
        Rope::from_str("abcd\nabc\n"),
        Rope::from_str("a\n\nb\n"),
    ];
    let providers = ProviderSet::new();

    for rope in &ropes {
        for line in (0..content_line_count(rope).get()).map(ContentLine::new) {
            for target in [DisplayColTarget::Cell, DisplayColTarget::NearestContent] {
                for col in 0..12u32 {
                    let mut s = PaneLineStore::new();
                    let mut dlm = map(rope, WrapMode::None, &providers, &mut s);
                    let expected = dlm.char_at(DisplayLinePos::new(line, 0), dc(col), target);
                    assert_eq!(
                        dlm.char_at_buffer_line_col(line, ldc(col), target),
                        expected,
                        "line {}, col {col}, {target:?}",
                        line.index()
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shared line store
// ---------------------------------------------------------------------------

/// The frame's two passes over one pane — the editor's scroll step and the
/// render pass — each build their own `DisplayLineMap`, but over the same store. Under
/// a wrapping mode each formats the lines it walks, so sharing the store must
/// collapse that to one format per line: `CountingInsert` counts INLINE
/// queries, which happen once per format and nowhere else.
#[test]
fn two_passes_over_one_pane_format_each_line_once() {
    let r = Rope::from_str("alpha\nbravo\ncharlie\ndelta\necho\n");
    let (providers, calls) = with_counting_insert(ContentLine::new(0), 0, "x");
    let mut store = PaneLineStore::new();
    let lines = 0..5;

    // Pass 1 (stands in for the scroll step).
    let mut dlm = map(&r, WrapMode::Soft { width: 80 }, &providers, &mut store);
    for line in lines.clone() {
        dlm.block(ContentLine::new(line));
    }
    drop(dlm);
    let after_first = calls.get();
    assert_eq!(after_first, 5, "the first pass formats each line once");

    // Pass 2 (stands in for the render pass): a cold `DisplayLineMap`, same store.
    let mut dlm = map(&r, WrapMode::Soft { width: 80 }, &providers, &mut store);
    for line in lines {
        dlm.block(ContentLine::new(line));
    }

    assert_eq!(
        calls.get(),
        after_first,
        "the second pass must reuse every line the first formatted"
    );
}

/// A line read back from the store must produce the display line list the formatter
/// would have. The line carries a tab and an inline insert so both the
/// `line_texts` slices (`Grapheme::byte_range`) and the `virtual_texts` arena
/// (`CellContent`'s `(start, len)`) are exercised — those offsets are
/// line-local, which is what makes an entry readable on its own.
#[test]
fn a_stored_format_reproduces_the_display_lines_it_replaced() {
    let r = Rope::from_str("ab\tcdefghij\n");
    let (providers, _) = with_counting_insert(ContentLine::new(0), 4, "HINT");
    let wrap = WrapMode::Soft { width: 6 };

    let display_lines_of =
        |dlm: &mut DisplayLineMap<'_>| -> Vec<(crate::types::DisplayLineKind, String)> {
            let total = dlm.block(ContentLine::new(0)).total();
            (0..total)
                .map(|slot_idx| {
                    let rendered =
                        dlm.render_display_line(DisplayLinePos::new(ContentLine::new(0), slot_idx));
                    (rendered.display_line.kind, display_line_text(&rendered))
                })
                .collect()
        };

    // A store that has never seen the line: the formatter runs.
    let mut fresh = PaneLineStore::new();
    let expected = display_lines_of(&mut map(&r, wrap, &providers, &mut fresh));

    // A store one pass already filled: the second reads it back.
    let mut shared = PaneLineStore::new();
    let mut warm = map(&r, wrap, &providers, &mut shared);
    warm.block(ContentLine::new(0));
    drop(warm);
    let actual = display_lines_of(&mut map(&r, wrap, &providers, &mut shared));

    assert!(expected.len() > 1, "the fixture must actually wrap");
    assert_eq!(actual, expected);
}

/// A horizontally clipped format is not interchangeable with an unclipped
/// one — it *drops* the graphemes left of its window rather than truncating,
/// so it cannot stand in for a scan that kept them. `LineFormat::covers`
/// checks the recorded window against the querying map's own, so the two
/// never meet; this is what stops the frame's two passes sharing a *format*
/// in `WrapMode::None`, where only the render pass clips (they still share
/// the entry's block shape — see `an_h_window_change_keeps_the_block_shape`).
#[test]
fn an_h_window_map_does_not_read_an_unclipped_format() {
    let r = Rope::from_str("alpha bravo charlie delta\n");
    let (providers, calls) = with_counting_insert(ContentLine::new(0), 0, "x");
    let mut store = PaneLineStore::new();

    let mut unclipped = map(&r, WrapMode::None, &providers, &mut store);
    unclipped.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));
    drop(unclipped);
    let after_first = calls.get();
    assert_eq!(after_first, 1, "no-wrap formats on render, not on block");

    let mut clipped =
        map(&r, WrapMode::None, &providers, &mut store).with_h_window(Some(dc(6)..dc(20)));
    clipped.render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));

    assert_eq!(
        calls.get(),
        after_first + 1,
        "a windowed map must format for itself, not read the unclipped entry"
    );
}

/// Block shape (virtual display lines, `before`/`after`) does not depend on the
/// horizontal window a `WrapMode::None` render clips to — only the formatted
/// display lines do. An `h_window` change must not force `block_entry` to re-query
/// providers for a line the store already has.
#[test]
fn an_h_window_change_keeps_the_block_shape() {
    let r = Rope::from_str("alpha\n");
    let (providers, calls) = with_counting_anchor();
    let mut store = PaneLineStore::new();

    map(&r, WrapMode::None, &providers, &mut store).block(ContentLine::new(0));
    let after_first = calls.get();

    map(&r, WrapMode::None, &providers, &mut store)
        .with_h_window(Some(dc(0)..dc(5)))
        .block(ContentLine::new(0));

    assert_eq!(
        calls.get(),
        after_first,
        "an h_window change must reuse the entry's block shape, not rebuild it"
    );
}

/// The scope key covers every formatting input, so a buffer that changed
/// under the same pane invalidates what was stored for it — those entries
/// describe a format the new inputs would not produce.
#[test]
fn a_changed_key_drops_the_scope() {
    let r = Rope::from_str("alpha\nbravo\n");
    let (providers, calls) = with_counting_insert(ContentLine::new(0), 0, "x");
    let mut store = PaneLineStore::new();
    let wrap = WrapMode::Soft { width: 80 };

    DisplayLineMap::new(
        &r,
        &providers,
        80,
        FormatKey {
            buffer_tag: [1; 3],
            wrap_mode: wrap,
            tab_width: 4,
            whitespace: ws(),
        },
        &mut store,
    )
    .block(ContentLine::new(0));
    let after_first = calls.get();

    // Same store, same line, different buffer tag.
    DisplayLineMap::new(
        &r,
        &providers,
        80,
        FormatKey {
            buffer_tag: [2; 3],
            wrap_mode: wrap,
            tab_width: 4,
            whitespace: ws(),
        },
        &mut store,
    )
    .block(ContentLine::new(0));

    assert_eq!(
        calls.get(),
        after_first + 1,
        "entries built under the old key must not be served under the new one"
    );
}

/// Entries must not outlive the frame that produced them: the per-pane
/// inline-insert mirrors are rebuilt each frame filtered to that frame's
/// viewport, without bumping anything the key can see. `rewind` is what
/// `EngineView::begin_frame` calls on every pane's store to enforce that.
#[test]
fn rewind_drops_the_previous_frames_entries() {
    let r = Rope::from_str("alpha\nbravo\n");
    let (providers, calls) = with_counting_insert(ContentLine::new(0), 0, "x");
    let mut store = PaneLineStore::new();
    let wrap = WrapMode::Soft { width: 80 };

    map(&r, wrap, &providers, &mut store).block(ContentLine::new(0));
    let after_first = calls.get();

    store.rewind();

    map(&r, wrap, &providers, &mut store).block(ContentLine::new(0));

    assert_eq!(
        calls.get(),
        after_first + 1,
        "a new frame must re-format rather than read last frame's entry"
    );
}

/// A line whose *block shape* is all anyone asked for must not pay for format
/// buffers it never fills. Under `WrapMode::None` — the default — `content_display_lines`
/// answers 1 without running the formatter, so a walk that only asks for shape
/// (a half-page motion stepping display line by display line, once per selection) touches many
/// lines and formats none of them. Their entries have to cost bookkeeping
/// rather than a buffer apiece.
#[test]
fn a_shape_only_entry_allocates_no_format_buffers() {
    let r = Rope::from_str("hello world\nsecond line\n");
    let providers = ProviderSet::new();
    let mut store = PaneLineStore::new();

    let breakdown = DisplayLineMap::new(
        &r,
        &providers,
        80,
        FormatKey {
            buffer_tag: [1; 3],
            wrap_mode: WrapMode::None,
            tab_width: 4,
            whitespace: ws(),
        },
        &mut store,
    )
    .block(ContentLine::new(0));
    assert_eq!(
        breakdown.content, 1,
        "sanity: no-wrap block shape must resolve without formatting"
    );

    let format = &store.entry(store.find(ContentLine::new(0)).unwrap()).format;
    assert_eq!(
        format.graphemes.capacity(),
        0,
        "a walked-but-unformatted entry must hold no grapheme buffer"
    );
    assert_eq!(
        format.line_texts.capacity(),
        0,
        "a walked-but-unformatted entry must hold no line-text buffer"
    );
}

/// A line pathologically wider than any ordinary source line (a minified-JS
/// file's single line, megabytes wide) must not pin its whole grapheme/text
/// capacity once the frame that formatted it has passed — that would reverse
/// the free list's own memory bound.
///
/// The frame boundary is the only moment that can be relied on to give it
/// back. Reusing the slot cannot: a slot is rebound only by a later frame
/// that walks at least that many lines, and a pane showing one wide line and
/// then a handful of short ones never walks far enough to reach it again.
#[test]
fn rewind_shrinks_an_oversized_entry() {
    let (before, after) = grapheme_capacity_across_rewind(50_000);
    assert!(
        before > 50_000,
        "sanity: formatting the huge line must have grown its capacity"
    );
    assert!(
        after < before,
        "the frame boundary must hand back a pathologically grown entry \
         rather than wait for a later frame to rebind its slot"
    );
}

/// Below the shrink ceiling, an entry keeps its capacity across the frame
/// boundary — the free list's whole point, which a naive "always shrink on
/// rewind" implementation would defeat for every ordinary line.
#[test]
fn rewind_does_not_shrink_an_ordinary_entry() {
    let (before, after) = grapheme_capacity_across_rewind(2_000);
    assert_eq!(
        after, before,
        "an ordinary-sized line's capacity must survive the frame boundary"
    );
}

/// Format a `width`-grapheme line into a store's single entry, then rewind
/// the store, reporting that entry's grapheme capacity on each side of the
/// rewind.
///
/// Reads the entry back by slot rather than by line: `rewind` clears the line
/// index, and a slot nothing later rebinds is exactly the case this pins.
fn grapheme_capacity_across_rewind(width: usize) -> (usize, usize) {
    let r = Rope::from_str(&format!("{}\n", "x".repeat(width)));
    let providers = ProviderSet::new();
    let mut store = PaneLineStore::new();

    DisplayLineMap::new(
        &r,
        &providers,
        80,
        FormatKey {
            buffer_tag: [1; 3],
            wrap_mode: WrapMode::None,
            tab_width: 4,
            whitespace: ws(),
        },
        &mut store,
    )
    .render_display_line(DisplayLinePos::new(ContentLine::new(0), 0));
    let before = store.entry(0).format.graphemes.capacity();

    store.rewind();

    (before, store.entry(0).format.graphemes.capacity())
}
