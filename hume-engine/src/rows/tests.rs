use std::cell::Cell;
use std::rc::Rc;

use ropey::Rope;

use super::*;
use crate::providers::DecorationSource;
use crate::types::ScopeId;

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
    scratch: &'a mut FormatScratch,
) -> RowMap<'a> {
    RowMap::new(rope, wrap, 4, ws(), providers, 80, scratch)
}

/// Emits `count` identical rows at one fixed anchor. Self-reports
/// `provider_id: 0` so the id-stamping test has something wrong to correct.
struct FixedAnchor {
    anchor: VirtualLineAnchor,
    count: usize,
    text: &'static str,
}

impl FixedAnchor {
    fn new(anchor: VirtualLineAnchor, count: usize) -> Self {
        Self {
            anchor,
            count,
            text: "V",
        }
    }
}

impl DecorationSource for FixedAnchor {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(&self, line_idx: usize, out: &mut Vec<Decoration>) {
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
struct NoRows;

impl DecorationSource for NoRows {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::VIRTUAL_LINE
    }
    fn decorations_for_line(&self, _line_idx: usize, _out: &mut Vec<Decoration>) {}
}

/// A LINE_BG-kind source that counts every `decorations_for_line` call —
/// used to prove the layout stage (`block`/`format_line`) never queries a
/// kind it has no use for, even when the same line is both counted and
/// formatted.
struct CountingLineBg(Rc<Cell<usize>>);

impl DecorationSource for CountingLineBg {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::LINE_BG
    }
    fn decorations_for_line(&self, _line_idx: usize, _out: &mut Vec<Decoration>) {
        self.0.set(self.0.get() + 1);
    }
}

/// One inline insert on `line`, counting how often it is queried — the only
/// observable proxy for "did the map run the formatter".
struct CountingInsert {
    line: usize,
    byte_offset: usize,
    text: &'static str,
    calls: Rc<Cell<usize>>,
}

impl DecorationSource for CountingInsert {
    fn kinds(&self) -> DecorationKinds {
        DecorationKinds::INLINE
    }
    fn decorations_for_line(&self, line_idx: usize, out: &mut Vec<Decoration>) {
        self.calls.set(self.calls.get() + 1);
        if line_idx == self.line {
            out.push(Decoration::Inline(InlineInsert {
                byte_offset: self.byte_offset,
                text: self.text.to_string(),
                scope: ScopeId(0),
            }));
        }
    }
}

fn with_counting_insert(
    line: usize,
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

/// Reconstruct the text a rendered row puts on screen, cell by cell. Derived
/// from the `Grapheme`/`CellContent` contract rather than from anything
/// `RowMap` computed, so it is an independent check of the render accessors.
fn row_text(r: &RenderRow<'_>) -> String {
    r.graphemes[r.row.graphemes.clone()]
        .iter()
        .filter_map(|g| match g.content {
            CellContent::Virtual { start, len }
            | CellContent::Indicator { start, len }
            | CellContent::Placeholder { start, len } => {
                let start = start as usize;
                Some(r.virtual_texts[start..start + len as usize].to_string())
            }
            CellContent::Grapheme => Some(r.line_text[g.byte_range.clone()].to_string()),
            CellContent::WidthContinuation | CellContent::Empty => None,
        })
        .collect()
}

/// Walk the whole document forward from its first row.
fn walk_forward(rm: &mut RowMap<'_>) -> Vec<RowPos> {
    let mut rows = vec![RowPos::default()];
    while let Some(next) = rm.next(*rows.last().expect("seeded above")) {
        rows.push(next);
    }
    rows
}

// ---------------------------------------------------------------------------
// block()
// ---------------------------------------------------------------------------

#[test]
fn block_without_providers_is_content_only() {
    let rope = Rope::from_str("hello\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        rm.block(0),
        RowsBreakdown {
            before: 0,
            content: 1,
            after: 0,
        }
    );
}

#[test]
fn block_counts_one_content_row_per_wrap_row() {
    // "abcdefgh" is 8 columns wrapped at 4 → "abcd" / "efgh" = 2 rows, and
    // "efgh" exactly fills its row, so the trailing '\n's own sentinel wraps
    // onto a third row rather than landing past the pane's edge.
    let rope = Rope::from_str("abcdefgh\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(rm.block(0).content, 3);
}

#[test]
fn block_counts_before_and_after_virtual_rows() {
    // Line 5 of "a\nb\nc\nd\ne\nf\n" gets 2 Before rows and 1 After row from
    // two separate providers, on top of its own single unwrapped content row.
    let rope = Rope::from_str("a\nb\nc\nd\ne\nf\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(5), 2)));
    providers.add_decoration_source(Box::new(FixedAnchor::new(VirtualLineAnchor::After(5), 1)));
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    let block = rm.block(5);
    assert_eq!(
        block,
        RowsBreakdown {
            before: 2,
            content: 1,
            after: 1,
        }
    );
    assert_eq!(block.total(), 4);
}

#[test]
fn block_ignores_virtual_rows_anchored_to_other_lines() {
    let rope = Rope::from_str("a\nb\nc\nd\ne\nf\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(2), 3)));
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(rm.block(5).before, 0);
    assert_eq!(rm.block(5).after, 0);
}

#[test]
fn layout_stage_never_queries_a_paint_only_kind() {
    // block() drives both the VIRTUAL_LINE query and, under wrapping,
    // format_line()'s INLINE query — a LINE_BG-kind source (paint-only)
    // must be invisible to both.
    let calls = Rc::new(Cell::new(0));
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(CountingLineBg(Rc::clone(&calls))));
    let rope = Rope::from_str("abcdef\n");
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    rm.block(0);
    assert_eq!(
        calls.get(),
        0,
        "block() must never query a LINE_BG-only source"
    );
}

#[test]
fn block_counts_inline_inserts_toward_wrapping() {
    // The bug this fixes: an inlay hint participates in wrapping, so a line
    // that fits without it can need two rows with it, and row counting that
    // ignores inserts disagrees with what the renderer emits.
    //
    // "abcdef" is 6 columns; wrapped at 8 that is one row. A 4-column insert
    // at the head of the line makes 10 columns, which is two.
    let rope = Rope::from_str("abcdef\n");
    let wrap = WrapMode::Soft { width: 8 };

    let bare = ProviderSet::new();
    let mut s = FormatScratch::new();
    assert_eq!(
        map(&rope, wrap, &bare, &mut s).block(0).content,
        1,
        "6 columns fit on one row at width 8"
    );

    let (providers, _calls) = with_counting_insert(0, 0, "hint");
    let mut s = FormatScratch::new();
    assert_eq!(
        map(&rope, wrap, &providers, &mut s).block(0).content,
        2,
        "4 columns of inlay hint push the line's 6 columns past width 8"
    );
}

#[test]
fn no_wrap_block_counts_without_running_the_formatter() {
    // `WrapMode::None` is always one content row, so counting must not format
    // — that is what keeps a row query O(1) instead of O(line length) on a
    // minified line megabytes wide. Querying decorations is what formatting
    // does first, so a zero call count is the observable proxy.
    let rope = Rope::from_str("abcdef\n");
    let (providers, calls) = with_counting_insert(0, 0, "hint");
    let mut s = FormatScratch::new();

    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);
    assert_eq!(rm.block(0).content, 1);
    assert_eq!(calls.get(), 0, "no-wrap counting must not format");

    // The same query while wrapping has to format, because the row count
    // genuinely depends on the content.
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 8 }, &providers, &mut s);
    rm.block(0);
    assert!(calls.get() > 0, "wrapping must format to count rows");
}

// ---------------------------------------------------------------------------
// slot() / clamp() / last_line()
// ---------------------------------------------------------------------------

/// Line 0 wrapped into 2 content rows, with 2 Before rows and 1 After row:
/// a 5-row block whose every row slot is hand-known.
fn mixed_block_providers() -> ProviderSet {
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(0), 2)));
    providers.add_decoration_source(Box::new(FixedAnchor::new(VirtualLineAnchor::After(0), 1)));
    providers
}

#[test]
fn slot_classifies_every_row_of_a_mixed_block() {
    // "abcdefgh\n" at width 4 supplies 3 content rows, not 2: "efgh" exactly
    // fills the wrap width, so the trailing '\n's own sentinel wraps onto a
    // row of its own (see `content_row_char_bounds_scopes_to_one_wrap_row`).
    let rope = Rope::from_str("abcdefgh\n");
    let providers = mixed_block_providers();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(rm.block(0).total(), 6);
    let slots: Vec<BlockSlot> = (0..6).map(|row| rm.slot(RowPos::new(0, row))).collect();
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
fn clamp_pulls_line_and_row_into_the_document() {
    let rope = Rope::from_str("abcdefgh\n");
    let providers = mixed_block_providers();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(
        rm.clamp(RowPos::new(0, 99)),
        RowPos::new(0, 5),
        "row clamps to the block's last row"
    );
    assert_eq!(
        rm.clamp(RowPos::new(99, 99)),
        RowPos::new(0, 5),
        "line clamps to the last real line, then row to its block"
    );
    assert_eq!(
        rm.clamp(RowPos::new(0, 2)),
        RowPos::new(0, 2),
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
    let mut s = FormatScratch::new();
    let rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(rm.last_line(), 2);
}

#[test]
fn clamp_reaches_the_documents_very_last_row() {
    // "a\nb\nc\n" has 3 real lines; line 2 carries 1 After row, so the
    // document's last row is that virtual row at (2, 1). `clamp` is the
    // documented way to reach it (RowMap has no dedicated accessor).
    let rope = Rope::from_str("a\nb\nc\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(VirtualLineAnchor::After(2), 1)));
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        rm.clamp(RowPos::new(usize::MAX, usize::MAX)),
        RowPos::new(2, 1)
    );
}

// ---------------------------------------------------------------------------
// Stepping
// ---------------------------------------------------------------------------

/// "a\nb\nc\n" with 2 Before rows on line 1 and 1 After row on line 2. Hand-
/// written row list, in document order:
///
/// ```text
///   (0,0)  line 0 content
///   (1,0)  line 1 Before #0
///   (1,1)  line 1 Before #1
///   (1,2)  line 1 content
///   (2,0)  line 2 content
///   (2,1)  line 2 After #0
/// ```
fn three_line_doc() -> (Rope, ProviderSet, Vec<RowPos>) {
    let rope = Rope::from_str("a\nb\nc\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(1), 2)));
    providers.add_decoration_source(Box::new(FixedAnchor::new(VirtualLineAnchor::After(2), 1)));
    let expected = vec![
        RowPos::new(0, 0),
        RowPos::new(1, 0),
        RowPos::new(1, 1),
        RowPos::new(1, 2),
        RowPos::new(2, 0),
        RowPos::new(2, 1),
    ];
    (rope, providers, expected)
}

#[test]
fn next_walks_the_documents_rows_in_order() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(walk_forward(&mut rm), expected);
}

#[test]
fn prev_is_the_inverse_of_next() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    let mut backward = vec![*expected.last().expect("non-empty")];
    while let Some(prev) = rm.prev(*backward.last().expect("seeded above")) {
        backward.push(prev);
    }
    backward.reverse();
    assert_eq!(backward, expected);
}

#[test]
fn next_and_prev_stop_exactly_at_the_documents_edges() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    let first = expected[0];
    let last = *expected.last().expect("non-empty");
    assert_eq!(
        rm.prev(first),
        None,
        "no row precedes the document's first row"
    );
    assert_eq!(
        rm.next(last),
        None,
        "no row follows the document's last row"
    );
}

#[test]
fn advance_matches_repeated_stepping_and_saturates_at_both_ends() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    for (i, &from) in expected.iter().enumerate() {
        for (j, &to) in expected.iter().enumerate() {
            let delta = j as isize - i as isize;
            assert_eq!(
                rm.advance(from, delta),
                to,
                "advance({from:?}, {delta}) should reach {to:?}"
            );
        }
    }

    let first = expected[0];
    let last = *expected.last().expect("non-empty");
    assert_eq!(rm.advance(first, -10), first, "saturates at the first row");
    assert_eq!(rm.advance(last, 10), last, "saturates at the last row");
}

#[test]
fn advance_counted_reports_how_far_it_actually_stepped() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    // In bounds: the count matches the requested delta exactly.
    for (i, &from) in expected.iter().enumerate() {
        for (j, &to) in expected.iter().enumerate() {
            let delta = j as isize - i as isize;
            let (pos, taken) = rm.advance_counted(from, delta);
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
        rm.advance_counted(first, -10),
        (first, 0),
        "no rows exist before the first row"
    );
    assert_eq!(
        rm.advance_counted(last, 10),
        (last, 0),
        "no rows exist past the last row"
    );

    // The documented "count is also the distance back" property
    // `scroll::scroll_back_from` relies on: having stepped `n` rows backward
    // from `pos`, `distance` from the landing spot back to `pos` is that
    // same `n`.
    let pos = expected[4];
    let (landed, taken) = rm.advance_counted(pos, -3);
    assert_eq!(rm.distance(landed, pos, expected.len()), Some(taken));
}

#[test]
fn distance_counts_rows_forward_and_rejects_backward_or_distant() {
    let (rope, providers, expected) = three_line_doc();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    for (i, &from) in expected.iter().enumerate() {
        for (j, &to) in expected.iter().enumerate().skip(i) {
            assert_eq!(
                rm.distance(from, to, 16),
                Some(j - i),
                "distance({from:?} → {to:?})"
            );
        }
    }

    assert_eq!(
        rm.distance(expected[3], expected[1], 16),
        None,
        "a row behind the start is not reachable forward"
    );
    assert_eq!(
        rm.distance(expected[0], expected[5], 2),
        None,
        "5 rows away is beyond a cap of 2"
    );
    assert_eq!(
        rm.distance(expected[0], expected[2], 2),
        Some(2),
        "exactly at the cap still resolves"
    );
}

#[test]
fn fits_in_counts_virtual_rows_toward_the_height() {
    // The hand-written list above is 6 rows: 3 content + 3 virtual.
    let (rope, providers, expected) = three_line_doc();
    assert_eq!(expected.len(), 6);
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert!(rm.fits_in(6), "6 rows fit in 6");
    assert!(!rm.fits_in(5), "6 rows do not fit in 5");
    assert!(
        !rm.fits_in(3),
        "counting content rows alone would wrongly fit 3 lines in 3 rows"
    );
}

#[test]
fn fits_in_zero_height_never_fits() {
    // Every document has at least one row (even a single empty line), so a
    // zero-height viewport can never fit it — regardless of how short the
    // document is.
    let rope = Rope::from_str("x\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert!(!rm.fits_in(0));
}

#[test]
fn degenerate_single_empty_line_document() {
    // A bare "\n" is the smallest possible buffer under the invariant: one
    // real (empty) line plus the structural trailing newline.
    let rope = Rope::from_str("\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(rm.last_line(), 0);
    assert_eq!(
        rm.clamp(RowPos::new(99, 99)),
        RowPos::new(0, 0),
        "the only row in a one-line document is (0, 0)"
    );
    assert!(
        rm.fits_in(1),
        "the document's single row fits a height of 1"
    );
    assert!(!rm.fits_in(0));
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "content_width >= 1")]
fn new_panics_on_zero_content_width() {
    let rope = Rope::from_str("x\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    RowMap::new(&rope, WrapMode::None, 4, ws(), &providers, 0, &mut s);
}

#[test]
fn wrap_width_one_emits_one_grapheme_per_row_without_hanging() {
    // One grapheme per row at wrap width 1, plus a 5th row for the trailing
    // '\n's own sentinel: every row at width 1 exactly fills, so the
    // sentinel always wraps onto a row of its own here.
    let rope = Rope::from_str("abcd\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = RowMap::new(
        &rope,
        WrapMode::Soft { width: 1 },
        4,
        ws(),
        &providers,
        80,
        &mut s,
    );

    let breakdown = rm.block(0);
    assert_eq!(breakdown.content, 5);
}

// ---------------------------------------------------------------------------
// locate() / char_at()
// ---------------------------------------------------------------------------

#[test]
fn locate_returns_the_wrap_row_and_column_of_a_char() {
    // "abcdefgh\n" at width 4: row 0 holds chars 0..3, row 1 holds 4..7.
    let rope = Rope::from_str("abcdefgh\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(
        rm.locate(0),
        (RowPos::new(0, 0), 0),
        "'a' — row 0, column 0"
    );
    assert_eq!(
        rm.locate(2),
        (RowPos::new(0, 0), 2),
        "'c' — row 0, column 2"
    );
    assert_eq!(
        rm.locate(4),
        (RowPos::new(0, 1), 0),
        "'e' — row 1, column 0"
    );
    assert_eq!(
        rm.locate(5),
        (RowPos::new(0, 1), 1),
        "'f' — row 1, column 1"
    );
}

#[test]
fn locate_offsets_the_row_by_the_lines_before_block() {
    // Same line, now with 2 Before rows above it: 'f' keeps its column but
    // its block row shifts from 1 to 3.
    let rope = Rope::from_str("abcdefgh\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(0), 2)));
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(rm.locate(5), (RowPos::new(0, 3), 1));
}

#[test]
fn locate_skips_a_mid_line_inline_insert_sharing_the_real_graphemes_offset() {
    // "ab\n", no wrap: an inline insert "XY" (an inlay hint, say) spliced in
    // right before 'b' shares 'b's char_offset (1). `locate` must resolve to
    // the real grapheme's column — 'a' at 0, the insert's own two cells at 1
    // and 2, 'b' at 3 — not the insert's column, matching what
    // `style::resolve_grapheme_display_col` already guarantees for selection styling.
    let rope = Rope::from_str("ab\n");
    let (providers, _calls) = with_counting_insert(0, 1, "XY");
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(rm.locate(1), (RowPos::new(0, 0), 3));
}

#[test]
fn char_at_cell_lands_on_the_eol_sentinel_past_the_text() {
    // "hi\n": h at column 0, i at column 1, and the end-of-line sentinel at
    // column 2 standing for the '\n' (char 2) — a real cursor position in
    // HUME's inclusive selection model, so a click out to the right lands
    // there rather than back on 'i'.
    let rope = Rope::from_str("hi\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(rm.char_at(RowPos::new(0, 0), 99, DisplayColTarget::Cell), 2);
}

#[test]
fn char_at_nearest_content_stays_off_the_eol_sentinel() {
    // The same row asked the other question: a sticky column past the end of
    // the text resolves to the last real character, never the '\n'.
    let rope = Rope::from_str("hi\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        rm.char_at(RowPos::new(0, 0), 99, DisplayColTarget::NearestContent),
        1
    );
}

#[test]
fn char_at_nearest_content_stays_off_the_newline_indicator() {
    // Same scenario as the sibling test above, but with the newline
    // indicator (`whitespace-newline`) enabled: `format.rs` pushes it at the
    // same column and char_offset as the EOL sentinel, so a sticky column
    // past the end of the text must still land on the last real character —
    // not the indicator cell, which `Indicator`'s tab/space-glyph cases make
    // ineligible for a blanket exclusion.
    let rope = Rope::from_str("hi\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut whitespace = ws();
    whitespace.newline = true;
    let mut rm = RowMap::new(&rope, WrapMode::None, 4, whitespace, &providers, 80, &mut s);

    assert_eq!(
        rm.char_at(RowPos::new(0, 0), 99, DisplayColTarget::NearestContent),
        1,
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
    let (providers, _calls) = with_counting_insert(0, 2, "ZZZ");
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        rm.char_at(RowPos::new(0, 0), 10, DisplayColTarget::NearestContent),
        1,
        "sticky column must land on 'i', not the trailing insert or the newline"
    );
}

#[test]
fn char_at_nearest_content_falls_back_to_the_sentinel_on_an_empty_line() {
    // An empty line's only cell *is* the sentinel, so it has to answer.
    let rope = Rope::from_str("\nx\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        rm.char_at(RowPos::new(0, 0), 5, DisplayColTarget::NearestContent),
        0
    );
}

#[test]
fn char_at_resolves_a_column_inside_a_wide_cell_differently_per_policy() {
    // "\tx\n" at tab width 4: the tab spans columns 0..3, 'x' sits at column
    // 4. Column 3 is inside the tab's expanse.
    let rope = Rope::from_str("\tx\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        rm.char_at(RowPos::new(0, 0), 3, DisplayColTarget::Cell),
        0,
        "a click at column 3 hit the tab, so it selects the tab"
    );
    assert_eq!(
        rm.char_at(RowPos::new(0, 0), 3, DisplayColTarget::NearestContent),
        1,
        "a sticky column of 3 is nearer 'x' at column 4 than the tab at 0"
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
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        rm.char_at(RowPos::new(0, 0), 2, DisplayColTarget::NearestContent),
        1,
        "sticky column 2 must land on 'x' (char 1), not '中' via its continuation cell"
    );
}

#[test]
fn char_at_on_a_virtual_row_clamps_to_the_lines_own_content() {
    // A virtual row carries no buffer position, so an address on one resolves
    // against the nearest content row of the line it is anchored to.
    let rope = Rope::from_str("a\nb\nc\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(1), 1)));
    providers.add_decoration_source(Box::new(FixedAnchor::new(VirtualLineAnchor::After(2), 1)));
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        rm.char_at(RowPos::new(1, 0), 0, DisplayColTarget::Cell),
        rope.line_to_char(1),
        "the Before row resolves to line 1's first content row"
    );
    assert_eq!(
        rm.char_at(RowPos::new(2, 1), 0, DisplayColTarget::Cell),
        rope.line_to_char(2),
        "the After row resolves to line 2's last content row"
    );
}

// ---------------------------------------------------------------------------
// content_row_char_bounds()
// ---------------------------------------------------------------------------

#[test]
fn content_row_char_bounds_scopes_to_one_wrap_row() {
    // "abcdefgh\n" at width 4: row 0 covers chars 0..4, row 1 covers 4..8.
    // Row 1 ("efgh") exactly fills the wrap width, so the trailing '\n's own
    // sentinel wraps onto a row of its own (char 8, the '\n' itself) instead
    // of being folded into row 1's bounds.
    let rope = Rope::from_str("abcdefgh\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(rm.content_row_char_bounds(RowPos::new(0, 0)), Some((0, 4)));
    assert_eq!(rm.content_row_char_bounds(RowPos::new(0, 1)), Some((4, 8)));
    assert_eq!(
        rm.content_row_char_bounds(RowPos::new(0, 2)),
        Some((8, 9)),
        "the wrapped sentinel row covers just the '\\n' itself"
    );
}

#[test]
fn content_row_char_bounds_rejects_a_virtual_row() {
    let rope = Rope::from_str("abcdefgh\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(0), 1)));
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(
        rm.content_row_char_bounds(RowPos::new(0, 0)),
        None,
        "row 0 is the Before row, not content"
    );
    assert_eq!(
        rm.content_row_char_bounds(RowPos::new(0, 1)),
        Some((0, 4)),
        "row 1 is the line's first content row"
    );
}

// ---------------------------------------------------------------------------
// Render accessors
// ---------------------------------------------------------------------------

#[test]
fn render_row_yields_a_content_lines_wrap_rows() {
    let rope = Rope::from_str("abcdefgh\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    let row0 = rm.render_row(RowPos::new(0, 0));
    assert_eq!(
        row0.row.kind,
        crate::types::RowKind::LineStart { line_idx: 0 }
    );
    assert_eq!(row_text(&row0), "abcd");

    let row1 = rm.render_row(RowPos::new(0, 1));
    assert_eq!(
        row1.row.kind,
        crate::types::RowKind::Wrap {
            line_idx: 0,
            wrap_row: 1,
        }
    );
    assert_eq!(row_text(&row1), "efgh");
}

#[test]
fn render_row_segments_a_virtual_rows_text() {
    let rope = Rope::from_str("hi\n");
    let mut providers = ProviderSet::new();
    // Consume id 0 so the emitting provider's real id is 1 — it self-reports
    // 0, which must be overwritten.
    providers.add_decoration_source(Box::new(NoRows));
    providers.add_decoration_source(Box::new(FixedAnchor {
        anchor: VirtualLineAnchor::Before(0),
        count: 1,
        text: "deleted line",
    }));
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    let virtual_row = rm.render_row(RowPos::new(0, 0));
    assert_eq!(
        virtual_row.row.kind,
        crate::types::RowKind::Virtual {
            provider_id: 1,
            anchor_line: 0,
        },
        "the registry's id replaces the provider's self-reported one"
    );
    assert_eq!(row_text(&virtual_row), "deleted line");
}

#[test]
fn render_row_expands_a_tab_in_a_virtual_lines_text() {
    // A virtual row must be tab-aware exactly like a real buffer line — this
    // is what lets `set-virtual-lines!` accept a literal `\t` in `'text`
    // instead of requiring the caller to expand it by hand (previously the
    // git-diff plugin's job, and the source of its column-counting bug).
    let rope = Rope::from_str("hi\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(NoRows));
    providers.add_decoration_source(Box::new(FixedAnchor {
        anchor: VirtualLineAnchor::Before(0),
        count: 1,
        text: "\tx",
    }));
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s); // tab_width == 4, see `map`

    let virtual_row = rm.render_row(RowPos::new(0, 0));
    let cells = &virtual_row.graphemes[virtual_row.row.graphemes.clone()];
    assert_eq!(cells.len(), 2, "one cell for the tab, one for 'x'");
    assert_eq!(cells[0].display_col, 0);
    assert_eq!(
        cells[0].width, 4,
        "tab at display_col 0, tab_width 4 -> full stop"
    );
    assert!(
        matches!(cells[0].content, CellContent::Indicator { .. }),
        "a tab renders as a space-filled Indicator, matching a real buffer line's tab with its indicator off"
    );
    assert_eq!(
        cells[1].display_col, 4,
        "'x' lands right after the tab stop"
    );
}

#[test]
fn render_row_wide_cjk_before_tab_in_a_virtual_lines_text_shifts_the_stop() {
    // A wide CJK grapheme before a tab must shift the tab's stop by its full
    // 2-column width, matching a real buffer line — the exact case
    // `git-diff/render.scm` used to get wrong when it counted one Steel char
    // (not one display column) per preceding character.
    let rope = Rope::from_str("hi\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(NoRows));
    providers.add_decoration_source(Box::new(FixedAnchor {
        anchor: VirtualLineAnchor::Before(0),
        count: 1,
        text: "\u{6F22}\tx",
    }));
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s); // tab_width == 4, see `map`

    let virtual_row = rm.render_row(RowPos::new(0, 0));
    let cells = &virtual_row.graphemes[virtual_row.row.graphemes.clone()];
    // 漢(w2) + its WidthContinuation, then the tab (tab_advance(2, 4) == 2,
    // so it also occupies 2 columns and gets its own WidthContinuation —
    // same as any width-2 cell, tab or not), then 'x'.
    assert_eq!(cells.len(), 5);
    assert_eq!(cells[0].display_col, 0);
    assert_eq!(cells[0].width, 2);
    assert!(matches!(cells[1].content, CellContent::WidthContinuation));
    assert_eq!(
        cells[2].display_col, 2,
        "tab starts right after the wide char"
    );
    assert_eq!(cells[2].width, 2, "tab_advance(2, 4) == 2");
    assert!(matches!(cells[3].content, CellContent::WidthContinuation));
    assert_eq!(cells[4].display_col, 4, "'x' lands at column 4, not 3");
}

#[test]
fn h_window_clips_an_unwrapped_rows_graphemes_without_changing_its_row_count() {
    // The render path's bound on arbitrarily long unwrapped lines: only the
    // window's columns are emitted, but the line is still one row.
    let rope = Rope::from_str("abcdefghij\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s).with_h_window(Some(2..5));

    assert_eq!(rm.block(0).content, 1, "clipping never adds or drops rows");
    assert_eq!(
        row_text(&rm.render_row(RowPos::new(0, 0))),
        "cde",
        "columns 2..5 of the line, and nothing else"
    );
}

#[test]
fn render_row_formats_a_line_once_however_many_rows_are_drawn() {
    // Both wrap rows of one line come from a single format pass.
    let rope = Rope::from_str("abcdef\n");
    let (providers, calls) = with_counting_insert(0, 0, "hint");
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 8 }, &providers, &mut s);

    assert_eq!(rm.block(0).content, 2);
    let after_count = calls.get();
    rm.render_row(RowPos::new(0, 0));
    rm.render_row(RowPos::new(0, 1));
    assert_eq!(
        calls.get(),
        after_count,
        "rendering rows of an already-counted line must not re-format it"
    );
}

#[test]
fn render_row_does_not_reformat_a_line_because_of_its_virtual_rows() {
    // A Before row is laid out and rendered before its line's content rows —
    // it must not disturb the already-formatted content row/grapheme/arena
    // state that follows it in the same block.
    let rope = Rope::from_str("abcdef\n");
    let (mut providers, calls) = with_counting_insert(0, 0, "hint");
    providers.add_decoration_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(0), 1)));
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 8 }, &providers, &mut s);

    assert_eq!(rm.block(0).content, 2);
    let after_count = calls.get();
    rm.render_row(RowPos::new(0, 0)); // the Before row
    rm.render_row(RowPos::new(0, 1)); // content row 0
    rm.render_row(RowPos::new(0, 2)); // content row 1
    assert_eq!(
        calls.get(),
        after_count,
        "rendering a line's Before row must not force its content rows to re-format"
    );
}

#[test]
fn render_row_yields_correct_content_rows_after_a_virtual_row() {
    // A virtual row's layout must not disturb the content rows that follow
    // it in the same block: they must still come back correct.
    let rope = Rope::from_str("abcdefgh\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor {
        anchor: VirtualLineAnchor::Before(0),
        count: 1,
        text: "V",
    }));
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(row_text(&rm.render_row(RowPos::new(0, 0))), "V");
    assert_eq!(row_text(&rm.render_row(RowPos::new(0, 1))), "abcd");
    assert_eq!(row_text(&rm.render_row(RowPos::new(0, 2))), "efgh");
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
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(rm.locate(5).1, 5, "pure ASCII: column equals char offset");

    // Dropping the map releases its borrow of the scratch, letting the test
    // read what the formatter actually emitted — an oracle over `format.rs`'s
    // output rather than anything `RowMap` reports about itself.
    drop(rm);
    assert_eq!(
        s.graphemes.len(),
        6,
        "the target grapheme and the five before it, not all 70k"
    );
}

#[test]
fn char_at_formats_only_as_far_as_the_target_column() {
    let rope = long_unwrapped_line();
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(rm.char_at(RowPos::new(0, 0), 5, DisplayColTarget::Cell), 5);

    drop(rm);
    assert_eq!(
        s.graphemes.len(),
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
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(rm.locate(3).1, 3);
    assert_eq!(rm.locate(50).1, 50, "the second query must rescan");
}

#[test]
fn a_column_query_after_an_offset_query_reformats() {
    // A byte-bounded scan says nothing about how far the columns reached, so
    // the two bound kinds can never satisfy each other.
    let rope = long_unwrapped_line();
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    let (pos, _) = rm.locate(3);
    assert_eq!(rm.char_at(pos, 40, DisplayColTarget::Cell), 40);
}

#[test]
fn locate_row_answers_without_formatting_in_no_wrap() {
    let rope = long_unwrapped_line();
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(0), 2)));
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        rm.locate_row(5),
        RowPos::new(0, 2),
        "the line's own row sits after the two Before rows above it"
    );

    // `format_buffer_line` pushes its first row before it scans anything, so
    // an empty `display_rows` proves the formatter never ran at all.
    drop(rm);
    assert!(
        s.display_rows.is_empty(),
        "a no-wrap row comes from the block breakdown, with no formatting"
    );
}

#[test]
fn locate_row_agrees_with_locate_in_both_wrap_modes() {
    // `locate` is the oracle here, which is not circular: the claim *is* that
    // the two agree, and `locate`'s own answers are pinned independently by
    // the `locate_*` tests above.
    //
    // "a\n\nébc\n" covers an empty line, a line with Before rows above it, a
    // multi-byte grapheme, and the phantom line past the last `\n`.
    let rope = Rope::from_str("a\n\nébc\n");
    let mut providers = ProviderSet::new();
    providers.add_decoration_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(1), 2)));

    for wrap in [WrapMode::None, WrapMode::Soft { width: 2 }] {
        let mut s = FormatScratch::new();
        let mut rm = map(&rope, wrap, &providers, &mut s);
        // Inclusive upper bound is defensive — the buffer invariant keeps a
        // cursor at `head < len_chars()`.
        for offset in 0..=rope.len_chars() {
            // `locate_row` first, so it has to be right without a previous
            // `locate` having warmed the scratch.
            let row = rm.locate_row(offset);
            let via_locate = rm.locate(offset).0;
            assert_eq!(row, via_locate, "{wrap:?}, offset {offset}");
        }
    }
}

// ---------------------------------------------------------------------------
// line_display_col / char_at_line_display_col
// ---------------------------------------------------------------------------

#[test]
fn line_display_col_matches_locate_column_in_no_wrap() {
    // No-wrap: a line is exactly one row, so the line-relative column and
    // `locate`'s row-relative one must agree everywhere — the invariant
    // `DisplayColOrigin` relies on to treat the two origins as
    // interchangeable there. `locate` is the oracle, pinned independently by
    // the `locate_*` tests above.
    let rope = Rope::from_str("hello\tworld\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    for offset in 0..=rope.len_chars() {
        let expected = rm.locate(offset).1;
        assert_eq!(rm.line_display_col(offset), expected, "offset {offset}");
    }
}

#[test]
fn line_display_col_accumulates_across_a_wrap_row() {
    // "永永永永\n": 4 CJK graphemes, each exactly 2 columns wide regardless of
    // position (unlike a tab), wrapped 2-per-row at width 4. The line-relative
    // column is then just 2x the char offset — an oracle independent of the
    // wrap point, which this asserts crosses the row boundary (offsets 2, 3).
    let rope = Rope::from_str("永永永永\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    // Sanity: the row boundary actually falls where the arithmetic assumes.
    assert_eq!(rm.locate(2).0.row, 1, "third character starts row 1");

    for offset in 0..4 {
        assert_eq!(
            rm.line_display_col(offset),
            offset as u32 * 2,
            "offset {offset}"
        );
    }
}

#[test]
fn line_display_col_excludes_wrap_indent() {
    // "    ab cd\n": 4 leading spaces register one tab-stop of indent
    // (`indent_depth` truncates to whole tab-stops at tab_width 4), so
    // `WrapMode::Indent` opens the continuation row 4 columns in. Every
    // grapheme here is exactly one cell wide with no tabs past the leading
    // run, so a position's TRUE line-relative column is trivially its own
    // char offset — an oracle independent of both `RowMap` and exactly where
    // the line wraps.
    let rope = Rope::from_str("    ab cd\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Indent { width: 7 }, &providers, &mut s);

    for offset in 0..rope.len_chars() {
        assert_eq!(
            rm.line_display_col(offset),
            offset as u32,
            "offset {offset}"
        );
    }

    // Sanity: the line really did wrap, and the last position really is on
    // the indented continuation row, where the row-relative column
    // (`locate`) disagrees with the line-relative one — proving
    // `line_display_col` isn't just forwarding `locate`'s answer verbatim.
    let last = rope.len_chars() - 1;
    let (pos, row_col) = rm.locate(last);
    assert!(
        pos.row > 0,
        "the line must actually wrap for this test to mean anything"
    );
    assert_ne!(
        row_col, last as u32,
        "row-relative column must differ from the line-relative one on an indented row"
    );
}

#[test]
fn line_display_col_counts_a_preceding_inline_insert() {
    // "ab\n" with a 2-cell inline insert ("XY", an inlay hint say) spliced in
    // right before 'b': the insert occupies columns 1..3, so 'b's
    // line-relative column is 3, not its char offset (1) — exactly the
    // quantity the rope-only mirror this API replaces (`place_display_column`)
    // could never see, since inline inserts live only in the decoration layer
    // `RowMap` formats through.
    let rope = Rope::from_str("ab\n");
    let (providers, _calls) = with_counting_insert(0, 1, "XY");
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(rm.line_display_col(0), 0, "'a' precedes the insert");
    assert_eq!(
        rm.line_display_col(1),
        3,
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
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Indent { width: 7 }, &providers, &mut s);

    for offset in 0..rope.len_chars() - 1 {
        let col = rm.line_display_col(offset);
        assert_eq!(
            rm.char_at_line_display_col(0, col, DisplayColTarget::NearestContent),
            offset,
            "offset {offset}, col {col}"
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
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        rm.char_at_line_display_col(1, 5, DisplayColTarget::NearestContent),
        rope.line_to_char(1) + 1,
        "clamps to 'b', not the '\\n'"
    );
}

#[test]
fn char_at_line_display_col_lands_on_newline_for_an_empty_line() {
    // An empty line's only cell *is* the sentinel, so it has to answer.
    let rope = Rope::from_str("\nx\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        rm.char_at_line_display_col(0, 5, DisplayColTarget::NearestContent),
        0
    );
}

#[test]
fn char_at_line_display_col_matches_char_at_in_no_wrap() {
    // No-wrap: line-relative and row-relative columns coincide, so
    // `char_at_line_display_col` must agree with `char_at` (row 0)
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
        for line in hume_rope::lines::content_lines_range(rope) {
            for target in [DisplayColTarget::Cell, DisplayColTarget::NearestContent] {
                for col in 0..12u32 {
                    let mut s = FormatScratch::new();
                    let mut rm = map(rope, WrapMode::None, &providers, &mut s);
                    let expected = rm.char_at(RowPos::new(line, 0), col, target);
                    assert_eq!(
                        rm.char_at_line_display_col(line, col, target),
                        expected,
                        "line {line}, col {col}, {target:?}"
                    );
                }
            }
        }
    }
}
