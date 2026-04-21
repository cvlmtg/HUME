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

    assert_eq!(ed.current_selections().len(), 2, "one selection per 'ab' match");
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

/// `select-all-matches` falls back to SEARCH_REGISTER when regex is cleared.
#[test]
fn select_all_matches_uses_search_register_fallback() {
    use crate::ops::register::SEARCH_REGISTER;
    let mut ed = editor_from("-[ab cd ab]>\n");
    ed.registers.write_text(SEARCH_REGISTER, vec!["ab".to_string()]);
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
    assert_eq!(ed.current_selections().len(), 2, "m/ should select all 'ab' matches");
}

// ── Use selection as search (*) ──────────────────────────────────────────────

/// `*` on a cursor expands to the word under the cursor and sets search.
#[test]
fn star_on_cursor_expands_to_word() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.handle_key(key('*'));
    assert_eq!(ed.mode, Mode::Normal);
    // Selection expanded to cover "hello".
    assert_eq!(state(&ed), "-[hello]> world\n");
    // Pattern in search register is the escaped word.
    assert_eq!(reg(&ed, 's'), vec!["hello"]);
    // Search direction set to forward.
    assert_eq!(ed.search.direction, SearchDirection::Forward);
    assert!(ed.search_pattern().is_some());
}

/// `*` on a non-cursor selection uses the selected text literally.
#[test]
fn star_on_selection_uses_selected_text() {
    let mut ed = editor_from("a-[b c]>d\n");
    ed.handle_key(key('*'));
    // Selection unchanged (non-cursor, no expansion).
    assert_eq!(state(&ed), "a-[b c]>d\n");
    assert_eq!(reg(&ed, 's'), vec!["b c"]);
}

/// `*` on the trailing structural newline does nothing.
#[test]
fn star_on_trailing_newline_is_noop() {
    let mut ed = editor_from("hello\n-[\n]>");
    // Exercise state() before and after the keypress to verify the
    // serialisation path doesn't panic on this edge-case cursor position.
    let _ = state(&ed);
    ed.handle_key(key('*'));
    // inner_word_impl on trailing \n produces a \n pattern.
    // This is a degenerate case but should not panic.
    assert_eq!(ed.mode, Mode::Normal);
    let _ = state(&ed);
}

/// `*` escapes regex metacharacters in the selection.
#[test]
fn star_escapes_metacharacters() {
    let mut ed = editor_from("-[f]>oo.bar\n");
    // Select "foo.bar" first via `v$` equivalent — use the whole line.
    // Easier: just set up a selection covering "foo.bar".
    let buf = crate::core::text::Text::from("foo.bar\n");
    let sels = crate::core::selection::SelectionSet::single(
        crate::core::selection::Selection::new(0, 6),
    );
    *ed.doc_mut() = crate::editor::buffer::Buffer::new(buf, sels.clone());
    ed.set_current_selections(sels);

    ed.handle_key(key('*'));
    assert_eq!(reg(&ed, 's'), vec!["foo\\.bar"]);
}

