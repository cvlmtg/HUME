//! `EditorHostImpl`'s background work with a Steel callback
//! (`spawn-async!`, `path-programs`, `cancel-async!`).

use std::path::PathBuf;

use super::EditorHostImpl;
use crate::editor::async_job::{AsyncWork, PendingJob};
use hume_platform::process::job::JobResult;
use hume_scripting::host::{AsyncProcessHost, async_callback_args};

impl EditorHostImpl<'_> {
    fn next_async_job_id(&mut self) -> u64 {
        let id = self.state.config.next_async_job_id;
        self.state.config.next_async_job_id += 1;
        id
    }
}

impl<'a> AsyncProcessHost for EditorHostImpl<'a> {
    fn spawn_async(
        &mut self,
        cmd: &str,
        args: Vec<String>,
        cwd: PathBuf,
        callback: hume_scripting::Rooted,
    ) -> u64 {
        let id = self.next_async_job_id();

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
                    PendingJob {
                        work: AsyncWork::Process(job),
                        callback,
                    },
                );
            }
            // Spawn failed before a job/callback contract could exist: fire
            // the callback right here rather than leaving it unfired, with
            // no output and no exit code, as for a signal-killed child.
            Err(e) => {
                self.state
                    .queue_background_call(hume_scripting::SteelCall::new(
                        callback,
                        async_callback_args(JobResult::failed(e.to_string())),
                    ));
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
    ) -> Result<u64, String> {
        let names = hume_platform::path::programs::spawn_program_scan(
            self.state.dirs.search_path.clone(),
            prefix,
            extensions,
            std::sync::Arc::clone(&self.state.wake),
        )
        .map_err(|e| format!("path-programs: the scan could not start: {e}"))?;
        let id = self.next_async_job_id();
        self.state.config.async_jobs.insert(
            id,
            PendingJob {
                work: AsyncWork::Names(names),
                callback,
            },
        );
        Ok(id)
    }

    fn cancel_async(&mut self, id: u64) {
        // Dropping the entry drops its work (a `SpawnedJob` kills and reaps
        // its child, a `Snapshots` stops its producer) and its callback
        // without ever calling it: a no-op if `id` already completed, was
        // already cancelled, or never existed (a spawn failure that already
        // fired its callback above).
        self.state.config.async_jobs.remove(&id);
    }
}
