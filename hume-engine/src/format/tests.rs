use super::*;
use crate::pane::{WhitespaceConfig, WrapMode};

#[test]
fn tab_display_width_normal_range() {
    assert_eq!(
        tab_display_width(0, 4),
        4,
        "tab at col 0, width 4 → full stop"
    );
    assert_eq!(
        tab_display_width(2, 4),
        2,
        "tab at col 2, width 4 → half stop"
    );
    assert_eq!(
        tab_display_width(4, 4),
        4,
        "tab exactly on a stop → full width"
    );
}

#[test]
fn tab_display_width_no_overflow_near_u32_max() {
    // col=u32::MAX, tab_width=4: u32::MAX % 4 == 3, so the distance to the
    // next stop is 4 - 3 = 1. The modulo-based formula never computes a
    // "next stop" value that could itself overflow u32.
    assert_eq!(tab_display_width(u32::MAX, 4), 1);
}

fn do_format(text: &str, wrap_mode: WrapMode) -> (Vec<DisplayRow>, Vec<Grapheme>) {
    let rope = Rope::from_str(text);
    let ws = WhitespaceConfig::default();
    let inserts = Vec::new();
    let mut scratch = FormatScratch::new();
    for line_idx in 0..rope.len_lines() {
        format_buffer_line(
            &rope,
            line_idx,
            4,
            &ws,
            &wrap_mode,
            None,
            FormatBound::Full,
            &inserts,
            &mut scratch,
        );
    }
    (scratch.display_rows, scratch.graphemes)
}

#[test]
fn single_line_no_wrap() {
    // No trailing newline → ropey sees exactly 1 line.
    let (rows, graphemes) = do_format("hello", WrapMode::None);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, RowKind::LineStart { line_idx: 0 });
    assert_eq!(graphemes.len(), 5); // 'h','e','l','l','o'
}

#[test]
fn eol_sentinel_emitted_on_non_empty_line() {
    // "hello\n" — the non-empty line must get an eol sentinel at the `\n`
    // position so the cursor is visible when a line-selection head lands on `\n`.
    let (rows, graphemes) = do_format("hello\n", WrapMode::None);
    // "hello\n" has two ropey lines: "hello\n" and "" (trailing).
    assert_eq!(rows.len(), 2);
    let row0_gs = &graphemes[rows[0].graphemes.clone()];
    // 5 content graphemes + 1 eol sentinel.
    assert_eq!(row0_gs.len(), 6, "5 content + eol sentinel");
    let sentinel = &row0_gs[5];
    assert!(
        matches!(sentinel.content, CellContent::Empty),
        "sentinel must be Empty"
    );
    assert_eq!(sentinel.col, 5, "sentinel one past last char");
    assert_eq!(sentinel.char_offset, 5, "sentinel at \\n char offset");
}

#[test]
fn empty_line_produces_empty_sentinel_grapheme() {
    // "a\n\nb" has 3 lines: "a", "", "b".
    // The middle empty line must produce exactly 1 sentinel grapheme with
    // CellContent::Empty so the selection head has something to render on.
    let (rows, graphemes) = do_format("a\n\nb", WrapMode::None);
    assert_eq!(rows.len(), 3, "three lines");
    let empty_row = &rows[1];
    assert_eq!(empty_row.kind, RowKind::LineStart { line_idx: 1 });
    let row_gs = &graphemes[empty_row.graphemes.clone()];
    assert_eq!(row_gs.len(), 1, "exactly one sentinel grapheme");
    assert!(
        matches!(row_gs[0].content, CellContent::Empty),
        "sentinel must be Empty"
    );
    assert_eq!(row_gs[0].col, 0);
    assert_eq!(row_gs[0].width, 1);
}

#[test]
fn two_lines_no_wrap() {
    // No trailing newline → ropey sees exactly 2 lines.
    // "ab\n" has a trailing \n so its row gets the eol sentinel (3 graphemes).
    // "cd" has no trailing \n, so no sentinel (2 graphemes).
    let (rows, graphemes) = do_format("ab\ncd", WrapMode::None);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].graphemes.len(), 3); // 'a', 'b', eol sentinel
    assert_eq!(rows[1].graphemes.len(), 2); // 'c', 'd'
    assert_eq!(graphemes.len(), 5);
}

#[test]
fn soft_wrap_produces_continuation_rows() {
    // 10 chars, wrapped at width 4: rows "hell", "o wo", "rld"
    let (rows, _) = do_format("hello world\n", WrapMode::Soft { width: 4 });
    assert!(
        rows.len() >= 2,
        "expected at least 2 rows, got {}",
        rows.len()
    );
    assert_eq!(rows[0].kind, RowKind::LineStart { line_idx: 0 });
    assert!(matches!(rows[1].kind, RowKind::Wrap { line_idx: 0, .. }));
}

#[test]
fn soft_wrap_splits_at_exact_column_not_whitespace() {
    // "hello world" (11 chars) wrapped at width 7. Soft must split mid-word
    // at column 7 → "hello w" (the 'w' is the 7th grapheme), NOT backtrack
    // to the space at index 5 ("hello").
    let (rows, graphemes) = do_format("hello world", WrapMode::Soft { width: 7 });
    assert!(rows.len() >= 2);
    let row0 = &graphemes[rows[0].graphemes.clone()];
    assert_eq!(
        row0.len(),
        7,
        "soft wrap must split at the exact wrap column, got {} graphemes",
        row0.len()
    );
    // The 7th grapheme (index 6) must be 'w', proving the split is mid-word.
    assert_eq!(
        row0[6].char_offset, 6,
        "last grapheme of row 0 is 'w' at char 6"
    );
}

#[test]
fn soft_and_word_differ_at_same_width() {
    // Same input/width as above; Word backtracks to the space, keeping it
    // as row0's last cell ("hello ", 6 graphemes — B11: the space ends
    // the row it was seen on, not the continuation row's first cell),
    // while Soft splits mid-word ("hello w", 7 graphemes). This is the
    // regression guard: before the fix both produced identical output.
    let (soft_rows, soft_graphemes) = do_format("hello world", WrapMode::Soft { width: 7 });
    let (word_rows, word_graphemes) = do_format("hello world", WrapMode::Word { width: 7 });
    let soft_row0 = &soft_graphemes[soft_rows[0].graphemes.clone()];
    let word_row0 = &word_graphemes[word_rows[0].graphemes.clone()];
    assert_ne!(
        soft_row0.len(),
        word_row0.len(),
        "soft and word must differ; soft row0 = {}, word row0 = {}",
        soft_row0.len(),
        word_row0.len()
    );
    assert_eq!(
        word_row0.len(),
        6,
        "word wrap backtracks to the space, which stays on row0 → \"hello \""
    );
    assert_eq!(
        soft_row0.len(),
        7,
        "soft wrap splits mid-word → \"hello w\""
    );
}

#[test]
fn soft_wrap_defers_wide_char_whole_to_next_row_when_it_would_straddle_column() {
    // width=5: "abcd" fills cols 0..4 (current_col=4). The next grapheme
    // '中' (CJK, display width 2) would need cols 4..6, straddling the
    // wrap column — `maybe_wrap` checks *before* placing a grapheme, so
    // it must defer '中' whole to the next row rather than splitting its
    // two display cells across rows.
    let (rows, graphemes) = do_format("abcd\u{4e2d}ef", WrapMode::Soft { width: 5 });
    assert_eq!(rows.len(), 2, "must wrap into exactly 2 rows");

    let row0 = &graphemes[rows[0].graphemes.clone()];
    assert_eq!(row0.len(), 4, "row 0 holds only \"abcd\", not a split '中'");
    assert_eq!(row0[3].char_offset, 3, "row 0's last grapheme is 'd'");

    let row1 = &graphemes[rows[1].graphemes.clone()];
    assert_eq!(row1.len(), 4, "'中' + its width continuation + 'e' + 'f'");
    assert_eq!(row1[0].char_offset, 4, "row 1 starts with '中'");
    assert_eq!(row1[0].width, 2, "'中' keeps its full display width");
    assert_eq!(row1[0].col, 0, "'中' starts at column 0 of the new row");
    assert!(
        matches!(row1[1].content, CellContent::WidthContinuation),
        "second cell of '中' stays paired with it on the same row"
    );
    assert_eq!(row1[2].char_offset, 5, "'e' follows on row 1");
    assert_eq!(row1[3].char_offset, 6, "'f' follows on row 1");
}

#[test]
fn soft_wrap_defers_tab_whole_to_next_row_when_it_would_straddle_column() {
    // "abcd" fills cols 0..4, tab-stop-aligned, so the tab's full
    // 4-column expansion (cols 4..8) straddles wrap_width=6. Soft wrap
    // must defer the whole tab to the next row rather than truncating
    // its expansion mid-tab. Column 4 keeps the tab tab-stop-aligned
    // both pre- and post-wrap, so this doesn't also exercise the
    // (separate) post-wrap width recompute — see
    // `soft_wrap_recomputes_tab_width_at_post_wrap_column` for that.
    let (rows, graphemes) = do_format("abcd\tef", WrapMode::Soft { width: 6 });
    assert_eq!(rows.len(), 2, "must wrap into exactly 2 rows");

    let row0 = &graphemes[rows[0].graphemes.clone()];
    assert_eq!(row0.len(), 4, "row 0 holds only \"abcd\"");

    let row1 = &graphemes[rows[1].graphemes.clone()];
    assert_eq!(row1.len(), 3, "tab + 'e' + 'f'");
    assert_eq!(row1[0].col, 0, "tab starts at column 0 of the new row");
    assert_eq!(row1[0].width, 4, "tab keeps its full 4-column expansion");
    assert_eq!(row1[1].char_offset, 5, "'e' follows the tab");
    assert_eq!(row1[2].char_offset, 6, "'f' follows 'e'");
}

#[test]
fn soft_wrap_recomputes_tab_width_at_post_wrap_column() {
    // Pre-wrap col=2 ("ab"): the tab would need cols 2..4 there (width 2,
    // its distance to the next tab stop from col 2). Deferred to a new
    // row, it starts at col 0 instead and must expand its full 4-column
    // tab stop — not keep the stale pre-wrap width of 2.
    let (rows, graphemes) = do_format("ab\tc", WrapMode::Soft { width: 3 });
    assert!(rows.len() >= 2, "tab must overflow onto a new row");
    let row1 = &graphemes[rows[1].graphemes.clone()];
    assert_eq!(row1[0].col, 0, "tab starts at column 0 of the new row");
    assert_eq!(
        row1[0].width, 4,
        "tab must expand its full post-wrap tab stop (4), not the stale pre-wrap width (2)"
    );
}

#[test]
fn soft_wrap_exact_fit_row_keeps_eol_sentinel_on_same_row() {
    // "abcde\n" wrapped at width 5 fits exactly, so no content wrap
    // triggers. The EOL sentinel, emitted after the main loop, bypasses
    // `maybe_wrap` entirely (see the "End-of-line sentinel" comment) and
    // lands at col 5 without pushing a wrap continuation row. Pins that a
    // cursor on the trailing '\n' of an exactly-full soft-wrapped row
    // still renders on that row, not a phantom wrap row.
    //
    // Row 1 here is the trailing empty ropey line's own sentinel (same
    // as plain "hello\n" in `eol_sentinel_emitted_on_non_empty_line`),
    // not a continuation of line 0.
    let (rows, graphemes) = do_format("abcde\n", WrapMode::Soft { width: 5 });
    assert_eq!(rows.len(), 2, "line 0's row + the phantom trailing line");
    assert_eq!(
        rows[0].kind,
        RowKind::LineStart { line_idx: 0 },
        "line 0 must not have wrapped into a second row of its own"
    );
    assert_eq!(rows[1].kind, RowKind::LineStart { line_idx: 1 });

    let row0 = &graphemes[rows[0].graphemes.clone()];
    assert_eq!(row0.len(), 6, "5 content graphemes + 1 eol sentinel");
    let sentinel = &row0[5];
    assert!(
        matches!(sentinel.content, CellContent::Empty),
        "sentinel must be Empty"
    );
    assert_eq!(
        sentinel.col, 5,
        "sentinel sits one column past the wrap width"
    );
    assert_eq!(sentinel.char_offset, 5, "sentinel at the \\n char offset");
}

#[test]
fn tab_expansion_advances_to_tabstop() {
    let (_, graphemes) = do_format("\t", WrapMode::None);
    assert_eq!(graphemes[0].width, 4); // tab at col 0 → 4 wide
}

#[test]
fn indent_depth_two_spaces() {
    assert_eq!(compute_indent_depth("  foo", 2), 1);
    assert_eq!(compute_indent_depth("    foo", 2), 2);
    assert_eq!(compute_indent_depth("foo", 2), 0);
}

#[test]
fn grapheme_cols_are_correct() {
    let (_, graphemes) = do_format("abc\n", WrapMode::None);
    assert_eq!(graphemes[0].col, 0);
    assert_eq!(graphemes[1].col, 1);
    assert_eq!(graphemes[2].col, 2);
}

// ── Whitespace indicators ─────────────────────────────────────────────

fn do_format_ws(text: &str, ws: WhitespaceConfig) -> (Vec<DisplayRow>, Vec<Grapheme>, String) {
    let rope = Rope::from_str(text);
    let inserts = Vec::new();
    let mut scratch = FormatScratch::new();
    for line_idx in 0..rope.len_lines() {
        format_buffer_line(
            &rope,
            line_idx,
            4,
            &ws,
            &WrapMode::None,
            None,
            FormatBound::Full,
            &inserts,
            &mut scratch,
        );
    }
    (
        scratch.display_rows,
        scratch.graphemes,
        scratch.virtual_texts,
    )
}

/// Slice the arena text backing an `Indicator`/`Virtual` cell — panics if
/// `content` isn't one of those variants (test-only helper).
fn cell_text<'a>(arena: &'a str, content: &CellContent) -> &'a str {
    match content {
        CellContent::Indicator { start, len } | CellContent::Virtual { start, len } => {
            &arena[*start as usize..*start as usize + *len as usize]
        }
        other => panic!("expected Indicator or Virtual content, got {other:?}"),
    }
}

#[test]
fn newline_indicator_all_mode() {
    let ws = WhitespaceConfig {
        newline: true,
        newline_char: "⏎",
        ..WhitespaceConfig::default()
    };
    let (rows, graphemes, arena) = do_format_ws("abc\n", ws);
    // "abc\n" has 2 ropey lines: "abc\n" (line 0) and "" (line 1, trailing).
    // Line 0: 3 content graphemes + 1 eol sentinel + 1 newline indicator = 5.
    // Line 1: 1 Empty sentinel (eol sentinel for empty trailing line).
    assert_eq!(rows.len(), 2);
    let row0_gs = &graphemes[rows[0].graphemes.clone()];
    assert_eq!(
        row0_gs.len(),
        5,
        "line 0: 3 content + eol sentinel + newline indicator"
    );
    // Sentinel is at index 3, newline indicator at index 4.
    let sentinel = &row0_gs[3];
    assert!(
        matches!(sentinel.content, CellContent::Empty),
        "index 3 is the eol sentinel"
    );
    assert_eq!(sentinel.col, 3);
    assert_eq!(sentinel.char_offset, 3); // char offset of the '\n'
    let nl_indicator = &row0_gs[4];
    assert_eq!(cell_text(&arena, &nl_indicator.content), "⏎");
    assert_eq!(nl_indicator.col, 3);
}

#[test]
fn newline_indicator_all_mode_blank_line() {
    // Newline is inherently always at end-of-line, so `all` shows it even
    // on a whitespace-only line — there's no "trailing" axis to exempt it.
    let ws = WhitespaceConfig {
        newline: true,
        ..WhitespaceConfig::default()
    };
    let (_, graphemes, _) = do_format_ws("   \n", ws);
    assert!(
        graphemes
            .iter()
            .any(|g| matches!(&g.content, CellContent::Indicator { .. }))
    );
}

#[test]
fn newline_indicator_none_mode() {
    let ws = WhitespaceConfig {
        newline: false,
        ..WhitespaceConfig::default()
    };
    let (_, graphemes, _) = do_format_ws("abc\n", ws);
    assert!(
        !graphemes
            .iter()
            .any(|g| matches!(&g.content, CellContent::Indicator { .. }))
    );
}

#[test]
fn space_indicator_all_mode() {
    let ws = WhitespaceConfig {
        space: crate::pane::WhitespaceRender::All,
        space_char: "·",
        ..WhitespaceConfig::default()
    };
    let (_, graphemes, arena) = do_format_ws("a b\n", ws);
    // Space at index 1 should be Indicator
    let space_g = graphemes.iter().find(|g| g.col == 1).unwrap();
    assert_eq!(cell_text(&arena, &space_g.content), "·");
}

#[test]
fn nbsp_indicator_all_mode() {
    // NBSP (U+00A0, width 1) and ideographic space (U+3000, width 2) are
    // gated by the `space` render mode but use the distinct nbsp glyph.
    let ws = WhitespaceConfig {
        space: crate::pane::WhitespaceRender::All,
        ..WhitespaceConfig::default()
    };
    let (_, graphemes, arena) = do_format_ws("a\u{A0}b\u{3000}c\n", ws);
    let nbsp_g = graphemes.iter().find(|g| g.col == 1).unwrap();
    assert_eq!(cell_text(&arena, &nbsp_g.content), "⍽");
    assert_eq!(nbsp_g.width, 1);
    let ideo_g = graphemes.iter().find(|g| g.col == 3).unwrap();
    assert_eq!(cell_text(&arena, &ideo_g.content), "⍽");
    assert_eq!(ideo_g.width, 2, "ideographic space keeps its 2-col width");
}

#[test]
fn nbsp_renders_as_itself_when_off() {
    // With space rendering off, invisible spaces stay CellContent::Grapheme
    // (rendered as themselves) and keep their unicode widths.
    let (_, graphemes, _) = do_format_ws("a\u{A0}b\u{3000}c\n", WhitespaceConfig::default());
    let nbsp_g = graphemes.iter().find(|g| g.col == 1).unwrap();
    assert!(matches!(nbsp_g.content, CellContent::Grapheme));
    assert_eq!(nbsp_g.width, 1);
    let ideo_g = graphemes.iter().find(|g| g.col == 3).unwrap();
    assert!(matches!(ideo_g.content, CellContent::Grapheme));
    assert_eq!(ideo_g.width, 2);
}

#[test]
fn tab_indicator_all_mode() {
    let ws = WhitespaceConfig {
        tab: crate::pane::WhitespaceRender::All,
        tab_char: "→",
        ..WhitespaceConfig::default()
    };
    let (_, graphemes, arena) = do_format_ws("\t", ws);
    assert_eq!(cell_text(&arena, &graphemes[0].content), "→");
    assert_eq!(graphemes[0].width, 4);
}

#[test]
fn space_indicator_trailing_mode_interior() {
    // Regression test: only true trailing whitespace (nothing but
    // whitespace follows it on the line) renders as an indicator.
    // Leading and interior spaces must stay plain even though they come
    // after some earlier non-ws content — the bug was classifying any ws
    // following *some* non-ws grapheme as trailing, regardless of
    // whether more content followed.
    let ws = WhitespaceConfig {
        space: crate::pane::WhitespaceRender::Trailing,
        space_char: "·",
        ..WhitespaceConfig::default()
    };
    let (_, graphemes, _) = do_format_ws("  A  B  \n", ws);
    let is_indicator = |col: u32| {
        matches!(
            graphemes.iter().find(|g| g.col == col).unwrap().content,
            CellContent::Indicator { .. }
        )
    };
    // Leading spaces (cols 0-1): plain.
    assert!(!is_indicator(0));
    assert!(!is_indicator(1));
    // Interior spaces (cols 3-4, between 'A' and 'B'): plain.
    assert!(!is_indicator(3));
    assert!(!is_indicator(4));
    // Trailing spaces (cols 6-7): indicators.
    assert!(is_indicator(6));
    assert!(is_indicator(7));
}

#[test]
fn space_indicator_trailing_mode_blank_line() {
    // A whitespace-only line renders all its spaces as trailing
    // indicators — there's no separate content to be "before".
    let ws = WhitespaceConfig {
        space: crate::pane::WhitespaceRender::Trailing,
        space_char: "·",
        ..WhitespaceConfig::default()
    };
    let (_, graphemes, arena) = do_format_ws("   \n", ws);
    for col in 0..3u32 {
        let g = graphemes.iter().find(|g| g.col == col).unwrap();
        assert_eq!(
            cell_text(&arena, &g.content),
            "·",
            "col {col} should be a trailing indicator"
        );
    }
}

#[test]
fn tab_indicator_trailing_mode_interior() {
    // Same interior-whitespace bug, for tabs. Both the glyph and the
    // off-state fallback are `CellContent::Indicator` (tabs always
    // render through the arena — see `grapheme_display`), so the glyph
    // text itself is the only way to distinguish "shown" from "hidden".
    let ws = WhitespaceConfig {
        tab: crate::pane::WhitespaceRender::Trailing,
        tab_char: "→",
        ..WhitespaceConfig::default()
    };
    let (_, graphemes, arena) = do_format_ws("\tA\tB\t\n", ws);
    let glyph_at_offset = |byte_offset: usize| {
        let g = graphemes
            .iter()
            .find(|g| g.byte_range.start == byte_offset)
            .unwrap();
        cell_text(&arena, &g.content).to_string()
    };
    assert_eq!(
        glyph_at_offset(0),
        " ",
        "leading tab renders as plain space"
    );
    assert_eq!(
        glyph_at_offset(2),
        " ",
        "interior tab renders as plain space"
    );
    assert_eq!(glyph_at_offset(4), "→", "trailing tab renders as the glyph");
}

// ── Wrap modes ────────────────────────────────────────────────────────

#[test]
fn word_wrap_breaks_at_whitespace() {
    // "ab cd ef" with width 5: "ab cd" fits, then "ef" on next row.
    let (rows, graphemes) = do_format("ab cd ef", WrapMode::Word { width: 5 });
    assert!(rows.len() >= 2);
    assert_eq!(rows[0].kind, RowKind::LineStart { line_idx: 0 });
    assert!(matches!(rows[1].kind, RowKind::Wrap { line_idx: 0, .. }));
    // The first row must not contain 'e' or 'f'.
    let row0_graphemes = &graphemes[rows[0].graphemes.clone()];
    assert!(row0_graphemes.len() <= 5);
}

#[test]
fn word_wrap_space_ends_previous_row_not_starts_continuation() {
    // "a b" at width 2 (B11 boundary case): 'a' fits at col0; the space
    // fits exactly at col1 (current_col becomes 2); 'b' then overflows
    // (2+1>2), backtracking to the space. The space (char offset 1) must
    // end row0 ("a "), not become row1's leading cell — splitting so the
    // new row would start with the space, rather than after it, was the
    // bug. Independent oracle: char_offset is the input's own char index,
    // computed by hand from "a b" (a=0, space=1, b=2), not derived from
    // any wrap-logic internals.
    let (rows, graphemes) = do_format("a b", WrapMode::Word { width: 2 });
    assert_eq!(rows.len(), 2, "must wrap into exactly 2 rows");
    let row0 = &graphemes[rows[0].graphemes.clone()];
    let row1 = &graphemes[rows[1].graphemes.clone()];
    assert_eq!(row0.len(), 2, "row0 is \"a \" (a + trailing space)");
    assert_eq!(row0[0].char_offset, 0, "row0[0] is 'a'");
    assert_eq!(row0[1].char_offset, 1, "row0[1] is the space");
    assert_eq!(row1.len(), 1, "row1 is \"b\" only");
    assert_eq!(row1[0].char_offset, 2, "row1[0] is 'b'");
}

#[test]
fn indent_wrap_continuation_starts_at_indent_col() {
    // "    long" with 4 spaces of indent (depth=1, tab_width=4), width=6.
    // First row: "    lo", continuation row starts at col 4.
    let (rows, graphemes) = do_format("    long text here", WrapMode::Indent { width: 6 });
    assert!(rows.len() >= 2);
    let wrap_row_graphemes = &graphemes[rows[1].graphemes.clone()];
    // The first grapheme on the continuation row should be at col 4 (indent level).
    assert_eq!(wrap_row_graphemes[0].col, 4);
}

// ── CJK double-width ─────────────────────────────────────────────────

#[test]
fn cjk_character_produces_width_continuation() {
    // '中' is a CJK character, display width 2.
    let (_, graphemes) = do_format("中", WrapMode::None);
    assert_eq!(graphemes.len(), 2);
    assert_eq!(graphemes[0].width, 2);
    assert_eq!(graphemes[0].col, 0);
    assert!(matches!(
        graphemes[1].content,
        CellContent::WidthContinuation
    ));
    assert_eq!(graphemes[1].col, 2);
}

// ── indent_depth helpers ─────────────────────────────────────────────

#[test]
fn indent_depth_with_tabs() {
    // Two tabs with tab_width=4 => 2 indent levels.
    assert_eq!(compute_indent_depth("\t\tfoo", 4), 2);
    // Mixed: tab (0→4) then space (4→5), depth = 5/4 = 1.
    assert_eq!(compute_indent_depth("\t foo", 4), 1);
}

#[test]
fn indent_depth_zero_tab_width_no_panic() {
    // tab_width=0 should be clamped to 1 internally.
    let depth = compute_indent_depth("  foo", 0);
    assert_eq!(depth, 2); // tw=1, col=2, depth=2
}

// ── strip_line_ending ─────────────────────────────────────────────────

#[test]
fn strip_line_ending_removes_newline() {
    let mut buf = "hello\n".to_string();
    strip_line_ending(&mut buf);
    assert_eq!(buf, "hello");
}

#[test]
fn strip_line_ending_no_newline_unchanged() {
    let mut buf = "hello".to_string();
    strip_line_ending(&mut buf);
    assert_eq!(buf, "hello");
}

#[test]
fn strip_line_ending_crlf_stripped_as_one_unit() {
    // A literal "\r\n" pair can reach the rope (Text::from's CRLF strip
    // leaves one behind in the "\r\r\n" edge case) — both chars go, not
    // just the \n.
    let mut buf = "hello\r\n".to_string();
    strip_line_ending(&mut buf);
    assert_eq!(buf, "hello");
}

#[test]
fn strip_line_ending_non_lf_unicode_break_stripped() {
    // NEL (U+0085) is one of ropey's unicode_lines break chars — a line
    // terminated by it must not render the NEL as a literal trailing char.
    let mut buf = "hello\u{85}".to_string();
    strip_line_ending(&mut buf);
    assert_eq!(buf, "hello");
}

// ── h_window clipping (B1) ───────────────────────────────────────────

fn do_format_windowed(
    text: &str,
    wrap_mode: WrapMode,
    h_window: Option<Range<u32>>,
) -> (Vec<DisplayRow>, Vec<Grapheme>) {
    let rope = Rope::from_str(text);
    let ws = WhitespaceConfig::default();
    let inserts = Vec::new();
    let mut scratch = FormatScratch::new();
    for line_idx in 0..rope.len_lines() {
        format_buffer_line(
            &rope,
            line_idx,
            4,
            &ws,
            &wrap_mode,
            h_window.clone(),
            FormatBound::Full,
            &inserts,
            &mut scratch,
        );
    }
    (scratch.display_rows, scratch.graphemes)
}

#[test]
fn long_line_no_wrap_clips_to_window_without_panic() {
    // 70,000 ASCII chars — without clipping this would overflow `u16`
    // (`current_col`) long before reaching the end. With a window of
    // [0, 80+slack) only a small prefix should be pushed.
    let text: String = "a".repeat(70_000);
    let (rows, graphemes) = do_format_windowed(&text, WrapMode::None, Some(0..80));
    assert_eq!(rows.len(), 1);
    assert!(
        graphemes.len() <= 90,
        "expected a small clipped prefix, got {} graphemes",
        graphemes.len()
    );
    // Every emitted grapheme must fall within (or just at) the window.
    assert!(graphemes.iter().all(|g| g.col < 90));
}

#[test]
fn long_line_no_wrap_window_scrolled_right_has_correct_cols() {
    // Same 70,000-char ASCII line, scrolled to h_offset = 65,000. Since
    // every char is 1 column wide, col must equal char index (independent
    // oracle) for every grapheme actually emitted around the window.
    let text: String = "a".repeat(70_000);
    let (rows, graphemes) = do_format_windowed(&text, WrapMode::None, Some(65_000..65_080));
    assert_eq!(rows.len(), 1);
    assert!(!graphemes.is_empty(), "window should still emit graphemes");
    for g in &graphemes {
        assert_eq!(
            g.col as usize, g.char_offset,
            "pure-ASCII line: col must equal char index"
        );
    }
    // Nothing before the window's left edge should appear.
    assert!(graphemes.iter().all(|g| g.col >= 65_000));
}

// ── Inline-insert char_offset partition invariant (B2) ──────────────

#[test]
fn row_char_offsets_are_non_decreasing_with_inline_inserts() {
    // Inserts at several offsets, including one at byte 0 (row-start) and
    // one past the last real char (trailing). `resolve_grapheme_col`'s
    // partition_point requires the whole row sorted by char_offset.
    let rope = Rope::from_str("abcdef");
    let inserts = vec![
        InlineInsert {
            byte_offset: 0,
            text: "Z".into(),
            scope: crate::types::ScopeId(0),
        },
        InlineInsert {
            byte_offset: 2,
            text: "XY".into(),
            scope: crate::types::ScopeId(0),
        },
        InlineInsert {
            byte_offset: 6,
            text: "W".into(),
            scope: crate::types::ScopeId(0),
        },
    ];
    let mut scratch = FormatScratch::new();
    format_buffer_line(
        &rope,
        0,
        4,
        &WhitespaceConfig::default(),
        &WrapMode::None,
        None,
        FormatBound::Full,
        &inserts,
        &mut scratch,
    );
    assert!(
        scratch
            .graphemes
            .windows(2)
            .all(|w| w[0].char_offset <= w[1].char_offset),
        "char_offset must be non-decreasing across the row: {:?}",
        scratch
            .graphemes
            .iter()
            .map(|g| g.char_offset)
            .collect::<Vec<_>>()
    );
}

// ── Inline-insert width clamp (B7) ──────────────────────────────────

#[test]
fn wide_inline_insert_emits_one_cell_per_grapheme_without_wraparound() {
    // A 300-char ASCII insert must produce 300 width-1 virtual cells
    // (one per grapheme — each cell can only paint one column, see the
    // fix in the insert-injection loop) with columns advancing 0..300,
    // not wrap around via `as u8` truncation anywhere in that count
    // (300 % 256 = 44 would be the buggy value).
    let rope = Rope::from_str("x");
    let text = String::from_utf8(vec![b'a'; 300]).unwrap();
    let inserts = vec![InlineInsert {
        byte_offset: 0,
        text,
        scope: crate::types::ScopeId(0),
    }];
    let mut scratch = FormatScratch::new();
    format_buffer_line(
        &rope,
        0,
        4,
        &WhitespaceConfig::default(),
        &WrapMode::None,
        None,
        FormatBound::Full,
        &inserts,
        &mut scratch,
    );
    let insert_cells: Vec<&Grapheme> = scratch
        .graphemes
        .iter()
        .filter(|g| matches!(g.content, CellContent::Virtual { .. }))
        .collect();
    assert_eq!(insert_cells.len(), 300, "one virtual cell per grapheme");
    assert!(insert_cells.iter().all(|g| g.width == 1));
    let cols: Vec<u32> = insert_cells.iter().map(|g| g.col).collect();
    let expected: Vec<u32> = (0..300).collect();
    assert_eq!(cols, expected, "columns advance 0..300 without wraparound");
}

#[test]
fn trailing_insert_emits_one_cell_per_grapheme() {
    // A multi-char insert past the end of the line (diagnostics' EOL
    // summary, an inlay hint's `'after` anchor on the last char, etc.)
    // must go through the same per-grapheme cell emission as a mid-line
    // insert — one Virtual cell per grapheme cluster, not a single wide
    // cell whose `symbol` a ratatui `Cell` can only paint at one column.
    let rope = Rope::from_str("abc");
    let inserts = vec![InlineInsert {
        byte_offset: 3, // == line_str.len(): never matched by the in-loop
        text: "hello".into(),
        scope: crate::types::ScopeId(0),
    }];
    let mut scratch = FormatScratch::new();
    format_buffer_line(
        &rope,
        0,
        4,
        &WhitespaceConfig::default(),
        &WrapMode::None,
        None,
        FormatBound::Full,
        &inserts,
        &mut scratch,
    );
    let insert_cells: Vec<&Grapheme> = scratch
        .graphemes
        .iter()
        .filter(|g| matches!(g.content, CellContent::Virtual { .. }))
        .collect();
    assert_eq!(insert_cells.len(), 5, "one virtual cell per grapheme");
    assert!(insert_cells.iter().all(|g| g.width == 1));
    let cols: Vec<u32> = insert_cells.iter().map(|g| g.col).collect();
    assert_eq!(
        cols,
        vec![3, 4, 5, 6, 7],
        "columns advance one-by-one starting right after 'abc'"
    );
}

#[test]
fn no_window_caller_reaches_true_column_past_former_u16_ceiling() {
    // `current_col`/`Grapheme::col` are `u32`: the column at the end of a
    // 70,000-char pure-ASCII line is its true (unclamped) char index, never
    // saturating at `u16::MAX` (65,535). Independent oracle: every char is 1
    // column wide, so col == index.
    let text: String = "a".repeat(70_000);
    let (rows, graphemes) = do_format_windowed(&text, WrapMode::None, None);
    assert_eq!(rows.len(), 1);
    assert_eq!(graphemes.len(), 70_000, "no window: every char is scanned");
    assert_eq!(
        graphemes.last().unwrap().col,
        69_999,
        "column exceeds the former u16 ceiling instead of saturating at it"
    );
}

#[test]
fn wrapping_modes_unaffected_by_h_window_none() {
    // Regression: passing None (the only value wrapping modes ever get)
    // must reproduce the existing wrap test's output exactly.
    let (rows, graphemes) = do_format_windowed("hello world", WrapMode::Soft { width: 7 }, None);
    let row0 = &graphemes[rows[0].graphemes.clone()];
    assert_eq!(row0.len(), 7, "soft wrap still splits mid-word at column 7");
}
