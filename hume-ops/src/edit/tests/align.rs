use super::super::*;
use hume_test_fixtures::assert_state;
use pretty_assertions::assert_eq;

// ── align_selections ──────────────────────────────────────────────────────────

#[test]
fn align_single_selection_noop() {
    // Only the primary: always at target col, no change.
    assert_state!(
        "foo -[=]> 1\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo -[=]> 1\n"
    );
}

#[test]
fn align_forward_insert_spaces() {
    // Primary '=' at col 4 ("foo ="). Secondary '=' at col 3 ("fo =").
    // One space inserted before secondary to reach col 4.
    assert_state!(
        "foo -[=]> 1\nfo -[=]> 2\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo -[=]> 1\nfo  -[=]> 2\n"
    );
}

#[test]
fn align_forward_multiple_spaces_inserted() {
    // Primary '=' at col 2 (on line "ab=c"). Secondary '=' at col 0 (on line "=de").
    // Two spaces inserted before secondary to reach col 2.
    // "foo = 1" has no selection — just buffer content.
    assert_state!(
        "foo = 1\nab-[=]>c\n-[=]>de\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo = 1\nab-[=]>c\n  -[=]>de\n"
    );
}

#[test]
fn align_forward_two_secondaries_insert() {
    // Primary '=' at col 6 (first sel). Both secondaries need spaces.
    // "foobar = 1"   — primary '=' at col 7
    // "foo = 2"      — secondary '=' at col 4, needs 3 spaces
    // "fo = 3"       — secondary '=' at col 3, needs 4 spaces
    assert_state!(
        "foobar -[=]> 1\nfoo -[=]> 2\nfo -[=]> 3\n",
        |(buf, sels)| align_selections(buf, sels),
        "foobar -[=]> 1\nfoo    -[=]> 2\nfo     -[=]> 3\n"
    );
}

#[test]
fn align_forward_remove_spaces() {
    // Line 0 (primary): '=' at col 3, avail=1, rem=0. Floor = 3−0 = 3.
    // Line 1:           '=' at col 6, avail=3, rem=2. Floor = 6−2 = 4.
    // target[0] = max(baseline=3, max_floor=4) = 4.
    // Line 0: insert 1 space → col 4. Line 1: remove 2 spaces → col 4.
    assert_state!(
        "fo -[=]> 1\nfoo   -[=]> 2\n",
        |(buf, sels)| align_selections(buf, sels),
        "fo  -[=]> 1\nfoo -[=]> 2\n"
    );
}

#[test]
fn align_forward_clamped_removal_one_space_left() {
    // Line 0 (primary): '=' at col 2, avail=0, rem=0. Floor = 2.
    // Line 1:           '=' at col 7, avail=2, rem=1. Floor = 7−1 = 6.
    // target[0] = max(baseline=2, max_floor=6) = 6.
    // Line 0: insert 4 spaces → col 6. Line 1: remove 1 space → col 6.
    assert_state!(
        "ab-[=]>\nabcde  -[=]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "ab    -[=]>\nabcde -[=]>\n"
    );
}

#[test]
fn align_clamped_exactly_one_space_available_removes_nothing() {
    // Line 0 (primary): '=' at col 3, avail=1, rem=0. Floor = 3.
    // Line 1:           '=' at col 4, avail=1, rem=0. Floor = 4.
    // target[0] = max(baseline=3, max_floor=4) = 4.
    // Line 0: insert 1 space → col 4. Line 1: amount=0 → unchanged.
    assert_state!(
        "fo -[=]>\nfoo -[=]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "fo  -[=]>\nfoo -[=]>\n"
    );
}

#[test]
fn align_bidirectional_insert_and_remove() {
    // Primary (first sel) '=' at col 4.
    // Second sel '=' at col 3 — insert 1 space → col 4.
    // Third sel '=' at col 6 with 3 spaces before it — need -2, avail=3,
    // max_remove = N-1 = 2 → remove 2, col 4.
    assert_state!(
        "foo -[=]>\nfo -[=]>\nfoo   -[=]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo -[=]>\nfo  -[=]>\nfoo -[=]>\n"
    );
}

#[test]
fn align_direction_preserved_forward() {
    // Forward selection spans multiple chars; direction preserved after align.
    assert_state!(
        "foo -[== ]> 1\nfo -[== ]> 2\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo -[== ]> 1\nfo  -[== ]> 2\n"
    );
}

#[test]
fn align_backward_selection_right_aligns() {
    // Backward selection: anchor = right edge. Primary anchor at col 5.
    // "foo  = 1" — primary, backward '=' anchor at col 5.
    // "foo = 2"  — secondary, backward '=' anchor at col 4. Insert 1 space.
    assert_state!(
        "foo  <[=]- 1\nfoo <[=]- 2\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo  <[=]- 1\nfoo  <[=]- 2\n"
    );
}

#[test]
fn align_multiline_passthrough() {
    // Primary '=' at col 4. Single-line secondary '=' at col 3 — gets +1 space.
    // Multiline "bar\nbaz" spans two lines — passed through unchanged, but its
    // buffer positions shift by +1 (the space inserted for the single-line sel).
    assert_state!(
        "foo -[=]>\nfo -[=]>\nfoo -[bar\nbaz]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo -[=]>\nfo  -[=]>\nfoo -[bar\nbaz]>\n"
    );
}

#[test]
fn align_primary_unchanged() {
    // Primary selection itself is never modified (amount == 0).
    assert_state!(
        "foo -[=]>\nfoo -[=]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo -[=]>\nfoo -[=]>\n"
    );
}

#[test]
fn align_remove_tab_before_selection() {
    // The avail count's reverse scan must treat a tab as whitespace, not a
    // chain-breaker — checking `== Some(' ')` alone would stop the scan at
    // the tab and yield avail=0, silently skipping all removal.
    //
    // Buffer " =\n  \t=\n":
    //   Line 0: ' '+'=' (primary '=' at grapheme col 1)  → target_col=1
    //   Line 1: ' ',' ','\t','=' (secondary '=' at grapheme col 3) → amount=-2
    //
    // Reverse scan continues through the tab and spaces, avail=3,
    // remove=min(2, 3-1)=2 → removes ' '+'\t' → '=' lands at col 1. ✓
    use crate::edit::align_selections;
    use hume_editing::selection::SelectionSet;
    let buf = hume_editing::text::Text::from(" =\n  \t=\n");
    let sels = SelectionSet::from_vec(
        vec![
            hume_editing::selection::Selection::collapsed(1), // primary: '=' col 1
            hume_editing::selection::Selection::collapsed(6), // secondary: '=' col 3
        ],
        0,
    );
    let (new_buf, _new_sels, _cs) = align_selections(buf, sels);
    assert_eq!(
        new_buf.to_string(),
        " =\n =\n",
        "tab should count toward alignment removal"
    );
}

#[test]
fn align_two_slots_per_line() {
    // Multi-slot: primary line has 2 selections defining 2 slot targets.
    // Target for slot 1 is derived per-line from baseline + that line's own
    // gap to slot 0, so line 1's wider gap before 'z' costs it a space
    // while line 0's 'b' gains one.
    assert_state!(
        "-[a]> -[b]>\n-[xy]>  -[z]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "-[a]>  -[b]>\n-[xy]> -[z]>\n"
    );
}

#[test]
fn align_two_slots_overflow_widens_primary() {
    // Multi-slot: another line's wider content forces target[1] past baseline,
    // so spaces are inserted on the primary line too (primary may move).
    assert_state!(
        "-[x]> -[y]>\n-[loooong]> -[z]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "-[x]>       -[y]>\n-[loooong]> -[z]>\n"
    );
}

#[test]
fn align_two_slots_static_text_between() {
    // Regression: static non-selected text before slot 0 and between slots must
    // set the floor, not the selection edge geometry.
    //
    // Input (each '=' and '//' selected, primary on line 0's '='):
    //   "const foo -[=]> 444; -[//]> foo\n"
    //   "const foobar -[=]> 6757383; -[//]> bar\n"
    //   "const a -[=]> 34; -[//]> a\n"
    use crate::edit::align_selections;
    use hume_editing::{
        selection::{Selection, SelectionSet},
        text::Text,
    };
    let buf =
        Text::from("const foo = 444; // foo\nconst foobar = 6757383; // bar\nconst a = 34; // a\n");
    let sels = SelectionSet::from_vec(
        vec![
            Selection::collapsed(10), // primary: '=' on line 0
            Selection::collapsed(17), // '//' on line 0
            Selection::collapsed(37), // '=' on line 1
            Selection::collapsed(48), // '//' on line 1
            Selection::collapsed(63), // '=' on line 2
            Selection::collapsed(69), // '//' on line 2
        ],
        0,
    );
    let (new_buf, _new_sels, _cs) = align_selections(buf, sels);
    assert_eq!(
        new_buf.to_string(),
        "const foo    = 444;     // foo\nconst foobar = 6757383; // bar\nconst a      = 34;      // a\n",
        "slots must widen to clear the widest line's non-removable content"
    );
}

#[test]
fn align_extras_on_same_line_pass_through() {
    // When a non-primary line has more selections than the primary line (N=1 here),
    // the extra selections (slot >= N) pass through shifted by the accumulated
    // edit delta — selection count is preserved.
    assert_state!(
        "foo -[x]>\na -[b]> -[c]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo -[x]>\na   -[b]> -[c]>\n"
    );
}
