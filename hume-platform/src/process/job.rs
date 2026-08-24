//! One-shot subprocess capture: run a command to completion and deliver its
//! whole stdout, whole stderr, and exit status exactly once — the shape
//! `spawn-async!` needs, as opposed to `line_source`'s per-line streaming.
//!
//! Completion here is the child *exiting*, which is not implied by its
//! pipes reaching EOF — a child can close (or exec away from) its stdio
//! while continuing to run, so "both pipes at EOF" is not "the child is
//! done". The capture thread this module spawns is free to block, so it
//! reads stdout to EOF, joins the stderr thread, then polls the child's
//! real exit status (never a blocking `wait`, which would hold the shared
//! [`TrackedChild`](crate::process::tracked::TrackedChild) slot's lock for
//! the child's entire remaining lifetime and starve a concurrent
//! `cancel-async!`/`SpawnedJob`'s `Drop`), and only then sends the complete
//! result. [`SpawnedJob::try_take_result`](crate::process::job::SpawnedJob::try_take_result)
//! is a pure receive with no reaping of its own — unlike
//! `line_source::SpawnedLineSource::finish`, which reaps on the main thread
//! because EOF really is completion for a line source.
//!
//! Two capture threads, not one: reading stdout to EOF and then stderr
//! would deadlock on a child that fills its stderr pipe while this thread
//! is still blocked reading stdout — the same hazard the Scheme sync path
//! documents at `runtime/plugins/core/stdlib/plugin.scm`'s `stdlib/run`.

use std::io;
use std::path::Path;
use std::process::ExitStatus;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::process::WakeCallback;
use crate::process::child::{
    JOB_STDOUT_CAP, STDERR_CAPTURE_CAP, WakeOnDrop, read_bounded, read_capped, spawn_piped,
};
use crate::process::tracked::TrackedChild;

/// The complete output of a finished [`SpawnedJob`]: whole stdout (never
/// *silently* truncated — capped at `JOB_STDOUT_CAP`, but exceeding it
/// fails the job rather than handing back a short prefix), whole stderr
/// (capped at `STDERR_CAPTURE_CAP` — diagnostic only, truncation there is
/// fine), and exit status (`None` on a stdout read failure/overflow, or if
/// the capture thread's own exit-status poll errored, or the thread
/// panicked before sending — the last three vanishingly rare).
///
/// Also what the capture thread sends the main thread over the channel —
/// `status` is `None` there only if the exit-status poll itself errored; a
/// thread that panics before sending is handled separately, by
/// [`SpawnedJob::try_take_result`] synthesizing an empty `JobResult` on
/// channel disconnect.
pub struct JobResult {
    pub stdout: String,
    pub stderr: String,
    pub status: Option<ExitStatus>,
}

/// A running external command whose whole output is being captured to
/// completion.
///
/// Owns the child: dropping it kills the child (`Drop` = kill + wait,
/// matching `SpawnedLineSource`), so an abandoned job — cancelled from
/// Steel, or the registry holding it torn down on `:reload-config` — can't
/// leak a process. The child is a [`TrackedChild`], its own process-group
/// leader, so it's also reaped on a force-exit that skips this `Drop`
/// entirely — see `tracked`'s module doc.
pub struct SpawnedJob {
    child: TrackedChild,
    rx: Option<mpsc::Receiver<JobResult>>,
}

/// Spawns `cmd` with `args` (direct argv, no shell), piped stdio, stdin
/// closed immediately. One thread reads stderr to EOF (capped, lenient); a
/// second reads stdout to EOF (capped at `JOB_STDOUT_CAP`, strict — a
/// read error or overflow fails the job), joins the first, then sends the
/// combined capture once and fires `wake`.
pub fn spawn_job(
    cmd: &str,
    args: &[String],
    cwd: Option<&Path>,
    wake: WakeCallback,
) -> io::Result<SpawnedJob> {
    let (child, stdout, stderr) = spawn_piped(cmd, args, cwd)?;
    // Converted to a `TrackedChild` up front, not deferred to the end like
    // `line_source` does — the job thread below needs its own handle to
    // poll the child's exit status, so it and this function's returned
    // `SpawnedJob` must share the same tracked slot from the start. From
    // here on this function owns reaping the child on every early return
    // (the disarmed `ReapOnDrop` guard no longer covers it).
    let child = TrackedChild::new(child.into_inner());

    let (tx, rx) = mpsc::sync_channel::<JobResult>(1);

    let stderr_thread = thread::Builder::new()
        .name("hume-job-stderr".into())
        .spawn(move || read_capped(stderr, STDERR_CAPTURE_CAP))
        .inspect_err(|_| {
            child.reap();
        })?;

    let job_child = child.clone();
    let job_cmd = cmd.to_string();
    // The returned `JoinHandle` is intentionally dropped, not stored:
    // dropping it detaches the thread (same as a bare `thread::spawn`
    // whose handle is discarded) — nothing here ever needs to join it, so
    // keeping it around would just be a field that's written once and read
    // never.
    thread::Builder::new()
        .name("hume-job".into())
        .spawn(move || {
            // Fires as this closure returns, after the send below — a
            // panicking read still wakes the drain to observe the
            // synthesized result `try_take_result` produces on disconnect.
            let _wake_on_drop = WakeOnDrop(wake);
            let stdout_result = read_bounded(stdout, JOB_STDOUT_CAP);
            // A panicked stderr thread degrades to empty stderr rather than
            // wedging this job forever — the exit status still carries the
            // failure, and stdout is what most callers actually want.
            let stderr_bytes = stderr_thread.join().unwrap_or_default();
            let status = wait_for_exit(&job_child);
            let result = match stdout_result {
                Ok(stdout_bytes) => JobResult {
                    stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
                    status,
                },
                // A truncated stdout is worse than none: hand back the
                // documented spawn-failure shape (empty stdout, a message
                // in stderr, no exit code) rather than silently short data.
                Err(e) => JobResult {
                    stdout: String::new(),
                    stderr: format!("{job_cmd}: {e}"),
                    status: None,
                },
            };
            let _ = tx.send(result);
        })
        .inspect_err(|_| {
            child.reap();
        })?;

    Ok(SpawnedJob {
        child,
        rx: Some(rx),
    })
}

/// Polls `child`'s exit status rather than blocking on a plain `wait()` —
/// `wait()` would hold the shared slot's mutex for the child's entire
/// remaining lifetime, starving the `try_wait`/`reap` calls a concurrent
/// `cancel-async!` or [`SpawnedJob::drop`] needs that same lock for. `None`
/// only if `try_wait` itself errors — vanishingly rare.
fn wait_for_exit(child: &TrackedChild) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => thread::sleep(EXIT_POLL_INTERVAL),
            Err(_) => return None,
        }
    }
}

/// Poll interval for [`wait_for_exit`] — frequent enough that a job's
/// result is delivered promptly after the child actually exits, cheap
/// enough that a long-running child costs nothing but idle wakeups.
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(5);

impl SpawnedJob {
    /// The child's OS process id, for a signal-0 liveness probe independent
    /// of this handle's own state. Test-support only — mirrors
    /// `SpawnedLineSource::pid`.
    #[cfg(any(test, feature = "test-util"))]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Non-blocking: `Some` at most once, the moment the capture thread's
    /// single message — stdout, stderr, and the child's real exit status,
    /// already waited for — arrives (or, if that thread panicked before
    /// sending, synthesized as empty output with no status — the callback
    /// must still fire exactly once). Nothing left to reap here: unlike
    /// `SpawnedLineSource::finish`, the capture thread already confirmed
    /// the child exited before sending.
    pub fn try_take_result(&mut self) -> Option<JobResult> {
        let Some(rx) = &self.rx else {
            return None;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => JobResult {
                stdout: String::new(),
                stderr: String::new(),
                status: None,
            },
        };
        self.rx = None;
        Some(result)
    }
}

impl Drop for SpawnedJob {
    fn drop(&mut self) {
        self.child.reap();
        // Bounded channel (capacity 1): the job thread can be blocked
        // mid-`send` if `try_take_result` was never polled — dropping the
        // receiver makes that `send` return `Err`, letting the detached
        // thread exit on its own.
        self.rx = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_missing_binary_is_io_error() {
        let wake: WakeCallback = std::sync::Arc::new(|| {});
        assert!(spawn_job("definitely-not-a-real-binary-xyz", &[], None, wake).is_err());
    }

    #[test]
    fn nonexistent_cwd_is_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bogus = dir.path().join("does-not-exist");
        let wake: WakeCallback = std::sync::Arc::new(|| {});
        assert!(spawn_job("sh", &[], Some(&bogus), wake).is_err());
    }

    #[cfg(unix)]
    mod unix {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::{Duration, Instant};

        use super::*;

        fn no_op_wake() -> WakeCallback {
            Arc::new(|| {})
        }

        fn counting_wake() -> (WakeCallback, Arc<AtomicUsize>) {
            let count = Arc::new(AtomicUsize::new(0));
            let counted = Arc::clone(&count);
            let wake: WakeCallback = Arc::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            });
            (wake, count)
        }

        /// Polls `job` until its result arrives, with a generous bound so a
        /// slow CI box can't flake this — mirrors
        /// `line_source::tests::unix::drain_until_disconnected`.
        fn poll_until_result(job: &mut SpawnedJob) -> JobResult {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if let Some(result) = job.try_take_result() {
                    return result;
                }
                assert!(Instant::now() < deadline, "job never completed");
                thread::sleep(Duration::from_millis(10));
            }
        }

        #[test]
        fn happy_path_captures_stdout_and_success() {
            let args = vec!["-c".to_string(), "printf 'hi'".to_string()];
            let mut job = spawn_job("sh", &args, None, no_op_wake()).expect("spawn sh");
            let result = poll_until_result(&mut job);
            assert_eq!(result.stdout, "hi");
            assert_eq!(result.stderr, "");
            assert_eq!(result.status.map(|s| s.success()), Some(true));
        }

        #[test]
        fn nonzero_exit_and_stderr_are_captured() {
            let args = vec!["-c".to_string(), "echo oops >&2; exit 3".to_string()];
            let mut job = spawn_job("sh", &args, None, no_op_wake()).expect("spawn sh");
            let result = poll_until_result(&mut job);
            assert_eq!(result.status.and_then(|s| s.code()), Some(3));
            assert!(result.stderr.contains("oops"), "got: {:?}", result.stderr);
        }

        #[test]
        fn child_still_running_after_pipes_close_is_not_reported_as_killed() {
            // Closes both pipes, then keeps running for a bit before a
            // real, successful exit — both pipes reaching EOF must not be
            // mistaken for the child having exited (regression: it used to
            // be `reap()`ed right there, turning this into exit code -1).
            let args = vec![
                "-c".to_string(),
                "printf hi; exec 1>&- 2>&-; sleep 0.3; exit 0".to_string(),
            ];
            let mut job = spawn_job("sh", &args, None, no_op_wake()).expect("spawn sh");
            let result = poll_until_result(&mut job);
            assert_eq!(result.stdout, "hi");
            assert_eq!(
                result.status.and_then(|s| s.code()),
                Some(0),
                "child ran to a real exit(0) after closing its pipes, not a kill"
            );
        }

        #[test]
        fn stderr_capture_is_capped() {
            let args = vec![
                "-c".to_string(),
                "yes x | head -c 200000 1>&2; exit 0".to_string(),
            ];
            let mut job = spawn_job("sh", &args, None, no_op_wake()).expect("spawn sh");
            let result = poll_until_result(&mut job);
            assert!(result.stderr.len() <= STDERR_CAPTURE_CAP);
        }

        #[test]
        fn stdout_is_not_truncated() {
            let args = vec![
                "-c".to_string(),
                "yes x | head -c 200000; exit 0".to_string(),
            ];
            let mut job = spawn_job("sh", &args, None, no_op_wake()).expect("spawn sh");
            let result = poll_until_result(&mut job);
            assert_eq!(
                result.stdout.len(),
                200_000,
                "stdout must never be capped, unlike stderr"
            );
        }

        #[test]
        fn concurrent_large_stdout_and_stderr_does_not_deadlock() {
            // Both streams fill well past a pipe buffer at the same time —
            // this is exactly the shape a sequential stdout-then-stderr
            // read would deadlock on.
            let args = vec![
                "-c".to_string(),
                "(yes x | head -c 200000) & (yes y | head -c 200000 1>&2) & wait".to_string(),
            ];
            let mut job = spawn_job("sh", &args, None, no_op_wake()).expect("spawn sh");
            let result = poll_until_result(&mut job);
            assert_eq!(result.stdout.len(), 200_000);
            assert!(result.stderr.len() <= STDERR_CAPTURE_CAP);
        }

        #[test]
        fn wake_fires_exactly_once() {
            let (wake, count) = counting_wake();
            let args = vec!["-c".to_string(), "printf hi".to_string()];
            let mut job = spawn_job("sh", &args, None, wake).expect("spawn sh");
            poll_until_result(&mut job);
            assert_eq!(count.load(Ordering::SeqCst), 1);
        }

        #[test]
        fn self_signal_kill_reports_no_exit_code() {
            let args = vec!["-c".to_string(), "kill -9 $$".to_string()];
            let mut job = spawn_job("sh", &args, None, no_op_wake()).expect("spawn sh");
            let result = poll_until_result(&mut job);
            assert_eq!(
                result.status.and_then(|s| s.code()),
                None,
                "a signal-killed child has no exit code"
            );
        }

        #[test]
        fn drop_kills_the_child_promptly() {
            let args = vec!["30".to_string()];
            let job = spawn_job("sleep", &args, None, no_op_wake()).expect("spawn sleep");
            let pid = nix::unistd::Pid::from_raw(i32::try_from(job.pid()).expect("pid fits i32"));
            let started = Instant::now();
            drop(job);
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "Drop must not block for the child's remaining lifetime, took {:?}",
                started.elapsed()
            );
            assert!(
                nix::sys::signal::kill(pid, None).is_err(),
                "child must already be dead once Drop has returned"
            );
        }
    }
}
