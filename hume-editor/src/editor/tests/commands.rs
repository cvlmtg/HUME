use super::*;
use crate::editor::dispatch::ArgSource;
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

// ── `c` leaves the typed replacement selected (select-changed-text) ──────────

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

#[test]
fn c_auto_pair_excludes_trailing_closer() {
    let mut ed = editor_from("-[foo]>\n");
    ed.handle_key(key('c'));
    ed.handle_key(key('('));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "-[(x]>)\n");
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
    ed.state.settings.select_changed_text = false;
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

#[test]
fn mii_after_insert_before_selects_typed_text() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('h'));
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "hi-[h]>ello\n"); // plain `i` leaves a collapsed cursor
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
    mii(&mut ed);
    assert_eq!(state(&ed), "h-[XY]>ello\n");
}

/// `A` steps the cursor back one grapheme on exit (cosmetic), but the span
/// `mii` reconstructs must cover everything typed, not just up to the
/// stepped-back cursor.
#[test]
fn mii_after_capital_a_selects_full_typed_run_despite_step_back() {
    let mut ed = editor_from("-[h]>ello\n");
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
    mii(&mut ed);
    assert_eq!(state(&ed), "foo\n-[xyz]>\nbar\n");
}

/// `mii` recomputes the span independently of `select-changed-text` — after
/// `c` already selected the replacement, `mii` must select the identical
/// range (a regression check that generalizing the pin capture didn't change
/// `c`'s own behavior). The selection is perturbed between `c`'s exit and
/// `mii` so the final assertion can only pass if `mii` actively recomputed
/// the span — not because `c`'s own selection was simply left untouched.
#[test]
fn mii_after_c_matches_select_changed_text_result() {
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

/// With `select-changed-text` off, `c` leaves a collapsed cursor — but `mii`
/// still recovers the typed span, since pinning doesn't depend on the
/// setting (only auto-select-on-exit does).
#[test]
fn mii_works_after_c_with_setting_off() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.settings.select_changed_text = false;
    ed.handle_key(key('c'));
    ed.handle_key(key('h'));
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "hi-[o]>\n");
    mii(&mut ed);
    assert_eq!(state(&ed), "-[hi]>o\n");
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
/// rather than being discarded or force-merged.
#[test]
fn mii_extend_mode_keeps_adjacent_current_selection_as_separate() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('h'));
    ed.handle_key(key('i'));
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), "hi-[h]>ello\n"); // plain `i` leaves a collapsed cursor
    ed.state.mode = Mode::Extend;
    mii(&mut ed);
    assert_eq!(state(&ed), "-[hi]>-[h]>ello\n");
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
    let pid = ed.state.focused_pane_id;
    let bid = ed.focused_buffer_id();
    ed.state.panes.state[pid][bid].selections = SelectionSet::single(Selection::new(1, 3));
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
    let pid = ed.state.focused_pane_id;
    let bid = ed.focused_buffer_id();
    ed.state.panes.state[pid][bid].selections = SelectionSet::single(Selection::new(8, 12));
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
    assert_eq!(state(&ed), "x-[h]>ello\n");
    ed.handle_key(key('d')); // unrelated edit — never touches `last_insert`
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
/// `c`'s `select-changed-text` already uses, reused unchanged here.
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

// ── Undo/redo boundary messages ────────────────────────────────────────────

#[test]
fn undo_at_root_shows_message() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    ed.handle_key(key('u'));
    assert!(
        ed.state.status_msg.is_none(),
        "first undo should succeed — no message"
    );
    ed.handle_key(key('u'));
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Already at oldest change"),
        "second undo at root should show message"
    );
}

#[test]
fn undo_message_cleared_on_next_keypress() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    ed.handle_key(key('u'));
    ed.handle_key(key('u'));
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Already at oldest change")
    );
    ed.handle_key(key('l'));
    assert!(
        ed.state.status_msg.is_none(),
        "next keypress clears the undo-at-root message"
    );
}

#[test]
fn redo_at_newest_shows_message() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    ed.handle_key(key('u'));
    ed.handle_key(key_ctrl('r'));
    assert!(
        ed.state.status_msg.is_none(),
        "first redo should succeed — no message"
    );
    ed.handle_key(key_ctrl('r'));
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Already at newest change"),
        "second redo at newest should show message"
    );
}

#[test]
fn undo_with_count_shows_message_on_exhaustion() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    // Type a count prefix "2" before "u"
    ed.handle_key(key('2'));
    ed.handle_key(key('u'));
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Already at oldest change"),
        "count=2 with only 1 undo step should show message on final step"
    );
}

#[test]
fn redo_with_count_shows_message_on_exhaustion() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    ed.handle_key(key('u'));
    // Type a count prefix "2" before Ctrl+r
    ed.handle_key(key('2'));
    ed.handle_key(key_ctrl('r'));
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Already at newest change"),
        "count=2 with only 1 redo step should show message on final step"
    );
}

// ── `d` pushes deleted text onto the kill ring ─────────────────────────────

/// Deleting a selection must push the deleted text onto the kill ring.
/// A bug in the mapping that removed the `yank_selections` call before
/// `delete_selection` would leave the ring empty — invisible to pure tests.
#[test]
fn d_yanks_selection_into_register_before_deleting() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('d'));

    assert_eq!(ed.doc().text().to_string(), "o\n", "buffer after delete");
    assert_eq!(
        ed.state.kill_ring.head(),
        Some(["hell".to_string()].as_slice()),
        "kill ring head after delete"
    );
}

// ── `y` yanks without modifying the buffer ─────────────────────────────────

/// `y` must write to the system clipboard (in-memory mirror) and push to the
/// kill ring, without changing the buffer or the selection.
/// This is the only way to test that `y` actually writes the correct storage —
/// pure tests of `yank_selections` never touch `Editor.registers` or `kill_ring`.
#[test]
fn y_populates_register_without_changing_buffer() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('y'));

    assert_eq!(state(&ed), "-[hell]>o\n", "buffer+selection unchanged");
    // Bare `y` writes to system clipboard (in-memory mirror in headless tests)
    // AND pushes to the kill ring.
    assert_eq!(
        reg(&ed, CLIPBOARD_REGISTER),
        &["hell"],
        "clipboard populated"
    );
    assert_eq!(
        ed.state.kill_ring.head(),
        Some(["hell".to_string()].as_slice()),
        "kill ring head populated"
    );
}

// ── `r<char>` pending-key replace sequence ─────────────────────────────────

/// `r` sets a wait-char constructor; the following character replaces every
/// grapheme in every selection; and `Esc` after a bare `r` cancels without
/// side effects.
#[test]
fn r_then_char_replaces_every_grapheme_in_selection() {
    let mut ed = editor_from("-[hell]>o\n");

    ed.handle_key(key('r'));
    assert!(ed.state.wait_char.is_some(), "wait_char set after 'r'");

    ed.handle_key(key('x'));
    assert!(
        ed.state.wait_char.is_none(),
        "wait_char cleared after replacement char"
    );
    assert_eq!(state(&ed), "-[xxxx]>o\n");
}

#[test]
fn r_then_esc_cancels_without_side_effects() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('r'));
    // Esc resets wait_char (and all other pending state).
    ed.handle_key(key_esc());

    assert!(ed.state.wait_char.is_none());
    assert_eq!(
        state(&ed),
        "-[hell]>o\n",
        "buffer unchanged after cancelled replace"
    );
}

/// Unlike `r`, find/till has extend duality — this exercises that branch
/// being cleanly torn down on Esc.
#[test]
fn f_then_esc_cancels_without_side_effects() {
    let mut ed = editor_from("-[h]>ello a\n");
    ed.handle_key(key('f'));
    assert!(ed.state.wait_char.is_some(), "wait_char set after 'f'");
    ed.handle_key(key_esc());

    assert!(ed.state.wait_char.is_none(), "wait_char cleared after Esc");
    assert!(ed.state.pending_char.is_none(), "pending_char not set");
    assert_eq!(
        state(&ed),
        "-[h]>ello a\n",
        "buffer and cursor unchanged after cancelled find"
    );
}

// ── `m i w` three-key text-object sequence ─────────────────────────────────

/// The trie must advance through `m` (Interior) → `mi` (Interior) → `miw`
/// (Leaf) and dispatch the correct text-object command on the third key.
/// This exercises the entire three-key pipeline end-to-end.
#[test]
fn m_i_w_selects_inner_word() {
    let mut ed = editor_from("-[h]>ello world\n");

    ed.handle_key(key('m'));
    assert_eq!(
        ed.state.pending_keys.len(),
        1,
        "pending_keys has 'm' after first press"
    );

    ed.handle_key(key('i'));
    assert_eq!(
        ed.state.pending_keys.len(),
        2,
        "pending_keys has 'm','i' after second press"
    );

    ed.handle_key(key('w'));
    assert!(
        ed.state.pending_keys.is_empty(),
        "pending_keys cleared after dispatch"
    );
    assert_eq!(state(&ed), "-[hello]> world\n");
}

/// An unrecognised object char after `ma` must clear pending state without
/// modifying the buffer or the selection.
#[test]
fn m_a_unknown_char_falls_through_cleanly() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.handle_key(key('m'));
    ed.handle_key(key('a'));
    // '~' is not a known text-object char — NoMatch clears pending state.
    ed.handle_key(key('~'));

    assert!(
        ed.state.pending_keys.is_empty(),
        "pending_keys cleared on NoMatch"
    );
    // Selection and buffer are unchanged.
    assert_eq!(state(&ed), "-[h]>ello\n");
}

// ── `e` extend-mode toggle ─────────────────────────────────────────────────

/// `e` must toggle `extend` on and off. While extend is active, motions must
/// grow the selection rather than collapse it to a cursor.
#[test]
fn e_toggles_extend_mode_and_motions_extend_selection() {
    let mut ed = editor_from("-[h]>ello\n");
    assert_eq!(ed.state.mode, Mode::Normal, "Normal mode initially");

    // Toggle extend on.
    ed.handle_key(key('e'));
    assert_eq!(ed.state.mode, Mode::Extend, "Extend mode after 'e'");

    // A motion in extend mode should grow the selection, not move a cursor.
    ed.handle_key(key('l'));
    assert_eq!(state(&ed), "-[he]>llo\n", "selection extended right by one");

    // Toggle extend off.
    ed.handle_key(key('e'));
    assert_eq!(ed.state.mode, Mode::Normal, "Normal mode after second 'e'");
}

// ── `x` select-line ────────────────────────────────────────────────────────

/// `x` selects the full current line including the trailing `\n`.
#[test]
fn x_selects_full_line_from_cursor() {
    let mut ed = editor_from("hello -[w]>orld\nfoo\n");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "-[hello world\n]>foo\n");
}

/// `x` on a line that is already fully selected jumps to the next line.
#[test]
fn x_on_full_line_jumps_to_next() {
    let mut ed = editor_from("-[hello world\n]>foo\n");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "hello world\n-[foo\n]>");
}

/// In extend mode, `x` extends the selection to include the next line.
#[test]
fn x_in_extend_mode_accumulates_lines() {
    let mut ed = editor_from("hello -[w]>orld\nfoo\nbar\n");
    // First `x` in normal mode: select current line.
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "-[hello world\n]>foo\nbar\n", "line 1 selected");
    // Toggle extend mode.
    ed.handle_key(key('e'));
    // `x` in extend mode: extend to include next line.
    ed.handle_key(key('x'));
    assert_eq!(
        state(&ed),
        "-[hello world\nfoo\n]>bar\n",
        "lines 1-2 selected"
    );
    // Another `x`: extend to line 3.
    ed.handle_key(key('x'));
    assert_eq!(
        state(&ed),
        "-[hello world\nfoo\nbar\n]>",
        "lines 1-3 selected"
    );
}

/// `x` repeated in normal mode walks downward: each press moves to the next line.
#[test]
fn x_repeated_walks_lines_down() {
    let mut ed = editor_from("hello -[w]>orld\nfoo\nbar\n");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "-[hello world\n]>foo\nbar\n", "line 1");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "hello world\n-[foo\n]>bar\n", "line 2");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "hello world\nfoo\n-[bar\n]>", "line 3");
}

/// `x` at the last line stays put (no panic).
#[test]
fn x_clamps_at_last_line() {
    let mut ed = editor_from("hello\n-[world\n]>");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "hello\n-[world\n]>");
}

/// `X` selects the current line with a backward selection (anchor=`\n`, head=start).
#[test]
fn shift_x_selects_line_backward() {
    let mut ed = editor_from("hello -[w]>orld\nfoo\n");
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "<[hello world\n]-foo\n");
}

/// `X` repeated in normal mode walks upward: each press moves to the previous line.
#[test]
fn shift_x_repeated_walks_lines_up() {
    let mut ed = editor_from("aaa\nbbb\nhello -[w]>orld\n");
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "aaa\nbbb\n<[hello world\n]-", "line 3");
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "aaa\n<[bbb\n]-hello world\n", "line 2");
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "<[aaa\n]-bbb\nhello world\n", "line 1");
}

/// `X` at the first line stays put (no panic).
#[test]
fn shift_x_clamps_at_first_line() {
    let mut ed = editor_from("<[hello world\n]-foo\n");
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "<[hello world\n]-foo\n");
}

/// Ctrl+x accumulates lines downward (extend behavior).
#[test]
fn ctrl_x_extends_selection_down() {
    let mut ed = editor_from("hello -[w]>orld\nfoo\nbar\n");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "-[hello world\n]>foo\nbar\n", "line 1 selected");
    ed.handle_key(key_ctrl('x'));
    assert_eq!(state(&ed), "-[hello world\nfoo\n]>bar\n", "lines 1-2");
    ed.handle_key(key_ctrl('x'));
    assert_eq!(state(&ed), "-[hello world\nfoo\nbar\n]>", "lines 1-3");
}

/// Ctrl+X accumulates lines upward (extend behavior).
#[test]
fn ctrl_shift_x_extends_selection_up() {
    let mut ed = editor_from("aaa\nbbb\nhello -[w]>orld\n");
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "aaa\nbbb\n<[hello world\n]-", "line 3 selected");
    ed.handle_key(key_ctrl('X'));
    assert_eq!(state(&ed), "aaa\n<[bbb\nhello world\n]-", "lines 2-3");
    ed.handle_key(key_ctrl('X'));
    assert_eq!(state(&ed), "<[aaa\nbbb\nhello world\n]-", "lines 1-3");
}

/// `x` (forward line) then `X` (backward line): flips direction, stays on same line
/// when already at the first line (no line to jump back to).
#[test]
fn x_then_shift_x_flips_direction() {
    let mut ed = editor_from("hello -[w]>orld\nfoo\n");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "-[hello world\n]>foo\n");
    // sel.start() == line_start AND top_line == 0 → can't jump, just flips to backward.
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "<[hello world\n]-foo\n");
}

/// `X` (backward line) then `x` (forward line): jumps to next line (flips direction).
#[test]
fn shift_x_then_x_flips_direction() {
    let mut ed = editor_from("aaa\nhello -[w]>orld\nfoo\n");
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "aaa\n<[hello world\n]-foo\n");
    // sel.end() is at `\n` of line 1 → x jumps to next line (forward selection).
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "aaa\nhello world\n-[foo\n]>");
}

/// Ctrl+x after `X` (backward selection): extends forward, flipping direction.
#[test]
fn ctrl_x_after_shift_x() {
    // Cursor mid-line so `X` selects the current line (doesn't jump back).
    let mut ed = editor_from("aaa\nfoo -[b]>ar\nbaz\n");
    ed.handle_key(key('X'));
    assert_eq!(state(&ed), "aaa\n<[foo bar\n]-baz\n");
    // Ctrl+x extends forward (adds next line, switches to forward selection).
    ed.handle_key(key_ctrl('x'));
    assert_eq!(state(&ed), "aaa\n-[foo bar\nbaz\n]>");
}

/// Ctrl+X after `x` (forward selection): extends backward, flipping direction.
#[test]
fn ctrl_shift_x_after_x() {
    let mut ed = editor_from("aaa\nbbb\n-[f]>oo\n");
    ed.handle_key(key('x'));
    assert_eq!(state(&ed), "aaa\nbbb\n-[foo\n]>");
    // Ctrl+X extends backward (adds previous line, switches to backward selection).
    ed.handle_key(key_ctrl('X'));
    assert_eq!(state(&ed), "aaa\n<[bbb\nfoo\n]-");
}

// ── `o` / `O` open-line variants ──────────────────────────────────────────

/// `o` must insert a blank line *below* the current line, position the cursor
/// on it, and enter Insert mode — all as a single composed operation.
#[test]
fn o_opens_line_below_and_enters_insert() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('o'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "hello\n\n");
    // Cursor should be on the new blank line (the second '\n').
    assert_eq!(state(&ed), "hello\n-[\n]>");
}

/// `o` on a blank line must open a new blank line *below* it, not overshoot
/// into the line after.
/// Regression: `goto_line_end + move_right` advanced past the `\n` on empty
/// lines, inserting the new `\n` one line too low.
#[test]
fn o_on_empty_line_places_cursor_on_new_blank_line() {
    let mut ed = editor_from("AAA\nBBB\n-[\n]>CCC\n");
    ed.handle_key(key('o'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "AAA\nBBB\n\n\nCCC\n");
    assert_eq!(state(&ed), "AAA\nBBB\n\n-[\n]>CCC\n");
}

/// `O` must insert a blank line *above* the current line, position the cursor
/// on it, and enter Insert mode.
#[test]
fn capital_o_opens_line_above_and_enters_insert() {
    let mut ed = editor_from("foo\n-[b]>ar\n");
    ed.handle_key(key('O'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "foo\n\nbar\n");
    // Cursor on the new blank line between "foo" and "bar".
    assert_eq!(state(&ed), "foo\n-[\n]>bar\n");
}

// ── Insert-entry variants position the cursor correctly ────────────────────

/// `a` collapses to one past the end of the selection and enters Insert mode.
/// On a collapsed cursor this is identical to a plain "append after cursor".
#[test]
fn a_enters_insert_after_selection_end() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('a'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(state(&ed), "h-[e]>llo\n");
}

/// `A` must jump to the end of the line and then step one right (onto the
/// newline), then enter Insert mode — "append at end of line".
#[test]
fn capital_a_enters_insert_after_end_of_line() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('A'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(state(&ed), "hello-[\n]>");
}

/// `I` jumps to the first non-blank character on the line and enters Insert mode.
#[test]
fn capital_i_enters_insert_at_line_start() {
    let mut ed = editor_from("  -[hello]>\n");
    ed.handle_key(key('I'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(state(&ed), "  -[h]>ello\n");
}

/// `i` on a multi-char selection collapses to the selection start (not just the
/// cursor head) and enters Insert mode.
#[test]
fn i_on_wide_selection_collapses_to_start() {
    // Backward selection: head=0 (h), anchor=3 (last l) → start=0.
    let mut ed = editor_from("<[hell]-o\n");
    ed.handle_key(key('i'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(state(&ed), "-[h]>ello\n");
}

/// `a` on a multi-char selection collapses to one past the selection end and
/// enters Insert mode — the cursor lands after the last selected character.
#[test]
fn a_on_wide_selection_collapses_after_end() {
    // Forward selection: anchor=0 (h), head=3 (l) → end=3, one past = 4.
    let mut ed = editor_from("-[hel]>lo\n");
    ed.handle_key(key('a'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(state(&ed), "hel-[l]>o\n");
}

// ── `a` / `A` step-back on Esc ────────────────────────────────────────────────

/// After `a` + typing + Esc the cursor must land on the last typed character,
/// not one position past it. A second `a` should re-enter Insert at the same
/// spot rather than advancing further.
#[test]
fn a_esc_steps_cursor_back_to_last_typed_char() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.handle_key(key('a')); // cursor → 'e', Insert
    ed.handle_key(key('X'));
    ed.handle_key(key_esc());

    // Cursor must be on 'X', not on 'e' (one past where X was inserted).
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), "h-[X]>ello\n");
}

/// Regression: `$ a <text> Esc a` must not jump to the next line.
/// After Esc the cursor must sit on the last appended character (on the same
/// line), so that a second `a` re-enters Insert at the end of that line.
#[test]
fn a_esc_at_end_of_line_does_not_advance_to_next_line() {
    let mut ed = editor_from("-[h]>ello\nworld\n");

    ed.handle_key(key('A')); // jump to end of line → '\n', Insert
    ed.handle_key(key('X'));
    ed.handle_key(key_esc());

    // Cursor on 'X' (last appended char), still on line 1.
    assert_eq!(state(&ed), "hello-[X]>\nworld\n");

    // A second `a` must re-enter Insert on the same line, not on 'w'.
    ed.handle_key(key('a'));
    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(state(&ed), "helloX-[\n]>world\n");
}

/// `i` must NOT step the cursor back on Esc — only `a`/`A` do.
#[test]
fn i_esc_does_not_step_cursor_back() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.handle_key(key('i')); // cursor stays on 'h', Insert
    ed.handle_key(key('X'));
    ed.handle_key(key_esc());

    // No step-back: cursor stays on 'h', not on 'X'.
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), "X-[h]>ello\n");
}

// ── `o` / `O` step-back on Esc ───────────────────────────────────────────────

/// After `o` + typing + Esc the cursor must land on the last typed character
/// (same as `a`), not on the trailing `\n` of the new line.
///
/// Regression: without `mark_insert_step_back`, pressing `x` after `o+text+Esc`
/// selected the *next* line rather than the just-created one.
#[test]
fn o_esc_steps_cursor_back_to_last_typed_char() {
    let mut ed = editor_from("-[h]>ello\nworld\n");

    ed.handle_key(key('o'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key('c'));
    ed.handle_key(key_esc());

    // Cursor on 'c', not on the new line's trailing '\n'.
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), "hello\nab-[c]>\nworld\n");
}

/// After `O` + typing + Esc the cursor must land on the last typed character.
#[test]
fn capital_o_esc_steps_cursor_back_to_last_typed_char() {
    let mut ed = editor_from("hello\n-[w]>orld\n");

    ed.handle_key(key('O'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key('c'));
    ed.handle_key(key_esc());

    // Cursor on 'c', not on the new line's trailing '\n'.
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), "hello\nab-[c]>\nworld\n");
}

/// `o` + immediate Esc (nothing typed): cursor must stay on the new blank
/// line's `\n` and must NOT step back onto the preceding line.
#[test]
fn o_esc_on_empty_line_does_not_step_to_previous_line() {
    let mut ed = editor_from("-[h]>ello\nworld\n");

    ed.handle_key(key('o'));
    ed.handle_key(key_esc());

    // New blank line inserted; cursor on its '\n' (head == line_start so no
    // step-back occurs — the empty-line guard in end_insert_session applies).
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), "hello\n-[\n]>world\n");
}

// ── multi-cursor `a` collision (merge edge cases) ─────────────────────────────

/// `a` on two cursors where one sits on the last character and the other on the
/// trailing `\n`: both land on the `\n` and must merge to one cursor.
///
/// - cursor on c(2): char_at(2)='c' → `next(2)=3`.
/// - cursor on \n(3): char_at(3)='\n' → stays at 3.
///
/// Both land on 3 → merge → single cursor on \n.
///
/// Regression: without `map` merging after the transform, this leaves two
/// identical collapsed selections — a `SelectionSet` invariant violation.
#[test]
fn a_multi_cursor_clamp_collision_merges_to_one() {
    let mut ed = editor_from("ab-[c]>-[\n]>");
    ed.handle_key(key('a'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(state(&ed), "abc-[\n]>");
}

/// `a Esc` on two cursors where one is on a `\n` and the other on a content char:
/// the `\n`-cursor stays put (on the `\n`) and the content-char cursor advances
/// one grapheme. No collision — they end up on distinct lines after step-back.
#[test]
fn a_esc_newline_cursor_stays_on_its_line() {
    // "ab\ncd\n": a=0 b=1 \n=2 c=3 d=4 \n=5.
    // `a`: \n(2) → stays 2 (it is a \n); c(3) → next(3)=4. Cursors at 2, 4.
    // Esc step-back: head=2, line_start=0, 2>0 → prev(2)=1 (b).
    //                head=4, line_start=3, 4>3 → prev(4)=3 (c).
    let mut ed = editor_from("ab-[\n]>-[c]>d\n");
    ed.handle_key(key('a'));
    ed.handle_key(key_esc());

    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), "a-[b]>\n-[c]>d\n");
}

// ── `a` / `A` trailing-newline content rule ────────────────────────────────────

/// `a` on an empty line must stay on that line (≡ `i`) — not jump to the next.
///
/// An empty line is just a `\n`; the selection ends on that `\n`. Under the
/// content rule `a` does not step past a trailing `\n`.
#[test]
fn a_on_empty_line_stays_on_same_line() {
    // Buffer: "foo\n\nbar\n" — the middle line is empty (char index 4 = '\n').
    let mut ed = editor_from("foo\n-[\n]>bar\n");
    ed.handle_key(key('a'));

    assert_eq!(ed.state.mode, Mode::Insert);
    // Cursor must remain on the \n at position 4, not jump to 'b'.
    assert_eq!(state(&ed), "foo\n-[\n]>bar\n");
}

/// `a` after `x` (select-line) on an interior non-last line must place the
/// cursor on the line's trailing `\n`, not at the start of the next line.
#[test]
fn a_after_select_line_stays_on_same_line() {
    // select-line on 'b' → anchor=4 ('b'), head=7 ('\n').
    // `a`: sel.end()=7, char_at(7)='\n' → stay at 7.
    let mut ed = editor_from("foo\n-[b]>ar\nbaz\n");
    ed.handle_key(key('x')); // select "bar\n" — head on '\n'
    ed.handle_key(key('a'));

    assert_eq!(ed.state.mode, Mode::Insert);
    // Cursor on the trailing '\n' of the line — same line, not on 'b' of next line.
    assert_eq!(state(&ed), "foo\nbar-[\n]>baz\n");
}

/// `A` on an empty line must stay on the `\n` of that line — not step onto
/// the next line. An unconditional `move_right` after `goto_line_end` would
/// advance past the `\n` on empty lines.
#[test]
fn capital_a_on_empty_line_stays_on_same_line() {
    let mut ed = editor_from("foo\n-[\n]>bar\n");
    ed.handle_key(key('A'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(state(&ed), "foo\n-[\n]>bar\n");
}

/// `A` on a non-empty line must still position after the last content character
/// (on the trailing `\n` slot).
#[test]
fn capital_a_on_nonempty_line_is_unchanged() {
    let mut ed = editor_from("-[h]>ello\nworld\n");
    ed.handle_key(key('A'));

    assert_eq!(ed.state.mode, Mode::Insert);
    // Cursor on the \n at position 5 (between "hello" and "world").
    assert_eq!(state(&ed), "hello-[\n]>world\n");
}

// ── `c` trailing-newline content rule ─────────────────────────────────────────

/// `c` on an empty line must not delete anything — the line stays, cursor stays.
/// Equivalent to pressing `i` on an empty line.
#[test]
fn change_on_empty_line_is_noop() {
    let mut ed = editor_from("foo\n-[\n]>bar\n");
    ed.handle_key(key('c'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(
        ed.doc().text().to_string(),
        "foo\n\nbar\n",
        "empty line must survive"
    );
    // Cursor on the \n (empty line).
    assert_eq!(state(&ed), "foo\n-[\n]>bar\n");
}

/// `c` after `x` (select-line) on an interior line clears the content but keeps
/// the line — `c` rewrites a line, not deletes it.
#[test]
fn change_after_select_line_keeps_line() {
    let mut ed = editor_from("foo\n-[b]>ar\nbaz\n");
    ed.handle_key(key('x')); // selects "bar\n" (head on \n)
    ed.handle_key(key('c'));

    assert_eq!(ed.state.mode, Mode::Insert);
    // "bar" deleted, \n kept → line 1 is now empty; cursor at line start.
    assert_eq!(
        ed.doc().text().to_string(),
        "foo\n\nbaz\n",
        "line must be kept"
    );
    assert_eq!(state(&ed), "foo\n-[\n]>baz\n");
}

/// Multi-line `c`: interior `\n`s are deleted normally; only the final `\n` is
/// kept, collapsing the selection to a single empty line.
#[test]
fn change_multi_line_collapses_to_one_empty_line() {
    // Selection covers "bar\nbaz\n" (anchor=4, head=11 on the last '\n').
    // change_span: sel.end()=11, char_at(11)='\n' → stop=11.
    // Deletes chars 4..11 = "bar\nbaz". Buffer → "foo\n\n".
    let mut ed = editor_from("foo\n-[bar\nbaz\n]>");
    ed.handle_key(key('c'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(
        ed.doc().text().to_string(),
        "foo\n\n",
        "two lines become one empty line"
    );
    assert_eq!(state(&ed), "foo\n-[\n]>");
}

/// `c` on a plain (non-`\n`) char still deletes it — regression guard.
#[test]
fn change_on_content_char_still_deletes() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('c'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "ello\n");
    assert_eq!(state(&ed), "-[e]>llo\n");
}

/// After `c` of a line-selected region, the kill ring must contain only the
/// line content — no trailing `\n`.
#[test]
fn change_kill_ring_excludes_trailing_newline() {
    let mut ed = editor_from("-[b]>ar\n");
    ed.handle_key(key('x')); // select "bar\n" (head on \n)
    ed.handle_key(key('c'));

    assert_eq!(
        ed.state.kill_ring.head(),
        Some(["bar".to_string()].as_slice()),
        "kill ring must hold content only, no trailing newline"
    );
}

/// `d` after `x` (select-line) still removes the whole line including its `\n`
/// — regression guard ensuring `d` was not affected by the `c`-only change.
#[test]
fn d_after_select_line_removes_entire_line() {
    let mut ed = editor_from("foo\n-[b]>ar\nbaz\n");
    ed.handle_key(key('x')); // selects "bar\n" (head on \n)
    ed.handle_key(key('d'));

    assert_eq!(
        ed.doc().text().to_string(),
        "foo\nbaz\n",
        "whole line including \\n must be deleted"
    );
    assert_eq!(
        ed.state.kill_ring.head(),
        Some(["bar\n".to_string()].as_slice()),
        "kill ring holds full line including \\n"
    );
}

// ── `d` / `xd` on blank last line ────────────────────────────────────────────

/// `d` on a blank last line must delete it, not silently no-op.
///
/// A blank last line is a collapsed cursor on the structural trailing `\n`.
/// Before the fix, `delete_one_grapheme` would no-op because the cursor is
/// already on the structural `\n`. After the fix it routes through
/// `delete_sel_region`'s merge path, consuming the preceding `\n`.
#[test]
fn d_on_blank_last_line_removes_it() {
    let mut ed = editor_from("foo\n-[\n]>");
    ed.handle_key(key('d'));

    assert_eq!(
        state(&ed),
        "-[f]>oo\n",
        "blank last line must be removed, cursor on first line"
    );
    assert_eq!(
        ed.state.kill_ring.head(),
        Some(["\n".to_string()].as_slice()),
        "kill ring holds the blank line"
    );
}

/// `x` on a blank last line leaves a collapsed selection (existing behaviour);
/// the subsequent `d` must still remove the blank line.
#[test]
fn xd_on_blank_last_line_removes_it() {
    let mut ed = editor_from("foo\n-[\n]>");
    ed.handle_key(key('x')); // collapsed selection stays on structural '\n'
    ed.handle_key(key('d'));

    assert_eq!(
        state(&ed),
        "-[f]>oo\n",
        "xd on blank last line must remove it"
    );
}

/// Reported regression: file ends in two blank lines; `d` on the last one
/// must remove exactly one blank line, leaving the other intact.
#[test]
fn d_on_last_of_two_blank_lines_removes_one() {
    let mut ed = editor_from("foo\n\n-[\n]>");
    ed.handle_key(key('d'));

    assert_eq!(
        state(&ed),
        "foo\n-[\n]>",
        "one blank line removed, second (now last) blank line intact"
    );
}

// ── `S` splits selection on newlines ──────────────────────────────────────────

/// `S` must split a multi-line selection into one cursor per line, which is
/// the primary way to turn a block selection into a multi-cursor.
#[test]
fn capital_s_splits_selection_on_newlines() {
    let mut ed = editor_from("-[foo\nbar\nbaz]>\n");

    ed.handle_key(key('S'));

    assert_eq!(state(&ed), "-[foo]>\n-[bar]>\n-[baz]>\n");
}

// ── `ctrl+,` removes the primary selection ────────────────────────────────────

/// `ctrl+,` must drop the primary selection and promote one of the secondaries,
/// leaving all other cursors intact. Plain `,` must still keep only the primary.
#[test]
fn ctrl_comma_removes_primary_selection() {
    let mut ed = editor_from_kitty("-[h]>ello -[w]>orld\n");

    ed.handle_key(key_ctrl(','));

    // Primary ('h') is dropped; 'w' becomes the new (only) primary.
    assert_eq!(state(&ed), "hello -[w]>orld\n");
}

#[test]
fn plain_comma_still_keeps_primary_selection() {
    let mut ed = editor_from("-[h]>ello -[w]>orld\n");

    ed.handle_key(key(','));

    // Only the primary ('h') survives.
    assert_eq!(state(&ed), "-[h]>ello world\n");
}

// ── `o` in extend mode ─────────────────────────────────────────────────────────

/// The default Extend override trie is empty, so `o` in Extend mode falls
/// through to the Normal trie (with extend=true) like any other unbound-in-Extend
/// key — same as `o` in Normal mode, `open-line-below`. The vim-style flip
/// alias lives only in `core:vim-keybind` (see `tests/vim_keybind.rs`).
#[test]
fn o_in_extend_mode_falls_through_to_open_line_below() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.mode = Mode::Extend;

    ed.handle_key(key('o'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "hello\n\n");
}

#[test]
fn o_in_normal_mode_still_opens_line_below() {
    let mut ed = editor_from("-[h]>ello\n");
    // extend is off (default).

    ed.handle_key(key('o'));

    assert_eq!(ed.state.mode, Mode::Insert);
    assert_eq!(ed.doc().text().to_string(), "hello\n\n");
}

// ── `Ctrl+e` flips the selection in Normal AND Extend mode ───────────────────

/// `Ctrl+e` in Normal mode must swap anchor and head. This works on legacy
/// terminals because `Ctrl+e` emits 0x05.
#[test]
fn ctrl_e_in_normal_mode_flips_selection() {
    let mut ed = editor_from("-[hell]>o\n");
    // Normal mode (the default) — no Extend active.

    ed.handle_key(key_ctrl('e'));

    // anchor and head are swapped; selection is now backward.
    assert_eq!(state(&ed), "<[hell]-o\n");
    // Normal mode stays; flip does not enter or exit Extend.
    assert_eq!(ed.state.mode, Mode::Normal);
}

/// `Ctrl+e` in Extend mode also flips (it falls through to the Normal trie with
/// extend=true; `cmd_flip_selections` ignores MotionMode). Extend mode must
/// remain active after the flip.
#[test]
fn ctrl_e_in_extend_mode_flips_selection() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.mode = Mode::Extend;

    ed.handle_key(key_ctrl('e'));

    assert_eq!(state(&ed), "<[hell]-o\n");
    assert_eq!(ed.state.mode, Mode::Extend);
}

// ── `;` collapses selection AND clears extend mode ─────────────────────────

/// `;` must (a) collapse every selection to its head and (b) clear the
/// `extend` flag. The extend side-effect only exists in the mapping — a pure
/// `cmd_collapse_selection_to_head` test cannot see it.
#[test]
fn semicolon_collapses_selection_and_resets_extend() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.mode = Mode::Extend;

    ed.handle_key(key(';'));

    assert_eq!(ed.state.mode, Mode::Normal, "extend cleared by ';'");
    // head of the original selection was 'l' (last char of "hell").
    assert_eq!(state(&ed), "hel-[l]>o\n");
}

// ── `Ctrl+;` collapses selection to anchor AND clears extend mode ─────────────

/// `Ctrl+;` must (a) collapse every selection to its anchor and (b) clear the
/// `extend` flag — the exact mirror of `;` with `head` replaced by `anchor`.
#[test]
fn ctrl_semicolon_collapses_to_anchor_and_resets_extend() {
    let mut ed = editor_from_kitty("-[hell]>o\n");
    ed.state.mode = Mode::Extend;

    ed.handle_key(key_ctrl(';'));

    assert_eq!(ed.state.mode, Mode::Normal, "extend cleared by 'Ctrl+;'");
    // anchor of the original selection was 'h' (offset 0).
    assert_eq!(state(&ed), "-[h]>ello\n");
}

// ── Extend mode exits after selection-consuming edits ────────────────────────
//
// Mirrors Vim visual-mode: any operator on a visual selection returns to Normal.
// Yank is the deliberate exception — it is non-destructive and preserves the
// selection (Helix-like).

#[test]
fn extend_exits_after_delete() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.mode = Mode::Extend;
    ed.handle_key(key('d'));
    assert_eq!(ed.state.mode, Mode::Normal, "delete exits Extend");
}

#[test]
fn extend_exits_after_replace() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.mode = Mode::Extend;
    ed.handle_key(key('r'));
    ed.handle_key(key('x')); // replacement char completes replace
    assert_eq!(ed.state.mode, Mode::Normal, "replace exits Extend");
}

#[test]
fn extend_exits_after_paste() {
    // Pre-populate a register so paste does real work.
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('y')); // yank "h" into ring
    ed.state.mode = Mode::Extend;
    ed.handle_key(key('p')); // paste-after
    assert_eq!(ed.state.mode, Mode::Normal, "paste exits Extend");
}

#[test]
fn extend_preserved_after_yank() {
    // Yank must NOT exit Extend — it is non-destructive and the selection stays live.
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.mode = Mode::Extend;
    ed.handle_key(key('y'));
    assert_eq!(ed.state.mode, Mode::Extend, "yank preserves Extend");
}

// ── `o`/`O` undo grouping ─────────────────────────────────────────────────────

/// `o` must group the structural newline insertion and the subsequent insert
/// session into one undo step. Without the fix, the newline would be a
/// separate `apply_edit` revision, so `u` would only undo the typed text and
/// leave behind an empty line.
#[test]
fn o_groups_newline_and_insert_session_into_one_undo_step() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.handle_key(key('o'));
    assert_eq!(ed.state.mode, Mode::Insert);

    ed.handle_key(key('w'));
    ed.handle_key(key('o'));
    ed.handle_key(key('r'));
    ed.handle_key(key('l'));
    ed.handle_key(key('d'));

    ed.handle_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "hello\nworld\n");

    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "-[h]>ello\n");
    assert!(!ed.doc().can_undo());
}

/// Same undo-grouping invariant for `O` (open line above).
#[test]
fn capital_o_groups_newline_and_insert_session_into_one_undo_step() {
    let mut ed = editor_from("foo\n-[b]>ar\n");

    ed.handle_key(key('O'));
    assert_eq!(ed.state.mode, Mode::Insert);

    ed.handle_key(key('n'));
    ed.handle_key(key('e'));
    ed.handle_key(key('w'));

    ed.handle_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "foo\nnew\nbar\n");

    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "foo\n-[b]>ar\n");
    assert!(!ed.doc().can_undo());
}

// ── Plain insert session groups all chars into one undo step ──────────────

/// `i` with a non-collapsed selection must collapse to the start of the
/// selection and enter Insert — it must NOT replace the selected text.
#[test]
fn i_collapses_selection_to_start() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('i'));

    assert_eq!(ed.state.mode, Mode::Insert);
    // Cursor collapsed to 'h' — nothing deleted.
    assert_eq!(state(&ed), "-[h]>ello\n");
    assert_eq!(ed.doc().text().to_string(), "hello\n");
}

/// `i` + typing + `Esc` must commit as one undo step, just like `c`. A single
/// `u` should restore the original buffer — not leave partial edits behind.
#[test]
fn i_groups_insert_session_into_one_undo_step() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.handle_key(key('i'));
    assert_eq!(ed.state.mode, Mode::Insert);

    ed.handle_key(key('X'));
    ed.handle_key(key('Y'));

    ed.handle_key(key_esc());
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(ed.doc().text().to_string(), "XYhello\n");

    // One undo restores the original state completely.
    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "-[h]>ello\n");

    // Only one revision was recorded.
    assert!(!ed.doc().can_undo());
}

// ── Line text objects (mil / mal) ─────────────────────────────────────────────

#[test]
fn mil_selects_line_content_excluding_newline() {
    let mut ed = editor_from("hell-[o]> world\nsecond\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('i'));
    ed.handle_key(key('l'));
    assert_eq!(state(&ed), "-[hello world]>\nsecond\n");
}

#[test]
fn mal_selects_line_including_newline() {
    let mut ed = editor_from("hell-[o]> world\nsecond\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('a'));
    ed.handle_key(key('l'));
    assert_eq!(state(&ed), "-[hello world\n]>second\n");
}

#[test]
fn mil_on_empty_line_is_noop() {
    // An empty line has no content — selection should not change.
    let mut ed = editor_from("foo\n-[\n]>bar\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('i'));
    ed.handle_key(key('l'));
    assert_eq!(state(&ed), "foo\n-[\n]>bar\n");
}

// ── Register prefix `"<reg>` ────────────────────────────────────────────────

/// `"5y` must write text into register '5', leaving `'"'` empty.
#[test]
fn register_prefix_routes_yank_to_named_register() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.handle_key(key('y'));

    assert_eq!(state(&ed), "-[hell]>o\n", "buffer unchanged");
    assert_eq!(reg(&ed, '5'), &["hell"], "register '5' populated");
    assert!(reg(&ed, '"').is_empty(), "'\"' register untouched");
}

/// After `"5y`, the prefix is consumed. The next bare `y` writes to clipboard
/// and the kill ring (not to register '5').
#[test]
fn register_prefix_clears_after_one_operation() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.handle_key(key('y'));

    // Now the prefix is cleared — move right to get a different selection,
    // then yank again without a prefix.
    ed.handle_key(key('l')); // move right
    ed.handle_key(key('y')); // bare yank — writes clipboard + kill ring

    // The second yank updated the clipboard, not register '5'.
    assert!(
        !reg(&ed, CLIPBOARD_REGISTER).is_empty(),
        "clipboard written by bare y"
    );
    // Kill ring head holds the latest bare yank.
    assert!(
        ed.state.kill_ring.head().is_some(),
        "kill ring head set by bare y"
    );
    // '5' is unchanged from the first yank.
    assert_eq!(reg(&ed, '5'), &["hell"], "register '5' unchanged");
}

/// `Esc` after `"` cancels the prefix — the next `y` writes to clipboard + ring.
#[test]
fn esc_cancels_register_prefix() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key_esc()); // cancel
    ed.handle_key(key('y'));

    assert_eq!(
        reg(&ed, CLIPBOARD_REGISTER),
        &["hell"],
        "clipboard populated"
    );
    assert_eq!(
        ed.state.kill_ring.head(),
        Some(["hell".to_string()].as_slice()),
        "kill ring head populated"
    );
    assert!(reg(&ed, '5').is_empty(), "register '5' untouched");
}

/// `"3y` then `"3p` must round-trip through in-memory register '3'.
/// Digit registers are symmetric: yank writes RegisterSet['3'], paste reads it.
#[test]
fn paste_from_named_register() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]>world\n");

    // "3y: yank "hello" into in-memory register '3'.
    ed.handle_key(key('"'));
    ed.handle_key(key('3'));
    ed.handle_key(key('y'));

    assert_eq!(reg(&ed, '3'), &["hello"], "register '3' populated by yank");

    // Move to a fresh position to make the paste visible.
    ed.handle_key(key('w')); // move word → selection on 'w' of "world"

    // Seed clipboard with "wrong" to verify "3p doesn't read it.
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["wrong".to_string()]);

    ed.handle_key(key('"'));
    ed.handle_key(key('3'));
    ed.handle_key(key('p')); // "3p → in-memory register '3' = "hello"

    assert!(
        ed.doc().text().to_string().contains("hello"),
        "pasted from in-memory register '3'"
    );
    assert!(
        !ed.doc().text().to_string().contains("wrong"),
        "clipboard not used by \"3p"
    );
}

/// `d` pushes to the kill ring; `"3p` reads in-memory register '3' (empty),
/// NOT ring slot 3. Digit registers are decoupled from the ring.
#[test]
fn digit_register_decoupled_from_kill_ring() {
    // Push 4 deletes so ring slot 3 = "P" (oldest).
    let mut ed = editor_from("-[P]>QRS\n");
    for _ in 0..4 {
        ed.handle_key(key('d'));
    }
    // ring: slot 0 = "S", slot 1 = "R", slot 2 = "Q", slot 3 = "P"
    // in-memory register '3' is empty (nothing yanked into it).
    assert!(
        reg(&ed, '3').is_empty(),
        "register '3' is empty — d never writes named registers"
    );

    // "3p reads in-memory register '3' which is empty → paste is a no-op.
    let text_before = ed.doc().text().to_string();
    ed.handle_key(key('"'));
    ed.handle_key(key('3'));
    ed.handle_key(key('p'));

    assert_eq!(
        ed.doc().text().to_string(),
        text_before,
        "\"3p is a no-op when register '3' is empty (not ring slot 3)"
    );
}

/// `"by` discards the yank — `'"'` must remain empty.
#[test]
fn black_hole_register_via_prefix() {
    use crate::ops::register::BLACK_HOLE_REGISTER;

    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key('b'));
    ed.handle_key(key('y'));

    assert_eq!(state(&ed), "-[hell]>o\n", "buffer unchanged");
    assert!(reg(&ed, '"').is_empty(), "'\"' register untouched");
    assert!(
        ed.state.registers.read(BLACK_HOLE_REGISTER).is_none(),
        "black hole register returns None"
    );
}

// ── Clipboard register fallback (in-memory mirror) ─────────────────────────

/// When the system clipboard is unavailable, `"cy` falls back to the in-memory
/// mirror and logs a Warning. The mirror is then used by `"cp`.
#[test]
fn clipboard_register_falls_back_to_memory_when_unavailable() {
    use crate::editor::Severity;
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]>\n");
    // Simulate a headless environment with no clipboard server.
    ed.state.clipboard.force_unavailable();

    ed.handle_key(key('"'));
    ed.handle_key(key('c'));
    ed.handle_key(key('y'));

    // A Warning must have been logged.
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Warning),
        "expected a Warning for clipboard unavailable"
    );

    // In-memory mirror must hold the yanked text.
    assert_eq!(
        reg(&ed, CLIPBOARD_REGISTER),
        &["hello"],
        "in-memory mirror populated"
    );

    // Move right so cursor is now on 'o', giving a distinct selection.
    ed.handle_key(key('l'));

    // `"cp` should read from the in-memory mirror and paste "hello".
    ed.handle_key(key('"'));
    ed.handle_key(key('c'));
    ed.handle_key(key('p'));

    assert!(
        ed.doc().text().to_string().contains("hello"),
        "pasted from in-memory mirror"
    );
}

// ── Kill-ring register (`"k`) ─────────────────────────────────────────────────

/// `"kp` must paste the kill-ring head, not the clipboard.
#[test]
fn kill_ring_register_pastes_ring_head() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]>world\n");
    ed.feed_key(key('d')); // delete "hello" → ring head = ["hello"]

    // Seed clipboard with "wrong" to confirm "kp doesn't read it.
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["wrong".to_string()]);

    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.feed_key(key('p'));

    assert!(
        ed.doc().text().to_string().contains("hello"),
        "\"kp pasted ring head"
    );
    assert!(
        !ed.doc().text().to_string().contains("wrong"),
        "clipboard not used by \"kp"
    );
}

/// `"kp` seeds the `[`/`]` cycle so pressing `[` after cycles to the older entry.
#[test]
fn kill_ring_register_paste_seeds_cycle() {
    // Build a ring with 2 entries: push "first" then "second" (head).
    let mut ed = editor_from("-[second]>X\n");
    ed.feed_key(key('d')); // ring head = ["second"]

    // Manually push an older entry.
    ed.state.kill_ring.push(vec!["first".to_string()]);
    // ring: head = ["first"] (newest push), slot 1 = ["second"]

    // "kp: paste ring head ("first"). This should open a paste session
    // seeded at the head so [ can cycle to the older entry.
    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.feed_key(key('p'));

    assert!(
        ed.doc().text().to_string().contains("first"),
        "\"kp pasted ring head"
    );

    // [ should cycle to the next-older entry ("second").
    ed.feed_key(key('['));
    assert!(
        ed.doc().text().to_string().contains("second"),
        "[ after \"kp cycled to older ring entry"
    );
}

/// `"ky` pushes the yank onto the kill ring and does NOT touch the clipboard.
#[test]
fn kill_ring_register_yank_pushes_ring_only() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]>world\n");
    // Ensure clipboard starts empty.
    assert!(
        reg(&ed, CLIPBOARD_REGISTER).is_empty(),
        "clipboard starts empty"
    );

    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.feed_key(key('y')); // "ky → ring push, no clipboard

    assert_eq!(
        ed.state.kill_ring.head(),
        Some(["hello".to_string()].as_slice()),
        "ring head set by \"ky"
    );
    assert!(
        reg(&ed, CLIPBOARD_REGISTER).is_empty(),
        "\"ky must not write the clipboard"
    );
}

/// `"kd` deletes and pushes to the ring, identical to bare `d`.
#[test]
fn kill_ring_register_delete_pushes_ring() {
    let mut ed = editor_from("-[hello]>world\n");

    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.feed_key(key('d')); // "kd → delete + push ring

    assert_eq!(
        ed.doc().text().to_string(),
        "world\n",
        "buffer after delete"
    );
    assert_eq!(
        ed.state.kill_ring.head(),
        Some(["hello".to_string()].as_slice()),
        "ring head set by \"kd"
    );
}

// ── surround-add (`mw`) ───────────────────────────────────────────────────────

#[test]
fn mw_wraps_with_bracket() {
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('['));
    assert_eq!(state(&ed), "[bar-[]]>\n");
}

#[test]
fn mw_wraps_with_brace_via_close_char() {
    // `mw}` should normalize to the pair `{` `}`.
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('}'));
    assert_eq!(state(&ed), "{bar-[}]>\n");
}

#[test]
fn mw_wraps_symmetric_quote() {
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('"'));
    assert_eq!(state(&ed), "\"bar-[\"]>\n");
}

#[test]
fn mw_wraps_unknown_char_symmetric() {
    // `*` is not a configured pair — wraps symmetrically open == close == `*`.
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('*'));
    assert_eq!(state(&ed), "*bar-[*]>\n");
}

#[test]
fn mw_wraps_multi_cursor() {
    let mut ed = editor_from("-[ab]>c-[de]>f\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('('));
    assert_eq!(state(&ed), "(ab-[)]>c(de-[)]>f\n");
}

#[test]
fn mw_wraps_cursor_one_char() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('['));
    assert_eq!(state(&ed), "[h-[]]>ello\n");
}

#[test]
fn mw_esc_cancels() {
    let mut ed = editor_from("-[bar]>\n");
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key_esc()); // cancel before typing the delimiter
    assert_eq!(state(&ed), "-[bar]>\n");
}

#[test]
fn mw_wraps_when_auto_pairs_disabled() {
    // surround-add uses the pairs table only as a lookup; it ignores the
    // auto-pairs-enabled flag. `mw[` must still wrap even when auto-pairs are off.
    let mut ed = editor_from("-[bar]>\n");
    ed.state.settings.auto_pairs_enabled = false;
    ed.handle_key(key('m'));
    ed.handle_key(key('w'));
    ed.handle_key(key('['));
    assert_eq!(state(&ed), "[bar-[]]>\n");
}

// ── Smart-p heuristic and kill ring ──────────────────────────────────────────

/// `d` then `p` reads from the kill ring (char-swap / dp pattern).
/// `last_command` after `d` is "delete" ∈ `SMART_P_LAST_CMDS`, so `p` reads ring.
#[test]
fn smart_p_dp_reads_ring() {
    // Buffer: "ab\n", cursor on 'a'.
    let mut ed = editor_from("-[a]>b\n");
    ed.feed_key(key('d')); // delete 'a' → ring = ["a"]
    // After delete: buffer = "b\n", cursor at 'b'.
    ed.feed_key(key('p')); // paste-after from ring → "ba\n"? No: paste-after on cursor 'b' inserts after 'b'.
    // Actually: after 'd', cursor is on 'b'. paste-after inserts "a" after 'b'. Buffer = "ba\n".
    assert!(
        ed.doc().text().to_string().contains('a'),
        "ring content pasted after delete"
    );
    // Clipboard is not written by bare 'd', so the pasted value came from ring.
    assert!(
        ed.state.kill_ring.head().is_some(),
        "kill ring still has an entry after paste"
    );
}

/// `c` <text> Esc then `p` reads the kill ring, not the clipboard.
///
/// Regression: `exit-insert` (Esc) ran through the dispatch pipeline and
/// overwrote `last_command = "exit-insert"` ∉ `SMART_P_LAST_CMDS`, so
/// smart-`p` fell through to the clipboard. Fix: `exit-insert` is registered
/// with `.transparent_to_last_command()`, setting `stamps_last_command = false`
/// on its `CmdMeta`.
///
/// Fail oracle: remove `.transparent_to_last_command()` from `exit-insert`'s
/// registration in `registry/defaults.rs` → `last_command` becomes "exit-insert"
/// → `p` pastes "CLIP" → `contains('a')` fails.
#[test]
fn smart_p_after_change_reads_ring() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[a]>b\n");
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('c')); // change 'a' → ring=["a"], enter Insert
    ed.feed_key(key('x')); // type replacement (doesn't touch last_command)
    ed.feed_key(key_esc()); // exit-insert — must NOT clobber last_command
    ed.feed_key(key('p')); // smart-p → must read ring head ("a"), not "CLIP"
    let text = ed.doc().text().to_string();
    assert!(
        text.contains('a'),
        "p after change must paste ring content ('a')"
    );
    assert!(
        !text.contains("CLIP"),
        "p after change must not paste clipboard"
    );
}

/// `exit-insert` must never overwrite `last_command`, regardless of what it held.
///
/// Directly pins the sole exception in the stamp mechanism: `exit-insert` is
/// registered with `stamps_last_command = false` (via `.transparent_to_last_command()`
/// in `registry/defaults.rs`), so `step_stamp_last_command` skips it.
///
/// Fail oracle: remove `.transparent_to_last_command()` from `exit-insert`'s
/// registration → `stamps_last_command` becomes `true` → marker becomes
/// `Some("exit-insert")`.
#[test]
fn exit_insert_does_not_stamp() {
    let mut ed = editor_from("-[a]>b\n");
    ed.feed_key(key('i')); // enter Insert (stamps "insert-at-selection-start")
    // Override last_command with a known kill marker — simulates a kill having
    // happened inside the insert session (e.g. via call! delete in Steel).
    ed.state.last_command = Some(std::borrow::Cow::Borrowed("delete"));
    ed.feed_key(key_esc()); // exit-insert — must NOT overwrite "delete"
    assert_eq!(
        ed.state.last_command.as_deref(),
        Some("delete"),
        "exit-insert must not stamp last_command",
    );
}

/// A native kill dispatched while in Insert mode stamps `last_command`.
///
/// Only `exit-insert` is exempt from stamping; all other commands — including
/// kills inside Insert — write their name. A future `Ctrl-w`-style command
/// (Steel body doing `call! delete`) therefore correctly informs smart-`p`.
///
/// Fail oracle: add a `stamps_last_command = false` check gated on Insert mode to
/// `step_stamp_last_command` → `last_command` stays `Some("insert-before")` and
/// the assertion fails.
#[test]
fn delete_in_insert_mode_stamps_marker() {
    let mut ed = editor_from("-[a]>b\n");
    ed.feed_key(key('i')); // enter Insert, last_command = Some("enter-insert")
    // Dispatch delete by name — 'd' in Insert self-inserts.
    ed.execute_keymap_command("delete".into(), Some(1), false, ArgSource::Keymap);
    assert_eq!(
        ed.state.last_command.as_deref(),
        Some("delete"),
        "native delete dispatched in Insert must stamp last_command",
    );
}

/// `c` <text> <Left> Esc `p` reads the clipboard, not the ring.
///
/// An arrow key in Insert mode stamps `"move-left"` ∉ `SMART_P_LAST_CMDS`,
/// resetting smart-p to clipboard — consistent with Normal-mode motion
/// behavior (`d j p` → clipboard).
#[test]
fn smart_p_insert_motion_resets_to_clipboard() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[a]>b\n");
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('c')); // change 'a' → ring=["a"], enter Insert
    ed.feed_key(key('x')); // type replacement
    ed.feed_key(key_left()); // move-left in Insert → stamps "move-left"
    ed.feed_key(key_esc()); // exit-insert — transparent
    ed.feed_key(key('p')); // smart-p → must read clipboard ("CLIP")
    let text = ed.doc().text().to_string();
    assert!(
        text.contains("CLIP"),
        "motion in Insert resets smart-p to clipboard"
    );
    assert!(
        !text.contains('a'),
        "ring head must not be pasted after insert motion"
    );
}

/// `d` then `j` (motion) then `p` reads from clipboard, not ring.
/// Motion is NOT in `SMART_P_LAST_CMDS`, so `p` falls back to clipboard.
#[test]
fn smart_p_motion_resets_to_clipboard() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    // Two-line buffer; cursor on line 0.
    let mut ed = editor_from("-[a]>b\ncd\n");
    // Seed clipboard with something distinct from what 'd' would yank.
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('d')); // delete 'a' → ring = ["a"]
    ed.feed_key(key('j')); // move-down → last_command = "move-down" ∉ SMART_P_LAST_CMDS
    ed.feed_key(key('p')); // paste-after → must read clipboard ("CLIP")
    assert!(
        ed.doc().text().to_string().contains("CLIP"),
        "p after motion reads clipboard"
    );
}

/// Bare `y` writes to both the clipboard AND the kill ring.
/// A subsequent `p` (no preceding `c`/`d`) reads from the clipboard.
#[test]
fn smart_p_after_yank_reads_clipboard() {
    const KILL_CMDS: &[&str] = &["change", "delete"];

    let mut ed = editor_from("-[hello]> world\n");
    ed.feed_key(key('y')); // yank → clipboard="hello" + ring="hello"

    // Verify yank did not set last_command to anything in the kill set.
    assert!(
        !ed.state
            .last_command
            .as_deref()
            .is_some_and(|c| KILL_CMDS.contains(&c)),
        "last_command after bare y is a kill command"
    );

    // Push a distinct value to ring so ring-head ≠ clipboard.
    // Now: clipboard="hello", ring head="RING".
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    // Move right and paste — should read clipboard ("hello"), not ring head ("RING").
    ed.feed_key(key('l'));
    ed.feed_key(key('p'));

    let buf = ed.doc().text().to_string();
    assert!(buf.contains("hello"), "p after y reads clipboard");
    assert!(!buf.contains("RING"), "ring head must not be used after y");
}

/// Consecutive `p p` after `d` keeps reading the ring (last_command stays in set).
#[test]
fn smart_p_consecutive_paste_stays_in_ring() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[X]>abc\n");
    // Seed clipboard with something distinct.
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('d')); // delete 'X' → ring = ["X"]
    ed.feed_key(key('p')); // first paste → from ring, last_command = "paste-after"
    // last_command = "paste-after", is_paste = true → is_append = true → appends from last_paste.
    ed.feed_key(key('p')); // second paste → still from ring
    // Buffer should contain "X" twice (pasted) and NOT "CLIP".
    let buf = ed.doc().text().to_string();
    assert!(buf.contains("X"), "ring entry appears in buffer");
    assert!(
        !buf.contains("CLIP"),
        "second consecutive p still reads ring"
    );
}

/// `x d p` pastes the kill-ring head, not the clipboard.
///
/// `last_command = "delete"` is in `SMART_P_LAST_CMDS`, so bare `p` reads the
/// ring even when the clipboard holds different content.
#[test]
fn xdp_pastes_ring_head_not_clipboard() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[A]>\nB\n");
    ed.state.clipboard.force_unavailable();
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);

    ed.feed_key(key('x')); // select "A\n"
    ed.feed_key(key('d')); // delete → ring = ["A\n"], last_command = "delete"
    ed.feed_key(key('p')); // prefer_ring = true → ring head

    assert_eq!(
        state(&ed),
        "B\n-[A\n]>",
        "xdp must paste the deleted line (ring head), not the clipboard sentinel"
    );
}

/// Regression: `drain_replay_queue` ran unconditionally after every key, setting
/// `last_command = None` even when the queue was empty. A bare `p` after `x d`
/// must still read the ring head — the idle drain must not neutralize `last_command`
/// (pre-432c24f bug: pasted the clipboard instead). `feed_key` / `feed_keys` include
/// the idle drain so this invariant is checked automatically by all paste tests now.
#[test]
fn smart_p_survives_idle_replay_drain() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[A]>\nB\n");
    ed.state.clipboard.force_unavailable();
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);

    ed.feed_keys([key('x'), key('d'), key('p')]);

    assert_eq!(
        state(&ed),
        "B\n-[A\n]>",
        "idle replay-queue drain must not reset last_command; p reads the ring head"
    );
}

/// Kill ring depth: after >10 pushes (via `d`), `len() == 10` and the oldest entry
/// is evicted.  The 11th push displaces the 1st.
#[test]
fn kill_ring_depth_capped_at_ten() {
    // 11 one-char lines: A through K.
    let mut ed = editor_from("-[A]>\nB\nC\nD\nE\nF\nG\nH\nI\nJ\nK\n");
    // Delete each line by repeatedly pressing x then d.
    for _ in 0..11 {
        ed.feed_key(key('x')); // select-line
        ed.feed_key(key('d')); // delete line → push ring
        // After delete, cursor lands on next line automatically.
    }
    assert_eq!(ed.state.kill_ring.len(), 10, "kill ring capped at depth 10");
}

/// `"cy` writes clipboard only — no kill-ring push.
#[test]
fn explicit_cy_writes_clipboard_only() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[hello]>\n");
    // Kill the ring beforehand so we can detect any erroneous push.
    ed.feed_key(key('"'));
    ed.feed_key(key('c'));
    ed.feed_key(key('y')); // "cy → clipboard only

    assert_eq!(
        reg(&ed, CLIPBOARD_REGISTER),
        &["hello"],
        "clipboard written"
    );
    assert!(
        ed.state.kill_ring.head().is_none(),
        "kill ring NOT pushed by explicit \"cy"
    );
}

/// `"5y` writes the in-memory named register '5'; kill ring is not touched.
///
/// Digit-register writes route through `write_register` → `registers.write_text`,
/// not through `kill_ring.push`. The in-memory and ring storage are orthogonal.
#[test]
fn explicit_digit_y_writes_in_memory_only() {
    let mut ed = editor_from("-[hello]>\n");
    ed.feed_key(key('"'));
    ed.feed_key(key('5'));
    ed.feed_key(key('y')); // "5y → in-memory register '5' (not kill ring push)

    assert_eq!(reg(&ed, '5'), &["hello"], "register '5' written");
    assert!(
        ed.state.kill_ring.head().is_none(),
        "kill ring head untouched by explicit \"5y"
    );
}

/// `"5p` reads in-memory register '5', not the kill ring.
/// Push 6 entries to the ring via bare `d`; `"5p` must be a no-op (register '5' empty).
#[test]
fn explicit_digit_p_reads_inmemory_not_ring() {
    let mut ed = editor_from("-[a]>bcdefg\n");
    for _ in 0..6 {
        ed.feed_key(key('d'));
    }
    // ring has 6 entries; in-memory register '5' was never written
    let before = state(&ed);
    ed.feed_key(key('"'));
    ed.feed_key(key('5'));
    ed.feed_key(key('p')); // "5p → in-memory register '5' is empty → no-op

    assert_eq!(
        state(&ed),
        before,
        "\"5p must be a no-op when in-memory register '5' is empty"
    );
}

/// `"5y` then `"5p` round-trips via in-memory storage, regardless of kill-ring contents.
#[test]
fn digit_register_roundtrip_inmemory() {
    let mut ed = editor_from("-[INMEM]>\n");
    ed.feed_key(key('"'));
    ed.feed_key(key('5'));
    ed.feed_key(key('y'));
    // Ring: empty (no d/c). In-memory register '5' = "INMEM".
    // "5p must paste from in-memory, not clipboard or ring.
    ed.feed_key(key(';')); // collapse selection
    ed.feed_key(key('"'));
    ed.feed_key(key('5'));
    ed.feed_key(key('p'));

    assert!(
        ed.doc().text().to_string().contains("INMEM"),
        "\"5p must paste what \"5y wrote (in-memory round-trip)"
    );
}

/// `paste-ring-older` / `paste-ring-newer` (`[` / `]`) on an empty ring are no-ops.
#[test]
fn paste_ring_older_empty_ring_is_noop() {
    let mut ed = editor_from("-[a]>bc\n");
    let before = state(&ed);
    ed.feed_key(key('['));
    assert_eq!(state(&ed), before, "[ on empty ring is a no-op");
    // Capture fresh snapshot so ] is verified against actual post-[ state,
    // not the original — if [ accidentally mutated state, this catches both.
    let after_open = state(&ed);
    ed.feed_key(key(']'));
    assert_eq!(state(&ed), after_open, "] on empty ring is a no-op");
}

/// `[ ]` cycle within a paste session: the ring cursor walks older then back newer.
#[test]
fn paste_ring_cycle_older_then_newer() {
    // Push 3 entries: A\n (oldest), B\n, C\n (newest/head at slot 0).
    let mut ed = editor_from("-[A]>\nB\nC\n");
    ed.feed_key(key('x'));
    ed.feed_key(key('d')); // ring = [A\n]
    ed.feed_key(key('x'));
    ed.feed_key(key('d')); // ring = [B\n, A\n]
    ed.feed_key(key('x'));
    ed.feed_key(key('d')); // ring = [C\n, B\n, A\n]

    // Open paste session: `p` reads ring head (C\n) since last_command ∈ SMART_P_LAST_CMDS.
    ed.feed_key(key('p')); // seeds cycle at Some(0) = C\n

    // `[` cycles older: Some(0) → Some(1) = B\n, re-pastes from session snapshot.
    ed.feed_key(key('['));
    let after_first_older = ed.doc().text().to_string();
    assert!(after_first_older.contains('B'), "first [ pastes slot 1 (B)");
    // `[` again → Some(1) → Some(2) = A\n.
    ed.feed_key(key('['));
    let after_second_older = ed.doc().text().to_string();
    assert!(
        after_second_older.contains('A'),
        "second [ pastes slot 2 (A)"
    );
    // `]` retreats → Some(2) → Some(1) = B\n.
    ed.feed_key(key(']'));
    let after_newer = ed.doc().text().to_string();
    assert!(after_newer.contains('B'), "] after two [ pastes slot 1 (B)");
}

/// Select a line with `x`, delete with `d`, move with `j`, then paste via
/// explicit ring head (`"kp`) — the deleted line must appear as its own line *below*
/// the cursor, not embedded inside the current line.
#[test]
fn paste_ring_linewise_pastes_below_not_inline() {
    // Buffer: "A\nB\nC\n". Delete line A (x+d), move to C (j), paste via "kp.
    let mut ed = editor_from("-[A]>\nB\nC\n");
    ed.feed_key(key('x')); // select line "A\n"
    ed.feed_key(key('d')); // push "A\n" to ring head, buffer → "B\nC\n"
    ed.feed_key(key('j')); // cursor → 'C'
    // "kp reads ring head (="A\n") directly — avoids smart-p clipboard routing
    // after the intervening motion cleared last_command.
    ed.feed_key(key('"'));
    ed.feed_key(key('k'));
    ed.feed_key(key('p')); // paste ring head (A\n) linewise below C

    // "A\n" must land as its own line below C — not inside C's text.
    assert_eq!(
        state(&ed),
        "B\nC\n-[A\n]>",
        "\"kp on a linewise ring entry must paste as a new line, not inline"
    );
}

/// `[`/`]` cycle within a paste session REPLACES the previous paste — never
/// accumulates a second copy.
#[test]
fn paste_ring_warm_cycle_replaces_not_accumulates() {
    // Two ring entries: A\n (older, slot 1), B\n (head, slot 0).
    let mut ed = editor_from("-[A]>\nB\nC\n");
    ed.feed_key(key('x'));
    ed.feed_key(key('d')); // ring = [A\n]; buffer = "B\nC\n"
    ed.feed_key(key('x'));
    ed.feed_key(key('d')); // ring = [B\n, A\n]; buffer = "C\n"

    // p: smart-p reads ring head B\n (last_command = "delete"), opens session.
    ed.feed_key(key('p'));
    assert_eq!(
        ed.doc().text().to_string().matches("B\n").count(),
        1,
        "p pastes B once"
    );

    // [: cycle older (slot 0 → slot 1 = A\n) — must REPLACE B, not add another.
    ed.feed_key(key('['));
    let after_older = ed.doc().text().to_string();
    assert_eq!(
        after_older.matches("A\n").count(),
        1,
        "[ replaces paste with A"
    );

    // ]: cycle newer (slot 1 → slot 0 = B\n) — must REPLACE A.
    ed.feed_key(key(']'));
    let after_newer = ed.doc().text().to_string();
    assert_eq!(
        after_newer.matches("B\n").count(),
        1,
        "] replaces back with B"
    );
    assert_eq!(after_newer.matches("A\n").count(), 0, "] removes A");
}

/// Single-char cycle: `[` within a session pastes the older entry, `]` replaces
/// it back with the head — collapsed selection is not an obstacle.
#[test]
fn paste_ring_warm_cycle_replaces_single_char_paste() {
    let mut ed = editor_from("-[X]>Y\n");
    ed.feed_key(key('d')); // kill "X"; ring = [X], buffer = "-[Y]>\n"
    ed.feed_key(key('d')); // kill "Y"; ring = [Y, X], buffer = "-[\n]>"

    // p: reads ring head Y (last_command = "delete"), opens session, seeds cycle at 0.
    ed.feed_key(key('p'));
    assert!(
        ed.doc().text().to_string().contains('Y'),
        "p pastes Y (ring head)"
    );

    // [: cycle older (slot 0 → slot 1 = X), replaces Y.
    ed.feed_key(key('['));
    assert!(
        ed.doc().text().to_string().contains('X'),
        "[ pastes X (slot 1)"
    );

    // ]: cycle newer (slot 1 → slot 0 = Y), replacing X.
    ed.feed_key(key(']'));
    let buf = ed.doc().text().to_string();
    assert!(buf.contains('Y'), "] pastes Y (slot 0)");
    assert!(!buf.contains('X'), "] replaces X — no 'X' remains");
}

/// `P` (paste-before) opens a before-session; `[`/`]` must re-paste BEFORE the
/// cursor, not after it.
#[test]
fn paste_before_cycle_stays_before_charwise() {
    let mut ed = editor_from("-[c]>d\n"); // cursor on 'c' at index 0
    ed.state.kill_ring.push(vec!["X".to_string()]); // slot 1 after next push
    ed.state.kill_ring.push(vec!["Y".to_string()]); // ring=[Y, X]; head=Y, slot 1=X

    // "kP: paste-before ring head ("Y") before cursor 'c'.
    ed.feed_key(key('"'));
    ed.feed_key(key('k'));
    ed.feed_key(key('P'));
    assert_eq!(state(&ed), "-[Y]>cd\n", "P pastes before the cursor");

    // [: cycle to slot 1 ("X"); must re-paste BEFORE the cursor snapshot (at 0).
    ed.feed_key(key('['));
    assert_eq!(
        state(&ed),
        "-[X]>cd\n",
        "[ after P re-pastes before the cursor (would be c-[X]>d if it used paste_after)"
    );
}

/// `p` (paste-after) opens an after-session; cycling stays after (regression).
#[test]
fn paste_after_cycle_stays_after_charwise() {
    let mut ed = editor_from("-[c]>d\n");
    ed.state.kill_ring.push(vec!["X".to_string()]);
    ed.state.kill_ring.push(vec!["Y".to_string()]); // ring=[Y, X]

    ed.feed_key(key('"'));
    ed.feed_key(key('k'));
    ed.feed_key(key('p'));
    assert_eq!(state(&ed), "c-[Y]>d\n", "p pastes after the cursor");

    ed.feed_key(key('['));
    assert_eq!(state(&ed), "c-[X]>d\n", "[ after p stays paste-after");
}

/// `P` on a linewise entry opens a before-session; `[` must re-paste ABOVE the
/// cursor line, not below it.
#[test]
fn paste_before_cycle_stays_above_linewise() {
    let mut ed = editor_from("-[B]>\nC\n"); // cursor on 'B', line 0
    ed.state.kill_ring.push(vec!["X\n".to_string()]); // slot 1
    ed.state.kill_ring.push(vec!["Y\n".to_string()]); // ring=[Y\n, X\n]; head=Y\n

    // "kP: linewise paste-before ring head ("Y\n") — inserts above line 0.
    ed.feed_key(key('"'));
    ed.feed_key(key('k'));
    ed.feed_key(key('P'));
    assert_eq!(
        ed.doc().text().to_string(),
        "Y\nB\nC\n",
        "P pastes above current line"
    );

    // [: cycle to slot 1 ("X\n"); must re-paste ABOVE line 0 (not below).
    ed.feed_key(key('['));
    assert_eq!(
        ed.doc().text().to_string(),
        "X\nB\nC\n",
        "[ after linewise P re-pastes above (would be B\\nX\\nC\\n if it used paste_after)"
    );
}

/// `p [ p` duplicates the currently-cycled entry — never does a fresh clipboard paste.
///
/// After `[` swaps the paste to the ring head, `last_command = "paste-ring-older"`
/// has `is_paste = true`, so the next `p` must append (not replace).
#[test]
fn paste_after_cycle_appends_cycled_entry() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[x]>\n");
    ed.state.clipboard.force_unavailable();
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.state.kill_ring.push(vec!["RING".to_string()]); // ring head = "RING"

    // p: last_command=None → clipboard "CLIP"; seed_cycle(None).
    ed.feed_key(key('p'));
    assert!(
        ed.doc().text().to_string().contains("CLIP"),
        "first p must paste clipboard (last_command=None → not in SMART_P_LAST_CMDS)"
    );

    // [: cycle_older None→0="RING"; replaces "CLIP" with "RING"; last_paste=["RING"].
    ed.feed_key(key('['));
    // p: is_append (last_command="paste-ring-older" ∈ PASTE_FAMILY) → append last_paste.
    ed.feed_key(key('p'));

    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("RING").count(),
        2,
        "p after [ must duplicate the cycled entry"
    );
    assert!(
        !buf.contains("CLIP"),
        "clipboard must not appear after [ cycle"
    );
}

/// Consecutive `p` presses append copies rather than replacing the selected paste.
#[test]
fn consecutive_paste_appends_copies() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[ab]>\n");
    ed.state.clipboard.force_unavailable();
    // Seed clipboard with "CLIP" (distinct from ring) to falsify the assertion:
    // if the second p reads clipboard instead of last_paste, "CLIP" would appear.
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["CLIP".to_string()]);
    ed.feed_key(key('d')); // delete "ab" → ring = ["ab"], last_command = "delete"
    ed.feed_key(key('p')); // smart-p reads ring head "ab" (last_command="delete"); last_paste=["ab"]
    ed.feed_key(key('p')); // is_append → appends from last_paste = ["ab"]
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("ab").count(),
        2,
        "two consecutive p presses must stack two copies of 'ab'"
    );
    assert!(
        !buf.contains("CLIP"),
        "clipboard not used — append reads last_paste"
    );
}

/// Consecutive `p` presses append when the previous paste came from the CLIPBOARD
/// and the kill ring is empty — the second `p` must not be a no-op.
#[test]
fn consecutive_clipboard_paste_appends() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[x]>\n");
    ed.state.clipboard.force_unavailable(); // headless: reads fall back to in-memory mirror
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["XY".to_string()]);
    // ring is empty — this is the regression case

    ed.feed_key(key('p')); // last_command=None → clipboard → pastes "XY"; last_paste=["XY"]
    ed.feed_key(key('p')); // last_command="paste-after" ∈ PASTE_FAMILY → repeat last_paste
    let buf = ed.doc().text().to_string();
    assert_eq!(
        buf.matches("XY").count(),
        2,
        "two consecutive p presses must stack two copies even with an empty kill ring"
    );
}

/// Consecutive `p` after a clipboard paste must repeat the clipboard value, not
/// whatever happens to be at the ring head.
#[test]
fn consecutive_paste_repeats_last_not_ring_head() {
    use crate::ops::register::CLIPBOARD_REGISTER;

    let mut ed = editor_from("-[x]>\n");
    ed.state.clipboard.force_unavailable();
    ed.state
        .registers
        .write_text(CLIPBOARD_REGISTER, vec!["XY".to_string()]);
    ed.state.kill_ring.push(vec!["ZZ".to_string()]); // ring has different content

    ed.feed_key(key('p')); // clipboard → "XY"; last_paste=["XY"]
    ed.feed_key(key('p')); // append → repeats "XY", not ring head "ZZ"
    let buf = ed.doc().text().to_string();
    assert_eq!(buf.matches("XY").count(), 2, "clipboard value repeated");
    assert!(
        !buf.contains("ZZ"),
        "ring head must not appear — append repeats last paste verbatim"
    );
}

/// After paste-after, the pasted text is selected (covers the full inserted span).
#[test]
fn paste_leaves_output_selected() {
    // Delete "ab" → ring head = "ab". Then paste: selection must cover "ab".
    let mut ed = editor_from("-[ab]>cd\n");
    ed.feed_key(key('d')); // kill "ab"; buffer = "-[c]>d\n"
    ed.feed_key(key('p')); // smart-p reads ring head "ab" (charwise); paste after 'c'
    assert_eq!(
        state(&ed),
        "c-[ab]>d\n",
        "paste must leave the pasted text selected"
    );
}

// ── Register prefix persistence across non-register commands ────────────────

/// `"5` arms the prefix; `l` (a motion) does not consume it; the next `y` writes
/// to register 5. This is the intended sticky behaviour — the prefix persists
/// until a register-consuming command runs or Esc cancels it.
#[test]
fn register_prefix_persists_across_motion() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.handle_key(key('l')); // motion — does not consume the prefix
    ed.handle_key(key('y')); // yank targets register 5, not '"'

    assert!(
        !reg(&ed, '5').is_empty(),
        "register '5' written after motion"
    );
    assert!(reg(&ed, '"').is_empty(), "'\"' register untouched");
}

/// An explicit `"Xp` while in the append state must paste from register X,
/// not silently re-paste the previous value.  Before the fix, the append path
/// returned without calling `take_register_prefix()`, so the named register was
/// ignored AND the prefix leaked into the next command.
#[test]
fn register_prefix_overrides_append_path() {
    let mut ed = editor_from("-[x]>\n");
    ed.state.registers.write_text('5', vec!["REG5".to_string()]);
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    // Delete 'x' so the ring has "x" at head; RING is at slot 1.
    ed.feed_key(key('d'));
    // Paste via kill register to get into the append state with last_paste=[ring head].
    ed.handle_key(key('"'));
    ed.handle_key(key('k'));
    ed.feed_key(key('p'));

    // Now try to paste from named register '5' — must NOT take the append path.
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.feed_key(key('p'));

    let buf = ed.doc().text().to_string();
    assert!(
        buf.contains("REG5"),
        "explicit \"5p must paste from register 5, not re-paste last_paste; buf={buf:?}"
    );
}

/// After an explicit `"Xp` the register prefix must be consumed (not leaked).
/// Before the fix the prefix persisted and the NEXT command accidentally used it.
#[test]
fn register_prefix_consumed_by_paste() {
    let mut ed = editor_from("-[x]>\n");
    ed.state.registers.write_text('5', vec!["REG5".to_string()]);
    ed.state.kill_ring.push(vec!["RING".to_string()]);

    // Get into append state via a paste.
    ed.feed_key(key('d')); // delete x; ring head = "x"
    ed.handle_key(key('"'));
    ed.handle_key(key('k')); // select kill register
    ed.feed_key(key('p')); // paste ring head

    // Now type "5p — explicit register paste.
    ed.handle_key(key('"'));
    ed.handle_key(key('5'));
    ed.feed_key(key('p')); // should consume the '5' prefix

    // The prefix must be gone — the next 'd' must NOT route to register 5.
    ed.feed_key(key('d')); // delete; should push to kill ring, not register 5
    // Register 5 must still hold "REG5" — if the prefix leaked into 'd', it
    // would be overwritten with the deleted char.
    let reg5 = ed
        .state
        .registers
        .read('5')
        .and_then(|r| r.as_text())
        .map(|v| v.to_vec());
    assert_eq!(
        reg5,
        Some(vec!["REG5".to_string()]),
        "register 5 must be unchanged after d — prefix leaked if it differs"
    );
}

// ── Bundled theme loading (end-to-end wiring) ─────────────────────────────────

/// Smoke-test all bundled themes through the full loader → bake → resolve
/// pipeline. Catches wiring regressions (bad paths, parse errors, missing palette
/// entries) without needing a running editor.
#[test]
fn bundled_themes_load_and_resolve() {
    use std::path::PathBuf;
    let themes_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/themes");
    let paths = vec![themes_dir];

    for name in ["dark", "light", "gruvbox", "sand"] {
        let mut theme = hume_engine::theme::loader::load_theme(name, &paths)
            .unwrap_or_else(|e| panic!("bundled theme '{name}' failed to load: {e}"));
        let mut reg = hume_engine::theme::ScopeRegistry::new();
        reg.intern("ui.cursor.primary");
        reg.intern("ui.selection");
        theme.bake(&reg);
        let style = theme.resolve_by_name(hume_engine::types::Scope("ui.cursor.primary"));
        assert!(
            style.fg.is_some() || style.bg.is_some(),
            "bundled theme '{name}': ui.cursor.primary has neither fg nor bg"
        );
    }
}

/// `load_theme_by_name` reports failure via the message log and returns `false`;
/// the theme stays unchanged.
#[test]
fn load_theme_by_name_fails_gracefully() {
    let mut ed = editor_from("-[a]>b\n");
    let ok = crate::editor::theme::load_theme_by_name(
        &mut ed.view,
        &mut ed.state.message_log,
        &mut ed.state.status_msg,
        "no_such_theme_xyz",
    );
    assert!(!ok, "expected false for nonexistent theme");
    // Failure warning ends up in the message log, not as an error result.
    assert!(
        ed.state.message_log.has_unseen(),
        "expected a warning message"
    );
}

// ── Minibuffer arity-rule for Steel commands ──────────────────────────────

/// Wire up a Steel command and return the editor + scripting host ready for use.
///
/// Uses `EditorHostImpl` so `define-command!` registers the command directly
/// into the editor's `CommandRegistry` inline — no separate `register_steel_cmds`
/// needed.  The `arity` / `is_variadic` override re-registers with explicit
/// values (useful when the test arity differs from what Steel infers).
fn setup_arity_test(src: &str, name: &str, arity: u16, is_variadic: bool) -> Editor {
    use crate::editor::registry::MappableCommand;
    use crate::editor::scripting_setup::make_init_host;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    {
        let mut init_host = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_source(src, &mut init_host).unwrap();
    }
    // Override arity/is_variadic so minibuffer dispatch uses the test-supplied values.
    ed.state.registry.register(MappableCommand::SteelBacked {
        name: name.to_owned().into(),
        doc: std::borrow::Cow::Borrowed(""),
        arity,
        is_variadic,
        inline_output: false,
        repeatable: false,
    });
    ed.scripting = Some(host);
    ed
}

/// arity-1 + string arg: the rule converts the typed string to `StringV` and the
/// lambda receives it, queuing it as a command name, which runs `move-right`.
/// Oracle: state changes → cursor moved → arg was forwarded.
/// Verification: changing "move-right" in the assert to something else → fails.
#[test]
fn minibuffer_arity_rule_forwards_string_arg_to_arity_1() {
    let mut ed = setup_arity_test(
        r#"(define-command! "echo-cmd" "" (lambda (x) (when (string? x) (call! x))))"#,
        "echo-cmd",
        1,
        false,
    );

    let before = state(&ed);
    // `:echo-cmd move-right<Enter>` — arity-1 rule passes "move-right" as StringV.
    ed.handle_key(key(':'));
    for ch in "echo-cmd move-right".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_ne!(
        state(&ed),
        before,
        "arity-1 rule must forward arg as StringV; cursor must have moved"
    );
}

/// arity-1 + no arg: the rule passes `IntV(1)` as default count.  A string-type
/// lambda that checks `(string? x)` gets an integer, fails the check, and does
/// nothing — cursor stays put.  A count-type lambda `(lambda (count) ...)` gets
/// a valid count=1 rather than a type-mismatch boolean.
#[test]
fn minibuffer_arity_rule_passes_default_count_when_no_arg() {
    let mut ed = setup_arity_test(
        r#"(define-command! "echo-cmd" "" (lambda (x) (when (string? x) (call! x))))"#,
        "echo-cmd",
        1,
        false,
    );

    let before = state(&ed);
    // `:echo-cmd<Enter>` — no arg → arity-1 rule passes IntV(1); string guard rejects it.
    ed.handle_key(key(':'));
    for ch in "echo-cmd".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_eq!(
        state(&ed),
        before,
        "arity-1 with no arg must not crash or move cursor"
    );
}

/// arity-2 + one arg (the most the minibuffer can supply): the rule reports an
/// error and never invokes the command.  Cursor stays; error is logged.
/// The command needs no real lambda — the early return fires before call_steel_cmd.
#[test]
fn minibuffer_arity_rule_errors_on_arity_2() {
    use crate::editor::registry::MappableCommand;

    let mut ed = editor_from("-[a]>b\n");
    ed.state.registry.register(MappableCommand::SteelBacked {
        name: "needs-two".to_owned().into(),
        doc: std::borrow::Cow::Borrowed(""),
        arity: 2,
        is_variadic: false,
        inline_output: false,
        repeatable: false,
    });

    let before = state(&ed);
    // `:needs-two<Enter>` — arity-2 command, minibuffer can only supply 1 arg.
    ed.handle_key(key(':'));
    for ch in "needs-two".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_eq!(
        state(&ed),
        before,
        "arity rule must not dispatch the command"
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.text.contains("requires 2 args")),
        "arity rule must log a user-facing error"
    );
}

// ── Extend-trie WaitChar sequence cleanup ─────────────────────────────────────

/// A multi-key wait-char sequence bound in sticky-Extend mode must clear
/// `pending_keys` (and `pending_ctrl_extend`) once the sequence resolves —
/// mirroring what the normal-trie `WaitChar` arm already does. Before the fix,
/// the Extend-trie arm left the prefix key (`g`) sitting in `pending_keys`,
/// so the very next ordinary keystroke walked the trie as `[g, <key>]`
/// instead of `[<key>]` alone — silently swallowing it.
#[test]
fn extend_trie_wait_char_sequence_clears_pending_keys() {
    use crate::editor::keymap::{BindMode, WaitCharPending};

    let mut ed = editor_from("-[h]>ello\n");
    ed.state.mode = Mode::Extend;

    // Two-key wait-char sequence: `g` (prefix) then `r` (wait-char leaf).
    ed.state.keymap.extend.bind_wait_char_sequence(
        &[key('g'), key('r')],
        WaitCharPending {
            cmd_name: "find-forward".into(),
            ctrl_extend: false,
        },
    );
    // A plain leaf on `x`, distinct from the `g`-prefixed sequence, so a
    // leftover `g` prefix would make this unreachable (NoMatch) instead of
    // executing it.
    ed.state.keymap.bind_user_with_extend(
        BindMode::Extend,
        &[key('x')],
        "delete-char-forward".into(),
        false,
    );

    ed.handle_key(key('g'));
    assert_eq!(
        ed.state.pending_keys,
        vec![key('g')],
        "sanity: 'g' commits as an interior prefix key"
    );

    ed.handle_key(key('r'));
    assert!(
        ed.state.pending_keys.is_empty(),
        "the completed wait-char sequence must clear pending_keys"
    );
    assert!(
        ed.state.wait_char.is_some(),
        "sanity: the sequence armed wait_char"
    );

    // Consume the wait-char argument (any key) — dispatches "find-forward".
    ed.handle_key(key('z'));
    assert!(ed.state.wait_char.is_none());

    // With pending_keys correctly cleared, this reaches the `x` leaf and
    // deletes the char under the cursor.
    ed.handle_key(key('x'));
    assert_eq!(
        ed.doc().text().to_string(),
        "ello\n",
        "'x' must delete the char under the cursor, not get swallowed by a stale 'g' prefix"
    );
}

// ── gu / gU / gC case-transform keypath ──────────────────────────────────────

#[test]
fn gu_lowercases_selection() {
    let mut ed = editor_from("-[HELLO]> world\n");
    ed.feed_keys([key('g'), key('u')]);
    assert_eq!(state(&ed), "-[hello]> world\n");
}

#[test]
fn g_uppercase_u_uppercases_selection() {
    let mut ed = editor_from("-[hello]> world\n");
    ed.feed_keys([key('g'), key('U')]);
    assert_eq!(state(&ed), "-[HELLO]> world\n");
}

#[test]
fn gc_capitalizes_words_in_selection() {
    let mut ed = editor_from("-[hELLO wORLD]>\n");
    ed.feed_keys([key('g'), key('C')]);
    assert_eq!(state(&ed), "-[Hello World]>\n");
}

#[test]
fn gu_dot_repeats() {
    // Confirms the full keymap-dispatch path (not just the pure op) stamps
    // make-text-lowercase as repeatable.
    let mut ed = editor_from("-[HELLO]>\nWORLD\n");
    ed.feed_keys([key('g'), key('u')]); // "hello"
    ed.feed_key(key('j')); // move to line 2
    ed.feed_key(key('x')); // select the whole line ("WORLD\n")
    ed.feed_key(key('.')); // replay make-text-lowercase
    assert_eq!(state(&ed), "hello\n-[world\n]>");
}

// ── word-selects-whitespace (mm/MM, w/W/b/B around-word default) ──────────
//
// Full-dispatch coverage of the default flip: the ops-level tests in
// ops/motion/tests.rs and ops/text_object/tests.rs cover the span math
// (leading-preferred, trailing fallback for the first word of a line); these
// confirm the setting actually gates behavior through the real
// keymap/registry/dispatch path (:set, direct field write, and replay).

#[test]
fn w_default_selects_leading_space() {
    let mut ed = editor_from("-[f]>oo bar baz\n");
    ed.feed_key(key('w'));
    assert_eq!(state(&ed), "foo-[ bar]> baz\n");
}

#[test]
fn w_with_setting_off_selects_bare_word() {
    let mut ed = editor_from("-[f]>oo bar baz\n");
    ed.state.settings.word_selects_whitespace = false;
    ed.feed_key(key('w'));
    assert_eq!(state(&ed), "foo -[bar]> baz\n");
}

#[test]
fn w_set_buffer_off_selects_bare_word() {
    // Exercises the typed-command path (:set), not just a direct field write.
    let mut ed = editor_from("-[f]>oo bar baz\n");
    type_cmd(&mut ed, ":set buffer word-selects-whitespace=false");
    ed.feed_key(key('w'));
    assert_eq!(state(&ed), "foo -[bar]> baz\n");
}

#[test]
#[allow(non_snake_case)]
fn W_default_selects_leading_space() {
    let mut ed = editor_from("-[f]>oo, bar baz\n");
    ed.feed_key(key('W'));
    assert_eq!(state(&ed), "foo,-[ bar]> baz\n");
}

#[test]
fn b_default_selects_leading_space() {
    let mut ed = editor_from("foo bar -[b]>az\n");
    ed.feed_key(key('b'));
    assert_eq!(state(&ed), "foo-[ bar]> baz\n");
}

/// Regression: pressing `b` twice in a row must walk back through two
/// distinct words, not get stuck re-selecting the same one. The first press
/// absorbs "three"'s trailing space (default around-word), landing head on
/// that space; a naive second press searching from head would be fooled
/// into re-finding "three" instead of advancing to "two" — see
/// apply_word_select's `backward` parameter in ops/motion/word.rs.
#[test]
fn b_b_walks_back_through_distinct_words() {
    let mut ed = editor_from("one two three -[f]>our\n");
    ed.feed_key(key('b'));
    assert_eq!(state(&ed), "one two-[ three]> four\n");
    ed.feed_key(key('b'));
    assert_eq!(state(&ed), "one-[ two]> three four\n");
    ed.feed_key(key('b'));
    assert_eq!(state(&ed), "-[one ]>two three four\n");
}

#[test]
fn mm_default_matches_around_word() {
    // "hello" is the first word of the buffer, so it falls back to trailing
    // absorption — coincidentally matching what maw would give here too.
    let mut ed = editor_from("-[h]>ello world\n");
    ed.feed_keys([key('m'), key('m')]);
    assert_eq!(state(&ed), "-[hello ]>world\n");
}

#[test]
fn mm_mid_line_diverges_from_maw() {
    let mut ed = editor_from("foo -[b]>ar baz\n");
    ed.feed_keys([key('m'), key('m')]);
    assert_eq!(state(&ed), "foo-[ bar]> baz\n");

    let mut ed2 = editor_from("foo -[b]>ar baz\n");
    ed2.feed_keys([key('m'), key('a'), key('w')]);
    assert_eq!(state(&ed2), "foo -[bar ]>baz\n");
}

#[test]
fn mm_with_setting_off_matches_inner_word() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.state.settings.word_selects_whitespace = false;
    ed.feed_keys([key('m'), key('m')]);
    assert_eq!(state(&ed), "-[hello]> world\n");
}

#[test]
fn mm_default_on_whitespace_extends_to_adjacent_word() {
    // around_word_impl's on-whitespace rule: cursor on the space itself
    // extends forward to the adjacent word, same as pressing maw there.
    let mut ed = editor_from("foo-[ ]>bar\n");
    ed.feed_keys([key('m'), key('m')]);
    assert_eq!(state(&ed), "foo-[ bar]>\n");
}

#[test]
#[allow(non_snake_case)]
fn MM_default_matches_around_uppercase_word() {
    let mut ed = editor_from("-[h]>ello.world foo\n");
    ed.feed_keys([key('M'), key('M')]);
    assert_eq!(state(&ed), "-[hello.world ]>foo\n");
}

#[test]
#[allow(non_snake_case)]
fn MM_with_setting_off_matches_inner_uppercase_word() {
    let mut ed = editor_from("-[h]>ello.world foo\n");
    ed.state.settings.word_selects_whitespace = false;
    ed.feed_keys([key('M'), key('M')]);
    assert_eq!(state(&ed), "-[hello.world]> foo\n");
}

#[test]
fn miw_unaffected_by_setting() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.feed_keys([key('m'), key('i'), key('w')]);
    assert_eq!(state(&ed), "-[hello]> world\n");

    let mut ed2 = editor_from("-[h]>ello world\n");
    ed2.state.settings.word_selects_whitespace = false;
    ed2.feed_keys([key('m'), key('i'), key('w')]);
    assert_eq!(state(&ed2), "-[hello]> world\n");
}

#[test]
fn maw_unaffected_by_setting() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.feed_keys([key('m'), key('a'), key('w')]);
    assert_eq!(state(&ed), "-[hello ]>world\n");

    let mut ed2 = editor_from("-[h]>ello world\n");
    ed2.state.settings.word_selects_whitespace = false;
    ed2.feed_keys([key('m'), key('a'), key('w')]);
    assert_eq!(state(&ed2), "-[hello ]>world\n");
}

/// `select-word` (`mm`) is a Selection command, so it pushes an establish
/// step onto the dot-repeat recipe (unlike the reaching word motions) —
/// replay re-runs it via `run_native_body`, which must re-resolve
/// `word-selects-whitespace` fresh each time rather than baking in whatever
/// was true at the original keypress.
#[test]
fn dot_repeat_of_mm_delete_reresolves_word_selects_whitespace() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.feed_keys([key('m'), key('m')]); // select "hello " (around, default on)
    ed.feed_key(key('d')); // delete "hello " -> "world\n"
    assert_eq!(ed.doc().text().to_string(), "world\n");

    ed.state.settings.word_selects_whitespace = false;
    ed.feed_key(key('.')); // replay: re-establishes via mm (now bare), then deletes

    assert_eq!(ed.doc().text().to_string(), "\n");
}
