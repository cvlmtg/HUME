// Edit + navigation primitives: apply-text-edits!,
// apply-workspace-edit!, goto-location!, and the workspace/applyEdit
// server-request swap.

use std::path::Path;

use super::*;
use crate::editor::lsp::LspState;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;

/// Attaches the focused buffer to a `Running` scripted server negotiated on
/// UTF-8. Negotiating the non-default encoding here does not by itself
/// prove `apply-text-edits!` consults it rather than assuming UTF-16 — a
/// wire offset only diverges between the two encodings on a line with a
/// multi-byte character, so most fixtures below (all ASCII) would pass
/// identically either way. The actual proof is
/// `apply_text_edits_utf8_server_uses_byte_offsets_not_utf16_units`, whose
/// fixture is chosen specifically to make that divergence observable.
fn attach_running_utf8_server(ed: &mut Editor) -> ServerId {
    let mut backend = InlineLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": {"positionEncoding": "utf-8"}}),
    );
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    let mut client = LspClient::new(sid, std::path::PathBuf::from("."));
    client.start_handshake(&mut backend);
    let (sid2, ev) = backend.drain().into_iter().next().unwrap();
    let actions = client.on_event(ev);
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp.insert_client_for_test(client);
    for action in actions {
        ed.dispatch_lsp_action(sid2, action);
    }
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    sid
}

// ── apply-text-edits! ───────────────────────────────────────────────────────

#[test]
fn apply_text_edits_single_edit() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    attach_running_utf8_server(&mut ed);
    let bid = ed.focused_buffer_id();
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (apply-text-edits! (current-buffer)
               (list (list (cons 0 1) (cons 0 3) "XY")))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(ed.doc().text().to_string(), "aXYdef\n");
    let _ = bid;
}

/// The encoding oracle: on line "aébcdef", `é` is 1 char but 2 UTF-8 bytes
/// and only 1 UTF-16 code unit, so byte offset 3 and code-unit offset 3 name
/// different characters (`b` vs `c`). A wire edit of `(0,3)-(0,4)` must
/// replace `b`, not `c` — if `apply-text-edits!` ever stopped consulting the
/// negotiated encoding and assumed UTF-16, this would silently corrupt the
/// wrong character instead of failing loudly.
#[test]
fn apply_text_edits_utf8_server_uses_byte_offsets_not_utf16_units() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>ébcdef\n");
    attach_running_utf8_server(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (apply-text-edits! (current-buffer)
               (list (list (cons 0 3) (cons 0 4) "X")))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(ed.doc().text().to_string(), "aéXcdef\n");
}

#[test]
fn apply_text_edits_multiple_edits_same_line_apply_descending() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    attach_running_utf8_server(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             ; Two edits on the same line, given out of order — must not
             ; corrupt each other's offsets (the classic ascending-with-
             ; fixups bug).
             (apply-text-edits! (current-buffer)
               (list (list (cons 0 0) (cons 0 1) "Z")
                     (list (cons 0 4) (cons 0 5) "W")))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(ed.doc().text().to_string(), "ZbcdWf\n");
}

/// L2 regression: two inserts at the same position must land in the order
/// the `edits` array gives them (LSP spec: array order defines apply order
/// for same-position edits) — a descending sort followed by a whole-`Vec`
/// `.reverse()` kept the tie in original order through the sort but then
/// flipped it via the reverse, applying them backwards.
#[test]
fn apply_text_edits_same_position_inserts_apply_in_array_order() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    attach_running_utf8_server(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (apply-text-edits! (current-buffer)
               (list (list (cons 0 0) (cons 0 0) "1")
                     (list (cons 0 0) (cons 0 0) "2")))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(
        ed.doc().text().to_string(),
        "12abcdef\n",
        "\"1\" must land before \"2\", matching the edits array's own order"
    );
}

#[test]
fn apply_text_edits_adjacent_not_overlapping_accepted() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    attach_running_utf8_server(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (apply-text-edits! (current-buffer)
               (list (list (cons 0 0) (cons 0 2) "AA")
                     (list (cons 0 2) (cons 0 4) "BB")))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(ed.doc().text().to_string(), "AABBef\n");
}

#[test]
fn apply_text_edits_overlapping_rejected() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    attach_running_utf8_server(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (apply-text-edits! (current-buffer)
               (list (list (cons 0 0) (cons 0 3) "A")
                     (list (cons 0 2) (cons 0 5) "B")))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(
        ed.doc().text().to_string(),
        "abcdef\n",
        "overlapping edits must reject with no partial application"
    );
}

#[test]
fn apply_text_edits_reversed_range_rejected() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    attach_running_utf8_server(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (apply-text-edits! (current-buffer)
               (list (list (cons 0 3) (cons 0 0) "X")))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(
        ed.doc().text().to_string(),
        "abcdef\n",
        "a reversed range (end before start) must reject cleanly, not panic on underflow"
    );
}

#[test]
fn apply_text_edits_is_one_undo_step() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    attach_running_utf8_server(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (apply-text-edits! (current-buffer)
               (list (list (cons 0 0) (cons 0 1) "Z")
                     (list (cons 0 5) (cons 0 6) "W")))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(ed.doc().text().to_string(), "Zbcdew\n".replace('w', "W"));

    ed.handle_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        "abcdef\n",
        "a single 'u' must restore the pre-edit text — both edits are one undo step"
    );
}

#[test]
fn apply_text_edits_version_mismatch_rejected() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    attach_running_utf8_server(&mut ed);
    let stale_gen = ed.doc().text_gen;
    // Make an unrelated edit first so the buffer's generation moves past
    // what the (fictional) LSP response was computed against.
    ed.handle_key(key('i'));
    ed.handle_key(key('!'));
    ed.handle_key(key_esc());

    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (apply-text-edits! (current-buffer)
                   (list (list (cons 0 0) (cons 0 1) "Z"))
                   #:expect-generation {stale_gen})))"#
        ),
    );
    let before = ed.doc().text().to_string();
    type_cmd(&mut ed, ":go");
    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "a stale expect-generation must reject the edit"
    );
}

// ── apply-workspace-edit! ────────────────────────────────────────────────────

#[test]
fn apply_workspace_edit_changes_shape() {
    let tmp = safe_tempdir();
    let file = tmp.path().join("a.txt");
    std::fs::write(&file, "abcdef\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut ed = editor_from("-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (apply-workspace-edit!
                   (hash "changes"
                     (hash {:?}
                       (list (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                               "end" (hash "line" 0 "character" 3))
                              "newText" "XYZ")))))))"#,
            uri.as_str()
        ),
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "abcdef\n",
        "workspace edits must not touch disk — :wa does that"
    );
    let bid = ed
        .state
        .buffers
        .find_by_path(&canonical)
        .expect("file must have been opened as a buffer");
    assert_eq!(ed.state.buffers.get(bid).text().to_string(), "XYZdef\n");
}

#[test]
fn apply_workspace_edit_document_changes_shape() {
    let tmp = safe_tempdir();
    let file = tmp.path().join("a.txt");
    std::fs::write(&file, "abcdef\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut ed = editor_from("-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (apply-workspace-edit!
                   (hash "documentChanges"
                     (list (hash "textDocument" (hash "uri" {:?} "version" void)
                                 "edits" (list (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                                "end" (hash "line" 0 "character" 1))
                                                "newText" "Z"))))))))"#,
            uri.as_str()
        ),
    );
    type_cmd(&mut ed, ":go");
    let bid = ed.state.buffers.find_by_path(&canonical).unwrap();
    assert_eq!(ed.state.buffers.get(bid).text().to_string(), "Zbcdef\n");
}

#[test]
fn apply_workspace_edit_mixed_open_and_unopened_files() {
    let tmp = safe_tempdir();
    let opened_path = tmp.path().join("opened.txt");
    let unopened_path = tmp.path().join("unopened.txt");
    std::fs::write(&opened_path, "hello\n").unwrap();
    std::fs::write(&unopened_path, "world\n").unwrap();
    let opened_canonical = std::fs::canonicalize(&opened_path).unwrap();
    let unopened_canonical = std::fs::canonicalize(&unopened_path).unwrap();
    let opened_uri = hume_lsp::uri::path_to_uri(&opened_canonical).unwrap();
    let unopened_uri = hume_lsp::uri::path_to_uri(&unopened_canonical).unwrap();

    let mut ed = editor_from("-[x]>\n");
    ed.execute_typed("e", Some(opened_path.to_str().unwrap()))
        .unwrap();
    assert!(ed.state.buffers.find_by_path(&unopened_canonical).is_none());

    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (apply-workspace-edit!
                   (hash "changes"
                     (hash {:?} (list (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                        "end" (hash "line" 0 "character" 1))
                                       "newText" "H"))
                          {:?} (list (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                        "end" (hash "line" 0 "character" 1))
                                       "newText" "W")))))))"#,
            opened_uri.as_str(),
            unopened_uri.as_str()
        ),
    );
    type_cmd(&mut ed, ":go");

    let opened_bid = ed.state.buffers.find_by_path(&opened_canonical).unwrap();
    assert_eq!(
        ed.state.buffers.get(opened_bid).text().to_string(),
        "Hello\n"
    );
    let unopened_bid = ed
        .state
        .buffers
        .find_by_path(&unopened_canonical)
        .expect("the unopened file must have been opened as a buffer");
    assert_eq!(
        ed.state.buffers.get(unopened_bid).text().to_string(),
        "World\n"
    );
}

/// The invalid entry is a directory, not a missing path: `resolve_or_open`
/// tolerates a missing path (opens a new-file buffer, same as `:e`), so only
/// a target that genuinely can't be opened — `Buffer::from_file_or_new` only
/// tolerates `NotFound` — still triggers this abort.
#[test]
fn apply_workspace_edit_one_invalid_file_aborts_the_whole_edit() {
    let tmp = safe_tempdir();
    let ok_path = tmp.path().join("ok.txt");
    std::fs::write(&ok_path, "abcdef\n").unwrap();
    let ok_canonical = std::fs::canonicalize(&ok_path).unwrap();
    let ok_uri = hume_lsp::uri::path_to_uri(&ok_canonical).unwrap();
    let invalid_dir = tmp.path().join("a_directory");
    std::fs::create_dir(&invalid_dir).unwrap();
    let invalid_uri =
        hume_lsp::uri::path_to_uri(&std::fs::canonicalize(&invalid_dir).unwrap()).unwrap();

    let mut ed = editor_from("-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (apply-workspace-edit!
                   (hash "documentChanges"
                     (list
                       ; The invalid entry is listed FIRST — documentChanges is an
                       ; ordered list (unlike `changes`' hashmap), so validation
                       ; reaches it before ever touching the valid file.
                       (hash "textDocument" (hash "uri" {:?} "version" void)
                             "edits" (list (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                            "end" (hash "line" 0 "character" 1))
                                            "newText" "Z")))
                       (hash "textDocument" (hash "uri" {:?} "version" void)
                             "edits" (list (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                            "end" (hash "line" 0 "character" 1))
                                            "newText" "Z"))))))))"#,
            invalid_uri.as_str(),
            ok_uri.as_str()
        ),
    );
    type_cmd(&mut ed, ":go");

    assert_eq!(
        std::fs::read_to_string(&ok_path).unwrap(),
        "abcdef\n",
        "the valid file's on-disk content is untouched (workspace edits never touch disk anyway)"
    );
    assert!(
        ed.state.buffers.find_by_path(&ok_canonical).is_none(),
        "the valid file must not even have been opened — validation stopped at the invalid entry first"
    );
}

/// L1 regression: two `documentChanges` entries for the same file (the spec
/// doesn't forbid it — server-controlled input) must be rejected, not build
/// a second changeset against text the first entry's already assumes and
/// panic in `commit_changeset`'s `cs.apply(&text).expect(...)`.
#[test]
fn apply_workspace_edit_duplicate_entry_for_the_same_file_is_rejected_not_a_panic() {
    let tmp = safe_tempdir();
    let file = tmp.path().join("a.txt");
    std::fs::write(&file, "abcdef\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut ed = editor_from("-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (apply-workspace-edit!
                   (hash "documentChanges"
                     (list
                       (hash "textDocument" (hash "uri" {0:?} "version" void)
                             "edits" (list (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                            "end" (hash "line" 0 "character" 1))
                                            "newText" "X")))
                       (hash "textDocument" (hash "uri" {0:?} "version" void)
                             "edits" (list (hash "range" (hash "start" (hash "line" 0 "character" 1)
                                                            "end" (hash "line" 0 "character" 2))
                                            "newText" "Y"))))))))"#,
            uri.as_str()
        ),
    );
    type_cmd(&mut ed, ":go"); // must not panic

    let bid = ed
        .state
        .buffers
        .find_by_path(&canonical)
        .expect("the first entry opens the file before the duplicate is detected");
    assert_eq!(
        ed.state.buffers.get(bid).text().to_string(),
        "abcdef\n",
        "a rejected edit must leave the buffer untouched — no partial apply"
    );
}

// ── goto-location! ───────────────────────────────────────────────────────────

#[test]
fn goto_location_same_buffer_char_indexed_shape() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let bid = ed.focused_buffer_id();
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (goto-location! (list (current-buffer) 0 3))))"#,
    );
    let before = state(&ed);
    type_cmd(&mut ed, ":go");
    assert_ne!(state(&ed), before);
    assert_eq!(ed.current_selections().primary().head(), 3);

    // A jump entry was pushed — Ctrl+o must return to the origin.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
    let _ = bid;
}

/// `goto-location!` to the position the cursor is already on is a no-op and
/// must not truncate forward jump-list history.
#[test]
fn goto_location_noop_does_not_clobber_forward_history() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (goto-location! (list (current-buffer) 0 0))))"#,
    );

    // `%` — jump-flagged, moves elsewhere, records a jump.
    ed.handle_key(key('%'));
    let after_percent = state(&ed);

    // Jump backward to the original head-0 position.
    ed.handle_key(key_ctrl('o'));
    let back_at_start = state(&ed);
    assert_ne!(back_at_start, after_percent);
    assert_eq!(ed.current_selections().primary().head(), 0);

    // `:go` targets char 0 — already there — a no-op.
    type_cmd(&mut ed, ":go");
    assert_eq!(
        state(&ed),
        back_at_start,
        ":go to the current position must not move"
    );

    // Forward history (the jump from `%`) must still be there.
    ed.handle_key(key_ctrl('i'));
    assert_eq!(
        state(&ed),
        after_percent,
        "a no-op goto-location! must not have truncated forward jump-list history"
    );
}

#[test]
fn goto_location_other_open_buffer_by_path_string() {
    let tmp = safe_tempdir();
    let file = tmp.path().join("other.txt");
    std::fs::write(&file, "xyz\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();

    let mut ed = editor_from("-[a]>bcdef\n");
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let other_bid = ed.state.buffers.find_by_path(&canonical).unwrap();
    // `:e` recorded a jump — jump back to the original scratch buffer, so
    // goto has to switch panes to reach the already-open "other.txt" buffer.
    ed.handle_key(key_ctrl('o'));
    assert_ne!(ed.focused_buffer_id(), other_bid);

    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (goto-location! (list {:?} 0 1))))"#,
            file.to_str().unwrap()
        ),
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(ed.focused_buffer_id(), other_bid);
    assert_eq!(ed.current_selections().primary().head(), 1);
}

#[test]
fn goto_location_unopened_path_opens_it() {
    let tmp = safe_tempdir();
    let file = tmp.path().join("fresh.txt");
    std::fs::write(&file, "hello\n").unwrap();

    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (goto-location! (list {:?} 0 2))))"#,
            file.to_str().unwrap()
        ),
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(ed.doc().text().to_string(), "hello\n");
    assert_eq!(ed.current_selections().primary().head(), 2);
}

#[test]
fn goto_location_char_indexed_target_past_eof_clamps_to_the_last_char() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (goto-location! (list (current-buffer) 999 0))))"#,
    );
    type_cmd(&mut ed, ":go");
    let len_chars = ed.doc().text().rope().len_chars();
    let head = ed.current_selections().primary().head();
    assert!(
        head < len_chars,
        "head must satisfy head < len_chars() — got head={head}, len_chars={len_chars}"
    );
    assert_eq!(
        head,
        len_chars - 1,
        "a target past EOF must clamp to the buffer's last char"
    );
}

/// `goto_location` must center the jump the same way `zz` does — by display
/// row, via `scroll::scroll_cursor_to_row` — not by re-deriving a
/// buffer-line-based centering of its own. The two only agree when nothing
/// wraps; under wrap they diverge, and a hand-rolled line-based centering
/// leaves `top_row_offset` untouched entirely (`clamp_viewport_top`'s own
/// doc names this exact call site as why it has to self-heal).
#[test]
fn goto_location_centers_by_display_row_not_buffer_line_under_wrap() {
    // Each line is 25 'x's, wrapped at width 10 into three display rows —
    // 10 + 10 + 5, the last one short of the wrap width so it doesn't also
    // trigger the trailing '\n' sentinel's own wrap onto a further row
    // (`format_buffer_line`'s end-of-line sentinel handling). A jump deep
    // into the file makes buffer-line and display-row centering diverge
    // sharply: line-based would center on line 20 directly; display-row
    // must center on line 20's own first row, three times as far down.
    let content: String = (0..30).map(|_| format!("{}\n", "x".repeat(25))).collect();
    let text = hume_editing::text::BufferText::from(content.as_str());
    let sels = SelectionSet::single(hume_editing::selection::Selection::collapsed(0));
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    let pid = ed.state.focused_pane_id;
    ed.execute_typed("set", Some("pane wrap-mode=soft:10"))
        .unwrap();
    ed.view.panes[pid].viewport.height = 10;

    let tmp = safe_tempdir();
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (goto-location! (list (current-buffer) 20 0))))"#,
    );
    type_cmd(&mut ed, ":go");

    let cursor_char = ed.current_selections().primary().head();
    let bid = ed.focused_buffer_id();
    let (mut rm, viewport) = crate::editor::commands::pane_row_map_mut(
        ed.state.buffers.get(bid),
        &ed.state.settings,
        &mut ed.view.panes[pid],
        &mut ed.state.motion_format_scratch,
    );
    let top = crate::editor::scroll::top_pos(viewport);
    let cursor_pos = rm.locate_row(cursor_char);
    assert_eq!(
        rm.distance(top, cursor_pos, 20),
        Some(5),
        "the cursor must land exactly height/2 (5) DISPLAY rows below the new top"
    );
}

/// A directory target genuinely can't be opened (`Buffer::from_file_or_new`
/// only tolerates `NotFound`, not `IsADirectory`) — a plain missing path
/// would not do here: `resolve_or_open` shares `:e`'s tolerance for those
/// (see `goto_missing_path_opens_new_file_buffer` below).
#[test]
fn goto_location_directory_target_errors_with_no_jump_entry() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let dir_target = tmp.path().to_str().unwrap().to_owned();
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
             (goto-location! (list {dir_target:?} 0 0))))"#
        ),
    );
    let before = state(&ed);
    type_cmd(&mut ed, ":go");
    assert_eq!(state(&ed), before, "a failed goto must not move the cursor");

    // No jump entry means Ctrl+o has nothing to do — state stays put.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
}

/// `(goto-location! (list path line char-col))` on a path that doesn't exist yet
/// must open a new-file buffer and jump to it, the same tolerance `:e` has —
/// `resolve_path_or_uri` shares `Editor::resolve_open_path`'s
/// `Buffer::from_file_or_new` chokepoint.
#[test]
fn goto_missing_path_opens_new_file_buffer() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let target = tmp.path().join("not-yet-created.txt");
    let target_str = target.to_str().unwrap().to_owned();
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
             (goto-location! (list {target_str:?} 0 0))))"#
        ),
    );
    let start_bid = ed.focused_buffer_id();

    type_cmd(&mut ed, ":go");

    assert_ne!(
        ed.focused_buffer_id(),
        start_bid,
        "goto must have switched to the new-file buffer"
    );
    assert!(ed.doc().is_new_file());
}

// ── workspace/applyEdit server-request swap ──────────────────────────────────

#[test]
fn server_initiated_apply_edit_actually_applies_and_answers_true() {
    let tmp = safe_tempdir();
    let file = tmp.path().join("srv.txt");
    std::fs::write(&file, "abcdef\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut ed = editor_from("-[x]>\n");
    let params = serde_json::json!({
        "edit": {
            "changes": {
                uri.as_str(): [{
                    "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
                    "newText": "XYZ",
                }]
            }
        }
    });
    let result = ed.apply_edit_request_response(&params).unwrap();
    assert_eq!(result["applied"], serde_json::json!(true));

    let bid = ed
        .state
        .buffers
        .find_by_path(&canonical)
        .expect("workspace/applyEdit must have opened the file as a buffer");
    assert_eq!(ed.state.buffers.get(bid).text().to_string(), "XYZdef\n");
}

/// `workspace/applyEdit` opens files via `lsp::edits::resolve_or_open` →
/// `buffer::lifecycle::open_or_dedup_and_notify`, which can't detect language
/// inline (see that function's doc) — it queues the buffer onto
/// `EditorState.pending_language_detection`. `apply_edit_request_response`
/// has a full `&mut Editor`, so it must drain that queue itself; nothing else
/// on this path (a server-initiated request answered from `drain_lsp`) ever
/// reaches `apply_script_effects`.
///
/// Fail oracle: drop the `self.detect_pending_languages()` call from
/// `apply_edit_request_response` — the opened buffer's `language` stays `None`.
#[test]
fn server_initiated_apply_edit_detects_language_of_newly_opened_file() {
    let tmp = safe_tempdir();
    let file = tmp.path().join("new.rs");
    std::fs::write(&file, "fn helper() {}\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut ed = editor_from("-[x]>\n");
    ed.state
        .config
        .languages
        .register_identity_no_rebuild("rust", &["rs"], &[], &[], None);
    ed.state
        .config
        .languages
        .rebuild_glob_set()
        .expect("rebuild ok");

    let params = serde_json::json!({
        "edit": {
            "changes": {
                uri.as_str(): [{
                    "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
                    "newText": "",
                }]
            }
        }
    });
    let result = ed.apply_edit_request_response(&params).unwrap();
    assert_eq!(result["applied"], serde_json::json!(true));

    let bid = ed
        .state
        .buffers
        .find_by_path(&canonical)
        .expect("workspace/applyEdit must have opened the file as a buffer");
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("rust"),
        "workspace/applyEdit must detect the newly-opened file's language"
    );
}

#[test]
fn server_initiated_apply_edit_answers_false_with_a_reason_on_bad_uri() {
    let mut ed = editor_from("-[x]>\n");
    let params = serde_json::json!({
        "edit": { "changes": { "not-a-uri": [] } }
    });
    let result = ed.apply_edit_request_response(&params).unwrap();
    assert_eq!(result["applied"], serde_json::json!(false));
    assert!(result["failureReason"].is_string());
}
