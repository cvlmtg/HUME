//! Tests for the consolidated `hume_scripting::Effect` log: emission-order
//! application across effect kinds, and atomic (all-or-nothing) evals.

use super::*;
use crate::editor::scripting_setup::make_init_host;
use hume_scripting::attribution::PluginId;
use hume_scripting::{Effect, PendingLanguageReg, PendingLspServerOp, ScriptingHost};

/// Writes a lazy `user/efx` plugin at `<dir>/plugins/user/efx/plugin.scm`
/// with `body` as its content, and a matching `init.scm` that declares it
/// with a single `#:commands` trigger (declaring is required to reach
/// `Declared` state; the trigger itself is never dispatched by these tests —
/// activation is driven directly via `ScriptingHost::activate_plugin_inline`).
fn write_efx_plugin(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    let plugin_dir = dir.join("plugins").join("user").join("efx");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), body).unwrap();
    let init_path = dir.join("init.scm");
    std::fs::write(
        &init_path,
        r#"(declare-plugin "user/efx" #:commands '("efx-noop"))"#,
    )
    .unwrap();
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
/// must leave nothing behind in the log — `ScriptingHost::take_eval_effects`
/// truncates back to the eval's start on `Err`, not just on success.
///
/// Flip: in `take_eval_effects`, drop the `self.effects.truncate(effects_start)`
/// call from the `Err` arm — this test starts failing because
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
