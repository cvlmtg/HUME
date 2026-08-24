use super::*;
use pretty_assertions::assert_eq;

// ── select-all-matches ────────────────────────────────────────────────────────

/// `select-all-matches` turns every match into a selection.
#[test]
fn select_all_matches_creates_selection_per_match() {
    // "ab cd ab\n" — two "ab" matches at 0 and 6.
    let mut ed = editor_from("-[a]>b cd ab\n").with_search_regex("ab");

    ed.handle_key(key(':'));
    for ch in "select-all-matches".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_eq!(
        ed.current_selections().len(),
        2,
        "one selection per 'ab' match"
    );
    let sels: Vec<_> = ed.current_selections().iter_sorted().collect();
    assert_eq!(sels[0].start(), 0);
    assert_eq!(sels[1].start(), 6);
}

/// `select-all-matches` with no active search is a no-op.
#[test]
fn select_all_matches_no_search_is_noop() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    let original = state(&ed);

    ed.handle_key(key(':'));
    for ch in "select-all-matches".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_eq!(state(&ed), original);
}

/// `select-all-matches` falls back to the search register when regex is cleared.
#[test]
fn select_all_matches_uses_search_register_fallback() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    ed.state.registers.set_search_register("ab".to_string());
    // No live regex — forces register fallback.
    assert!(ed.search_pattern().is_none());

    ed.handle_key(key(':'));
    for ch in "select-all-matches".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_eq!(ed.current_selections().len(), 2);
}

/// `m/` keybind reaches `select-all-matches` (tests the keymap path, not just `:select-all-matches`).
#[test]
fn select_all_matches_via_m_slash_keybind() {
    let mut ed = editor_from("-[a]>b cd ab\n").with_search_regex("ab");
    ed.handle_key(key('m'));
    ed.handle_key(key('/'));
    assert_eq!(
        ed.current_selections().len(),
        2,
        "m/ should select all 'ab' matches"
    );
}

// ── Use selection as search (*) ──────────────────────────────────────────────

/// `*` on a cursor expands to the word under the cursor and sets search.
#[test]
fn star_on_cursor_expands_to_word() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.handle_key(key('*'));
    assert_eq!(ed.state.mode, Mode::Normal);
    // Selection expanded to cover "hello".
    assert_eq!(state(&ed), "-[hello]> world\n");
    // Pattern in search register is the word with whole-word boundaries.
    assert_eq!(reg(&ed, 's'), vec![r"\bhello\b"]);
    // Search direction set to forward.
    assert_eq!(ed.state.search.direction, SearchDirection::Forward);
    assert!(ed.search_pattern().is_some());
}

/// `*` on a partial-word selection expands to the whole word under the head —
/// it must NOT search the literal partial text. Searching the literal
/// substring would produce `\bell\b` (from "ell"), which can never match
/// anything, because `\b` doesn't exist inside "hello".
#[test]
fn star_on_partial_selection_expands_to_word() {
    // "hello world\n", selection covers "ell" (head on the second 'l').
    let mut ed = editor_from("h-[ell]>o world\n");
    ed.handle_key(key('*'));
    // Selection expands to the full word "hello", discarding the partial selection.
    assert_eq!(state(&ed), "-[hello]> world\n");
    assert_eq!(reg(&ed, 's'), vec![r"\bhello\b"]);

    // Independent oracle: the pattern must actually match "hello" in the buffer.
    let sp = ed.search_pattern().expect("search pattern must be set");
    let buf = ed.doc().text();
    let matches = hume_ops::search::find_all_matches(buf, &sp.regex);
    assert_eq!(
        matches,
        vec![(0, 4)],
        "pattern must match the word it came from"
    );
}

/// `*` on a selection spanning multiple words searches only the word under
/// the head, not the whole selection.
#[test]
fn star_on_multiword_selection_uses_word_under_head() {
    // "hello world\n", selection covers "hello wor" (head on the 'r' of "world").
    let mut ed = editor_from("-[hello wor]>ld\n");
    ed.handle_key(key('*'));
    assert_eq!(state(&ed), "hello -[world]>\n");
    assert_eq!(reg(&ed, 's'), vec![r"\bworld\b"]);
}

/// `*` on a `\n` cursor is a noop — no word to search for.
///
/// Without the `CharClass::Eol` guard, `inner_word_impl` would expand the
/// cursor to the adjacent newline run and set a useless newline regex.
#[test]
fn star_on_trailing_newline_is_noop() {
    let mut ed = editor_from("hello\n-[\n]>");
    let before = state(&ed);
    ed.handle_key(key('*'));
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), before);
    // The cursor/selection check above already catches a regressed guard (it would
    // move the selection), but pin the register too: a noop must not set a search regex.
    assert!(reg(&ed, 's').is_empty());
}

/// `*` with the head on whitespace is a no-op — no word to search for.
///
/// Regression: without the `CharClass::Space` guard, `inner_word_impl` would
/// expand the cursor to the adjacent whitespace run and set a bare-space
/// search pattern, which matches every run of whitespace in the buffer.
#[test]
fn star_on_whitespace_is_noop() {
    // "a b c\n", cursor on the space between 'a' and 'b'.
    let mut ed = editor_from("a-[ ]>b c\n");
    let before = state(&ed);
    ed.handle_key(key('*'));
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), before);
    assert!(reg(&ed, 's').is_empty());
}

/// `*` escapes regex metacharacters in the word it expands to.
#[test]
fn star_escapes_metacharacters() {
    // "a.b\n", cursor on '.' — a 1-char Punctuation run, escaped literally.
    let mut ed = editor_from("a-[.]>b\n");
    ed.handle_key(key('*'));
    assert_eq!(reg(&ed, 's'), vec![r"\."]);
}

/// `*` on a word matches whole words only — the `as` in `"last"` must not match.
#[test]
fn star_whole_word_skips_substring_matches() {
    // Buffer: "as last\n". Cursor on 'a' (position 0).
    let mut ed = editor_from("-[a]>s last\n");
    ed.handle_key(key('*'));

    assert_eq!(
        reg(&ed, 's'),
        vec![r"\bas\b"],
        "register should be whole-word pattern"
    );

    // The pattern must match standalone "as" (position 0) but NOT the "as"
    // inside "last" (positions 4-5). Expected matches: exactly one, at char 0.
    let sp = ed.search_pattern().expect("search pattern must be set");
    let buf = ed.doc().text();
    let matches = hume_ops::search::find_all_matches(buf, &sp.regex);
    assert_eq!(matches, vec![(0, 1)], "only standalone 'as' must match");
}

/// `*` on a punctuation run adds no word boundaries (regex \\b is meaningless there).
#[test]
fn star_punctuation_run_stays_literal() {
    // Buffer: "a -> b\n". Collapsed cursor on '-' (position 2).
    let mut ed = editor_from("-[a]>b\n");
    let buf = hume_editing::text::BufferText::from("a -> b\n");
    let sels = hume_editing::selection::SelectionSet::single(
        hume_editing::selection::Selection::collapsed(2),
    );
    *ed.doc_mut() = crate::editor::buffer::Buffer::new(buf, sels.clone());
    ed.set_current_selections(sels);

    ed.handle_key(key('*'));
    let r = reg(&ed, 's');
    // '-' and '>' are both Punctuation — no \b boundaries should be added.
    assert_eq!(r, vec!["->"]);
}

// ── Search selection (Ctrl+/) ────────────────────────────────────────────────

/// `Ctrl+/` on a partial-word selection searches the literal substring —
/// unlike `*`, it does NOT expand to the whole word, and it does NOT add
/// word-boundary anchors. This is the point of the feature: it finds "ell"
/// wherever it occurs, including as a substring of other words.
#[test]
fn search_selection_uses_literal_text() {
    // "hello ell x\n" — selection covers "ell" inside "hello" (head on second 'l').
    let mut ed = editor_from("h-[ell]>o ell x\n");
    ed.handle_key(key_ctrl('/'));

    // Selection is untouched (no expansion).
    assert_eq!(state(&ed), "h-[ell]>o ell x\n");
    // No \b anchors — literal substring pattern.
    assert_eq!(reg(&ed, 's'), vec!["ell"]);

    // Independent oracle: matches both the substring inside "hello" (1..4) and
    // the standalone "ell" (6..9) — proving it's substring, not whole-word, search.
    let sp = ed.search_pattern().expect("search pattern must be set");
    let buf = ed.doc().text();
    let matches = hume_ops::search::find_all_matches(buf, &sp.regex);
    assert_eq!(matches, vec![(1, 3), (6, 8)]);
}

/// After `Ctrl+/`, `n` cycles to the next literal occurrence — the full
/// "select, mark as search, jump" flow this feature exists for.
#[test]
fn search_selection_then_n_jumps_to_next_occurrence() {
    // "hello ell x\n" — select "ell" inside "hello".
    let mut ed = editor_from("h-[ell]>o ell x\n");
    ed.handle_key(key_ctrl('/'));
    ed.handle_key(key('n'));

    // Jumps to the next "ell" — the standalone one at position 6.
    assert_eq!(state(&ed), "hello -[ell]> x\n");
}

/// `Ctrl+/` escapes regex metacharacters in the selected text.
#[test]
fn search_selection_escapes_metacharacters() {
    // "a.b axb\n" — select "a.b".
    let mut ed = editor_from("-[a.b]> axb\n");
    ed.handle_key(key_ctrl('/'));
    assert_eq!(reg(&ed, 's'), vec![r"a\.b"]);

    // Oracle: the escaped '.' must NOT match "axb" as a wildcard.
    let sp = ed.search_pattern().expect("search pattern must be set");
    let buf = ed.doc().text();
    let matches = hume_ops::search::find_all_matches(buf, &sp.regex);
    assert_eq!(matches, vec![(0, 2)], "escaped '.' must not match 'axb'");
}

/// `Ctrl+/` on a collapsed cursor searches just that one character literally.
#[test]
fn search_selection_on_collapsed_cursor_searches_char() {
    let mut ed = editor_from("-[a]>bc abc\n");
    ed.handle_key(key_ctrl('/'));
    assert_eq!(reg(&ed, 's'), vec!["a"]);
}

/// `Ctrl+/` on a collapsed cursor sitting on a structural `\n` is a no-op.
///
/// Regression: without this guard, the 1-char selection "\n" becomes the
/// search pattern — a raw-newline regex that matches every line end,
/// clobbering the search register with something useless (the same
/// degenerate case `*` avoids via its `CharClass::Eol` guard).
#[test]
fn search_selection_on_newline_is_noop() {
    let mut ed = editor_from("hello\n-[\n]>");
    let before = state(&ed);
    ed.handle_key(key_ctrl('/'));
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(state(&ed), before);
    assert!(reg(&ed, 's').is_empty());
}
