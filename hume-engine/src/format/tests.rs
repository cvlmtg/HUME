use super::*;
use crate::pane::{WhitespaceConfig, WrapMode};

// Tab-stop arithmetic itself (`hume_rope::width::tab_advance`) is tested at
// its own definition in `hume-rope`, this crate's SSOT for display-column
// math — see `hume_rope::width::tests::tab_advance_*`. What's left to cover
// here is `format_buffer_line`'s use of it, below.

fn do_format(text: &str, wrap_mode: WrapMode) -> (Vec<DisplayRow>, Vec<Grapheme>) {
    let rope = Rope::from_str(text);
    let ws = WhitespaceConfig::default();
    let inserts = Vec::new();
    let mut scratch = LineFormat::new();
    for line_idx in hume_rope::lines::ropey_lines_range(&rope) {
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
    assert_eq!(sentinel.display_col, 5, "sentinel one past last char");
    assert_eq!(sentinel.char_offset, 5, "sentinel at \\n char offset");
}

#[test]
fn a_cr_is_line_content_not_a_line_break() {
    // "a\rb\n" is one line, not two: `\n` is the only break ropey splits on
    // here. A live buffer can't hold a `\r` at all (`BufferText::from`
    // normalizes it away), and `do_format` builds a `Rope::from_str`
    // directly, so this pins the raw-rope contract — the `\r` sits in the
    // row like any other char instead of ending it.
    let (rows, graphemes) = do_format("a\rb\n", WrapMode::None);
    assert_eq!(rows.len(), 2, "\"a\\rb\\n\", \"\" (trailing)");
    let row0_gs = &graphemes[rows[0].graphemes.clone()];
    assert_eq!(row0_gs.len(), 4, "3 content graphemes + eol sentinel");
    let sentinel = &row0_gs[3];
    assert!(
        matches!(sentinel.content, CellContent::Empty),
        "sentinel must be Empty"
    );
    assert_eq!(sentinel.char_offset, 3, "sentinel at the '\\n' char offset");
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
    assert_eq!(row_gs[0].display_col, 0);
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
    // as row0's last cell ("hello ", 6 graphemes — the space ends the row
    // it was seen on, not the continuation row's first cell),
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
    // width=5: "abcd" fills cols 0..4 (current_display_col=4). The next grapheme
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
    assert_eq!(
        row1[0].display_col, 0,
        "'中' starts at column 0 of the new row"
    );
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
    assert_eq!(
        row1[0].display_col, 0,
        "tab starts at column 0 of the new row"
    );
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
    assert_eq!(
        row1[0].display_col, 0,
        "tab starts at column 0 of the new row"
    );
    assert_eq!(
        row1[0].width, 4,
        "tab must expand its full post-wrap tab stop (4), not the stale pre-wrap width (2)"
    );
}

#[test]
fn soft_wrap_exact_fit_row_wraps_the_eol_sentinel_to_a_continuation_row() {
    // "abcde\n" wrapped at width 5 fits exactly, so no content wrap
    // triggers — but the EOL sentinel needs a column of its own, and there
    // isn't one left on a row that's already full. It wraps the same way
    // any other cell that wouldn't fit does: onto a fresh continuation row,
    // at that row's column 0, rather than landing one column past the
    // pane's own right edge.
    //
    // Row 2 here is the trailing empty ropey line's own sentinel (same
    // as plain "hello\n" in `eol_sentinel_emitted_on_non_empty_line`),
    // not a further continuation of line 0.
    let (rows, graphemes) = do_format("abcde\n", WrapMode::Soft { width: 5 });
    assert_eq!(
        rows.len(),
        3,
        "line 0's row + its wrapped sentinel row + the phantom trailing line"
    );
    assert_eq!(rows[0].kind, RowKind::LineStart { line_idx: 0 });
    assert_eq!(
        rows[1].kind,
        RowKind::Wrap {
            line_idx: 0,
            wrap_row: 1
        },
        "the sentinel's row is a continuation of line 0"
    );
    assert_eq!(rows[2].kind, RowKind::LineStart { line_idx: 1 });

    let row0 = &graphemes[rows[0].graphemes.clone()];
    assert_eq!(
        row0.len(),
        5,
        "exactly the 5 content graphemes, no sentinel"
    );

    let row1 = &graphemes[rows[1].graphemes.clone()];
    assert_eq!(row1.len(), 1, "the sentinel alone");
    let sentinel = &row1[0];
    assert!(
        matches!(sentinel.content, CellContent::Empty),
        "sentinel must be Empty"
    );
    assert_eq!(
        sentinel.display_col, 0,
        "sentinel sits at its own row's first column"
    );
    assert_eq!(sentinel.char_offset, 5, "sentinel at the \\n char offset");
}

#[test]
fn tab_expansion_advances_to_tabstop() {
    let (_, graphemes) = do_format("\t", WrapMode::None);
    assert_eq!(graphemes[0].width, 4); // tab at col 0 → 4 wide
}

#[test]
fn grapheme_display_cols_are_correct() {
    let (_, graphemes) = do_format("abc\n", WrapMode::None);
    assert_eq!(graphemes[0].display_col, 0);
    assert_eq!(graphemes[1].display_col, 1);
    assert_eq!(graphemes[2].display_col, 2);
}

// ── Whitespace indicators ─────────────────────────────────────────────

fn do_format_ws(text: &str, ws: WhitespaceConfig) -> (Vec<DisplayRow>, Vec<Grapheme>, String) {
    let rope = Rope::from_str(text);
    let inserts = Vec::new();
    let mut scratch = LineFormat::new();
    for line_idx in hume_rope::lines::ropey_lines_range(&rope) {
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

/// Slice the arena text backing a `Whitespace`/`Virtual` cell — panics if
/// `content` isn't one of those variants (test-only helper).
fn cell_text<'a>(arena: &'a str, content: &CellContent) -> &'a str {
    match content {
        CellContent::Whitespace { start, len } | CellContent::Virtual { start, len } => {
            &arena[*start as usize..*start as usize + *len as usize]
        }
        other => panic!("expected Whitespace or Virtual content, got {other:?}"),
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
    assert_eq!(sentinel.display_col, 3);
    assert_eq!(sentinel.char_offset, 3); // char offset of the '\n'
    let nl_indicator = &row0_gs[4];
    assert_eq!(cell_text(&arena, &nl_indicator.content), "⏎");
    assert_eq!(nl_indicator.display_col, 3);
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
            .any(|g| matches!(&g.content, CellContent::Whitespace { .. }))
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
            .any(|g| matches!(&g.content, CellContent::Whitespace { .. }))
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
    // Space at index 1 should be a Whitespace indicator
    let space_g = graphemes.iter().find(|g| g.display_col == 1).unwrap();
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
    let nbsp_g = graphemes.iter().find(|g| g.display_col == 1).unwrap();
    assert_eq!(cell_text(&arena, &nbsp_g.content), "⍽");
    assert_eq!(nbsp_g.width, 1);
    let ideo_g = graphemes.iter().find(|g| g.display_col == 3).unwrap();
    assert_eq!(cell_text(&arena, &ideo_g.content), "⍽");
    assert_eq!(ideo_g.width, 2, "ideographic space keeps its 2-col width");
}

#[test]
fn nbsp_renders_as_itself_when_off() {
    // With space rendering off, invisible spaces stay CellContent::Grapheme
    // (rendered as themselves) and keep their unicode widths.
    let (_, graphemes, _) = do_format_ws("a\u{A0}b\u{3000}c\n", WhitespaceConfig::default());
    let nbsp_g = graphemes.iter().find(|g| g.display_col == 1).unwrap();
    assert!(matches!(nbsp_g.content, CellContent::Grapheme));
    assert_eq!(nbsp_g.width, 1);
    let ideo_g = graphemes.iter().find(|g| g.display_col == 3).unwrap();
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
    let is_indicator = |display_col: u32| {
        matches!(
            graphemes
                .iter()
                .find(|g| g.display_col == display_col)
                .unwrap()
                .content,
            CellContent::Whitespace { .. }
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
    for display_col in 0..3u32 {
        let g = graphemes
            .iter()
            .find(|g| g.display_col == display_col)
            .unwrap();
        assert_eq!(
            cell_text(&arena, &g.content),
            "·",
            "display col {display_col} should be a trailing indicator"
        );
    }
}

#[test]
fn tab_indicator_trailing_mode_interior() {
    // Same interior-whitespace bug, for tabs. The leading/interior tabs
    // must be `TabFill` (blank, no scope) and only the trailing tab a
    // `Whitespace` glyph — the variant itself is now the "shown" vs.
    // "hidden" distinction, rather than a string compare on arena text.
    let ws = WhitespaceConfig {
        tab: crate::pane::WhitespaceRender::Trailing,
        tab_char: "→",
        ..WhitespaceConfig::default()
    };
    let (_, graphemes, arena) = do_format_ws("\tA\tB\t\n", ws);
    let content_at_offset = |byte_offset: usize| {
        graphemes
            .iter()
            .find(|g| g.byte_range.start == byte_offset)
            .unwrap()
            .content
    };
    assert!(
        matches!(content_at_offset(0), CellContent::TabFill),
        "leading tab renders as blank fill"
    );
    assert!(
        matches!(content_at_offset(2), CellContent::TabFill),
        "interior tab renders as blank fill"
    );
    assert_eq!(
        cell_text(&arena, &content_at_offset(4)),
        "→",
        "trailing tab renders as the glyph"
    );
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
    // "a b" at width 2 (a boundary case): 'a' fits at col0; the space
    // fits exactly at col1 (current_display_col becomes 2); 'b' then overflows
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
fn word_wrap_keeps_a_two_column_tabs_continuation_cell_on_its_own_row() {
    // "ab\tXXXXXXXXXXXX" at width 10, tab_width 4: the tab at display col 2
    // expands to columns 2-3 (advance 2, so it also gets a
    // `WidthContinuation` cell like a CJK character does). Word wrap
    // backtracks to the last whitespace boundary on overflow — that boundary
    // must include the tab's continuation cell, not just the tab's own cell,
    // or the continuation strands itself as the next row's first cell while
    // its primary stays behind on the previous row.
    let (rows, graphemes) = do_format("ab\tXXXXXXXXXXXX", WrapMode::Word { width: 10 });
    assert!(rows.len() >= 2, "the line must wrap");
    let row0 = &graphemes[rows[0].graphemes.clone()];
    let row1 = &graphemes[rows[1].graphemes.clone()];

    let (tab_idx, _) = row0
        .iter()
        .enumerate()
        .find(|(_, g)| matches!(g.content, CellContent::TabFill))
        .expect("the tab's own TabFill cell must be on row0");
    assert!(
        tab_idx + 1 < row0.len()
            && matches!(row0[tab_idx + 1].content, CellContent::WidthContinuation),
        "the tab's WidthContinuation cell must stay on row0, right after the tab"
    );
    assert!(
        !matches!(row1[0].content, CellContent::WidthContinuation),
        "row1 must not start with the tab's stranded continuation cell"
    );
}

#[test]
fn placeholder_wraps_whole_to_a_new_row_when_it_would_straddle_the_wrap_boundary() {
    // "abc" fills display cols 0-2 of a width-6 row, leaving 3 columns —
    // not the 6 a zero-width space's `<200b>` placeholder needs (6 chars,
    // `needs_placeholder` reports it via its own byte length). `maybe_wrap`
    // sees the placeholder's real width before it's ever split into cells,
    // so it must move the whole thing to a fresh row rather than letting it
    // straddle the boundary.
    let (rows, graphemes) = do_format("abc\u{200b}", WrapMode::Soft { width: 6 });
    assert!(rows.len() >= 2, "the line must wrap before the placeholder");
    let row0 = &graphemes[rows[0].graphemes.clone()];
    let row1 = &graphemes[rows[1].graphemes.clone()];
    assert!(
        row0.iter()
            .all(|g| !matches!(g.content, CellContent::Placeholder { .. })),
        "the placeholder must not be split onto row0"
    );
    let ph = row1
        .first()
        .expect("row1 must have at least the placeholder");
    assert!(matches!(ph.content, CellContent::Placeholder { .. }));
    assert_eq!(
        ph.display_col, 0,
        "wrapped placeholder starts at its new row's own column 0"
    );
}

#[test]
fn indent_wrap_continuation_starts_at_indent_display_col() {
    // "    long" with 4 spaces of indent (depth=1, tab_width=4), width=6.
    // First row: "    lo", continuation row starts at display col 4.
    let (rows, graphemes) = do_format("    long text here", WrapMode::Indent { width: 6 });
    assert!(rows.len() >= 2);
    let wrap_row_graphemes = &graphemes[rows[1].graphemes.clone()];
    // The first grapheme on the continuation row should be at display col 4 (indent level).
    assert_eq!(wrap_row_graphemes[0].display_col, 4);
}

// ── CJK double-width ─────────────────────────────────────────────────

#[test]
fn cjk_character_produces_width_continuation() {
    // '中' is a CJK character, display width 2.
    let (_, graphemes) = do_format("中", WrapMode::None);
    assert_eq!(graphemes.len(), 2);
    assert_eq!(graphemes[0].width, 2);
    assert_eq!(graphemes[0].display_col, 0);
    assert!(matches!(
        graphemes[1].content,
        CellContent::WidthContinuation
    ));
    assert_eq!(graphemes[1].display_col, 2);
}

// ── truncate_line_break ─────────────────────────────────────────────────
//
// The break-set contract itself (LF only; CRLF keeps its CR; other Unicode
// breaks are content) belongs to `hume-rope` and is pinned there
// (`hume-rope/src/lines/tests.rs`). These two just cover what
// `truncate_line_break` adds over `strip_line_break`: in-place truncation
// and reporting whether a break was actually removed.

#[test]
fn truncate_line_break_removes_newline_and_reports_true() {
    let mut buf = "hello\n".to_string();
    assert!(hume_rope::lines::truncate_line_break(&mut buf));
    assert_eq!(buf, "hello");
}

#[test]
fn truncate_line_break_no_newline_unchanged_and_reports_false() {
    let mut buf = "hello".to_string();
    assert!(!hume_rope::lines::truncate_line_break(&mut buf));
    assert_eq!(buf, "hello");
}

// ── h_window clipping ─────────────────────────────────────────────────

fn do_format_windowed(
    text: &str,
    wrap_mode: WrapMode,
    h_window: Option<Range<u32>>,
) -> (Vec<DisplayRow>, Vec<Grapheme>) {
    let rope = Rope::from_str(text);
    let ws = WhitespaceConfig::default();
    let inserts = Vec::new();
    let mut scratch = LineFormat::new();
    for line_idx in hume_rope::lines::ropey_lines_range(&rope) {
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
    // (`current_display_col`) long before reaching the end. With a window of
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
    assert!(graphemes.iter().all(|g| g.display_col < 90));
}

#[test]
fn long_line_no_wrap_window_scrolled_right_has_correct_display_cols() {
    // Same 70,000-char ASCII line, scrolled to h_offset = 65,000. Since
    // every char is 1 column wide, display_col must equal char index
    // (independent oracle) for every grapheme actually emitted around the
    // window.
    let text: String = "a".repeat(70_000);
    let (rows, graphemes) = do_format_windowed(&text, WrapMode::None, Some(65_000..65_080));
    assert_eq!(rows.len(), 1);
    assert!(!graphemes.is_empty(), "window should still emit graphemes");
    for g in &graphemes {
        assert_eq!(
            g.display_col as usize, g.char_offset,
            "pure-ASCII line: display_col must equal char index"
        );
    }
    // Nothing before the window's left edge should appear.
    assert!(graphemes.iter().all(|g| g.display_col >= 65_000));
}

// ── Inline-insert char_offset partition invariant ─────────────────────

#[test]
fn row_char_offsets_are_non_decreasing_with_inline_inserts() {
    // Inserts at several offsets, including one at byte 0 (row-start) and
    // one past the last real char (trailing). `resolve_grapheme_display_col`'s
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
    let mut scratch = LineFormat::new();
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

// ── Inline-insert width clamp ──────────────────────────────────────────

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
    let mut scratch = LineFormat::new();
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
    let display_cols: Vec<u32> = insert_cells.iter().map(|g| g.display_col).collect();
    let expected: Vec<u32> = (0..300).collect();
    assert_eq!(
        display_cols, expected,
        "columns advance 0..300 without wraparound"
    );
}

/// Format `line` of a one-line buffer with a single insert at `byte_offset`,
/// returning the format so a test can read cells and resolve arena text.
fn format_with_insert(line: &str, byte_offset: usize, text: &str) -> LineFormat {
    let rope = Rope::from_str(line);
    let inserts = vec![InlineInsert {
        byte_offset,
        text: text.into(),
        scope: crate::types::ScopeId(0),
    }];
    let mut scratch = LineFormat::new();
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
    scratch
}

#[test]
fn control_characters_in_an_inline_insert_render_as_their_codepoint() {
    // An LSP server's `InlayHint.label` reaches the formatter verbatim — no
    // sanitiser sits between `set-inlay-hints!` and here (unlike
    // `set-virtual-lines!`, which substitutes at the Steel boundary). The
    // backend writes each cell's symbol to the terminal as-is, so a literal
    // `\t` would move the terminal's own cursor to its next hardware tab
    // stop and a `\n` would break the frame outright. Both are shown as
    // their codepoint instead, the way Vim does.
    let scratch = format_with_insert("x", 0, ": \tFoo\nBar");

    let resolve = |g: &Grapheme| -> String {
        match g.content {
            // `Whitespace` never appears here — inline inserts have no
            // whitespace-indicator setting of their own; a tab is always
            // `TabFill`, drawn as a plain space with no arena entry.
            CellContent::Placeholder { start, len } | CellContent::Virtual { start, len } => {
                scratch.virtual_texts[start as usize..start as usize + len as usize].to_string()
            }
            CellContent::TabFill => " ".to_string(),
            _ => String::new(),
        }
    };
    for g in &scratch.graphemes {
        assert!(
            !resolve(g).contains(char::is_control),
            "no cell may carry a control character as its symbol, got {:?}",
            resolve(g)
        );
    }

    let insert_cells: Vec<&Grapheme> = scratch
        .graphemes
        .iter()
        .filter(|g| g.byte_range.is_empty() && !matches!(g.content, CellContent::Empty))
        .collect();
    // ": \tFoo\nBar" — cols 0,1 are ": ", the tab at col 2 runs to the next
    // stop (4) and so occupies 2 columns, which earns it a
    // `WidthContinuation` like any other width-2 cell (see
    // `rows::tests::render_row_wide_cjk_before_tab_in_a_virtual_lines_text_shifts_the_stop`).
    // "Foo" then occupies 4..7, and the `\n` renders as `<a>` from col 7.
    let tab_cell = insert_cells[2];
    assert_eq!(tab_cell.display_col, 2);
    assert_eq!(tab_cell.width, 2, "tab at display col 2 reaches stop 4");
    assert_eq!(resolve(tab_cell), " ", "a tab keeps its stop expansion");
    assert!(matches!(
        insert_cells[3].content,
        CellContent::WidthContinuation
    ));
    let newline_cell = insert_cells[7];
    assert_eq!(newline_cell.display_col, 7);
    assert_eq!(resolve(newline_cell), "<a>");
    assert_eq!(newline_cell.width, 3, "the placeholder's own width");
    assert_eq!(
        insert_cells[8].display_col, 10,
        "'B' follows the whole placeholder"
    );
}

#[test]
fn an_invisible_cluster_in_buffer_text_renders_as_its_codepoint() {
    // A zero-width space draws as nothing, so writing it into a cell would
    // advance the terminal by nothing and slide every later grapheme left of
    // the display column the engine believes it is at. It is shown as
    // `<200b>` instead of a blank so a reader can see it is there *and*
    // which character it is — a bidi override rendered as a space is the
    // Trojan Source attack.
    let rope = Rope::from_str("a\u{200B}b");
    let mut scratch = LineFormat::new();
    format_buffer_line(
        &rope,
        0,
        4,
        &WhitespaceConfig::default(),
        &WrapMode::None,
        None,
        FormatBound::Full,
        &[],
        &mut scratch,
    );

    let cells = &scratch.graphemes;
    assert_eq!(cells.len(), 3, "one cell per cluster: 'a', ZWSP, 'b'");
    let CellContent::Placeholder { start, len } = cells[1].content else {
        panic!(
            "an invisible cluster must not render as its own glyph, got {:?}",
            cells[1].content
        );
    };
    assert_eq!(
        &scratch.virtual_texts[start as usize..start as usize + len as usize],
        "<200b>"
    );
    assert_eq!(cells[1].display_col, 1);
    assert_eq!(cells[1].width, 6);
    assert_eq!(
        cells[2].display_col, 7,
        "'b' follows the whole placeholder, not one cell"
    );
}

#[test]
fn a_control_character_in_buffer_text_never_reaches_the_terminal() {
    // The hole a zero-measure test alone leaves open: `unicode-width` calls a
    // control character 1 column, so an ESC would have fallen through to
    // `CellContent::Grapheme` and been written to the terminal verbatim —
    // letting the contents of an opened file drive the editor's own display.
    let rope = Rope::from_str("a\u{1b}b");
    let mut scratch = LineFormat::new();
    format_buffer_line(
        &rope,
        0,
        4,
        &WhitespaceConfig::default(),
        &WrapMode::None,
        None,
        FormatBound::Full,
        &[],
        &mut scratch,
    );

    let cells = &scratch.graphemes;
    let CellContent::Placeholder { start, len } = cells[1].content else {
        panic!(
            "a control character must never render as itself, got {:?}",
            cells[1].content
        );
    };
    assert_eq!(
        &scratch.virtual_texts[start as usize..start as usize + len as usize],
        "<1b>"
    );
    assert_eq!(cells[2].display_col, 5, "'b' follows the placeholder");
}

#[test]
fn a_bidi_override_is_distinguishable_from_a_zero_width_space() {
    // The reason for showing the codepoint rather than a generic marker: the
    // Trojan Source characters are the same `Default_Ignorable` class as a
    // zero-width space, so a single marker glyph would render an attack and
    // a stray invisible space identically.
    let placeholder_for = |text: &str| {
        let rope = Rope::from_str(text);
        let mut scratch = LineFormat::new();
        format_buffer_line(
            &rope,
            0,
            4,
            &WhitespaceConfig::default(),
            &WrapMode::None,
            None,
            FormatBound::Full,
            &[],
            &mut scratch,
        );
        let CellContent::Placeholder { start, len } = scratch.graphemes[0].content else {
            panic!("expected a placeholder cell");
        };
        scratch.virtual_texts[start as usize..start as usize + len as usize].to_string()
    };
    assert_eq!(placeholder_for("\u{202E}"), "<202e>");
    assert_eq!(placeholder_for("\u{200B}"), "<200b>");
    assert_ne!(placeholder_for("\u{202E}"), placeholder_for("\u{200B}"));
}

#[test]
fn an_invisible_cluster_in_an_inline_insert_renders_as_its_codepoint() {
    // Same rule on the decoration side: an LSP `InlayHint.label` reaches the
    // formatter verbatim, so an invisible cluster in one would otherwise be
    // written raw into the cell reserved for it.
    let scratch = format_with_insert("x", 0, "a\u{200B}b");

    let insert_cells: Vec<&Grapheme> = scratch
        .graphemes
        .iter()
        .filter(|g| g.byte_range.is_empty() && !matches!(g.content, CellContent::Empty))
        .collect();
    assert_eq!(insert_cells.len(), 3);
    let CellContent::Placeholder { start, len } = insert_cells[1].content else {
        panic!(
            "an invisible cluster must not render as its own glyph, got {:?}",
            insert_cells[1].content
        );
    };
    assert_eq!(
        &scratch.virtual_texts[start as usize..start as usize + len as usize],
        "<200b>"
    );
    assert_eq!(insert_cells[2].display_col, 7);
}

#[test]
fn wide_grapheme_in_an_inline_insert_gets_a_width_continuation_cell() {
    // A double-width cluster in an inlay hint must emit the same
    // primary-plus-continuation pair a real buffer grapheme does: the second
    // cell is what makes that column addressable (`RowMap`'s
    // `NearestContent`) and styled with the first (`style`'s continuation
    // arm). Without it the two columns of one glyph disagree.
    let scratch = format_with_insert("x", 0, "漢");

    let cells: Vec<&Grapheme> = scratch
        .graphemes
        .iter()
        .filter(|g| g.byte_range.is_empty() && !matches!(g.content, CellContent::Empty))
        .collect();
    assert_eq!(cells.len(), 2, "one primary cell plus its continuation");
    assert_eq!(cells[0].width, 2);
    assert_eq!(cells[0].display_col, 0);
    assert!(matches!(cells[0].content, CellContent::Virtual { .. }));
    assert_eq!(
        cells[1].display_col, 2,
        "continuation is pushed at the post-advance column, as the buffer-line \
         emitter does for a real wide grapheme"
    );
    assert_eq!(cells[1].width, 0, "and consumes no columns of its own");
    assert!(matches!(cells[1].content, CellContent::WidthContinuation));
    assert_eq!(
        cells[1].char_offset, cells[0].char_offset,
        "both cells address the same buffer position"
    );
}

#[test]
fn trailing_insert_emits_one_cell_per_grapheme() {
    // A multi-char insert past the end of the line (diagnostics' EOL
    // summary, an inlay hint's `'after` anchor on the last char, etc.)
    // must go through the same per-grapheme cell emission as a mid-line
    // insert — one Virtual cell per grapheme cluster, not a single wide
    // cell whose text a `Cell` can only paint at one column.
    let rope = Rope::from_str("abc");
    let inserts = vec![InlineInsert {
        byte_offset: 3, // == line_str.len(): never matched by the in-loop
        text: "hello".into(),
        scope: crate::types::ScopeId(0),
    }];
    let mut scratch = LineFormat::new();
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
    let display_cols: Vec<u32> = insert_cells.iter().map(|g| g.display_col).collect();
    assert_eq!(
        display_cols,
        vec![3, 4, 5, 6, 7],
        "columns advance one-by-one starting right after 'abc'"
    );
}

#[test]
fn no_window_caller_reaches_true_column_past_former_u16_ceiling() {
    // `current_display_col`/`Grapheme::display_col` are `u32`: the column at
    // the end of a 70,000-char pure-ASCII line is its true (unclamped) char
    // index, never saturating at `u16::MAX` (65,535). Independent oracle:
    // every char is 1 column wide, so display_col == index.
    let text: String = "a".repeat(70_000);
    let (rows, graphemes) = do_format_windowed(&text, WrapMode::None, None);
    assert_eq!(rows.len(), 1);
    assert_eq!(graphemes.len(), 70_000, "no window: every char is scanned");
    assert_eq!(
        graphemes.last().unwrap().display_col,
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

/// A virtual row far wider than any ordinary one — a provider emitting a
/// pathological string — must not pin its capacity in the pane's scratch for
/// the rest of the session. The frame boundary is where that is given back;
/// `clear` alone (run before laying out *each* row, and followed immediately
/// by filling it again) deliberately does not shrink.
#[test]
fn clear_and_shrink_reclaims_an_oversized_virtual_row() {
    let mut vrow = VirtualRowScratch::new();
    vrow.texts.push_str(&"x".repeat(50_000));
    let grown = vrow.texts.capacity();
    assert!(grown >= 50_000, "sanity: the push must have grown it");

    vrow.clear();
    assert_eq!(
        vrow.texts.capacity(),
        grown,
        "clear runs mid-frame before an immediate refill — shrinking there \
         would only force a re-grow"
    );

    vrow.clear_and_shrink();
    assert!(
        vrow.texts.capacity() < grown,
        "the frame boundary must hand back a pathologically grown buffer"
    );
}

/// Below the ceiling, the scratch keeps its capacity across frames — the
/// whole point of holding one per pane rather than allocating per row.
#[test]
fn clear_and_shrink_keeps_an_ordinary_virtual_row() {
    let mut vrow = VirtualRowScratch::new();
    vrow.texts.push_str(&"x".repeat(200));
    let grown = vrow.texts.capacity();

    vrow.clear_and_shrink();

    assert_eq!(
        vrow.texts.capacity(),
        grown,
        "an ordinary virtual row's capacity must survive the frame boundary"
    );
}
