use super::super::*;
use hume_test_fixtures::assert_state;
use pretty_assertions::assert_eq;

// ── align_selections ──────────────────────────────────────────────────────────

#[test]
fn align_single_selection_noop() {
    // Only the primary: always at target col, no change.
    assert_state!(
        "foo -[=]> 1\n",
        |(buf, sels)| align_selections(buf, sels, 4),
        "foo -[=]> 1\n"
    );
}

#[test]
fn align_forward_insert_spaces() {
    // Primary '=' at col 4 ("foo ="). Secondary '=' at col 3 ("fo =").
    // One space inserted before secondary to reach col 4.
    assert_state!(
        "foo -[=]> 1\nfo -[=]> 2\n",
        |(buf, sels)| align_selections(buf, sels, 4),
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
        |(buf, sels)| align_selections(buf, sels, 4),
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
        |(buf, sels)| align_selections(buf, sels, 4),
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
        |(buf, sels)| align_selections(buf, sels, 4),
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
        |(buf, sels)| align_selections(buf, sels, 4),
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
        |(buf, sels)| align_selections(buf, sels, 4),
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
        |(buf, sels)| align_selections(buf, sels, 4),
        "foo -[=]>\nfo  -[=]>\nfoo -[=]>\n"
    );
}

#[test]
fn align_direction_preserved_forward() {
    // Forward selection spans multiple chars; direction preserved after align.
    assert_state!(
        "foo -[== ]> 1\nfo -[== ]> 2\n",
        |(buf, sels)| align_selections(buf, sels, 4),
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
        |(buf, sels)| align_selections(buf, sels, 4),
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
        |(buf, sels)| align_selections(buf, sels, 4),
        "foo -[=]>\nfo  -[=]>\nfoo -[bar\nbaz]>\n"
    );
}

#[test]
fn align_primary_unchanged() {
    // Primary selection itself is never modified (amount == 0).
    assert_state!(
        "foo -[=]>\nfoo -[=]>\n",
        |(buf, sels)| align_selections(buf, sels, 4),
        "foo -[=]>\nfoo -[=]>\n"
    );
}

#[test]
fn align_remove_tab_before_selection() {
    // The avail count's reverse scan must treat a tab as whitespace, not a
    // chain-breaker — checking `== Some(' ')` alone would stop the scan at
    // the tab and yield avail=0, silently skipping all removal.
    //
    // Buffer " =\n  \t=\n", tab_width 4:
    //   Line 0: ' '+'=' — primary '=' at display col 1.
    //   Line 1: ' ',' ','\t','=' — secondary '=' at display col 4 (the tab
    //     at position 2 advances only to column 4, `tab_advance(2, 4)`).
    //
    // fit_0 = max(line 0: 1-0, line 1: 4-2) = 2, so target[0] = 2 — wider
    // than either line's own baseline, which moves the *primary* too
    // (inserts 1 space: " =" → "  ="). Secondary's removal is char-counted,
    // not display-counted (`align_selections`'s own doc): removing the 2
    // chars closest to '=' (a space and the tab) frees 1+2=3 display
    // columns, one more than the 2 needed, so '=' lands at display col 1,
    // not the target's col 2 — the documented residual imprecision of a tab
    // inside the compressible run itself, distinct from (and narrower than)
    // the cross-line baseline bug this fix closes.
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
    let (new_buf, _new_sels, _cs) = align_selections(buf, sels, 4);
    assert_eq!(
        new_buf.to_string(),
        "  =\n =\n",
        "tab counts toward removal (2 chars removed, not skipped at the tab), \
         though the freed width overshoots by the tab's extra column"
    );
}

#[test]
fn align_accounts_for_a_tab_before_the_alignment_point() {
    // Primary "a\tx = 1" at tab_width 4: 'a' (col 0→1), '\t' (col 1→4,
    // `tab_advance(1, 4)`), 'x' (col 4→5), ' ' (col 5→6) — '=' sits at
    // *display* col 6 though it's only the 4th grapheme cluster on the
    // line. Secondary "bb = 2" has no tab, so its grapheme and display
    // columns agree (3). Aligning by grapheme column alone would insert
    // just enough spaces to match column 4 — landing secondary's '=' two
    // display columns left of primary's, visibly ragged despite matching
    // grapheme counts. Aligning by display column inserts enough to match
    // column 6, where both actually line up on screen.
    use crate::edit::align_selections;
    use hume_editing::selection::SelectionSet;
    let buf = hume_editing::text::Text::from("a\tx = 1\nbb = 2\n");
    let sels = SelectionSet::from_vec(
        vec![
            hume_editing::selection::Selection::collapsed(4), // primary: '=' in "a\tx = 1"
            hume_editing::selection::Selection::collapsed(11), // secondary: '=' in "bb = 2"
        ],
        0,
    );
    let (new_buf, _new_sels, _cs) = align_selections(buf, sels, 4);
    assert_eq!(
        new_buf.to_string(),
        "a\tx = 1\nbb    = 2\n",
        "secondary's '=' must reach display col 6, matching primary's, not grapheme col 4"
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
        |(buf, sels)| align_selections(buf, sels, 4),
        "-[a]>  -[b]>\n-[xy]> -[z]>\n"
    );
}

#[test]
fn align_two_slots_overflow_widens_primary() {
    // Multi-slot: another line's wider content forces target[1] past baseline,
    // so spaces are inserted on the primary line too (primary may move).
    assert_state!(
        "-[x]> -[y]>\n-[loooong]> -[z]>\n",
        |(buf, sels)| align_selections(buf, sels, 4),
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
    let (new_buf, _new_sels, _cs) = align_selections(buf, sels, 4);
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
        |(buf, sels)| align_selections(buf, sels, 4),
        "foo -[x]>\na   -[b]> -[c]>\n"
    );
}
