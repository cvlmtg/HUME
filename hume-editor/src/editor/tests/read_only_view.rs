use super::*;
use crate::editor::doc_ops;
use hume_scripting::host::CommandHost;
use pretty_assertions::assert_eq;

// ── Read-only view buffer properties ─────────────────────────────────────────

/// `:messages` opens a real read-only buffer with label `[messages]`.
#[test]
fn messages_opens_read_only_view_buffer() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "test message".to_string());
    ed.execute_typed("messages", None).unwrap();
    assert!(
        ed.doc().is_read_only(),
        ":messages must open a read-only buffer"
    );
    assert_eq!(ed.doc().display_name(), "[messages]");
}

/// Bug regression: Up/Down in a read-only view must move the cursor (collapsed),
/// not select whole lines. In Normal mode, Down maps to `move-down`.
#[test]
fn view_buffer_arrow_keys_move_cursor_not_select() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "line one".to_string());
    ed.report(Severity::Warning, "line two".to_string());
    ed.execute_typed("messages", None).unwrap();

    // Cursor starts at last content line. Move up one line.
    let head_before = ed.current_view().primary().head().offset();
    ed.handle_key(key_up());

    let sel = ed.current_view().primary();
    // Selection must be collapsed (anchor == head), not a whole-line span.
    assert_eq!(
        sel.anchor().offset(),
        sel.head().offset(),
        "Up in view buffer must produce a collapsed cursor, not a selection"
    );
    assert!(
        sel.head().offset() < head_before,
        "Up must move the cursor backward in the buffer"
    );
}

/// View buffers must carry no language so syntax highlighting from the prior focus does not bleed in.
#[test]
fn view_buffer_has_no_language() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "test".to_string());
    ed.execute_typed("messages", None).unwrap();
    assert!(
        ed.doc().language.is_none(),
        "view buffer must have no language (no syntax highlighting)"
    );
}

/// Repeated `:messages` calls reuse the same buffer rather than accumulating duplicates.
#[test]
fn messages_reuses_existing_view_buffer() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "msg1".to_string());
    ed.execute_typed("messages", None).unwrap();
    // Switch back to the scratch buffer so the second :messages performs a real switch.
    let scratch_id = ed
        .state
        .buffers
        .iter()
        .find(|(_, buf)| buf.label.is_none() && buf.path().is_none())
        .map(|(id, _)| id)
        .expect("scratch buffer must exist");
    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), scratch_id);
    ed.report(Severity::Warning, "msg2".to_string());
    ed.execute_typed("messages", None).unwrap();

    assert_eq!(
        labelled_count(&ed, "[messages]"),
        1,
        ":messages must reuse the existing [messages] buffer"
    );
}

/// Editing commands on a read-only view buffer must be silently blocked.
#[test]
fn view_buffer_blocks_edits() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "test".to_string());
    ed.execute_typed("messages", None).unwrap();
    let content_before = ed.doc().text().to_string();

    // Try to delete the focused character: should be a no-op.
    ed.handle_key(key('x'));
    assert_eq!(
        ed.doc().text().to_string(),
        content_before,
        "x (delete) must not mutate a read-only buffer"
    );
}

/// Entering Insert mode on a read-only buffer must be refused.
#[test]
fn view_buffer_blocks_insert_mode() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "test".to_string());
    ed.execute_typed("messages", None).unwrap();

    ed.handle_key(key('i'));
    assert_ne!(
        ed.state.mode(),
        Mode::Insert,
        "i must not enter Insert mode on a read-only buffer"
    );
}

/// Every Insert entry that places the cursor first is refused on a
/// read-only buffer before it moves anything.
#[test]
fn insert_entry_on_a_read_only_buffer_leaves_the_selection_in_place() {
    for cmd in [
        "insert-before",
        "insert-after",
        "insert-at-line-start",
        "insert-at-line-end",
        "insert-at-selection-start",
        "insert-at-selection-end",
    ] {
        let mut ed = editor_from("  he-[ll]>o world\n");
        ed.doc_mut().read_only = true;
        let before = state(&ed);

        ed.execute_keymap_command(cmd.to_string().into(), None, false);

        assert_eq!(state(&ed), before, "{cmd}");
        assert_ne!(ed.state.mode(), Mode::Insert, "{cmd}");
        assert_eq!(status(&ed), "Buffer is read-only", "{cmd}");
    }
}

/// An Insert entry refused because another pane holds an Insert session on
/// the same buffer leaves this pane's selection where it was.
#[test]
fn insert_entry_beside_another_panes_insert_session_leaves_the_selection_in_place() {
    let mut ed = editor_from("he-[ll]>o world\n");
    let pid_a = ed.view.focused_pane();
    let bid = ed.focused_buffer_id();
    live_host!(ed)
        .run_command_sync(
            "pane-vsplit",
            hume_scripting::PaneHandle::with_pane(bid, pid_a),
            hume_scripting::host::NativeCall {
                count: Some(1),
                ..Default::default()
            },
        )
        .expect("pane-vsplit must succeed");
    let pid_b = ed.view.focused_pane();
    assert_ne!(pid_a, pid_b, "setup: the split must focus a new pane");
    ed.state.focus.set_for_test(&mut ed.view, pid_a);
    ed.feed_key(key('i'));
    assert_eq!(ed.state.mode(), Mode::Insert, "setup: pane A is in Insert");
    ed.state.focus.set_for_test(&mut ed.view, pid_b);
    let before = state(&ed);

    ed.execute_keymap_command("insert-after".to_string().into(), None, false);

    assert_eq!(state(&ed), before);
    assert!(
        status(&ed).contains("open insert/paste session"),
        "got {:?}",
        status(&ed)
    );
}

// ── Post-review fixes ─────────────────────────────────────────────────────────

/// `u` and `Ctrl-r` on a read-only buffer leave the text alone, report why,
/// and mark the command refused.
#[test]
fn read_only_buffer_blocks_undo_and_redo() {
    let mut ed = editor_from("-[h]>ello\n");

    // Make an edit to create undo history.
    ed.handle_key(key('d'));
    let after_delete = ed.doc().text().to_string();

    // Flip the buffer to read-only (simulates the condition where a view buffer
    // somehow has undo history, e.g. from a future API path).
    ed.doc_mut().read_only = true;

    // u (undo) must be a no-op, and must report why.
    ed.handle_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        after_delete,
        "u must not undo on a read-only buffer"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Buffer is read-only"),
        "u must report the refusal, same as :earlier does"
    );
    let pane = focused_pane(&ed);
    let ran = live_host!(ed)
        .run_command_sync("undo", pane, hume_scripting::host::NativeCall::default())
        .expect("undo must dispatch");
    assert!(!ran, "a refused undo reports the dispatch as refused");

    // Ctrl-r (redo) must also be a no-op.
    ed.doc_mut().read_only = false; // undo first to create redo history
    ed.handle_key(key('u'));
    let after_undo = ed.doc().text().to_string();
    ed.doc_mut().read_only = true;
    ed.state.status_msg = None;

    ed.handle_key(key_ctrl('r'));
    assert_eq!(
        ed.doc().text().to_string(),
        after_undo,
        "Ctrl-r must not redo on a read-only buffer"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Buffer is read-only"),
        "Ctrl-r must report the refusal, same as :later does"
    );
}

/// `apply_doc_history_walk`'s own read-only refusal must be distinguishable
/// from genuine root/leaf exhaustion. See `HistoryWalk`'s own doc for why.
#[test]
fn apply_doc_history_walk_distinguishes_refusal_from_exhaustion() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('d')); // creates one undo step, not exhausted
    ed.doc_mut().read_only = true;

    let focused = ed.view.focused_pane();
    let bid = ed.focused_buffer_id();
    let (buffers, mut stores) = ed.state.edit_parts();
    let result = doc_ops::apply_doc_history_walk(
        buffers,
        &mut stores,
        focused,
        bid,
        |b, id, stores, pane| Ok(b.undo_n(id, stores, pane, 1)),
    )
    .unwrap();
    assert_eq!(
        result,
        doc_ops::HistoryWalk::RefusedReadOnly,
        "a read-only buffer with real undo history available must report \
         refusal, not the `Took(0)` a genuinely exhausted walk would also report"
    );
}

/// `p` and `P` (paste) on a read-only view buffer must report "Buffer is
/// read-only" and leave the buffer content unchanged.
/// Validity: remove the `ensure_writable()` guard from
/// `do_smart_paste` (p/P dispatch through it) and this test fails
/// (status_msg will not contain the expected message, and the paste would
/// silently diverge from the read-only contract).
#[test]
fn view_buffer_blocks_paste() {
    let mut ed = editor_from("-[h]>ello\n");

    // Yank from the writable buffer so the kill-ring / clipboard is non-empty.
    ed.handle_key(key('y'));
    ed.handle_key(key('y'));

    ed.report(Severity::Warning, "test message".to_string());
    ed.execute_typed("messages", None).unwrap();
    assert!(ed.doc().is_read_only());
    let content_before = ed.doc().text().to_string();

    // p (paste after)
    ed.handle_key(key('p'));
    assert_eq!(
        ed.doc().text().to_string(),
        content_before,
        "p must not mutate a read-only buffer"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Buffer is read-only"),
        "p must report 'Buffer is read-only'"
    );

    // P (paste before)
    ed.handle_key(key('P'));
    assert_eq!(
        ed.doc().text().to_string(),
        content_before,
        "P must not mutate a read-only buffer"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Buffer is read-only"),
        "P must report 'Buffer is read-only'"
    );
}

/// `d` on a read-only view buffer must report "Buffer is read-only", leave
/// the buffer content unchanged, and must not push anything onto the kill
/// ring.
///
/// Validity: drop the `ensure_writable()` guard from `cmd_delete` and this
/// test fails: the ring head becomes the (refused) delete's yank instead of
/// the pre-existing one.
#[test]
fn read_only_buffer_blocks_delete_kill() {
    let mut ed = editor_from("-[hell]>o\n");

    // Populate the ring from the writable buffer.
    ed.handle_key(key('y'));
    let ring_before = ed
        .state
        .kill_ring
        .head()
        .map(<[hume_ops::register::Piece]>::to_vec);
    let ring_len_before = ed.state.kill_ring.len();

    ed.report(Severity::Warning, "test message".to_string());
    ed.execute_typed("messages", None).unwrap();
    assert!(ed.doc().is_read_only());
    let content_before = ed.doc().text().to_string();

    ed.handle_key(key('d'));

    assert_eq!(
        ed.doc().text().to_string(),
        content_before,
        "d must not mutate a read-only buffer"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Buffer is read-only"),
        "d must report 'Buffer is read-only'"
    );
    assert_eq!(
        ed.state
            .kill_ring
            .head()
            .map(<[hume_ops::register::Piece]>::to_vec),
        ring_before,
        "a refused d must not change the kill ring head"
    );
    assert_eq!(
        ed.state.kill_ring.len(),
        ring_len_before,
        "a refused d must not push a new entry onto the kill ring"
    );
}

/// `c` on a read-only view buffer must report "Buffer is read-only", leave
/// the buffer content and mode unchanged, and must not push anything onto
/// the kill ring or touch the paste stamp.
///
/// Validity: drop the `ensure_writable()` guard from `cmd_change` and this
/// test fails: the ring head becomes the (refused) change's yank and the
/// editor drops into Insert mode.
#[test]
fn read_only_buffer_blocks_change_kill() {
    let mut ed = editor_from("-[hell]>o\n");

    ed.handle_key(key('y'));
    let ring_before = ed
        .state
        .kill_ring
        .head()
        .map(<[hume_ops::register::Piece]>::to_vec);
    let stamp_before = ed.state.paste_stamp;

    ed.report(Severity::Warning, "test message".to_string());
    ed.execute_typed("messages", None).unwrap();
    assert!(ed.doc().is_read_only());
    let content_before = ed.doc().text().to_string();

    ed.handle_key(key('c'));

    assert_eq!(
        ed.doc().text().to_string(),
        content_before,
        "c must not mutate a read-only buffer"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Buffer is read-only"),
        "c must report 'Buffer is read-only'"
    );
    assert_eq!(
        ed.state.mode(),
        Mode::Normal,
        "a refused c must not enter Insert mode"
    );
    assert_eq!(
        ed.state
            .kill_ring
            .head()
            .map(<[hume_ops::register::Piece]>::to_vec),
        ring_before,
        "a refused c must not change the kill ring head"
    );
    assert_eq!(
        format!("{:?}", ed.state.paste_stamp),
        format!("{stamp_before:?}"),
        "a refused c must not touch the paste stamp"
    );
}

/// A refused `"3` + `d` on a read-only buffer must not leave register `3`
/// populated, and must not leave the `"<reg>` prefix armed for the next
/// command. `3`, not `a`: `a` is not a valid register name
/// (`is_valid_register_name` accepts only `0`–`9`, `k`, `c`, `b`), so
/// `input_stack/base.rs` would already have dropped the prefix on `"a` alone,
/// and a register that never armed can't tell this test whether the refusal
/// itself cleared anything.
///
/// Validity: drop the `state.register_prefix = None` line from
/// `read_only_refusal()` and this test fails: the prefix survives and
/// silently redirects the next yank/kill into register `3`.
#[test]
fn read_only_refusal_clears_register_prefix_on_delete() {
    let mut ed = editor_from("-[hell]>o\n");

    ed.report(Severity::Warning, "test message".to_string());
    ed.execute_typed("messages", None).unwrap();
    assert!(ed.doc().is_read_only());

    ed.handle_key(key('"'));
    ed.handle_key(key('3'));
    ed.handle_key(key('d'));

    assert!(
        reg(&ed, '3').is_empty(),
        "a refused \"3d must not write register '3'"
    );
    assert!(
        ed.state.register_prefix.is_none(),
        "a refused \"3d must not leave the register prefix armed"
    );
}

/// A refused `"3` + `p` on a read-only buffer must not leave the `"<reg>`
/// prefix armed for the next command. `3`, not `a`: see the comment on
/// `read_only_refusal_clears_register_prefix_on_delete`.
///
/// Validity: drop the `state.register_prefix = None` line from
/// `read_only_refusal()` and this test fails: the prefix survives.
#[test]
fn read_only_refusal_clears_register_prefix_on_paste() {
    let mut ed = editor_from("-[hell]>o\n");

    ed.report(Severity::Warning, "test message".to_string());
    ed.execute_typed("messages", None).unwrap();
    assert!(ed.doc().is_read_only());

    ed.handle_key(key('"'));
    ed.handle_key(key('3'));
    ed.handle_key(key('p'));

    assert!(
        ed.state.register_prefix.is_none(),
        "a refused \"ap must not leave the register prefix armed"
    );
}

/// `:w` on a read-only view buffer ([messages]) must error with
/// "Buffer is read-only", not "no file name".
///
/// Validity: remove the `is_read_only()` guard from `write_file` and this
/// test fails: the status_msg will contain "no file name" instead.
#[test]
fn view_buffer_blocks_write() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.report(Severity::Warning, "test message".to_string());
    ed.execute_typed("messages", None).unwrap();
    assert!(ed.doc().is_read_only());

    ed.execute_typed("w", None).unwrap_err();
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Buffer is read-only"),
        ":w on a read-only view must report 'Buffer is read-only'"
    );
}

/// `:e!` on a synthetic buffer (path-less, labeled) must error with
/// "no file name": there is no source to reload from, force or not.
///
/// Validity: restore the force-branch that replaces with scratch and this
/// test fails: the buffer becomes a scratch buffer instead of erroring.
#[test]
fn synthetic_buffer_e_bang_errors() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "test message".to_string());
    ed.execute_typed("messages", None).unwrap();
    assert!(ed.doc().is_synthetic());

    let err = ed.execute_typed("e!", None).unwrap_err();
    assert!(
        err.to_string().contains("no file name"),
        ":e! on synthetic must error 'no file name', got: {err}"
    );

    // Buffer must be untouched.
    assert!(ed.doc().is_synthetic(), "buffer must still be synthetic");
    assert_eq!(
        ed.doc().display_name(),
        "[messages]",
        "label must be preserved"
    );
    assert!(ed.doc().is_read_only(), "buffer must still be read-only");
}

// ── show-read-only-buffer! ────────────────────────────────────────────────────

/// Defines `:probe` as `body`, with `pane` bound to the pane it runs in, and
/// runs it.
fn run_show_probe(ed: &mut Editor, body: &str) {
    let tmp = tempfile::tempdir().unwrap();
    run(
        ed,
        tmp.path(),
        &format!(r#"(define-typed-command! "probe" "" (lambda (pane) {body}))"#),
    );
    type_cmd(ed, ":probe");
}

fn labelled_count(ed: &Editor, label: &str) -> usize {
    ed.state
        .buffers
        .iter()
        .filter(|(_, buf)| buf.label.as_ref().map(ViewLabel::as_str) == Some(label))
        .count()
}

/// The pane switches to a read-only buffer labelled `[name]` holding `text`,
/// with the cursor on the first line.
#[test]
fn show_read_only_buffer_opens_the_text_in_the_pane() {
    let mut ed = editor_from("-[h]>ello\n");
    run_show_probe(
        &mut ed,
        r#"(show-read-only-buffer! pane "report" "one\ntwo\nthree\n")"#,
    );

    assert!(ed.doc().is_read_only());
    assert_eq!(ed.doc().display_name(), "[report]");
    assert_eq!(ed.doc().text().to_string(), "one\ntwo\nthree\n");
    assert_eq!(ed.current_view().primary().head().offset().index(), 0);
}

/// A second call with the same name replaces the first call's text in the
/// same buffer. It names the pane through `(focused-pane)`: after the first
/// call, `pane` names a buffer its pane does not show.
#[test]
fn show_read_only_buffer_reuses_the_named_buffer() {
    let mut ed = editor_from("-[h]>ello\n");
    run_show_probe(
        &mut ed,
        r#"(show-read-only-buffer! pane "report" "first\n")
           (show-read-only-buffer! (focused-pane) "report" "second\n")"#,
    );

    assert_eq!(labelled_count(&ed, "[report]"), 1);
    assert_eq!(ed.doc().text().to_string(), "second\n");
}

/// The editor's own view names and an empty name raise, and leave the pane
/// where it was.
#[test]
fn show_read_only_buffer_refuses_editor_view_names() {
    for name in ["messages", "plugin-status", "lsp-status", ""] {
        let mut ed = editor_from("-[h]>ello\n");
        let before = ed.focused_buffer_id();
        run_show_probe(
            &mut ed,
            &format!(r#"(show-read-only-buffer! pane "{name}" "x\n")"#),
        );

        assert_eq!(ed.focused_buffer_id(), before, "{name:?}");
        assert_eq!(labelled_count(&ed, &format!("[{name}]")), 0, "{name:?}");
        assert!(
            has_error(&ed, "show-read-only-buffer!"),
            "{name:?} must raise"
        );
    }
}

/// View text is not a file: a shebang in it gives the view no language, and
/// the view still announces its open.
#[test]
fn show_read_only_buffer_text_gets_no_language_but_announces_its_open() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.state
        .config
        .languages
        .register_identity("python", &["py"], &[], &["python"])
        .expect("a test identity registers");
    let tmp = tempfile::tempdir().unwrap();
    run(
        &mut ed,
        tmp.path(),
        r##"(register-hook! 'on-buffer-open (lambda (pane) (log! 'warn "opened")))
           (define-typed-command! "probe" "" (lambda (pane)
             (show-read-only-buffer! pane "report" "#!/usr/bin/env python\nprint(1)\n")))"##,
    );
    type_cmd(&mut ed, ":probe");
    ed.settle();

    assert_eq!(ed.doc().display_name(), "[report]");
    assert_eq!(ed.doc().language, None);
    let opened = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.text == "opened")
        .count();
    assert_eq!(opened, 1);
}
