//! Step-budget / interrupt builtin: `hume/yield!`.
//!
//! Cooperative interruption: scripts that want to be interruptible call
//! `(hume/yield!)` regularly (typically inside long loops).  On each call the
//! builtin checks an [`std::sync::atomic::AtomicBool`] shared with
//! [`crate::scripting::ScriptingHost`].  If the flag is set, the script is
//! aborted with a Steel error; otherwise execution continues normally.
//!
//! The flag is set by:
//! - The [`EvalWatchdog`](crate::scripting::EvalWatchdog) spawned at the start of
//!   each eval (fires after the configured budget; see `steel-init-budget-ms`
//!   and `steel-command-budget-ms`).
//! - Future Ctrl-C handling: the editor can set
//!   [`ScriptingHost::interrupt_flag`](crate::scripting::ScriptingHost) when
//!   the user presses Ctrl-C while a script is running.
//!
//! **Limitation:** interruption is cooperative only.  A script without
//! `(hume/yield!)` calls will run to completion regardless of the budget.
//! Steel 0.8.2 does not expose an op-callback hook for involuntary interruption.
//!
//! ## TLS design
//!
//! `(hume/yield!)` is callable from both `eval_source_raw` (where `EVAL_CTX`
//! is armed) and `call_steel_cmd` (where it is not — only the command dispatch
//! TLS slots are armed).  Using a dedicated `YIELD_FLAG` TLS decouples the
//! yield check from the full `EvalCtx` and makes it work in both contexts.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use steel::rvals::SteelVal;
use steel::rerrs::SteelErr;

type SteelResult = Result<SteelVal, SteelErr>;

thread_local! {
    /// The interrupt flag for the current Steel eval or command invocation.
    ///
    /// Armed by `eval_source_raw` and `call_steel_cmd` before calling into
    /// Steel, cleared afterwards.  `hume_yield` reads from this TLS so it
    /// works in both contexts (unlike reading through `EVAL_CTX`, which is
    /// `None` during `call_steel_cmd`).
    pub(crate) static YIELD_FLAG: RefCell<Option<Arc<AtomicBool>>> = RefCell::new(None);
}

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
    YIELD_FLAG.with(|cell| {
        if let Some(flag) = cell.borrow().as_ref() {
            if flag.load(Ordering::Relaxed) {
                steel::stop!(Generic =>
                    "hume/yield!: script interrupted \
                     (step budget exceeded or editor requested cancellation)");
            }
        }
        Ok(SteelVal::Void)
    })
}
