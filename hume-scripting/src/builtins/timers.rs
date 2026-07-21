//! `(after ms thunk)` / `(cancel-timer! id)` — Steel timer surface.
//! Not LSP-specific (any plugin can debounce/delay work), hence a sibling
//! module rather than living in `lsp.rs`.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;

use super::args::usize_arg;

type SteelResult = Result<SteelVal, SteelErr>;

/// `(after ms thunk)` → timer id (int). `thunk` is called with no args at
/// the drain boundary once `ms` milliseconds have passed (never inline —
/// same queued-Steel-call delivery as the LSP callbacks).
pub(crate) fn after(ctx: &mut SteelCtx, ms: SteelVal, thunk: SteelVal) -> SteelResult {
    let ms = usize_arg(ms, "after")? as u64;
    match ctx.host.timers().and_then(|t| t.schedule_timer(ms, thunk)) {
        Some(id) => Ok(SteelVal::IntV(id as isize)),
        None => steel::stop!(Generic => "after: no timer support in this context"),
    }
}

/// `(cancel-timer! id)` → void. Idempotent: a no-op if `id` already fired,
/// was already cancelled, or never existed.
pub(crate) fn cancel_timer(ctx: &mut SteelCtx, id: SteelVal) -> SteelResult {
    let id = usize_arg(id, "cancel-timer!")? as u64;
    if let Some(timers) = ctx.host.timers() {
        timers.cancel_timer(id);
    }
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests;
