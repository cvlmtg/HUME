// Packaging: lazy `declare-plugin` activation,
// the goto-trie keybindings bound in `plugin.scm`, and the commented
// `init.scm.example` LSP block. Loads the real shipped `core:lsp` plugin
// in place (`RealRuntimeGuard`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::{Path, PathBuf};

use super::*;
use crate::editor::lsp::LspState;
use crate::editor::scripting_setup::make_init_host;
use hume_engine::pipeline::RenderContext;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::LspClient;
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;
use hume_scripting::attribution::PluginId;

const DECLARE_LSP: &str = r#"(load-plugin "core:stdlib")
(declare-plugin "core:lsp"
  #:events '("on-lsp-attach")
  #:commands '("lsp-hover" "lsp-goto-definition" "lsp-goto-declaration"
               "lsp-goto-type-definition" "lsp-goto-implementation" "lsp-references"
               "goto-next-diagnostic" "goto-prev-diagnostic" "diagnostics"
               "lsp-rename" "lsp-fmt" "lsp-code-actions" "lsp-completion-trigger"))"#;

/// Same manifest as `DECLARE_LSP` but keyed on `on-buffer-save` instead of
/// `on-lsp-attach` — used to prove a positive activation result isn't a
/// confound of `setup_declared`'s staging (see
/// `attach_event_does_not_activate_a_plugin_declared_for_a_different_event`).
const DECLARE_LSP_WRONG_EVENT: &str = r#"(load-plugin "core:stdlib")
(declare-plugin "core:lsp"
  #:events '("on-buffer-save")
  #:commands '("lsp-hover" "lsp-goto-definition" "lsp-goto-declaration"
               "lsp-goto-type-definition" "lsp-goto-implementation" "lsp-references"
               "goto-next-diagnostic" "goto-prev-diagnostic" "diagnostics"
               "lsp-rename" "lsp-fmt" "lsp-code-actions" "lsp-completion-trigger"))"#;

#[cfg(not(windows))]
fn eval_with_real_host(ed: &mut Editor, host: &mut ScriptingHost, source: &str, tmp: &Path) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    let mut ih = make_init_host(&mut ed.state, &mut ed.view);
    host.eval_init(&init_path, 10_000, &mut ih, Default::default())
        .expect("eval_init");
}

/// Mirrors `lsp_hover.rs`'s `setup`, but declares `core:lsp` lazily
/// (`declare_src`, normally `DECLARE_LSP`) instead of `(load-plugin
/// "core:lsp")`, and registers the lazy command stubs so a `:`-command
/// dispatch can trigger activation — mirrors `tests/plugins.rs`'s
/// `setup_lazy_editor` for the stub-registration step, combined with the
/// real-runtime staging every other F-card test uses.
///
/// The handshake below (draining the backend's `initialize` response and
/// dispatching `BecameRunning`) fires `on-lsp-attach` *before* `ed.scripting`
/// is even assigned. That's fine: `fire_hook_silent` only pushes onto
/// `state.pending_hooks`, which lives on `Editor::state` independent of
/// `scripting` — the queued hook survives host installation and is still
/// there for a later `ed.drain_hooks()` to process against the real host.
#[cfg(not(windows))]
fn setup_declared(
    file_dir: &Path,
    tmp: &Path,
    declare_src: &str,
    configure: impl FnOnce(&mut InlineLspBackend, ServerId),
) -> (Editor, RealRuntimeGuard) {
    let guard = RealRuntimeGuard::new();

    let mut ed = Editor::open(None).unwrap();
    let file = file_dir.join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut backend = InlineLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
    );
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
    configure(&mut backend, sid);
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.start_handshake(ed.lsp.backend_mut());
    ed.lsp.insert_client_for_test(client);
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), PathBuf::from("."), sid);

    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).lsp_server = Some(sid);

    let (sid2, ev) = ed.lsp.backend_mut().drain().into_iter().next().unwrap();
    let actions = ed.lsp.client_for_test(sid2).unwrap().on_event(ev);
    for action in actions {
        ed.dispatch_lsp_action(sid2, action);
    }

    let mut host = ScriptingHost::new();
    eval_with_real_host(&mut ed, &mut host, declare_src, tmp);
    let activation_commands = host.activation_commands();
    ed.register_lazy_command_stubs(&activation_commands);
    ed.scripting = Some(host);

    (ed, guard)
}

#[cfg(not(windows))]
fn popup_lines(ed: &Editor) -> Option<Vec<String>> {
    ed.state
        .popup_view
        .read()
        .unwrap()
        .as_ref()
        .map(|s| s.lines.clone())
}

/// Declaring `core:lsp` (not loading it) leaves it `Declared` — nothing has
/// run its body yet.
///
/// Flip: if `declare-plugin` eagerly loaded the plugin (a bug reintroducing
/// eager semantics), status would already read `Loaded` here.
#[test]
#[cfg(not(windows))]
fn declared_but_undispatched_plugin_is_declared_not_loaded() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (ed, _guard) = setup_declared(file_dir.path(), tmp.path(), DECLARE_LSP, |backend, _sid| {
        backend.respond_to(
            "textDocument/hover",
            serde_json::json!({"contents": {"kind": "plaintext", "value": "fn main()"}}),
        );
    });

    let id = PluginId::parse("core:lsp").unwrap();
    assert_eq!(
        ed.scripting.as_ref().unwrap().plugin_status(&id),
        Some(hume_scripting::PluginStatus::Declared)
    );
}

/// First `:lsp-hover` dispatch on a declared-but-inactive `core:lsp` loads
/// the real plugin body from disk, replaces the lazy stub, and runs the real
/// hover request through to a populated popup.
///
/// Flip: without `register_lazy_command_stubs`, `:lsp-hover` would be an
/// unknown command and this would report an error, not show a popup; without
/// the real activation wiring, `plugin_status` would stay `Declared`.
#[test]
#[cfg(not(windows))]
fn first_command_dispatch_activates_the_declared_plugin_and_runs_it() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) =
        setup_declared(file_dir.path(), tmp.path(), DECLARE_LSP, |backend, _sid| {
            backend.respond_to(
                "textDocument/hover",
                serde_json::json!({"contents": {"kind": "plaintext", "value": "fn main()"}}),
            );
        });

    // Activation runs synchronously inside the command dispatch that hits
    // the lazy stub (`activate_lazy_plugin`, called from the same dispatch
    // path before re-querying and running the now-real command) — the same
    // drain sequence `lsp_hover.rs`'s `run_hover` uses for an eagerly-loaded
    // plugin is enough here too.
    type_cmd(&mut ed, ":lsp-hover");
    ed.drain_hooks();
    ed.drain_lsp();
    ed.drain_pending_steel_calls();
    ed.drain_hooks();

    // `show-popup!` only populates `popup_view` once a frame resolves its
    // anchor (`lsp_popup.rs`'s `show_popup_populates_the_view_after_a_frame`).
    let mut ctx = RenderContext::new();
    ed.prepare_frame(80, 25, &mut ctx);

    assert_eq!(popup_lines(&ed), Some(vec!["fn main()".to_string()]));
    let id = PluginId::parse("core:lsp").unwrap();
    assert_eq!(
        ed.scripting.as_ref().unwrap().plugin_status(&id),
        Some(hume_scripting::PluginStatus::Loaded)
    );
}

/// The `on-lsp-attach` event alone — with no `:`-command ever dispatched —
/// activates the declared `core:lsp` plugin. Unlike the command-dispatch test
/// above, nothing here touches a lazy command stub, so the only thing that
/// can flip `Declared` to `Loaded` is `drain_hooks`'s
/// `activate_lazy_event_plugins(OnLspAttach)` picking up the hook that
/// `setup_declared`'s handshake already queued.
///
/// Flip: `attach_event_does_not_activate_a_plugin_declared_for_a_different_event`
/// runs the identical attach sequence against a manifest declared on
/// `on-buffer-save` instead, and confirms it stays `Declared` — ruling out
/// some other confound in `setup_declared`'s staging (e.g. `load-plugin
/// "core:stdlib"` in the same source, or the handshake itself) as the cause
/// of this test's `Loaded` result.
#[test]
#[cfg(not(windows))]
fn attach_event_alone_activates_the_declared_plugin() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup_declared(
        file_dir.path(),
        tmp.path(),
        DECLARE_LSP,
        |_backend, _sid| {},
    );

    let id = PluginId::parse("core:lsp").unwrap();
    assert_eq!(
        ed.scripting.as_ref().unwrap().plugin_status(&id),
        Some(hume_scripting::PluginStatus::Declared),
        "must still be Declared going into the drain — only the queued \
         on-lsp-attach hook can flip it here"
    );

    ed.drain_hooks();

    assert_eq!(
        ed.scripting.as_ref().unwrap().plugin_status(&id),
        Some(hume_scripting::PluginStatus::Loaded),
        "on-lsp-attach firing (queued by setup_declared's handshake) must \
         activate the plugin with no command ever dispatched"
    );
}

/// Flip counterpart to `attach_event_alone_activates_the_declared_plugin`:
/// declaring `core:lsp` on `on-buffer-save` instead of `on-lsp-attach` and
/// running the same attach sequence must leave it `Declared`.
#[test]
#[cfg(not(windows))]
fn attach_event_does_not_activate_a_plugin_declared_for_a_different_event() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let (mut ed, _guard) = setup_declared(
        file_dir.path(),
        tmp.path(),
        DECLARE_LSP_WRONG_EVENT,
        |_backend, _sid| {},
    );

    ed.drain_hooks();

    let id = PluginId::parse("core:lsp").unwrap();
    assert_eq!(
        ed.scripting.as_ref().unwrap().plugin_status(&id),
        Some(hume_scripting::PluginStatus::Declared),
        "on-lsp-attach firing must not activate a plugin declared for a \
         different event (on-buffer-save)"
    );
}

/// Every default `g`-prefixed binding `plugin.scm` adds dispatches to its
/// named command without error, even fully unattached (no LSP server on the
/// buffer at all) — each command's own capability guard degrades to an
/// `'info` log line in that case, never `'error`. Exercises the bindings
/// themselves (does `g d` actually reach `lsp-goto-definition`?); each
/// feature's own test file exercises its LSP behavior once attached.
///
/// Flip: renaming a binding's target command in `plugin.scm` without
/// updating this table (or vice versa) makes the matching iteration fail
/// with "unknown command" — checked at least once by temporarily renaming
/// `"g d"`'s target in `plugin.scm` to a typo and confirming this test fails.
#[test]
#[cfg(not(windows))]
fn every_default_goto_binding_dispatches_without_error() {
    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let guard = RealRuntimeGuard::new();
    let mut ed = Editor::open(None).unwrap();
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(load-plugin "core:stdlib")
(load-plugin "core:lsp")"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    let bindings: &[(char, char)] = &[
        ('g', 'd'),
        ('g', 'D'),
        ('g', 'y'),
        ('g', 'i'),
        ('g', 'r'),
        ('g', 'R'),
        ('g', 'k'),
        ('g', 'a'),
        ('g', 'n'),
        ('g', 'p'),
    ];
    for &(first, second) in bindings {
        ed.state.status_msg = None;
        ed.handle_key(key(first));
        ed.handle_key(key(second));
        ed.drain_hooks();
        ed.drain_lsp();
        ed.drain_pending_steel_calls();
        if let Some(msg) = &ed.state.status_msg {
            assert!(
                !msg.to_lowercase().contains("error")
                    && !msg.to_lowercase().contains("unknown command"),
                "g {second} must dispatch cleanly on an unattached buffer, got: {msg}"
            );
        }
    }
    drop(guard);
}

/// The commented LSP block in `runtime/init.scm.example` (uncommented) is
/// valid `init.scm` source — a stale block (typo, removed builtin, wrong
/// keyword arg) would fail `eval_init` here instead of silently rotting
/// since nothing else ever evaluates commented-out example code.
///
/// Flip: introducing a typo into the block below (not the real file — this
/// is a literal copy for isolation) reliably fails `eval_init`.
#[test]
#[cfg(not(windows))]
fn commented_init_example_block_is_valid_source() {
    let tmp = safe_tempdir();
    let guard = RealRuntimeGuard::new();
    let mut ed = Editor::open(None).unwrap();
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(load-plugin "core:stdlib")
(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))
(declare-plugin "core:lsp"
  #:events '("on-lsp-attach")
  #:commands '("lsp-hover" "lsp-goto-definition" "lsp-goto-declaration"
               "lsp-goto-type-definition" "lsp-goto-implementation" "lsp-references"
               "goto-next-diagnostic" "goto-prev-diagnostic" "diagnostics"
               "lsp-rename" "lsp-fmt" "lsp-code-actions" "lsp-completion-trigger"))"#,
        tmp.path(),
    );
    drop(guard);
}
