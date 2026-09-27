//! `%stdout-gate!`: the Rust half of HUME's gated print builtins.
//!
//! steel-core's ten print names (`display`, `displayln`, `write-string`, ...)
//! write to the real stdout, which would corrupt the alt-screen TUI. They are
//! prelude exports, and steel-core prepends its prelude to every compiled unit
//! including each required plugin file, so a top-level shadow doesn't reach
//! plugins. HUME instead appends gated redefinitions (`PRINT_GATE_SHIMS` in
//! `builtins/mod.rs`) to the prelude string via `Engine::set_prelude_string`.
//!
//! Two steel-core limitations shape the shims:
//! - One unit can't both capture a name's value and redefine it, so
//!   `register_all` runs BOOTSTRAP (captures originals) and `PRINT_GATE_SHIMS`
//!   as two separate calls (still rejected on 0.8.3).
//! - A required module can't call a shadowed prelude name with 2+ positional
//!   args when the shim mixes fixed and rest parameters (seen on 0.8.2), so
//!   every shim is rest-only. An explicit-port call from inside a required
//!   module may still hit this; no HUME code does that.
//!
//! Explicit-port calls are gated too, since the port can be the real stdout.
//! `%port-safe?` compares the supplied port against the captured stdout, so
//! string ports and pipes pass through ungated.

use steel::rvals::SteelVal;

use crate::SteelCtx;

use super::SteelResult;
use super::errors::generic_err;

/// Whether the host reports an inline-output bracket currently live,
/// read fresh from `ctx.host` on every call rather than a value cached at
/// session start, so a bracket a `call!`-armed nested command opens mid-body
/// (`OutputHost::arm_inline_output`) is visible to the very next print.
fn is_inline_output_command(ctx: &mut SteelCtx) -> bool {
    ctx.host
        .output()
        .is_some_and(|output| output.is_inline_output_command())
}

/// `(%stdout-gate!)`: called by each gated print shim (see
/// `PRINT_GATE_SHIMS` in `builtins/mod.rs`) immediately before it would write
/// to the real stdout. Returns `#f` (write must be suppressed) unless it's
/// currently safe to write directly to the real process stdout: init (before
/// the alt-screen TUI is up) or an `#:inline-output` command body (alt-screen
/// temporarily left). When safe via [`is_inline_output_command`] specifically
/// (not the init session, which prints pre-terminal with no bracket to open),
/// lazily enters the alt-screen bracket on this, the first real write of the
/// command body.
///
/// `inline` is read once and reused for both checks below (safe-at-all, then
/// whether to enter the alt-screen) rather than re-read, since each read is a
/// non-devirtualizable hop through `ctx.host`.
///
/// The two safe reasons are joined by `||`. Each is pinned on its own by
/// `stdout_gate_returns_true_and_skips_ensure_when_open_via_init_session_only`
/// and `stdout_gate_returns_true_and_calls_ensure_when_open_via_inline_output_command`
/// below, so a guard of `session != Init || !inline` fails one of them.
pub(crate) fn stdout_gate(ctx: &mut SteelCtx) -> SteelResult {
    let inline = is_inline_output_command(ctx);
    if ctx.session != crate::context::EvalSession::Init && !inline {
        return Ok(SteelVal::BoolV(false));
    }
    if inline && let Some(output) = ctx.host.output() {
        output
            .ensure_inline_output_screen()
            .map_err(|e| generic_err(format!("print: {e}")))?;
    }
    Ok(SteelVal::BoolV(true))
}

#[cfg(test)]
mod tests;
