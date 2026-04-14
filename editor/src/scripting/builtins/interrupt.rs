//! Step-budget / interrupt builtin: `hume/yield!`.
//!
//! Cooperative interruption: scripts that want to be interruptible call
//! `(hume/yield!)` regularly (typically inside long loops).  On each call the
//! builtin checks an [`std::sync::atomic::AtomicBool`] shared with
//! [`crate::scripting::ScriptingHost`].  If the flag is set, the script is
//! aborted with a Steel error; otherwise execution continues normally.
//!
//! The flag is set by:
//! - The watchdog thread spawned at the start of `eval_init` (fires after
//!   [`EVAL_BUDGET_SECS`](crate::scripting::EVAL_BUDGET_SECS) of wall-clock time).
//! - Future Ctrl-C handling: the editor can set
//!   [`ScriptingHost::interrupt_flag`](crate::scripting::ScriptingHost) when
//!   the user presses Ctrl-C while a script is running.
//!
//! **Limitation:** interruption is cooperative only.  A script without
//! `(hume/yield!)` calls will run to completion regardless of the budget.
//! Steel 0.8.2 does not expose an op-callback hook for involuntary interruption.

use std::sync::atomic::Ordering;

use steel::rvals::SteelVal;
use steel::rerrs::SteelErr;

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
pub(crate) fn hume_yield(args: &[SteelVal]) -> SteelResult {
    if !args.is_empty() {
        steel::stop!(ArityMismatch => "hume/yield! expects 0 args, got {}", args.len());
    }
    super::with_ctx(|ctx| {
        if ctx.interrupt_flag.load(Ordering::Relaxed) {
            steel::stop!(Generic =>
                "hume/yield!: script interrupted \
                 (step budget exceeded or editor requested cancellation)");
        }
        Ok(SteelVal::Void)
    })
}
