use super::*;
use pretty_assertions::assert_eq;

// ── Jump list ────────────────────────────────────────────────────────────────

/// `gg` from the middle of the file records the pre-jump position.
#[test]
fn goto_first_line_records_jump() {
    let mut ed = jump_editor(10);
    let before = state(&ed);

    // `gg` — goto first line.
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        0
    );

    // jump-backward should restore the pre-jump position.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
}

/// `ge` (goto-last-line) records a jump.
#[test]
fn goto_last_line_records_jump() {
    let mut ed = jump_editor(5);
    let before = state(&ed);

    ed.handle_key(key('g'));
    ed.handle_key(key('e'));
    assert_ne!(state(&ed), before); // moved somewhere else

    // jump-backward should restore the pre-jump position.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
}

/// Full round-trip: jump → jump-backward → jump-forward.
#[test]
fn jump_backward_then_forward() {
    let mut ed = jump_editor(10);

    // Jump to first line.
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    let at_top = state(&ed);

    // Back to original position.
    ed.handle_key(key_ctrl('o'));
    assert_ne!(state(&ed), at_top);

    // Forward returns to top.
    ed.handle_key(key_ctrl('i'));
    assert_eq!(state(&ed), at_top);
}

/// `#` (goto-matching-pair) jumps to the matching bracket and records the jump.
#[test]
fn goto_matching_pair_records_jump() {
    let text = BufferText::from("foo(bar)\n");
    let sels = SelectionSet::single(hume_editing::selection::Selection::collapsed(3)); // on '('
    let doc = Buffer::new(text, sels);
    let mut ed = Editor::for_testing(doc);
    ed.state.mode = Mode::Normal;
    let before = state(&ed);

    ed.handle_key(key('#'));
    assert_eq!(ed.current_selections().primary().head(), 7); // on ')'

    // jump-backward should restore the pre-jump position.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
}

/// A small motion (e.g. `2j`) does NOT record a jump.
#[test]
fn small_motion_does_not_record_jump() {
    let mut ed = jump_editor(10);
    let before = state(&ed);

    // Move down 2 lines — below the threshold.
    ed.handle_key(key('2'));
    ed.handle_key(key('j'));
    let after = state(&ed);
    assert_ne!(after, before);

    // jump-backward should NOT go back — nothing was recorded.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), after);
}

/// A large motion (e.g. `10j`) records a jump via the line-distance threshold.
#[test]
fn large_motion_records_jump() {
    let mut ed = jump_editor(0);
    let before = state(&ed);

    // Move down 10 lines — exceeds the threshold of 5.
    // Type "10j" as separate key presses.
    ed.handle_key(key('1'));
    ed.handle_key(key('0'));
    ed.handle_key(key('j'));
    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        10
    );

    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
}

/// Search confirm records a jump; search cancel does not.
#[test]
fn search_confirm_records_jump() {
    let mut ed = jump_editor(0);
    let before = state(&ed);

    // Search for "line 15".
    ed.handle_key(key('/'));
    for ch in "line 15".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        15
    );

    // jump-backward should return to line 0.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
}

/// Confirming a search whose only match sits at the cursor (so `n`'s wrap
/// lands right back on the start) is a no-op and must not truncate forward
/// jump-list history.
#[test]
fn search_confirm_noop_does_not_clobber_forward_history() {
    // Selection already spans the whole (only) match — exactly the shape
    // `search_sel` itself builds — so confirming search truly changes
    // nothing, not just "landed near where it started".
    let text = BufferText::from("foo\n");
    let sels = SelectionSet::single(hume_editing::selection::Selection::new(0, 2));
    let doc = Buffer::new(text, sels);
    let mut ed = Editor::for_testing(doc);
    ed.state.mode = Mode::Normal;

    // `%` — jump-flagged, moves elsewhere, records a jump.
    ed.handle_key(key('%'));
    let after_percent = state(&ed);

    // Jump backward to the original selection.
    ed.handle_key(key_ctrl('o'));
    let back_at_start = state(&ed);
    assert_ne!(back_at_start, after_percent);

    // Search for "foo" — the buffer's only occurrence, already selected —
    // wraps around and lands right back on the same span.
    ed.handle_key(key('/'));
    for ch in "foo".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(
        state(&ed),
        back_at_start,
        "search confirm on the buffer's only match, already under the cursor, must not move"
    );

    // Forward history (the jump from `%`) must still be there.
    ed.handle_key(key_ctrl('i'));
    assert_eq!(
        state(&ed),
        after_percent,
        "a no-op search confirm must not have truncated forward jump-list history"
    );
}

/// Search cancel (Esc) does NOT record a jump.
#[test]
fn search_cancel_does_not_record_jump() {
    let mut ed = jump_editor(0);
    let before = state(&ed);

    ed.handle_key(key('/'));
    for ch in "line 15".chars() {
        ed.handle_key(key(ch));
    }
    // Cancel — restores position.
    ed.handle_key(key_esc());
    assert_eq!(state(&ed), before);

    // jump-backward should NOT go anywhere — nothing recorded.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
}

/// `n` (search-next) records a jump.
#[test]
fn search_next_records_jump() {
    let mut ed = jump_editor(0);

    // Set up a search pattern first.
    ed.handle_key(key('/'));
    for ch in "line".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    // Now on line 1 (first match after line 0 which is also "line 0").
    let after_search = state(&ed);

    // Press `n` to go to next match.
    ed.handle_key(key('n'));
    let after_n = state(&ed);
    assert_ne!(after_n, after_search);

    // jump-backward should go back to the position before search-next.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), after_search);
}

/// When search-next lands on the same line as the previous match, jump-forward
/// must still return to the exact pre-jump-backward position.
#[test]
fn ctrl_i_works_when_current_is_same_line_as_last_jump() {
    // Two "editor" matches on the same line.
    let text = hume_editing::text::BufferText::from("the editor and the editor\nother line\n");
    let sels = hume_editing::selection::SelectionSet::single(
        hume_editing::selection::Selection::collapsed(0),
    );
    let doc = crate::editor::buffer::Buffer::new(text, sels);
    let mut ed = Editor::for_testing(doc);
    ed.kitty_enabled = true;

    // Search "editor" — lands on first match.
    ed.handle_key(key('/'));
    for ch in "editor".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    let first_match = state(&ed);

    // `n` — lands on second "editor" on the SAME line.
    ed.handle_key(key('n'));
    let second_match = state(&ed);
    assert_ne!(first_match, second_match);

    // jump-backward should go back to first match.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(
        state(&ed),
        first_match,
        "jump-backward should return to first match"
    );

    // jump-forward MUST return to the second match (the pre-jump-backward position).
    ed.handle_key(key_ctrl('i'));
    assert_eq!(
        state(&ed),
        second_match,
        "jump-forward should return to second match"
    );
}

/// `%` (select-all) records a jump so Ctrl-o returns to the pre-`%` position.
/// `select-all` needs a jump flag on its `Selection` command registration —
/// without it, the dispatch path never captures a pre-command snapshot and
/// Ctrl-o lands at whatever stale entry was already in the list.
#[test]
fn select_all_records_jump() {
    // Start on line 5 of a 20-line buffer.
    let mut ed = jump_editor(5);
    let before = state(&ed);

    // `%` — select entire buffer; cursor moves to the last character (line 19).
    ed.handle_key(key('%'));
    let after_select_all = state(&ed);
    assert_ne!(after_select_all, before, "% must move the cursor");
    // The cursor must be on the last line (line 19 in a 20-line buffer).
    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        19,
        "% should place the cursor at the last line"
    );

    // Ctrl-o must restore the position we were at before `%` (line 5).
    ed.handle_key(key_ctrl('o'));
    assert_eq!(
        state(&ed),
        before,
        "jump-backward should restore the pre-% position"
    );
}

/// `%` (select-all) from the buffer's own last char must still record a
/// jump — the head doesn't move (it was already `%`'s own landing spot), so
/// a `moved` check that compares heads alone sees no movement and drops the
/// entry, even though the anchor moved and the selection is now different.
#[test]
fn select_all_from_last_char_still_records_jump() {
    let text = BufferText::from("foo\nbar\n");
    let last = text.len_chars() - 1;
    let sels = SelectionSet::single(hume_editing::selection::Selection::collapsed(last));
    let doc = Buffer::new(text, sels);
    let mut ed = Editor::for_testing(doc);
    ed.state.mode = Mode::Normal;

    ed.handle_key(key('%'));
    let after = ed.current_selections().primary();
    assert_eq!(
        after.head(),
        last,
        "head shouldn't move — already on the last char"
    );
    assert_eq!(after.anchor(), 0, "% should still select the whole buffer");

    // Ctrl-o must restore the pre-% collapsed cursor.
    ed.handle_key(key_ctrl('o'));
    let restored = ed.current_selections().primary();
    assert_eq!(restored.anchor(), last);
    assert_eq!(restored.head(), last);
}

/// A no-op `#` (cursor not on a bracket or tag) must not truncate forward
/// jump-list history — `is_jump` alone used to record an entry regardless
/// of whether the command actually moved, and `JumpList::push` truncates
/// forward history unconditionally.
#[test]
fn goto_matching_pair_noop_does_not_clobber_forward_history() {
    let mut ed = jump_editor(10);

    // `gg` — records a jump, puts us at line 0.
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    let at_top = state(&ed);

    // Jump backward to the original line-10 position.
    ed.handle_key(key_ctrl('o'));
    let back_at_start = state(&ed);
    assert_ne!(back_at_start, at_top);

    // `#` on an ordinary character (not a bracket or tag) — a no-op.
    ed.handle_key(key('#'));
    assert_eq!(state(&ed), back_at_start, "# must not move on plain text");

    // Forward history (the jump to line 0) must still be there.
    ed.handle_key(key_ctrl('i'));
    assert_eq!(
        state(&ed),
        at_top,
        "a no-op # must not have truncated forward jump-list history"
    );
}

/// `}` (goto-next-paragraph) records a jump even for a one-line hop —
/// unlike a plain motion, it's registered `jump: true`, the same as the
/// structural `goto-next-<kind>` family, since it now selects a whole
/// object exactly like they do.
#[test]
fn goto_next_paragraph_records_jump_even_for_a_short_hop() {
    let text = BufferText::from("hello\n\nworld\n");
    let sels = SelectionSet::single(hume_editing::selection::Selection::collapsed(0));
    let doc = Buffer::new(text, sels);
    let mut ed = Editor::for_testing(doc);
    ed.state.mode = Mode::Normal;
    let before = state(&ed);

    ed.handle_key(key('}'));
    assert_ne!(state(&ed), before);

    // jump-backward should restore the pre-jump position, even though the
    // hop crossed only one line — well under the jump-line-threshold a
    // plain motion is gated by.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
}

/// `{` (goto-prev-paragraph) records a jump even for a one-line hop — same
/// `jump: true` registration as `}`, checked separately since the two are
/// driven by distinct start-finding code (`next_paragraph_start` vs
/// `prev_paragraph_start`).
#[test]
fn goto_prev_paragraph_records_jump_even_for_a_short_hop() {
    let text = BufferText::from("hello\n\nworld\n");
    let sels = SelectionSet::single(hume_editing::selection::Selection::collapsed(7)); // on 'w'
    let doc = Buffer::new(text, sels);
    let mut ed = Editor::for_testing(doc);
    ed.state.mode = Mode::Normal;
    let before = state(&ed);

    ed.handle_key(key('{'));
    assert_ne!(state(&ed), before);

    // jump-backward should restore the pre-jump position, even though the
    // hop crossed only one line — well under the jump-line-threshold a
    // plain motion is gated by.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
}

/// A no-op `}` (no paragraph below) must not truncate forward jump-list
/// history — same failure mode as
/// `goto_matching_pair_noop_does_not_clobber_forward_history` above.
/// Asserting the pre/post state is unchanged (as the previous version of
/// this test did) can't catch a wrongly-pushed entry: a no-op's own jump
/// restores right back to where it already was, so that assertion passes
/// whether or not the entry was pushed. Forward history is the only
/// observable that distinguishes the two.
#[test]
fn goto_next_paragraph_noop_does_not_clobber_forward_history() {
    let mut ed = jump_editor(10); // single paragraph, nothing below to select

    // `gg` — records a jump, puts us at line 0.
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    let at_top = state(&ed);

    // Jump backward to the original line-10 position.
    ed.handle_key(key_ctrl('o'));
    let back_at_start = state(&ed);
    assert_ne!(back_at_start, at_top);

    // `}` — the whole buffer is one paragraph, nothing below — a no-op.
    ed.handle_key(key('}'));
    assert_eq!(
        state(&ed),
        back_at_start,
        "}} must not move — nothing below"
    );

    // Forward history (the jump to line 0) must still be there.
    ed.handle_key(key_ctrl('i'));
    assert_eq!(
        state(&ed),
        at_top,
        "a no-op }} must not have truncated forward jump-list history"
    );
}

/// `{` counterpart to the no-op test above, checked separately since the
/// no-op paths are driven by distinct code (`prev_paragraph_start`'s
/// `checked_sub` underflow vs `next_paragraph_start`'s `line < total`
/// guard).
#[test]
fn goto_prev_paragraph_noop_does_not_clobber_forward_history() {
    let mut ed = jump_editor(10); // single paragraph, nothing above to select

    // `gg` — records a jump, puts us at line 0.
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    let at_top = state(&ed);

    // Jump backward to the original line-10 position.
    ed.handle_key(key_ctrl('o'));
    let back_at_start = state(&ed);
    assert_ne!(back_at_start, at_top);

    // `{` — the whole buffer is one paragraph, nothing above — a no-op.
    ed.handle_key(key('{'));
    assert_eq!(
        state(&ed),
        back_at_start,
        "{{ must not move — nothing above"
    );

    // Forward history (the jump to line 0) must still be there.
    ed.handle_key(key_ctrl('i'));
    assert_eq!(
        state(&ed),
        at_top,
        "a no-op {{ must not have truncated forward jump-list history"
    );
}

/// search-next + jump-backward + jump-forward round-trip, all matches on different lines.
#[test]
fn search_n_ctrl_o_ctrl_i_different_lines() {
    let mut ed = jump_editor(0);

    // Search "line 1" — matches lines 1, 10, 11, 12, ...
    ed.handle_key(key('/'));
    for ch in "line 1".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    // `n` twice to advance through matches on different lines.
    ed.handle_key(key('n'));
    let state_after_n1 = state(&ed);
    ed.handle_key(key('n'));
    let state_after_n2 = state(&ed);

    // jump-backward goes back.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), state_after_n1);

    // jump-forward goes forward.
    ed.handle_key(key_ctrl('i'));
    assert_eq!(state(&ed), state_after_n2);
}

// ── Remapping through edits ─────────────────────────────────────────────────
//
// `finish_edit` (`doc_ops.rs`) remaps every pane's jump list through each
// edit's `ChangeSet`, same chokepoint sibling panes' selections go through.
// Every test below locates its expected landing spot independently (via
// `str::find` on the post-edit text), never by re-deriving the offset the
// code under test itself computed — a stale-but-in-range offset and a
// correctly remapped one can otherwise look identical if the assertion
// secretly reuses the same arithmetic.

/// Assert the focused cursor sits exactly at the start of `marker` within
/// `text`. `text` is passed in rather than read from `ed`, since several
/// callers already hold an independently-known expected buffer content —
/// reading it back from the editor here would make half of that comparison
/// tautological.
fn assert_cursor_at_marker(ed: &Editor, text: &str, marker: &str, msg: &str) {
    let target = text.find(marker).expect("marker line present");
    let expected = serialize_state(
        &BufferText::from(text),
        &SelectionSet::single(hume_editing::selection::Selection::collapsed(target)),
    );
    assert_eq!(state(ed), expected, "{msg}");
}

/// Inserting a line above a recorded jump entry must not leave `Ctrl+O`
/// landing on the entry's stale (now-wrong) line — it must follow the text.
#[test]
fn insert_above_jump_entry_lands_on_marker_text() {
    let mut ed = jump_editor(10);

    // `gg` records a jump entry at line 10, lands the cursor at line 0.
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));

    // Open a blank line above line 0 — every original line shifts down by one.
    ed.handle_key(key('O'));
    ed.handle_key(key_esc());

    ed.handle_key(key_ctrl('o'));

    let text_after = ed.doc().text().to_string();
    assert_cursor_at_marker(
        &ed,
        &text_after,
        "line 10\n",
        "Ctrl+O must land on the marker text, not the pre-edit line index",
    );
}

/// Deleting a line above a recorded jump entry must shift it up along with
/// the text, not leave it pointing at whatever now occupies the old offset.
#[test]
fn delete_above_jump_entry_lands_on_marker_text() {
    let mut ed = jump_editor(10);

    ed.handle_key(key('g'));
    ed.handle_key(key('g')); // records a jump entry at line 10, lands at line 0

    // Delete line 0 entirely — every remaining line shifts up by one.
    ed.handle_key(key('x')); // select-line
    ed.handle_key(key('d')); // delete selection

    ed.handle_key(key_ctrl('o'));

    let text_after = ed.doc().text().to_string();
    assert_cursor_at_marker(
        &ed,
        &text_after,
        "line 10\n",
        "Ctrl+O must land on the marker text after a deletion above it",
    );
}

/// Undoing an edit must remap jump entries back too — the inverse
/// `ChangeSet` goes through the same `finish_edit` chokepoint as any edit.
#[test]
fn undo_after_edit_remaps_jump_entry_back() {
    let mut ed = jump_editor(10);
    ed.handle_key(key('g'));
    ed.handle_key(key('g')); // records a jump entry at line 10, lands at line 0

    let text_before = ed.doc().text().to_string();

    ed.handle_key(key('O'));
    ed.handle_key(key_esc());
    ed.handle_key(key('u')); // undo the insert

    ed.handle_key(key_ctrl('o'));

    assert_cursor_at_marker(
        &ed,
        &text_before,
        "line 10\n",
        "undo's inverse ChangeSet must remap the entry back to its original position",
    );
}

/// Redoing an edit must remap jump entries forward again too — `redo`
/// replays the edit's own forward `ChangeSet` through the same `finish_edit`
/// chokepoint as any other edit, a separate call site from `undo`'s.
#[test]
fn redo_after_undo_remaps_jump_entry_forward_again() {
    let mut ed = jump_editor(10);
    ed.handle_key(key('g'));
    ed.handle_key(key('g')); // records a jump entry at line 10, lands at line 0

    ed.handle_key(key('O'));
    ed.handle_key(key_esc());

    let text_after_insert = ed.doc().text().to_string();

    ed.handle_key(key('u')); // undo the insert
    ed.handle_key(key('U')); // redo it

    ed.handle_key(key_ctrl('o'));

    assert_cursor_at_marker(
        &ed,
        &text_after_insert,
        "line 10\n",
        "redo's forward ChangeSet must remap the entry forward again",
    );
}

/// An edit made from one pane must remap jump entries in every pane viewing
/// the buffer, not just the pane that performed the edit — jump lists are
/// per-pane, but the edit chokepoint doesn't know or care which pane a given
/// list belongs to.
#[test]
fn edit_remaps_jump_entries_in_every_pane_viewing_the_buffer() {
    let mut ed = jump_editor(10);
    let pid_a = ed.state.focused_pane_id;

    ed.handle_key(key('g'));
    ed.handle_key(key('g')); // pane A records a jump entry at line 10

    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_a, pid_b, "focus moved to the new pane");
    assert_eq!(
        ed.state.panes.jumps[pid_b].len(),
        1,
        "split clones the source pane's jump list"
    );

    // Edit from pane B — inserts a blank line above line 0.
    ed.handle_key(key('O'));
    ed.handle_key(key_esc());

    let text_after = ed.doc().text().to_string();

    ed.switch_focused_pane(pid_a);
    ed.handle_key(key_ctrl('o'));
    assert_cursor_at_marker(
        &ed,
        &text_after,
        "line 10\n",
        "pane A's entry must be remapped even though pane B made the edit",
    );
}

/// A jump entry for a buffer that wasn't edited must not move when a
/// different buffer is edited — the jump list is cross-buffer, so a remap
/// triggered by buffer B's edit must skip buffer A's entries entirely.
#[test]
fn edit_in_one_buffer_does_not_move_a_jump_entry_for_another_buffer() {
    let dir = safe_tempdir();
    let file1 = dir.path().join("file1.txt");
    let file2 = dir.path().join("file2.txt");
    let content: String = (0..20).map(|i| format!("line {i}\n")).collect();
    std::fs::write(&file1, &content).unwrap();
    std::fs::write(&file2, "other\nfile\n").unwrap();

    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from("scratch\n"),
        SelectionSet::default(),
    ));
    ed.execute_typed("e", Some(file1.to_str().unwrap()))
        .unwrap();
    let buf1 = ed.focused_buffer_id();

    // Move to line 10 with per-step motions (below the jump threshold, so no
    // entry is recorded for the walk itself), then `gg` records an entry at
    // (file1, line 10) and lands the cursor at line 0.
    for _ in 0..10 {
        ed.handle_key(key('j'));
    }
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    let file1_text = ed.doc().text().to_string();

    // `:e file2` records a second entry — (file1, line 0), the switch-away
    // point — then focuses file2.
    ed.execute_typed("e", Some(file2.to_str().unwrap()))
        .unwrap();
    assert_ne!(ed.focused_buffer_id(), buf1, "focus moved to file2");

    // Edit file2 — must have zero effect on file1's entries.
    ed.handle_key(key('O'));
    ed.handle_key(key_esc());
    ed.handle_key(key('O'));
    ed.handle_key(key_esc());

    // First Ctrl+O returns to the switch-away point in file1; second Ctrl+O
    // reaches the earlier (file1, line 10) entry.
    ed.handle_key(key_ctrl('o'));
    ed.handle_key(key_ctrl('o'));
    assert_eq!(ed.focused_buffer_id(), buf1, "landed back in file1");

    assert_cursor_at_marker(
        &ed,
        &file1_text,
        "line 10\n",
        "file1's jump entry must be untouched by edits made in file2",
    );
}

/// `:e!` reloads through a line-diff `ChangeSet` (`Buffer::reload_from_text`)
/// that bypasses the ordinary edit chokepoint — jump entries must still be
/// remapped through it, not just entries produced by in-editor edits.
#[test]
fn reload_remaps_jump_entries_through_line_diff() {
    let dir = safe_tempdir();
    let path = dir.path().join("reload.txt");
    let content: String = (0..20).map(|i| format!("line {i}\n")).collect();
    std::fs::write(&path, &content).unwrap();

    let mut ed = Editor::for_testing(Buffer::new(
        BufferText::from("scratch\n"),
        SelectionSet::default(),
    ));
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();

    for _ in 0..10 {
        ed.handle_key(key('j'));
    }
    ed.handle_key(key('g'));
    ed.handle_key(key('g')); // records an entry at line 10, lands at line 0
    // 2 entries: `:e` itself recorded leaving the initial scratch buffer,
    // then `gg` recorded line 10.
    assert_eq!(ed.state.panes.jumps[ed.state.focused_pane_id].len(), 2);

    // Reload with 3 new lines prepended on disk.
    let new_content = format!("new0\nnew1\nnew2\n{content}");
    std::fs::write(&path, &new_content).unwrap();
    ed.execute_typed("e!", None).unwrap();

    ed.handle_key(key_ctrl('o'));

    assert_cursor_at_marker(
        &ed,
        &new_content,
        "line 10\n",
        "Ctrl+O must land on the marker text after :e! shifted it",
    );
}

/// A view buffer refreshed via `open_read_only_view` (e.g. re-running
/// `:messages`) resets its history — there is no `ChangeSet` to remap jump
/// entries through, so they must be dropped outright rather than left
/// pointing at arbitrary text in the regenerated content.
#[test]
fn view_buffer_refresh_drops_its_jump_entries() {
    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.open_read_only_view("[jump-list-test]", "one\ntwo\nthree\nfour\nfive\n", 0);

    // `ge` (goto-last-line) records an entry tagged with this view buffer.
    ed.handle_key(key('g'));
    ed.handle_key(key('e'));

    let pid = ed.state.focused_pane_id;
    assert!(
        ed.state.panes.jumps[pid].entries_for_buffer(bid),
        "goto-last-line should have recorded an entry for the view buffer"
    );

    // Re-open the same label — refreshes content in place via `set_view_content`.
    ed.open_read_only_view("[jump-list-test]", "brand new content\n", 0);

    assert!(
        !ed.state.panes.jumps[pid].entries_for_buffer(bid),
        "a regenerated view buffer's stale jump entries must be dropped"
    );
}

/// A view buffer refresh must reseed every pane viewing it, not just the
/// focused one — otherwise a sibling pane keeps a selection computed against
/// content that no longer exists, potentially pointing past the new
/// content's end.
#[test]
fn view_buffer_refresh_reseeds_every_pane_viewing_it() {
    let mut ed = editor_from("-[a]>b\n");
    let pid_a = ed.state.focused_pane_id;
    ed.open_read_only_view("[jump-list-test]", "one\ntwo\nthree\nfour\nfive\n", 0);

    // Split so a second pane also views the view buffer; focus moves to it.
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_a, pid_b, "focus moved to the new pane");
    let bid = ed.focused_buffer_id();

    // Move pane B's cursor off line 0, deep into content that won't exist
    // after the refresh below.
    ed.handle_key(key('g'));
    ed.handle_key(key('e')); // goto-last-line
    assert_ne!(
        ed.current_selections().primary().head(),
        0,
        "cursor actually moved off the initial position"
    );

    // Refresh from pane A (still focused there), with content too short for
    // pane B's stale offset to remain valid.
    ed.switch_focused_pane(pid_a);
    ed.open_read_only_view("[jump-list-test]", "x\n", 0);

    assert_eq!(
        ed.state.panes.state[pid_b][bid].selections,
        ed.state.buffers.get(bid).initial_sels(),
        "a sibling pane's selection must be reseeded on a view-buffer refresh, not left stale"
    );
}

/// A view-buffer refresh must also reseed a pane that *used to* view it but
/// switched to a different buffer before the refresh — not just panes
/// viewing it at refresh time. Otherwise switching that pane back finds a
/// selection computed against content the refresh already discarded,
/// potentially pointing past the new content's end.
#[test]
fn view_buffer_refresh_reseeds_a_pane_that_switched_away_before_the_refresh() {
    let mut ed = editor_from("-[a]>b\n");
    let bid_scratch = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;
    let bid = ed.open_read_only_view("[jump-list-test]", "one\ntwo\nthree\nfour\nfive\n", 0);

    // Split so pane B also views the view buffer; focus moves to it.
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_a, pid_b, "focus moved to the new pane");

    // Pane A moves deep into content that won't exist after the refresh below.
    ed.switch_focused_pane(pid_a);
    ed.handle_key(key('g'));
    ed.handle_key(key('e')); // goto-last-line
    assert_ne!(
        ed.current_selections().primary().head(),
        0,
        "cursor actually moved off the initial position"
    );

    // Pane A switches away to a different buffer — it is no longer a viewer
    // of the view buffer when it gets refreshed below.
    ed.switch_to_buffer_without_jump(bid_scratch);

    // Refresh from pane B, with content too short for pane A's stale cached
    // selection to remain valid.
    ed.switch_focused_pane(pid_b);
    ed.open_read_only_view("[jump-list-test]", "x\n", 0);

    // Pane A switches back to the view buffer — its cached selection for
    // `bid` must have been reseeded by the refresh, not left stale.
    ed.switch_focused_pane(pid_a);
    ed.switch_to_buffer_without_jump(bid);
    assert_eq!(
        ed.current_selections(),
        &ed.state.buffers.get(bid).initial_sels(),
        "a pane that switched away before a view-buffer refresh must still be reseeded, \
         not just panes that were viewing it at refresh time"
    );
}
