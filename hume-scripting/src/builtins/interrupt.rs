//! Step-budget / interrupt builtin: `hume/yield!`.
//!
//! Cooperative interruption: scripts that want to be interruptible call
//! `(hume/yield!)` regularly (typically inside long loops).  On each call the
//! builtin checks an [`std::sync::atomic::AtomicBool`] shared with
//! [`crate::ScriptingHost`].  If the flag is set, the script is
//! aborted with a Steel error; otherwise execution continues normally.
//!
//! The flag is set by:
//! - The [`EvalWatchdog`](crate::EvalWatchdog) spawned at the start of
//!   each eval (fires after the configured budget; see `steel-init-budget-ms`
//!   and `steel-command-budget-ms`).
//! - Future Ctrl-C handling: the editor can set
//!   [`ScriptingHost::interrupt_flag`](crate::ScriptingHost) when
//!   the user presses Ctrl-C while a script is running.
//!
//! **Limitation:** interruption is cooperative only.  A script without
//! `(hume/yield!)` calls will run to completion regardless of the budget.
//! Steel 0.8.2 does not expose an op-callback hook for involuntary interruption.

use std::sync::atomic::Ordering;

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;

type SteelResult = Result<SteelVal, SteelErr>;

/// `(hume/yield!)` — check the interrupt flag and abort if it is set.
///
/// Call this inside long-running loops to make scripts interruptible:
///
/// ```scheme
/// (let loop ((n 0))
///   (hume/yield!)   ; abort here if the budget is exceeded
///   (do-work n)
///   (loop (+ n 1)))
/// ```
///
/// Returns `#<void>` normally.  Raises a Steel error (aborting the script)
/// when the interrupt flag is set.
pub(crate) fn hume_yield(ctx: &mut SteelCtx) -> SteelResult {
    if ctx.interrupt_flag.load(Ordering::Relaxed) {
        steel::stop!(Generic =>
            "hume/yield!: script interrupted \
             (step budget exceeded or editor requested cancellation)");
    }
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use super::*;
    use crate::test_support::SteelCtxTestHarness;

    /// `hume_yield` returns `Void` when the interrupt flag is clear.
    ///
    /// Fail oracle: always fire the stop! regardless of the flag → this test errors.
    #[test]
    fn hume_yield_returns_void_when_flag_clear() {
        let mut h = SteelCtxTestHarness::new();
        // Flag starts `false` (clear) — yield must be a transparent no-op.
        let mut ctx = h.ctx();
        let result = hume_yield(&mut ctx);
        assert!(result.is_ok(), "yield with clear flag must return Ok");
        assert!(
            matches!(result.unwrap(), SteelVal::Void),
            "yield must return Void"
        );
    }

    /// `hume_yield` raises a Steel error when the interrupt flag is set.
    ///
    /// Fail oracle: remove the flag check → the error is never raised → test fails.
    #[test]
    fn hume_yield_errors_when_flag_set() {
        let mut h = SteelCtxTestHarness::new();
        h.interrupt_flag.store(true, Ordering::Relaxed);
        let mut ctx = h.ctx();
        let result = hume_yield(&mut ctx);
        assert!(result.is_err(), "yield with set flag must return Err");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("interrupted"), "error must mention 'interrupted'; got: {msg}");
    }
}
