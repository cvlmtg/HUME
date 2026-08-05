//! Per-frame drain for in-flight `spawn-async!` jobs. Arrival-driven, like
//! the picker's spawned line source (`picker_source.rs`) — no `AsyncSource`
//! entry, only a drain call from `drain_async_sources` (see
//! `async_source.rs`'s module doc).

use steel::rvals::SteelVal;

use super::Editor;

/// An in-flight `spawn-async!` job: the process handle plus the Steel
/// callback to fire once it completes. Held in `ConfigState.async_jobs` so
/// an abandoned job is killed for free on `:reload-config` — `ConfigState`'s
/// wholesale rebuild drops the map, and `SpawnedJob::drop` kills the child.
/// That teardown is mandatory, not incidental: `callback` belongs to the
/// outgoing Steel engine and must never be invoked after it's gone — the
/// same hazard `LspState::reset_config_state` documents for its own
/// callback map.
pub(crate) struct PendingJob {
    pub(crate) job: hume_platform::process::job::SpawnedJob,
    pub(crate) callback: SteelVal,
}

impl Editor {
    /// Fires the callback of every job that has completed since the last
    /// frame — `(stdout stderr exit-code)`, `exit-code` `-1` for a
    /// signal-killed child, a status the OS never returned, or a stdout
    /// read that failed or exceeded `JOB_STDOUT_CAP` (`stderr` then names
    /// the failure instead of carrying the child's own diagnostics).
    /// Queued via
    /// `queue_steel_call`, never invoked inline: this runs from
    /// `drain_async_sources`, the same per-frame chokepoint the LSP/timer
    /// callbacks already share, which is what puts a completing job's
    /// callback under the watchdog/step-budget guard for free.
    pub(super) fn drain_async_jobs(&mut self) {
        // Snapshot ids before polling — `try_take_result` has a side effect
        // (it's `Some` at most once), so it can't be called from inside a
        // borrow of the map that would also need to remove the entry.
        // Mirrors `drain_lsp`'s `let server_ids: Vec<_> = ...collect();`.
        let ids: Vec<u64> = self.state.config.async_jobs.keys().copied().collect();
        for id in ids {
            let Some(pending) = self.state.config.async_jobs.get_mut(&id) else {
                continue;
            };
            let Some(result) = pending.job.try_take_result() else {
                continue;
            };
            let pending = self
                .state
                .config
                .async_jobs
                .remove(&id)
                .expect("just found by get_mut above");
            let code = result.status.and_then(|s| s.code()).unwrap_or(-1);
            self.state.queue_steel_call(
                pending.callback,
                vec![
                    SteelVal::StringV(result.stdout.into()),
                    SteelVal::StringV(result.stderr.into()),
                    SteelVal::IntV(code as isize),
                ],
            );
        }
    }
}
