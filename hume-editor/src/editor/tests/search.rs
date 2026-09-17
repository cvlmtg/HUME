use super::*;
use pretty_assertions::assert_eq;

// ── Search ────────────────────────────────────────────────────────────────────

/// `/` opens Search mode; typing a pattern triggers live search; `Enter` confirms
/// the match and writes the pattern to the `'s'` register.
#[test]
fn search_forward_enter_confirms() {
    let mut ed = editor_from("-[h]>ello world\n");

    ed.handle_key(key('/'));
    assert_eq!(ed.state.mode(), Mode::Search);

    for ch in "world".chars() {
        ed.handle_key(key(ch));
    }
    // Live search has already moved the selection to "world".
    assert_eq!(state(&ed), "hello -[world]>\n");

    ed.handle_key(key_enter());
    assert_eq!(ed.state.mode(), Mode::Normal);
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
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(state(&ed), "-[h]>ello world\n");
}

/// Re-entering search (`search-backward` reached, e.g. via a hook or timer's
/// `(call! "search-backward")`, while a `/` session is still open — there's
/// no key path for this, since `?` typed into an open `/` prompt is just a
/// literal character) must replace the session rather than no-op, and must
/// stash the *true* pre-search state, not the mid-`/`-session preview
/// selection: `push_mode_layer`'s truncate runs the outgoing `Search`
/// layer's own `tear_down` first, restoring `pre_sels` to what it held
/// before the first session, and only then does the new stash read
/// `current_selections`.
///
/// Fail oracle: `push_mode_layer` only no-ops on same-kind re-entry for
/// `Insert` — if `Search` were made to no-op the same way, this would leave
/// the `/` prompt and its stash exactly as they were, and the assertion on
/// `?` below would fail. Stashing *before* the push instead of in
/// `SearchLayer::setup` would capture the mid-`/`-search preview selection
/// instead, and cancelling would land on `"world"`, not the original
/// pre-search position.
#[test]
fn search_backward_reentry_while_forward_search_open_replaces_and_restashes() {
    use crate::editor::commands::cmd_search_backward;
    use hume_ops::MotionMode;

    let mut ed = editor_from("-[h]>ello world\n");
    ed.handle_key(key('/'));
    for ch in "world".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(
        state(&ed),
        "hello -[world]>\n",
        "sanity: live `/` search moved the selection"
    );

    cmd_search_backward(&mut ed.state, &mut ed.view, 1, MotionMode::Move).unwrap();
    assert_eq!(
        ed.state.minibuf().unwrap().prompt,
        "?",
        "must replace the `/` session with a fresh `?` one, not no-op"
    );

    ed.handle_key(key_esc());
    assert_eq!(
        state(&ed),
        "-[h]>ello world\n",
        "cancel must restore the true pre-`/`-search position, not the mid-`/`-search preview"
    );
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
    assert_eq!(ed.state.mode(), Mode::Search);

    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());

    assert_eq!(ed.state.mode(), Mode::Normal);
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

    assert_eq!(ed.state.mode(), Mode::Normal);
    // Position restored to pre-search (live search restores on each no-match keystroke).
    assert_eq!(state(&ed), "-[h]>ello\n");

    // n: "no match" status message.
    ed.handle_key(key('n'));
    assert_eq!(ed.state.status_msg.as_deref(), Some("no match"));
    // "no match" is a boundary condition, not a failure — it must not reach
    // `:messages` or raise the unread-message statusline nudge.
    assert_eq!(ed.state.message_log.totals(), (0, 0));
    assert!(!ed.state.message_log.has_unseen());
}

/// Extend-search-next keeps the original anchor and moves the head to the match.
#[test]
fn extend_search_next_extends_selection() {
    // Cursor on 'h'; search forward for "world" with extend active.
    let mut ed = editor_from("-[h]>ello world\n");
    ed.state.input.set_extend(true);

    ed.handle_key(key('/'));
    for ch in "world".chars() {
        ed.handle_key(key(ch));
    }
    // Live search in extend mode: anchor stays at 0 ('h'), head moves to 10 ('d').
    assert_eq!(state(&ed), "-[hello world]>\n");

    ed.handle_key(key_enter());

    // n in extend mode: anchor stays at 0, head jumps to next match.
    ed.state.input.set_extend(true);
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

    ed.handle_key(KeyEvent::new(KeyCode::Escape, Modifiers::NONE));
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

/// `clear-search`, dispatched as a key command (it's not typed — `Esc` is
/// its usual trigger), clears the active search regex and its cached state.
#[test]
fn clear_search_command_clears_search() {
    let mut ed = editor_from("-[h]>ello hello\n").with_search_regex("hello");

    assert!(
        ed.search_pattern().is_some(),
        "pre-condition: search pattern is set"
    );

    ed.execute_keymap_command("clear-search".into(), Some(1), false);
    ed.sync_search_cache();

    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(
        ed.search_pattern().is_none(),
        "search pattern should be cleared by clear-search"
    );
    assert!(
        ed.current_search_cursor().match_count.is_none(),
        "match_count should be cleared by clear-search"
    );
    assert!(
        ed.search_matches().matches.is_empty(),
        "matches should be cleared by clear-search"
    );
}

// ── Sift within (s) ────────────────────────────────────────────────────────

/// `s` is a noop when all selections are collapsed (anchor == head).
#[test]
fn sift_within_noop_when_collapsed() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('s'));
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(ed.state.minibuf().is_none());
}

/// `s` enters Sift mode, sets up minibuffer, and snapshots selections.
#[test]
fn sift_within_enters_sift_mode() {
    let mut ed = editor_from("-[hello world]>\n");
    ed.handle_key(key('s'));
    assert_eq!(ed.state.mode(), Mode::Sift);
    assert!(
        ed.state
            .input
            .find::<crate::editor::input_stack::SiftLayer>()
            .unwrap()
            .snap
            .selections()
            .is_some()
    );
    assert!(ed.state.minibuf().is_some());
    assert_eq!(ed.state.minibuf().unwrap().prompt, "⫽");
}

/// `s` + pattern + Enter confirms: selections become matches, mode returns to Normal.
#[test]
fn sift_within_confirm_replaces_selections() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    ed.handle_key(key('s'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());

    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(
        ed.state
            .input
            .find::<crate::editor::input_stack::SiftLayer>()
            .is_none(),
        "the Sift layer itself is gone once Normal mode takes over"
    );
    // Two "ab" matches within the original selection.
    assert_eq!(ed.current_selections().len(), 2);
    assert_eq!(ed.current_selections().primary().anchor(), co(0));
    assert_eq!(ed.current_selections().primary().head(), co(1));
}

/// `s` + Esc restores original selections.
#[test]
fn sift_within_esc_restores() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    let original = state(&ed);
    ed.handle_key(key('s'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    // Live preview should have changed selections.
    assert_ne!(state(&ed), original);
    ed.handle_key(key_esc());
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(state(&ed), original);
}

/// `s` + Enter with empty pattern cancels (same as Esc).
#[test]
fn sift_within_empty_confirm_cancels() {
    let mut ed = editor_from("-[hello]>\n");
    let original = state(&ed);
    ed.handle_key(key('s'));
    ed.handle_key(key_enter());
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(state(&ed), original);
}

/// `s` does not overwrite the search register — it is a selection op, not a search.
/// A prior search pattern must survive a sift-within so that n/N still works.
#[test]
fn sift_within_does_not_overwrite_search_register() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    // Simulate a prior search by writing directly to the search register (as
    // search confirm does).
    ed.state.registers.set_search_register("cd".to_string());
    // Sift within using a different pattern.
    ed.handle_key(key('s'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    ed.handle_key(key_enter());
    // The search register must still hold "cd", not "ab".
    assert_eq!(ed.state.registers.search_register(), Some("cd"));
}

/// `s` does not set the search regex — highlights would be misleading
/// because they appear outside the selection scope.
#[test]
fn sift_within_does_not_set_search_regex() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    ed.handle_key(key('s'));
    ed.handle_key(key('a'));
    ed.handle_key(key('b'));
    assert!(ed.search_pattern().is_none());
}

/// `s` with no matches restores original selections on each keystroke.
#[test]
fn sift_within_no_matches_keeps_originals() {
    let mut ed = editor_from("-[hello]>\n");
    let original = state(&ed);
    ed.handle_key(key('s'));
    ed.handle_key(key('z'));
    // No match for "z" in "hello" — should still show original selections.
    assert_eq!(state(&ed), original);
}

// ── sift-within with multiple cursors ───────────────────────────────────────

/// Two pre-existing selections each containing matches — `s` produces one
/// result selection per match, across all original selections.
///
/// "aa bb aa\n" with two selections: [aa ] and [aa] at start/end.
/// Splitting on "aa" yields two "aa" selections, one from each original.
#[test]
fn sift_within_multiple_selections_finds_matches_in_each() {
    use hume_editing::selection::{Selection, SelectionSet};
    // "aa bb aa\n"
    //  0123456789
    let mut ed = editor_from("-[aa bb aa]>\n");
    // Replace with two selections: "aa " (0..2) and "aa" (6..7).
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection::new(co(0), co(2)), // "aa " — primary
            Selection::new(co(6), co(7)), // "aa"
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
    assert_eq!(sels[0].start(), co(0));
    assert_eq!(sels[0].end_inclusive(ed.doc().text()), co(1));
    // Second match: chars 6..7 ("aa" in second selection).
    assert_eq!(sels[1].start(), co(6));
    assert_eq!(sels[1].end_inclusive(ed.doc().text()), co(7));
}

/// When one selection has matches and another does not, only the matching
/// selection produces results — the non-matching one is dropped.
#[test]
fn sift_within_drops_selections_with_no_match() {
    use hume_editing::selection::{Selection, SelectionSet};
    // "aa bb cc\n"
    //  01234567
    let mut ed = editor_from("-[aa bb cc]>\n");
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection::new(co(0), co(1)), // "aa" — primary, has match
            Selection::new(co(6), co(7)), // "cc" — no "aa" here
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
    assert_eq!(ed.current_selections().primary().start(), co(0));
    assert_eq!(
        ed.current_selections()
            .primary()
            .end_inclusive(ed.doc().text()),
        co(1)
    );
}

/// When NO selection contains a match, the original selections are restored.
#[test]
fn sift_within_multiple_selections_no_match_restores_all() {
    use hume_editing::selection::{Selection, SelectionSet};
    let mut ed = editor_from("-[aa bb cc]>\n");
    let two_sels = SelectionSet::from_vec(
        vec![Selection::new(co(0), co(1)), Selection::new(co(3), co(4))],
        0,
    );
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

/// Primary index after sift-within tracks to the first match within the
/// original primary selection, even when that selection is not first in order.
#[test]
fn sift_within_primary_tracks_original_primary() {
    use hume_editing::selection::{Selection, SelectionSet};
    // "aa bb aa\n" — two selections, primary is the SECOND one (6..7).
    let mut ed = editor_from("-[aa bb aa]>\n");
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection::new(co(0), co(1)), // first in order, NOT primary
            Selection::new(co(6), co(7)), // second in order, IS primary
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
        co(6),
        "primary should come from the original primary selection"
    );
}

/// Esc after live-preview with multiple selections restores all originals.
#[test]
fn sift_within_esc_restores_multiple_selections() {
    use hume_editing::selection::{Selection, SelectionSet};
    // Use wider original selections ("aa bb" and "aa") so the live-preview
    // of "aa" visibly shrinks them — confirming the snapshot is correct.
    // "aa bb aa\n"
    //  012345678
    let mut ed = editor_from("-[aa bb aa]>\n");
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection::new(co(0), co(4)), // "aa bb" — wider than any "aa" match
            Selection::new(co(6), co(7)), // "aa"
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

// ── Search / sift-within independence ──────────────────────────────────────

/// After `/foo` + confirm, `s` + `bar` + confirm, pressing `n` must jump to the
/// next "foo" — not "bar". This is the critical end-to-end independence test.
#[test]
fn search_n_after_sift_within_uses_original_search() {
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
fn search_n_after_cancelled_sift_within_uses_original_search() {
    let mut ed = editor_from("-[x]>x ab cd ab cd\n");

    // Search for "ab", confirm.
    ed.handle_key(key('/'));
    for ch in "ab".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert_eq!(state(&ed), "xx -[ab]> cd ab cd\n");

    // Select all, start sift-within with "cd", then cancel.
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

/// A prior search pattern must survive a sift-within confirm.
#[test]
fn search_regex_survives_sift_within_confirm() {
    let mut ed = editor_from("-[ab cd ab]>\n").with_search_regex("cd");
    assert!(ed.search_pattern().is_some());

    ed.handle_key(key('s'));
    for ch in "ab".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert!(
        ed.search_pattern().is_some(),
        "search pattern should survive sift-within confirm"
    );
}

/// A prior search pattern must survive a sift-within cancel.
#[test]
fn search_regex_survives_sift_within_cancel() {
    let mut ed = editor_from("-[ab cd ab]>\n").with_search_regex("cd");
    assert!(ed.search_pattern().is_some());

    ed.handle_key(key('s'));
    for ch in "ab".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_esc());

    assert!(
        ed.search_pattern().is_some(),
        "search pattern should survive sift-within cancel"
    );
}

/// `s` + confirm with no prior search — pressing `n` afterward should be a
/// no-op (no crash, no match, selection unchanged).
#[test]
fn search_n_after_sift_within_with_no_prior_search() {
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

/// Sift shares the search prompt's flag grammar: `v` (verbatim) literalizes
/// the pattern, so `.` matches only a literal dot within the selection.
#[test]
fn sift_within_verbatim_flag_matches_literal_dot() {
    // "a.b axb\n" selected whole — non-verbatim "." would additionally match
    // the "x" in "axb".
    let mut ed = editor_from("-[a.b axb]>\n");
    ed.handle_key(key('s'));
    for ch in "v/a.b".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert_eq!(ed.current_selections().len(), 1);
    let text = ed.doc().text();
    let primary = ed.current_selections().primary();
    assert_eq!(
        (primary.start(), primary.end_inclusive(text)),
        (co(0), co(2))
    );
}

/// `m` (multi) parses at the sift prompt but is inert — sift already
/// operates on every selection, so it changes nothing.
#[test]
fn sift_within_multi_flag_is_inert() {
    // "aa bb aa\n" — same setup as
    // `sift_within_multiple_selections_finds_matches_in_each`, but with the
    // (meaningless here) `m` flag prefixed onto the pattern.
    use hume_editing::selection::{Selection, SelectionSet};
    let mut ed = editor_from("-[aa bb aa]>\n");
    let two_sels = SelectionSet::from_vec(
        vec![Selection::new(co(0), co(2)), Selection::new(co(6), co(7))],
        0,
    );
    ed.set_current_selections(two_sels);

    ed.handle_key(key('s'));
    for ch in "m/aa".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_eq!(ed.current_selections().len(), 2);
    let sels: Vec<_> = ed.current_selections().iter_sorted().collect();
    let text = ed.doc().text();
    assert_eq!(
        (sels[0].start(), sels[0].end_inclusive(text)),
        (co(0), co(1))
    );
    assert_eq!(
        (sels[1].start(), sels[1].end_inclusive(text)),
        (co(6), co(7))
    );
}

// ── n merges overlapping selections ──────────────────────────────────────────

/// When `n` moves the primary to a position already covered by a secondary
/// selection, the two must merge — no duplicate/overlapping selections.
#[test]
fn search_n_merges_with_overlapping_secondary() {
    use hume_editing::selection::{Selection, SelectionSet};
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
            Selection::new(co(0), co(1)), // first "ab" — primary
            Selection::new(co(6), co(7)), // second "ab" — secondary
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
    assert_eq!(ed.current_selections().primary().start(), co(6));
    assert_eq!(
        ed.current_selections()
            .primary()
            .end_inclusive(ed.doc().text()),
        co(7)
    );
}

// ── Multi-selection (`m`) and verbatim (`v`) flags ─────────────────────────────

/// `/m/bar` — every selection independently moves to its own next "bar",
/// not just the primary.
///
/// "aaa bar bbb bar ccc bar\n" with collapsed selections on the first char of
/// each "aaa"/"bbb"/"ccc" run — each has exactly one "bar" ahead of it before
/// the next run's cursor, so the three results are distinguishable.
#[test]
fn multi_flag_moves_every_selection_to_its_own_next_match() {
    use hume_editing::selection::{Selection, SelectionSet};
    let mut ed = editor_from("-[a]>aa bar bbb bar ccc bar\n");
    let sels = SelectionSet::from_vec(
        vec![
            Selection::new(co(0), co(0)),   // "aaa" — primary
            Selection::new(co(8), co(8)),   // "bbb"
            Selection::new(co(16), co(16)), // "ccc"
        ],
        0,
    );
    ed.set_current_selections(sels);

    search_forward(&mut ed, "m/bar");

    assert_eq!(reg(&ed, 's'), vec!["m/bar"]);
    assert_eq!(ed.current_selections().len(), 3);
    let sels: Vec<_> = ed.current_selections().iter_sorted().collect();
    let text = ed.doc().text();
    assert_eq!(
        (sels[0].start(), sels[0].end_inclusive(text)),
        (co(4), co(6))
    );
    assert_eq!(
        (sels[1].start(), sels[1].end_inclusive(text)),
        (co(12), co(14))
    );
    assert_eq!(
        (sels[2].start(), sels[2].end_inclusive(text)),
        (co(20), co(22))
    );
}

/// Without `m`, a search (and a follow-up `n`) moves only the primary
/// selection — secondaries keep their exact prior anchor and head.
#[test]
fn no_flags_search_and_n_move_only_primary() {
    use hume_editing::selection::{Selection, SelectionSet};
    let mut ed = editor_from("-[a]>aa bar bbb bar ccc bar\n");
    let sels = SelectionSet::from_vec(
        vec![
            Selection::new(co(0), co(0)),   // "aaa" — primary
            Selection::new(co(8), co(8)),   // "bbb" — untouched
            Selection::new(co(16), co(16)), // "ccc" — untouched
        ],
        0,
    );
    ed.set_current_selections(sels);

    search_forward(&mut ed, "bar");

    assert_eq!(ed.current_selections().len(), 3);
    let sorted: Vec<_> = ed.current_selections().iter_sorted().collect();
    let text = ed.doc().text();
    assert_eq!(
        (sorted[0].start(), sorted[0].end_inclusive(text)),
        (co(4), co(6)),
        "primary moved to first \"bar\""
    );
    assert_eq!(sorted[1].start(), co(8), "secondary untouched");
    assert_eq!(sorted[2].start(), co(16), "secondary untouched");

    // A follow-up `n` (no `m` flag stored) still moves only the primary —
    // read it back via `.primary()`, since moving past a secondary's
    // position changes document-order (`iter_sorted`) placement.
    ed.handle_key(key('n'));
    assert_eq!(ed.current_selections().len(), 3);
    let text = ed.doc().text();
    let primary = ed.current_selections().primary();
    assert_eq!(
        (primary.start(), primary.end_inclusive(text)),
        (co(12), co(14)),
        "n moved primary to second \"bar\""
    );
    let starts: Vec<_> = ed
        .current_selections()
        .iter_sorted()
        .map(hume_editing::selection::Selection::start)
        .collect();
    assert_eq!(
        starts,
        vec![co(8), co(12), co(16)],
        "the two untouched secondaries (8, 16) are unchanged; only the primary moved"
    );
}

/// `n` after a multi search keeps moving every selection — the `m` flag is
/// stored with the pattern and inherited by repeat, not a one-shot effect.
#[test]
fn multi_flag_persists_across_n() {
    use hume_editing::selection::{Selection, SelectionSet};
    let mut ed = editor_from("-[a]>aa bar bbb bar ccc bar\n");
    let sels = SelectionSet::from_vec(
        vec![
            Selection::new(co(0), co(0)),
            Selection::new(co(8), co(8)),
            Selection::new(co(16), co(16)),
        ],
        0,
    );
    ed.set_current_selections(sels);

    search_forward(&mut ed, "m/bar");
    // All three now sit on the three "bar" occurrences.
    assert_eq!(ed.current_selections().len(), 3);

    ed.handle_key(key('n'));
    // Each selection independently steps to the buffer's next "bar",
    // wrapping where needed — the cyclic permutation of the same three
    // matches, still three distinct selections.
    assert_eq!(ed.current_selections().len(), 3);
    let sorted: Vec<_> = ed.current_selections().iter_sorted().collect();
    let text = ed.doc().text();
    let ranges: Vec<_> = sorted
        .iter()
        .map(|s| (s.start(), s.end_inclusive(text)))
        .collect();
    assert_eq!(
        ranges,
        vec![(co(4), co(6)), (co(12), co(14)), (co(20), co(22))]
    );
}

/// `N` after a multi search moves every selection backward independently —
/// direction is not a one-shot property of the confirm keystroke either.
#[test]
fn multi_flag_backward_capital_n() {
    use hume_editing::selection::{Selection, SelectionSet};
    // "bar aaa bar bbb bar\n" — three "bar" matches at (0,2), (8,10), (16,18).
    let mut ed = editor_from("-[b]>ar aaa bar bbb bar\n");
    let sels = SelectionSet::from_vec(
        vec![
            Selection::new(co(6), co(6)),   // inside "aaa" — primary
            Selection::new(co(14), co(14)), // inside "bbb"
        ],
        0,
    );
    ed.set_current_selections(sels);

    search_forward(&mut ed, "m/bar");
    // Forward multi search: co(6) -> (8,10), co(14) -> (16,18).
    let text = ed.doc().text();
    let sorted: Vec<_> = ed.current_selections().iter_sorted().collect();
    assert_eq!(
        (sorted[0].start(), sorted[0].end_inclusive(text)),
        (co(8), co(10))
    );
    assert_eq!(
        (sorted[1].start(), sorted[1].end_inclusive(text)),
        (co(16), co(18))
    );

    ed.handle_key(key('N'));
    // Each selection independently steps backward to the previous "bar":
    // (8,10) -> (0,2), (16,18) -> (8,10).
    let text = ed.doc().text();
    let sorted: Vec<_> = ed.current_selections().iter_sorted().collect();
    assert_eq!(sorted.len(), 2);
    assert_eq!(
        (sorted[0].start(), sorted[0].end_inclusive(text)),
        (co(0), co(2))
    );
    assert_eq!(
        (sorted[1].start(), sorted[1].end_inclusive(text)),
        (co(8), co(10))
    );
}

/// A count prefix on `n` (e.g. `2n`) chains that many hops per selection
/// under the multi flag too, not just for the primary.
#[test]
fn multi_flag_count_prefix_on_n() {
    use hume_editing::selection::{Selection, SelectionSet};
    // "x bar y bar z bar w bar\n" — four "bar" matches: (2,4),(8,10),(14,16),(20,22).
    let mut ed = editor_from("-[x]> bar y bar z bar w bar\n");
    let sels = SelectionSet::from_vec(
        vec![
            Selection::new(co(0), co(0)), // before the buffer's first "bar" — primary
            Selection::new(co(6), co(6)), // on "y", before the second "bar"
        ],
        0,
    );
    ed.set_current_selections(sels);

    search_forward(&mut ed, "m/bar");
    // Confirm (count 1): co(0) -> (2,4), co(6) -> (8,10).
    let text = ed.doc().text();
    let sorted: Vec<_> = ed.current_selections().iter_sorted().collect();
    assert_eq!(
        (sorted[0].start(), sorted[0].end_inclusive(text)),
        (co(2), co(4))
    );
    assert_eq!(
        (sorted[1].start(), sorted[1].end_inclusive(text)),
        (co(8), co(10))
    );

    ed.handle_key(key('2'));
    ed.handle_key(key('n'));
    // Each selection independently hops two matches forward:
    // (2,4) -> (8,10) -> (14,16); (8,10) -> (14,16) -> (20,22).
    let text = ed.doc().text();
    let sorted: Vec<_> = ed.current_selections().iter_sorted().collect();
    assert_eq!(sorted.len(), 2);
    assert_eq!(
        (sorted[0].start(), sorted[0].end_inclusive(text)),
        (co(14), co(16))
    );
    assert_eq!(
        (sorted[1].start(), sorted[1].end_inclusive(text)),
        (co(20), co(22))
    );
}

/// When no selection has a match anywhere in the buffer under a multi
/// search, `n` reports the same "no match" transient the single-selection
/// path does, and leaves every selection untouched.
#[test]
fn multi_flag_no_selection_matches_reports_transient() {
    use hume_editing::selection::{Selection, SelectionSet};
    let mut ed = editor_from("-[a]>aa bbb\n");
    let sels = SelectionSet::from_vec(
        vec![Selection::new(co(0), co(0)), Selection::new(co(4), co(4))],
        0,
    );
    ed.set_current_selections(sels);

    search_forward(&mut ed, "m/zzz");
    let before = state(&ed);

    ed.handle_key(key('n'));
    assert_eq!(ed.state.status_msg.as_deref(), Some("no match"));
    assert_eq!(state(&ed), before, "selections must be left untouched");
}

/// A pattern typed with a leading `/` (e.g. a path) is not read as an empty
/// flag run — the flag run must be non-empty, so `/usr` is the literal
/// pattern "/usr", not flags "" + pattern "usr".
#[test]
fn leading_slash_is_part_of_the_pattern() {
    let mut ed = editor_from("-[x]> /usr y\n");
    ed.handle_key(key('/'));
    for ch in "/usr".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(state(&ed), "x -[/usr]> y\n");
    ed.handle_key(key_enter());
    assert_eq!(reg(&ed, 's'), vec!["/usr"]);
}

/// A pattern that reads as flags (a flag letter run followed by `/`) has no
/// literal spelling of its own — `v/` reaches it instead: `v/m/s` matches
/// the literal text "m/s", the same text a bare `m/s` would instead read as
/// the multi flag plus pattern "s".
#[test]
fn verbatim_flag_reaches_a_pattern_that_looks_like_flags() {
    let mut ed = editor_from("-[x]> m/s y\n");
    ed.handle_key(key('/'));
    for ch in "v/m/s".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(state(&ed), "x -[m/s]> y\n");
    ed.handle_key(key_enter());
    assert_eq!(reg(&ed, 's'), vec!["v/m/s"]);
}

/// `mv` combines multi and verbatim: `.` must match only a literal dot (not
/// any character), and every selection moves independently — selections that
/// converge on the same literal match merge into one.
///
/// "axrs a.rs\n": without `verbatim`, the pattern `.rs` would match "xrs" as
/// a wildcard-`.` hit at (1,3); `verbatim` forces both selections past it to
/// the literal ".rs" at (6,8), where they converge.
#[test]
fn multi_verbatim_flags_combine_and_converging_selections_merge() {
    use hume_editing::selection::{Selection, SelectionSet};
    let mut ed = editor_from("-[a]>xrs a.rs\n");
    let sels = SelectionSet::from_vec(
        vec![
            Selection::new(co(0), co(0)), // "axrs" — primary
            Selection::new(co(5), co(5)), // "a.rs"
        ],
        0,
    );
    ed.set_current_selections(sels);

    search_forward(&mut ed, "mv/.rs");

    assert_eq!(
        ed.current_selections().len(),
        1,
        "both selections converge on the one literal \".rs\" match"
    );
    let text = ed.doc().text();
    let primary = ed.current_selections().primary();
    assert_eq!(
        (primary.start(), primary.end_inclusive(text)),
        (co(6), co(8))
    );
}

/// Multi + Extend: each selection extends from its own anchor, not a shared one.
#[test]
fn multi_extend_extends_each_selection_from_its_own_anchor() {
    use hume_editing::selection::{Selection, SelectionSet};
    // "aa foo bb foo\n"
    let mut ed = editor_from("-[a]>a foo bb foo\n");
    let sels = SelectionSet::from_vec(
        vec![
            Selection::new(co(0), co(1)), // "aa" — anchor 0, primary
            Selection::new(co(7), co(8)), // "bb" — anchor 7
        ],
        0,
    );
    ed.set_current_selections(sels);
    ed.state.input.set_extend(true);

    search_forward(&mut ed, "m/foo");

    assert_eq!(ed.current_selections().len(), 2);
    let sorted: Vec<_> = ed.current_selections().iter_sorted().collect();
    assert_eq!(
        sorted[0].anchor(),
        co(0),
        "first selection keeps its own anchor"
    );
    assert_eq!(sorted[0].head(), co(5));
    assert_eq!(
        sorted[1].anchor(),
        co(7),
        "second selection keeps its own, different anchor"
    );
    assert_eq!(sorted[1].head(), co(12));
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
    assert_eq!(ed.state.minibuf().unwrap().input, "foo");
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
    assert_eq!(ed.state.minibuf().unwrap().input, "");
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
    assert_eq!(ed.state.minibuf().unwrap().input, "alpha");
    ed.handle_key(key_esc());
    // Backward ring only has "beta".
    ed.handle_key(key('?'));
    ed.handle_key(key_up());
    assert_eq!(ed.state.minibuf().unwrap().input, "beta");
    ed.handle_key(key_esc());
}

#[test]
fn search_recall_updates_live_preview() {
    // Buffer has two words; submit a search for the first, then recall it.
    let mut ed = editor_from("-[h]>ello world\n");
    search_forward(&mut ed, "hello");
    // Cursor should now be on "hello". Move to start so we can observe the jump.
    assert_eq!(state(&ed), "-[hello]> world\n");
    // Open search, type a one-char prefix of the stored pattern — live search
    // matches on the partial pattern, then Up recalls the full stored entry
    // (it starts with "h") and re-runs the live preview off the recall.
    ed.handle_key(key('/'));
    ed.handle_key(key('h'));
    // Live search: partial match highlights just "h".
    assert_eq!(state(&ed), "-[h]>ello world\n");
    // Up recalls "hello" and updates live search to the full match.
    ed.handle_key(key_up());
    assert_eq!(ed.state.minibuf().unwrap().input, "hello");
    assert_eq!(state(&ed), "-[hello]> world\n");
    ed.handle_key(key_esc());
}

#[test]
fn search_down_walks_forward_and_restores_scratch() {
    let mut ed = editor_from("-[h]>ello world\n");
    // Both entries share the "al" prefix the user types below.
    search_forward(&mut ed, "alphabet");
    search_forward(&mut ed, "alpine");

    ed.handle_key(key('/'));
    // Type a prefix matching both entries as scratch before navigating.
    for ch in "al".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(ed.state.minibuf().unwrap().input, "al");

    // Up walks back: "alpine", then "alphabet".
    ed.handle_key(key_up());
    assert_eq!(ed.state.minibuf().unwrap().input, "alpine");
    ed.handle_key(key_up());
    assert_eq!(ed.state.minibuf().unwrap().input, "alphabet");

    // Down walks forward: "alpine".
    ed.handle_key(key_down());
    assert_eq!(ed.state.minibuf().unwrap().input, "alpine");

    // Down past newest restores original scratch text.
    ed.handle_key(key_down());
    assert_eq!(ed.state.minibuf().unwrap().input, "al");

    // Another Down when not navigating is a no-op.
    ed.handle_key(key_down());
    assert_eq!(ed.state.minibuf().unwrap().input, "al");

    ed.handle_key(key_esc());
}

#[test]
fn search_edit_after_recall_demotes_to_scratch() {
    let mut ed = editor_from("-[h]>ello world\n");
    // Oldest first: only "helloxyz" prefix-matches the "hellox" text the
    // user will type onto the recalled "hello" below.
    search_forward(&mut ed, "helloxyz");
    search_forward(&mut ed, "hellfire");
    search_forward(&mut ed, "hello");

    // Open search, recall "hello" via Up (empty prefix matches newest).
    ed.handle_key(key('/'));
    ed.handle_key(key_up());
    assert_eq!(ed.state.minibuf().unwrap().input, "hello");

    // Edit the recalled entry — demotes nav state so the next Up re-stashes
    // the current (now-edited) text as fresh scratch.
    ed.handle_key(key('x')); // input is now "hellox"
    assert_eq!(ed.state.minibuf().unwrap().input, "hellox");

    // Up: stashes "hellox" as scratch, recalls the only match: "helloxyz".
    ed.handle_key(key_up());
    assert_eq!(ed.state.minibuf().unwrap().input, "helloxyz");

    // Down past newest match: restores "hellox" (the edited scratch).
    ed.handle_key(key_down());
    assert_eq!(ed.state.minibuf().unwrap().input, "hellox");

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
    assert_eq!(
        ed.state.mode(),
        Mode::Search,
        "first Backspace must keep Search open"
    );
    assert!(ed.state.minibuf().is_some());
    assert_eq!(state(&ed), "-[h]>ello world\n"); // snapshot restored

    // Second Backspace: BackspaceOnEmpty — dismiss.
    ed.handle_key(key_backspace());
    assert_eq!(
        ed.state.mode(),
        Mode::Normal,
        "second Backspace must dismiss"
    );
    assert!(ed.state.minibuf().is_none());
    assert_eq!(state(&ed), "-[h]>ello world\n");
}

/// Backspace when the search input was empty from the start must dismiss
/// immediately (no EmptiedByBackspace intermediate step).
#[test]
fn search_backspace_on_empty_from_start_dismisses() {
    let mut ed = editor_from("-[h]>ello world\n");

    ed.handle_key(key('/'));
    ed.handle_key(key_backspace()); // BackspaceOnEmpty right away
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(ed.state.minibuf().is_none());
    assert_eq!(state(&ed), "-[h]>ello world\n");
}
