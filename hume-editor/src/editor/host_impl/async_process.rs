//! `AsyncProcessHost` — moved out of `host_impl.rs`'s per-capability split.

use std::path::PathBuf;

use super::EditorHostImpl;
use hume_scripting::host::AsyncProcessHost;

impl<'a> AsyncProcessHost for EditorHostImpl<'a> {
    fn spawn_async(
        &mut self,
        cmd: &str,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        callback: steel::rvals::SteelVal,
    ) -> u64 {
        let id = self.state.config.next_async_job_id;
        self.state.config.next_async_job_id += 1;

        match hume_platform::process::job::spawn_job(
            cmd,
            &args,
            cwd.as_deref(),
            std::sync::Arc::clone(&self.state.wake),
        ) {
            Ok(job) => {
                self.state
                    .config
                    .async_jobs
                    .insert(id, crate::editor::async_job::PendingJob { job, callback });
            }
            // Spawn failed before a job/callback contract could exist — fire
            // the callback right here rather than leaving it unfired, with
            // the same "no output, -1 exit code" shape a signal-killed
            // child produces (the sentinel `%run-inline-output!` already
            // uses — a real exit code can never be -1, it's u8-wide).
            Err(e) => {
                self.state.queue_steel_call(
                    callback,
                    vec![
                        steel::rvals::SteelVal::StringV("".into()),
                        steel::rvals::SteelVal::StringV(format!("cannot run '{cmd}': {e}").into()),
                        steel::rvals::SteelVal::IntV(-1),
                    ],
                );
                // The `Ok` arm needs no wake: the job thread wakes the loop
                // itself on completion. This callback has no background
                // thread behind it — `settle()`'s fixpoint does pick it up
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

    fn cancel_async(&mut self, id: u64) {
        // Dropping the entry drops its `SpawnedJob` (kills + reaps the
        // child) and its callback `SteelVal` without ever calling it — a
        // no-op if `id` already completed, was already cancelled, or never
        // existed (a spawn failure that already fired its callback above).
        self.state.config.async_jobs.remove(&id);
    }
}
