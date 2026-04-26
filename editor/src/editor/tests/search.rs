use super::*;
use pretty_assertions::assert_eq;

// ── Search ────────────────────────────────────────────────────────────────────

/// `/` opens Search mode; typing a pattern triggers live search; `Enter` confirms
/// the match and writes the pattern to the `'s'` register.
#[test]
fn search_forward_enter_confirms() {
    let mut ed = editor_from("-[h]>ello world\n");

    ed.handle_key(key('/'));
    assert_eq!(ed.mode, Mode::Search);

    for ch in "world".chars() {
        ed.handle_key(key(ch));
    }
    // Live search has already moved the selection to "world".
    assert_eq!(state(&ed), "hello -[world]>\n");

    ed.handle_key(key_enter());
    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(state(&ed), "hello -[world]>\n");
    // Pattern written to the 's' register for n/N repeat.
    assert_eq!(reg(&ed, 's'), vec!["world"]);
}

/// `Esc` during search restores the selection to its pre-search state.
#[test]
fn search_esc_restores_position() {
    let mut ed = editor_from("-[h]>ello world\n");

    ed.handle_key(key('/'));
    for ch in "world".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(state(&ed), "hello -[world]>\n");

    ed.handle_key(key_esc());
    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(state(&ed), "-[h]>ello world\n");
}

/// `n` repeats the last confirmed forward search, advancing through matches in
/// document order.
#[test]
fn search_n_repeats_forward() {
    // "ab ab ab\n" — three "ab" matches at (0,1), (3,4), (6,7).
    let mut ed = editor_from("-[a]>b ab ab\n");

    ed.handle_key(key('/'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "-[ab]> ab ab\n");

    ed.handle_key(key('n'));
    assert_eq!(state(&ed), "ab -[ab]> ab\n");

    ed.handle_key(key('n'));
    assert_eq!(state(&ed), "ab ab -[ab]>\n");
}

/// `n` always goes forward and `N` always goes backward regardless of how the search was initiated.
#[test]
fn search_n_repeats_backward() {
    let mut ed = editor_from("-[a]>b ab ab\n");

    ed.handle_key(key('/'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());
    // Advance to the second match.
    ed.handle_key(key('n'));
    assert_eq!(state(&ed), "ab -[ab]> ab\n");

    // N goes back.
    ed.handle_key(key('N'));
    assert_eq!(state(&ed), "-[ab]> ab ab\n");
}

/// After a `?` backward search, `n` still goes forward (absolute direction model).
///
/// Vim would go backward here; HUME uses absolute direction (same choice as Kakoune/Helix).
#[test]
fn search_backward_n_goes_forward() {
    // Three matches; cursor at the third. `?` lands on the second.
    let mut ed = editor_from("ab ab -[a]>b\n");

    ed.handle_key(key('?'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "ab -[ab]> ab\n");

    // n must go forward (to the third match), not backward (to the first).
    ed.handle_key(key('n'));
    assert_eq!(state(&ed), "ab ab -[ab]>\n");
}

/// `?` searches backward — the confirmed match is the last occurrence before
/// the pre-search cursor position.
#[test]
fn search_backward_confirms() {
    // Cursor at the third "ab"; backward search should land on the second.
    let mut ed = editor_from("ab ab -[a]>b\n");

    ed.handle_key(key('?'));
    assert_eq!(ed.mode, Mode::Search);

    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());

    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(state(&ed), "ab -[ab]> ab\n");
}

/// When no match exists, `n` sets the "no match" status message.
/// Confirming a search with no match returns to the pre-search position.
#[test]
fn search_no_match_behaviour() {
    let mut ed = editor_from("-[h]>ello\n");

    // Confirm a pattern that matches nothing.
    ed.handle_key(key('/'));
    ed.handle_key(key('x'));
    ed.handle_key(key('y'));
    ed.handle_key(key('z'));
    ed.handle_key(key_enter());

    assert_eq!(ed.mode, Mode::Normal);
    // Position restored to pre-search (live search restores on each no-match keystroke).
    assert_eq!(state(&ed), "-[h]>ello\n");

    // n: "no match" status message.
    ed.handle_key(key('n'));
    assert_eq!(ed.status_msg.as_deref(), Some("no match"));
}

/// Extend-search-next keeps the original anchor and moves the head to the match.
#[test]
fn extend_search_next_extends_selection() {
    // Cursor on 'h'; search forward for "world" with extend active.
    let mut ed = editor_from("-[h]>ello world\n");
    ed.mode = Mode::Extend;

    ed.handle_key(key('/'));
    for ch in "world".chars() {
        ed.handle_key(key(ch));
    }
    // Live search in extend mode: anchor stays at 0 ('h'), head moves to 10 ('d').
    assert_eq!(state(&ed), "-[hello world]>\n");

    ed.handle_key(key_enter());
    ed.mode = Mode::Normal;

    // n in extend mode: anchor stays at 0, head jumps to next match.
    ed.mode = Mode::Extend;
    // Only one "world" — wraps back to the same match.
    ed.handle_key(key('n'));
    // Selection should still cover from anchor=0 to the match end.
    assert_eq!(state(&ed), "-[hello world]>\n");
}

/// `Esc` in Normal mode clears the active search regex and its cached state.
#[test]
fn esc_in_normal_clears_search() {
    let mut ed = editor_from("-[h]>ello hello\n").with_search_regex("hello");

    assert!(
        ed.search_pattern().is_some(),
        "pre-condition: search pattern is set"
    );
    assert!(
        ed.current_search_cursor().match_count.is_some(),
        "pre-condition: cache is populated"
    );

    ed.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    ed.sync_search_cache();

    assert!(
        ed.search_pattern().is_none(),
        "search pattern should be cleared by Esc"
    );
    assert!(
        ed.current_search_cursor().match_count.is_none(),
        "match_count should be cleared by Esc"
    );
    assert!(
        ed.search_matches().matches.is_empty(),
        "matches should be cleared by Esc"
    );
}

/// `:clear-search` in Command mode clears the active search regex and its cached state.
#[test]
fn command_clear_search_clears_search() {
    let mut ed = editor_from("-[h]>ello hello\n").with_search_regex("hello");

    assert!(
        ed.search_pattern().is_some(),
        "pre-condition: search pattern is set"
    );

    // :clear-search (canonical name)
    ed.handle_key(key(':'));
    for ch in "clear-search".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    ed.sync_search_cache();

    assert_eq!(ed.mode, Mode::Normal);
    assert!(
        ed.search_pattern().is_none(),
        "search pattern should be cleared by :clear-search"
    );
    assert!(
        ed.current_search_cursor().match_count.is_none(),
        "match_count should be cleared by :clear-search"
    );
    assert!(
        ed.search_matches().matches.is_empty(),
        "matches should be cleared by :clear-search"
    );
}

// ── Select within (s) ────────────────────────────────────────────────────────

/// `s` is a noop when all selections are collapsed (anchor == head).
#[test]
fn select_within_noop_when_collapsed() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('s'));
    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.minibuf.is_none());
}

/// `s` enters Select mode, sets up minibuffer, and snapshots selections.
#[test]
fn select_within_enters_select_mode() {
    let mut ed = editor_from("-[hello world]>\n");
    ed.handle_key(key('s'));
    assert_eq!(ed.mode, Mode::Select);
    assert!(
        ed.pane_transient[ed.focused_pane_id]
            .pre_select_sels
            .is_some()
    );
    assert!(ed.minibuf.is_some());
    assert_eq!(ed.minibuf.as_ref().unwrap().prompt, '⫽');
}

/// `s` + pattern + Enter confirms: selections become matches, mode returns to Normal.
#[test]
fn select_within_confirm_replaces_selections() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    ed.handle_key(key('s'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());

    assert_eq!(ed.mode, Mode::Normal);
    assert!(
        ed.pane_transient[ed.focused_pane_id]
            .pre_select_sels
            .is_none()
    );
    // Two "ab" matches within the original selection.
    assert_eq!(ed.current_selections().len(), 2);
    assert_eq!(ed.current_selections().primary().anchor, 0);
    assert_eq!(ed.current_selections().primary().head, 1);
}

/// `s` + Esc restores original selections.
#[test]
fn select_within_esc_restores() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    let original = state(&ed);
    ed.handle_key(key('s'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    // Live preview should have changed selections.
    assert_ne!(state(&ed), original);
    ed.handle_key(key_esc());
    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(state(&ed), original);
}

/// `s` + Enter with empty pattern cancels (same as Esc).
#[test]
fn select_within_empty_confirm_cancels() {
    let mut ed = editor_from("-[hello]>\n");
    let original = state(&ed);
    ed.handle_key(key('s'));
    ed.handle_key(key_enter());
    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(state(&ed), original);
}

/// `s` does not overwrite the search register — it is a selection op, not a search.
/// A prior search pattern must survive a select-within so that n/N still works.
#[test]
fn select_within_does_not_overwrite_search_register() {
    use crate::ops::register::SEARCH_REGISTER;
    let mut ed = editor_from("-[ab cd ab]>\n");
    // Simulate a prior search by writing directly to the search register (as
    // search confirm does).
    ed.registers
        .write_text(SEARCH_REGISTER, vec!["cd".to_string()]);
    // Select within using a different pattern.
    ed.handle_key(key('s'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());
    // The search register must still hold "cd", not "ab".
    assert_eq!(reg(&ed, 's'), vec!["cd"]);
}

/// `s` does not set the search regex — highlights would be misleading
/// because they appear outside the selection scope.
#[test]
fn select_within_does_not_set_search_regex() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    ed.handle_key(key('s'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    assert!(ed.search_pattern().is_none());
}

/// `s` with no matches restores original selections on each keystroke.
#[test]
fn select_within_no_matches_keeps_originals() {
    let mut ed = editor_from("-[hello]>\n");
    let original = state(&ed);
    ed.handle_key(key('s'));
    ed.handle_key(key('z'));
    // No match for "z" in "hello" — should still show original selections.
    assert_eq!(state(&ed), original);
}

// ── select-within with multiple cursors ───────────────────────────────────────

/// Two pre-existing selections each containing matches — `s` produces one
/// result selection per match, across all original selections.
///
/// "aa bb aa\n" with two selections: [aa ] and [aa] at start/end.
/// Splitting on "aa" yields two "aa" selections, one from each original.
#[test]
fn select_within_multiple_selections_finds_matches_in_each() {
    use crate::core::selection::{Selection, SelectionSet};
    // "aa bb aa\n"
    //  0123456789
    let mut ed = editor_from("-[aa bb aa]>\n");
    // Replace with two selections: "aa " (0..2) and "aa" (6..7).
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection::new(0, 2), // "aa " — primary
            Selection::new(6, 7), // "aa"
        ],
        0,
    );
    ed.set_current_selections(two_sels);

    ed.handle_key(key('s'));
    for ch in "aa".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    // One "aa" from each original selection → 2 selections total.
    assert_eq!(ed.current_selections().len(), 2);
    // First match: chars 0..1 ("aa" in first selection).
    let sels: Vec<_> = ed.current_selections().iter_sorted().collect();
    assert_eq!(sels[0].start(), 0);
    assert_eq!(sels[0].end_inclusive(ed.doc().text()), 1);
    // Second match: chars 6..7 ("aa" in second selection).
    assert_eq!(sels[1].start(), 6);
    assert_eq!(sels[1].end_inclusive(ed.doc().text()), 7);
}

/// When one selection has matches and another does not, only the matching
/// selection produces results — the non-matching one is dropped.
#[test]
fn select_within_drops_selections_with_no_match() {
    use crate::core::selection::{Selection, SelectionSet};
    // "aa bb cc\n"
    //  01234567
    let mut ed = editor_from("-[aa bb cc]>\n");
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection::new(0, 1), // "aa" — primary, has match
            Selection::new(6, 7), // "cc" — no "aa" here
        ],
        0,
    );
    ed.set_current_selections(two_sels);

    ed.handle_key(key('s'));
    for ch in "aa".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    // Only one match (from the first selection).
    assert_eq!(ed.current_selections().len(), 1);
    assert_eq!(ed.current_selections().primary().start(), 0);
    assert_eq!(
        ed.current_selections()
            .primary()
            .end_inclusive(ed.doc().text()),
        1
    );
}

/// When NO selection contains a match, the original selections are restored.
#[test]
fn select_within_multiple_selections_no_match_restores_all() {
    use crate::core::selection::{Selection, SelectionSet};
    let mut ed = editor_from("-[aa bb cc]>\n");
    let two_sels = SelectionSet::from_vec(vec![Selection::new(0, 1), Selection::new(3, 4)], 0);
    ed.set_current_selections(two_sels.clone());

    let original = state(&ed);
    ed.handle_key(key('s'));
    ed.handle_key(key('z')); // no "z" in either selection
    // Live preview found no matches → originals already restored.
    assert_eq!(state(&ed), original);
    // Confirm with a non-empty pattern that has no matches. Live preview already
    // restored the originals, so confirm keeps them in place.
    ed.handle_key(key_enter());
    assert_eq!(
        ed.current_selections().len(),
        2,
        "original two selections should be restored"
    );
}

/// Primary index after select-within tracks to the first match within the
/// original primary selection, even when that selection is not first in order.
#[test]
fn select_within_primary_tracks_original_primary() {
    use crate::core::selection::{Selection, SelectionSet};
    // "aa bb aa\n" — two selections, primary is the SECOND one (6..7).
    let mut ed = editor_from("-[aa bb aa]>\n");
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection::new(0, 1), // first in order, NOT primary
            Selection::new(6, 7), // second in order, IS primary
        ],
        1,
    );
    ed.set_current_selections(two_sels);

    ed.handle_key(key('s'));
    for ch in "aa".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_eq!(ed.current_selections().len(), 2);
    // Primary must be the match from the original primary selection (6..7).
    let primary = ed.current_selections().primary();
    assert_eq!(
        primary.start(),
        6,
        "primary should come from the original primary selection"
    );
}

/// Esc after live-preview with multiple selections restores all originals.
#[test]
fn select_within_esc_restores_multiple_selections() {
    use crate::core::selection::{Selection, SelectionSet};
    // Use wider original selections ("aa bb" and "aa") so the live-preview
    // of "aa" visibly shrinks them — confirming the snapshot is correct.
    // "aa bb aa\n"
    //  012345678
    let mut ed = editor_from("-[aa bb aa]>\n");
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection::new(0, 4), // "aa bb" — wider than any "aa" match
            Selection::new(6, 7), // "aa"
        ],
        0,
    );
    ed.set_current_selections(two_sels);
    let original = state(&ed);

    ed.handle_key(key('s'));
    for ch in "aa".chars() {
        ed.handle_key(key(ch));
    }
    // Live preview shrinks "aa bb" → "aa", so state differs.
    assert_ne!(state(&ed), original);

    ed.handle_key(key_esc());
    assert_eq!(
        ed.current_selections().len(),
        2,
        "both original selections restored"
    );
    assert_eq!(state(&ed), original);
}

// ── Search / select-within independence ──────────────────────────────────────

/// After `/foo` + confirm, `s` + `bar` + confirm, pressing `n` must jump to the
/// next "foo" — not "bar". This is the critical end-to-end independence test.
#[test]
fn search_n_after_select_within_uses_original_search() {
    // "xx ab cd ab cd\n" — cursor starts before all matches.
    let mut ed = editor_from("-[x]>x ab cd ab cd\n");

    // Search for "ab", confirm → lands on first "ab".
    ed.handle_key(key('/'));
    for ch in "ab".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "xx -[ab]> cd ab cd\n");

    // Select the whole line and split on "cd".
    ed.handle_key(key('%'));
    ed.handle_key(key('s'));
    for ch in "cd".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    // `n` must jump to an "ab", not a "cd".
    ed.handle_key(key('n'));
    let st = state(&ed);
    assert!(
        st.contains("-[ab]>") || st.contains("<[ab]-"),
        "expected primary on 'ab', got: {st}"
    );
}

/// After `/foo` + confirm, `s` + `bar` + Esc (cancel), pressing `n` must still
/// jump to the next "foo".
#[test]
fn search_n_after_cancelled_select_within_uses_original_search() {
    let mut ed = editor_from("-[x]>x ab cd ab cd\n");

    // Search for "ab", confirm.
    ed.handle_key(key('/'));
    for ch in "ab".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "xx -[ab]> cd ab cd\n");

    // Select all, start select-within with "cd", then cancel.
    ed.handle_key(key('%'));
    ed.handle_key(key('s'));
    for ch in "cd".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_esc());

    // `n` must still find "ab".
    ed.handle_key(key('n'));
    let st = state(&ed);
    assert!(
        st.contains("-[ab]>") || st.contains("<[ab]-"),
        "expected primary on 'ab', got: {st}"
    );
}

/// A prior search pattern must survive a select-within confirm.
#[test]
fn search_regex_survives_select_within_confirm() {
    let mut ed = editor_from("-[ab cd ab]>\n").with_search_regex("cd");
    assert!(ed.search_pattern().is_some());

    ed.handle_key(key('s'));
    for ch in "ab".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert!(
        ed.search_pattern().is_some(),
        "search pattern should survive select-within confirm"
    );
}

/// A prior search pattern must survive a select-within cancel.
#[test]
fn search_regex_survives_select_within_cancel() {
    let mut ed = editor_from("-[ab cd ab]>\n").with_search_regex("cd");
    assert!(ed.search_pattern().is_some());

    ed.handle_key(key('s'));
    for ch in "ab".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_esc());

    assert!(
        ed.search_pattern().is_some(),
        "search pattern should survive select-within cancel"
    );
}

/// `s` + confirm with no prior search — pressing `n` afterward should be a
/// no-op (no crash, no match, selection unchanged).
#[test]
fn search_n_after_select_within_with_no_prior_search() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    assert!(ed.search_pattern().is_none());
    assert!(reg(&ed, 's').is_empty());

    ed.handle_key(key('s'));
    for ch in "ab".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    let before = state(&ed);
    ed.handle_key(key('n'));
    // With no search pattern, `n` is a no-op — selection unchanged.
    assert_eq!(state(&ed), before);
}

// ── n merges overlapping selections ──────────────────────────────────────────

/// When `n` moves the primary to a position already covered by a secondary
/// selection, the two must merge — no duplicate/overlapping selections.
#[test]
fn search_n_merges_with_overlapping_secondary() {
    use crate::core::selection::{Selection, SelectionSet};
    // "ab cd ab\n" — set up two selections already on the "ab" matches,
    // then confirm a search for "ab" and press `n` so the primary lands
    // on the second "ab", which is also the secondary.
    let mut ed = editor_from("-[ab cd ab]>\n");

    // Search for "ab", confirm → primary lands on first "ab".
    ed.handle_key(key('/'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "-[ab]> cd ab\n");

    // Add a secondary selection manually on the second "ab" (chars 6..7).
    let sels = SelectionSet::from_vec(
        vec![
            Selection::new(0, 1), // first "ab" — primary
            Selection::new(6, 7), // second "ab" — secondary
        ],
        0,
    );
    ed.set_current_selections(sels);
    assert_eq!(ed.current_selections().len(), 2);

    // `n` moves primary from first "ab" to second "ab", which already has a
    // secondary selection there → they must merge.
    ed.handle_key(key('n'));

    // After merge: one selection covering the second "ab".
    assert_eq!(
        ed.current_selections().len(),
        1,
        "overlapping selections must merge"
    );
    assert_eq!(ed.current_selections().primary().start(), 6);
    assert_eq!(
        ed.current_selections()
            .primary()
            .end_inclusive(ed.doc().text()),
        7
    );
}

// ── Search history ────────────────────────────────────────────────────────────

/// Helper: submit a forward search through the minibuffer.
fn search_forward(ed: &mut Editor, pattern: &str) {
    ed.handle_key(key('/'));
    for ch in pattern.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
}

/// Helper: submit a backward search.
fn search_backward(ed: &mut Editor, pattern: &str) {
    ed.handle_key(key('?'));
    for ch in pattern.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
}

#[test]
fn search_up_recalls_previous_forward_pattern() {
    let mut ed = editor_from("-[h]>ello world\n");
    search_forward(&mut ed, "foo");
    // Open forward search and press Up.
    ed.handle_key(key('/'));
    ed.handle_key(key_up());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "foo");
    ed.handle_key(key_esc());
}

#[test]
fn search_history_is_separate_from_command_history() {
    // Submit a command, then open search — command history must not bleed in.
    let mut ed = editor_from("-[h]>ello world\n");
    ed.handle_key(key(':'));
    for ch in "messages".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    // Open forward search and press Up — history should be empty.
    ed.handle_key(key('/'));
    ed.handle_key(key_up());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "");
    ed.handle_key(key_esc());
}

#[test]
fn forward_and_backward_search_histories_are_separate() {
    let mut ed = editor_from("-[h]>ello world\n");
    search_forward(&mut ed, "alpha");
    search_backward(&mut ed, "beta");
    // Forward ring only has "alpha".
    ed.handle_key(key('/'));
    ed.handle_key(key_up());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "alpha");
    ed.handle_key(key_esc());
    // Backward ring only has "beta".
    ed.handle_key(key('?'));
    ed.handle_key(key_up());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "beta");
    ed.handle_key(key_esc());
}

#[test]
fn search_recall_updates_live_preview() {
    // Buffer has two words; submit a search for the first, then recall it.
    let mut ed = editor_from("-[h]>ello world\n");
    search_forward(&mut ed, "hello");
    // Cursor should now be on "hello". Move to start so we can observe the jump.
    assert_eq!(state(&ed), "-[hello]> world\n");
    // Open search, type something else so live search moves cursor, then Up to recall.
    ed.handle_key(key('/'));
    for ch in "world".chars() {
        ed.handle_key(key(ch));
    }
    // Live search: cursor should now be on "world".
    assert_eq!(state(&ed), "hello -[world]>\n");
    // Up recalls "hello" and updates live search.
    ed.handle_key(key_up());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "hello");
    // Live preview should have jumped back to "hello".
    assert_eq!(state(&ed), "-[hello]> world\n");
    ed.handle_key(key_esc());
}

#[test]
fn search_down_walks_forward_and_restores_scratch() {
    let mut ed = editor_from("-[h]>ello world\n");
    search_forward(&mut ed, "alpha");
    search_forward(&mut ed, "beta");

    ed.handle_key(key('/'));
    // Type something as scratch before navigating.
    for ch in "typed".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "typed");

    // Up walks back: "beta", then "alpha".
    ed.handle_key(key_up());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "beta");
    ed.handle_key(key_up());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "alpha");

    // Down walks forward: "beta".
    ed.handle_key(key_down());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "beta");

    // Down past newest restores original scratch text.
    ed.handle_key(key_down());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "typed");

    // Another Down when not navigating is a no-op.
    ed.handle_key(key_down());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "typed");

    ed.handle_key(key_esc());
}

#[test]
fn search_edit_after_recall_demotes_to_scratch() {
    let mut ed = editor_from("-[h]>ello world\n");
    search_forward(&mut ed, "hello");

    // Open search, recall "hello" via Up.
    ed.handle_key(key('/'));
    ed.handle_key(key_up());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "hello");

    // Edit the recalled entry — demotes nav state so the next Up re-stashes
    // the current (now-edited) text as fresh scratch.
    ed.handle_key(key('x')); // input is now "hellox"
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "hellox");

    // Up: stashes "hellox" as scratch, recalls "hello".
    ed.handle_key(key_up());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "hello");

    // Down past newest: restores "hellox" (the edited scratch).
    ed.handle_key(key_down());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "hellox");

    ed.handle_key(key_esc());
}

/// Backspace on an empty search input (second Backspace in the sequence
/// `/f` → Backspace → Backspace) must dismiss and return to Normal.
///
/// Mirrors command mode: `EmptiedByBackspace` stays open; `BackspaceOnEmpty`
/// cancels.
#[test]
fn search_backspace_on_empty_dismisses() {
    let mut ed = editor_from("-[h]>ello world\n");

    // /f → Backspace: EmptiedByBackspace — input empty, but stay in Search.
    ed.handle_key(key('/'));
    ed.handle_key(key('f'));
    ed.handle_key(key_backspace());
    assert_eq!(ed.mode, Mode::Search, "first Backspace must keep Search open");
    assert!(ed.minibuf.is_some());
    assert_eq!(state(&ed), "-[h]>ello world\n"); // snapshot restored

    // Second Backspace: BackspaceOnEmpty — dismiss.
    ed.handle_key(key_backspace());
    assert_eq!(ed.mode, Mode::Normal, "second Backspace must dismiss");
    assert!(ed.minibuf.is_none());
    assert_eq!(state(&ed), "-[h]>ello world\n");
}

/// Backspace when the search input was empty from the start must dismiss
/// immediately (no EmptiedByBackspace intermediate step).
#[test]
fn search_backspace_on_empty_from_start_dismisses() {
    let mut ed = editor_from("-[h]>ello world\n");

    ed.handle_key(key('/'));
    ed.handle_key(key_backspace()); // BackspaceOnEmpty right away
    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.minibuf.is_none());
    assert_eq!(state(&ed), "-[h]>ello world\n");
}
