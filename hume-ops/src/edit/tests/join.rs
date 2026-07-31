use super::super::*;
use hume_test_fixtures::assert_state;

// ── join_lines_select_spaces ───────────────────────────────────────────────

#[test]
fn join_lines_cursor_on_line_joins_with_next() {
    // Cursor on '2' (line 2 of 5). Single-line selection → extends to next line.
    assert_state!(
        "1\n-[2]>\n3\n4\n5\n",
        |(buf, sels)| join_lines_select_spaces(buf, sels),
        "1\n2-[ ]>3\n4\n5\n"
    );
}

#[test]
fn join_lines_range_spans_two_lines_joins_them() {
    // Forward selection spanning lines 2-3 → joins them.
    assert_state!(
        "1\n-[2\n3]>\n4\n5\n",
        |(buf, sels)| join_lines_select_spaces(buf, sels),
        "1\n2-[ ]>3\n4\n5\n"
    );
}

#[test]
fn join_lines_range_spans_three_lines_joins_all() {
    // Forward selection spanning lines 2-3-4 → joins all three.
    assert_state!(
        "1\n-[2\n3\n4]>\n5\n",
        |(buf, sels)| join_lines_select_spaces(buf, sels),
        "1\n2-[ ]>3-[ ]>4\n5\n"
    );
}

#[test]
fn join_lines_two_disjoint_cursors_each_joins_independently() {
    // Two cursors: one on '2' (line 2), one on '4' (line 4).
    // Each joins its line with the next, independently.
    assert_state!(
        "1\n-[2]>\n3\n-[4]>\n5\n",
        |(buf, sels)| join_lines_select_spaces(buf, sels),
        "1\n2-[ ]>3\n4-[ ]>5\n"
    );
}

#[test]
fn join_lines_skips_empty_line_no_space() {
    // Cursor on line 2, next line is empty → no space inserted.
    assert_state!(
        "1\n-[2]>\n\n3\n",
        |(buf, sels)| join_lines_select_spaces(buf, sels),
        "1\n-[2]>\n3\n"
    );
}

#[test]
fn join_lines_multi_cursor_including_last_line_joins_others() {
    // Cursors on every line, including the last. The last-line cursor has no
    // next line to join — it must not consume the structural '\n' (which would
    // make the changeset invalid). The other cursors join normally and the
    // inserted spaces become the new selections.
    assert_state!(
        "-[1]>\n-[2]>\n-[3]>\n",
        |(buf, sels)| join_lines_select_spaces(buf, sels),
        "1-[ ]>2-[ ]>3\n"
    );
}

#[test]
fn join_lines_cursor_on_last_line_noop() {
    // Cursor on the last line — nothing to join, buffer and cursor unchanged.
    assert_state!(
        "1\n2\n3\n4\n-[5]>\n",
        |(buf, sels)| join_lines_select_spaces(buf, sels),
        "1\n2\n3\n4\n-[5]>\n"
    );
}
