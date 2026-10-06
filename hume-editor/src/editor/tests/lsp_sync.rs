// Document sync glue: didOpen / didChange /
// didSave / didClose. The core test is the version-sync invariant:
// replaying the recorded protocol stream against an independent reference
// (hume_lsp's string-mirror, reused via the `test-util` feature) must
// reproduce the buffer's real final text and generation.

use super::lsp_rig::{LspRig, RigSpec};
use super::*;
use crate::editor::lsp::LspState;
use hume_engine::pipeline::BufferId;
use hume_lsp::sync::apply_events_to_string_mirror;
use hume_lsp::test_util::{NotificationLog, RecordingLspBackend};

/// The seed file every rust-buffer test below opens, cursor on its first
/// char.
const SEED: &str = "-[h]>ello world\n";

/// A `src/main.rs` holding [`SEED`], attached through the rig to a server
/// whose handshake is pre-scripted to succeed (INCREMENTAL sync), drained
/// once so the handshake completes and the queued `didOpen` flushes.
/// Returns the editor, the buffer id, and the shared notification log.
fn attached_editor(tmp: &tempfile::TempDir) -> (Editor, BufferId, NotificationLog) {
    let (backend, log, _requests) = RecordingLspBackend::with_default_handshake();
    let rig = LspRig::drained(tmp.path(), RigSpec::rust(SEED), backend);
    (rig.ed, rig.bid, log)
}

/// Same attach as `attached_editor`, but against a server that answers
/// `initialize` with `initialize_result` instead of the default scripted
/// (INCREMENTAL) handshake, for tests that need to control
/// `textDocumentSync` specifically.
fn attached_editor_with_handshake(
    tmp: &tempfile::TempDir,
    initialize_result: serde_json::Value,
) -> (Editor, BufferId, NotificationLog) {
    let (mut backend, log, _requests) = RecordingLspBackend::new();
    backend.respond_to("initialize", initialize_result);
    let rig = LspRig::drained(tmp.path(), RigSpec::rust(SEED), backend);
    (rig.ed, rig.bid, log)
}

/// Replays a recorded `(method, params)` stream against a plain `String`
/// mirror. `didOpen` seeds the mirror and the version; `didChange` applies
/// each `contentChanges` entry (ranged via the hume-lsp mirror, or whole-
/// document when `range` is absent: a FULL-sync server's case).
fn replay(log: &[(String, serde_json::Value)]) -> (String, Option<i64>) {
    let mut mirror = String::new();
    let mut version = None;
    for (method, params) in log {
        match method.as_str() {
            "textDocument/didOpen" => {
                let td = &params["textDocument"];
                mirror = td["text"].as_str().unwrap().to_string();
                version = td["version"].as_i64();
            }
            "textDocument/didChange" => {
                version = params["textDocument"]["version"].as_i64();
                for change in params["contentChanges"].as_array().unwrap() {
                    if change.get("range").is_some() {
                        let event: lsp_types::TextDocumentContentChangeEvent =
                            serde_json::from_value(change.clone()).unwrap();
                        mirror = apply_events_to_string_mirror(
                            mirror,
                            std::slice::from_ref(&event),
                            hume_rope::position_encoding::PositionEncoding::Utf16,
                        );
                    } else {
                        mirror = change["text"].as_str().unwrap().to_string();
                    }
                }
            }
            _ => {}
        }
    }
    (mirror, version)
}

/// Every `textDocument/didChange` entry in `log`, in order.
fn did_changes(log: &[(String, serde_json::Value)]) -> Vec<(String, serde_json::Value)> {
    log.iter()
        .filter(|(m, _)| m == "textDocument/didChange")
        .cloned()
        .collect()
}

/// Replays `log` against the independent string mirror and asserts it
/// reproduces the buffer's real text and the real the text version: the invariant
/// every LSP-sync test in this file ultimately checks. `context` names the
/// scenario in the assertion message (e.g. "a 3-step composed undo").
fn assert_mirror_matches(
    ed: &Editor,
    bid: BufferId,
    log: &[(String, serde_json::Value)],
    context: &str,
) {
    let real_text = ed.state.buffers.get(bid).text().to_string();
    let real_version = ed.state.buffers.get(bid).text().generation() as i64;
    let (mirrored, last_version) = replay(log);
    assert_eq!(
        mirrored, real_text,
        "{context}: replaying the didOpen+didChange stream must reproduce the buffer exactly"
    );
    assert_eq!(
        last_version,
        Some(real_version),
        "{context}: the last didChange's version must equal the buffer's real generation"
    );
}

#[test]
fn did_open_carries_full_text_and_language_id() {
    let tmp = safe_tempdir();
    let (_ed, _bid, log) = attached_editor(&tmp);

    let log = log.borrow();
    // `initialized` (handshake completion) plus the one didOpen it flushed
    // (nothing else queued yet).
    assert_eq!(
        log.len(),
        2,
        "expected [initialized, didOpen], got: {log:?}"
    );
    let (method, params) = &log[1];
    assert_eq!(method, "textDocument/didOpen");
    assert_eq!(params["textDocument"]["languageId"], "rust");
    assert_eq!(params["textDocument"]["text"], "hello world\n");
}

/// `didOpen`'s `languageId` is the language's registered `lsp_language_id`
/// override, not HUME's own language name. The actual bug this test guards:
/// a bare `typescript-language-server` rejects `"tsx"` and logs "Invalid
/// languageId", correcting it to `"typescriptreact"` itself. The expected
/// string here is a hardcoded literal, never derived from `name_of` /
/// `lsp_language_id_of`.
#[test]
fn did_open_carries_the_lsp_language_id_override_not_the_hume_name() {
    let tmp = safe_tempdir();
    let (backend, log, _requests) = RecordingLspBackend::with_default_handshake();
    let spec = RigSpec {
        language: "tsx",
        extension: "tsx",
        file: "component.tsx",
        marked: "-[c]>onst x = 1;\n",
        markers: &[],
        init: r#"(%define-language! "tsx" '("tsx") '() '() "typescriptreact" '())
                 (register-lsp-server! "typescript-language-server"
                                       #:command "typescript-language-server")
                 (set-language-servers! "tsx" '("typescript-language-server"))"#,
    };
    let _rig = LspRig::drained(tmp.path(), spec, backend);

    let log = log.borrow();
    let (method, params) = log
        .iter()
        .find(|(m, _)| m == "textDocument/didOpen")
        .expect("didOpen must have been sent");
    assert_eq!(method, "textDocument/didOpen");
    assert_eq!(params["textDocument"]["languageId"], "typescriptreact");
}

/// `didOpen` must queue behind the handshake, never write to
/// the wire before `initialize` completes: the spec forbids anything else
/// arriving first. Before the drain that carries the initialize response,
/// nothing has been sent at all; after it, the log is exactly
/// `initialized` then `didOpen`, in that order.
#[test]
fn did_open_is_queued_until_the_handshake_completes_then_flushes_in_order() {
    let tmp = safe_tempdir();
    let (backend, log, _requests) = RecordingLspBackend::with_default_handshake();
    let mut rig = LspRig::open(tmp.path(), RigSpec::rust(SEED), backend);
    assert!(
        log.borrow().is_empty(),
        "didOpen must not reach the wire before the handshake completes, got: {:?}",
        log.borrow()
    );

    rig.ed.drain_lsp();

    let log = log.borrow();
    let methods: Vec<&str> = log.iter().map(|(m, _)| m.as_str()).collect();
    assert_eq!(
        methods,
        vec!["initialized", "textDocument/didOpen"],
        "initialized must precede the flushed didOpen, in that exact order"
    );
}

#[test]
fn no_notifications_for_a_buffer_without_a_server() {
    let (backend, log, _requests) = RecordingLspBackend::new();
    let mut ed = editor_from("-[w]>ord\n");
    ed.state.lsp = LspState::with_backend(Box::new(backend));

    // No register-lsp-server! call at all: a scratch buffer must never attach.
    ed.step(key('i'));
    ed.step(key('X'));
    ed.step(key_esc());
    ed.drain_lsp();

    assert!(log.borrow().is_empty());
}

#[test]
fn version_sync_invariant_across_insert_delete_paste_undo_redo() {
    let tmp = safe_tempdir();
    let (mut ed, bid, log) = attached_editor(&tmp);

    // Re-select something to yank/delete/paste against, then run a session
    // covering every edit shape the invariant must survive.
    ed.feed_key(key('d')); // delete "w" of "word"           (DELETE)
    ed.feed_key(key('i'));
    ed.feed_key(key('Z'));
    ed.feed_key(key_esc()); // insert "Z"                     (INSERT)
    ed.feed_key(key('y')); // yank a char
    ed.feed_key(key('p')); // paste it back                   (PASTE)
    ed.feed_key(key('u')); // undo the paste                  (UNDO)
    ed.feed_key(key_ctrl('r')); // redo the paste              (REDO)

    ed.drain_lsp(); // flush every queued change to the log

    assert_mirror_matches(&ed, bid, &log.borrow(), "full session replay");
}

/// The case that exercises the composed walk: a counted `u` that
/// crosses several revisions must reach an INCREMENTAL-sync server as one
/// `didChange` for the net change, not one per revision it walked. Queuing
/// one change per revision would make this count 3.
#[test]
fn counted_undo_on_an_incremental_server_sends_one_didchange_for_the_whole_walk() {
    let tmp = safe_tempdir();
    let (mut ed, bid, log) = attached_editor(&tmp);

    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc()); // rev 1: insert "X"
    ed.feed_key(key('i'));
    ed.feed_key(key('Y'));
    ed.feed_key(key_esc()); // rev 2: insert "Y"
    ed.feed_key(key('i'));
    ed.feed_key(key('Z'));
    ed.feed_key(key_esc()); // rev 3: insert "Z"
    ed.drain_lsp(); // flush the three per-edit changes before the counted undo
    let before = log.borrow().len();

    ed.feed_key(key('3'));
    ed.feed_key(key('u')); // one composed walk back across all three revisions
    ed.drain_lsp();

    let did_changes_after = did_changes(&log.borrow()[before..]);
    assert_eq!(
        did_changes_after.len(),
        1,
        "a 3-step composed undo must send exactly one didChange, got: {did_changes_after:?}"
    );

    assert_mirror_matches(&ed, bid, &log.borrow(), "a 3-step composed undo");
}

#[test]
fn did_save_and_did_close_each_fire_once() {
    let tmp = safe_tempdir();
    let (mut ed, bid, log) = attached_editor(&tmp);

    ed.execute_typed("w", None).unwrap();
    ed.close_buffer(bid);

    let methods: Vec<&str> = log
        .borrow()
        .iter()
        .map(|(m, _)| m.as_str())
        .filter(|m| *m != "textDocument/didOpen" && *m != "initialized")
        .map(|m| match m {
            "textDocument/didSave" => "didSave",
            "textDocument/didClose" => "didClose",
            "exit" => "exit",
            other => panic!("unexpected notification: {other}"),
        })
        .collect();
    // Closing the server's last buffer stops the instance, hence the `exit`.
    assert_eq!(methods, vec!["didSave", "didClose", "exit"]);
}

/// `:e!` under macro replay, with an edit queued but not yet drained
/// (`drain_replay_queue` loops `handle_input` with no `drain_lsp` between
/// keys), immediately followed by a reload in the same window. The reload's
/// replacement is recorded like any other edit, so an INCREMENTAL server
/// gets it as one ranged `didChange` after the queued edit's own, at a
/// strictly higher version, and the replayed stream reproduces the reloaded
/// buffer.
#[test]
fn reload_sends_one_incremental_didchange_through_the_ordinary_record_path() {
    let tmp = safe_tempdir();
    let (mut ed, bid, log) = attached_editor(&tmp);

    // Queue an incremental change without draining.
    ed.feed_key(key('d'));

    let path = ed.state.buffers.get(bid).path().unwrap().to_path_buf();
    std::fs::write(&path, "hi\n").unwrap();
    ed.execute_typed("e!", None).unwrap();

    ed.drain_lsp();

    let changes: Vec<(Option<i64>, bool)> = did_changes(&log.borrow())
        .iter()
        .map(|(_, p)| {
            let version = p["textDocument"]["version"].as_i64();
            let ranged = p["contentChanges"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c.get("range").is_some());
            (version, ranged)
        })
        .collect();

    assert_eq!(
        changes.len(),
        2,
        "expected the queued edit's didChange then the reload's, got: {changes:?}"
    );
    assert!(
        changes.iter().all(|&(_, ranged)| ranged),
        "an INCREMENTAL server must get ranged events only, never a whole document: {changes:?}"
    );
    assert!(
        changes[0].0 < changes[1].0,
        "versions must strictly increase across the two didChange notifications: {changes:?}"
    );
    assert_eq!(ed.state.buffers.get(bid).text().to_string(), "hi\n");
    assert_mirror_matches(&ed, bid, &log.borrow(), "a queued edit then a reload");
}

/// A byte-identical `:e!` reload (the file on disk hasn't actually changed)
/// must send no `didChange` at all: `replace_text_recorded`'s identity branch
/// never changes the text version, so there is no new version to announce, and
/// `reload_buffer_in_place` must not fall back to sending one at the
/// buffer's unchanged version (which would be a version regression from the
/// server's point of view: a second notification carrying a version it
/// already has).
#[test]
fn identical_reload_sends_no_didchange() {
    let tmp = safe_tempdir();
    let (mut ed, bid, log) = attached_editor(&tmp);

    let path = ed.state.buffers.get(bid).path().unwrap().to_path_buf();
    std::fs::write(&path, "hello world\n").unwrap(); // identical to SEED's text
    ed.execute_typed("e!", None).unwrap();
    ed.drain_lsp();

    let changes = did_changes(&log.borrow());
    assert!(
        changes.is_empty(),
        "a byte-identical reload must send no didChange, got: {changes:?}"
    );
}

/// Regression: same root shape as the reload ordering test above, for `:w`.
/// An edit queued but not yet drained, immediately followed by a save in
/// the same window. `didSave` carries no text, so a server doing
/// save-triggered work (e.g. lint-on-save) must see the didChange
/// describing the just-saved content first, or it runs against a document
/// state one edit behind what's actually on disk.
#[test]
fn save_flushes_pending_change_before_did_save() {
    let tmp = safe_tempdir();
    let (mut ed, _bid, log) = attached_editor(&tmp);

    ed.feed_key(key('d'));
    ed.execute_typed("w", None).unwrap();

    let recorded = log.borrow();
    let methods: Vec<&str> = recorded
        .iter()
        .map(|(m, _)| m.as_str())
        .filter(|m| *m != "textDocument/didOpen" && *m != "initialized")
        .collect();

    assert_eq!(
        methods,
        vec!["textDocument/didChange", "textDocument/didSave"],
        "the queued edit's didChange must reach the wire before didSave, got: {methods:?}"
    );
}

/// A server declaring `textDocumentSync: FULL` (as `steel-language-server` does) must
/// never receive a ranged `didChange`. Per spec it ignores `range` and
/// treats each event's `text` as the whole new document, so a ranged insert
/// becomes the entire file, and every diagnostic position it computes next
/// is against that garbage. Every queued entry in one flush must collapse
/// into exactly one whole-document event, not one whole-document event per
/// entry (which would each carry the *final* text under a *stale* version).
#[test]
fn full_sync_server_gets_one_whole_document_didchange_per_flush() {
    let tmp = safe_tempdir();
    let (mut ed, bid, log) = attached_editor_with_handshake(
        &tmp,
        serde_json::json!({"capabilities": {"textDocumentSync": 1}}),
    );

    ed.feed_key(key('d')); // delete a char
    ed.feed_key(key('i'));
    ed.feed_key(key('Z'));
    ed.feed_key(key_esc()); // insert a char: two queued entries, one flush
    ed.drain_lsp();

    let did_changes = did_changes(&log.borrow());
    assert_eq!(
        did_changes.len(),
        1,
        "two queued edits on a FULL-sync server must collapse into one didChange, \
         got: {did_changes:?}"
    );
    let (_, params) = &did_changes[0];
    let changes = params["contentChanges"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    let event = &changes[0];
    assert!(
        event.get("range").is_none(),
        "a FULL-sync server must get a whole-document event, never a range: {event:?}"
    );
    let buf = ed.state.buffers.get(bid);
    assert_eq!(changes[0]["text"], buf.text().to_string());
    assert_eq!(
        params["textDocument"]["version"].as_i64(),
        Some(buf.text().generation() as i64)
    );
}

/// The INCREMENTAL-sync sibling of `full_sync_server_gets_one_whole_document_
/// didchange_per_flush`: an insert session queues one change per
/// keystroke, and each queued entry must reach the wire as its own
/// `didChange`. Replaying the whole sequence must still reproduce the
/// buffer exactly, checked against the independent string mirror.
#[test]
fn insert_session_sends_one_didchange_per_keystroke() {
    let tmp = safe_tempdir();
    let (mut ed, bid, log) = attached_editor(&tmp);

    ed.feed_key(key('i'));
    ed.feed_key(key('h'));
    ed.feed_key(key('e'));
    ed.feed_key(key('l'));
    ed.feed_key(key('l'));
    ed.feed_key(key('o'));
    ed.feed_key(key_esc()); // five text-mutating keystrokes, one queued entry each
    ed.drain_lsp();

    let did_changes = did_changes(&log.borrow());
    assert_eq!(
        did_changes.len(),
        5,
        "an insert session on an INCREMENTAL-sync server sends one didChange \
         per keystroke, got: {did_changes:?}"
    );

    assert_mirror_matches(
        &ed,
        bid,
        &log.borrow(),
        "insert session, one didChange per keystroke",
    );
}

/// A server declaring `textDocumentSync: NONE` must receive no `didChange`
/// at all, but the diagnostics store must still remap through the edit, so
/// positions from an earlier publish don't silently go stale just because
/// there was nowhere to announce the edit.
#[test]
fn none_sync_server_gets_no_didchange_but_diagnostics_still_remap() {
    let tmp = safe_tempdir();
    let (mut backend, log, _requests) = RecordingLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": {"textDocumentSync": 0}}),
    );
    // `apply-text-edits!` only accepts a server-tagged wire edit (via a
    // real response); this canned response is what the `:stash` dispatch
    // below turns into one.
    backend.respond_to(
        "test/textEdits",
        serde_json::json!([{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "X"}]),
    );
    let mut rig = LspRig::drained(tmp.path(), RigSpec::rust(SEED), backend);

    // "world" is chars 6..11 of "hello world\n".
    let sid = rig.sid("rust-analyzer");
    rig.publish(
        sid,
        serde_json::json!([{
            "range": {"start": {"line": 0, "character": 6}, "end": {"line": 0, "character": 11}},
            "severity": 1,
            "message": "boom",
        }]),
    );
    let bid = rig.bid;
    let mut ed = rig.ed;

    let before: Vec<(usize, usize)> = ed
        .state
        .buffer_positions
        .diagnostics
        .spans_for_test(bid)
        .collect();
    assert_eq!(
        before,
        vec![(6, 11)],
        "diagnostic must ingest at its wire position"
    );

    // Insert one char before the diagnostic: it must shift by one.
    run(
        &mut ed,
        tmp.path(),
        r#"(define stashed-edits (box #f))
           (define-typed-command! "stash" "" (lambda (bid)
             (lsp-request! bid "test/textEdits" (hash) (lambda (err res) (set-box! stashed-edits res)))))
           (define-typed-command! "go" "" (lambda (bid)
             (apply-text-edits! bid (json-list (unbox stashed-edits)))))"#,
    );
    type_cmd(&mut ed, ":stash");
    ed.drain_lsp();
    ed.settle();
    type_cmd(&mut ed, ":go");
    ed.drain_lsp();

    assert!(
        log.borrow()
            .iter()
            .all(|(m, _)| m != "textDocument/didChange"),
        "a NONE-sync server must get no didChange, got: {:?}",
        log.borrow()
    );
    let after: Vec<(usize, usize)> = ed
        .state
        .buffer_positions
        .diagnostics
        .spans_for_test(bid)
        .collect();
    assert_eq!(
        after,
        vec![(7, 12)],
        "the diagnostic must still remap through the edit despite no wire send"
    );
}

/// #:init-options registered through the real Steel path must reach the
/// spawned server's `initialize` request as `initializationOptions`:
/// end-to-end proof that `LspServerConfig.init_options` isn't dead weight.
#[test]
fn register_lsp_server_init_options_reach_the_initialize_request() {
    let tmp = safe_tempdir();
    let (backend, _notifications, requests) = RecordingLspBackend::with_default_handshake();
    let spec = RigSpec::rust(SEED).with_init(
        r#"(register-lsp-server! "rust-analyzer" #:command "rust-analyzer"
                                 #:init-options (hash "check" (hash "command" "clippy")))
           (set-language-servers! "rust" '("rust-analyzer"))"#,
    );
    let _rig = LspRig::drained(tmp.path(), spec, backend);

    let recorded = requests.borrow();
    let initialize_calls: Vec<_> = recorded
        .iter()
        .filter(|(_, method, _)| method == "initialize")
        .collect();
    assert_eq!(
        initialize_calls.len(),
        1,
        "expected exactly one initialize request, got: {recorded:?}"
    );
    let (_, _, params) = initialize_calls[0];
    assert_eq!(
        params["initializationOptions"]["check"]["command"],
        "clippy"
    );
}
