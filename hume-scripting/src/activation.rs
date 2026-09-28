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
//! - `load-plugin!` (init.scm): the BOOTSTRAP Scheme wrapper calls `%load-plugin!`
//!   (declare/record) then `%activate-plugin-inline` (inline body eval via `hm.eval-string`).
//! - Lazy keypress dispatch: `%dispatch-command` activates the owner inline on a
//!   `command_table` miss, then retries.
//! - Event/language activations: `activate_plugin_inline` (Rust) bounces into
//!   `(%activate-plugin-inline id)` via `run_steel_call` (a direct function
//!   call, not source), using the `ScriptingHost`'s one persistent watchdog.

use rustc_hash::FxHashSet;
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
/// `*hume.ctx*`, then cancel the watchdog, reset the interrupt flag, and
/// truncate any unrestored inline-output frame back to zero
/// (`OutputHost::truncate_inline_output`): the backstop for a `call!`-armed
/// bracket (`bootstrap.scm`'s `%apply-command`) whose body raised before
/// reaching its matching restore; see that function's own comment in
/// `builtins/mod.rs`'s BOOTSTRAP block for why it isn't paired via
/// `with-handler` instead. A no-op whenever every arm this session was
/// already paired, the common case.
///
/// Exists so the arm / eval / cancel / reset / truncate ceremony lives in
/// one place: the one boundary every Steel entry point passes through
/// regardless of outcome.
pub(crate) fn run_steel_session<'a, R>(
    steel: &mut Engine,
    watchdog: &EvalWatchdog,
    ctx: &mut SteelCtx<'a>,
    budget_ms: u64,
    body: impl FnOnce(&mut Engine) -> Result<R, SteelErr>,
) -> Result<(), SessionError> {
    watchdog.arm(
        Arc::clone(&ctx.interrupt_flag),
        std::time::Duration::from_millis(budget_ms),
    );
    // Bound to a `let` (rather than chained straight onto `.map`/`.map_err`)
    // so the `LifetimeGuard` `with_mut_reference` returns is fully dropped
    // (ending its exclusive borrow of `steel`) before `describe_steel_error`
    // below needs `steel` back for an immutable read.
    let raw_result: Result<R, SteelErr> = steel
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
        });
    watchdog.cancel();
    // Read before the reset below, so a caller distinguishing a cooperative
    // watchdog interrupt (`hume/yield!`) from an ordinary Steel error (see
    // `SessionError`'s own doc) sees this session's own flag, not whatever
    // the next session sets it to.
    let interrupted = ctx.interrupt_flag.swap(false, Ordering::Relaxed);
    if let Some(output) = ctx.host.output() {
        output.truncate_inline_output(0);
    }

    // A plugin activation contained mid-session (see `finish_lazy_activation`/
    // `finish_manifest_declare` in `builtins/plugins.rs`) never surfaces as
    // this session's own `Err`. It's reported here instead, once per
    // failure, through the same `pending_messages` sink every other `log!`
    // and soft-error uses, so it reaches the user via the ordinary message
    // pipeline regardless of which entry point (`init.scm`, a command body, a
    // runtime activation) this session came from.
    for (id, err) in std::mem::take(&mut ctx.failed_activations) {
        ctx.log(
            crate::log::LogLevel::Error,
            format!(
                "plugin '{id}' failed to load: {}",
                describe_steel_error(steel, &err)
            ),
        );
    }

    raw_result.map(|_| ()).map_err(|e| SessionError {
        message: describe_steel_error(steel, &e),
        interrupted,
    })
}

/// A failed [`run_steel_session`]: the reported message, plus whether the
/// failure was the watchdog's own cooperative interrupt (`hume/yield!`,
/// raised when the step budget is exceeded) rather than an ordinary Steel
/// error (a raised exception, a stale-buffer `LivePane` raise, an arity
/// mismatch). `interrupted` is read off `ctx.interrupt_flag` before this
/// session resets it. A caller that only wants the message (most of them)
/// reads `.message` and ignores it; [`crate::ScriptingHost::run_steel_calls`]
/// is the one caller that needs to know which.
pub(crate) struct SessionError {
    pub(crate) message: String,
    pub(crate) interrupted: bool,
}

/// Render a `SteelErr` with its source location when one is available,
/// resolving the error's span against the engine's own `Sources` map (every
/// `(require "<path>")`'d file, including a plugin's `plugin.scm`/
/// `manifest.scm`, is registered there by Steel's module loader) and
/// formatting it with `SteelErr::emit_result_to_string`, the same snippet
/// renderer Steel's own top-level error reporting uses.
///
/// Deliberately not `Engine::raise_error_to_string`: that also walks the
/// live call stack and prepends a `note:` block per frame. That is useful for a
/// script author debugging their own Steel code interactively, but here the
/// frames are `bootstrap.scm`'s own activation plumbing (an empty file name,
/// since it's compiled from a string, not a real path) and Steel's
/// `with-handler` macro expansion itself, neither of which means anything to
/// someone reading a plugin-load failure. Falls back to the bare `Display`
/// message for a `SteelErr` with no resolvable span (e.g. one raised
/// directly by a Rust builtin via `steel::stop!` with no span set).
fn describe_steel_error(steel: &Engine, err: &SteelErr) -> String {
    (|| {
        let span = err.span()?;
        let source_id = span.source_id()?;
        let content = steel.get_source(&source_id)?;
        let file_name = steel
            .get_path_for_source_id(&source_id)
            .map(|p| hume_platform::path::display_form(&p))
            .unwrap_or_default();
        Some(err.emit_result_to_string(&file_name, &content))
    })()
    .unwrap_or_else(|| err.to_string())
}

/// [`run_steel_session`] with a source-program body: parse + compile + run.
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
    .map_err(|e| e.message)
}

/// [`run_steel_session`] with a direct function-call body.
///
/// Calls the global Steel function `fn_name` with `args`: no source string,
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
    .map_err(|e| e.message)
}

// ── ScriptingHost — activation impl ──────────────────────────────────────────

impl ScriptingHost {
    /// Core eval machinery.
    ///
    /// Evaluates `source` (init.scm) synchronously.  `(load-plugin! …)` calls
    /// inside the source activate their plugin bodies inline via the BOOTSTRAP
    /// `%activate-plugin-inline` helper (VM-aware `hm.eval-string`, no
    /// `&mut Engine` borrow).  `(define-command! …)` calls register commands
    /// directly into the editor's `CommandRegistry` via `host.register_command`.
    ///
    /// Returns the effects this eval queued (atomically on success; on error,
    /// only effects committed by a nested successful plugin activation; see
    /// `ScriptingHost::take_eval_effects`).
    pub(crate) fn eval_source_raw(
        &mut self,
        source: String,
        builtin_names: FxHashSet<String>,
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
    /// Used by event- and language-activation paths that
    /// fire outside any running eval and need their own watchdog.  The plugin body
    /// runs inside `hm.eval-string` (VM-aware, no `&mut Engine` borrow), sharing
    /// the same `ctx.registries` as any concurrent eval.  `define-command!` calls
    /// inside the body register directly into `host.register_command` inline.
    ///
    /// Returns the activating body's queued effects (`register-lsp-server!`,
    /// `set-buffer-language!`, etc.) so the caller can apply them immediately.
    /// Otherwise they'd sit unapplied until some unrelated later drain, which can
    /// skip attaching the very buffer that triggered this activation.
    ///
    /// A failed activation's own effects are already rolled back by the
    /// BOOTSTRAP `%activate-plugin-inline` wrapper's `%begin-lazy-activation`/
    /// `%finish-lazy-activation` mark/pop (`ctx.pop_effect_marks`) before the
    /// error reaches here, except any effects committed by a nested
    /// successful plugin activation, which `pop_effect_marks` keeps and which
    /// `take_eval_effects` salvages onto the returned `EvalError`.
    pub fn activate_plugin_inline(
        &mut self,
        id: &attribution::PluginId,
        budget_ms: u64,
        host: &mut dyn EditorHost,
        builtin_names: &FxHashSet<String>,
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

#[cfg(test)]
mod tests;
