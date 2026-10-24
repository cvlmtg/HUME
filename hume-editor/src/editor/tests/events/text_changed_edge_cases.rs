//! `on-text-changed` across identity edits, reloads, refused edits and closed buffers.

use super::*;

/// `:e!` reload (`Editor::reload_buffer_in_place`) fires `on-text-changed`:
/// the case a raise site at `doc_ops::finish_edit` would have missed, since
/// reload never goes through `doc_ops` (see `BufferStore::edit_seq`'s doc).
#[test]
fn e_bang_reload_fires_on_text_changed() {
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

    let replacement = Buffer::at_start(BufferText::from("reloaded\n"));
    ed.reload_buffer_in_place(
        FocusedPane::current(&ed.view),
        ReplaceSource::disk(replacement),
    )
    .expect("the buffer is not held");
    ed.settle();

    let fires = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace && e.text == "changed")
        .count();
    assert_eq!(fires, 1, ":e! reload must fire on-text-changed");
}

/// An edit refused by the read-only guard (`doc_ops::apply_doc_edit`'s early
/// `return` before `cmd` ever runs) never changes the text version, so it must not
/// fire `on-text-changed`.
///
/// Without the read-only guard, or with this raise moved upstream of it,
/// `insert_char` would run, the text version would bump, and the hook would fire.
#[test]
fn read_only_refused_edit_fires_no_on_text_changed() {
    use crate::editor::doc_ops;
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).read_only = true;

    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    let focused = ed.view.focused_pane();
    let before_gen = ed.state.buffers.get(bid).text().generation();
    let (buffers, mut stores) = ed.state.edit_parts();
    doc_ops::apply_doc_edit(buffers, &mut stores, focused, bid, |sels| {
        hume_ops::edit::insert_char(sels, 'z')
    })
    .unwrap();
    ed.settle();

    assert_eq!(
        ed.state.buffers.get(bid).text().generation(),
        before_gen,
        "read-only guard must block the edit before it reaches install"
    );
    assert_eq!(
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace)
            .count(),
        0,
        "a read-only-refused edit must not fire on-text-changed"
    );
}

/// Opening a buffer fires `on-buffer-open`, not `on-text-changed`. A fresh
/// buffer's text generation starts at 0 and `announced_generation` is seeded to
/// match, so there is no diff to observe.
#[test]
fn opening_a_buffer_fires_on_buffer_open_not_on_text_changed() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-buffer-open (lambda (bid) (log! 'trace "opened")))
           (register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    let path = tmp.path().join("fresh.txt");
    std::fs::write(&path, "hello\n").unwrap();
    type_cmd(&mut ed, &format!(":e {}", path.display()));
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
        vec!["opened"],
        "opening a buffer must fire on-buffer-open only, not on-text-changed"
    );
}

/// A handler that itself edits on `on-text-changed` is a feedback loop that
/// must be cut off by the same drain cap the other cascade tests exercise,
/// not livelock the editor. Detection runs *inside* `drain_pending_work`'s
/// fixpoint specifically so this cap can catch it (see the call site's doc).
///
/// The handler alternates `make-text-uppercase`/`make-text-lowercase` on the
/// selected letter: whichever direction the selection is in, at least one
/// of the two always changes the character (a lowercase letter capitalizes;
/// an uppercase one lowercases), so every invocation changes the text version and
/// re-triggers `on-text-changed`, guaranteeing the loop never runs dry on
/// its own.
#[test]
fn text_changed_feedback_loop_is_cut_off_by_drain_cap() {
    // Selection starts on the letter `a` and never moves (`make-text-*`
    // transforms case in place), so the alternation below never runs dry.
    // `define-command!`'s effect (registering "kick") only takes hold once
    // applied, so this uses `eval_with_real_host` rather than
    // `eval_source`+`MockHost`: the latter never calls
    // `apply_script_effects`, so a defined command would never actually
    // reach `ed.state.config.commands`.
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>b\n");
    let mut host = hume_scripting::ScriptingHost::new(&Dirs::none());
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-text-changed
             (lambda (bid)
               (call! "make-text-uppercase" (focused-pane))
               (call! "make-text-lowercase" (focused-pane))))
           (define-typed-command! "kick" "" (lambda () (call! "make-text-uppercase" (focused-pane))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    ed.settle();

    type_cmd(&mut ed, ":kick");
    ed.settle(); // must return, not hang

    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error
                && e.text.contains("event/callback cascade exceeded")),
        "drain cap must log an Error naming the hook cascade"
    );
    assert!(
        ed.state.config.pending_work.is_empty(),
        "pending hooks must be dropped when the cap fires"
    );
}

/// Closing the last buffer frees its slot and opens a fresh scratch buffer
/// under a brand new `BufferId` (`close_buffer`'s last-buffer branch): a
/// close+open pair, not a content swap under a surviving id. `on-text-
/// changed` must never fire for it (no surviving buffer's content changed);
/// the new scratch buffer instead announces its own `on-buffer-open`, like
/// any other freshly opened buffer.
///
/// A content swap under the closed id would leave `focused_buffer_id()`
/// equal to the closed `bid` and fire `on-text-changed` where
/// `on-buffer-open` belongs.
#[test]
fn last_buffer_close_opens_a_fresh_scratch_buffer_not_a_content_swap() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))
           (register-hook! 'on-buffer-open (lambda (bid) (log! 'trace "opened")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    ed.close_buffer(bid, crate::editor::buffer::OnUnsaved::Refuse)
        .expect("the buffer has no unsaved changes and runs no pipe");
    ed.settle();

    let new_bid = ed.focused_buffer_id();
    assert_ne!(
        new_bid, bid,
        "the last-buffer close must allocate a fresh BufferId, not reuse the closed one"
    );
    assert!(
        ed.state.buffers.try_get(bid).is_none(),
        "the closed buffer's slot must actually be freed"
    );

    let changed_fires = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace && e.text == "changed")
        .count();
    assert_eq!(
        changed_fires, 0,
        "no surviving buffer changed content: on-text-changed must not fire"
    );
    let opened_fires = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace && e.text == "opened")
        .count();
    assert_eq!(
        opened_fires, 1,
        "the fresh scratch buffer must announce on-buffer-open exactly once"
    );
}

/// Opening a read-only view (`:messages`/`:ls`) for the first time is silent
/// (fresh `Buffer`, baseline matches); refreshing an existing one via the
/// same label reuses the buffer and calls `set_view_content`, which must
/// fire `on-text-changed`: the documented trigger `store/tests.rs`'s
/// coverage never exercised end-to-end (it calls `set_view_content`
/// directly, bypassing `open_read_only_view`'s reuse path).
///
/// If `open_read_only_view`'s reuse branch closed and reopened the view
/// buffer, the second open would start from a fresh baseline too and the
/// hook would never fire.
#[test]
fn read_only_view_refresh_fires_on_text_changed() {
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

    ed.open_read_only_view(
        FocusedPane::current(&ed.view),
        &view_label("test-view"),
        "one\n",
        Some(hume_rope::line::ContentLine::new(0)),
    );
    ed.settle();
    assert_eq!(
        fire_count(&ed),
        0,
        "the first open of a read-only view must not fire on-text-changed"
    );

    ed.open_read_only_view(
        FocusedPane::current(&ed.view),
        &view_label("test-view"),
        "two\n",
        Some(hume_rope::line::ContentLine::new(0)),
    );
    ed.settle();
    assert_eq!(
        fire_count(&ed),
        1,
        "refreshing an existing read-only view must fire on-text-changed once"
    );
}

/// An identity edit (a command whose `ChangeSet` is the identity transform,
/// every op a `Retain`) must not bump the text version: `Buffer::apply_edit` skips
/// `install` entirely for one, so it must not fire `on-text-changed` either.
/// Also asserts `doc_ops::finish_edit`'s matching guard: `edit_seq` (the
/// global paste-staleness counter, see `BufferStore::edit_seq`'s doc) must
/// not move either, or a no-op edit command would wrongly stale a pending
/// paste stamp.
#[test]
fn identity_edit_fires_no_on_text_changed() {
    use crate::editor::doc_ops;
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    let focused = ed.view.focused_pane();
    let before_gen = ed.state.buffers.get(bid).text().generation();
    let before_edit_seq = ed.state.buffers.edit_seq();
    let (buffers, mut stores) = ed.state.edit_parts();
    doc_ops::apply_doc_edit(
        buffers,
        &mut stores,
        focused,
        bid,
        hume_editing::edit::Edited::unchanged,
    )
    .unwrap();
    ed.settle();

    assert_eq!(
        ed.state.buffers.get(bid).text().generation(),
        before_gen,
        "an identity edit must not change the text version"
    );
    assert_eq!(
        ed.state.buffers.edit_seq(),
        before_edit_seq,
        "an identity edit must not bump edit_seq"
    );
    assert_eq!(
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace)
            .count(),
        0,
        "an identity edit must not fire on-text-changed"
    );
}

/// An identity edit records no undo revision (`Buffer::apply_edit`'s guard
/// returns before `record_revision`): `u` right after one must undo the
/// *previous* real edit directly, not silently do nothing as if the
/// identity edit itself were on the undo stack.
#[test]
fn identity_edit_records_no_undo_revision() {
    use crate::editor::doc_ops;

    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    let original = ed.doc().text().to_string();

    // One real edit.
    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key_esc());
    ed.settle();
    let after_real_edit = ed.doc().text().to_string();
    assert_ne!(
        after_real_edit, original,
        "the real edit must have changed the text"
    );

    // An identity edit: no-op, must not land on the undo stack.
    let focused = ed.view.focused_pane();
    let (buffers, mut stores) = ed.state.edit_parts();
    doc_ops::apply_doc_edit(
        buffers,
        &mut stores,
        focused,
        bid,
        hume_editing::edit::Edited::unchanged,
    )
    .unwrap();
    ed.settle();
    assert_eq!(
        ed.doc().text().to_string(),
        after_real_edit,
        "an identity edit must not itself change the text"
    );

    // `u` must undo the real edit directly, not a phantom identity revision.
    ed.feed_key(key('u'));
    ed.settle();
    assert_eq!(
        ed.doc().text().to_string(),
        original,
        "undo must skip the identity edit and revert straight to the pre-edit text"
    );
}

/// An insert session whose composed edits cancel out to the identity
/// transform (type a character, then backspace it, all inside one insert
/// session) must record no undo revision at all: `commit_edit_group`'s
/// identity guard. the text version still moves during the session itself (each
/// keystroke is individually a real, non-identity `install`, so
/// `on-text-changed` correctly fires once for it, coalesced). The guard's
/// job is narrower: making sure nothing lands on the undo stack for `u` to
/// later replay as a *second*, phantom mutation.
///
/// Without `commit_edit_group`'s `cs.is_identity()` guard, the no-op
/// revision would be recorded, `is_dirty()`/`can_undo()` would both read
/// `true` right after `<Esc>`, and `u` would fire a second `on-text-changed`
/// for a byte-identical buffer.
#[test]
fn insert_then_backspace_records_no_revision() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    let original = ed.doc().text().to_string();

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

    // Type 'z', then backspace it, all within one insert session. The
    // composed ChangeSet cancels to identity (the same cancellation
    // `changeset/tests.rs`'s `compose_insert_then_delete` pins).
    ed.feed_key(key('i'));
    ed.feed_key(key('z'));
    ed.feed_key(key_backspace());
    ed.feed_key(key_esc());
    ed.settle();

    assert_eq!(
        ed.doc().text().to_string(),
        original,
        "the session must have made no net change"
    );
    assert!(
        !ed.state.has_unsaved_changes(bid),
        "a session that cancelled out to identity must not read dirty"
    );
    assert!(
        !ed.state.buffers.get(bid).can_undo(),
        "a session that cancelled out to identity must record no undo revision"
    );
    assert_eq!(
        fire_count(&ed),
        1,
        "the session's own keystrokes are real intermediate mutations: must fire once, coalesced"
    );

    // `u` must be a silent no-op (nothing was ever recorded for it to undo),
    // not replay a phantom identity revision, and must therefore not fire a
    // second on-text-changed.
    ed.feed_key(key('u'));
    ed.settle();
    assert_eq!(
        ed.doc().text().to_string(),
        original,
        "undo must leave the text untouched: there is nothing to undo"
    );
    assert_eq!(
        fire_count(&ed),
        1,
        "a no-op undo (nothing was ever recorded) must not fire a second on-text-changed"
    );
}

/// A byte-identical `:e!` reload (`replace_text_recorded`'s `forward.is_identity()`
/// case) must not bump the text version, so it must not fire `on-text-changed`.
///
/// The identity guard sits above `install`. Below it, the text version would
/// already have bumped by the time the guard returned, and the hook would
/// fire.
#[test]
fn identity_reload_fires_no_on_text_changed() {
    use crate::testing::MockHost;
    use hume_scripting::ScriptingHost;

    let mut ed = editor_from("-[a]>b\n");
    let bid = ed.focused_buffer_id();
    let text_before = ed.state.buffers.get(bid).text().clone();
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut mock = MockHost::new();
    host.eval_source(
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))"#,
        &mut mock,
    )
    .unwrap();
    ed.scripting = Some(host);
    ed.settle();

    let before_gen = ed.state.buffers.get(bid).text().generation();
    let replacement = Buffer::at_start(text_before);
    ed.reload_buffer_in_place(
        FocusedPane::current(&ed.view),
        ReplaceSource::disk(replacement),
    )
    .expect("the buffer is not held");
    ed.settle();

    assert_eq!(
        ed.state.buffers.get(bid).text().generation(),
        before_gen,
        "a byte-identical reload must not change the text version"
    );
    assert_eq!(
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace)
            .count(),
        0,
        "a byte-identical reload must not fire on-text-changed"
    );
}

/// `OnTextChanged` is queued from the live-buffer sweep at the top of a
/// drain pass, but fires behind whatever `Call` items (timer thunks, async
/// callbacks) were already queued ahead of it in the same batch. A due timer
/// that closes the edited buffer before its own `on-text-changed` fires must
/// not hand a dead `BufferId` to any handler.
///
/// Without `fire_one_event`'s liveness check, the handler would receive the
/// closed buffer's dead id, and its first buffer builtin on it would log a
/// `Severity::Error`.
#[test]
fn on_text_changed_skips_a_buffer_closed_earlier_in_the_batch() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>b\n");
    let bid_b = ed.open_buffer(Buffer::at_start(BufferText::from("hello\n")));
    ed.switch_to_buffer_with_jump(FocusedPane::current(&ed.view), bid_b);

    let mut host = hume_scripting::ScriptingHost::new(&Dirs::none());
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-text-changed (lambda (bid) (log! 'trace "changed")))
           (define-typed-command! "start" ""
             (lambda (bid) (after! 0 (lambda () (close-buffer! bid #:discard-unsaved #t)))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    ed.settle(); // drain startup on-buffer-open/on-buffer-enter

    // Real edit on B: changes the text version, not yet observed by a drain pass.
    ed.feed_key(key('i'));
    ed.feed_key(key('z'));
    ed.feed_key(key_esc());

    // Schedule the close, due immediately, and convert it to a queued Call
    // but don't drain it yet.
    type_cmd(&mut ed, ":start");
    ed.drain_async_sources();
    assert!(
        ed.state.buffers.try_get(bid_b).is_some(),
        "scheduling the close must not run it yet"
    );

    // One settle: `detect_text_changed` queues B's event behind the
    // already-queued close Call, in the same `pending_work` batch.
    ed.settle();

    assert!(
        ed.state.buffers.try_get(bid_b).is_none(),
        "the scheduled close must have run"
    );
    assert_eq!(
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace)
            .count(),
        0,
        "on-text-changed must not fire for a buffer closed earlier in the batch"
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .all(|e| e.severity != Severity::Error),
        "a dead buffer id must not surface as a hook error"
    );
}
