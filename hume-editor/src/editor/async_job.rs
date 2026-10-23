//! Per-frame drain for in-flight background work with a Steel callback:
//! `spawn-async!` jobs and `path-programs` scans. Arrival-driven, like the
//! picker's spawned line source (`picker_source.rs`), with no `AsyncSource`
//! entry, only a drain call from `drain_async_sources` (see
//! `async_source.rs`'s module doc).

use std::sync::Arc;

use hume_platform::process::job::SpawnedJob;
use hume_platform::snapshots::Snapshots;
use hume_scripting::Rooted;
use hume_scripting::host::{async_callback_args, names_callback_args};

use super::Editor;

/// In-flight background work plus the Steel callback it reports to. Held
/// in `ConfigState.async_jobs` so abandoned work stops for free on
/// `:reload-config`: `ConfigState`'s wholesale rebuild drops the map,
/// `SpawnedJob::drop` kills a child, and a dropped `Snapshots` stops its
/// producer. That teardown is mandatory, not incidental: `callback`
/// belongs to the outgoing Steel engine and must never be invoked after
/// it's gone, the same hazard `LspState::reset_config_state` documents for
/// its own callback map.
pub(in crate::editor) struct PendingJob {
    pub(in crate::editor) work: AsyncWork,
    pub(in crate::editor) callback: Rooted,
}

pub(in crate::editor) enum AsyncWork {
    /// A `spawn-async!` child, reported once, when it exits.
    Process(SpawnedJob),
    /// A background listing of names, reported as it grows: `(names
    /// done?)`, with every name so far.
    Names(Snapshots<Arc<[String]>>),
}

#[cfg(test)]
impl PendingJob {
    pub(in crate::editor) fn process(&self) -> &SpawnedJob {
        match &self.work {
            AsyncWork::Process(job) => job,
            AsyncWork::Names(_) => panic!("not a process"),
        }
    }
}

impl Editor {
    /// Fires the callback of every job that has completed since the last
    /// frame: `(stdout stderr exit-code)`, `exit-code` `#f` for a
    /// signal-killed child, a status the OS never returned, or a stdout
    /// read that failed or exceeded `JOB_STDOUT_CAP` (`stderr` then names
    /// the failure instead of carrying the child's own diagnostics). A
    /// listing's callback fires once per drain with news, with its latest
    /// names, and the listing leaves the map once it reports `done?`.
    /// Queued via
    /// `queue_background_call`, never invoked inline: this runs from
    /// `drain_async_sources`, the same per-frame chokepoint the LSP/timer
    /// callbacks already share, which is what puts a completing job's
    /// callback under the watchdog/step-budget guard for free.
    pub(super) fn drain_async_jobs(&mut self) {
        // Snapshot ids before polling: `try_take_result` has a side effect
        // (it's `Some` at most once), so it can't be called from inside a
        // borrow of the map that would also need to remove the entry.
        // Mirrors `drain_lsp`'s `let server_ids: Vec<_> = ...collect();`.
        let ids: Vec<u64> = self.state.config.async_jobs.keys().copied().collect();
        for id in ids {
            let Some(pending) = self.state.config.async_jobs.get_mut(&id) else {
                continue;
            };
            let (args, ended) = match &mut pending.work {
                AsyncWork::Process(job) => match job.try_take_result() {
                    Some(result) => (async_callback_args(result), true),
                    None => continue,
                },
                AsyncWork::Names(names) => match names.try_take() {
                    Some(progress) => (names_callback_args(&progress), progress.done),
                    None => continue,
                },
            };
            let callback = pending.callback.clone();
            if ended {
                self.state.config.async_jobs.remove(&id);
            }
            self.state
                .queue_background_call(hume_scripting::SteelCall::new(callback, args));
        }
    }
}
