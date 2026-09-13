//! change.rs — `c` (with select-inserted-text) and `mii`/select-last-insertion tests.

use super::super::*;
use pretty_assertions::assert_eq;

// ── Tests ─────────────────────────────────────────────────────────────────────

/// `c` must group the delete and the subsequent insert session into a single
/// undo step. One `u` should restore the original selection, not leave a
/// half-undone intermediate state.
///
/// This test feeds real key events through `handle_key` so it catches bugs
/// in the mapping itself (e.g. reverting to ungrouped `apply_edit` for the
/// delete), not just in the underlying group primitives.
#[test]
fn c_groups_delete_and_insert_into_one_undo_step() {
    let mut ed = editor_from("-[hell]>o\n");

    // `c` — delete "hell", enter Insert.
    ed.handle_key(key('c'));
    assert_eq!(ed.state.mode, Mode::Insert);

    // Type the replacement.
    ed.handle_key(key('h'));
    ed.handle_key(key('i'));

    // Exit Insert — commits the group.
    ed.handle_key(key_esc());
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(ed.doc().text().to_string(), "hio\n");

    // One undo should restore the original word entirely.
    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "-[hell]>o\n");

    // Only one revision was recorded.
    assert!(!ed.doc().can_undo());
}

// ── `c` leaves the typed replacement selected (select-inserted-text) ─────────

#[test]
fn c_type_esc_selects_replacement() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('c'));
    ed.handle_key(key('h'));
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "-[hi]>o\n");
}

#[test]
fn c_esc_without_typing_stays_collapsed() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('c'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "-[o]>\n");
}

/// Ctrl-W (delete-word-backward) inside a `c` session must not clear the
/// insertion pin — it's a text edit within the session, not a cursor motion
/// away from it. `select-inserted-text` must still select what survived.
#[test]
fn c_type_ctrl_w_esc_selects_surviving_typed_run() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('c'));
    for ch in "foo bar".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_ctrl('w')); // deletes "bar", keeps "foo "
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "-[foo ]>o\n");
}

#[test]
fn c_multi_cursor_selects_each_replacement() {
    let mut ed = editor_from("-[foo]> -[bar]>\n");
    ed.handle_key(key('c'));
    ed.handle_key(key('x'));
    ed.handle_key(key('y'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "-[xy]> -[xy]>\n");
}

#[test]
fn c_backspace_past_run_start_clamps() {
    let mut ed = editor_from("a-[foo]>b\n");
    ed.handle_key(key('c'));
    ed.handle_key(key('x'));
    ed.handle_key(key_backspace());
    ed.handle_key(key_backspace());
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "-[b]>\n");
}

#[test]
fn c_enter_mid_session_selects_across_newline() {
    let mut ed = editor_from("-[foo]>\n");
    ed.handle_key(key('c'));
    ed.handle_key(key('a'));
    ed.handle_key(key_enter());
    ed.handle_key(key('b'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "-[a\nb]>\n");
}

/// The auto-inserted closer is part of what the session wrote, so the
/// typed-run selection includes it — `run_ends` tracks insertions, not the
/// live cursor head, and `insert_pair_close` inserts `()` as one edit that
/// pushes `run_end` past both chars even though the head lands between them.
#[test]
fn c_auto_pair_includes_trailing_closer() {
    let mut ed = editor_from("-[foo]>\n");
    ed.handle_key(key('c'));
    ed.handle_key(key('('));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "-[(x)]>\n");
}

/// Regression: a cursor motion during the session (arrows etc.) must
/// invalidate the pin — otherwise Esc would select across text the cursor
/// moved away from, using a stale anchor.
#[test]
fn c_arrow_mid_session_cancels_selection() {
    let mut ed = editor_from("-[foo]>\n");
    ed.handle_key(key('c'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_left());
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "a-[b]>\n");
}

#[test]
fn c_setting_false_keeps_current_behavior() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.settings.select_inserted_text = false;
    ed.handle_key(key('c'));
    ed.handle_key(key('h'));
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "hi-[o]>\n");
}

// ── `mii` (select-last-insertion) ────────────────────────────────────────────

fn mii(ed: &mut Editor) {
    ed.handle_key(key('m'));
    ed.handle_key(key('i'));
    ed.handle_key(key('i'));
}

/// With `select-inserted-text` on (the default), `i` already selects what it
/// typed on `Esc` — this test's real point is that `mii` recomputes the
/// identical span independently, so the selection is perturbed (`;`,
/// collapse-to-head) between `Esc` and `mii` to prove `mii` actively
/// reconstructed it rather than the perturbation simply not having happened.
#[test]
fn mii_after_insert_before_selects_typed_text() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('h'));
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "-[hi]>hello\n");
    ed.handle_key(key(';')); // collapse to head — perturb before mii
    mii(&mut ed);
    assert_eq!(state(&ed), "-[hi]>hello\n");
}

#[test]
fn mii_after_insert_after_selects_typed_text() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('a'));
    ed.handle_key(key('X'));
    ed.handle_key(key('Y'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "h-[XY]>ello\n");
    ed.handle_key(key(';'));
    mii(&mut ed);
    assert_eq!(state(&ed), "h-[XY]>ello\n");
}

/// `A` steps the cursor back one grapheme when nothing is typed, but here the
/// typed run's own selected span already lands on the same head position, so
/// `select-inserted-text` selects the whole run, not just the last char.
#[test]
fn mii_after_capital_a_selects_full_typed_run_despite_step_back() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('A'));
    for ch in " world".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "hello-[ world]>\n");
    ed.handle_key(key(';'));
    mii(&mut ed);
    assert_eq!(state(&ed), "hello-[ world]>\n");
}

/// With `select-inserted-text` off, `A`'s own step-back-on-exit still leaves
/// a genuinely collapsed cursor — `mii` must still recover the full typed
/// run despite that, not just reflect whatever Esc left selected.
#[test]
fn mii_after_capital_a_selects_full_typed_run_despite_step_back_setting_off() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.settings.select_inserted_text = false;
    ed.handle_key(key('A'));
    for ch in " world".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "hello worl-[d]>\n"); // step-back cursor
    mii(&mut ed);
    assert_eq!(state(&ed), "hello-[ world]>\n");
}

/// `o` opens the new line before pinning — the anchor must mark the start of
/// typed content, never the structural newline `o` itself inserted.
#[test]
fn mii_after_o_excludes_structural_newline() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('o'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key('c'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "hello\n-[abc]>\n");
    ed.handle_key(key(';'));
    mii(&mut ed);
    assert_eq!(state(&ed), "hello\n-[abc]>\n");
}

#[test]
fn mii_after_capital_o_excludes_structural_newline() {
    let mut ed = editor_from("foo\n-[b]>ar\n");
    ed.handle_key(key('O'));
    ed.handle_key(key('x'));
    ed.handle_key(key('y'));
    ed.handle_key(key('z'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "foo\n-[xyz]>\nbar\n");
    ed.handle_key(key(';'));
    mii(&mut ed);
    assert_eq!(state(&ed), "foo\n-[xyz]>\nbar\n");
}

/// `mii` recomputes the span independently of `select-inserted-text` — after
/// `c` already selected the replacement, `mii` must select the identical
/// range (a regression check that generalizing the pin capture didn't change
/// `c`'s own behavior). The selection is perturbed between `c`'s exit and
/// `mii` so the final assertion can only pass if `mii` actively recomputed
/// the span — not because `c`'s own selection was simply left untouched.
#[test]
fn mii_after_c_matches_select_inserted_text_result() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('c'));
    ed.handle_key(key('h'));
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "-[hi]>o\n");
    ed.handle_key(key(';')); // collapse to head — "h-[i]>o\n"
    assert_eq!(state(&ed), "h-[i]>o\n");
    mii(&mut ed);
    assert_eq!(state(&ed), "-[hi]>o\n");
}

/// With `select-inserted-text` off, `c` leaves a collapsed cursor — but `mii`
/// still recovers the typed span, since pinning doesn't depend on the
/// setting (only auto-select-on-exit does).
#[test]
fn mii_works_after_c_with_setting_off() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.settings.select_inserted_text = false;
    ed.handle_key(key('c'));
    ed.handle_key(key('h'));
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "hi-[o]>\n");
    mii(&mut ed);
    assert_eq!(state(&ed), "-[hi]>o\n");
}

/// Ctrl-W during an `i`-entered session is a text edit, not a cursor motion
/// — it must not clear the pinned anchor `mii` reconstructs from.
#[test]
fn mii_after_insert_with_ctrl_w_selects_surviving_typed_run() {
    let mut ed = editor_from("-[x]>\n");
    ed.handle_key(key('i'));
    for ch in "hello world".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_ctrl('w')); // deletes "world", keeps "hello "
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "-[hello ]>x\n");
    ed.handle_key(key(';'));
    mii(&mut ed);
    assert_eq!(state(&ed), "-[hello ]>x\n");
}

#[test]
fn mii_multi_cursor_selects_each_span_primary_is_last() {
    let mut ed = editor_from("-[foo]> -[bar]>\n");
    ed.handle_key(key('c'));
    ed.handle_key(key('x'));
    ed.handle_key(key('y'));
    ed.handle_key(key_esc());
    mii(&mut ed);
    assert_eq!(state(&ed), "-[xy]> -[xy]>\n");
    // Primary must have relocated to the last (rightmost) span.
    ed.handle_key(key(','));
    assert_eq!(state(&ed), "xy -[xy]>\n");
}

/// In Extend mode `mii` must keep the current selection instead of discarding
/// it — matching the `.extendable()` contract every other `mi*` text object
/// honors. Here the insertion span and the current (collapsed cursor left by
/// `i`) selection are adjacent but don't share an index, so — consistent with
/// `SelectionSet`'s merge rule elsewhere in the codebase, which merges only on
/// genuine overlap, not mere touching — both survive as separate selections
/// rather than being discarded or force-merged. `select-inserted-text` off:
/// this test is about `mii`'s Extend-mode merge logic, which needs a plain
/// collapsed cursor left over from `i` to set up a genuinely adjacent (not
/// identical) current selection.
#[test]
fn mii_extend_mode_keeps_adjacent_current_selection_as_separate() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.settings.select_inserted_text = false;
    ed.handle_key(key('i'));
    ed.handle_key(key('h'));
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "hi-[h]>ello\n"); // setting off — plain collapsed cursor
    ed.state.mode = Mode::Extend;
    mii(&mut ed);
    assert_eq!(state(&ed), "-[hi]>-[h]>ello\n");
}

/// With `select-inserted-text` on (the default), Esc already leaves the
/// current selection identical to the insertion span — the sibling test
/// above disables the setting specifically to avoid this case, so it's
/// covered here instead: `mii` in Extend mode must merge them into one
/// selection (full self-overlap), not append a duplicate.
#[test]
fn mii_extend_mode_default_setting_merges_identical_current_selection() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('h'));
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "-[hi]>hello\n"); // setting on — Esc already selects "hi"
    ed.state.mode = Mode::Extend;
    mii(&mut ed);
    assert_eq!(state(&ed), "-[hi]>hello\n");
}

/// When the current selection genuinely overlaps the insertion span, the
/// union collapses into a single merged selection — proving `mii` in Extend
/// mode actually reaches `SelectionSet`'s merge path, not just an append.
#[test]
fn mii_extend_mode_merges_overlapping_current_selection() {
    use hume_editing::selection::Selection;

    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('h'));
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());
    // buffer is now "hihello"; the insertion span covers indices [0,1] ("hi").
    // Set the current selection to genuinely overlap it: indices [1,3] ("ihe").
    ed.set_current_selections(SelectionSet::single(Selection::new(co(1), co(3))));
    assert_eq!(state(&ed), "h-[ihe]>llo\n");

    ed.state.mode = Mode::Extend;
    mii(&mut ed);
    assert_eq!(state(&ed), "-[hihe]>llo\n");
}

/// When the current selection is disjoint from the insertion span, Extend
/// mode must add it as a separate selection rather than merging or replacing
/// — and the pre-existing selection must stay primary, exactly like every
/// other `mi*` object in Extend mode.
#[test]
fn mii_extend_mode_adds_disjoint_selection_and_keeps_current_primary() {
    use hume_editing::selection::Selection;

    let mut ed = editor_from("-[h]>ello world\n");
    ed.handle_key(key('a'));
    ed.handle_key(key('X'));
    ed.handle_key(key('Y'));
    ed.handle_key(key_esc());

    // Move the current selection onto a disjoint word ("world"), independent
    // of the stashed insertion span. Set directly rather than via a motion
    // command, so this test doesn't couple to unrelated motion mechanics.
    ed.set_current_selections(SelectionSet::single(Selection::new(co(8), co(12))));
    assert_eq!(state(&ed), "hXYello -[world]>\n");

    ed.state.mode = Mode::Extend;
    mii(&mut ed);
    assert_eq!(state(&ed), "h-[XY]>ello -[world]>\n");

    // Primary must have stayed on the pre-existing selection ("world"), not
    // jumped to the newly-unioned insertion span.
    ed.handle_key(key(','));
    assert_eq!(state(&ed), "hXYello -[world]>\n");
}

#[test]
fn mii_reports_info_when_nothing_ever_typed() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());
    mii(&mut ed);
    assert_eq!(ed.state.status_msg.as_deref(), Some("no last insertion"));
}

#[test]
fn mii_reports_info_when_fully_backspaced_away() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_backspace());
    ed.handle_key(key_esc());
    mii(&mut ed);
    assert_eq!(ed.state.status_msg.as_deref(), Some("no last insertion"));
}

/// Any mutation after the session — including one that has nothing to do
/// with inserting — bumps `text_gen` past the stash's stamp.
#[test]
fn mii_stash_goes_stale_after_a_later_edit() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "-[x]>hello\n");
    // Reposition off the just-typed "x" before the "unrelated" edit — Esc
    // left it selected (`select-inserted-text`), so deleting it in place
    // wouldn't distinguish "text_gen bumped" (the thing under test) from
    // "the stashed text is simply gone" (a different, weaker guarantee).
    set_cursor(&mut ed, 2); // "e" of "ello"
    ed.handle_key(key('d')); // unrelated edit — never touches the stashed "x"
    mii(&mut ed);
    assert_eq!(ed.state.status_msg.as_deref(), Some("no last insertion"));
}

#[test]
fn mii_stash_goes_stale_after_undo() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    ed.handle_key(key('u'));
    mii(&mut ed);
    assert_eq!(ed.state.status_msg.as_deref(), Some("no last insertion"));
}

/// A selection-count mismatch mid-session (cursors merging via Backspace)
/// drops the pins entirely — `mii` must find nothing stashed.
#[test]
fn mii_reports_info_when_cursors_merge_mid_session() {
    let mut ed = editor_from("-[a]>-[b]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_backspace());
    ed.handle_key(key_esc());
    mii(&mut ed);
    assert_eq!(ed.state.status_msg.as_deref(), Some("no last insertion"));
}

/// A read-only buffer never opens an edit group, so `i` is refused outright —
/// `mii` must find nothing stashed rather than panicking or stale-reading.
#[test]
fn mii_reports_info_on_read_only_buffer() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.doc_mut().read_only = true;
    ed.handle_key(key('i'));
    assert_eq!(
        ed.state.mode,
        Mode::Normal,
        "read-only buffer refuses Insert"
    );
    mii(&mut ed);
    assert_eq!(ed.state.status_msg.as_deref(), Some("no last insertion"));
}

/// The span's end must land on a grapheme boundary, never mid-cluster —
/// `prev_grapheme_boundary` on a base+combining-mark run steps back to the
/// start of the whole cluster (position 0), not to the invalid position
/// between the base char and its combining mark. The resulting selection
/// covers the base char only (HUME's "1-char selection" is one `char`
/// (codepoint), not one rendered grapheme) — this is the exact formula
/// `end_insert_session`'s own select-on-exit path already uses, reused
/// unchanged here.
#[test]
fn mii_span_end_never_lands_mid_grapheme_cluster() {
    let mut ed = editor_from("-[\n]>");
    ed.handle_key(key('i'));
    ed.handle_key(key('e'));
    ed.handle_key(key('\u{301}')); // combining acute accent
    ed.handle_key(key_esc());
    mii(&mut ed);
    assert_eq!(state(&ed), "-[e]>\u{301}\n");
}

/// A typed combining mark can merge with a PRE-EXISTING base char (not one
/// typed this session), pulling the run's last grapheme boundary behind the
/// anchor rather than onto it. `end_insert_session` must recognize the run
/// as empty in that case, not produce a backwards `(anchor, end)` pair.
#[test]
fn esc_after_combining_mark_merges_with_pre_existing_char_selects_nothing() {
    let mut ed = editor_from("-[e]>\n");
    ed.handle_key(key('a')); // anchor pins at 1, right after the pre-existing 'e'
    ed.handle_key(key('\u{301}')); // combining acute accent merges with 'e'
    ed.handle_key(key_esc());
    // No typed run to select — the empty-run fallback steps the cursor back
    // one grapheme (`a` sets `step_back_on_exit`), and the whole merged
    // cluster is one grapheme, so it steps all the way back to 'e'.
    assert_eq!(state(&ed), "-[e]>\u{301}\n");
    mii(&mut ed);
    assert_eq!(ed.state.status_msg.as_deref(), Some("no last insertion"));
}
