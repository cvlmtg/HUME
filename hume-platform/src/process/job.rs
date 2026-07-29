//! One-shot subprocess capture: run a command to completion and deliver its
//! whole stdout, whole stderr, and exit status exactly once — the shape
//! `spawn-async!` needs, as opposed to `line_source`'s per-line streaming.
//!
//! Completion here is the child *exiting*, not stdout reaching EOF (as it
//! is for `line_source`, which drains on a per-frame budget and can't
//! block). The capture thread this module spawns is free to block, so it
//! reads stdout to EOF, joins the stderr thread, and only then sends the
//! result — the main thread never reaps a lingering child on a schedule
//! ([`SpawnedJob::try_take_result`]'s job/`line_source::SpawnedLineSource`'s
//! `finish` split is exactly this: the capture thread owns waiting for
//! output, the main thread owns reaping the exit status once output is
//! known to be complete).
//!
//! Two capture threads, not one: reading stdout to EOF and then stderr
//! would deadlock on a child that fills its stderr pipe while this thread
//! is still blocked reading stdout — the same hazard the Scheme sync path
//! documents at `runtime/plugins/core/pickers/plugin.scm`'s
//! `pickers/run-stdout-raw`.

use std::io;
use std::path::Path;
use std::process::ExitStatus;
use std::sync::mpsc;
use std::thread;

use crate::process::child::{STDERR_CAPTURE_CAP, WakeOnDrop, read_capped, spawn_piped};
use crate::process::tracked::TrackedChild;

pub use crate::process::child::WakeCallback;

/// The complete output of a finished [`SpawnedJob`]: whole stdout (never
/// truncated — it's the caller's data, not a diagnostic), whole stderr
/// (capped at [`STDERR_CAPTURE_CAP`] — diagnostic only), and exit status
/// (`None` only if the OS gave none back even after a kill+wait fallback —
/// vanishingly rare).
pub struct JobResult {
    pub stdout: String,
    pub stderr: String,
    pub status: Option<ExitStatus>,
}

/// What the capture thread hands to the main thread — everything except the
/// exit status, which [`SpawnedJob::try_take_result`] reaps itself once
/// this arrives (see its doc for why that's not done on the capture
/// thread).
struct Captured {
    stdout: String,
    stderr: String,
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
    cmd: String,
    child: TrackedChild,
    rx: Option<mpsc::Receiver<Captured>>,
    /// Detached (not joined) on drop — same rationale as
    /// `SpawnedLineSource::threads`: nothing here needs to flush before
    /// exit.
    threads: Vec<thread::JoinHandle<()>>,
}

/// Spawns `cmd` with `args` (direct argv, no shell), piped stdio, stdin
/// closed immediately. One thread reads stderr to EOF (capped); a second
/// reads stdout to EOF (uncapped), joins the first, then sends the combined
/// capture once and fires `wake`.
pub fn spawn_job(
    cmd: &str,
    args: &[String],
    cwd: Option<&Path>,
    wake: WakeCallback,
) -> io::Result<SpawnedJob> {
    let (child, stdout, stderr) = spawn_piped(cmd, args, cwd)?;

    let (tx, rx) = mpsc::sync_channel::<Captured>(1);

    let stderr_thread = thread::Builder::new()
        .name("hume-job-stderr".into())
        .spawn(move || read_capped(stderr, STDERR_CAPTURE_CAP))?;

    let job_thread = thread::Builder::new()
        .name("hume-job".into())
        .spawn(move || {
            // Fires as this closure returns, after the send below — a
            // panicking read still wakes the drain to observe the
            // synthesized result `try_take_result` produces on disconnect.
            let _wake_on_drop = WakeOnDrop(wake);
            let stdout_bytes = read_capped(stdout, usize::MAX);
            // A panicked stderr thread degrades to empty stderr rather than
            // wedging this job forever — the exit status still carries the
            // failure, and stdout is what most callers actually want.
            let stderr_bytes = stderr_thread.join().unwrap_or_default();
            let captured = Captured {
                stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
                stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
            };
            let _ = tx.send(captured);
        })?;

    Ok(SpawnedJob {
        cmd: cmd.to_string(),
        child: TrackedChild::new(child.into_inner()),
        rx: Some(rx),
        threads: vec![job_thread],
    })
}

impl SpawnedJob {
    pub fn cmd(&self) -> &str {
        &self.cmd
    }

    /// The child's OS process id, for a signal-0 liveness probe independent
    /// of this handle's own state. Test-support only: every caller is
    /// cross-crate test code, so this can't be `#[cfg(test)]`-gated the way
    /// a same-crate test-only method would be — mirrors
    /// `SpawnedLineSource::pid`.
    #[doc(hidden)]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Non-blocking: `Some` at most once, the moment the capture thread's
    /// single message arrives (or, if that thread panicked before sending,
    /// synthesized as empty output — the callback must still fire exactly
    /// once). Reaps the exit status here, not on the capture thread: both
    /// pipes are already at EOF by the time this message lands, so the
    /// child has almost always already exited and `try_wait` returns
    /// immediately; the rare child that lingers is killed right away rather
    /// than waited for, same tradeoff as `SpawnedLineSource::finish`.
    pub fn try_take_result(&mut self) -> Option<JobResult> {
        let Some(rx) = &self.rx else {
            return None;
        };
        let captured = match rx.try_recv() {
            Ok(captured) => Some(captured),
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => None,
        };
        self.rx = None;
        let Captured { stdout, stderr } = captured.unwrap_or_else(|| Captured {
            stdout: String::new(),
            stderr: String::new(),
        });
        let status = match self.child.try_wait() {
            Ok(Some(status)) => Some(status),
            Ok(None) => self.child.reap(),
            Err(_) => None,
        };
        Some(JobResult {
            stdout,
            stderr,
            status,
        })
    }
}

impl Drop for SpawnedJob {
    fn drop(&mut self) {
        self.child.reap();
        // Bounded channel (capacity 1): the job thread can be blocked
        // mid-`send` if `try_take_result` was never polled — dropping the
        // receiver makes that `send` return `Err`, letting the thread exit
        // even though it's detached rather than joined above.
        self.rx = None;
        self.threads.clear();
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
