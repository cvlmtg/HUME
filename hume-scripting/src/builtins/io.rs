//! `%stdout-gate!` — the Rust half of HUME's gated print builtins.
//!
//! steel-core's `displayln`/`display`/`print`/`println`/`newline`/`write`/
//! `write-string`/`write-char`/`simple-display`/`simple-displayln` all write
//! to the real process stdout by default. Calling any of them while HUME's
//! alt-screen TUI owns the terminal would corrupt the rendered frame.
//!
//! Why a plain top-level shadow misses plugin code: these ten names are
//! prelude exports, and steel-core prepends its prelude source to *every*
//! compiled unit, including each `(require "path.scm")` plugin file (a
//! separate compilation unit from HUME's top level). A top-level rebind —
//! `(define displayln …)` or `register_value`, in any form scoped to one
//! unit — only shadows the name in that unit; every plugin unit still
//! imports the original straight from the prelude.
//!
//! Fix: HUME appends its own gated redefinitions of all ten names to
//! steel-core's prelude string itself via `Engine::set_prelude_string` (see
//! `builtins/mod.rs::register_all`). Landing in every unit's prelude means
//! the shims shadow the imports inside every plugin module too.
//!
//! Two steel-core 0.8.2 wrinkles, both verified empirically:
//! - A single `compile_and_run_raw_program` call cannot both capture a
//!   name's current value and redefine that same name in the same unit
//!   (rejected at compile time). `register_all` therefore runs BOOTSTRAP
//!   (captures the originals) and `PRINT_GATE_SHIMS` (redefines the ten
//!   names) as two separate sequential calls — by the second, the names are
//!   ordinary already-bound globals, so redefining them is a plain rebind.
//! - A required module's compiled unit cannot contain a call site invoking a
//!   locally-shadowed prelude name with 2+ positional args (e.g. an
//!   explicit-port `(display obj port)`) when that name's shim declares a
//!   *mixed* fixed-plus-rest parameter list — reproduced independent of
//!   naming, even with `case-lambda`. `PRINT_GATE_SHIMS` therefore uses a
//!   *rest-only* parameter list for every shim, which dodges it; the
//!   implicit 0/1-arg form (the actual plugin use case) works everywhere.
//!   Residual gap: a plugin calling one of these names with an **explicit
//!   port argument from inside its own required-module body** still hits
//!   the limitation (no workaround short of patching steel-core; no real
//!   HUME code does this today) — the explicit-port form works everywhere
//!   else (top-level command bodies, `with-output-to-string`, etc).
//!
//! Explicit-port calls are gated too, not just forwarded — `port` can itself
//! be `(current-output-port)` (or the real stdout port via steel-core's own
//! error printer), which is exactly as unsafe as the implicit-port case.
//! [`stdout_gate`]'s Scheme-side caller, `%port-safe?`, checks the *supplied*
//! port's identity against the captured real stdout port, so a custom port
//! (string port, pipe) always passes through ungated. `write-string`/
//! `write-char` need the gate too even though steel-core's natives ignore
//! `(current-output-port)` in their 1-arg form and default straight to real
//! stdout — the shim explicitly threads it through so redirection
//! (`with-output-to-string`) works.
//!
//! This module provides only the gate check itself — [`stdout_gate`],
//! registered as `%stdout-gate!` — called by each Scheme shim before it
//! forwards to the captured original. See `PRINT_GATE_SHIMS` in
//! `builtins/mod.rs` for the shim definitions.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;

use super::errors::generic_err;

/// Whether it is currently safe to write directly to the real process
/// stdout: init (before the alt-screen TUI is up) or an `#:inline-output`
/// command body (alt-screen temporarily left). See `SteelCtx::is_inline_output`.
fn stdout_is_safe(ctx: &SteelCtx) -> bool {
    ctx.is_inline_output || ctx.session == crate::context::EvalSession::Init
}

/// `(%stdout-gate!)` — called by each gated print shim (see
/// `PRINT_GATE_SHIMS` in `builtins/mod.rs`) immediately before it would write
/// to the real stdout. Returns `#f` (write must be suppressed) unless
/// [`stdout_is_safe`]. When safe via `is_inline_output` specifically (not
/// the init session, which prints pre-terminal with no bracket to open),
/// lazily enters the alt-screen bracket on this, the first real write of the
/// command body.
pub(crate) fn stdout_gate(ctx: &mut SteelCtx) -> Result<SteelVal, SteelErr> {
    if !stdout_is_safe(ctx) {
        return Ok(SteelVal::BoolV(false));
    }
    if ctx.is_inline_output
        && let Some(output) = ctx.host.output()
    {
        output
            .ensure_inline_output_screen()
            .map_err(|e| generic_err(format!("print: {e}")))?;
    }
    Ok(SteelVal::BoolV(true))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
