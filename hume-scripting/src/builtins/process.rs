//! `(spawn-async! cmd args callback #:cwd dir)` / `(cancel-async! job)`: generic
//! async subprocess execution. `run-capture!`: a blocking counterpart with
//! no callback.
//!
//! Spawns a command off the main thread and delivers its whole
//! stdout/stderr/exit-status to `callback` once, at completion.
//! `hume-platform`'s `process::job` module is the transport; see its module
//! doc for why this is a one-shot capture rather than the picker's
//! line-batch streaming (`picker-source-spawn!`).

use hume_platform::process::{Launch, decode_lossy};
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::process_result;
use hume_core::Severity;

use super::SteelResult;
use super::args::{
    ENV_PAIR_SHAPE, cwd_arg, job_arg, list_to_string_pairs, list_to_strings, string_arg,
};
use super::errors::{generic_err, require_cap};

/// `(%spawn-async! cmd args cwd callback)`, wrapped by `spawn-async!`'s
/// `#:cwd` keyword in `bootstrap.scm`: runs `cmd` with `args` (direct
/// argv, no shell) in `cwd` (`#f` = the editor's cwd, a relative path joins
/// onto it), off the main
/// thread. `callback` runs one time, after the child exits, with the hash
/// `run-capture!` returns; never inline, so typing never stalls waiting for
/// it. Unlike `picker-source-spawn!`, a spawn failure (missing binary, bad
/// `cwd`) does not raise: `callback` still fires, with empty `'stdout`, a
/// message naming `cmd` in `'stderr`, and `'exit` `#f`: the same
/// "callback always fires, exactly once" contract as `lsp-request!`, so a
/// plugin never has to handle failure in two places. Returns a job for
/// `cancel-async!`.
pub(crate) fn spawn_async(
    ctx: &mut SteelCtx,
    cmd: SteelVal,
    args: SteelVal,
    cwd: SteelVal,
    callback: crate::Callback,
) -> SteelResult {
    let cmd = string_arg(cmd, "cmd")?;
    // No empty-cmd guard here, unlike `picker-source-spawn!`: this builtin's
    // contract is "callback always fires, never raises" (see the doc
    // above), and `Command::new("")` already fails with ENOENT, producing
    // exactly the documented failure triple without a special case.
    let args = list_to_strings(args, "args")?;
    let cwd = cwd_arg(cwd, &ctx.host.buffers().cwd(), "cwd")?;

    let job = require_cap(ctx.host.async_process())?.spawn_async(&cmd, args, cwd, callback);
    Ok(job.into_steel_val())
}

/// `(cancel-async! job)` → void. Stops the job or scan and drops its
/// callback without firing it, even one already due
/// ([`AsyncProcessHost::cancel_async`](crate::host::AsyncProcessHost::cancel_async)).
/// Idempotent: a job whose last callback has run or one already cancelled
/// is a no-op, matching `cancel-timer!`'s contract. Raises for anything but
/// a job.
pub(crate) fn cancel_async(ctx: &mut SteelCtx, job: SteelVal) -> SteelResult {
    let job = job_arg(&job, "job")?;
    require_cap(ctx.host.async_process())?.cancel_async(job);
    Ok(SteelVal::Void)
}

/// `(%run-capture! cmd args cwd)` → `(hash 'stdout s 'stderr s 'exit code)`, wrapped by
/// `run-capture!`'s `#:cwd` keyword in `bootstrap.scm`. Runs `cmd`
/// with `args` (direct argv, no shell) in `cwd` (`#f` = the editor's cwd, a
/// relative path joins onto it), blocking the calling thread until it exits: the small-output,
/// synchronous-with-the-TUI-still-up shape `run-capture!` is for; use
/// `spawn-async!` instead for anything that shouldn't stall typing.
///
/// `'exit` is `#f` (never a sentinel int) for a spawn failure (`cmd`
/// not found, bad `cwd`) or a signal-killed child, matching Steel's own
/// `wait` (`ExitStatus::code()` is `None` in both shapes `run_capture`
/// collapses into one `Err`, and in the signal-killed shape it returns
/// `Ok`). On spawn failure `stdout` is `""` and `stderr` names `cmd` and the
/// io error. Callers rely on this three-case contract.
pub(crate) fn run_capture(
    ctx: &mut SteelCtx,
    cmd: SteelVal,
    args: SteelVal,
    cwd: SteelVal,
) -> SteelResult {
    let cmd = string_arg(cmd, "cmd")?;
    let args = list_to_strings(args, "args")?;
    let cwd = cwd_arg(cwd, &ctx.host.buffers().cwd(), "cwd")?;
    let search_path = ctx.host.buffers().search_path();

    ctx.log(
        Severity::Trace,
        format!("run-capture!: running {cmd} {args:?}"),
    );

    let launch = Launch {
        cwd: &cwd,
        search_path: search_path.as_deref(),
    };
    let captured = ctx.while_waiting(|| hume_platform::process::run_capture(&cmd, &args, launch));
    Ok(match captured {
        Ok(output) => process_result(
            decode_lossy(output.stdout),
            decode_lossy(output.stderr),
            Some(output.status),
        ),
        Err(e) => process_result(String::new(), e.to_string(), None),
    })
}

/// `(%run-inline-output! cmd args cwd env)`: spawn `cmd` with `args` (a list of
/// strings) and inherited stdio; blocks until exit and raises unless it
/// exited 0. `cwd` is a string or `#f` (the editor's cwd; a relative path
/// joins onto it); `env` is a list of `("KEY" . "VALUE")` pairs added to the
/// inherited environment.
///
/// A Rust builtin rather than Steel's own `spawn-process` because it runs
/// only inside an `#:inline-output` command and leaves the editor's screen
/// before the child writes, and starts the child on the editor's search
/// path with git's credential prompt denied. No sandbox
/// checks: plugins are trusted code (see `user-manual/docs/writing-plugins.md`'s
/// "Filesystem and processes").
///
/// # Errors
/// Called outside an `#:inline-output` command, the binary can't be spawned
/// (e.g. not found on `PATH`), or it exited non-zero or was killed by a
/// signal.
pub(crate) fn run_inline_output(
    ctx: &mut SteelCtx,
    cmd: String,
    args_val: SteelVal,
    cwd_val: SteelVal,
    env_val: SteelVal,
) -> SteelResult {
    let args = list_to_strings(args_val, "args")?;
    let cwd = cwd_arg(cwd_val, &ctx.host.buffers().cwd(), "cwd")?;
    let env = list_to_string_pairs(env_val, "env", ENV_PAIR_SHAPE)?;
    let search_path = ctx.host.buffers().search_path();

    // The child inherits stdio, so it writes to the terminal.
    super::io::claim_terminal(ctx)?;

    let launch = Launch {
        cwd: &cwd,
        search_path: search_path.as_deref(),
    };
    let status = ctx
        .while_waiting(|| hume_platform::process::run_inline_output(&cmd, &args, launch, &env))
        .map_err(generic_err)?;

    if status.success() {
        Ok(SteelVal::Void)
    } else {
        Err(generic_err(hume_platform::process::failure_message(
            &cmd,
            Some(status),
            "",
        )))
    }
}

#[cfg(test)]
mod tests;
