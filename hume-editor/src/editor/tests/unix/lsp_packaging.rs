// Packaging: lazy `declare-plugin!` activation, and the goto-trie keybindings
// bound in `plugin.scm`. Loads the real shipped `core:lsp` plugin in place
// (`RealRuntimeDirs`).
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals (same constraint as tests/plugins.rs).

use std::path::Path;

use super::*;
use crate::editor::tests::lsp_rig::{LspRig, RUST_ANALYZER, RigSpec};
use hume_engine::pipeline::RenderContext;
use hume_lsp::test_util::RecordingLspBackend;
use hume_scripting::ScriptingHost;
use hume_scripting::attribution::PluginId;

const DECLARE_LSP: &str = r#"(load-plugin! "core:stdlib")
(declare-plugin! "core:lsp"
  #:events '(on-lsp-attach)
  #:commands '("lsp-hover" "lsp-goto-definition" "lsp-goto-declaration"
               "lsp-goto-type-definition" "lsp-goto-implementation" "lsp-references"
               "goto-next-diagnostic" "goto-prev-diagnostic"
               "lsp-rename" "lsp-fmt" "lsp-code-actions")
  #:typed-commands '("diagnostics"))"#;

/// Same manifest as `DECLARE_LSP` but keyed on `on-buffer-save` instead of
/// `on-lsp-attach`, used to prove a positive activation result isn't a
/// confound of `setup_declared`'s staging (see
/// `attach_event_does_not_activate_a_plugin_declared_for_a_different_event`).
const DECLARE_LSP_WRONG_EVENT: &str = r#"(load-plugin! "core:stdlib")
(declare-plugin! "core:lsp"
  #:events '(on-buffer-save)
  #:commands '("lsp-hover" "lsp-goto-definition" "lsp-goto-declaration"
               "lsp-goto-type-definition" "lsp-goto-implementation" "lsp-references"
               "goto-next-diagnostic" "goto-prev-diagnostic"
               "lsp-rename" "lsp-fmt" "lsp-code-actions")
  #:typed-commands '("diagnostics"))"#;

/// Declares `core:lsp` lazily (`declare_src`, normally `DECLARE_LSP`)
/// instead of `(load-plugin! "core:lsp")`: `declare-plugin!` registers the
/// `Lazy` stub directly via `CommandHost::register_lazy_command` as the
/// rig's init runs, so a `:`-command dispatch can trigger activation with no
/// separate stub-registration step.
///
/// The rig's drain completes the handshake, which queues `on-lsp-attach` on
/// `state.config.pending_work` without processing it: the hook is still
/// pending when this returns, for a later `ed.settle()` to fire.
fn setup_declared(
    tmp: &Path,
    declare_src: &str,
    configure: impl FnOnce(&mut RecordingLspBackend),
) -> (Editor, RealRuntimeDirs) {
    let guard = RealRuntimeDirs::new();

    // 30 lines: comfortably taller than the default pane height's ⅓-cap
    // (see lsp_hover.rs's `setup` for the full rationale): a 1-2 line
    // fixture makes even trivial hover content overflow to the drawer once
    // `(viewport-range bid)` resolves against real (if pre-`prepare_frame`
    // default) pane geometry instead of a test-only #f fallback.
    let filler = (0..29)
        .map(|i| format!("// line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let marked = format!("-[f]>n main() {{}}\n{filler}\n");

    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to(
        "initialize",
        serde_json::json!({"capabilities": {"hoverProvider": true}}),
    );
    configure(&mut backend);
    let init = format!("{declare_src}\n{RUST_ANALYZER}");
    let spec = RigSpec::rust(&marked)
        .with_init(&init)
        .with_dirs(guard.dirs());
    let rig = LspRig::drained(tmp, spec, backend);

    (rig.ed, guard)
}

fn popup_lines(ed: &Editor) -> Option<Vec<String>> {
    ed.state
        .views
        .popup
        .read()
        .as_ref()
        .map(|s| (*s.lines).clone())
}

/// Declaring `core:lsp` (not loading it) leaves it `Declared`: nothing has
/// run its body yet.
#[test]
fn declared_but_undispatched_plugin_is_declared_not_loaded() {
    let tmp = safe_tempdir();
    let (ed, _guard) = setup_declared(tmp.path(), DECLARE_LSP, |backend| {
        backend.respond_to(
            "textDocument/hover",
            serde_json::json!({"contents": {"kind": "plaintext", "value": "fn main()"}}),
        );
    });

    let id = PluginId::parse("core:lsp").unwrap();
    assert_eq!(
        ed.scripting
            .as_ref()
            .unwrap()
            .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
        Some(hume_scripting::PluginStatus::Declared)
    );
}

/// First `:lsp-hover` dispatch on a declared-but-inactive `core:lsp` loads
/// the real plugin body from disk, replaces the lazy stub, and runs the real
/// hover request through to a populated popup.
///
/// Without `CommandHost::register_lazy_command` claiming the stub at
/// declare time, `:lsp-hover` would be an unknown command and this would
/// report an error, not show a popup; without the real activation wiring,
/// `plugin_status` would stay `Declared`.
#[test]
fn first_command_dispatch_activates_the_declared_plugin_and_runs_it() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup_declared(tmp.path(), DECLARE_LSP, |backend| {
        backend.respond_to(
            "textDocument/hover",
            serde_json::json!({"contents": {"kind": "plaintext", "value": "fn main()"}}),
        );
    });

    // Activation runs synchronously inside the command dispatch that hits
    // the lazy stub (`activate_lazy_plugin`, called from the same dispatch
    // path before re-querying and running the now-real command), and the same
    // drain sequence `lsp_hover.rs`'s `run_hover` uses for an eagerly-loaded
    // plugin is enough here too. lsp-hover is key-bindable, not typed, so
    // dispatch through the keymap pipeline, the way `K` would.
    ed.execute_keymap_command("lsp-hover".into(), Some(1), false);
    ed.settle();
    ed.drain_lsp();
    ed.settle();

    // `show-popup!` only populates the popup view once a frame resolves its
    // anchor (`lsp_popup.rs`'s `show_popup_populates_the_view_after_a_frame`).
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert_eq!(popup_lines(&ed), Some(vec!["fn main()".to_string()]));
    let id = PluginId::parse("core:lsp").unwrap();
    assert_eq!(
        ed.scripting
            .as_ref()
            .unwrap()
            .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
        Some(hume_scripting::PluginStatus::Loaded)
    );
}

/// The `on-lsp-attach` event alone, with no `:`-command ever dispatched,
/// activates the declared `core:lsp` plugin. Unlike the command-dispatch test
/// above, nothing here touches a lazy command stub, so the only thing that
/// can flip `Declared` to `Loaded` is `settle`'s
/// `activate_lazy_event_plugins(OnLspAttach)` picking up the hook that
/// `setup_declared`'s handshake already queued.
///
/// `attach_event_does_not_activate_a_plugin_declared_for_a_different_event`
/// runs the identical attach sequence against a manifest declared on
/// `on-buffer-save` instead, and confirms it stays `Declared`, ruling out
/// some other confound in `setup_declared`'s staging (e.g. `load-plugin!
/// "core:stdlib"` in the same source, or the handshake itself) as the cause
/// of this test's `Loaded` result.
#[test]
fn attach_event_alone_activates_the_declared_plugin() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup_declared(tmp.path(), DECLARE_LSP, |_backend| {});

    let id = PluginId::parse("core:lsp").unwrap();
    assert_eq!(
        ed.scripting
            .as_ref()
            .unwrap()
            .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
        Some(hume_scripting::PluginStatus::Declared),
        "must still be Declared going into the drain: only the queued \
         on-lsp-attach hook can flip it here"
    );

    ed.settle();

    assert_eq!(
        ed.scripting
            .as_ref()
            .unwrap()
            .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
        Some(hume_scripting::PluginStatus::Loaded),
        "on-lsp-attach firing (queued by setup_declared's handshake) must \
         activate the plugin with no command ever dispatched"
    );
}

/// Counterpart to `attach_event_alone_activates_the_declared_plugin`:
/// declaring `core:lsp` on `on-buffer-save` instead of `on-lsp-attach` and
/// running the same attach sequence must leave it `Declared`.
#[test]
fn attach_event_does_not_activate_a_plugin_declared_for_a_different_event() {
    let tmp = safe_tempdir();
    let (mut ed, _guard) = setup_declared(tmp.path(), DECLARE_LSP_WRONG_EVENT, |_backend| {});

    ed.settle();

    let id = PluginId::parse("core:lsp").unwrap();
    assert_eq!(
        ed.scripting
            .as_ref()
            .unwrap()
            .plugin_status(&hume_scripting::attribution::EntryId::main(id.clone())),
        Some(hume_scripting::PluginStatus::Declared),
        "on-lsp-attach firing must not activate a plugin declared for a \
         different event (on-buffer-save)"
    );
}

/// Every default `core:lsp` binding dispatches to its named command without
/// error, even fully unattached (no LSP server on the buffer at all); each
/// command's own capability guard degrades to an `'info` log line in that
/// case, never `'error`. Exercises the bindings themselves (does `G R`
/// actually reach `lsp-rename`?); each feature's own test file exercises
/// its LSP behavior once attached.
///
/// `G R` and bare `K` are the two keys `core:lsp` shares a namespace with:
/// `G` is otherwise the case-transform prefix, and `z k` (a plain viewport
/// command, distinct from `K`) sits right next to `core:lsp`'s own `z r`/
/// `z a`. `z k` is asserted below to still resolve to `top-view-on-cursor`,
/// and `G L` is asserted via `lookup_command` after the loop, as load-order
/// proofs that `core:lsp` slots in beside these rather than clobbering them.
///
/// Renaming a binding's target command in `plugin.scm` without updating this
/// table (or the other way round) makes the matching iteration fail with
/// "unknown command".
#[test]
fn every_default_lsp_binding_dispatches_without_error() {
    use crate::editor::keymap::BindMode;

    let tmp = safe_tempdir();
    let file_dir = safe_tempdir();
    let file = file_dir.path().join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let guard = RealRuntimeDirs::new();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {}), guard.dirs()).unwrap();
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();

    let mut host = ScriptingHost::new(&ed.state.dirs);
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(load-plugin! "core:stdlib")
(load-plugin! "core:lsp") (%activate-plugin-inline! "core:lsp" #f)"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    // Each entry is the key sequence to press: one key for `K`, two for
    // every `g`/`z`/`G`-prefixed bind.
    let bindings: &[&[char]] = &[
        &['g', 'd'],
        &['g', 'D'],
        &['g', 'y'],
        &['g', 'i'],
        &['G', 'R'],
        &['K'],
        &['z', 'r'],
        &['z', 'a'],
        &['g', 'n'],
        &['g', 'p'],
    ];
    for keys in bindings {
        ed.state.status_msg = None;
        for &k in *keys {
            ed.handle_key(key(k));
        }
        ed.settle();
        ed.drain_lsp();
        ed.settle();
        if let Some(msg) = &ed.state.status_msg {
            assert!(
                !msg.to_lowercase().contains("error")
                    && !msg.to_lowercase().contains("unknown command"),
                "{keys:?} must dispatch cleanly on an unattached buffer, got: {msg}"
            );
        }
    }

    // Load-order proofs: core:lsp's binds sit beside these, not over them.
    assert_eq!(
        ed.state
            .config
            .keymap
            .lookup_command(BindMode::Normal, &[key('z'), key('k')]),
        Some(("top-view-on-cursor".to_string(), false)),
        "z k must still be the native viewport command with core:lsp loaded"
    );
    assert_eq!(
        ed.state
            .config
            .keymap
            .lookup_command(BindMode::Normal, &[key('G'), key('L')]),
        Some(("make-text-lowercase".to_string(), false)),
        "G L must still be the case-transform command with core:lsp loaded"
    );
    drop(guard);
}

/// Loading `core:lsp` without `core:stdlib` declared or loaded first must
/// fail to load (contained, not aborting `eval_init`), naming `core:stdlib`
/// (`core:lsp`'s `(declared-plugins)` guard rejects a `core:stdlib` that
/// was never declared or loaded at all).
#[test]
fn missing_stdlib_errors_at_load() {
    let guard = RealRuntimeDirs::new();
    let tmp = safe_tempdir();
    let init_path = tmp.path().join("init.scm");
    std::fs::write(
        &init_path,
        r#"(load-plugin! "core:lsp") (%activate-plugin-inline! "core:lsp" #f)"#,
    )
    .unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    ed.state.dirs = guard.dirs();
    let mut host = ScriptingHost::new(&ed.state.dirs);
    let result = {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    };
    result.expect("a failed plugin load must be contained, not abort eval_init");
    assert!(
        host.peek_pending_messages().iter().any(|(level, msg)| {
            matches!(level, hume_scripting::LogLevel::Error) && msg.contains("core:stdlib")
        }),
        "error must name the missing dependency; got: {:?}",
        host.peek_pending_messages()
    );
    drop(guard);
}
