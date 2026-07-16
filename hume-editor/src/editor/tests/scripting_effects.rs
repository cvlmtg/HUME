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
/// deliberately NOT the old per-kind grouping order (language regs → LSP
/// server ops → LSP calls → buffer-language sets) a pre-refactor reader
/// might expect. The returned log must reflect the exact push order
/// (proving builtins share one `Vec<Effect>`, not per-kind queues that
/// `apply_script_effects` would have to regroup), and applying that log
/// must still land all three: language identity registered, LSP server
/// config recorded, buffer's language field set — none of which depends on
/// `define-language!` having run first, which is precisely why the old
/// grouped scheme could get away with reordering.
///
/// Fail oracle: reintroduce separate per-kind accumulators in `SteelCtx`
/// (e.g. a dedicated `pending_lsp_server_ops` alongside `effects`) — a
/// builtin pushing to the wrong one would still leave `effects.len() == 3`
/// here (nothing dropped) but with a different relative order, and the
/// `effects[0]`/`effects[1]`/`effects[2]` variant assertions below fail.
#[test]
#[cfg(not(windows))]
fn effect_log_preserves_emission_order_across_kinds() {
    let dir = safe_tempdir();
    let init_path = write_efx_plugin(
        dir.path(),
        // `%define-language!` (the raw builtin), not the `define-language!`
        // macro — that macro lives in `runtime/scheme/prelude.scm`, not
        // loaded by this test's bare `ScriptingHost::new()`.
        r#"(register-lsp-server! "widget" #:command "widget-lsp" #:root-markers '())
           (set-buffer-language! (car (buffers)) "widget")
           (%define-language! "widget" '("widget") '() '())
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
        ed.state.languages.by_name("widget").is_some(),
        "language identity must be registered"
    );
    assert_eq!(
        ed.lsp.config_command_for_test("widget"),
        Some("widget-lsp".to_string()),
        "LSP server config must be registered"
    );
    assert_eq!(
        ed.state.buffers.get(bid).language.as_deref(),
        Some("widget"),
        "buffer language must be set"
    );
}

/// Atomic-eval contract, exercised through `call_steel_cmd` (a plain
/// command dispatch, no nested plugin activation involved — that path has
/// its own independent rollback via `pop_effect_marks`, already covered by
/// `hume-scripting`'s `queued_effects_before_failure_are_rolled_back`). A
/// command body that queues a `register-lsp-server!` effect and then errors
/// must leave nothing behind in the log — with no nested activation to
/// commit anything, `ScriptingHost::take_eval_effects` has nothing to
/// salvage on `Err`, so the whole eval's own uncommitted entries are dropped.
///
/// Flip: in `take_eval_effects`, salvage every entry regardless of
/// `committed` on the `Err` arm — this test starts failing because
/// `host.effects_for_test()` comes back non-empty (the queued
/// `register-lsp-server!` survives the error).
#[test]
fn failed_command_eval_effects_do_not_leak() {
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_source(
            r#"(define-command! "efx-fail" ""
                 (lambda ()
                   (register-lsp-server! "leaked" #:command "leaked-lsp")
                   (error "intentional mid-body failure")))"#,
            &mut ih,
        )
    }
    .expect("define-command! must succeed");

    let pid = ed.state.focused_pane_id;
    let bid = ed.focused_buffer_id();
    let result = {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.call_steel_cmd("efx-fail", None, vec![], pid, bid, &mut ih)
    };
    assert!(
        result.is_err(),
        "the command must fail on its intentional error"
    );
    assert!(
        host.effects_for_test().is_empty(),
        "the failed command's queued register-lsp-server! effect must not survive; got: {:?}",
        host.effects_for_test()
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
#[cfg(not(windows))]
fn failed_command_delivers_committed_activation_effects() {
    let dir = safe_tempdir();
    let init_path = write_efx_plugin(
        dir.path(),
        r#"(register-lsp-server! "widget" #:command "widget-lsp")
           (define-command! "b-cmd" "" (lambda () 0))"#,
        r#"(declare-plugin "user/efx" #:commands '("b-cmd"))
           (define-command! "outer-fail" ""
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
#[cfg(not(windows))]
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
