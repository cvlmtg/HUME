//! `%stdout-gate!` — the Rust half of HUME's gated print builtins.
//!
//! steel-core's `displayln`/`display`/`print`/`println`/`newline`/`write`/
//! `write-string`/`write-char`/`simple-display`/`simple-displayln` all write
//! to the real process stdout by default. Calling any of them while HUME's
//! alt-screen TUI owns the terminal would corrupt the rendered frame.
//!
//! A plain top-level shadow (`(define displayln …)` or `register_value`)
//! only shadows the name in its own compilation unit — these ten names are
//! prelude exports, and steel-core prepends its prelude source to every
//! compiled unit including each `(require "path.scm")` plugin file, so every
//! plugin unit still imports the original straight from the prelude. Fix:
//! HUME appends gated redefinitions of all ten names to steel-core's own
//! prelude string via `Engine::set_prelude_string` (see
//! `builtins/mod.rs::register_all`), so the shims shadow the imports inside
//! every plugin module too.
//!
//! Two steel-core 0.8.2 limitations shaped how the shims are built, both
//! verified empirically:
//! - One `compile_and_run_raw_program` call can't both capture a name's
//!   current value and redefine that same name in the same unit (rejected at
//!   compile time) — `register_all` runs BOOTSTRAP (captures originals) and
//!   `PRINT_GATE_SHIMS` (redefines the names) as two separate sequential
//!   calls, so by the second the names are ordinary bound globals.
//! - A required module can't call a locally-shadowed prelude name with 2+
//!   positional args (e.g. explicit-port `(display obj port)`) when the
//!   shim's parameter list is *mixed* fixed-plus-rest — reproduced
//!   independent of naming, even with `case-lambda`. Every shim below uses a
//!   *rest-only* list instead, which dodges it for the implicit 0/1-arg form
//!   (the actual plugin use case). Residual gap: a plugin calling one of
//!   these names with an explicit port from inside its own required-module
//!   body still hits the limitation — no workaround short of patching
//!   steel-core, and no real HUME code does this today.
//!
//! Explicit-port calls are gated too, not just forwarded: `port` can itself
//! be `(current-output-port)` (or the real stdout port via steel-core's own
//! error printer), exactly as unsafe as the implicit-port case.
//! [`stdout_gate`]'s Scheme-side caller, `%port-safe?`, checks the *supplied*
//! port's identity against the captured real stdout port, so a custom port
//! (string port, pipe) always passes through ungated. `write-string`/
//! `write-char` need the gate too even though steel-core's natives ignore
//! `(current-output-port)` in their 1-arg form — the shim explicitly threads
//! it through so redirection (`with-output-to-string`) works.
//!
//! This module provides only the gate check itself — [`stdout_gate`],
//! registered as `%stdout-gate!` — called by each Scheme shim before it
//! forwards to the captured original. See `PRINT_GATE_SHIMS` in
//! `builtins/mod.rs` for the shim definitions.

use steel::rvals::SteelVal;

use crate::SteelCtx;

use super::SteelResult;
use super::errors::generic_err;

/// Whether the host reports an inline-output bracket currently live —
/// read fresh from `ctx.host` on every call rather than a value cached at
/// session start, so a bracket a `call!`-armed nested command opens mid-body
/// (`OutputHost::arm_inline_output`) is visible to the very next print.
fn is_inline_output_command(ctx: &mut SteelCtx) -> bool {
    ctx.host
        .output()
        .is_some_and(|output| output.is_inline_output_command())
}

/// Whether it is currently safe to write directly to the real process
/// stdout: init (before the alt-screen TUI is up) or an `#:inline-output`
/// command body (alt-screen temporarily left). `inline` is
/// [`is_inline_output_command`]'s result, hoisted by the caller rather than
/// read again here — `stdout_gate` needs the same fact twice (whether it's
/// safe at all, then whether to enter the alt-screen), and each read is a
/// non-devirtualizable hop through `ctx.host`.
fn stdout_is_safe(ctx: &SteelCtx, inline: bool) -> bool {
    ctx.session == crate::context::EvalSession::Init || inline
}

/// `(%stdout-gate!)` — called by each gated print shim (see
/// `PRINT_GATE_SHIMS` in `builtins/mod.rs`) immediately before it would write
/// to the real stdout. Returns `#f` (write must be suppressed) unless
/// [`stdout_is_safe`]. When safe via [`is_inline_output_command`] specifically
/// (not the init session, which prints pre-terminal with no bracket to open),
/// lazily enters the alt-screen bracket on this, the first real write of the
/// command body.
pub(crate) fn stdout_gate(ctx: &mut SteelCtx) -> SteelResult {
    let inline = is_inline_output_command(ctx);
    if !stdout_is_safe(ctx, inline) {
        return Ok(SteelVal::BoolV(false));
    }
    if inline && let Some(output) = ctx.host.output() {
        output
            .ensure_inline_output_screen()
            .map_err(|e| generic_err(format!("print: {e}")))?;
    }
    Ok(SteelVal::BoolV(true))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
