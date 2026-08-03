//! `(spawn-async! cmd args cwd callback)` / `(cancel-async! id)` — generic
//! async subprocess execution.
//!
//! Spawns a command off the main thread and delivers its whole
//! stdout/stderr/exit-status to `callback` once, at completion —
//! `hume-platform`'s `process::job` module is the transport; see its module
//! doc for why this is a one-shot capture rather than the picker's
//! line-batch streaming (`picker-source-spawn!`).

use steel::rvals::SteelVal;

use crate::SteelCtx;

use super::SteelResult;
use super::args::{list_to_strings, optional_path_arg, string_arg, usize_arg};
use super::errors::require_cap;

/// `(spawn-async! cmd args cwd callback)` — runs `cmd` with `args` (direct
/// argv, no shell) in `cwd` (`#f` = the editor's own cwd), off the main
/// thread. `callback` fires exactly once — `(stdout stderr exit-code)` —
/// once the child exits; never inline, so typing never stalls waiting for
/// it. Unlike `picker-source-spawn!`, a spawn failure (missing binary, bad
/// `cwd`) does not raise: `callback` still fires, with empty stdout, a
/// message naming `cmd` in stderr, and `exit-code` `-1` — the same
/// "callback always fires, exactly once" contract as `lsp-request`, so a
/// plugin never has to handle failure in two places. Returns a job id for
/// `cancel-async!`.
pub(crate) fn spawn_async(
    ctx: &mut SteelCtx,
    cmd: SteelVal,
    args: SteelVal,
    cwd: SteelVal,
    callback: SteelVal,
) -> SteelResult {
    let cmd = string_arg(cmd, "spawn-async! cmd")?;
    // No empty-cmd guard here, unlike `picker-source-spawn!`: this builtin's
    // contract is "callback always fires, never raises" (see the doc
    // above), and `Command::new("")` already fails with ENOENT, producing
    // exactly the documented failure triple without a special case.
    let args = list_to_strings(args, "spawn-async! args")?;
    let cwd = optional_path_arg(cwd, "spawn-async! cwd")?;

    let id = require_cap(ctx.host.async_process(), "spawn-async!")?
        .spawn_async(&cmd, args, cwd, callback);
    Ok(SteelVal::IntV(id as isize))
}

/// `(cancel-async! id)` → void. Kills the job's child and drops its
/// callback without firing it. Idempotent: an already-completed,
/// already-cancelled, or unknown id — including a spawn failure that
/// already fired its callback — is a no-op, matching `cancel-timer!`'s
/// contract.
pub(crate) fn cancel_async(ctx: &mut SteelCtx, id: SteelVal) -> SteelResult {
    let id = usize_arg(id, "cancel-async!")? as u64;
    if let Some(host) = ctx.host.async_process() {
        host.cancel_async(id);
    }
    Ok(SteelVal::Void)
}
