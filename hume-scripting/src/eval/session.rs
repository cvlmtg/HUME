//! The session every eval passes through: the watchdog armed around it,
//! `*hume.ctx*` bound for its builtins, and what it reports when it ends.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;
use steel::steel_vm::engine::Engine;

use crate::HUME_CTX;
use crate::context::SteelCtx;
use crate::watchdog::EvalWatchdog;

/// Arm the watchdog, run `body` against `steel` with `ctx` visible as
/// `*hume.ctx*`, then cancel the watchdog, reset the interrupt flag, and
/// truncate any unrestored inline-output frame back to zero
/// (`OutputHost::truncate_inline_output`): the backstop for a `call!`-armed
/// bracket (`bootstrap.scm`'s `%apply-command!`) whose body raised before
/// reaching its matching restore; see that function's own comment in
/// `builtins/mod.rs`'s BOOTSTRAP block for why it isn't paired via
/// `with-handler` instead. A no-op whenever every arm this session was
/// already paired, the common case.
///
/// Exists so the arm / eval / cancel / reset / truncate ceremony lives in
/// one place: the one boundary every Steel entry point passes through
/// regardless of outcome.
pub(super) fn run_steel_session<'a, R>(
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
    // `finish_manifest_load` in `builtins/plugins/activation.rs`) never surfaces as
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
/// reads `.message` and ignores it; [`crate::ScriptingHost::run_steel_calls`] is
/// the one caller that needs to know which.
pub(super) struct SessionError {
    pub(super) message: String,
    pub(super) interrupted: bool,
}

/// Render a `SteelErr` with its source location when one is available,
/// resolving the error's span against the engine's own `Sources` map (every
/// `(require "<path>")`'d file, including a plugin's `plugin.scm`/
/// `manifest.scm`, is registered there by Steel's module loader) and
/// formatting it with `SteelErr::emit_result_to_string`, the same snippet
/// renderer Steel's own top-level error reporting uses.
///
/// Not `Engine::raise_error_to_string`: that also walks the
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
