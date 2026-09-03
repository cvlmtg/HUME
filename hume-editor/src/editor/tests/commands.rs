use super::*;
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

/// Ctrl-W (delete-word-backward) inside a `c` session must not clear the
/// insertion pin — it's a text edit within the session, not a cursor motion
/// away from it. `select-changed-text` must still select what survived.
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
    use hume_ops::register::CLIPBOARD_REGISTER;

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

// ── WaitChar accepts Enter and Tab (Kakoune parity) ────────────────────────────

/// `r<ret>` replaces the char under the cursor with a literal newline. The
/// wait-char consumer translates `KeyCode::Enter` to `'\n'` before dispatch.
#[test]
fn r_then_enter_replaces_with_newline() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('r'));
    ed.handle_key(key_enter());
    assert!(
        ed.state.wait_char.is_none(),
        "wait_char cleared after Enter"
    );
    assert_eq!(state(&ed), "-[\n]>ello\n");
}

/// Multi-char selection: every grapheme becomes '\n', except a grapheme that
/// already was '\n' — `replace_selections` never replaces an existing newline,
/// it is retained as-is (see `replace_multiline_selection_skips_newline` in
/// `hume-ops/src/edit/tests/replace.rs`). This exercises that rule with the new Enter argument.
#[test]
fn r_then_enter_multi_char_selection_replaces_each_grapheme() {
    let mut ed = editor_from("-[ab\ncd]>\n");
    ed.handle_key(key('r'));
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "-[\n\n\n\n\n]>\n");
}

/// `r<tab>` replaces with a literal tab character.
#[test]
fn r_then_tab_replaces_with_tab() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('r'));
    ed.handle_key(key_tab());
    assert!(ed.state.wait_char.is_none(), "wait_char cleared after Tab");
    assert_eq!(state(&ed), "-[\t]>ello\n");
}

/// `f<ret>` is accepted as a wait-char argument (the wait clears, unlike Esc)
/// but never matches: `find_char_on_line_forward` (hume-ops/src/motion/find.rs)
/// explicitly excludes '\n' as a structural line boundary, not content — by
/// design, not a bug. This documents that "accepted argument" and "found on
/// line" are separate questions, exactly like `fz` on a line with no 'z'.
#[test]
fn f_then_enter_is_accepted_but_never_matches() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('f'));
    ed.handle_key(key_enter());
    assert!(
        ed.state.wait_char.is_none(),
        "wait_char cleared after Enter"
    );
    assert_eq!(
        state(&ed),
        "-[h]>ello\n",
        "'\\n' is never a match target for find — cursor unchanged"
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
    ed.handle_key(key('p')); // smart-paste-after
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

// ── Typed-command arity rule for Steel commands ───────────────────────────

/// Wire up a Steel *typed* command and return the editor + scripting host
/// ready for use.
///
/// Uses `EditorHostImpl` so `define-typed-command!` registers the command
/// directly into the editor's `CommandRegistry` inline. The `arity` /
/// `is_variadic` override re-registers with explicit values (useful when the
/// test arity differs from what Steel infers).
fn setup_typed_arity_test(src: &str, name: &str, arity: u16, is_variadic: bool) -> Editor {
    use crate::editor::registry::{TypedBody, TypedCommand};
    use crate::editor::scripting_setup::make_init_host;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    {
        let mut init_host = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_source(src, &mut init_host).unwrap();
    }
    // Override arity/is_variadic so typed dispatch uses the test-supplied values.
    ed.state.config.registry.register_typed(TypedCommand {
        name: name.to_owned().into(),
        doc: std::borrow::Cow::Borrowed(""),
        aliases: &[],
        body: TypedBody::Steel {
            arity,
            is_variadic,
            inline_output: false,
        },
        completer: None,
    });
    ed.scripting = Some(host);
    ed
}

/// arity-1 + typed arg: the arg reaches the lambda as `StringV`, queued as a
/// command name via `call!`, which runs `move-right`.
/// Oracle: state changes → cursor moved → arg was forwarded.
/// Verification: changing "move-right" in the assert to something else → fails.
#[test]
fn typed_arity_rule_forwards_string_arg_to_arity_1() {
    let mut ed = setup_typed_arity_test(
        r#"(define-typed-command! "echo-cmd" "" (lambda (x) (when (string? x) (call! x))))"#,
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

/// arity-1 + no arg: the rule passes `#f` (Scheme's spelling of "no argument
/// typed"), not a sentinel string or a fabricated count. A string-type lambda
/// that checks `(string? x)` gets a boolean, fails the check, and does
/// nothing — cursor stays put.
#[test]
fn typed_arity_rule_passes_false_when_no_arg() {
    let mut ed = setup_typed_arity_test(
        r#"(define-typed-command! "echo-cmd" "" (lambda (x) (when (string? x) (call! x))))"#,
        "echo-cmd",
        1,
        false,
    );

    let before = state(&ed);
    // `:echo-cmd<Enter>` — no arg → arity-1 rule passes #f; string guard rejects it.
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

/// arity-2 (`arg force`, the most a typed command can receive): both values
/// reach the lambda — the arg as `StringV`, `!` as `#t`.
#[test]
fn typed_arity_rule_forwards_arg_and_force_to_arity_2() {
    let mut ed = setup_typed_arity_test(
        r#"(define-typed-command! "echo-cmd" ""
             (lambda (x force) (when (and (string? x) force) (call! x))))"#,
        "echo-cmd",
        2,
        false,
    );

    let before = state(&ed);
    // `:echo-cmd! move-right<Enter>` — force=#t only when `!` is appended.
    ed.handle_key(key(':'));
    for ch in "echo-cmd! move-right".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_ne!(
        state(&ed),
        before,
        "arity-2 rule must forward both arg and force; cursor must have moved"
    );
}

/// arity-3 (more than a typed command can supply): the rule reports an error
/// and never invokes the command. Cursor stays; error is logged. The command
/// needs no real lambda — the early return fires before call_steel_cmd.
#[test]
fn typed_arity_rule_errors_on_arity_3() {
    use crate::editor::registry::{TypedBody, TypedCommand};

    let mut ed = editor_from("-[a]>b\n");
    ed.state.config.registry.register_typed(TypedCommand {
        name: "needs-three".to_owned().into(),
        doc: std::borrow::Cow::Borrowed(""),
        aliases: &[],
        body: TypedBody::Steel {
            arity: 3,
            is_variadic: false,
            inline_output: false,
        },
        completer: None,
    });

    let before = state(&ed);
    // `:needs-three<Enter>` — arity-3 command, typed dispatch supplies at most 2.
    ed.handle_key(key(':'));
    for ch in "needs-three".chars() {
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
            .any(|e| e.text.contains("supplies at most 2")),
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
    ed.state.config.keymap.extend.bind_wait_char_sequence(
        &[key('g'), key('r')],
        WaitCharPending {
            cmd_name: "find-forward".into(),
            ctrl_extend: false,
        },
    );
    // A plain leaf on `x`, distinct from the `g`-prefixed sequence, so a
    // leftover `g` prefix would make this unreachable (NoMatch) instead of
    // executing it.
    ed.state.config.keymap.bind_user_with_extend(
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

// ── G L / G U / G C case-transform keypath ───────────────────────────────────

#[test]
fn g_shift_l_lowercases_selection() {
    let mut ed = editor_from("-[HELLO]> world\n");
    ed.feed_keys([key('G'), key('L')]);
    assert_eq!(state(&ed), "-[hello]> world\n");
}

#[test]
fn g_shift_u_uppercases_selection() {
    let mut ed = editor_from("-[hello]> world\n");
    ed.feed_keys([key('G'), key('U')]);
    assert_eq!(state(&ed), "-[HELLO]> world\n");
}

#[test]
fn g_shift_c_capitalizes_words_in_selection() {
    let mut ed = editor_from("-[hELLO wORLD]>\n");
    ed.feed_keys([key('G'), key('C')]);
    assert_eq!(state(&ed), "-[Hello World]>\n");
}

#[test]
fn g_shift_l_dot_repeats() {
    // Confirms the full keymap-dispatch path (not just the pure op) stamps
    // make-text-lowercase as repeatable.
    let mut ed = editor_from("-[HELLO]>\nWORLD\n");
    ed.feed_keys([key('G'), key('L')]); // "hello"
    ed.feed_key(key('j')); // move to line 2
    ed.feed_key(key('x')); // select the whole line ("WORLD\n")
    ed.feed_key(key('.')); // replay make-text-lowercase
    assert_eq!(state(&ed), "hello\n-[world\n]>");
}

// ── `>` / `<` indent / unindent ─────────────────────────────────────────────

#[test]
fn angle_bracket_indents_and_unindents() {
    // Default settings: tab-style=hard, tab-width=4.
    let mut ed = editor_from("-[f]>oo\n");
    ed.handle_key(key('>'));
    assert_eq!(state(&ed), "\t-[f]>oo\n");
    ed.handle_key(key('<'));
    assert_eq!(state(&ed), "-[f]>oo\n");
}

/// A count reaches the pure op (proving the keymap→`EditorCmd` count plumbing
/// works for `indent`, same as `cmd_align_selections`), and the whole `3>`
/// composes into a single undo step rather than three.
#[test]
fn three_indent_is_one_undo_step() {
    let mut ed = editor_from("-[f]>oo\n");
    ed.feed_keys([key('3'), key('>')]);
    assert_eq!(state(&ed), "\t\t\t-[f]>oo\n");

    ed.handle_key(key('u'));
    assert_eq!(state(&ed), "-[f]>oo\n");
    assert!(!ed.doc().can_undo());
}
