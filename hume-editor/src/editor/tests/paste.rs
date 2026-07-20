// Terminal bracketed-paste (`Event::Paste`) handling: `handle_terminal_paste`
// in `mappings/paste.rs`. Distinct from the register/kill-ring `p`/`P` paste
// commands covered in `commands.rs`.

use super::*;
use crate::editor::lsp::completion::{CompletionSession, StoredCompletionItem};
use pretty_assertions::assert_eq;
use termina::event::Event;

fn paste(ed: &mut Editor, text: &str) {
    ed.handle_event(Event::Paste(text.to_string()));
}

fn begin_completion_session(ed: &mut Editor, items: &[&str]) {
    let bid = ed.focused_buffer_id();
    let items: Vec<StoredCompletionItem> = items
        .iter()
        .map(|label| {
            StoredCompletionItem::from_json(&serde_json::json!({"label": label}))
                .expect("test item")
        })
        .collect();
    let session = CompletionSession::begin(&ed.state, bid, items, false).unwrap();
    ed.lsp.completion = Some(session);
}

// ── No-op guards ──────────────────────────────────────────────────────────

#[test]
fn empty_paste_is_a_noop_in_normal_mode() {
    let mut ed = editor_from("-[h]>ello\n");
    let before = state(&ed);
    paste(&mut ed, "");
    assert_eq!(state(&ed), before);
    // Not even an undo step was opened.
    ed.feed_key(key('u'));
    assert_eq!(state(&ed), before);
}

#[test]
fn newline_only_paste_flattens_to_empty_and_is_a_noop_in_command_mode() {
    // Distinct from the top-level empty-text guard: "\n\n" survives
    // `normalize_paste_newlines` (it's non-empty), but `flatten_for_minibuf`
    // trims all of it away, and the Command/Search/Select arm's own
    // empty-after-flatten guard must catch that.
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    assert_eq!(ed.state.mode, Mode::Command);
    paste(&mut ed, "\n\n");
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "");
}

// ── Insert mode ───────────────────────────────────────────────────────────

#[test]
fn insert_mode_paste_lands_whole_text_in_one_undo_step() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.feed_key(key('i'));
    paste(&mut ed, "xyz");
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "xyzhello\n");

    ed.feed_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "hello\n");
}

#[test]
fn insert_mode_paste_composes_with_surrounding_typing_in_one_undo_group() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    paste(&mut ed, "bc");
    ed.feed_key(key('d'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "abcd\n");

    // Typing + paste + typing all landed as one undo group.
    ed.feed_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "\n");
}

#[test]
fn insert_mode_paste_skips_auto_pairs() {
    // Auto-pairs is on by default; a typed '(' would insert "()" with the
    // cursor between them. A pasted '(' must land literally.
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    paste(&mut ed, "(");
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "(\n");
}

#[test]
fn insert_mode_paste_dismisses_open_completion_session() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    begin_completion_session(&mut ed, &["foo", "bar"]);
    assert!(ed.lsp.completion.is_some());

    paste(&mut ed, "xyz");
    assert!(ed.lsp.completion.is_none());
}

#[test]
fn insert_mode_paste_with_embedded_newline() {
    let mut ed = editor_from("h-[e]>llo\n");
    ed.feed_key(key('i'));
    paste(&mut ed, "X\nY");
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "hX\nYello\n");
}

#[test]
fn insert_mode_paste_with_embedded_escape_sequence_inserts_literally() {
    // `handle_terminal_paste` receives the payload already stripped of the
    // bracketed-paste markers — an embedded control/escape sequence in that
    // payload is just more text to insert, not something to interpret.
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    paste(&mut ed, "\x1b[31mred\x1b[0m");
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "\x1b[31mred\x1b[0m\n");
}

// ── Normal / Extend mode ─────────────────────────────────────────────────

#[test]
fn normal_mode_paste_replaces_selection_in_one_undo_step() {
    let mut ed = editor_from("-[hell]>o\n");
    assert_eq!(ed.state.mode, Mode::Normal);
    paste(&mut ed, "xyz");
    assert_eq!(ed.doc().text().to_string(), "xyzo\n");

    ed.feed_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "hello\n");
}

#[test]
fn extend_mode_paste_replaces_selection() {
    // The paste dispatcher only branches on `self.state.mode()`, not on how
    // Extend was entered — set it directly.
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.mode = Mode::Extend;
    paste(&mut ed, "xyz");
    assert_eq!(ed.doc().text().to_string(), "xyzo\n");
}

#[test]
fn normal_mode_paste_replaces_every_selection_in_a_multi_cursor_selection() {
    // `insert_str` (the closure `apply_doc_edit` invokes) iterates the whole
    // SelectionSet, same as any other command — a paste replacing multiple
    // selections at once is no exception. Two-char selections here (not
    // bare 1-char cursors) so this exercises replace, not insert-before —
    // see `insert_str_replaces_forward_selection` vs. `insert_str_two_cursors`
    // in `ops/edit/tests.rs` for why that distinction matters.
    let mut ed = editor_from("-[ab]>cd-[ef]>gh\n");
    paste(&mut ed, "X");
    assert_eq!(ed.doc().text().to_string(), "XcdXgh\n");

    // One undo step undoes both replacements together.
    ed.feed_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "abcdefgh\n");
}

// ── Command / Search minibuffer ──────────────────────────────────────────

#[test]
fn command_mode_paste_flattens_trailing_and_interior_newlines() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    assert_eq!(ed.state.mode, Mode::Command);
    paste(&mut ed, "foo\nbar\n");
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "foo bar");
}

#[test]
fn search_mode_paste_triggers_live_search() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.handle_key(key('/'));
    assert_eq!(ed.state.mode, Mode::Search);
    paste(&mut ed, "world");
    // Live search already moved the selection onto the match.
    assert_eq!(state(&ed), "hello -[world]>\n");
}

#[test]
fn select_mode_paste_triggers_live_select() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    ed.handle_key(key('s'));
    assert_eq!(ed.state.mode, Mode::Select);
    paste(&mut ed, "ab");
    // Live select-within already narrowed to the two "ab" matches within the
    // original selection — same `on_minibuf_paste_edited` follow-up a typed
    // pattern would trigger.
    assert_eq!(ed.current_selections().len(), 2);
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "ab");
}

// ── Dot-repeat ────────────────────────────────────────────────────────────

#[test]
fn dot_repeat_replays_a_paste() {
    let mut ed = editor_from("-[foo]> bar\n");
    ed.feed_key(key('c')); // change: delete "foo", enter Insert
    paste(&mut ed, "hi");
    ed.feed_key(key_esc()); // buffer: "hi bar\n"
    assert_eq!(ed.doc().text().to_string(), "hi bar\n");

    ed.feed_key(key('w')); // select " bar" (leading space, no trailing — EOL)
    ed.feed_key(key('.')); // repeat: delete " bar", paste "hi"
    assert_eq!(ed.doc().text().to_string(), "hihi\n");
}
