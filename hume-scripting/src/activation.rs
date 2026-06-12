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
//! - Event/language triggers: `activate_plugin_inline` (Rust) bounces into
//!   `(%activate-plugin-inline id)` via `run_steel`, using its own watchdog.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use steel::rvals::SteelVal;

use crate::attribution;
use crate::codegen::HUME_CTX;
use crate::context::SteelCtx;
use crate::host::EditorHost;
use crate::watchdog::EvalWatchdog;
use crate::ScriptingHost;

// ── run_steel ──────────────────────────────────��──────────────────────────────

/// Arm the watchdog, run `program` inside `steel` with `ctx` visible as
/// `*hume.ctx*`, then cancel the watchdog and reset the interrupt flag.
///
/// Used by `eval_source_raw`, `call_steel_cmd`, and `fire_hook` to avoid
/// repeating the same arm / eval / cancel / reset ceremony in each entry point.
pub(crate) fn run_steel<'a>(
    steel: &mut steel::steel_vm::engine::Engine,
    ctx: &mut SteelCtx<'a>,
    program: String,
    budget_ms: u64,
) -> Result<(), String> {
    let watchdog = EvalWatchdog::arm(
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
            let res = steel.compile_and_run_raw_program(program);
            steel.update_value(HUME_CTX, SteelVal::Void);
            res
        })
        .map(|_| ())
        .map_err(|e| e.to_string());
    watchdog.cancel();
    ctx.interrupt_flag.store(false, Ordering::Relaxed);
    result
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
    pub(crate) fn eval_source_raw(
        &mut self,
        source: String,
        builtin_names: HashSet<String>,
        budget_ms: u64,
        host: &mut dyn EditorHost,
    ) -> Result<(), String> {
        let (steel, bundle) = self.steel_and_bundle();
        let mut steel_ctx = SteelCtx::new_init(host, bundle, builtin_names);
        run_steel(steel, &mut steel_ctx, source, budget_ms)
    }

    /// Activate a plugin inline via `%activate-plugin-inline` using `run_steel`.
    ///
    /// Used by event- and language-trigger paths (`activate_and_register`) that
    /// fire outside any running eval and need their own watchdog.  The plugin body
    /// runs inside `hm.eval-string` (VM-aware, no `&mut Engine` borrow), sharing
    /// the same `ctx.registries` as any concurrent eval.  `define-command!` calls
    /// inside the body register directly into `host.register_command` inline.
    pub fn activate_plugin_inline(
        &mut self,
        id: &attribution::PluginId,
        budget_ms: u64,
        host: &mut dyn EditorHost,
        builtin_names: &HashSet<String>,
    ) -> Result<(), String> {
        let program = format!(r#"(%activate-plugin-inline "{id}")"#);
        let (steel, bundle) = self.steel_and_bundle();
        let mut steel_ctx = SteelCtx::new_activation(host, bundle, builtin_names.clone());
        run_steel(steel, &mut steel_ctx, program, budget_ms)
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
        host.registries.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Declared { path });

        host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins()).unwrap();

        assert!(
            host.registries.command_table.contains_key("test-cmd"),
            "command must be in command_table after activation"
        );
        assert!(
            matches!(host.registries.lazy_registry.plugins.get(&id), Some(PluginState::Loaded)),
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
        host.registries.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Declared { path });

        let result = host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins());

        assert!(result.is_err(), "must return Err on syntax error");
        assert!(
            matches!(host.registries.lazy_registry.plugins.get(&id), Some(PluginState::Failed)),
            "plugin must be in Failed state after syntax error"
        );
    }

    // ── Case 3: Idempotent no-ops for non-Declared states ────────────────────

    #[test]
    fn already_loaded_is_noop() {
        let id = plugin_id("core:loaded");
        let mut host = ScriptingHost::new();
        host.registries.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loaded);

        host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins()).unwrap();

        assert!(
            matches!(host.registries.lazy_registry.plugins.get(&id), Some(PluginState::Loaded)),
            "state must remain Loaded"
        );
    }

    #[test]
    fn already_failed_is_noop() {
        let id = plugin_id("core:failed");
        let mut host = ScriptingHost::new();
        host.registries.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Failed);

        host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins()).unwrap();

        assert!(
            matches!(host.registries.lazy_registry.plugins.get(&id), Some(PluginState::Failed)),
            "state must remain Failed"
        );
    }

    #[test]
    fn absent_plugin_is_noop() {
        let id = plugin_id("core:absent");
        let mut host = ScriptingHost::new();

        host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins()).unwrap();

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
        host.registries.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loading);

        host.activate_plugin_inline(&id, 10_000, &mut NullHost, &no_builtins()).unwrap();

        assert!(
            matches!(host.registries.lazy_registry.plugins.get(&id), Some(PluginState::Loading)),
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
            matches!(host.registries.lazy_registry.plugins.get(&id), Some(PluginState::Failed)),
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
        // init mode, so is_init=true allows define-command!.
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
    /// increments `activation_depth`, and returns the require-string.
    #[test]
    fn begin_lazy_activation_declared_returns_require_string() {
        let dir = TempDir::new().unwrap();
        let path = write_plugin(&dir, "p.scm", r#"(define-command! "p-cmd" "doc" (lambda () 0))"#);
        let id = plugin_id("core:p");
        let mut host = ScriptingHost::new();
        host.registries.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Declared { path: path.clone() });

        let program = r#"(define result (%begin-lazy-activation "core:p"))"#;
        host.eval_source(program, &mut NullHost).unwrap();

        // Plugin must be in Loading state.
        assert!(
            matches!(host.registries.lazy_registry.plugins.get(&id), Some(PluginState::Loading)),
            "Declared plugin must be Loading after %begin-lazy-activation"
        );
        // activation_depth must be incremented.
        assert_eq!(host.registries.activation_depth, 1, "activation_depth must be 1");
    }

    /// `%begin-lazy-activation` on a `Loading` plugin returns `#f` (cycle guard).
    #[test]
    fn begin_lazy_activation_loading_returns_false() {
        let id = plugin_id("core:cycling");
        let mut host = ScriptingHost::new();
        host.registries.lazy_registry
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
            matches!(host.registries.lazy_registry.plugins.get(&id), Some(PluginState::Loading)),
            "state must remain Loading (cycle guard)"
        );
    }

    /// `%finish-lazy-activation` with success=true transitions to `Loaded`.
    #[test]
    fn finish_lazy_activation_success_transitions_to_loaded() {
        let id = plugin_id("core:finishing");
        let mut host = ScriptingHost::new();
        host.registries.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loading);
        host.registries.activation_depth = 1;

        let program = r#"(%finish-lazy-activation "core:finishing" #t)"#;
        host.eval_source(program, &mut NullHost).unwrap();

        assert!(
            matches!(host.registries.lazy_registry.plugins.get(&id), Some(PluginState::Loaded)),
            "plugin must be Loaded after successful finish"
        );
        assert_eq!(host.registries.activation_depth, 0, "activation_depth must return to 0");
    }

    /// `%finish-lazy-activation` with success=false transitions to `Failed`.
    #[test]
    fn finish_lazy_activation_failure_transitions_to_failed() {
        let id = plugin_id("core:failing");
        let mut host = ScriptingHost::new();
        host.registries.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loading);
        host.registries.activation_depth = 1;

        let program = r#"(%finish-lazy-activation "core:failing" #f)"#;
        host.eval_source(program, &mut NullHost).unwrap();

        assert!(
            matches!(host.registries.lazy_registry.plugins.get(&id), Some(PluginState::Failed)),
            "plugin must be Failed after failed finish"
        );
    }
}
