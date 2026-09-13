//! `buffer_line_col`/`char_at_buffer_line_col` tests.

use super::*;

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
