//! `on-text-changed` and `on-undo-history-changed`.

use super::*;

// ── OnTextChanged ────────────────────────────────────────────────────────────

/// Typing one character fires exactly one `on-text-changed`, naming the
/// edited buffer.
///
/// A stale or wrong id in `into_steel_args`, or a missing raise, trips either the
/// count or the buffer-id check below.
#[test]
fn typing_one_character_fires_on_text_changed_once() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed
             (lambda (bid)
               (log! 'trace (if (equal? (buffer-key bid) (buffer-key (focused-pane))) "correct-bid" "wrong-bid"))))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle(); // drain the startup on-buffer-open/on-buffer-enter

    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key_esc());
    ed.settle();

    let traces: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace)
        .map(|e| e.text.as_str())
        .collect();
    assert_eq!(
        traces,
        vec!["correct-bid"],
        "exactly one on-text-changed must fire, naming the edited buffer"
    );
}

/// Several mutations to the same buffer before a single `settle()` coalesce
/// into one `on-text-changed`: the contract `on-text-changed`'s doc states
/// and `BufferStore::take_text_changed` implements.
#[test]
fn several_edits_before_one_settle_coalesce_into_one_event() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    // Three keystrokes, no settle() between them: `feed_key` only steps the
    // keymap, it never drains `pending_work` on its own.
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key('c'));
    ed.feed_key(key_esc());
    ed.settle();

    let fires = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace && e.text == "changed")
        .count();
    assert_eq!(
        fires, 1,
        "three mutations before one settle() must coalesce into one event"
    );
}

/// Undo fires `on-text-changed` (it changes the text version via `Buffer::undo`); a
/// second undo once history is back at its root does not, since nothing
/// mutated (`buffer/tests.rs`'s `text_generation_unchanged_when_undo_at_root` pins
/// the same non-bump at the `Buffer` layer).
#[test]
fn undo_fires_but_a_no_op_undo_at_root_does_not() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    let fire_count = |ed: &Editor| {
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace && e.text == "changed")
            .count()
    };

    // One real edit, drained, to give undo something to undo.
    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key_esc());
    ed.settle();
    assert_eq!(fire_count(&ed), 1, "the edit itself must fire once");

    // `u` (undo) restores the pre-edit text: a real mutation, must fire.
    ed.feed_key(key('u'));
    ed.settle();
    assert_eq!(fire_count(&ed), 2, "undoing the edit must fire again");

    // History is now at its root, so a second `u` is a no-op, must not fire.
    ed.feed_key(key('u'));
    ed.settle();
    assert_eq!(
        fire_count(&ed),
        2,
        "a no-op undo at the history root must not fire"
    );
}

// ── OnUndoHistoryChanged ─────────────────────────────────────────────────────

/// An editor with `on-text-changed` and `on-undo-history-changed` hooks that
/// each log one trace line, plus a counter for either.
fn editor_with_history_hooks(input: &str) -> Editor {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from(input);
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "text")))
           (register-hook! 'on-undo-history-changed (lambda (bid) (log! 'trace "history")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();
    ed
}

/// An edit, its undo and its redo each fire once; an undo at the root, which
/// moves nothing, does not.
#[test]
fn undo_history_hook_fires_once_per_edit_undo_and_redo() {
    let mut ed = editor_with_history_hooks("-[a]>b\n");

    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    assert_eq!(
        trace_count(&ed, "history"),
        0,
        "an open insert session has recorded no revision yet"
    );
    ed.feed_key(key_esc());
    ed.settle();
    assert_eq!(trace_count(&ed, "history"), 1, "the edit");

    ed.feed_key(key('u'));
    ed.settle();
    assert_eq!(trace_count(&ed, "history"), 2, "the undo");

    ed.feed_key(key('U'));
    ed.settle();
    assert_eq!(trace_count(&ed, "history"), 3, "the redo");

    ed.feed_key(key('U'));
    ed.settle();
    assert_eq!(
        trace_count(&ed, "history"),
        3,
        "a redo at the tip moves nothing"
    );
}

/// A write moves the saved revision, which `buffer-undo-tree` reports as
/// `saved?`, so it announces once. A write that finds the same revision
/// already saved moves nothing.
#[test]
fn undo_history_hook_fires_when_a_write_moves_the_saved_revision() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("saved.txt");
    std::fs::write(&path, "ab\n").unwrap();
    let mut ed = editor_with_history_hooks("-[a]>b\n");
    ed.execute_typed("w", Some(path.to_str().unwrap())).unwrap();
    ed.settle();
    let after_first_write = trace_count(&ed, "history");

    ed.feed_key(key('d'));
    ed.settle();
    assert_eq!(
        trace_count(&ed, "history"),
        after_first_write + 1,
        "the edit"
    );

    ed.execute_typed("w", None).unwrap();
    ed.settle();
    assert_eq!(
        trace_count(&ed, "history"),
        after_first_write + 2,
        "the write"
    );

    ed.execute_typed("w", None).unwrap();
    ed.settle();
    assert_eq!(
        trace_count(&ed, "history"),
        after_first_write + 2,
        "a second write of the same revision moves nothing"
    );
}

/// Undoing an insert together with its later delete nets to no text change:
/// `on-text-changed` stays quiet, `on-undo-history-changed` does not.
#[test]
fn net_identity_undo_fires_history_hook_but_not_text_hook() {
    let mut ed = editor_with_history_hooks("-[h]>ello\n");
    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key_esc());
    ed.feed_key(key('d'));
    ed.settle();
    let (text, history) = (trace_count(&ed, "text"), trace_count(&ed, "history"));
    let before = ed.doc().text().to_string();

    ed.feed_key(key('2'));
    ed.feed_key(key('u'));
    ed.settle();

    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "setup: the walk nets to no change"
    );
    assert_eq!(trace_count(&ed, "text"), text);
    assert_eq!(trace_count(&ed, "history"), history + 1);
}

/// Several revisions recorded or walked before one `settle()` announce once.
#[test]
fn several_history_changes_before_one_settle_coalesce_into_one_event() {
    let mut ed = editor_with_history_hooks("-[a]>bc\n");

    ed.feed_key(key('d'));
    ed.feed_key(key('d'));
    ed.feed_key(key('u'));
    ed.settle();

    assert_eq!(trace_count(&ed, "history"), 1);
}
