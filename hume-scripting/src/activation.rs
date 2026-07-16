//! Plugin activation and core eval machinery for [`super::ScriptingHost`].
//!
//! ## Activation state machine
//!
//! ```text
//! Declared ──── %activate-plugin-inline ────► Loading ──┬──► Loaded
//!                                                        └──► Failed
//!
//! Loaded / Failed / Loading / absent  ──► no-op (#f guard in %begin-lazy-activation)
//! ```
//!
//! All plugin activation is synchronous/inline:
//! - `load-plugin` (init.scm): the BOOTSTRAP Scheme wrapper calls `%load-plugin!`
//!   (declare/record) then `%activate-plugin-inline` (inline body eval via `hm.eval-string`).
//! - Lazy keypress dispatch: `%dispatch-command` activates the owner inline on a
//!   `command_table` miss, then retries.
//! - Event/language activations: `activate_plugin_inline` (Rust) bounces into
//!   `(%activate-plugin-inline id)` via `run_steel_call` — a direct function
//!   call, not source — using the `ScriptingHost`'s one persistent watchdog.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;
use steel::steel_vm::engine::Engine;

use crate::HUME_CTX;
use crate::ScriptingHost;
use crate::attribution;
use crate::context::SteelCtx;
use crate::host::EditorHost;
use crate::types::{Effect, EvalError};
use crate::watchdog::EvalWatchdog;

// ── run_steel ──────────────────────────────────��──────────────────────────────

/// Arm the watchdog, run `body` against `steel` with `ctx` visible as
/// `*hume.ctx*`, then cancel the watchdog and reset the interrupt flag.
///
/// Shared by `eval_source_raw` (compiles a source program), `call_steel_cmd` /
/// `fire_hook` / `activate_plugin_inline` (direct function calls) so the
/// arm / eval / cancel / reset ceremony lives in one place.
pub(crate) fn run_steel_session<'a, R>(
    steel: &mut Engine,
    watchdog: &EvalWatchdog,
    ctx: &mut SteelCtx<'a>,
    budget_ms: u64,
    body: impl FnOnce(&mut Engine) -> Result<R, SteelErr>,
) -> Result<(), String> {
    watchdog.arm(
        Arc::clone(&ctx.interrupt_flag),
        std::time::Duration::from_millis(budget_ms),
    );
    let result = steel
        .with_mut_reference::<SteelCtx<'a>, SteelCtx<'static>>(ctx)
        .consume_once(|steel, args| {
            let ctx_val = args
                .into_iter()
                .next()
                .expect("with_mut_reference yields one arg");
            steel.update_value(HUME_CTX, ctx_val);
            let res = body(steel);
            steel.update_value(HUME_CTX, SteelVal::Void);
            res
        })
        .map(|_| ())
        .map_err(|e| e.to_string());
    watchdog.cancel();
    ctx.interrupt_flag.store(false, Ordering::Relaxed);
    result
}

/// [`run_steel_session`] with a source-program body — parse + compile + run.
///
/// Only for genuinely dynamic source (init.scm, test snippets).  Fixed-shape
/// invocations use [`run_steel_call`], which skips the compiler entirely.
pub(crate) fn run_steel<'a>(
    steel: &mut Engine,
    watchdog: &EvalWatchdog,
    ctx: &mut SteelCtx<'a>,
    program: String,
    budget_ms: u64,
) -> Result<(), String> {
    run_steel_session(steel, watchdog, ctx, budget_ms, |steel| {
        steel.compile_and_run_raw_program(program)
    })
}

/// [`run_steel_session`] with a direct function-call body.
///
/// Calls the global Steel function `fn_name` with `args` — no source string,
/// no compilation, and no way for a hostile name or argument to alter program
/// structure (args are passed as values, never spliced into source).
pub(crate) fn run_steel_call<'a>(
    steel: &mut Engine,
    watchdog: &EvalWatchdog,
    ctx: &mut SteelCtx<'a>,
    fn_name: &str,
    args: Vec<SteelVal>,
    budget_ms: u64,
) -> Result<(), String> {
    run_steel_session(steel, watchdog, ctx, budget_ms, |steel| {
        steel.call_function_by_name_with_args(fn_name, args)
    })
}

// ── ScriptingHost — activation impl ──────────────────────────────────────────

impl ScriptingHost {
    /// Core eval machinery used by [`ScriptingHost::eval_init`].
    ///
    /// Evaluates `source` (init.scm) synchronously.  `(load-plugin …)` calls
    /// inside the source activate their plugin bodies inline via the BOOTSTRAP
    /// `%activate-plugin-inline` helper (VM-aware `hm.eval-string`, no
    /// `&mut Engine` borrow).  `(define-command! …)` calls register commands
    /// directly into the editor's `CommandRegistry` via `host.register_command`.
    ///
    /// Returns the effects this eval queued (atomically on success; on error,
    /// only effects committed by a nested successful plugin activation — see
    /// `ScriptingHost::take_eval_effects`).
    pub(crate) fn eval_source_raw(
        &mut self,
        source: String,
        builtin_names: HashSet<String>,
        budget_ms: u64,
        host: &mut dyn EditorHost,
    ) -> Result<Vec<Effect>, EvalError> {
        let effects_start = self.effects.len();
        let result = {
            let (steel, watchdog, bundle) = self.steel_and_bundle();
            let mut steel_ctx = SteelCtx::new_init(host, bundle, builtin_names);
            run_steel(steel, watchdog, &mut steel_ctx, source, budget_ms)
        };
        self.take_eval_effects(effects_start, result)
    }

    /// Activate a plugin inline via `%activate-plugin-inline` using `run_steel_call`.
    ///
    /// Used by event- and language-activation paths (`activate_and_register`) that
    /// fire outside any running eval and need their own watchdog.  The plugin body
    /// runs inside `hm.eval-string` (VM-aware, no `&mut Engine` borrow), sharing
    /// the same `ctx.registries` as any concurrent eval.  `define-command!` calls
    /// inside the body register directly into `host.register_command` inline.
    ///
    /// Returns the activating body's queued effects (`register-lsp-server!`,
    /// `set-buffer-language!`, etc.) so the caller can apply them immediately —
    /// otherwise they'd sit unapplied until some unrelated later drain, which can
    /// skip attaching the very buffer that triggered this activation.
    ///
    /// A failed activation's own effects are already rolled back by the
    /// BOOTSTRAP `%activate-plugin-inline` wrapper's `%begin-lazy-activation`/
    /// `%finish-lazy-activation` mark/pop (`ctx.pop_effect_marks`) before the
    /// error reaches here — except any effects committed by a nested
    /// successful plugin activation, which `pop_effect_marks` keeps and which
    /// `take_eval_effects` salvages onto the returned `EvalError`.
    pub fn activate_plugin_inline(
        &mut self,
        id: &attribution::PluginId,
        budget_ms: u64,
        host: &mut dyn EditorHost,
        builtin_names: &HashSet<String>,
    ) -> Result<Vec<Effect>, EvalError> {
        let args = vec![SteelVal::StringV(id.to_string().into())];
        let effects_start = self.effects.len();
        let result = {
            let (steel, watchdog, bundle) = self.steel_and_bundle();
            let mut steel_ctx = SteelCtx::new_activation(host, bundle, builtin_names.clone());
            run_steel_call(
                steel,
                watchdog,
                &mut steel_ctx,
                "%activate-plugin-inline",
                args,
                budget_ms,
            )
        };
        self.take_eval_effects(effects_start, result)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::io::Write as _;

    use tempfile::TempDir;

    use crate::ScriptingHost;
    use crate::attribution::PluginId;
    use crate::lazy::PluginState;
    use crate::null_host::NullHost;

    /// Write a Steel source file into `dir` and return its path.
    fn write_plugin(dir: &TempDir, name: &str, src: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(src.as_bytes()).unwrap();
        path
    }

    fn plugin_id(name: &str) -> PluginId {
        PluginId::parse(name).unwrap()
    }

    fn no_builtins() -> HashSet<String> {
        HashSet::new()
    }

    // ── Case 1: Declared → Loaded with a valid command body ──────────────────

    #[test]
    fn declared_to_loaded_registers_command() {
        let dir = TempDir::new().unwrap();
        let path = write_plugin(
            &dir,
            "plugin.scm",
            r#"(define-command! "test-cmd" "A test command." (lambda () 0))"#,
        );
        let id = plugin_id("core:test");
        let mut host = ScriptingHost::new();
        host.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Declared { path });

        host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins())
            .unwrap();

        assert!(
            host.registries.command_table.contains_key("test-cmd"),
            "command must be in command_table after activation"
        );
        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id),
                Some(PluginState::Loaded)
            ),
            "plugin must be in Loaded state after successful activation"
        );
    }

    // ── Case 2: Syntax error → Failed, Err returned ──────────────────────────

    #[test]
    fn syntax_error_transitions_to_failed() {
        let dir = TempDir::new().unwrap();
        let path = write_plugin(&dir, "bad.scm", "(((invalid syntax");
        let id = plugin_id("core:bad");
        let mut host = ScriptingHost::new();
        host.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Declared { path });

        let result = host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins());

        assert!(result.is_err(), "must return Err on syntax error");
        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id),
                Some(PluginState::Failed)
            ),
            "plugin must be in Failed state after syntax error"
        );
    }

    // ── Case 3: Idempotent no-ops for non-Declared states ────────────────────

    #[test]
    fn already_loaded_is_noop() {
        let id = plugin_id("core:loaded");
        let mut host = ScriptingHost::new();
        host.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loaded);

        host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins())
            .unwrap();

        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id),
                Some(PluginState::Loaded)
            ),
            "state must remain Loaded"
        );
    }

    #[test]
    fn already_failed_is_noop() {
        let id = plugin_id("core:failed");
        let mut host = ScriptingHost::new();
        host.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Failed);

        host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins())
            .unwrap();

        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id),
                Some(PluginState::Failed)
            ),
            "state must remain Failed"
        );
    }

    #[test]
    fn absent_plugin_is_noop() {
        let id = plugin_id("core:absent");
        let mut host = ScriptingHost::new();

        host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins())
            .unwrap();

        assert!(
            !host.registries.lazy_registry.plugins.contains_key(&id),
            "absent plugin must not appear in registry after no-op"
        );
    }

    // ── Case 4: Loading re-entrancy guard → no-op ────────────────────────────

    #[test]
    fn loading_reentrancy_guard_is_noop() {
        let id = plugin_id("core:cycling");
        let mut host = ScriptingHost::new();
        host.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loading);

        host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins())
            .unwrap();

        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id),
                Some(PluginState::Loading)
            ),
            "state must remain Loading (re-entrancy guard must not overwrite)"
        );
    }

    // ── Case 5: Path containing '"' rejected before any eval ─────────────────

    #[test]
    fn path_with_quote_char_transitions_to_failed() {
        let id = plugin_id("core:quoted");
        let mut host = ScriptingHost::new();
        host.registries.lazy_registry.plugins.insert(
            id.clone(),
            PluginState::Declared {
                path: std::path::PathBuf::from("/some/path\"with/quote/plugin.scm"),
            },
        );

        let result = host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins());

        assert!(result.is_err(), "path with '\"' must be rejected");
        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id),
                Some(PluginState::Failed)
            ),
            "plugin must be Failed after path-with-quote rejection"
        );
    }

    // ── Stage B: eval-string plumbing ─────────────────────────────────────────

    /// A `define-command!` issued via `eval-string` inside a `run_steel` session
    /// registers the command in `command_table` — proving that the nested eval-string
    /// sees the same `ctx.registries` as the outer eval.
    ///
    /// Verification: commenting out the `eval-string` call makes the assert fail.
    #[test]
    fn eval_string_nested_registers_command_in_command_table() {
        let mut host = ScriptingHost::new();
        // Eval a snippet that eval-strings a define-command! — the outer eval is
        // in EvalMode::Init, which allows define-command!.
        let program = r#"
(hm.eval-string "(define-command! \"inner-cmd\" \"doc\" (lambda () 0))")
"#;
        host.eval_source(program, &mut NullHost).unwrap();
        assert!(
            host.registries.command_table.contains_key("inner-cmd"),
            "command defined via eval-string must appear in command_table"
        );
    }

    /// `%begin-lazy-activation` on a `Declared` plugin transitions to `Loading`,
    /// pushes onto `plugin_stack`, and returns the require-string.
    #[test]
    fn begin_lazy_activation_declared_returns_require_string() {
        let dir = TempDir::new().unwrap();
        let path = write_plugin(
            &dir,
            "p.scm",
            r#"(define-command! "p-cmd" "doc" (lambda () 0))"#,
        );
        let id = plugin_id("core:p");
        let mut host = ScriptingHost::new();
        host.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Declared { path: path.clone() });

        let program = r#"(define result (%begin-lazy-activation "core:p"))"#;
        host.eval_source(program, &mut NullHost).unwrap();

        // Plugin must be in Loading state.
        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id),
                Some(PluginState::Loading)
            ),
            "Declared plugin must be Loading after %begin-lazy-activation"
        );
        // plugin_stack must have grown by one — begin pushed the id.
        assert_eq!(
            host.plugin_stack_depth_for_test(),
            1,
            "plugin_stack depth must be 1"
        );
    }

    /// `%begin-lazy-activation` on a `Loading` plugin returns `#f` (cycle guard).
    #[test]
    fn begin_lazy_activation_loading_returns_false() {
        let id = plugin_id("core:cycling");
        let mut host = ScriptingHost::new();
        host.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loading);

        // The result `#f` means the (when prog ...) in %activate-plugin-inline
        // does nothing — activation is a no-op.
        let program = r#"
(define result (%begin-lazy-activation "core:cycling"))
(when result (error "cycle guard must return #f!"))
"#;
        host.eval_source(program, &mut NullHost).unwrap();
        // State must remain Loading.
        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id),
                Some(PluginState::Loading)
            ),
            "state must remain Loading (cycle guard)"
        );
    }

    /// `%finish-lazy-activation` with success=true transitions to `Loaded`.
    #[test]
    fn finish_lazy_activation_success_transitions_to_loaded() {
        let id = plugin_id("core:finishing");
        let mut host = ScriptingHost::new();
        host.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loading);
        // Seed the stack as begin_lazy_activation would have done.
        host.push_plugin_for_test(id.clone());

        let program = r#"(%finish-lazy-activation "core:finishing" #t)"#;
        host.eval_source(program, &mut NullHost).unwrap();

        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id),
                Some(PluginState::Loaded)
            ),
            "plugin must be Loaded after successful finish"
        );
        assert_eq!(
            host.plugin_stack_depth_for_test(),
            0,
            "plugin_stack must be empty after finish"
        );
    }

    /// `%finish-lazy-activation` with success=false transitions to `Failed`.
    #[test]
    fn finish_lazy_activation_failure_transitions_to_failed() {
        let id = plugin_id("core:failing");
        let mut host = ScriptingHost::new();
        host.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loading);
        host.push_plugin_for_test(id.clone());

        let program = r#"(%finish-lazy-activation "core:failing" #f)"#;
        host.eval_source(program, &mut NullHost).unwrap();

        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id),
                Some(PluginState::Failed)
            ),
            "plugin must be Failed after failed finish"
        );
    }

    // ── Partial-define rollback (D2) ─────────────────────────────────────────

    /// A plugin body that defines one command and then errors: the plugin
    /// transitions to `Failed` and `finish_lazy_activation` rolls back the
    /// partial `define-command!` — removing it from `command_table` and
    /// `cmd_owners`.  A `Failed` plugin must not leave callable orphan commands.
    ///
    /// Fail oracle: without the rollback the key persists in `command_table` →
    /// the `command_table` assert fires, exposing the orphan.
    #[test]
    fn partial_define_before_failure_is_rolled_back() {
        let dir = TempDir::new().unwrap();
        let path = write_plugin(
            &dir,
            "partial.scm",
            r#"(define-command! "partial-cmd" "doc" (lambda () 0))
               (error "intentional mid-body error")"#,
        );
        let id = plugin_id("core:partial");
        let mut host = ScriptingHost::new();
        host.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Declared { path });

        let result = host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins());

        assert!(result.is_err(), "activation must fail on intentional error");
        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id),
                Some(PluginState::Failed)
            ),
            "plugin must be Failed after mid-body error"
        );
        // The partial define must be rolled back — no callable orphan left behind.
        assert!(
            !host.registries.command_table.contains_key("partial-cmd"),
            "partial define-command! must be removed from command_table on failure"
        );
        assert!(
            !host.registries.cmd_owners.contains_key("partial-cmd"),
            "partial define-command! must be removed from cmd_owners on failure"
        );
    }

    /// A plugin body that queues an LSP server registration and a language
    /// registration and then errors: both must be rolled back from the
    /// effect log, not left for some later unrelated drain to silently apply.
    ///
    /// Fail oracle: without `SteelCtx::pop_effect_marks` truncating on
    /// failure, `effects_for_test()` comes back non-empty.
    #[test]
    fn queued_effects_before_failure_are_rolled_back() {
        let dir = TempDir::new().unwrap();
        let path = write_plugin(
            &dir,
            "effects.scm",
            r#"(register-lsp-server! "rust" #:command "rust-analyzer")
               (%define-language! "foo" '() '() '())
               (error "intentional mid-body error")"#,
        );
        let id = plugin_id("core:effects");
        let mut host = ScriptingHost::new();
        host.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Declared { path });

        let result = host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins());

        assert!(result.is_err(), "activation must fail on intentional error");
        assert!(
            host.effects_for_test().is_empty(),
            "failed activation must not leave a queued LSP server op or language registration behind"
        );
    }

    // ── Committed-effects salvage across enclosing eval failure ──────────────

    /// A command dispatched via `call_steel_cmd` `call!`s a lazy command owned
    /// by plugin B; B activates inline mid-body and finishes successfully
    /// (queuing `register-lsp-server!` and committing `Loaded`), then the
    /// outer command errors afterward. B's committed effect must survive —
    /// discarding it while B stays permanently `Loaded` would mean its LSP
    /// server never registers (activation is one-shot). Effects the outer
    /// command itself queued, before and after the nested activation, must
    /// NOT survive.
    ///
    /// Fail oracle: revert `take_eval_effects`'s `Err` arm to a flat
    /// `self.effects.truncate(effects_start)` → `e.effects` comes back empty
    /// even though B is `Loaded`.
    #[test]
    fn committed_activation_effects_survive_failed_outer_command() {
        use crate::host::EditorHost;
        use crate::null_host::LazyStubHost;
        use crate::types::{Effect, PendingLspServerOp};

        let dir = TempDir::new().unwrap();
        let path = write_plugin(
            &dir,
            "b.scm",
            r#"(register-lsp-server! "b-lang" #:command "b-lsp")
               (define-command! "b-cmd" "doc" (lambda () 0))"#,
        );
        let id_b = plugin_id("core:b");
        let mut host = ScriptingHost::new();
        host.registries
            .lazy_registry
            .plugins
            .insert(id_b.clone(), PluginState::Declared { path });

        let mut editor_host = LazyStubHost::default();
        editor_host
            .commands()
            .register_lazy_command("b-cmd", &id_b)
            .expect("stub claim must succeed on a fresh host");

        host.eval_source(
            r#"(define-command! "outer-a" "doc"
                 (lambda ()
                   (register-lsp-server! "before" #:command "x")
                   (call! "b-cmd")
                   (register-lsp-server! "after" #:command "y")
                   (error "intentional outer failure")))"#,
            &mut editor_host,
        )
        .expect("defining outer-a must not error");

        let result = host.call_steel_cmd(
            "outer-a",
            None,
            vec![],
            hume_engine::pipeline::PaneId::default(),
            hume_engine::pipeline::BufferId::default(),
            &mut editor_host,
        );

        let err = result.expect_err("outer-a's intentional error must propagate");
        assert!(
            err.message.contains("intentional outer failure"),
            "got: {}",
            err.message
        );
        assert_eq!(
            err.effects.len(),
            1,
            "only B's committed register-lsp-server! must survive; got: {:?}",
            err.effects
        );
        assert!(
            matches!(
                &err.effects[0],
                Effect::LspServerOp(PendingLspServerOp::Register(reg)) if reg.language == "b-lang"
            ),
            "surviving effect must be B's 'b-lang' registration, not 'before'/'after'; got: {:?}",
            err.effects[0]
        );
        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id_b),
                Some(PluginState::Loaded)
            ),
            "B must be Loaded — its activation succeeded before A's own failure"
        );
        assert!(
            host.effects_for_test().is_empty(),
            "the effect log must be fully drained after take_eval_effects"
        );
    }

    /// One level deeper: plugin C activates successfully inside plugin B's
    /// body (via `call!` to a command C owns), and B then fails. C's
    /// committed `register-lsp-server!` must survive B's own rollback — C is
    /// `Loaded` and its effect is irreversible-by-omission, same reasoning as
    /// the outer-command case above, but exercised through nested
    /// `pop_effect_marks` calls instead of `take_eval_effects` alone.
    #[test]
    fn nested_activation_commit_survives_enclosing_plugin_failure() {
        use crate::host::EditorHost;
        use crate::null_host::LazyStubHost;
        use crate::types::{Effect, PendingLspServerOp};

        let dir = TempDir::new().unwrap();
        let path_c = write_plugin(
            &dir,
            "c.scm",
            r#"(register-lsp-server! "c-lang" #:command "c-lsp")
               (define-command! "c-cmd" "doc" (lambda () 0))"#,
        );
        let path_b = write_plugin(
            &dir,
            "b.scm",
            r#"(register-lsp-server! "b-lang" #:command "b-lsp")
               (call! "c-cmd")
               (error "b fails")"#,
        );
        let id_b = plugin_id("core:b");
        let id_c = plugin_id("core:c");
        let mut host = ScriptingHost::new();
        host.registries
            .lazy_registry
            .plugins
            .insert(id_b.clone(), PluginState::Declared { path: path_b });
        host.registries
            .lazy_registry
            .plugins
            .insert(id_c.clone(), PluginState::Declared { path: path_c });

        let mut editor_host = LazyStubHost::default();
        editor_host
            .commands()
            .register_lazy_command("c-cmd", &id_c)
            .expect("stub claim must succeed on a fresh host");

        let result = host.activate_plugin_inline(&id_b, 10_000, &mut editor_host, &no_builtins());

        let err = result.expect_err("B's intentional error must propagate");
        assert!(err.message.contains("b fails"), "got: {}", err.message);
        assert_eq!(
            err.effects.len(),
            1,
            "only C's committed register-lsp-server! must survive; got: {:?}",
            err.effects
        );
        assert!(
            matches!(
                &err.effects[0],
                Effect::LspServerOp(PendingLspServerOp::Register(reg)) if reg.language == "c-lang"
            ),
            "surviving effect must be C's 'c-lang' registration, not B's 'b-lang'; got: {:?}",
            err.effects[0]
        );
        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id_b),
                Some(PluginState::Failed)
            ),
            "B must be Failed"
        );
        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id_c),
                Some(PluginState::Loaded)
            ),
            "C must be Loaded — its activation succeeded before B's own failure"
        );
        assert!(
            host.registries.command_table.contains_key("c-cmd"),
            "C's command must remain registered — C is Loaded, not rolled back"
        );
        assert!(
            host.effects_for_test().is_empty(),
            "the effect log must be fully drained after take_eval_effects"
        );
    }

    // ── G4: self-ownership exemption ──────────────────────────────────────────

    /// A lazy plugin is allowed to call `define-command!` for its own activation
    /// command inside its body, even though that name is still claimed as its
    /// `Lazy` stub at the time (the stub is only removed by
    /// `unregister_lazy_stubs_of` *after* the body completes in
    /// `finish_lazy_activation`).
    ///
    /// Fail oracle: remove the `is_self` exemption from `define_command_inner` →
    /// the plugin's `define-command!` call is rejected → activation returns Err.
    #[test]
    fn lazy_plugin_can_define_its_own_activation_command() {
        use crate::host::EditorHost;
        use crate::null_host::LazyStubHost;

        let dir = TempDir::new().unwrap();
        let path = write_plugin(
            &dir,
            "self-act.scm",
            r#"(define-command! "self-act-cmd" "doc" (lambda () 0))"#,
        );
        let id = plugin_id("core:self-act");
        let mut host = ScriptingHost::new();
        host.registries
            .lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Declared { path });

        // Simulate declare-plugin having claimed self-act-cmd as the
        // activation entry — now tracked in the editor's registry (here,
        // the stateful test host), not a scripting-crate map.
        let mut editor_host = LazyStubHost::default();
        editor_host
            .commands()
            .register_lazy_command("self-act-cmd", &id)
            .expect("stub claim must succeed on a fresh host");

        let result = host.activate_plugin_inline(&id, 10_000, &mut editor_host, &no_builtins());

        assert!(
            result.is_ok(),
            "plugin defining its own activation command must activate successfully; got: {result:?}"
        );
        assert!(
            matches!(
                host.registries.lazy_registry.plugins.get(&id),
                Some(PluginState::Loaded)
            ),
            "plugin must be Loaded after activation"
        );
        assert!(
            host.registries.command_table.contains_key("self-act-cmd"),
            "self-act-cmd must be in command_table after activation"
        );
    }
}
