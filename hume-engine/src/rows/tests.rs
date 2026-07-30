use std::cell::Cell;
use std::rc::Rc;

use ropey::Rope;

use super::*;
use crate::providers::{InlineDecoration, VirtualLineSource};
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

impl VirtualLineSource for FixedAnchor {
    fn virtual_lines(&self, visible: Range<usize>, _width: u16, out: &mut Vec<VirtualLine>) {
        let line = match self.anchor {
            VirtualLineAnchor::Before(n) | VirtualLineAnchor::After(n) => n,
        };
        if visible.contains(&line) {
            for _ in 0..self.count {
                out.push(VirtualLine {
                    anchor: self.anchor,
                    provider_id: 0,
                    text: self.text.to_string(),
                    segments: Vec::new(),
                });
            }
        }
    }
}

/// A `VirtualLineSource` that never emits anything — registered only to consume
/// a `ProviderId` so the next provider's real id is not 0.
struct NoRows;

impl VirtualLineSource for NoRows {
    fn virtual_lines(&self, _visible: Range<usize>, _width: u16, _out: &mut Vec<VirtualLine>) {}
}

/// One inline insert on `line`, counting how often it is queried — the only
/// observable proxy for "did the map run the formatter".
struct CountingInsert {
    line: usize,
    byte_offset: usize,
    text: &'static str,
    calls: Rc<Cell<usize>>,
}

impl InlineDecoration for CountingInsert {
    fn decorations_for_line(&self, line_idx: usize, out: &mut Vec<InlineInsert>) {
        self.calls.set(self.calls.get() + 1);
        if line_idx == self.line {
            out.push(InlineInsert {
                byte_offset: self.byte_offset,
                text: self.text.to_string(),
                scope: ScopeId(0),
            });
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
    providers.add_inline_decoration(Box::new(CountingInsert {
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
            CellContent::Virtual { start, len } | CellContent::Indicator { start, len } => {
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
    // "abcdefgh" is 8 columns wrapped at 4 → "abcd" / "efgh" = 2 rows.
    let rope = Rope::from_str("abcdefgh\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(rm.block(0).content, 2);
}

#[test]
fn block_counts_before_and_after_virtual_rows() {
    // Line 5 of "a\nb\nc\nd\ne\nf\n" gets 2 Before rows and 1 After row from
    // two separate providers, on top of its own single unwrapped content row.
    let rope = Rope::from_str("a\nb\nc\nd\ne\nf\n");
    let mut providers = ProviderSet::new();
    providers.add_virtual_line_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(5), 2)));
    providers.add_virtual_line_source(Box::new(FixedAnchor::new(VirtualLineAnchor::After(5), 1)));
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
    providers.add_virtual_line_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(2), 3)));
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(rm.block(5).before, 0);
    assert_eq!(rm.block(5).after, 0);
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
// kind() / clamp() / last_pos()
// ---------------------------------------------------------------------------

/// Line 0 wrapped into 2 content rows, with 2 Before rows and 1 After row:
/// a 5-row block whose every row kind is hand-known.
fn mixed_block_providers() -> ProviderSet {
    let mut providers = ProviderSet::new();
    providers.add_virtual_line_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(0), 2)));
    providers.add_virtual_line_source(Box::new(FixedAnchor::new(VirtualLineAnchor::After(0), 1)));
    providers
}

#[test]
fn kind_classifies_every_row_of_a_mixed_block() {
    let rope = Rope::from_str("abcdefgh\n");
    let providers = mixed_block_providers();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(rm.block(0).total(), 5);
    let kinds: Vec<RowKind> = (0..5).map(|row| rm.kind(RowPos::new(0, row))).collect();
    assert_eq!(
        kinds,
        vec![
            RowKind::Before(0),
            RowKind::Before(1),
            RowKind::Content(0),
            RowKind::Content(1),
            RowKind::After(0),
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
        RowPos::new(0, 4),
        "row clamps to the block's last row"
    );
    assert_eq!(
        rm.clamp(RowPos::new(99, 99)),
        RowPos::new(0, 4),
        "line clamps to the last real line, then row to its block"
    );
    assert_eq!(
        rm.clamp(RowPos::new(0, 2)),
        RowPos::new(0, 2),
        "an address already inside the document is untouched"
    );
}

#[test]
fn last_pos_is_the_final_block_row() {
    // "a\nb\nc\n" has 3 real lines; line 2 carries 1 After row, so the
    // document's last row is that virtual row at (2, 1).
    let rope = Rope::from_str("a\nb\nc\n");
    let mut providers = ProviderSet::new();
    providers.add_virtual_line_source(Box::new(FixedAnchor::new(VirtualLineAnchor::After(2), 1)));
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        rm.last_line(),
        2,
        "phantom trailing line is not a real line"
    );
    assert_eq!(rm.last_pos(), RowPos::new(2, 1));
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
    providers.add_virtual_line_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(1), 2)));
    providers.add_virtual_line_source(Box::new(FixedAnchor::new(VirtualLineAnchor::After(2), 1)));
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
    assert_eq!(
        breakdown.content, 4,
        "one grapheme per row at wrap width 1"
    );
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
    providers.add_virtual_line_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(0), 2)));
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
    // `style::resolve_grapheme_col` already guarantees for selection styling.
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

    assert_eq!(rm.char_at(RowPos::new(0, 0), 99, ColTarget::Cell), 2);
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
        rm.char_at(RowPos::new(0, 0), 99, ColTarget::NearestContent),
        1
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
        rm.char_at(RowPos::new(0, 0), 10, ColTarget::NearestContent),
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
        rm.char_at(RowPos::new(0, 0), 5, ColTarget::NearestContent),
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
        rm.char_at(RowPos::new(0, 0), 3, ColTarget::Cell),
        0,
        "a click at column 3 hit the tab, so it selects the tab"
    );
    assert_eq!(
        rm.char_at(RowPos::new(0, 0), 3, ColTarget::NearestContent),
        1,
        "a sticky column of 3 is nearer 'x' at column 4 than the tab at 0"
    );
}

#[test]
fn char_at_on_a_virtual_row_clamps_to_the_lines_own_content() {
    // A virtual row carries no buffer position, so an address on one resolves
    // against the nearest content row of the line it is anchored to.
    let rope = Rope::from_str("a\nb\nc\n");
    let mut providers = ProviderSet::new();
    providers.add_virtual_line_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(1), 1)));
    providers.add_virtual_line_source(Box::new(FixedAnchor::new(VirtualLineAnchor::After(2), 1)));
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::None, &providers, &mut s);

    assert_eq!(
        rm.char_at(RowPos::new(1, 0), 0, ColTarget::Cell),
        rope.line_to_char(1),
        "the Before row resolves to line 1's first content row"
    );
    assert_eq!(
        rm.char_at(RowPos::new(2, 1), 0, ColTarget::Cell),
        rope.line_to_char(2),
        "the After row resolves to line 2's last content row"
    );
}

// ---------------------------------------------------------------------------
// content_row_char_bounds()
// ---------------------------------------------------------------------------

#[test]
fn content_row_char_bounds_scopes_to_one_wrap_row() {
    // "abcdefgh\n" at width 4: row 0 covers chars 0..4, row 1 covers 4..9
    // (the next line starts at char 9).
    let rope = Rope::from_str("abcdefgh\n");
    let providers = ProviderSet::new();
    let mut s = FormatScratch::new();
    let mut rm = map(&rope, WrapMode::Soft { width: 4 }, &providers, &mut s);

    assert_eq!(rm.content_row_char_bounds(RowPos::new(0, 0)), Some((0, 4)));
    assert_eq!(rm.content_row_char_bounds(RowPos::new(0, 1)), Some((4, 9)));
}

#[test]
fn content_row_char_bounds_rejects_a_virtual_row() {
    let rope = Rope::from_str("abcdefgh\n");
    let mut providers = ProviderSet::new();
    providers.add_virtual_line_source(Box::new(FixedAnchor::new(VirtualLineAnchor::Before(0), 1)));
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
    providers.add_virtual_line_source(Box::new(NoRows));
    providers.add_virtual_line_source(Box::new(FixedAnchor {
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
fn render_row_reformats_content_after_a_virtual_row_reused_the_scratch() {
    // Laying out a virtual row overwrites the buffers a content line formats
    // into, which is the render order for any line with a Before block. The
    // content rows that follow must still come back correct.
    let rope = Rope::from_str("abcdefgh\n");
    let mut providers = ProviderSet::new();
    providers.add_virtual_line_source(Box::new(FixedAnchor {
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
