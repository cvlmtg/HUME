// Edit + navigation primitives: apply-text-edits!,
// apply-workspace-edit!, goto-location!, and the workspace/applyEdit
// server-request swap.

use std::path::Path;

use super::*;
use crate::editor::buffer::Buffer;
use crate::editor::lsp::LspState;
use hume_editing::selection::SelectionSet;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;

/// Attaches the focused buffer to a `Running` scripted server negotiated on
/// UTF-8, after handing `configure` the backend to script canned responses
/// on (called before the handshake, matching `lsp_bridge.rs`'s `setup_with`
/// convention). Negotiating the non-default encoding here does not by
/// itself prove `apply-text-edits!` consults it rather than assuming
/// UTF-16 — a wire offset only diverges between the two encodings on a line
/// with a multi-byte character, so most fixtures below (all ASCII) would
/// pass identically either way. The actual proof is
/// `apply_text_edits_utf8_server_uses_byte_offsets_not_utf16_units`, whose
/// fixture is chosen specifically to make that divergence observable.
fn attach_running_utf8_server_with(
    ed: &mut Editor,
    configure: impl FnOnce(&mut InlineLspBackend, ServerId),
) -> ServerId {
    let mut backend = InlineLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": {"positionEncoding": "utf-8"}}),
    );
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    configure(&mut backend, sid);
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

/// [`attach_running_utf8_server_with`] for a test with no canned response of
/// its own to script.
fn attach_running_utf8_server(ed: &mut Editor) -> ServerId {
    attach_running_utf8_server_with(ed, |_, _| {})
}

/// One wire `TextEdit` JSON object, the shape `apply-text-edits!` now
/// requires every entry to be tagged as (via a real response) — the
/// `((start-line . start-char) (end-line . end-char) text)` hand-built
/// tuple shape this used to build directly no longer exists.
fn wire_edit(start: (u32, u32), end: (u32, u32), new_text: &str) -> serde_json::Value {
    serde_json::json!({
        "range": {
            "start": {"line": start.0, "character": start.1},
            "end": {"line": end.0, "character": end.1},
        },
        "newText": new_text,
    })
}

/// Sends `edits` (built with [`wire_edit`]) through a scripted
/// `test/textEdits` request/response round trip and applies the response to
/// the focused buffer via `apply-text-edits!` — the one way a test can hand
/// it a server-tagged value, now that the hand-built tuple shape is gone.
/// `expect_gen_clause` is spliced into the call verbatim (empty string to
/// omit `#:expect-generation`).
fn apply_wire_text_edits(
    ed: &mut Editor,
    tmp: &std::path::Path,
    edits: Vec<serde_json::Value>,
    expect_gen_clause: &str,
) {
    attach_running_utf8_server_with(ed, |backend, _sid| {
        backend.respond_to("test/textEdits", serde_json::Value::Array(edits));
    });
    run(
        ed,
        tmp,
        &format!(
            r#"(define-typed-command! "go" "" (lambda (bid)
                 (lsp-request bid "test/textEdits" (hash) (lambda (err res)
                   (apply-text-edits! bid (json-list res){expect_gen_clause})))))"#
        ),
    );
    type_cmd(ed, ":go");
    ed.drain_lsp();
    ed.settle();
}

// ── apply-text-edits! ───────────────────────────────────────────────────────

#[test]
fn apply_text_edits_single_edit() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    apply_wire_text_edits(
        &mut ed,
        tmp.path(),
        vec![wire_edit((0, 1), (0, 3), "XY")],
        "",
    );
    assert_eq!(ed.doc().text().to_string(), "aXYdef\n");
}

/// A server can send `new_text` in its own platform's line-ending
/// convention — nothing guarantees it's `\n`-only. `apply-text-edits!` must
/// normalize it the same way every other text-insertion path does.
#[test]
fn apply_text_edits_normalizes_crlf_in_new_text() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    apply_wire_text_edits(
        &mut ed,
        tmp.path(),
        vec![wire_edit((0, 1), (0, 3), "X\r\nY")],
        "",
    );
    assert_eq!(ed.doc().text().to_string(), "aX\nYdef\n");
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
    apply_wire_text_edits(
        &mut ed,
        tmp.path(),
        vec![wire_edit((0, 3), (0, 4), "X")],
        "",
    );
    assert_eq!(ed.doc().text().to_string(), "aéXcdef\n");
}

#[test]
fn apply_text_edits_multiple_edits_same_line_apply_descending() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    // Two edits on the same line, given out of order — must not corrupt
    // each other's offsets (the classic ascending-with-fixups bug).
    apply_wire_text_edits(
        &mut ed,
        tmp.path(),
        vec![
            wire_edit((0, 0), (0, 1), "Z"),
            wire_edit((0, 4), (0, 5), "W"),
        ],
        "",
    );
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
    apply_wire_text_edits(
        &mut ed,
        tmp.path(),
        vec![
            wire_edit((0, 0), (0, 0), "1"),
            wire_edit((0, 0), (0, 0), "2"),
        ],
        "",
    );
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
    apply_wire_text_edits(
        &mut ed,
        tmp.path(),
        vec![
            wire_edit((0, 0), (0, 2), "AA"),
            wire_edit((0, 2), (0, 4), "BB"),
        ],
        "",
    );
    assert_eq!(ed.doc().text().to_string(), "AABBef\n");
}

#[test]
fn apply_text_edits_overlapping_rejected() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    apply_wire_text_edits(
        &mut ed,
        tmp.path(),
        vec![
            wire_edit((0, 0), (0, 3), "A"),
            wire_edit((0, 2), (0, 5), "B"),
        ],
        "",
    );
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
    apply_wire_text_edits(
        &mut ed,
        tmp.path(),
        vec![wire_edit((0, 3), (0, 0), "X")],
        "",
    );
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
    apply_wire_text_edits(
        &mut ed,
        tmp.path(),
        vec![
            wire_edit((0, 0), (0, 1), "Z"),
            wire_edit((0, 5), (0, 6), "W"),
        ],
        "",
    );
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
    let stale_gen = ed.doc().text_gen;
    // Make an unrelated edit first so the buffer's generation moves past
    // what the (fictional) LSP response was computed against.
    ed.handle_key(key('i'));
    ed.handle_key(key('!'));
    ed.handle_key(key_esc());

    let before = ed.doc().text().to_string();
    apply_wire_text_edits(
        &mut ed,
        tmp.path(),
        vec![wire_edit((0, 0), (0, 1), "Z")],
        &format!(" #:expect-generation {stale_gen}"),
    );
    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "a stale expect-generation must reject the edit"
    );
}

/// `apply-text-edits!` only accepts server-tagged wire edits (via a real
/// response) — a hand-built entry (constructed directly in Scheme, never
/// crossed through a response) has no producing server to have negotiated
/// an encoding with, and is rejected before ever reaching the host, rather
/// than silently guessing UTF-16 the way the removed `encoding_for_buffer`
/// once did.
#[test]
fn apply_text_edits_rejects_a_hand_built_edit() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    attach_running_utf8_server(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda (bid)
             (apply-text-edits! bid
               (list (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                       "end" (hash "line" 0 "character" 1))
                      "newText" "X")))))"#,
    );
    let before = ed.doc().text().to_string();
    type_cmd(&mut ed, ":go");
    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "a hand-built (untagged) edit must be rejected, not applied with a guessed encoding"
    );
}

// ── apply-workspace-edit! ────────────────────────────────────────────────────

/// Sends `wsedit` (a `WorkspaceEdit` JSON blob) through a scripted
/// `test/workspaceEdit` request/response round trip and applies the
/// response via `apply-workspace-edit!` — the one way a test can hand it a
/// server-tagged value, now that `#:from`/a hand-built hashmap no longer
/// carries any encoding at all.
fn apply_wire_workspace_edit(ed: &mut Editor, tmp: &std::path::Path, wsedit: serde_json::Value) {
    attach_running_utf8_server_with(ed, |backend, _sid| {
        backend.respond_to("test/workspaceEdit", wsedit);
    });
    run(
        ed,
        tmp,
        r#"(define-typed-command! "go" "" (lambda (bid)
             (lsp-request bid "test/workspaceEdit" (hash) (lambda (err res)
               (apply-workspace-edit! bid res)))))"#,
    );
    type_cmd(ed, ":go");
    ed.drain_lsp();
    ed.settle();
}

#[test]
fn apply_workspace_edit_changes_shape() {
    let tmp = safe_tempdir();
    let file = tmp.path().join("a.txt");
    std::fs::write(&file, "abcdef\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut ed = editor_from("-[x]>\n");
    apply_wire_workspace_edit(
        &mut ed,
        tmp.path(),
        serde_json::json!({"changes": {uri.as_str(): [
            {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
             "newText": "XYZ"},
        ]}}),
    );
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
    apply_wire_workspace_edit(
        &mut ed,
        tmp.path(),
        serde_json::json!({"documentChanges": [
            {"textDocument": {"uri": uri.as_str(), "version": null},
             "edits": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                         "newText": "Z"}]},
        ]}),
    );
    let bid = ed.state.buffers.find_by_path(&canonical).unwrap();
    assert_eq!(ed.state.buffers.get(bid).text().to_string(), "Zbcdef\n");
}

/// `apply-workspace-edit!` decodes the whole edit in the *response's own*
/// tagged encoding, not the (possibly brand-new, unattached) target file's
/// — the Steel-entry-point counterpart to
/// `server_initiated_apply_edit_into_unopened_file_uses_the_requesting_servers_encoding`.
/// Same divergent fixture: byte offset 3 is `b`, UTF-16 code-unit offset 3
/// is `c`.
#[test]
fn apply_workspace_edit_into_unopened_file_uses_the_responses_encoding() {
    let tmp = safe_tempdir();
    let file = tmp.path().join("new.txt");
    std::fs::write(&file, "aébcdef\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut ed = editor_from("-[x]>\n");
    apply_wire_workspace_edit(
        &mut ed,
        tmp.path(),
        serde_json::json!({"changes": {uri.as_str(): [
            {"range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 4}},
             "newText": "X"},
        ]}}),
    );

    let bid = ed
        .state
        .buffers
        .find_by_path(&canonical)
        .expect("file must have been opened as a buffer");
    assert_eq!(
        ed.state.buffers.get(bid).text().to_string(),
        "aéXcdef\n",
        "must decode the edit in the response's own UTF-8 encoding, not guess UTF-16"
    );
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

    apply_wire_workspace_edit(
        &mut ed,
        tmp.path(),
        serde_json::json!({"changes": {
            opened_uri.as_str(): [
                {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                 "newText": "H"},
            ],
            unopened_uri.as_str(): [
                {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                 "newText": "W"},
            ],
        }}),
    );

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
    apply_wire_workspace_edit(
        &mut ed,
        tmp.path(),
        // The invalid entry is listed FIRST — documentChanges is an ordered
        // list (unlike `changes`' hashmap), so validation reaches it before
        // ever touching the valid file.
        serde_json::json!({"documentChanges": [
            {"textDocument": {"uri": invalid_uri.as_str(), "version": null},
             "edits": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                         "newText": "Z"}]},
            {"textDocument": {"uri": ok_uri.as_str(), "version": null},
             "edits": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                         "newText": "Z"}]},
        ]}),
    );

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

/// A file later in the plan with another pane's open insert session must
/// abort the whole edit *before* any earlier file's changeset is committed —
/// `doc_ops::check_no_conflicting_session` runs in the planning loop now,
/// not only inside `commit_changeset`'s own check partway through the
/// commit loop (which would have already mutated `ok.txt` by the time
/// `conflict.txt` is reached). Called directly through `EditHost`, not the
/// `apply_wire_workspace_edit`/`lsp-request` round trip — this needs no LSP
/// server, only a second pane to issue the edit from.
#[test]
fn apply_workspace_edit_conflicting_session_on_another_pane_leaves_earlier_files_untouched() {
    use crate::editor::commands::open_pane_in_layout;
    use hume_editing::text::BufferText;
    use hume_engine::pipeline::Direction;
    use hume_scripting::PaneHandle;
    use hume_scripting::host::EditHost;

    let tmp = safe_tempdir();
    let ok_path = tmp.path().join("ok.txt");
    std::fs::write(&ok_path, "abcdef\n").unwrap();
    let ok_canonical = std::fs::canonicalize(&ok_path).unwrap();
    let ok_uri = hume_lsp::uri::path_to_uri(&ok_canonical).unwrap();

    let conflict_path = tmp.path().join("conflict.txt");
    std::fs::write(&conflict_path, "ghijkl\n").unwrap();
    let conflict_canonical = std::fs::canonicalize(&conflict_path).unwrap();
    let conflict_uri = hume_lsp::uri::path_to_uri(&conflict_canonical).unwrap();

    let mut ed = editor_from("-[x]>\n");
    // Open conflict.txt on the focused pane and start (but don't close) an
    // Insert session on it — an open `edit_group` that must survive.
    ed.execute_typed("e", Some(conflict_path.to_str().unwrap()))
        .unwrap();
    let conflict_bid = ed.focused_buffer_id();
    ed.feed_key(key('i'));
    type_chars(&mut ed, "Z");
    assert_eq!(
        ed.state.mode(),
        Mode::Insert,
        "sanity: Insert open on conflict.txt"
    );

    // A second, unrelated pane issues the workspace edit — "remote" relative
    // to the focused pane's own open session, the same shape a `call!`
    // targeting a different pane would produce.
    let pid_a = ed.state.focus.id();
    let other_bid = ed.open_buffer(Buffer::new(
        BufferText::from("misc\n"),
        SelectionSet::default(),
    ));
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        other_bid,
        Direction::Horizontal,
    )
    .expect("split must succeed");

    // ok.txt listed first, conflict.txt second — under the old commit-time-
    // only check, ok.txt would already have been committed by the time
    // conflict.txt's own conflict aborted the loop.
    let wsedit = serde_json::json!({"documentChanges": [
        {"textDocument": {"uri": ok_uri.as_str(), "version": null},
         "edits": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                     "newText": "Z"}]},
        {"textDocument": {"uri": conflict_uri.as_str(), "version": null},
         "edits": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                     "newText": "Z"}]},
    ]});

    let pane_b = PaneHandle::with_pane(other_bid, pid_b);
    let result = live_host!(ed).apply_workspace_edit(
        pane_b,
        &wsedit,
        hume_rope::position_encoding::PositionEncoding::Utf16,
        None,
    );
    assert!(
        result.is_err(),
        "must refuse: conflict.txt has an open session on another pane"
    );

    let ok_bid = ed
        .state
        .buffers
        .find_by_path(&ok_canonical)
        .expect("ok.txt is still opened as a side effect of planning, just never committed");
    assert_eq!(
        ed.state.buffers.get(ok_bid).text().to_string(),
        "abcdef\n",
        "ok.txt must be untouched — the conflict on conflict.txt must abort before any commit"
    );
    assert_eq!(
        ed.state.buffers.get(conflict_bid).text().to_string(),
        "Zghijkl\n",
        "conflict.txt keeps only its own still-open session's own edit, not the workspace edit"
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
    apply_wire_workspace_edit(
        &mut ed,
        tmp.path(),
        serde_json::json!({"documentChanges": [
            {"textDocument": {"uri": uri.as_str(), "version": null},
             "edits": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                         "newText": "X"}]},
            {"textDocument": {"uri": uri.as_str(), "version": null},
             "edits": [{"range": {"start": {"line": 0, "character": 1}, "end": {"line": 0, "character": 2}},
                         "newText": "Y"}]},
        ]}),
    ); // must not panic

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
        r#"(define-typed-command! "go" "" (lambda (bid)
             (goto-location! bid (list bid 0 3))))"#,
    );
    let before = state(&ed);
    type_cmd(&mut ed, ":go");
    assert_ne!(state(&ed), before);
    assert_eq!(ed.current_selections().primary().head(), co(3));

    // A jump entry was pushed — Ctrl-o must return to the origin.
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
        r#"(define-typed-command! "go" "" (lambda (bid)
             (goto-location! bid (list bid 0 0))))"#,
    );

    // `%` — jump-flagged, moves elsewhere, records a jump.
    ed.handle_key(key('%'));
    let after_percent = state(&ed);

    // Jump backward to the original head-0 position.
    ed.handle_key(key_ctrl('o'));
    let back_at_start = state(&ed);
    assert_ne!(back_at_start, after_percent);
    assert_eq!(ed.current_selections().primary().head(), co(0));

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
            r#"(define-typed-command! "go" "" (lambda (bid)
                 (goto-location! bid (list {:?} 0 1))))"#,
            file.to_str().unwrap()
        ),
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(ed.focused_buffer_id(), other_bid);
    assert_eq!(ed.current_selections().primary().head(), co(1));
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
            r#"(define-typed-command! "go" "" (lambda (bid)
                 (goto-location! bid (list {:?} 0 2))))"#,
            file.to_str().unwrap()
        ),
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(ed.doc().text().to_string(), "hello\n");
    assert_eq!(ed.current_selections().primary().head(), co(2));
}

#[test]
fn goto_location_char_indexed_target_past_eof_clamps_to_the_last_char() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda (bid)
             (goto-location! bid (list bid 999 0))))"#,
    );
    type_cmd(&mut ed, ":go");
    let len_chars = ed.doc().text().end();
    let head = ed.current_selections().primary().head();
    assert!(
        head < len_chars,
        "head must satisfy head < len_chars() — got head={head:?}, len_chars={len_chars:?}"
    );
    assert_eq!(
        head,
        len_chars.shift(-1),
        "a target past EOF must clamp to the buffer's last char"
    );
}

/// `goto_location` must center the jump the same way `zz` does — by display
/// line, via `scroll::scroll_cursor_to_display_line` — not by re-deriving a
/// buffer-line-based centering of its own. The two only agree when nothing
/// wraps; under wrap they diverge, and a hand-rolled line-based centering
/// leaves `top()`'s slot untouched entirely (`Viewport::top_at`'s own
/// doc names this exact call site as why a stale write must self-heal on
/// the next read).
#[test]
fn goto_location_centers_by_display_line_not_buffer_line_under_wrap() {
    // Each line is 25 'x's, wrapped at width 10 into three display lines —
    // 10 + 10 + 5, the last one short of the wrap width so it doesn't also
    // trigger the trailing '\n' sentinel's own wrap onto a further display
    // line (`format_buffer_line`'s end-of-line sentinel handling). A jump
    // deep into the file makes buffer-line and display-line centering
    // diverge sharply: line-based would center on line 20 directly;
    // display-line must center on line 20's own first display line, three
    // times as far down.
    let content: String = (0..30).map(|_| format!("{}\n", "x".repeat(25))).collect();
    let text = hume_editing::text::BufferText::from(content.as_str());
    let sels = SelectionSet::single(hume_editing::selection::Selection::collapsed(co(0)));
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    let pid = ed.state.focus.id();
    ed.execute_typed("set", Some("pane wrap-mode=soft:10"))
        .unwrap();
    ed.view.panes[pid].viewport.height = 10;

    let tmp = safe_tempdir();
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda (bid)
             (goto-location! bid (list bid 20 0))))"#,
    );
    type_cmd(&mut ed, ":go");

    let cursor_char = ed.current_selections().primary().head();
    let bid = ed.focused_buffer_id();
    let key = ed.state.format_key(&ed.view.panes[pid]);
    let (mut dlm, viewport) = crate::editor::commands::pane_display_lines(
        ed.state.buffers.get(bid),
        &mut ed.view.panes[pid],
        key,
    );
    let top = viewport.top();
    let cursor_pos = dlm.locate_display_line(cursor_char);
    assert_eq!(
        dlm.distance(top, cursor_pos, 20),
        Some(5),
        "the cursor must land exactly height/2 (5) display lines below the new top"
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
            r#"(define-typed-command! "go" "" (lambda (bid)
             (goto-location! bid (list {dir_target:?} 0 0))))"#
        ),
    );
    let before = state(&ed);
    type_cmd(&mut ed, ":go");
    assert_eq!(state(&ed), before, "a failed goto must not move the cursor");

    // No jump entry means Ctrl-o has nothing to do — state stays put.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before);
}

/// `(goto-location! loc)`'s wire shape decodes `loc`'s position in the
/// response's own tagged encoding — same divergent fixture as
/// `apply_text_edits_utf8_server_uses_byte_offsets_not_utf16_units`: on
/// "aébcdef", byte offset 3 is `b`, UTF-16 code-unit offset 3 is `c`.
///
/// Fail oracle: guessing UTF-16 instead of reading the response's own tag
/// would land on `c` instead of `b`.
#[test]
fn goto_location_wire_shape_decodes_with_the_responses_encoding() {
    let tmp = safe_tempdir();
    let file = tmp.path().join("target.txt");
    std::fs::write(&file, "aébcdef\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut ed = editor_from("-[a]>bcdef\n");
    attach_running_utf8_server_with(&mut ed, |backend, _sid| {
        backend.respond_to(
            "test/gotoTarget",
            serde_json::json!({
                "uri": uri.as_str(),
                "range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 3}},
            }),
        );
    });
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda (bid)
             (lsp-request bid "test/gotoTarget" (hash) (lambda (err res)
               (goto-location! bid res)))))"#,
    );
    type_cmd(&mut ed, ":go");
    ed.drain_lsp();
    ed.settle();

    let bid = ed
        .state
        .buffers
        .find_by_path(&canonical)
        .expect("goto-location! must have opened the target file");
    assert_eq!(ed.focused_buffer_id(), bid);
    assert_eq!(
        ed.current_selections().primary().head(),
        co(2),
        "byte offset 3 on \"aébcdef\" names char index 2 ('b') — a UTF-16 guess would land on \
         char index 3 ('c') instead"
    );
}

/// `(goto-location! bid (list path line char-col))` on a path that doesn't exist yet
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
            r#"(define-typed-command! "go" "" (lambda (bid)
             (goto-location! bid (list {target_str:?} 0 0))))"#
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

/// A server-initiated `workspace/applyEdit` into a file this edit is
/// opening for the *first time* must still decode positions in the
/// requesting server's own negotiated encoding — the new buffer has no
/// attached server yet (`detect_pending_languages` only runs after this
/// returns), so resolving the encoding from the *target* buffer instead of
/// the *requesting server* would silently fall back to UTF-16 and corrupt
/// any non-ASCII line. Same fixture divergence as
/// `apply_text_edits_utf8_server_uses_byte_offsets_not_utf16_units`: byte
/// offset 3 is `b` (`a`, then `é`'s 2 bytes, then `b`), but UTF-16 code-unit
/// offset 3 is `c` (`é` is one code unit).
#[test]
fn server_initiated_apply_edit_into_unopened_file_uses_the_requesting_servers_encoding() {
    let tmp = safe_tempdir();
    let file = tmp.path().join("new.txt");
    std::fs::write(&file, "aébcdef\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut ed = editor_from("-[x]>\n");
    let sid = attach_running_utf8_server(&mut ed);
    let params = serde_json::json!({
        "edit": {
            "changes": {
                uri.as_str(): [{
                    "range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 4}},
                    "newText": "X",
                }]
            }
        }
    });
    let result = ed.apply_edit_request_response(&params, sid).unwrap();
    assert_eq!(result["applied"], serde_json::json!(true));

    let bid = ed
        .state
        .buffers
        .find_by_path(&canonical)
        .expect("workspace/applyEdit must have opened the file as a buffer");
    assert_eq!(
        ed.state.buffers.get(bid).text().to_string(),
        "aéXcdef\n",
        "must decode the edit in the requesting server's UTF-8 encoding, not guess UTF-16"
    );
}

#[test]
fn server_initiated_apply_edit_actually_applies_and_answers_true() {
    let tmp = safe_tempdir();
    let file = tmp.path().join("srv.txt");
    std::fs::write(&file, "abcdef\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut ed = editor_from("-[x]>\n");
    let sid = attach_running_utf8_server(&mut ed);
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
    let result = ed.apply_edit_request_response(&params, sid).unwrap();
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
    let sid = attach_running_utf8_server(&mut ed);
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
    let result = ed.apply_edit_request_response(&params, sid).unwrap();
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
    let sid = attach_running_utf8_server(&mut ed);
    let params = serde_json::json!({
        "edit": { "changes": { "not-a-uri": [] } }
    });
    let result = ed.apply_edit_request_response(&params, sid).unwrap();
    assert_eq!(result["applied"], serde_json::json!(false));
    assert!(result["failureReason"].is_string());
}
