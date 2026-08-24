// New hooks: on-lsp-attach, on-diagnostics-changed,
// on-viewport-change (debounced), on-trigger-char + register-trigger-chars!.

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::lsp::LspState;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

/// Wires a scripted backend attached to the focused buffer, and returns the
/// `ServerId` — handshake not yet driven (client is `Starting`).
fn wire_starting_server(ed: &mut Editor) -> ServerId {
    let mut backend = InlineLspBackend::new();
    backend.respond_to("initialize", serde_json::json!({"capabilities": {}}));
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
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
    let tmp = safe_tempdir();
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
    ed.settle();

    assert_ne!(
        state(&ed),
        before,
        "on-lsp-attach must fire with the language once the handshake completes"
    );
}

/// `on-lsp-detach` — the counterpart to `on-lsp-attach`, giving a
/// plugin its only signal to clear buffer-scoped state derived from a
/// server that `:lsp-stop`/`:lsp-restart` just tore down.
#[test]
fn on_lsp_detach_fires_with_the_language_when_a_server_is_stopped() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    ed.lsp
        .insert_client_for_test(LspClient::new(sid, PathBuf::from(".")));
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), PathBuf::from("."), sid);
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-lsp-detach (lambda (bid server-name)
             (when (equal? server-name "rust") (call! "move-right"))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    let before = state(&ed);
    ed.lsp_stop(Some("rust"));
    ed.settle();

    assert_ne!(
        state(&ed),
        before,
        "on-lsp-detach must fire with the language once the server is stopped"
    );
}

#[test]
fn register_trigger_chars_from_inside_a_hook_handler_takes_effect() {
    // register-trigger-chars! must work from command context (not just
    // init/plugin-load) — hover/signature-help register a server's trigger characters from
    // inside their on-lsp-attach handler, which runs as plain command
    // context. Oracle mirrors `on_trigger_char_fires_only_for_registered_
    // chars_in_insert_mode_after_insertion`: compare against a parallel
    // plain editor so the assertion isolates "did the extra move-right
    // additionally fire" from "was '.' inserted" (typing '.' changes state
    // either way, so a bare before/after diff on `ed` alone wouldn't catch
    // a registration that silently failed).
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let sid = wire_starting_server(&mut ed);
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.state.buffers.get_mut(bid).language = Some(lang);

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-lsp-attach (lambda (bid server-name)
             (register-trigger-chars! "test" server-name '("."))))
           (register-hook! 'on-trigger-char (lambda (bid ch source) (call! "move-right")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    complete_handshake(&mut ed, sid);
    ed.settle();

    let mut plain = editor_from("-[a]>bcdef\n");
    ed.feed_key(key('i'));
    ed.settle();
    plain.feed_key(key('i'));
    ed.feed_key(key('.'));
    ed.settle();
    plain.feed_key(key('.'));
    assert_ne!(
        state(&ed),
        state(&plain),
        "register-trigger-chars! called from inside a hook handler (command \
         context, not init/plugin-load) must still register the char and \
         fire the extra move-right"
    );
}

/// R5's fix: `register-trigger-chars!` is keyed `(source, language)`, not
/// globally per source — a second language attaching under the same source
/// must not clobber the first's chars, and a char typed in the wrong
/// language's buffer must not fire at all.
#[test]
fn register_trigger_chars_for_two_languages_under_the_same_source_do_not_clobber_each_other() {
    use hume_editing::selection::Selection;

    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let bid_a = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.state.buffers.get_mut(bid_a).language = Some(lang);

    let mut backend = InlineLspBackend::new();
    backend.respond_to("initialize", serde_json::json!({"capabilities": {}}));
    backend.respond_to("initialize", serde_json::json!({"capabilities": {}}));
    let sid_a = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
    let sid_b = backend.start("pylsp", &[], Path::new("."), &[]).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));

    let mut client_a = LspClient::new(sid_a, PathBuf::from("."));
    client_a.start_handshake(ed.lsp.backend_mut());
    ed.lsp.insert_client_for_test(client_a);
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), PathBuf::from("."), sid_a);
    ed.state.buffers.get_mut(bid_a).lsp_server = Some(sid_a);

    let mut client_b = LspClient::new(sid_b, PathBuf::from("."));
    client_b.start_handshake(ed.lsp.backend_mut());
    ed.lsp.insert_client_for_test(client_b);
    ed.lsp
        .insert_server_key_for_test("python".to_string(), PathBuf::from("."), sid_b);

    let bid_b = ed.open_buffer(Buffer::new(
        BufferText::from("x\n"),
        SelectionSet::single(Selection::collapsed(0)),
    ));
    let lang = ed.state.config.languages.intern("python");
    ed.state.buffers.get_mut(bid_b).language = Some(lang);
    ed.state.buffers.get_mut(bid_b).lsp_server = Some(sid_b);

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-lsp-attach (lambda (bid server-name)
             (register-trigger-chars! "test" server-name
               (if (equal? server-name "rust") '(".") '(",")))))
           (register-hook! 'on-trigger-char (lambda (bid ch source) (call! "move-right")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    // Both buffers are already attached (lsp_server set above) before either
    // handshake completes — the BecameRunning sweep fires on-lsp-attach for
    // both, in whichever order the backend queued their responses.
    for (sid, ev) in ed.lsp.backend_mut().drain() {
        let actions = ed.lsp.client_for_test(sid).unwrap().on_event(ev);
        for action in actions {
            ed.dispatch_lsp_action(sid, action);
        }
    }
    ed.settle();

    // Buffer A ("rust", registered "."): "," must not fire, "." must.
    // Parallel plain editor (no hook) isolates "did the extra move fire"
    // from "was the char inserted", same pattern as the single-language
    // trigger-char tests above.
    ed.switch_to_buffer_without_jump(bid_a);
    let mut plain_a = editor_from("-[a]>bcdef\n");
    ed.feed_key(key('i'));
    ed.settle();
    plain_a.feed_key(key('i'));
    ed.feed_key(key(','));
    ed.settle();
    plain_a.feed_key(key(','));
    assert_eq!(
        state(&ed),
        state(&plain_a),
        "\",\" is unregistered for \"rust\" and must not fire"
    );
    ed.feed_key(key('.'));
    ed.settle();
    plain_a.feed_key(key('.'));
    assert_ne!(
        state(&ed),
        state(&plain_a),
        "\".\" is registered for \"rust\" and must fire the extra move"
    );
    ed.feed_key(key_esc());
    ed.settle();

    // Buffer B ("python", registered ","): "." must not fire, "," must —
    // proving "python"'s attach registering under the same "test" source
    // didn't clobber "rust"'s "." entry (checked above), and that "rust"'s
    // registration doesn't leak into "python"'s buffer either.
    ed.switch_to_buffer_without_jump(bid_b);
    let mut plain_b = Editor::for_testing(Buffer::new(
        BufferText::from("x\n"),
        SelectionSet::single(Selection::collapsed(0)),
    ));
    ed.feed_key(key('i'));
    ed.settle();
    plain_b.feed_key(key('i'));
    ed.feed_key(key('.'));
    ed.settle();
    plain_b.feed_key(key('.'));
    assert_eq!(
        state(&ed),
        state(&plain_b),
        "\".\" is unregistered for \"python\" and must not fire"
    );
    ed.feed_key(key(','));
    ed.settle();
    plain_b.feed_key(key(','));
    assert_ne!(
        state(&ed),
        state(&plain_b),
        "\",\" is registered for \"python\" and must fire the extra move"
    );
}

#[test]
fn on_diagnostics_changed_fires_once_per_drain_batch_not_per_publish() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "abcdef\n").unwrap();
    let canonical = std::fs::canonicalize(&file).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&canonical).unwrap();

    let mut backend = InlineLspBackend::new();
    let sid = backend
        .start("rust-analyzer", &[], Path::new("."), &[])
        .unwrap();
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
    ed.settle();
    // Exactly one fire (one move-right), not zero (dropped) or two (one per
    // publish) — the two coalesced publishes must yield one hook call.
    assert_eq!(
        state(&ed),
        "a-[b]>cdef\n",
        "on-diagnostics-changed must fire exactly once for the coalesced batch"
    );

    // A second drain (nothing new queued) must not fire again.
    ed.drain_lsp();
    ed.settle();
    assert_eq!(
        state(&ed),
        "a-[b]>cdef\n",
        "a drain with no new publishDiagnostics must not fire the hook again"
    );
}

#[test]
fn on_viewport_change_debounces_a_scroll_burst_into_one_fire() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    ed.state.settings.lsp_viewport_debounce_ms = 0;
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-hook! 'on-viewport-change (lambda (bid first end) (call! "move-right")))"#,
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
    ed.settle();

    assert_eq!(
        state(&ed),
        "a-[b]>cdef\n",
        "a burst of 3 debounce calls must collapse to exactly one on-viewport-change fire"
    );
}

#[test]
fn on_trigger_char_fires_only_for_registered_chars_in_insert_mode_after_insertion() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.state.buffers.get_mut(bid).language = Some(lang);
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-trigger-chars! "test" "rust" '("."))
           (register-hook! 'on-trigger-char (lambda (bid ch source)
             (when (equal? ch ".")
               (call! "move-right"))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    // `feed_key` (unlike `handle_input`) doesn't drain hooks itself — the
    // interactive loop does that separately so tests without a scripting
    // host don't need one. Drain explicitly after each key that could have
    // enqueued one. Compare against a parallel plain editor with no hook to
    // isolate "did move-right additionally fire" from "was the char inserted".
    let mut plain = editor_from("-[a]>bcdef\n");

    ed.feed_key(key('i'));
    ed.settle();
    plain.feed_key(key('i'));
    assert_eq!(
        state(&ed),
        state(&plain),
        "entering Insert mode alone must not fire anything"
    );

    ed.feed_key(key('x'));
    ed.settle();
    plain.feed_key(key('x'));
    assert_eq!(
        state(&ed),
        state(&plain),
        "an unregistered char must not fire on-trigger-char"
    );

    ed.feed_key(key('.'));
    ed.settle();
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
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[.]>bcdef\n");
    let bid = ed.focused_buffer_id();
    let lang = ed.state.config.languages.intern("rust");
    ed.state.buffers.get_mut(bid).language = Some(lang);
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(register-trigger-chars! "test" "rust" '("."))
           (register-hook! 'on-trigger-char (lambda (bid ch source) (call! "move-right")))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    // Normal mode: 'l' moves right by one grapheme via the keymap, not
    // through handle_insert at all, so on-trigger-char can't fire.
    let before = state(&ed);
    ed.feed_key(key('l'));
    let after_l = state(&ed);
    assert_ne!(
        before, after_l,
        "the motion itself must still move the cursor"
    );

    let mut plain = editor_from("-[.]>bcdef\n");
    plain.feed_key(key('l'));
    assert_eq!(
        state(&ed),
        state(&plain),
        "no extra move must occur — on-trigger-char never fires outside Insert mode"
    );
}
