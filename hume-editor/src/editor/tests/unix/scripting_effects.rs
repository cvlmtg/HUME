//! Tests for the consolidated `hume_scripting::Effect` log: emission-order
//! application across effect kinds, and atomic (all-or-nothing) evals.

use super::*;
use crate::editor::scripting_setup::make_init_host;
use hume_scripting::attribution::PluginId;
use hume_scripting::{Effect, PendingLanguageReg, PendingLspServerOp, PluginStatus, ScriptingHost};

/// Writes a lazy `user/efx` plugin at `<dir>/plugins/user/efx/plugin.scm`
/// with `plugin_body` as its content, and `init_src` as `init.scm`'s content.
fn write_efx_plugin(
    dir: &std::path::Path,
    plugin_body: &str,
    init_src: &str,
) -> std::path::PathBuf {
    let plugin_dir = dir.join("plugins").join("user").join("efx");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), plugin_body).unwrap();
    let init_path = dir.join("init.scm");
    std::fs::write(&init_path, init_src).unwrap();
    init_path
}

/// One eval (a lazy plugin's activation body) emits, in this exact order:
/// `register-lsp-server!` → `set-buffer-language!` → `define-language!` —
/// deliberately not grouped by kind (language regs, then LSP ops, then
/// buffer-language sets). The returned log must reflect the exact push
/// order (proving builtins share one `Vec<Effect>`, not per-kind queues
/// that `apply_script_effects` would have to regroup), and applying that
/// log must still land all three: language identity registered, LSP server
/// config recorded, buffer's language field set — none of which depends on
/// `define-language!` having run first, so a per-kind grouping scheme could
/// silently get away with reordering these.
///
/// Fail oracle: reintroduce separate per-kind accumulators in `SteelCtx`
/// (e.g. a dedicated `pending_lsp_server_ops` alongside `effects`) — a
/// builtin pushing to the wrong one would still leave `effects.len() == 3`
/// here (nothing dropped) but with a different relative order, and the
/// `effects[0]`/`effects[1]`/`effects[2]` variant assertions below fail.
#[test]
fn effect_log_preserves_emission_order_across_kinds() {
    let dir = safe_tempdir();
    let init_path = write_efx_plugin(
        dir.path(),
        // `%define-language!` (the raw builtin), not the `define-language!`
        // macro — that macro lives in `runtime/scheme/prelude.scm`, not
        // loaded by this test's bare `ScriptingHost::new()`.
        r#"(register-lsp-server! "widget" #:command "widget-lsp" #:root-markers '())
           (set-buffer-language! (car (buffers)) "widget")
           (%define-language! "widget" '("widget") '() '() #f)
           (define-command! "efx-noop" "" (lambda () 0))"#,
        r#"(declare-plugin "user/efx" #:commands '("efx-noop"))"#,
    );

    let mut ed = editor_from("-[a]>bcdef\n");
    let bid = ed.focused_buffer_id();
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    // declare-plugin queues no effects — nothing to apply from this eval.
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");

    let plugin_id = PluginId::User {
        user: "user".to_string(),
        repo: "efx".to_string(),
    };
    let effects = {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.activate_plugin_inline(&plugin_id, 10_000, &mut ih, &Default::default())
    }
    .expect("activation must succeed");

    assert_eq!(
        effects.len(),
        3,
        "expected exactly 3 queued effects, got: {effects:?}"
    );
    assert!(
        matches!(
            &effects[0],
            Effect::LspServerOp(PendingLspServerOp::Register(reg)) if reg.language == "widget"
        ),
        "effect 0 must be the register-lsp-server! call, pushed first; got {:?}",
        effects[0]
    );
    assert!(
        matches!(
            &effects[1],
            Effect::SetBufferLanguage { language, .. } if language.as_deref() == Some("widget")
        ),
        "effect 1 must be the set-buffer-language! call, pushed second; got {:?}",
        effects[1]
    );
    assert!(
        matches!(
            &effects[2],
            Effect::LanguageReg(PendingLanguageReg::Identity { name, .. }) if name == "widget"
        ),
        "effect 2 must be the define-language! call, pushed third; got {:?}",
        effects[2]
    );

    ed.apply_script_effects(effects);

    assert!(
        ed.state.config.languages.by_name("widget").is_some(),
        "language identity must be registered"
    );
    assert_eq!(
        ed.lsp.config_command_for_test("widget"),
        Some("widget-lsp".to_string()),
        "LSP server config must be registered"
    );
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("widget"),
        "buffer language must be set"
    );
}

/// Full dispatch pipeline (`Editor::run_steel_command`'s `Err` arm), not just
/// `ScriptingHost` directly: `:outer-fail` `call!`s a lazy command owned by
/// plugin `user/efx`. The plugin activates inline mid-body, committing
/// `Loaded` and queuing `register-lsp-server!` for "widget", then the outer
/// command errors. The editor must still apply the plugin's committed effect
/// — otherwise `user/efx` is permanently `Loaded` with its LSP server never
/// registered, since activation is one-shot. The outer command's own effects
/// (queued before and after the nested activation) must not apply.
///
/// Fail oracle: drop the `self.apply_script_effects(e.effects)` call from
/// `run_steel_command`'s `Err` arm — `config_command_for_test("widget")`
/// comes back `None` even though the plugin is `Loaded`.
#[test]
fn failed_command_delivers_committed_activation_effects() {
    let dir = safe_tempdir();
    let init_path = write_efx_plugin(
        dir.path(),
        r#"(register-lsp-server! "widget" #:command "widget-lsp")
           (define-command! "b-cmd" "" (lambda () 0))"#,
        r#"(declare-plugin "user/efx" #:commands '("b-cmd"))
           (define-typed-command! "outer-fail" ""
             (lambda ()
               (register-lsp-server! "before" #:command "x")
               (call! "b-cmd")
               (register-lsp-server! "after" #:command "y")
               (error "intentional outer failure")))"#,
    );

    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":outer-fail");

    assert_eq!(
        ed.lsp.config_command_for_test("widget"),
        Some("widget-lsp".to_string()),
        "the activated plugin's committed effect must apply despite the outer command's failure"
    );
    assert_eq!(
        ed.lsp.config_command_for_test("before"),
        None,
        "the outer command's own pre-activation effect must not apply"
    );
    assert_eq!(
        ed.lsp.config_command_for_test("after"),
        None,
        "the outer command's own post-activation effect must not apply"
    );

    let plugin_id = PluginId::User {
        user: "user".to_string(),
        repo: "efx".to_string(),
    };
    assert_eq!(
        ed.scripting.as_ref().unwrap().plugin_status(&plugin_id),
        Some(PluginStatus::Loaded),
        "user/efx must be Loaded — its activation succeeded before outer-fail's own failure"
    );

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("intentional outer failure"),
        "the outer command's error must still be reported: {log:?}"
    );
}

/// Pins the salvage contract at the exact boundary `init_scripting` uses:
/// `eval_init` returning `Err(EvalError)` when `load-plugin` (eager
/// activation) already committed effects before a later top-level error.
/// The caller (mirroring `init_scripting`'s error arm) must apply
/// `EvalError::effects` before reporting.
#[test]
fn failed_init_eval_salvages_eager_plugin_effects() {
    let dir = safe_tempdir();
    let init_path = write_efx_plugin(
        dir.path(),
        r#"(register-lsp-server! "widget" #:command "widget-lsp")"#,
        r#"(load-plugin "user/efx")
           (error "init fails")"#,
    );

    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    host.set_data_dir(dir.path().to_path_buf());
    let err = {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect_err("eval_init must fail on the top-level error");

    assert_eq!(
        err.effects.len(),
        1,
        "load-plugin's committed register-lsp-server! must be salvaged; got: {:?}",
        err.effects
    );
    assert!(
        matches!(
            &err.effects[0],
            Effect::LspServerOp(PendingLspServerOp::Register(reg)) if reg.language == "widget"
        ),
        "salvaged effect must be the widget registration; got: {:?}",
        err.effects[0]
    );

    ed.apply_script_effects(err.effects);
    assert_eq!(
        ed.lsp.config_command_for_test("widget"),
        Some("widget-lsp".to_string()),
        "applying the salvaged effect must register the LSP server"
    );

    let plugin_id = PluginId::User {
        user: "user".to_string(),
        repo: "efx".to_string(),
    };
    assert_eq!(
        host.plugin_status(&plugin_id),
        Some(PluginStatus::Loaded),
        "user/efx must be Loaded — load-plugin's activation succeeded before the top-level error"
    );
}

// ── open-buffer! detects language via pending_language_detection ───────────

/// `(open-buffer! path)` can't run language detection inline — the host it
/// executes against has no Steel-eval capability for lazy-plugin activation
/// (see `buffer::lifecycle::open_buffer_and_notify`'s doc) — so the open
/// chokepoint queues the buffer id onto `EditorState.pending_language_
/// detection` instead, drained once the eval returns. This is the full
/// pipeline: `:go` → `call_steel_cmd` → `open-buffer!` opens (queuing the
/// bid) → `apply_script_effects`'s tail drain runs `detect_and_set_language`.
///
/// Fail oracle: drop the `state.config.pending_language_detection.push(bid)` line
/// from `open_buffer_and_notify` (or the tail drain from
/// `apply_script_effects`) — the opened buffer's `language` stays `None`.
#[test]
fn steel_open_buffer_detects_language() {
    let dir = safe_tempdir();
    let target = dir.path().join("target.rs");
    std::fs::write(&target, "fn main() {}\n").unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        format!(
            r#"(define-typed-command! "go" "" (lambda () (open-buffer! "{}")))"#,
            target.display()
        ),
    )
    .unwrap();

    let mut ed = editor_from("-[a]>bcdef\n");
    ed.state
        .config
        .languages
        .register_identity_no_rebuild("rust", &["rs"], &[], &[], None);
    ed.state
        .config
        .languages
        .rebuild_glob_set()
        .expect("rebuild ok");

    let mut host = ScriptingHost::new();
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":go");

    let canonical = target.canonicalize().unwrap();
    let bid = ed
        .state
        .buffers
        .find_by_path(&canonical)
        .expect("open-buffer! must have opened the file");
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("rust"),
        "open-buffer! must detect the opened file's language"
    );
}

/// `(open-buffer! path)` on a path that doesn't exist yet must open an empty
/// new-file buffer, the same tolerance `:e` has (`host_impl::open_buffer`
/// shares `Editor::resolve_buffer_path` / `Buffer::from_file_or_new` with
/// `:e` for exactly this reason) — not error out the way a hard
/// `std::fs::canonicalize` would.
#[test]
fn steel_open_buffer_missing_path_opens_new_file() {
    let dir = safe_tempdir();
    let target = dir.path().join("not-yet-created.txt");
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        format!(
            r#"(define-typed-command! "go" "" (lambda () (open-buffer! "{}")))"#,
            target.display()
        ),
    )
    .unwrap();

    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":go");

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.text.contains("open-buffer!")),
        "must not error for a missing path — it opens instead"
    );
    let canonical_dir = dir.path().canonicalize().unwrap();
    let bid = ed
        .state
        .buffers
        .find_by_path(&canonical_dir.join("not-yet-created.txt"))
        .expect("open-buffer! must have opened a new-file buffer");
    assert!(ed.state.buffers.get(bid).is_new_file());
}

// ── open+close in one eval fires neither hook ──────────────────────────────

/// `(open-buffer! path)` queues `bid` onto `pending_language_detection`
/// rather than firing `OnBufferOpen` inline (see
/// `steel_open_buffer_detects_language` above) — the fire happens only once
/// `apply_script_effects`'s tail drain runs, after this eval returns. If the
/// same eval closes `bid` first via `(close-buffer!)`, the drain finds the
/// slot gone and skips it: `OnBufferOpen` never fires. `OnBufferClose` must
/// not fire either in that case — a buffer that never announced its open
/// must not announce a close, or a plugin sees a close for an id it never
/// heard opened.
///
/// Fail oracle: drop the `open_announced` gate in `close_buffer_and_notify`
/// (queue `OnBufferClose` unconditionally) — `pending_work` gains an
/// `OnBufferClose` entry after `:go`, with no matching `OnBufferOpen`.
#[test]
fn buffer_opened_and_closed_in_one_eval_fires_neither_hook() {
    use crate::editor::event::EditorEvent;

    let dir = safe_tempdir();
    let target = dir.path().join("target.txt");
    std::fs::write(&target, "hello\n").unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        format!(
            r#"(define-typed-command! "go" "" (lambda () (close-buffer! (open-buffer! "{}"))))"#,
            target.display()
        ),
    )
    .unwrap();

    let mut ed = editor_from("-[a]>bcdef\n");

    let mut host = ScriptingHost::new();
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":go");

    let canonical = target.canonicalize().unwrap();
    assert!(
        ed.state.buffers.find_by_path(&canonical).is_none(),
        "close-buffer! must have closed the just-opened buffer"
    );

    let pending: Vec<&EditorEvent> = ed
        .state
        .config
        .pending_work
        .iter()
        .filter_map(|w| match w {
            crate::editor::event::PendingWork::Event(e) => Some(e),
            crate::editor::event::PendingWork::Call(..) => None,
        })
        .collect();
    assert!(
        !pending
            .iter()
            .any(|e| matches!(e, EditorEvent::OnBufferOpen { .. })),
        "a buffer closed before its deferred OnBufferOpen fired must not announce open; got {pending:?}"
    );
    assert!(
        !pending
            .iter()
            .any(|e| matches!(e, EditorEvent::OnBufferClose { .. })),
        "a buffer that never announced OnBufferOpen must not announce OnBufferClose either; got {pending:?}"
    );
}
