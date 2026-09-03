// Document sync glue: didOpen / didChange /
// didSave / didClose. The load-bearing test is the version-sync invariant:
// replaying the recorded protocol stream against an independent oracle
// (hume_lsp's string-mirror, reused via the `test-util` feature) must
// reproduce the buffer's real final text and text_gen exactly.

use std::path::Path;

use super::*;
use crate::editor::lsp::LspState;
use crate::editor::scripting_setup::make_init_host;
use hume_engine::pipeline::BufferId;
use hume_lsp::backend::LspBackend;
use hume_lsp::client::LspClient;
use hume_lsp::sync::apply_events_to_string_mirror;
use hume_lsp::test_util::{NotificationLog, RecordingLspBackend};
use hume_scripting::ScriptingHost;

fn eval_register(ed: &mut Editor, host: &mut ScriptingHost, source: &str, tmp: &Path) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    let effects = {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init");
    ed.apply_script_effects(effects);
}

/// Sets up an editor with a `RecordingLspBackend` (handshake pre-scripted
/// to succeed), a registered "rust" server, and a real on-disk file
/// matching (so `:e` triggers a genuine attach through
/// `lsp_attach_buffer`). Drains once so the handshake completes and
/// anything queued while `Starting` (currently just `didOpen`) flushes.
/// Returns the editor, the buffer id, and the shared notification log.
fn attached_editor(tmp: &tempfile::TempDir) -> (Editor, BufferId, NotificationLog) {
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::write(root.join("Cargo.toml"), b"").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let file = root.join("src/main.rs");
    std::fs::write(&file, "hello world\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let (backend, log, _requests) = RecordingLspBackend::with_default_handshake();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[], None)
        .unwrap();

    let mut host = ScriptingHost::new();
    eval_register(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))"#,
        tmp.path(),
    );

    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    ed.drain_lsp();
    let bid = ed.focused_buffer_id();
    (ed, bid, log)
}

/// Same attach as `attached_editor`, but against a server that answers
/// `initialize` with `initialize_result` instead of the default scripted
/// (INCREMENTAL) handshake — for tests that need to control
/// `textDocumentSync` specifically.
fn attached_editor_with_handshake(
    tmp: &tempfile::TempDir,
    initialize_result: serde_json::Value,
) -> (Editor, BufferId, NotificationLog) {
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::write(root.join("Cargo.toml"), b"").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let file = root.join("src/main.rs");
    std::fs::write(&file, "hello world\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let (mut backend, log, _requests) = RecordingLspBackend::new();
    backend.respond_to("initialize", initialize_result);
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[], None)
        .unwrap();

    let mut host = ScriptingHost::new();
    eval_register(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))"#,
        tmp.path(),
    );

    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    ed.drain_lsp();
    let bid = ed.focused_buffer_id();
    (ed, bid, log)
}

/// Replays a recorded `(method, params)` stream against a plain `String`
/// mirror. `didOpen` seeds the mirror and the version; `didChange` applies
/// each `contentChanges` entry (ranged via the hume-lsp oracle, or whole-
/// document when `range` is absent — the `:e!`/`reload` case).
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

#[test]
fn did_open_carries_full_text_and_language_id() {
    let tmp = safe_tempdir();
    let (_ed, _bid, log) = attached_editor(&tmp);

    let log = log.borrow();
    // `initialized` (handshake completion) plus the one didOpen it flushed
    // — nothing else queued yet.
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
/// override, not HUME's own language name — the actual bug this test guards:
/// a bare `typescript-language-server` rejects `"tsx"` and logs "Invalid
/// languageId", correcting it to `"typescriptreact"` itself. The expected
/// string here is a hardcoded literal, never derived from `name_of` /
/// `lsp_language_id_of` — an independent oracle.
///
/// Flip: reverting `lsp_did_open` to `name_of` instead of
/// `lsp_language_id_of` makes this fail (`languageId` would be `"tsx"`).
#[test]
fn did_open_carries_the_lsp_language_id_override_not_the_hume_name() {
    let tmp = safe_tempdir();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let file = root.join("component.tsx");
    std::fs::write(&file, "const x = 1;\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let (backend, log, _requests) = RecordingLspBackend::with_default_handshake();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.state
        .config
        .languages
        .register_identity("tsx", &["tsx"], &[], &[], Some("typescriptreact"))
        .unwrap();

    let mut host = ScriptingHost::new();
    eval_register(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "tsx" #:command "typescript-language-server")"#,
        tmp.path(),
    );

    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    ed.drain_lsp();

    let log = log.borrow();
    let (method, params) = log
        .iter()
        .find(|(m, _)| m == "textDocument/didOpen")
        .expect("didOpen must have been sent");
    assert_eq!(method, "textDocument/didOpen");
    assert_eq!(params["textDocument"]["languageId"], "typescriptreact");
}

/// Fix 1: `lsp_did_open` must queue behind the handshake, never write to
/// the wire before `initialize` completes — the spec forbids anything else
/// arriving first. Before the drain that carries the initialize response,
/// nothing has been sent at all; after it, the log is exactly
/// `initialized` then `didOpen`, in that order.
#[test]
fn did_open_is_queued_until_the_handshake_completes_then_flushes_in_order() {
    let tmp = safe_tempdir();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::write(root.join("Cargo.toml"), b"").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let file = root.join("src/main.rs");
    std::fs::write(&file, "hello world\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let (backend, log, _requests) = RecordingLspBackend::with_default_handshake();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[], None)
        .unwrap();
    let mut host = ScriptingHost::new();
    eval_register(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))"#,
        tmp.path(),
    );

    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    assert!(
        log.borrow().is_empty(),
        "didOpen must not reach the wire before the handshake completes, got: {:?}",
        log.borrow()
    );

    ed.drain_lsp();

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
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));

    // No register-lsp-server! call at all — a scratch buffer must never attach.
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

    let real_text = ed.state.buffers.get(bid).text().to_string();
    let real_version = ed.state.buffers.get(bid).text_gen as i64;

    let (mirrored, last_version) = replay(&log.borrow());
    assert_eq!(
        mirrored, real_text,
        "replaying the didOpen+didChange stream must reproduce the buffer exactly"
    );
    assert_eq!(
        last_version,
        Some(real_version),
        "the last didChange's version must equal the buffer's real text_gen"
    );
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
            other => panic!("unexpected notification: {other}"),
        })
        .collect();
    assert_eq!(methods, vec!["didSave", "didClose"]);
}

/// Regression: `:e!` under macro replay — an edit queued but not yet
/// drained (`drain_replay_queue` loops `handle_input` with no `drain_lsp`
/// between keys), immediately followed by a reload in the same window.
/// Before the fix, the reload's whole-document `didChange` (at the new,
/// higher version) reached the wire ahead of the still-queued incremental
/// one (computed against the pre-reload text, at an older version) — a
/// version regression the server can't recover from, permanently
/// desyncing its copy of the document for the rest of the session.
#[test]
fn reload_flushes_pending_change_before_the_whole_document_didchange() {
    let tmp = safe_tempdir();
    let (mut ed, bid, log) = attached_editor(&tmp);

    // Queue an incremental change without draining.
    ed.feed_key(key('d'));

    let path = ed.state.buffers.get(bid).path().unwrap().to_path_buf();
    std::fs::write(&path, "hi\n").unwrap();
    ed.execute_typed("e!", None).unwrap();

    ed.drain_lsp();

    let changes: Vec<(Option<i64>, bool)> = log
        .borrow()
        .iter()
        .filter(|(m, _)| m == "textDocument/didChange")
        .map(|(_, p)| {
            let version = p["textDocument"]["version"].as_i64();
            let whole_document = p["contentChanges"][0].get("range").is_none();
            (version, whole_document)
        })
        .collect();

    assert_eq!(
        changes.len(),
        2,
        "expected one incremental change (the queued edit) then one whole-document \
         reload change, got: {changes:?}"
    );
    assert!(
        !changes[0].1,
        "the queued edit's incremental didChange must reach the wire first: {changes:?}"
    );
    assert!(
        changes[1].1,
        "the reload's whole-document didChange must reach the wire second: {changes:?}"
    );
    assert!(
        changes[0].0 < changes[1].0,
        "versions must strictly increase across the two didChange notifications: {changes:?}"
    );
}

/// A byte-identical `:e!` reload (the file on disk hasn't actually changed)
/// must send no `didChange` at all — `reload_from_text`'s identity branch
/// never bumps `text_gen`, so there is no new version to announce, and
/// `reload_buffer_in_place` must not fall back to sending one at the
/// buffer's unchanged version (which would be a version regression from the
/// server's point of view: a second notification carrying a version it
/// already has).
///
/// Fail oracle: un-gate `lsp_did_change_whole_document` in
/// `reload_buffer_in_place` (send it unconditionally, as before the fix) →
/// a second `didChange` appears at the same version as the last one on file.
#[test]
fn identical_reload_sends_no_didchange() {
    let tmp = safe_tempdir();
    let (mut ed, bid, log) = attached_editor(&tmp);

    let path = ed.state.buffers.get(bid).path().unwrap().to_path_buf();
    std::fs::write(&path, "hello world\n").unwrap(); // identical to attached_editor's seed content
    ed.execute_typed("e!", None).unwrap();
    ed.drain_lsp();

    let did_changes: Vec<_> = log
        .borrow()
        .iter()
        .filter(|(m, _)| m == "textDocument/didChange")
        .cloned()
        .collect();
    assert!(
        did_changes.is_empty(),
        "a byte-identical reload must send no didChange, got: {did_changes:?}"
    );
}

/// Regression: same root shape as the reload ordering test above, for `:w`
/// — an edit queued but not yet drained, immediately followed by a save in
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

/// Regression test for the scheme-LSP wrong-line-diagnostics bug: a server
/// declaring `textDocumentSync: FULL` (as `steel-language-server` does) must
/// never receive a ranged `didChange` — per spec it ignores `range` and
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
    ed.feed_key(key_esc()); // insert a char — two queued entries, one flush
    ed.drain_lsp();

    let did_changes: Vec<_> = log
        .borrow()
        .iter()
        .filter(|(m, _)| m == "textDocument/didChange")
        .cloned()
        .collect();
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
        Some(buf.text_gen as i64)
    );
}

/// A server declaring `textDocumentSync: NONE` must receive no `didChange`
/// at all — but the diagnostics store must still remap through the edit, so
/// positions from an earlier publish don't silently go stale just because
/// there was nowhere to announce the edit.
#[test]
fn none_sync_server_gets_no_didchange_but_diagnostics_still_remap() {
    let tmp = safe_tempdir();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let file = root.join("main.rs");
    std::fs::write(&file, "hello world\n").unwrap();

    let (mut backend, log, _requests) = RecordingLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": {"textDocumentSync": 0}}),
    );
    let sid = backend.start("x", &[], &root, &[]).unwrap();
    let mut client = LspClient::new(sid, root.clone());
    client.start_handshake(&mut backend);
    let (_sid, ev) = backend.drain().into_iter().next().unwrap();
    let actions = client.on_event(ev);

    // "world" is chars 6..11 of "hello world\n".
    let uri = hume_lsp::uri::path_to_uri(&file).unwrap();
    backend.push_from_server(
        sid,
        hume_lsp::codec::Message::Notification {
            method: "textDocument/publishDiagnostics".to_string(),
            params: serde_json::json!({
                "uri": uri.as_str(),
                "diagnostics": [{
                    "range": {"start": {"line": 0, "character": 6}, "end": {"line": 0, "character": 11}},
                    "severity": 1,
                    "message": "boom",
                }],
            }),
        },
    );

    let mut ed = editor_from("-[w]>ord\n");
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp.insert_client_for_test(client);
    for action in actions {
        ed.dispatch_lsp_action(sid, action);
    }

    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    ed.drain_lsp();

    let before: Vec<(usize, usize)> = ed.lsp.diagnostics_for_test(bid).collect();
    assert_eq!(
        before,
        vec![(6, 11)],
        "diagnostic must ingest at its wire position"
    );

    // Insert one char before the diagnostic — it must shift by one.
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda ()
             (apply-text-edits! (current-buffer)
               (list (list (cons 0 0) (cons 0 0) "X")))))"#,
    );
    type_cmd(&mut ed, ":go");
    ed.drain_lsp();

    assert!(
        log.borrow()
            .iter()
            .all(|(m, _)| m != "textDocument/didChange"),
        "a NONE-sync server must get no didChange, got: {:?}",
        log.borrow()
    );
    let after: Vec<(usize, usize)> = ed.lsp.diagnostics_for_test(bid).collect();
    assert_eq!(
        after,
        vec![(7, 12)],
        "the diagnostic must still remap through the edit despite no wire send"
    );
}

/// #:init-options registered through the real Steel path must reach the
/// spawned server's `initialize` request as `initializationOptions` —
/// end-to-end proof that `LspServerConfig.init_options` isn't dead weight.
#[test]
fn register_lsp_server_init_options_reach_the_initialize_request() {
    let tmp = safe_tempdir();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::write(root.join("Cargo.toml"), b"").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let file = root.join("src/main.rs");
    std::fs::write(&file, "hello world\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let (backend, _notifications, requests) = RecordingLspBackend::with_default_handshake();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[], None)
        .unwrap();

    let mut host = ScriptingHost::new();
    eval_register(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml")
                                       #:init-options (hash "check" (hash "command" "clippy")))"#,
        tmp.path(),
    );

    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    ed.drain_lsp();

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
