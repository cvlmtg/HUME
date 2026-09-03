// Observability + lifecycle commands:
// :lsp-status, :lsp-stop, :lsp-restart, and stderr/log-message routing.

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::lsp::LspState;
use crate::editor::scripting_setup::make_init_host;
use hume_lsp::backend::LspBackend;
use hume_lsp::client::{ClientAction, LspClient, ServerState};
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

fn eval_register(ed: &mut Editor, host: &mut ScriptingHost, source: &str, tmp: &Path) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    let effects = {
        let mut ih = make_init_host(
            &mut ed.state,
            &mut ed.view,
            ed.terminal.as_ref(),
            ed.tui_active,
            ed.kitty_enabled,
        );
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init");
    ed.apply_script_effects(effects);
}

#[test]
fn status_text_lists_a_running_server_with_root_and_pending_count() {
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));

    let root = PathBuf::from("/tmp/hume-lsp-status-test");
    let mut client = LspClient::new(sid, root.clone());
    client.set_state_for_test(ServerState::Running);
    ed.lsp.insert_client_for_test(client);
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), root.clone(), sid);
    ed.lsp
        .insert_server_name_for_test(sid, "rust-analyzer".to_string());

    let text = ed.lsp_status_text();
    assert!(text.contains("rust @"), "must show the language: {text:?}");
    assert!(
        text.contains(&root.display().to_string()),
        "must show the root: {text:?}"
    );
    assert!(
        text.contains("Running"),
        "must show the lifecycle state: {text:?}"
    );
    assert!(
        text.contains("0 in flight"),
        "must show the pending count: {text:?}"
    );
}

#[test]
fn status_text_reports_no_servers_when_none_are_registered() {
    let ed = editor_from("-[w]>ord\n");
    assert_eq!(ed.lsp_status_text(), "No LSP servers registered.");
}

#[test]
fn lsp_stop_deregisters_the_server_and_clears_buffer_attachment() {
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let root = PathBuf::from("/tmp/hume-lsp-stop-test");
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, root.clone()));
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), root, sid);

    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    let n = ed.lsp_stop(Some("rust"));

    assert_eq!(n, 1);
    assert!(
        ed.lsp.client_for_test(sid).is_none(),
        "the client must be deregistered"
    );
    assert_eq!(
        ed.lsp.server_count_for_test(),
        0,
        "the (language, root) key must be removed"
    );
    assert_eq!(
        ed.state.buffers.get(bid).lsp_server,
        None,
        "the buffer must be detached so it can re-attach later"
    );
}

#[test]
fn lsp_stop_with_no_matching_server_stops_nothing() {
    let mut ed = editor_from("-[w]>ord\n");
    assert_eq!(ed.lsp_stop(Some("nonexistent-language")), 0);
    assert_eq!(ed.lsp_stop(None), 0);
}

/// A queued didChange entry left over from before the stop must
/// not survive to be flushed against a future server's didOpen baseline —
/// it would desync that server's document state on the very first edit.
#[test]
fn lsp_stop_clears_the_buffer_s_pending_change_queue() {
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let root = PathBuf::from("/tmp/hume-lsp-stop-pending-test");
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, root.clone()));
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), root, sid);

    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    let mut b = hume_editing::changeset::ChangeSetBuilder::new(4);
    b.retain(0).insert("X").retain_rest();
    ed.state
        .buffers
        .get_mut(bid)
        .lsp_pending
        .push(crate::editor::lsp::sync::LspPendingChange {
            cs: b.finish(),
            before: ropey::Rope::from_str("word"),
            version: 1,
        });
    assert!(!ed.state.buffers.get(bid).lsp_pending.is_empty());

    ed.lsp_stop(Some("rust"));

    assert!(
        ed.state.buffers.get(bid).lsp_pending.is_empty(),
        "a stopped buffer's pending queue must be cleared, not carried into a future attach"
    );
}

#[test]
fn lsp_restart_spawns_a_fresh_server_id_and_reattaches_the_buffer() {
    let tmp = safe_tempdir();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::write(root.join("Cargo.toml"), b"").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let file = root.join("src/main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    ed.lsp = LspState::new_inline();
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
    let bid = ed.focused_buffer_id();
    let old_sid = ed
        .state
        .buffers
        .get(bid)
        .lsp_server
        .expect("attached on open");

    let n = ed.lsp_restart(Some("rust"));
    assert_eq!(n, 1);

    let new_sid = ed
        .state
        .buffers
        .get(bid)
        .lsp_server
        .expect("re-attached after restart");
    assert_ne!(
        old_sid, new_sid,
        "restart must yield a fresh ServerId, not reuse the old one"
    );
    assert_eq!(
        ed.lsp.server_count_for_test(),
        1,
        "the old server entry must be gone, exactly one fresh one inserted"
    );
}

/// Regression: without `DiagnosticsStore::remove_server`, a restarted
/// server's fresh `ServerId` would coexist with the old (frozen, detached)
/// server's entry for the same buffer — `replace`'s "push if no matching
/// sid" path — doubling the count instead of replacing it.
#[test]
fn lsp_restart_does_not_duplicate_diagnostics_after_a_republish() {
    let tmp = safe_tempdir();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::write(root.join("Cargo.toml"), b"").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let file = root.join("src/main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    ed.lsp = LspState::new_inline();
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
    let bid = ed.focused_buffer_id();
    let old_sid = ed
        .state
        .buffers
        .get(bid)
        .lsp_server
        .expect("attached on open");

    let uri = hume_lsp::uri::path_to_uri(&std::fs::canonicalize(&file).unwrap()).unwrap();
    let current_gen = ed.state.buffers.get(bid).text_gen as i32;
    let mut params = serde_json::json!({
        "uri": uri.as_str(),
        "version": current_gen,
        "diagnostics": [{
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 2}},
            "severity": 1,
            "message": "boom",
        }],
    });
    ed.ingest_publish_diagnostics(old_sid, serde_json::from_value(params.clone()).unwrap());
    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (1, 0),
        "seed publish from the original server must land"
    );

    ed.lsp_restart(Some("rust"));
    let new_sid = ed
        .state
        .buffers
        .get(bid)
        .lsp_server
        .expect("re-attached after restart");
    assert_ne!(old_sid, new_sid, "restart must yield a fresh ServerId");

    // The fresh server republishes the same diagnostic — this must replace
    // the old, now-detached server's entry, not stack alongside it.
    params["version"] = serde_json::json!(ed.state.buffers.get(bid).text_gen as i32);
    ed.ingest_publish_diagnostics(new_sid, serde_json::from_value(params).unwrap());

    assert_eq!(
        ed.lsp.diagnostic_counts_for_test(bid),
        (1, 0),
        "the old server's diagnostics must not coexist with the new server's republish"
    );
}

#[test]
fn stderr_action_is_logged_at_trace_with_the_server_name_prefix() {
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, PathBuf::from(".")));
    ed.lsp
        .insert_server_name_for_test(sid, "rust-analyzer".to_string());

    ed.dispatch_lsp_action(sid, ClientAction::Stderr("panic: oh no".to_string()));

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("[trace] rust-analyzer: panic: oh no"),
        "expected a Trace-prefixed stderr line, got: {log:?}"
    );
}

#[test]
fn log_message_error_type_is_reported_at_error_severity() {
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, PathBuf::from(".")));
    ed.lsp
        .insert_server_name_for_test(sid, "rust-analyzer".to_string());

    ed.dispatch_lsp_action(
        sid,
        ClientAction::LogMessage(
            serde_json::from_value(serde_json::json!({"type": 1, "message": "something broke"}))
                .unwrap(),
        ),
    );

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("[error] rust-analyzer: something broke"),
        "type=1 (Error) must be reported at Error severity, got: {log:?}"
    );
}

#[test]
fn log_message_info_type_is_reported_at_trace_not_shown_as_status() {
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, PathBuf::from(".")));
    ed.lsp
        .insert_server_name_for_test(sid, "rust-analyzer".to_string());

    ed.dispatch_lsp_action(
        sid,
        ClientAction::LogMessage(
            serde_json::from_value(serde_json::json!({"type": 3, "message": "indexing"})).unwrap(),
        ),
    );

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("[trace] rust-analyzer: indexing"),
        "type=3 (Info) must be demoted to Trace, not shown, got: {log:?}"
    );
}

#[test]
fn show_message_is_reported_at_info_severity() {
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, PathBuf::from(".")));
    ed.lsp
        .insert_server_name_for_test(sid, "rust-analyzer".to_string());

    ed.dispatch_lsp_action(
        sid,
        ClientAction::ShowMessage(
            serde_json::from_value(serde_json::json!({"type": 3, "message": "ready"})).unwrap(),
        ),
    );

    // Info severity is never pushed to the persistent log (see
    // Editor::report) — only shown as the transient status message.
    assert_eq!(ed.state.status_msg.as_deref(), Some("rust-analyzer: ready"));
}

#[test]
fn progress_report_events_are_dropped_without_any_log_line() {
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, PathBuf::from(".")));
    ed.lsp
        .insert_server_name_for_test(sid, "rust-analyzer".to_string());

    ed.dispatch_lsp_action(
        sid,
        ClientAction::Progress(
            serde_json::from_value(
                serde_json::json!({"token": "1", "value": {"kind": "begin", "title": "indexing"}}),
            )
            .unwrap(),
        ),
    );
    ed.dispatch_lsp_action(
        sid,
        ClientAction::Progress(
            serde_json::from_value(
                serde_json::json!({"token": "1", "value": {"kind": "report", "percentage": 50}}),
            )
            .unwrap(),
        ),
    );
    ed.dispatch_lsp_action(
        sid,
        ClientAction::Progress(
            serde_json::from_value(serde_json::json!({"token": "1", "value": {"kind": "end"}}))
                .unwrap(),
        ),
    );

    let entries: Vec<_> = ed.state.message_log.entries().collect();
    assert_eq!(
        entries.len(),
        2,
        "begin and end must log once each; report must never log — got: {entries:?}"
    );
}

// ── Fix 3 — graceful shutdown on quit ──────────────────────────────────────

#[test]
fn lsp_shutdown_all_transitions_every_running_client_to_dead() {
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, PathBuf::from("/tmp/hume-shutdown-test"));
    client.set_state_for_test(ServerState::Running);
    ed.lsp.insert_client_for_test(client);

    // Duration::ZERO means the grace-window loop's `Instant::now() < deadline`
    // is false on first check — no sleep, no waiting for a real process.
    ed.lsp_shutdown_all(std::time::Duration::ZERO);

    let client = ed
        .lsp
        .client_for_test(sid)
        .expect("the client stays tracked (only its state changes) — lsp_stop is what deregisters");
    assert_eq!(client.state(), ServerState::Dead);
}

#[test]
fn lsp_shutdown_all_on_a_starting_client_skips_the_protocol_but_still_tears_down() {
    // A client that never completed its handshake must not receive
    // shutdown/exit (nothing but `initialize` is legal before `initialized`)
    // — but it must still not be left dangling forever; the transport-level
    // `backend.shutdown` call covers it regardless of protocol state.
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, PathBuf::from(".")));

    ed.lsp_shutdown_all(std::time::Duration::ZERO);

    let client = ed
        .lsp
        .client_for_test(sid)
        .expect("still tracked, but untouched by begin_shutdown");
    assert_eq!(
        client.state(),
        ServerState::Starting,
        "a Starting client's state must not change — it never got the shutdown/exit messages"
    );
}

#[test]
fn lsp_shutdown_all_with_no_clients_returns_immediately() {
    let mut ed = editor_from("-[w]>ord\n");
    let start = std::time::Instant::now();
    ed.lsp_shutdown_all(std::time::Duration::from_secs(5));
    assert!(
        start.elapsed() < std::time::Duration::from_millis(50),
        "no clients means nothing to wait for — must not block on the grace window"
    );
}
