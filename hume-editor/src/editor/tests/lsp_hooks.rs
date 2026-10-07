// New hooks: on-lsp-attach, on-diagnostics-changed,
// on-viewport-change (debounced), on-trigger-char + set-hook-triggers!.

use std::path::Path;

use super::lsp_rig::{LspRig, RUST_ANALYZER, RigSpec};
use super::*;
use hume_lsp::backend::ServerId;
use hume_lsp::test_util::RecordingLspBackend;
use hume_scripting::ScriptingHost;

/// `src/main.rs` holding `abcdef`, its one server still `Starting`, with
/// `hooks` evaluated before the file opens.
fn starting_rig(tmp: &tempfile::TempDir, hooks: &str, backend: RecordingLspBackend) -> LspRig {
    let init = format!("{RUST_ANALYZER}\n{hooks}");
    LspRig::open(
        tmp.path(),
        RigSpec::rust("-[a]>bcdef\n").with_init(&init),
        backend,
    )
}

/// How many listeners have trigger characters on `bid`'s attachments.
fn trigger_count(ed: &Editor, bid: BufferId) -> usize {
    ed.state
        .buffer_positions
        .lsp
        .trigger_tables(bid)
        .map(crate::editor::triggers::TriggerTable::len)
        .sum()
}

fn answering_initialize() -> RecordingLspBackend {
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to("initialize", serde_json::json!({ "capabilities": {} }));
    backend
}

#[test]
fn on_lsp_attach_fires_for_buffers_attached_before_the_handshake_completes() {
    let tmp = safe_tempdir();
    let mut rig = starting_rig(
        &tmp,
        r#"(register-hook! 'on-lsp-attach (lambda (pane server) (call! "move-right" (focused-pane))))"#,
        answering_initialize(),
    );
    let before = state(&rig.ed);

    rig.ed.drain_lsp();
    rig.ed.settle();

    assert_ne!(
        state(&rig.ed),
        before,
        "on-lsp-attach must fire once the handshake completes"
    );
}

/// `on-lsp-detach`: the counterpart to `on-lsp-attach`, giving a plugin
/// its signal to drop buffer-scoped state derived from a server that just
/// detached.
#[test]
fn on_lsp_detach_fires_when_a_server_is_stopped() {
    let tmp = safe_tempdir();
    let mut rig = starting_rig(
        &tmp,
        r#"(register-hook! 'on-lsp-detach (lambda (pane server) (call! "move-right" (focused-pane))))"#,
        answering_initialize(),
    );
    rig.ed.drain_lsp();
    rig.ed.settle();
    let before = state(&rig.ed);

    rig.ed
        .apply_lsp_server_op(hume_scripting::PendingLspServerOp::Stop {
            target: hume_scripting::LspServerTarget::Buffer(rig.bid),
        });
    rig.ed.settle();

    assert_ne!(
        state(&rig.ed),
        before,
        "on-lsp-detach must fire once the server is stopped"
    );
}

#[test]
fn set_hook_triggers_from_inside_a_hook_handler_takes_effect() {
    // set-hook-triggers! must work from command context (not just
    // init/plugin-load): hover/signature-help register a server's trigger
    // characters from inside their on-lsp-attach handler, which runs as
    // plain command context. Compare against a parallel plain editor so the
    // assertion isolates "did the extra move-right additionally fire" from
    // "was '.' inserted".
    let tmp = safe_tempdir();
    let mut rig = starting_rig(
        &tmp,
        r#"(register-hook! 'on-lsp-attach (lambda (pane server)
             (set-hook-triggers! "test" (get-buffer-option pane "language") '("."))))
           (register-hook! 'on-trigger-char (lambda (pane ch source) (call! "move-right" pane)))"#,
        answering_initialize(),
    );
    rig.ed.drain_lsp();
    rig.ed.settle();
    let ed = &mut rig.ed;

    let mut plain = editor_from("-[a]>bcdef\n");
    ed.feed_key(key('i'));
    ed.settle();
    plain.feed_key(key('i'));
    ed.feed_key(key('.'));
    ed.settle();
    plain.feed_key(key('.'));
    assert_ne!(
        state(ed),
        state(&plain),
        "set-hook-triggers! called from inside a hook handler (command \
         context, not init/plugin-load) must still register the char and \
         fire the extra move-right"
    );
}

/// `set-hook-triggers!` is keyed `(source, language)`, not globally per
/// source: a second language attaching under the same source must not
/// clobber the first's chars, and a char typed in the wrong language's
/// buffer must not fire at all.
#[test]
fn set_hook_triggers_for_two_languages_under_the_same_source_do_not_clobber_each_other() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to("initialize", serde_json::json!({ "capabilities": {} }));
    backend.respond_to("initialize", serde_json::json!({ "capabilities": {} }));
    let mut rig = starting_rig(
        &tmp,
        r#"(register-lsp-server! "pylsp" #:command "pylsp")
           (set-language-servers! "python" '("pylsp"))
           (register-hook! 'on-lsp-attach (lambda (pane server)
             (let ((language (get-buffer-option pane "language")))
               (set-hook-triggers! "test" language
                 (if (equal? language "rust") '(".") '(","))))))
           (register-hook! 'on-trigger-char (lambda (pane ch source) (call! "move-right" pane)))"#,
        backend,
    );
    let bid_a = rig.bid;
    rig.ed
        .state
        .config
        .languages
        .register_identity("python", &["py"], &[], &[], None)
        .unwrap();
    let py = rig.root.join("x.py");
    std::fs::write(&py, "x\n").unwrap();
    rig.ed
        .execute_typed("e", Some(py.to_str().unwrap()))
        .unwrap();
    let bid_b = rig.ed.focused_buffer_id();
    rig.ed.drain_lsp();
    rig.ed.settle();
    let ed = &mut rig.ed;

    // Buffer A ("rust", registered "."): "," must not fire, "." must.
    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.state), bid_a);
    let mut plain_a = editor_from("-[a]>bcdef\n");
    ed.feed_key(key('i'));
    ed.settle();
    plain_a.feed_key(key('i'));
    ed.feed_key(key(','));
    ed.settle();
    plain_a.feed_key(key(','));
    assert_eq!(
        state(ed),
        state(&plain_a),
        "\",\" is unregistered for \"rust\" and must not fire"
    );
    ed.feed_key(key('.'));
    ed.settle();
    plain_a.feed_key(key('.'));
    assert_ne!(
        state(ed),
        state(&plain_a),
        "\".\" is registered for \"rust\" and must fire the extra move"
    );
    ed.feed_key(key_esc());
    ed.settle();

    // Buffer B ("python", registered ","): "." must not fire, "," must.
    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.state), bid_b);
    let mut plain_b = Editor::for_testing(Buffer::at_start(BufferText::from("x\n")));
    ed.feed_key(key('i'));
    ed.settle();
    plain_b.feed_key(key('i'));
    ed.feed_key(key('.'));
    ed.settle();
    plain_b.feed_key(key('.'));
    assert_eq!(
        state(ed),
        state(&plain_b),
        "\".\" is unregistered for \"python\" and must not fire"
    );
    ed.feed_key(key(','));
    ed.settle();
    plain_b.feed_key(key(','));
    assert_ne!(
        state(ed),
        state(&plain_b),
        "\",\" is registered for \"python\" and must fire the extra move"
    );
}

#[test]
fn on_diagnostics_changed_fires_once_per_drain_batch_not_per_publish() {
    let tmp = safe_tempdir();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let uri = hume_lsp::uri::path_to_uri(&root.join("src/main.rs")).unwrap();
    let mut backend = answering_initialize();
    // Two publishes for the same (server, uri) within one drain batch:
    // `drain_lsp` coalesces to the last one, but the hook must still fire
    // exactly once, not zero (dropped) or twice (one per publish).
    for _ in 0..2 {
        backend.push_from_server(
            ServerId(0),
            hume_lsp::codec::Message::Notification {
                method: "textDocument/publishDiagnostics".to_string(),
                params: serde_json::json!({"uri": uri.as_str(), "diagnostics": []}),
            },
        );
    }
    let mut rig = starting_rig(
        &tmp,
        r#"(register-hook! 'on-diagnostics-changed (lambda (pane) (call! "move-right" (focused-pane))))"#,
        backend,
    );

    rig.ed.drain_lsp();
    rig.ed.settle();
    assert_eq!(
        state(&rig.ed),
        "a-[b]>cdef\n",
        "on-diagnostics-changed must fire exactly once for the coalesced batch"
    );

    // A second drain (nothing new queued) must not fire again.
    rig.ed.drain_lsp();
    rig.ed.settle();
    assert_eq!(
        state(&rig.ed),
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
        r#"(register-hook! 'on-viewport-change (lambda (bid first end) (call! "move-right" bid)))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    let pane_id = ed.state.focus.id();
    // Simulate a scroll burst: each call cancels the previous pending timer
    // and reschedules; three rapid calls must still yield one fire.
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

/// Registers an `on-viewport-change` hook that bumps `tab-width` once per
/// fire, so a test reads the fire count off the buffer's override.
fn count_viewport_fires(ed: &mut Editor, dir: &Path) {
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        ed,
        &mut host,
        r#"(register-hook! 'on-viewport-change (lambda (bid first end)
             (set-buffer-option! bid "tab-width" (+ 1 (get-buffer-option bid "tab-width")))))"#,
        dir,
    );
    ed.scripting = Some(host);
}

#[test]
fn on_viewport_change_fires_when_an_edit_moves_the_clamped_end() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\nb\n");
    seed_frame(&mut ed, 40, 10);
    count_viewport_fires(&mut ed, tmp.path());

    type_text(&mut ed, "x\n");
    frame(&mut ed, 40, 10);
    ed.drain_async_sources();
    ed.settle();

    let bid = ed.focused_buffer_id();
    assert_eq!(
        ed.state.buffers.get(bid).overrides.tab_width,
        Some(EditorSettings::default().tab_width + 1),
        "a buffer shorter than the pane that gains a line changes the visible range's end, so the hook must fire once"
    );
}

#[test]
fn on_viewport_change_does_not_fire_when_a_resize_leaves_the_range_unchanged() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\nb\n");
    seed_frame(&mut ed, 40, 10);
    count_viewport_fires(&mut ed, tmp.path());

    frame(&mut ed, 40, 12);
    ed.drain_async_sources();
    ed.settle();

    let bid = ed.focused_buffer_id();
    assert_eq!(
        ed.state.buffers.get(bid).overrides.tab_width,
        None,
        "a taller pane over a buffer that already fits leaves the visible range at 0..2, so the hook must not fire"
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
        r#"(set-hook-triggers! "test" "rust" '("."))
           (register-hook! 'on-trigger-char (lambda (bid ch source)
             (when (equal? ch ".")
               (call! "move-right" bid))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    // `feed_key` (unlike `handle_input`) doesn't drain hooks itself; the
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
        r#"(set-hook-triggers! "test" "rust" '("."))
           (register-hook! 'on-trigger-char (lambda (bid ch source) (call! "move-right" bid)))"#,
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
        "no extra move must occur: on-trigger-char never fires outside Insert mode"
    );
}

/// An attachment's trigger fires in the buffer it was set for and nowhere
/// else, though another buffer is attached to the same server.
#[test]
fn set_attachment_hook_triggers_fires_for_that_buffer_only() {
    let tmp = safe_tempdir();
    let mut rig = starting_rig(
        &tmp,
        r#"(register-hook! 'on-lsp-attach (lambda (pane server)
             (when (equal? (get-buffer-option pane "language") "rust")
               (set-attachment-hook-triggers! "test" pane server 'diagnostics '(".")))))
           (register-hook! 'on-trigger-char (lambda (pane ch source) (log! 'warn source)))"#,
        answering_initialize(),
    );
    rig.ed.drain_lsp();
    rig.ed.settle();
    let notes = rig.root.join("notes.txt");
    std::fs::write(&notes, "x\n").unwrap();
    rig.ed
        .execute_typed("e", Some(notes.to_str().unwrap()))
        .unwrap();

    for _ in 0..2 {
        rig.ed.feed_key(key('i'));
        rig.ed.feed_key(key('.'));
        rig.ed.feed_key(key_esc());
        rig.ed.settle();
        let rust = rig.bid;
        rig.ed
            .switch_to_buffer_without_jump(FocusedPane::current(&rig.ed.state), rust);
    }

    assert_eq!(rig.warnings(), vec!["test".to_string()]);
}

/// Two buffers on one instance each carry their own table: setting it for
/// one leaves the other with none.
#[test]
fn attachment_triggers_are_per_buffer_not_per_instance() {
    let tmp = safe_tempdir();
    let mut rig = starting_rig(&tmp, "", answering_initialize());
    rig.ed.drain_lsp();
    rig.ed.settle();
    let lib = rig.root.join("src/lib.rs");
    std::fs::write(&lib, "// lib\n").unwrap();
    rig.ed
        .execute_typed("e", Some(lib.to_str().unwrap()))
        .unwrap();
    let lib_bid = rig.ed.focused_buffer_id();
    assert_ne!(lib_bid, rig.bid);

    rig.probe(r#"(set-attachment-hook-triggers! "test" pane (car (lsp-servers pane)) 'diagnostics '("."))"#);

    assert_eq!(trigger_count(&rig.ed, lib_bid), 1);
    assert_eq!(trigger_count(&rig.ed, rig.bid), 0);
}

/// A set for a feature the server does not advertise is refused when it
/// applies, so a plugin need not check before registering.
#[test]
fn attachment_triggers_for_an_unadvertised_feature_set_nothing() {
    let tmp = safe_tempdir();
    let mut rig = starting_rig(&tmp, "", answering_initialize());
    rig.ed.drain_lsp();
    rig.ed.settle();

    rig.probe(
        r#"(set-attachment-hook-triggers! "test" pane (car (lsp-servers pane)) 'hover '("."))"#,
    );

    assert_eq!(trigger_count(&rig.ed, rig.bid), 0);
}

/// A listener set under the buffer's language and under its attachment
/// fires once.
#[test]
fn a_listener_set_in_both_scopes_fires_once() {
    let tmp = safe_tempdir();
    let mut rig = starting_rig(&tmp, "", answering_initialize());
    rig.ed.drain_lsp();
    rig.ed.settle();

    rig.probe(r#"(set-hook-triggers! "test" "rust" '("."))"#);
    rig.probe(
        r#"(set-attachment-hook-triggers! "test" pane (car (lsp-servers pane)) 'diagnostics '("."))"#,
    );

    let hooks = rig.ed.state.triggered_by('.', rig.bid).hooks;
    assert_eq!(hooks.len(), 1);
    assert_eq!(*hooks[0], *"test");
}

/// A completion listener whose source is not registered fires nothing.
#[test]
fn an_attachment_completion_listener_naming_no_source_fires_nothing() {
    let tmp = safe_tempdir();
    let mut rig = starting_rig(&tmp, "", answering_initialize());
    rig.ed.drain_lsp();
    rig.ed.settle();
    let sid = rig.sid("rust-analyzer");

    rig.ed.state.buffer_positions.lsp.set_triggers(
        rig.bid,
        sid,
        crate::editor::triggers::Listener::Completion("ghost".into()),
        vec!['.'],
    );

    assert!(
        rig.ed
            .state
            .triggered_by('.', rig.bid)
            .completions
            .is_empty()
    );
}

#[test]
fn attachment_triggers_vanish_when_the_server_stops() {
    let tmp = safe_tempdir();
    let mut rig = starting_rig(
        &tmp,
        r#"(register-hook! 'on-lsp-attach (lambda (pane server)
             (set-attachment-hook-triggers! "test" pane server 'diagnostics '("."))))
           (register-hook! 'on-trigger-char (lambda (pane ch source) (log! 'warn source)))"#,
        answering_initialize(),
    );
    rig.ed.drain_lsp();
    rig.ed.settle();
    rig.ed
        .apply_lsp_server_op(hume_scripting::PendingLspServerOp::Stop {
            target: hume_scripting::LspServerTarget::Buffer(rig.bid),
        });

    rig.ed.feed_key(key('i'));
    rig.ed.feed_key(key('.'));
    rig.ed.settle();

    assert!(rig.warnings().is_empty(), "{:?}", rig.warnings());
}

/// A list change that excludes a feature clears the attachment's triggers
/// and fires `on-lsp-attach` again, so a handler that installs triggers
/// only for admitted features installs none.
#[test]
fn a_filter_change_clears_the_attachments_triggers_and_refires_attach() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({ "capabilities": { "hoverProvider": true } }),
    );
    let mut rig = starting_rig(
        &tmp,
        r#"(register-hook! 'on-lsp-attach (lambda (pane server)
             (log! 'warn "attach")
             (when (member server (lsp-servers pane #:feature 'hover))
               (set-attachment-hook-triggers! "test" pane server 'diagnostics '(".")))))"#,
        backend,
    );
    rig.ed.drain_lsp();
    rig.ed.settle();
    let hook_chars = |rig: &LspRig| trigger_count(&rig.ed, rig.bid);
    assert_eq!(hook_chars(&rig), 1, "setup: installed on the first attach");

    rig.eval(
        r#"(set-language-servers! "rust"
             (list (hash 'name "rust-analyzer" 'except-features '(hover))))"#,
    );
    rig.ed.settle();

    assert_eq!(hook_chars(&rig), 0);
    assert_eq!(
        rig.warnings(),
        vec!["attach".to_string(), "attach".to_string()]
    );
}

/// `:reload-config` drops the outgoing engine's attachment triggers; the new
/// engine's `on-lsp-attach` handlers install their own on resync.
#[test]
fn reload_clears_attachment_triggers() {
    let tmp = safe_tempdir();
    let mut rig = starting_rig(
        &tmp,
        r#"(register-hook! 'on-lsp-attach (lambda (pane server)
             (set-attachment-hook-triggers! "test" pane server 'diagnostics '("."))))"#,
        answering_initialize(),
    );
    rig.ed.drain_lsp();
    rig.ed.settle();

    rig.ed.reset_config_state();

    assert!(trigger_count(&rig.ed, rig.bid) == 0);
}
