//! Async subprocess execution (`spawn-async!`/`cancel-async!`) — moved out
//! of `host.rs`'s per-capability split.

use std::path::PathBuf;

/// Async subprocess execution — accessed through
/// [`EditorHost::async_process`](super::EditorHost::async_process). Backs `(spawn-async! cmd args cwd
/// callback)` / `(cancel-async! id)`.
pub trait AsyncProcessHost {
    /// Spawns `cmd` with `args` (direct argv, no shell) in `cwd` (`None` =
    /// the editor's own cwd), capturing its whole stdout/stderr to
    /// completion. Always returns a job id, even if the spawn itself fails
    /// — `callback` still fires exactly once either way, `(stdout stderr
    /// exit-code)`, `exit-code` `-1` for a signal-killed child, a status the
    /// OS never returned, or a spawn failure (missing binary, bad `cwd`).
    /// No error channel: a plugin holding a callback should never have to
    /// handle failure in two places, matching `lsp-request`/`prompt!`/
    /// `picker!`'s exactly-once contract.
    fn spawn_async(
        &mut self,
        cmd: &str,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        callback: steel::rvals::SteelVal,
    ) -> u64;

    /// Kills and reaps the job's child and drops its callback without
    /// firing it. A no-op if `id` already completed, was already
    /// cancelled, or never existed (a spawn failure that already fired its
    /// callback) — same idempotent contract as `cancel_timer`.
    fn cancel_async(&mut self, id: u64);
}
