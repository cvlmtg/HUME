//! `(after! ms thunk)` / `(cancel-timer! id)`: Steel timer surface.
//! Not LSP-specific (any plugin can debounce/delay work), hence a sibling
//! module rather than living in `lsp.rs`.

use steel::rvals::SteelVal;

use crate::SteelCtx;

use super::SteelResult;
use super::args::{timer_arg, usize_arg};
use super::errors::require_cap;

/// `(after! ms thunk)` → a timer for `cancel-timer!`. `thunk` is called with no args at
/// the drain boundary once `ms` milliseconds have passed (never inline;
/// same queued-Steel-call delivery as the LSP callbacks).
pub(crate) fn after(ctx: &mut SteelCtx, ms: SteelVal, thunk: crate::Rooted) -> SteelResult {
    let ms = usize_arg(ms, "ms")? as u64;
    match require_cap(ctx.host.timers())?.schedule_timer(ms, thunk) {
        Some(timer) => Ok(timer.into_steel_val()),
        None => steel::stop!(Generic => "no timer wheel in this context"),
    }
}

/// `(cancel-timer! timer)` → void. The thunk of the `after!` timer never
/// runs, even one already due. Idempotent: a no-op once the thunk has run
/// or after an earlier cancel. Raises for anything but a timer.
pub(crate) fn cancel_timer(ctx: &mut SteelCtx, timer: SteelVal) -> SteelResult {
    let timer = timer_arg(&timer, "timer")?;
    require_cap(ctx.host.timers())?.cancel_timer(timer);
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests;
