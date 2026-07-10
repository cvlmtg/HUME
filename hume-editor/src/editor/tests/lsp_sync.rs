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
use hume_lsp::sync::apply_events_to_string_mirror;
use hume_lsp::test_util::{NotificationLog, RecordingLspBackend};
use hume_scripting::ScriptingHost;

fn eval_register(ed: &mut Editor, host: &mut ScriptingHost, source: &str, tmp: &Path) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init");
    ed.flush_pending_lsp_server_ops(host);
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
        .languages
        .register_identity("rust", &["rs"], &[], &[])
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
                            hume_editing::PositionEncoding::Utf16,
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
    let tmp = tempfile::tempdir().unwrap();
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

/// Fix 1: `lsp_did_open` must queue behind the handshake, never write to
/// the wire before `initialize` completes — the spec forbids anything else
/// arriving first. Before the drain that carries the initialize response,
/// nothing has been sent at all; after it, the log is exactly
/// `initialized` then `didOpen`, in that order.
#[test]
fn did_open_is_queued_until_the_handshake_completes_then_flushes_in_order() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::write(root.join("Cargo.toml"), b"").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let file = root.join("src/main.rs");
    std::fs::write(&file, "hello world\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    let (backend, log, _requests) = RecordingLspBackend::with_default_handshake();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.state
        .languages
        .register_identity("rust", &["rs"], &[], &[])
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
    let tmp = tempfile::tempdir().unwrap();
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
    let tmp = tempfile::tempdir().unwrap();
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
