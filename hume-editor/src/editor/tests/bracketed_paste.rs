// Terminal bracketed-paste (`TerminalEvent::Paste`) handling: `handle_terminal_paste`
// in `mappings/bracketed_paste.rs`. Distinct from the register/kill-ring
// `p`/`P` paste commands covered in `commands/paste.rs`.

use super::*;
use crate::editor::buffer::{DiskCheckTrigger, DiskState};
use crate::editor::input_stack::InputLayer;
use crate::editor::lsp::completion::{CompletionSession, StoredCompletionItem};
use crate::editor::overlay_models::{DrawerModel, MenuModel};
use crate::editor::picker::{self, PickerItem, PickerSession};
use hume_engine::types::TruncateEnd;
use hume_scripting::host::{LivePickerOpts, PickerOpts};
use pretty_assertions::assert_eq;
use steel::rvals::SteelVal;

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
    ed.state
        .input
        .push(InputLayer::Completion { session, ui: None });
}

// ── No-op guards ──────────────────────────────────────────────────────────

#[test]
fn empty_paste_is_a_noop_in_normal_mode() {
    let mut ed = editor_from("-[h]>ello\n");
    let before = state(&ed);
    ed.feed_paste("");
    assert_eq!(state(&ed), before);
    // Not even an undo step was opened.
    ed.feed_key(key('u'));
    assert_eq!(state(&ed), before);
}

#[test]
fn newline_only_paste_flattens_to_empty_and_is_a_noop_in_command_mode() {
    // Distinct from the top-level empty-text guard: "\n\n" survives
    // `normalize_line_endings` (it's non-empty), but `flatten_single_line`
    // trims all of it away — `MiniBuffer::insert_str` then returns `Ignored`
    // rather than `Edited`, the same result an unbound key would produce.
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    assert_eq!(ed.state.mode(), Mode::Command);
    ed.feed_paste("\n\n");
    assert_eq!(ed.state.minibuf().unwrap().input, "");
}

// ── Insert mode ───────────────────────────────────────────────────────────

#[test]
fn insert_mode_paste_lands_whole_text_in_one_undo_step() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.feed_key(key('i'));
    ed.feed_paste("xyz");
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
    ed.feed_paste("bc");
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
    ed.feed_paste("(");
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "(\n");
}

#[test]
fn insert_mode_paste_dismisses_open_completion_session() {
    let mut ed = editor_from("-[\n]>");
    ed.feed_key(key('i'));
    begin_completion_session(&mut ed, &["foo", "bar"]);
    assert!(ed.state.input.completion().is_some());

    ed.feed_paste("xyz");
    assert!(ed.state.input.completion().is_none());
}

#[test]
fn insert_mode_paste_with_embedded_newline() {
    let mut ed = editor_from("h-[e]>llo\n");
    ed.feed_key(key('i'));
    ed.feed_paste("X\nY");
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
    ed.feed_paste("\x1b[31mred\x1b[0m");
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "\x1b[31mred\x1b[0m\n");
}

// ── Normal / Extend mode ─────────────────────────────────────────────────

#[test]
fn normal_mode_paste_replaces_selection_in_one_undo_step() {
    let mut ed = editor_from("-[hell]>o\n");
    assert_eq!(ed.state.mode(), Mode::Normal);
    ed.feed_paste("xyz");
    assert_eq!(ed.doc().text().to_string(), "xyzo\n");

    ed.feed_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "hello\n");
}

#[test]
fn extend_mode_paste_replaces_selection() {
    // `base_input`'s `Paste` arm doesn't distinguish Normal from Extend
    // (both run through the same `Base` layer) — set Extend directly.
    let mut ed = editor_from("-[hell]>o\n");
    ed.state.input.set_extend(true);
    ed.feed_paste("xyz");
    assert_eq!(ed.doc().text().to_string(), "xyzo\n");
}

#[test]
fn normal_mode_paste_replaces_every_selection_in_a_multi_cursor_selection() {
    // `insert_str` (the closure `apply_doc_edit` invokes) iterates the whole
    // SelectionSet, same as any other command — a paste replacing multiple
    // selections at once is no exception. Two-char selections here (not
    // bare 1-char cursors) so this exercises replace, not insert-before —
    // see `insert_str_replaces_forward_selection` vs. `insert_str_two_cursors`
    // in `hume-ops/src/edit/tests/insert.rs` for why that distinction matters.
    let mut ed = editor_from("-[ab]>cd-[ef]>gh\n");
    ed.feed_paste("X");
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
    assert_eq!(ed.state.mode(), Mode::Command);
    ed.feed_paste("foo\nbar\n");
    assert_eq!(ed.state.minibuf().unwrap().input, "foo bar");
}

#[test]
fn search_mode_paste_triggers_live_search() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.handle_key(key('/'));
    assert_eq!(ed.state.mode(), Mode::Search);
    ed.feed_paste("world");
    // Live search already moved the selection onto the match.
    assert_eq!(state(&ed), "hello -[world]>\n");
}

#[test]
fn sift_mode_paste_triggers_live_sift() {
    let mut ed = editor_from("-[ab cd ab]>\n");
    ed.handle_key(key('s'));
    assert_eq!(ed.state.mode(), Mode::Sift);
    ed.feed_paste("ab");
    // Live sift-within already narrowed to the two "ab" matches within the
    // original selection — the paste ran through the same `Edited` arm a
    // typed pattern would.
    assert_eq!(ed.current_selections().len(), 2);
    assert_eq!(ed.state.minibuf().unwrap().input, "ab");
}

// ── Overlays ──────────────────────────────────────────────────────────────

fn marker(name: &str) -> SteelVal {
    SteelVal::StringV(name.into())
}

fn open_filter_test_picker(ed: &mut Editor, items: &[&str]) {
    let mut session = PickerSession::new(marker("on-select"), PickerOpts::default());
    session.push(
        items
            .iter()
            .map(|s| PickerItem {
                display: s.to_string(),
                payload: SteelVal::StringV((*s).into()),
            })
            .collect(),
    );
    picker::open_picker(&mut ed.state, &ed.view, session).expect("nothing else is open");
}

fn open_live_test_picker(ed: &mut Editor) {
    let session = PickerSession::new_live(
        marker("on-select"),
        LivePickerOpts {
            prompt: String::new(),
            query: String::new(),
            on_query_change: marker("on-query-change"),
            truncate: TruncateEnd::Head,
            actions: Vec::new(),
        },
    );
    picker::open_picker(&mut ed.state, &ed.view, session).expect("nothing else is open");
}

#[test]
fn picker_paste_lands_flattened_in_the_query_and_fires_one_query_change() {
    let mut ed = editor_from("-[h]>ello\n");
    open_live_test_picker(&mut ed);

    ed.feed_paste("foo\nbar\n");

    assert_eq!(ed.state.input.picker().unwrap().query(), "foo bar");
    assert_eq!(pending_calls(&ed).len(), 1);
    // The buffer underneath never saw the paste.
    assert_eq!(ed.doc().text().to_string(), "hello\n");
}

#[test]
fn picker_paste_into_a_non_live_session_queues_no_callback() {
    // A `picker!` (`PickerMode::Filter`) session has no `on_query_change` —
    // the query still updates and reranks, but nothing is queued to fire.
    let mut ed = editor_from("-[h]>ello\n");
    open_filter_test_picker(&mut ed, &["foo", "bar"]);

    ed.feed_paste("fo");

    assert_eq!(ed.state.input.picker().unwrap().query(), "fo");
    assert!(pending_calls(&ed).is_empty());
}

#[test]
fn confirm_paste_is_swallowed_and_the_confirm_stays_open() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    std::fs::write(&tmp, "hello, world!\n").unwrap();
    let bid = ed.focused_buffer_id();
    ed.check_buffer_disk_state(bid, DiskCheckTrigger::Ambient);
    assert!(ed.state.input.confirm().is_some());

    ed.feed_paste("xyz");

    assert_eq!(ed.doc().text().to_string(), "hello\n");
    assert!(ed.state.input.confirm().is_some(), "the confirm stays open");
    assert!(
        matches!(ed.state.buffers.get(bid).disk_state, DiskState::Changed(_)),
        "declining wasn't recorded — the paste never answered the prompt"
    );
}

#[test]
fn menu_paste_is_swallowed_but_clears_the_status_message() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.status_msg = Some("previous message".to_string());
    ed.state.input.push(InputLayer::Menu(MenuModel {
        rows: hume_ui::popup::MenuRows::measure(std::sync::Arc::new(vec!["m0".into()])),
        selected: 0,
        callback: marker("menu-cb"),
    }));

    ed.feed_paste("xyz");

    assert_eq!(ed.doc().text().to_string(), "hello\n");
    assert!(ed.state.input.menu().is_some(), "the menu stays open");
    assert!(ed.state.status_msg.is_none());
}

#[test]
fn drawer_paste_is_swallowed_but_clears_the_status_message() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.status_msg = Some("previous message".to_string());
    ed.state.input.push(InputLayer::Drawer(DrawerModel {
        items: std::sync::Arc::new(vec!["d0".to_string()]),
        selected: 0,
        scroll: 0,
        callback: marker("drawer-cb"),
    }));

    ed.feed_paste("xyz");

    assert_eq!(ed.doc().text().to_string(), "hello\n");
    assert!(ed.state.input.drawer().is_some(), "the drawer stays open");
    assert!(ed.state.status_msg.is_none());
}

// ── Dot-repeat ────────────────────────────────────────────────────────────

#[test]
fn dot_repeat_replays_a_paste() {
    let mut ed = editor_from("-[foo]> bar\n");
    ed.feed_key(key('c')); // change: delete "foo", enter Insert
    ed.feed_paste("hi");
    ed.feed_key(key_esc()); // buffer: "hi bar\n"
    assert_eq!(ed.doc().text().to_string(), "hi bar\n");

    ed.feed_key(key('w')); // select " bar" (leading space, no trailing — EOL)
    ed.feed_key(key('.')); // repeat: delete " bar", paste "hi"
    assert_eq!(ed.doc().text().to_string(), "hihi\n");
}
