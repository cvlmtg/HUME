// B7 (docs/lsp/step-2.md) — new hooks: on-lsp-attach, on-diagnostics-changed,
// on-viewport-change (debounced), on-trigger-char + register-trigger-chars!.

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::lsp::LspState;
use crate::editor::scripting_setup::make_init_host;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

fn eval_with_real_host(ed: &mut Editor, host: &mut ScriptingHost, source: &str, tmp: &Path) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    let mut ih = make_init_host(&mut ed.state, &mut ed.view);
    host.eval_init(&init_path, 10_000, &mut ih, Default::default())
        .expect("eval_init");
}

/// Wires a scripted backend attached to the focused buffer, and returns the
/// `ServerId` — handshake not yet driven (client is `Starting`).
fn wire_starting_server(ed: &mut Editor) -> ServerId {
    let mut backend = InlineLspBackend::new();
    backend.respond_to("initialize", serde_json::json!({"capabilities": {}}));
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.start_handshake(ed.lsp.backend_mut());
    ed.lsp.insert_client_for_test(client);
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), PathBuf::from("."), sid);
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);
    sid
}

/// Drives the queued `initialize` response through to `BecameRunning`.
fn complete_handshake(ed: &mut Editor, sid: ServerId) {
    let (sid2, ev) = ed.lsp.backend_mut().drain().into_iter().next().unwrap();
    let actions = ed.lsp.client_for_test(sid2).unwrap().on_event(ev);
    for action in actions {
        ed.dispatch_lsp_action(sid2, action);
    }
    assert_eq!(sid2, sid);
}

#[test]
fn on_lsp_attach_fires_for_buffers_attached_before_the_handshake_completes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let sid = wire_starting_server(&mut ed);

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-lsp-attach (lambda (bid server-name)
             (when (equal? server-name "rust") (call! "move-right"))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    let before = state(&ed);
    complete_handshake(&mut ed, sid);
    ed.drain_hooks();

    assert_ne!(
        state(&ed),
        before,
        "on-lsp-attach must fire with the language once the handshake completes"
    );
}

#[test]
fn register_trigger_chars_from_inside_a_hook_handler_takes_effect() {
    // B10a: register-trigger-chars! must work from command context (not just
    // init/plugin-load) — F3/F7 register a server's trigger characters from
    // inside their on-lsp-attach handler, which runs as plain command
    // context. Oracle mirrors `on_trigger_char_fires_only_for_registered_
    // chars_in_insert_mode_after_insertion`: compare against a parallel
    // plain editor so the assertion isolates "did the extra move-right
    // additionally fire" from "was '.' inserted" (typing '.' changes state
    // either way, so a bare before/after diff on `ed` alone wouldn't catch
    // a registration that silently failed).
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let sid = wire_starting_server(&mut ed);

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-lsp-attach (lambda (bid server-name)
             (register-trigger-chars! "test" '("."))))
           (register-hook! 'on-trigger-char (lambda (bid ch) (call! "move-right")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    complete_handshake(&mut ed, sid);
    ed.drain_hooks();

    let mut plain = editor_from("-[a]>bcdef\n");
    ed.feed_key(key('i'));
    ed.drain_hooks();
    plain.feed_key(key('i'));
    ed.feed_key(key('.'));
    ed.drain_hooks();
    plain.feed_key(key('.'));
    assert_ne!(
        state(&ed),
        state(&plain),
        "register-trigger-chars! called from inside a hook handler (command \
         context, not init/plugin-load) must still register the char and \
         fire the extra move-right"
    );
}

#[test]
fn on_diagnostics_changed_fires_once_per_drain_batch_not_per_publish() {
    let tmp = tempfile::tempdir().unwrap();
    let file_dir = tempfile::tempdir().unwrap();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "abcdef\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut backend = InlineLspBackend::new();
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
    // Two publishes for the same (server, uri) within one drain batch —
    // `drain_lsp` coalesces to the last one, but the hook must still fire
    // exactly once, not zero (dropped) or twice (one per publish).
    for _ in 0..2 {
        backend.push_from_server(
            sid,
            hume_lsp::codec::Message::Notification {
                method: "textDocument/publishDiagnostics".to_string(),
                params: serde_json::json!({"uri": uri.as_str(), "diagnostics": []}),
            },
        );
    }
    let mut ed = editor_from("-[x]>\n");
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, PathBuf::from(".")));
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-diagnostics-changed (lambda (bid) (call! "move-right")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    ed.drain_lsp();
    ed.drain_hooks();
    // Exactly one fire (one move-right), not zero (dropped) or two (one per
    // publish) — the two coalesced publishes must yield one hook call.
    assert_eq!(
        state(&ed),
        "a-[b]>cdef\n",
        "on-diagnostics-changed must fire exactly once for the coalesced batch"
    );

    // A second drain (nothing new queued) must not fire again.
    ed.drain_lsp();
    ed.drain_hooks();
    assert_eq!(
        state(&ed),
        "a-[b]>cdef\n",
        "a drain with no new publishDiagnostics must not fire the hook again"
    );
}

#[test]
fn on_viewport_change_debounces_a_scroll_burst_into_one_fire() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    ed.state.settings.lsp_viewport_debounce_ms = 0;
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-viewport-change (lambda (bid first last) (call! "move-right")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    let pane_id = ed.state.focused_pane_id;
    // Simulate a scroll burst: each call cancels the previous pending timer
    // and reschedules — three rapid calls must still yield one fire.
    ed.debounce_viewport_change(pane_id);
    ed.debounce_viewport_change(pane_id);
    ed.debounce_viewport_change(pane_id);

    ed.drain_async_sources();
    ed.drain_hooks();

    assert_eq!(
        state(&ed),
        "a-[b]>cdef\n",
        "a burst of 3 debounce calls must collapse to exactly one on-viewport-change fire"
    );
}

#[test]
fn on_trigger_char_fires_only_for_registered_chars_in_insert_mode_after_insertion() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-trigger-chars! "test" '("."))
           (register-hook! 'on-trigger-char (lambda (bid ch)
             (when (equal? ch ".")
               (call! "move-right"))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    // `feed_key` (unlike `handle_event`) doesn't drain hooks itself — the
    // interactive loop does that separately so tests without a scripting
    // host don't need one. Drain explicitly after each key that could have
    // enqueued one. Compare against a parallel plain editor with no hook to
    // isolate "did move-right additionally fire" from "was the char inserted".
    let mut plain = editor_from("-[a]>bcdef\n");

    ed.feed_key(key('i'));
    ed.drain_hooks();
    plain.feed_key(key('i'));
    assert_eq!(state(&ed), state(&plain), "entering Insert mode alone must not fire anything");

    ed.feed_key(key('x'));
    ed.drain_hooks();
    plain.feed_key(key('x'));
    assert_eq!(
        state(&ed),
        state(&plain),
        "an unregistered char must not fire on-trigger-char"
    );

    ed.feed_key(key('.'));
    ed.drain_hooks();
    plain.feed_key(key('.'));
    assert_ne!(
        state(&ed),
        state(&plain),
        "the registered '.' must fire on-trigger-char after it was inserted, moving the \
         cursor one extra step past what plain typing alone would produce"
    );
}

#[test]
fn on_trigger_char_does_not_fire_in_normal_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[.]>bcdef\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-trigger-chars! "test" '("."))
           (register-hook! 'on-trigger-char (lambda (bid ch) (call! "move-right")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    // Normal mode: 'l' moves right by one grapheme via the keymap, not
    // through handle_insert at all, so on-trigger-char can't fire.
    let before = state(&ed);
    ed.feed_key(key('l'));
    let after_l = state(&ed);
    assert_ne!(before, after_l, "the motion itself must still move the cursor");

    let mut plain = editor_from("-[.]>bcdef\n");
    plain.feed_key(key('l'));
    assert_eq!(
        state(&ed),
        state(&plain),
        "no extra move must occur — on-trigger-char never fires outside Insert mode"
    );
}
