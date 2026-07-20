use super::*;
use pretty_assertions::assert_eq;

pub(super) fn ls_output(ed: &mut Editor) -> String {
    ed.execute_typed("ls", None).unwrap();
    // After :ls the focused buffer is the read-only [buffers] view.
    ed.doc().text().rope().to_string()
}

// ── Single buffer ─────────────────────────────────────────────────────────────

#[test]
fn ls_single_buffer_marks_current() {
    let mut ed = editor_from("-[h]>ello\n");
    let out = ls_output(&mut ed);
    assert!(
        out.contains('%'),
        ":ls must mark the focused buffer with '%'"
    );
    assert!(
        !out.contains('#'),
        ":ls must not show '#' when there is no alternate buffer"
    );
    // Row count: 1 header + 1 buffer
    assert_eq!(out.lines().count(), 2, "must have header + 1 buffer row");
}

#[test]
fn ls_long_alias_works() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("list-buffers", None).unwrap();
    assert!(
        ed.doc().is_read_only(),
        ":list-buffers must open a read-only view buffer"
    );
    assert_eq!(
        ed.doc().display_name(),
        "[buffers]",
        ":list-buffers must focus the [buffers] view buffer"
    );
}

// ── Multiple buffers ──────────────────────────────────────────────────────────

// ── Dirty indicator ───────────────────────────────────────────────────────────

#[test]
fn ls_dirty_buffer_shows_plus() {
    let mut ed = editor_from("-[h]>ello\n");
    // Make the buffer dirty: enter insert, type a char, escape.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty(), "buffer must be dirty after edit");
    let out = ls_output(&mut ed);
    assert!(out.contains('+'), ":ls must show '+' for dirty buffers");
}

#[test]
fn ls_clean_buffer_no_plus() {
    let mut ed = editor_from("-[h]>ello\n");
    let out = ls_output(&mut ed);
    // Header has no '+'. Buffer row should not have '+' (clean).
    // Only check the buffer rows (skip header).
    for line in out.lines().skip(1) {
        assert!(!line.contains('+'), "clean buffer row must not contain '+'");
    }
}

// ── Scratch buffer ────────────────────────────────────────────────────────────

#[test]
fn ls_scratch_buffer_shows_scratch_name() {
    let mut ed = editor_from("-[h]>ello\n");
    let out = ls_output(&mut ed);
    // The initial unnamed buffer has path=None, label=None → display_name() = "*scratch*".
    assert!(
        out.contains("*scratch*"),
        ":ls must show '*scratch*' for nameless buffers, got:\n{out}"
    );
}

// ── Cursor placement ──────────────────────────────────────────────────────────

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
    let head_before = ed.current_selections().primary().head();
    ed.handle_key(key_up());

    let sel = ed.current_selections().primary();
    // Selection must be collapsed (anchor == head) — not a whole-line span.
    assert_eq!(
        sel.anchor(),
        sel.head(),
        "Up in view buffer must produce a collapsed cursor, not a selection"
    );
    assert!(
        sel.head() < head_before,
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
    ed.switch_to_buffer_without_jump(scratch_id);
    ed.report(Severity::Warning, "msg2".to_string());
    ed.execute_typed("messages", None).unwrap();

    // Count buffers with the [messages] label — must be exactly 1.
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

    // Try to delete the focused character — should be a no-op.
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
        ed.state.mode,
        Mode::Insert,
        "i must not enter Insert mode on a read-only buffer"
    );
}

// ── Post-review fixes ─────────────────────────────────────────────────────────

/// `:ls` called twice must not list the `[buffers]` buffer in its own output.
/// Validity: remove the `find_by_label("[buffers]")` skip from typed_list_buffers
/// and this test fails on the second call (output contains a `[buffers]` row).
#[test]
fn ls_does_not_list_itself_on_second_call() {
    let mut ed = editor_from("-[h]>ello\n");

    // First call: [buffers] doesn't yet exist, so output is clean.
    let out1 = ls_output(&mut ed);
    assert!(
        !out1.contains("[buffers]"),
        "first :ls must not mention [buffers]"
    );

    // Switch back to the scratch buffer so the second :ls triggers a real switch.
    let scratch_id = ed
        .state
        .buffers
        .iter()
        .find(|(_, buf)| buf.label.is_none() && buf.path().is_none())
        .map(|(id, _)| id)
        .expect("scratch buffer must still exist");
    ed.switch_to_buffer_without_jump(scratch_id);

    // Second call: [buffers] exists now but must be excluded from the listing.
    let out2 = ls_output(&mut ed);
    assert!(
        !out2.contains("[buffers]"),
        ":ls must not list the [buffers] view buffer in its own output; got:\n{out2}"
    );

    // Row count must be stable — one content row for the scratch buffer, one header.
    assert_eq!(
        out1.lines().count(),
        out2.lines().count(),
        ":ls row count must not grow across repeated calls"
    );
}

/// `:ls` must not push an entry to the jump list — view buffers are ephemeral.
/// Validity: change switch_to_buffer_without_jump back to switch_to_buffer_with_jump
/// in open_read_only_view and this test fails (departure buffer gains a jump entry).
#[test]
fn ls_does_not_pollute_jump_list() {
    let mut ed = editor_from("-[h]>ello\n");
    let scratch_id = ed.focused_buffer_id(); // the buffer we switch away from
    let pid = ed.state.focused_pane_id;

    // No jump entries for the scratch buffer before :ls.
    assert!(!ed.state.panes.jumps[pid].entries_for_buffer(scratch_id));

    ed.execute_typed("ls", None).unwrap();

    assert!(
        !ed.state.panes.jumps[pid].entries_for_buffer(scratch_id),
        ":ls must not push a jump entry for the departure buffer"
    );
}

/// `u` and `Ctrl+R` on a read-only buffer must be no-ops.
/// Validity: remove the is_read_only() guards from apply_doc_undo/apply_doc_redo
/// and this test fails (undo reverts the edit, changing the buffer text).
#[test]
fn read_only_buffer_blocks_undo_and_redo() {
    let mut ed = editor_from("-[h]>ello\n");

    // Make an edit to create undo history.
    ed.handle_key(key('d'));
    let after_delete = ed.doc().text().to_string();

    // Flip the buffer to read-only (simulates the condition where a view buffer
    // somehow has undo history — e.g. from a future API path).
    ed.doc_mut().read_only = true;

    // u (undo) must be a no-op.
    ed.handle_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        after_delete,
        "u must not undo on a read-only buffer"
    );

    // Ctrl+R (redo) must also be a no-op.
    ed.doc_mut().read_only = false; // undo first to create redo history
    ed.handle_key(key('u'));
    let after_undo = ed.doc().text().to_string();
    ed.doc_mut().read_only = true;

    ed.handle_key(key_ctrl('r'));
    assert_eq!(
        ed.doc().text().to_string(),
        after_undo,
        "Ctrl+R must not redo on a read-only buffer"
    );
}

/// `p` and `P` (paste) on a read-only view buffer must report "Buffer is
/// read-only" and leave the buffer content unchanged.
/// Validity: remove the `focused_buffer_read_only()` guard from `do_paste`
/// and this test fails (status_msg will not contain the expected message,
/// and the paste would silently diverge from the read-only contract).
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

/// `:w` on a read-only view buffer ([buffers]) must error with
/// "Buffer is read-only", not "no file name".
///
/// Validity: remove the `is_read_only()` guard from `write_file` and this
/// test fails — the status_msg will contain "no file name" instead.
#[test]
fn view_buffer_blocks_write() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.execute_typed("ls", None).unwrap();
    assert!(ed.doc().is_read_only());

    ed.execute_typed("w", None).unwrap_err();
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("Buffer is read-only"),
        ":w on a read-only view must report 'Buffer is read-only'"
    );
}

/// `:e!` on a synthetic buffer (path-less, labeled) must error with
/// "no file name" — there is no source to reload from, force or not.
///
/// Validity: restore the force-branch that replaces with scratch and this
/// test fails — the buffer becomes a scratch buffer instead of erroring.
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
