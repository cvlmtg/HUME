use super::*;
use crate::editor::doc_ops;
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
    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.state), scratch_id);
    ed.report(Severity::Warning, "msg2".to_string());
    ed.execute_typed("messages", None).unwrap();

    // Count buffers with the [messages] label: must be exactly 1.
    let count = ed
        .state
        .buffers
        .iter()
        .filter(|(_, buf)| buf.label.as_deref() == Some("[messages]"))
        .count();
    assert_eq!(
        count, 1,
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
    assert!(
        ed.state.command_refused,
        "a refused u marks the command refused"
    );

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

    let focused = ed.state.focus.id();
    let bid = ed.focused_buffer_id();
    let result = doc_ops::apply_doc_history_walk(
        &mut ed.state.buffers,
        &mut crate::editor::position_stores::PositionStores::new(
            &mut ed.state.panes,
            &mut ed.state.input,
            &mut ed.state.buffer_positions,
            &mut ed.state.config.decorations,
        ),
        &mut ed.state.active_session,
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
/// Validity: remove the `refuse_if_read_only()` guard from
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
/// Validity: drop the `refuse_if_read_only()` guard from `cmd_delete` and this
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
/// Validity: drop the `refuse_if_read_only()` guard from `cmd_change` and this
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
/// `refuse_if_read_only()` and this test fails: the prefix survives and
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
/// `refuse_if_read_only()` and this test fails: the prefix survives.
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
