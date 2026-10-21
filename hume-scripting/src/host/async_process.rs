//! Async subprocess execution (`spawn-async!`/`cancel-async!`).

use std::path::PathBuf;
use std::process::ExitStatus;

use hume_platform::process::decode_lossy;
use hume_platform::process::job::JobResult;
use steel::rvals::SteelVal;

/// A process's exit code as Steel sees it: the code, or `#f` when there is
/// none (the process could not start, was killed by a signal, or its exit
/// could not be read). Every builtin that reports an exit code uses this, so
/// "no exit code" has one spelling.
pub(crate) fn exit_code_value(status: Option<ExitStatus>) -> SteelVal {
    status
        .and_then(|status| status.code())
        .map_or(SteelVal::BoolV(false), |code| SteelVal::IntV(code as isize))
}

/// The arguments a `spawn-async!` callback receives for `result`:
/// `(stdout stderr exit-code)`, with `exit-code` `#f` when there is none.
pub fn async_callback_args(result: JobResult) -> Vec<SteelVal> {
    vec![
        SteelVal::StringV(decode_lossy(result.stdout).into()),
        SteelVal::StringV(result.stderr.into()),
        exit_code_value(result.status),
    ]
}

/// Async subprocess execution, accessed through
/// [`EditorHost::async_process`](super::EditorHost::async_process). Backs `(spawn-async! cmd args callback
/// #:cwd dir)` / `(cancel-async! id)`.
pub trait AsyncProcessHost {
    /// Spawns `cmd` with `args` (direct argv, no shell) in `cwd`, capturing its whole stdout/stderr to
    /// completion. Always returns a job id, even if the spawn itself fails;
    /// `callback` still fires exactly once either way, with
    /// [`async_callback_args`]: `exit-code` is `#f` for a signal-killed
    /// child, a status the OS never returned, or a spawn failure (missing
    /// binary, bad `cwd`).
    /// No error channel: a plugin holding a callback should never have to
    /// handle failure in two places, matching `lsp-request!`/`prompt!`/
    /// `picker!`'s exactly-once contract.
    fn spawn_async(
        &mut self,
        cmd: &str,
        args: Vec<String>,
        cwd: PathBuf,
        callback: steel::rvals::SteelVal,
    ) -> u64;

    /// Kills and reaps the job's child and drops its callback without
    /// firing it. A no-op if `id` already completed, was already
    /// cancelled, or never existed (a spawn failure that already fired its
    /// callback), the same idempotent contract as `cancel_timer`.
    fn cancel_async(&mut self, id: u64);
}
