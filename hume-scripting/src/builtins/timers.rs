//! `(after! ms thunk)` / `(cancel-timer! id)`: Steel timer surface.
//! Not LSP-specific (any plugin can debounce/delay work), hence a sibling
//! module rather than living in `lsp.rs`.

use steel::rvals::SteelVal;

use crate::SteelCtx;

use super::SteelResult;
use super::args::usize_arg;
use super::errors::require_cap;

/// `(after! ms thunk)` → timer id (int). `thunk` is called with no args at
/// the drain boundary once `ms` milliseconds have passed (never inline;
/// same queued-Steel-call delivery as the LSP callbacks).
pub(crate) fn after(ctx: &mut SteelCtx, ms: SteelVal, thunk: crate::Rooted) -> SteelResult {
    let ms = usize_arg(ms, "after!")? as u64;
    match require_cap(ctx.host.timers(), "after!")?.schedule_timer(ms, thunk) {
        Some(id) => Ok(SteelVal::IntV(id as isize)),
        None => steel::stop!(Generic => "after!: no timer wheel in this context"),
    }
}

/// `(cancel-timer! id)` → void. The thunk of the `after!` timer `id` never
/// runs, even one already due. Idempotent: a no-op once the thunk has run,
/// after an earlier cancel, or for an id `after!` never returned.
pub(crate) fn cancel_timer(ctx: &mut SteelCtx, id: SteelVal) -> SteelResult {
    let id = usize_arg(id, "cancel-timer!")? as u64;
    require_cap(ctx.host.timers(), "cancel-timer!")?.cancel_timer(id);
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests;
