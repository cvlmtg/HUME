use super::super::*;
use test_fixtures::assert_state;

// ── join_lines_select_spaces ───────────────────────────────────────────────

#[test]
fn join_lines_cursor_on_line_joins_with_next() {
    // Cursor on '2' (line 2 of 5). Single-line selection → extends to next line.
    assert_state!(
        "1\n-[2]>\n3\n4\n5\n",
        |(text, sels)| join_lines_select_spaces(test_fixtures::testing::state(text, sels)),
        "1\n2-[ ]>3\n4\n5\n"
    );
}

#[test]
fn join_lines_range_spans_two_lines_joins_them() {
    // Forward selection spanning lines 2-3 → joins them.
    assert_state!(
        "1\n-[2\n3]>\n4\n5\n",
        |(text, sels)| join_lines_select_spaces(test_fixtures::testing::state(text, sels)),
        "1\n2-[ ]>3\n4\n5\n"
    );
}

#[test]
fn join_lines_range_spans_three_lines_joins_all() {
    // Forward selection spanning lines 2-3-4 → joins all three.
    assert_state!(
        "1\n-[2\n3\n4]>\n5\n",
        |(text, sels)| join_lines_select_spaces(test_fixtures::testing::state(text, sels)),
        "1\n2-[ ]>3-[ ]>4\n5\n"
    );
}

#[test]
fn join_lines_two_disjoint_cursors_each_joins_independently() {
    // Two cursors: one on '2' (line 2), one on '4' (line 4).
    // Each joins its line with the next, independently.
    assert_state!(
        "1\n-[2]>\n3\n-[4]>\n5\n",
        |(text, sels)| join_lines_select_spaces(test_fixtures::testing::state(text, sels)),
        "1\n2-[ ]>3\n4-[ ]>5\n"
    );
}

#[test]
fn join_lines_two_cursors_on_one_line_join_it_once() {
    assert_state!(
        "a-[b]>c d-[e]>\nf\n",
        |(text, sels)| join_lines_select_spaces(test_fixtures::testing::state(text, sels)),
        "abc de-[ ]>f\n"
    );
}

#[test]
fn join_lines_keeps_a_cursor_that_has_nothing_to_join() {
    // The cursor on '1' joins with the empty line after it (no space); the
    // cursor on the last line has no next line and stays where it is.
    assert_state!(
        "-[1]>\n\n-[3]>\n",
        |(text, sels)| join_lines_select_spaces(test_fixtures::testing::state(text, sels)),
        "-[1]>\n-[3]>\n"
    );
}

#[test]
fn join_lines_skips_empty_line_no_space() {
    // Cursor on line 2, next line is empty → no space inserted.
    assert_state!(
        "1\n-[2]>\n\n3\n",
        |(text, sels)| join_lines_select_spaces(test_fixtures::testing::state(text, sels)),
        "1\n-[2]>\n3\n"
    );
}

#[test]
fn join_lines_multi_cursor_including_last_line_joins_others() {
    // Cursors on every line, including the last. The last-line cursor has no
    // next line to join, so it must not consume the structural '\n' (which would
    // make the changeset invalid). The other cursors join normally and the
    // inserted spaces become the new selections.
    assert_state!(
        "-[1]>\n-[2]>\n-[3]>\n",
        |(text, sels)| join_lines_select_spaces(test_fixtures::testing::state(text, sels)),
        "1-[ ]>2-[ ]>3\n"
    );
}

#[test]
fn join_lines_cursor_on_last_line_noop() {
    // Cursor on the last line: nothing to join, buffer and cursor unchanged.
    assert_state!(
        "1\n2\n3\n4\n-[5]>\n",
        |(text, sels)| join_lines_select_spaces(test_fixtures::testing::state(text, sels)),
        "1\n2\n3\n4\n-[5]>\n"
    );
}

#[test]
fn join_lines_skips_an_indent_cluster_made_of_a_space_and_a_combining_mark() {
    assert_state!(
        "-[a]>\n \u{301}b\n",
        |(text, sels)| join_lines_select_spaces(test_fixtures::testing::state(text, sels)),
        "a-[ ]>b\n"
    );
}

#[test]
fn join_lines_drops_a_non_breaking_space_indent() {
    assert_state!(
        "-[a]>\n\u{a0}\u{3000}b\n",
        |(text, sels)| join_lines_select_spaces(test_fixtures::testing::state(text, sels)),
        "a-[ ]>b\n"
    );
}

#[test]
fn a_space_joined_after_a_prepend_char_selects_the_cluster_it_joins() {
    // A Prepend char glues to what follows, so the inserted space joins it.
    assert_state!(
        "-[a]>\u{600}\nb\n",
        |(text, sels)| join_lines_select_spaces(test_fixtures::testing::state(text, sels)),
        "a-[\u{600} ]>b\n"
    );
}

#[test]
fn join_lines_on_an_empty_first_line_keeps_the_cursor_on_its_break() {
    assert_state!(
        "-[\n]>\n",
        |(text, sels)| join_lines_select_spaces(test_fixtures::testing::state(text, sels)),
        "-[\n]>"
    );
}

#[test]
fn join_lines_over_a_blank_line_after_an_empty_first_line_keeps_the_cursor_on_its_break() {
    assert_state!(
        "-[\n   \n]>x\n",
        |(text, sels)| join_lines_select_spaces(test_fixtures::testing::state(text, sels)),
        "-[\n]>x\n"
    );
}

#[test]
fn join_lines_on_an_empty_line_without_a_space_lands_on_that_lines_own_break() {
    assert_state!(
        "a\n-[\n]>\n\nb\n",
        |(text, sels)| join_lines_select_spaces(test_fixtures::testing::state(text, sels)),
        "a\n-[\n]>\nb\n"
    );
}
