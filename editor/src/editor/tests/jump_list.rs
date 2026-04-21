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
    assert_eq!(ed.doc().text().char_to_line(ed.current_selections().primary().head), 0);

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
    assert_eq!(ed.doc().text().char_to_line(ed.current_selections().primary().head), 10);

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
    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(ed.doc().text().char_to_line(ed.current_selections().primary().head), 15);

    // jump-backward should return to line 0.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
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
    let text = "the editor and the editor\nother line\n";
    let buf = crate::core::text::Text::from(text);
    let sels = crate::core::selection::SelectionSet::single(
        crate::core::selection::Selection::collapsed(0),
    );
    let doc = crate::editor::buffer::Buffer::new(buf, sels);
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
    assert_eq!(state(&ed), first_match, "jump-backward should return to first match");

    // jump-forward MUST return to the second match (the pre-jump-backward position).
    ed.handle_key(key_ctrl('i'));
    assert_eq!(state(&ed), second_match, "jump-forward should return to second match");
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

