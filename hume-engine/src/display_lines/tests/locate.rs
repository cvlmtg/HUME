//! `locate()`/`char_at()`/`content_display_line_char_bounds()` tests.

use super::*;

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
    providers.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        2,
        "V",
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
    providers.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        VirtualLineAnchor::Before(ContentLine::new(1)),
        1,
        "V",
    )));
    providers.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        VirtualLineAnchor::After(ContentLine::new(2)),
        1,
        "V",
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
    providers.add_decoration_source(Box::new(VirtualLineBlock::uniform(
        VirtualLineAnchor::Before(ContentLine::new(0)),
        1,
        "V",
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
