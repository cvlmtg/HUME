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
mod tests {
    use super::*;
    use crate::test_support::SteelCtxTestHarness;
    use steel::rvals::IntoSteelVal;

    /// `NullHost`'s default `schedule_timer` returns `None` (no timer wheel
    /// to schedule onto) — `after` must surface that as an error, not
    /// silently return a meaningless id.
    #[test]
    fn after_errors_when_the_host_has_no_timer_support() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = after(
            &mut ctx,
            100i64.into_steelval().unwrap(),
            SteelVal::BoolV(true),
        );
        assert!(result.is_err());
    }

    #[test]
    fn after_rejects_a_negative_ms_argument() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = after(
            &mut ctx,
            (-1i64).into_steelval().unwrap(),
            SteelVal::BoolV(true),
        );
        assert!(result.is_err());
    }

    #[test]
    fn cancel_timer_is_a_harmless_no_op_against_nullhost() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = cancel_timer(&mut ctx, 0i64.into_steelval().unwrap());
        assert!(result.is_ok());
    }

    #[test]
    fn after_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        let result = super::super::errors::require_cmd(&h.ctx_init(), "after");
        assert!(result.is_err());
    }
}
