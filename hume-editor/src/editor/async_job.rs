//! Per-frame drain for in-flight background work with a Steel callback:
//! `spawn-async!` jobs and `path-programs` scans. Arrival-driven, like the
//! picker's spawned line source (`picker_source.rs`), with no `AsyncSource`
//! entry, only a drain call from `drain_async_sources` (see
//! `async_source.rs`'s module doc).

use std::sync::Arc;

use hume_platform::process::job::SpawnedJob;
use hume_platform::snapshots::Snapshots;
use hume_scripting::SteelCall;
use hume_scripting::host::{JobHandle, async_callback_args, names_callback_args};

use super::Editor;

/// One `ConfigState.async_jobs` entry. Held there so abandoned work stops
/// for free on `:reload-config`: `ConfigState`'s wholesale rebuild drops
/// the map, `SpawnedJob::drop` kills a child, and a dropped `Snapshots`
/// stops its producer. That teardown is mandatory, not incidental: a
/// callback belongs to the outgoing Steel engine and must never be invoked
/// after it's gone, the same hazard `LspState::reset_config_state`
/// documents for its own callback map.
pub(in crate::editor) enum PendingJob {
    /// In-flight work plus the Steel callback it reports to.
    Live {
        work: AsyncWork,
        callback: hume_scripting::Callback,
    },
    /// Work whose last callback is queued and has not run yet. The entry
    /// stays so `cancel-async!` can still drop that callback.
    Ended,
}

pub(in crate::editor) enum AsyncWork {
    /// A `spawn-async!` child, reported once, when it exits.
    Process(SpawnedJob),
    /// A background listing of names, reported as it grows: `(names
    /// done?)`, with every name so far.
    Names(Snapshots<Arc<[Arc<str>]>>),
}

#[cfg(test)]
impl PendingJob {
    pub(in crate::editor) fn process(&self) -> &SpawnedJob {
        match self {
            Self::Live {
                work: AsyncWork::Process(job),
                ..
            } => job,
            _ => panic!("not a live process"),
        }
    }
}

impl PendingJob {
    /// The callback call for the work's news since the last poll, if any.
    /// The work's last report leaves the entry [`Self::Ended`].
    fn poll(&mut self) -> Option<SteelCall> {
        let Self::Live { mut work, callback } = std::mem::replace(self, Self::Ended) else {
            return None;
        };
        let news = match &mut work {
            AsyncWork::Process(job) => job
                .try_take_result()
                .map(|result| (async_callback_args(result), true)),
            AsyncWork::Names(names) => names
                .try_take()
                .map(|progress| (names_callback_args(&progress), progress.done)),
        };
        let Some((args, ended)) = news else {
            *self = Self::Live { work, callback };
            return None;
        };
        if ended {
            return Some(SteelCall::new(callback, args));
        }
        let call = SteelCall::new(callback.clone(), args);
        *self = Self::Live { work, callback };
        Some(call)
    }
}

impl Editor {
    /// Fires the callback of every job that has completed since the last
    /// frame, with its process-result hash: `'exit` is `#f` for a
    /// signal-killed child, a status the OS never returned, or a stdout
    /// read that failed or exceeded `JOB_STDOUT_CAP` (`'stderr` then names
    /// the failure instead of carrying the child's own diagnostics). A
    /// listing's callback fires once per drain with news, with its latest
    /// names. Work that has reported its end stays in the map as
    /// [`PendingJob::Ended`] until that last callback has run. Queued via
    /// `queue_job_callback`, never invoked inline: this runs from
    /// `drain_async_sources`, the same per-frame chokepoint the LSP/timer
    /// callbacks already share, which is what puts a completing job's
    /// callback under the watchdog/step-budget guard for free.
    pub(super) fn drain_async_jobs(&mut self) {
        // Snapshot ids before polling: queuing a callback needs `&mut
        // self.state`, which a borrow of the map held across the loop would
        // block. Mirrors `drain_lsp`'s `let server_ids: Vec<_> = ...collect();`.
        let ids: Vec<JobHandle> = self.state.config.async_jobs.keys().copied().collect();
        for id in ids {
            if let Some(call) = self
                .state
                .config
                .async_jobs
                .get_mut(&id)
                .and_then(PendingJob::poll)
            {
                self.state.queue_job_callback(id, call);
            }
        }
    }

    /// Removes job `id` when its last callback was the one just run or
    /// dropped. A dropped earlier report needs nothing: the next one
    /// carries every name it did.
    pub(in crate::editor) fn forget_job_callback(&mut self, id: JobHandle) {
        let jobs = &mut self.state.config.async_jobs;
        if jobs
            .get(&id)
            .is_some_and(|pending| matches!(pending, PendingJob::Ended))
        {
            jobs.remove(&id);
        }
    }
}
