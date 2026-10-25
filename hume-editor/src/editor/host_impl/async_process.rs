//! `EditorHostImpl`'s background work with a Steel callback
//! (`spawn-async!`, `path-programs`, `cancel-async!`).

use std::path::PathBuf;

use super::EditorHostImpl;
use crate::editor::async_job::{AsyncWork, PendingJob};
use hume_platform::process::job::JobResult;
use hume_scripting::host::{AsyncProcessHost, JobHandle, async_callback_args};

impl EditorHostImpl<'_> {
    fn next_async_job(&mut self) -> JobHandle {
        let id = self.state.config.next_async_job_id;
        self.state.config.next_async_job_id += 1;
        JobHandle::from_raw(id)
    }
}

impl<'a> AsyncProcessHost for EditorHostImpl<'a> {
    fn spawn_async(
        &mut self,
        cmd: &str,
        args: Vec<String>,
        cwd: PathBuf,
        callback: hume_scripting::Rooted,
    ) -> JobHandle {
        let id = self.next_async_job();

        let launch = hume_platform::process::Launch {
            cwd: &cwd,
            search_path: self.state.dirs.search_path.as_deref(),
        };
        match hume_platform::process::job::spawn_job(
            cmd,
            &args,
            launch,
            Vec::new(),
            std::sync::Arc::clone(&self.state.wake),
        ) {
            Ok(job) => {
                self.state.config.async_jobs.insert(
                    id,
                    PendingJob::Live {
                        work: AsyncWork::Process(job),
                        callback,
                    },
                );
            }
            // Spawn failed: the callback is queued right here, with no
            // output and no exit code, as for a signal-killed child, and
            // the entry waits for it like any job's last callback, so
            // `cancel-async!` can still drop it.
            Err(e) => {
                self.state.config.async_jobs.insert(id, PendingJob::Ended);
                self.state.queue_job_callback(
                    id,
                    hume_scripting::SteelCall::new(
                        callback,
                        async_callback_args(JobResult::failed(e.to_string())),
                    ),
                );
                // The `Ok` arm needs no wake: the job thread wakes the loop
                // itself on completion. This callback has no background
                // thread behind it. `settle()`'s fixpoint does pick it up
                // within the same `settle()` call even when `spawn-async!`
                // was itself invoked from a queued Steel callback, but this
                // wake is kept anyway: cheap, harmless if the loop is already
                // awake, and the one thing standing between "unfired" and
                // "fired eventually" if that invariant ever changes.
                (self.state.wake)();
            }
        }
        id
    }

    fn scan_programs(
        &mut self,
        prefix: String,
        extensions: Vec<String>,
        callback: hume_scripting::Rooted,
    ) -> Result<JobHandle, String> {
        let names = hume_platform::path::programs::spawn_program_scan(
            self.state.dirs.search_path.clone(),
            self.state.cwd.to_path_buf(),
            prefix,
            extensions,
            std::sync::Arc::clone(&self.state.wake),
        )
        .map_err(|e| format!("path-programs: the scan could not start: {e}"))?;
        let id = self.next_async_job();
        self.state.config.async_jobs.insert(
            id,
            PendingJob::Live {
                work: AsyncWork::Names(names),
                callback,
            },
        );
        Ok(id)
    }

    fn cancel_async(&mut self, job: JobHandle) {
        // Dropping the entry drops its work (a `SpawnedJob` kills and reaps
        // its child, a `Snapshots` stops its producer) and its callback,
        // which a queued call then finds gone and never runs: a no-op once
        // the last callback has run or after an earlier cancel.
        self.state.config.async_jobs.remove(&job);
    }
}
