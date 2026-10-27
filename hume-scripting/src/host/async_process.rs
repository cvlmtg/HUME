//! Background work Steel waits on with a callback: subprocesses
//! (`spawn-async!`) and program scans (`path-programs`), both stopped with
//! `cancel-async!`.

use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::Arc;

use hume_platform::process::decode_lossy;
use hume_platform::process::job::JobResult;
use hume_platform::snapshots::Progress;
use steel::rvals::SteelVal;

use crate::builtins::args::{string_list, symbol_hash};

/// A process's exit code as Steel sees it: the code, or `#f` when there is
/// none (the process could not start, was killed by a signal, or its exit
/// could not be read). Every exit code a builtin reports goes through
/// [`process_result`], so "no exit code" has one spelling.
fn exit_code_value(status: Option<ExitStatus>) -> SteelVal {
    status
        .and_then(|status| status.code())
        .map_or(SteelVal::BoolV(false), |code| SteelVal::IntV(code as isize))
}

/// A finished process as Steel sees it, from `run-capture!` and
/// `spawn-async!` alike: `(hash 'stdout s 'stderr s 'exit code)`, with
/// `'exit` from [`exit_code_value`].
pub(crate) fn process_result(
    stdout: String,
    stderr: String,
    status: Option<ExitStatus>,
) -> SteelVal {
    symbol_hash([
        ("stdout", SteelVal::StringV(stdout.into())),
        ("stderr", SteelVal::StringV(stderr.into())),
        ("exit", exit_code_value(status)),
    ])
}

/// The arguments a `spawn-async!` callback receives for `result`: its one
/// `process_result`.
pub fn async_callback_args(result: JobResult) -> Vec<SteelVal> {
    vec![process_result(
        decode_lossy(result.stdout),
        result.stderr,
        result.status,
    )]
}

/// The arguments a background listing's callback receives for `progress`:
/// `(names done?)`, every name found so far and whether the listing ended.
pub fn names_callback_args(progress: &Progress<Arc<[Arc<str>]>>) -> Vec<SteelVal> {
    vec![
        string_list(progress.value.iter().map(|name| name.to_string())),
        SteelVal::BoolV(progress.done),
    ]
}

/// A `spawn-async!` job or `path-programs` scan, as Steel holds it: an
/// opaque value only `cancel-async!` accepts, so a timer's handle can never
/// name a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobHandle(u64);

impl JobHandle {
    /// Wraps an id the host minted.
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The id the host minted.
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// Background work with a Steel callback, accessed through
/// [`EditorHost::async_process`](super::EditorHost::async_process). Backs
/// `(spawn-async! cmd args callback #:cwd dir)`, `(path-programs prefix
/// extensions callback)` and `(cancel-async! job)`, which share one kind of
/// handle.
pub trait AsyncProcessHost {
    /// Spawns `cmd` with `args` (direct argv, no shell) in `cwd`, capturing its whole stdout/stderr to
    /// completion. Always returns a job, even if the spawn itself fails;
    /// `callback` still fires exactly once either way, unless cancelled, with
    /// [`async_callback_args`]: `'exit` is `#f` for a signal-killed
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
        callback: crate::Callback,
    ) -> JobHandle;

    /// Lists the programs on the editor's search path whose names start
    /// with `prefix`, on a background thread, and returns the scan.
    /// `callback` gets [`names_callback_args`] after each search path
    /// directory that adds a name, or for several at once when the editor
    /// falls behind, and a last time with `done?` `#t`. On Windows a
    /// program is a file ending in one of `extensions`, named without it.
    /// `Err` when the thread cannot start.
    fn scan_programs(
        &mut self,
        prefix: String,
        extensions: Vec<String>,
        callback: crate::Callback,
    ) -> Result<JobHandle, String>;

    /// Stops `job` and drops its callback without firing it, even a
    /// callback already due: a job's child is killed and reaped, a scan
    /// reads no further directory. A no-op once its last callback has run
    /// or after an earlier cancel, the same idempotent contract as
    /// `cancel_timer`.
    fn cancel_async(&mut self, job: JobHandle);
}
