//! `(spawn-async! cmd args cwd callback)` / `(cancel-async! id)` — generic
//! async subprocess execution. `run-capture!` — a blocking counterpart with
//! no callback, backing `core:stdlib`'s `stdlib/run`.
//!
//! Spawns a command off the main thread and delivers its whole
//! stdout/stderr/exit-status to `callback` once, at completion —
//! `hume-platform`'s `process::job` module is the transport; see its module
//! doc for why this is a one-shot capture rather than the picker's
//! line-batch streaming (`picker-source-spawn!`).

use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::log::LogLevel;

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

/// `(run-capture! cmd args cwd)` → `(stdout stderr exit-code)`. Runs `cmd`
/// with `args` (direct argv, no shell) in `cwd` (`#f` = the editor's own
/// cwd), blocking the calling thread until it exits — the small-output,
/// synchronous-with-the-TUI-still-up shape `stdlib/run` is for; use
/// `spawn-async!` instead for anything that shouldn't stall typing.
///
/// `exit-code` is `#f` — never a sentinel int — for a spawn failure (`cmd`
/// not found, bad `cwd`) or a signal-killed child, matching Steel's own
/// `wait` (`ExitStatus::code()` is `None` in both shapes `run_capture`
/// collapses into one `Err`, and in the signal-killed shape it returns
/// `Ok`). On spawn failure `stdout` is `""` and `stderr` names `cmd` and the
/// io error — `stdlib/run`'s exact preexisting three-case contract, kept so
/// its callers (`stdlib/run-stdout`, the git probes) need no changes.
pub(crate) fn run_capture(
    ctx: &mut SteelCtx,
    cmd: SteelVal,
    args: SteelVal,
    cwd: SteelVal,
) -> SteelResult {
    let cmd = string_arg(cmd, "run-capture! cmd")?;
    let args = list_to_strings(args, "run-capture! args")?;
    let cwd = optional_path_arg(cwd, "run-capture! cwd")?;

    ctx.log(
        LogLevel::Trace,
        format!("run-capture!: running {cmd} {args:?}"),
    );

    let result = match hume_platform::process::run_capture(&cmd, &args, cwd.as_deref()) {
        Ok(output) => vec![
            SteelVal::StringV(String::from_utf8_lossy(&output.stdout).into_owned().into()),
            SteelVal::StringV(String::from_utf8_lossy(&output.stderr).into_owned().into()),
            match output.status.code() {
                Some(code) => SteelVal::IntV(code as isize),
                None => SteelVal::BoolV(false),
            },
        ],
        Err(e) => vec![
            SteelVal::StringV(String::new().into()),
            SteelVal::StringV(format!("{cmd}: {e}").into()),
            SteelVal::BoolV(false),
        ],
    };
    Ok(SteelVal::ListV(result.into()))
}
