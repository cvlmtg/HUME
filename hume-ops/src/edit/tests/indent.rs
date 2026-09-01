use super::super::*;
use hume_editing::tab_style::TabStyle;
use hume_test_fixtures::assert_state;

// ── indent_lines ──────────────────────────────────────────────────────────

#[test]
fn indent_soft_flush_line() {
    // Flush line, soft style, tab_width=4 → 4 spaces prepended.
    assert_state!(
        "-[f]>oo\n",
        |(text, sels)| indent_lines(text, sels, TabStyle::Soft, 4, 1),
        "    -[f]>oo\n"
    );
}

#[test]
fn indent_hard_flush_line() {
    // Flush line, hard style → one '\t' prepended.
    assert_state!(
        "-[f]>oo\n",
        |(text, sels)| indent_lines(text, sels, TabStyle::Hard, 4, 1),
        "\t-[f]>oo\n"
    );
}

#[test]
fn indent_normalizes_existing_tab_to_soft() {
    // Existing indent is a hard tab (width 4 at tab_width=4). Indenting once
    // more under soft style re-renders the WHOLE new width (8) as spaces,
    // not "tab + 4 spaces" — normalization, not append.
    assert_state!(
        "\t-[f]>oo\n",
        |(text, sels)| indent_lines(text, sels, TabStyle::Soft, 4, 1),
        "        -[f]>oo\n"
    );
}

#[test]
fn indent_normalizes_existing_spaces_to_hard() {
    // Existing indent is 2 spaces (tab_width=4). Indenting once more under
    // hard style re-renders the new width (6) as one tab + 2 spaces.
    assert_state!(
        "  -[f]>oo\n",
        |(text, sels)| indent_lines(text, sels, TabStyle::Hard, 4, 1),
        "\t  -[f]>oo\n"
    );
}

#[test]
fn indent_two_levels_at_once() {
    assert_state!(
        "-[f]>oo\n",
        |(text, sels)| indent_lines(text, sels, TabStyle::Soft, 4, 2),
        "        -[f]>oo\n"
    );
}

#[test]
fn indent_tab_width_eight() {
    assert_state!(
        "-[f]>oo\n",
        |(text, sels)| indent_lines(text, sels, TabStyle::Soft, 8, 1),
        "        -[f]>oo\n"
    );
}

#[test]
fn indent_multiline_selection_indents_every_line() {
    // Anchor sits at column 0 but this selection is NOT linewise (it doesn't
    // reach the last line's trailing '\n'), so it clamps forward past the new
    // indent like any other in-indent position — only a genuinely linewise
    // selection stays pinned at absolute column 0 (see
    // `indent_linewise_selection_stays_linewise`).
    assert_state!(
        "-[one\ntwo\nthree]>\n",
        |(text, sels)| indent_lines(text, sels, TabStyle::Soft, 4, 1),
        "    -[one\n    two\n    three]>\n"
    );
}

// ── blank lines are skipped ──────────────────────────────────────────────

#[test]
fn indent_skips_blank_line_inside_selection() {
    // Middle line is empty — left untouched, no trailing whitespace added.
    assert_state!(
        "-[one\n\nthree]>\n",
        |(text, sels)| indent_lines(text, sels, TabStyle::Soft, 4, 1),
        "    -[one\n\n    three]>\n"
    );
}

#[test]
fn indent_skips_whitespace_only_line_inside_selection() {
    assert_state!(
        "-[one\n   \nthree]>\n",
        |(text, sels)| indent_lines(text, sels, TabStyle::Soft, 4, 1),
        "    -[one\n   \n    three]>\n"
    );
}

#[test]
fn indent_all_blank_selection_is_noop() {
    // Every line touched is blank — no lines rewritten anywhere, so this must
    // take the identity fast path: buffer AND selection both unchanged.
    assert_state!(
        "-[\n\n]>\n",
        |(text, sels)| indent_lines(text, sels, TabStyle::Soft, 4, 1),
        "-[\n\n]>\n"
    );
}

#[test]
fn indent_all_blank_selection_returns_identity_changeset() {
    // Same input as `indent_all_blank_selection_is_noop`, but pinning the
    // property that test can't see: the identity fast path, not a full
    // retain-everything edit that happens to look like a no-op.
    let (text, sels) = hume_test_fixtures::testing::parse_state("-[\n\n]>\n");
    let (_, _, cs) = indent_lines(text, sels, TabStyle::Soft, 4, 1);
    assert!(cs.is_identity());
}

// ── selection endpoints ───────────────────────────────────────────────────

#[test]
fn indent_linewise_selection_stays_linewise() {
    // Anchor at line start, head on the trailing '\n' — a whole-line
    // selection. After indenting, the anchor snaps to the new line start so
    // the selection still covers the whole (now-indented) line.
    assert_state!(
        "-[foo\n]>bar\n",
        |(text, sels)| indent_lines(text, sels, TabStyle::Soft, 4, 1),
        "-[    foo\n]>bar\n"
    );
}

#[test]
fn indent_cursor_inside_old_indent_clamps_to_new_indent_end() {
    // Cursor sits mid-way through 2 spaces of existing indent; after
    // indenting to 6, it clamps to just past the new indent (not shifted by
    // a uniform delta, which would land it back inside the whitespace).
    assert_state!(
        " -[ ]>foo\n",
        |(text, sels)| indent_lines(text, sels, TabStyle::Soft, 4, 1),
        "      -[f]>oo\n"
    );
}

#[test]
fn indent_backward_selection_keeps_direction() {
    assert_state!(
        "<[foo]-\n",
        |(text, sels)| indent_lines(text, sels, TabStyle::Soft, 4, 1),
        "    <[foo]-\n"
    );
}

#[test]
fn indent_two_cursors_same_line_indent_once() {
    assert_state!(
        "f-[o]>o -[b]>ar\n",
        |(text, sels)| indent_lines(text, sels, TabStyle::Soft, 4, 1),
        "    f-[o]>o -[b]>ar\n"
    );
}

#[test]
fn indent_two_disjoint_selections_each_shift_own_lines() {
    assert_state!(
        "-[one]>\ntwo\n-[three]>\n",
        |(text, sels)| indent_lines(text, sels, TabStyle::Soft, 4, 1),
        "    -[one]>\ntwo\n    -[three]>\n"
    );
}

// ── unindent_lines ────────────────────────────────────────────────────────

#[test]
fn unindent_one_level_off_whole_indent() {
    assert_state!(
        "    -[f]>oo\n",
        |(text, sels)| unindent_lines(text, sels, TabStyle::Soft, 4, 1),
        "-[f]>oo\n"
    );
}

#[test]
fn unindent_partial_width_preserving_not_level_snapping() {
    // 6 display columns of indent (tab_width=4) is not a whole number of
    // levels. Unindenting one level removes exactly tab_width (4) columns,
    // landing at width 2 — not snapped down to the nearest lower level
    // boundary (which would be 0).
    assert_state!(
        "      -[f]>oo\n",
        |(text, sels)| unindent_lines(text, sels, TabStyle::Soft, 4, 1),
        "  -[f]>oo\n"
    );
}

#[test]
fn unindent_flush_line_is_noop() {
    assert_state!(
        "-[f]>oo\n",
        |(text, sels)| unindent_lines(text, sels, TabStyle::Soft, 4, 1),
        "-[f]>oo\n"
    );
}

#[test]
fn unindent_flush_line_returns_identity_changeset() {
    // Same input as `unindent_flush_line_is_noop`, pinning the identity fast
    // path rather than just its externally-indistinguishable no-op result.
    let (text, sels) = hume_test_fixtures::testing::parse_state("-[f]>oo\n");
    let (_, _, cs) = unindent_lines(text, sels, TabStyle::Soft, 4, 1);
    assert!(cs.is_identity());
}

#[test]
fn unindent_hard_tab_by_one_level() {
    assert_state!(
        "\t-[f]>oo\n",
        |(text, sels)| unindent_lines(text, sels, TabStyle::Hard, 4, 1),
        "-[f]>oo\n"
    );
}

#[test]
fn indent_then_unindent_round_trips() {
    // Width-preserving normalization: > followed by < with the same
    // tab-width/style must reproduce the original indent exactly, even
    // starting from an off-stop width.
    assert_state!(
        "  -[f]>oo\n",
        |(text, sels)| {
            let (text, sels, _) = indent_lines(text, sels, TabStyle::Soft, 4, 1);
            unindent_lines(text, sels, TabStyle::Soft, 4, 1)
        },
        "  -[f]>oo\n"
    );
}
