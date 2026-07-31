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
fn align_two_columns_per_line() {
    // Multi-column: primary line has 2 selections defining 2 column targets.
    //
    // Input:
    //   "a b\n"  ← primary line: 'a' at col 0 (slot 0), 'b' at col 2 (slot 1)
    //   "xy  z\n" ←             'xy' at col 0 (slot 0), 'z' at col 4 (slot 1)
    //
    // baseline = [0, 2].
    // fit_need[1]: line 0 → target[0]+(acol_b−acol_a)−rem_b = 0+(2−0)−0 = 2
    //              line 1 → 0+(4−0)−1 = 3  ('z' has 2 spaces before it, rem=1).
    // target = [0, 3].
    //
    // Line 0: 'a' amount=0 (retain); 'b' amount=3-2=+1 → insert 1 space.
    // Line 1: 'xy' amount=0 (retain); 'z' amount=3-4=-1 → remove 1 space.
    assert_state!(
        "-[a]> -[b]>\n-[xy]>  -[z]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "-[a]>  -[b]>\n-[xy]> -[z]>\n"
    );
}

#[test]
fn align_two_columns_overflow_widens_primary() {
    // Multi-column: another line's wider content forces target[1] past baseline,
    // so spaces are inserted on the primary line too (primary may move).
    //
    // Input:
    //   "x y\n"      ← primary line: 'x' col 0 (slot 0), 'y' col 2 (slot 1)
    //   "loooong z\n" ←              'loooong' col 0-6 (slot 0), 'z' col 8 (slot 1)
    //
    // baseline = [0, 2].
    // fit_need[1]: line 0 → 0+(2−0)−0=2; line 1 → 0+(8−0)−0=8  ('z' has 1
    //              space before it, rem=0 since avail=1 → rem=avail−1=0).
    // target = [0, max(2,8)=8].
    //
    // Line 0 (primary): 'x' retain; 'y' amount=8-2=+6 → inserts 6 spaces.
    // Line 1:           'loooong' retain; 'z' amount=8-8=0 → retain.
    assert_state!(
        "-[x]> -[y]>\n-[loooong]> -[z]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "-[x]>       -[y]>\n-[loooong]> -[z]>\n"
    );
}

#[test]
fn align_two_columns_static_text_between() {
    // Regression: static non-selected text before slot 0 and between slots must
    // set the floor, not the selection edge geometry.
    //
    // Input (each '=' and '//' selected, primary on line 0's '='):
    //   "const foo -[=]> 444; -[//]> foo\n"
    //   "const foobar -[=]> 6757383; -[//]> bar\n"
    //   "const a -[=]> 34; -[//]> a\n"
    //
    // slot 0 anchors: line 0 = col 10, line 1 = col 13, line 2 = col 8.
    //   all rem=0 (no removable whitespace before '=').
    //   fit_0 = max(10−0, 13−0, 8−0) = 13. target[0] = max(10, 13) = 13.
    //
    // slot 1 anchors: line 0 = col 17, line 1 = col 24, line 2 = col 14.
    //   all rem=0.
    //   fit_1 = max(13+(17−10)−0, 13+(24−13)−0, 13+(14−8)−0) = max(20,24,19) = 24.
    //   target[1] = max(17, 24) = 24.
    //
    // Line 0: '=' @10 → insert 3 → col 13; '//' @17+3=20 → insert 4 → col 24.
    // Line 1: '=' @13 → amount=0;          '//' @24 → amount=0.
    // Line 2: '=' @8  → insert 5 → col 13; '//' @14+5=19 → insert 5 → col 24.
    use crate::edit::align_selections;
    use hume_editing::{
        selection::{Selection, SelectionSet},
        text::Text,
    };
    let buf =
        Text::from("const foo = 444; // foo\nconst foobar = 6757383; // bar\nconst a = 34; // a\n");
    // char offsets (0-based):
    //   line 0: "const foo = 444; // foo\n"
    //            0123456789...
    //     '='  at char 10; '//' starts at char 17
    //   line 1: starts at char 24; "const foobar = 6757383; // bar\n"
    //     '='  at char 24+13=37; '//' at char 24+24=48
    //   line 2: starts at char 55; "const a = 34; // a\n"
    //     '='  at char 55+8=63;  '//' at char 55+14=69
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
        "columns must widen to clear the widest line's non-removable content"
    );
}

#[test]
fn align_extras_on_same_line_pass_through() {
    // When a non-primary line has more selections than the primary line (N=1 here),
    // the extra selections (slot >= N) pass through shifted by the accumulated
    // edit delta — selection count is preserved.
    //
    // Input:
    //   "foo x\n"  ← primary: only 'x' at col 4 (N=1, target[0]=4)
    //   "a b c\n"  ← slot 0='b' col 2 (→ Align(4)), slot 1='c' col 4 (→ Passthrough)
    //
    // 'b' shifts right by +2 → 'c' (Passthrough) also shifts right by +2.
    assert_state!(
        "foo -[x]>\na -[b]> -[c]>\n",
        |(buf, sels)| align_selections(buf, sels),
        "foo -[x]>\na   -[b]> -[c]>\n"
    );
}
