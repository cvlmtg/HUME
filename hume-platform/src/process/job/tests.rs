use std::path::Path;

use super::*;
use crate::wake::no_op_wake;

#[test]
fn a_missing_binary_fails_to_spawn_naming_the_program() {
    let Err(e) = spawn_job(
        "definitely-not-a-real-binary-xyz",
        &[],
        Launch::with_process_path(Path::new(".")),
        Vec::new(),
        no_op_wake(),
    ) else {
        panic!("a missing binary cannot spawn");
    };
    assert_eq!(e.kind(), io::ErrorKind::NotFound);
    assert!(
        e.to_string()
            .starts_with("cannot run 'definitely-not-a-real-binary-xyz': "),
        "got: {e}"
    );
}

#[test]
fn nonexistent_cwd_is_reported_as_the_missing_directory() {
    let Err(e) = spawn_job(
        "sh",
        &[],
        Launch::with_process_path(Path::new("hume-no-such-dir")),
        Vec::new(),
        no_op_wake(),
    ) else {
        panic!("a missing working directory cannot spawn");
    };
    insta::assert_snapshot!(e, @"cannot run 'sh': directory 'hume-no-such-dir' does not exist");
}

#[cfg(unix)]
mod unix {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::process::test_support::{is_alive, kept_stderr};
    use crate::wake::counting_wake;

    fn spawn_sh(script: &str) -> SpawnedJob {
        spawn_sh_with(script, Vec::new(), no_op_wake())
    }

    fn spawn_sh_with(script: &str, input: Vec<u8>, wake: WakeCallback) -> SpawnedJob {
        let args = ["-c".to_string(), script.to_string()];
        spawn_job(
            "sh",
            &args,
            Launch::with_process_path(Path::new(".")),
            input,
            wake,
        )
        .expect("spawn sh")
    }

    /// Polls `job` until its result arrives, with a generous bound so a
    /// slow CI box can't flake this. Mirrors
    /// `line_source::tests::unix::drain_until_disconnected`.
    fn poll_until_result(job: &mut SpawnedJob) -> JobResult {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = job.try_take_result() {
                return result;
            }
            assert!(Instant::now() < deadline, "job never completed");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn happy_path_captures_stdout_and_success() {
        let mut job = spawn_sh("printf 'hi'");
        let result = poll_until_result(&mut job);
        assert_eq!(result.stdout, b"hi");
        assert_eq!(result.stderr, "");
        assert_eq!(result.status.map(|s| s.success()), Some(true));
    }

    #[test]
    fn nonzero_exit_and_stderr_are_captured() {
        let mut job = spawn_sh("echo oops >&2; exit 3");
        let result = poll_until_result(&mut job);
        assert_eq!(result.status.and_then(|s| s.code()), Some(3));
        assert!(result.stderr.contains("oops"), "got: {:?}", result.stderr);
    }

    #[test]
    fn child_still_running_after_pipes_close_is_not_reported_as_killed() {
        // Closes both pipes, then keeps running for a bit before a
        // real, successful exit. Both pipes reaching EOF must not be
        // mistaken for the child having exited (reaping it right there
        // would kill it, leaving no exit code).
        let mut job = spawn_sh("printf hi; exec 1>&- 2>&-; sleep 0.3; exit 0");
        let result = poll_until_result(&mut job);
        assert_eq!(result.stdout, b"hi");
        assert_eq!(
            result.status.and_then(|s| s.code()),
            Some(0),
            "child ran to a real exit(0) after closing its pipes, not a kill"
        );
    }

    #[test]
    fn stderr_capture_is_capped() {
        let mut job = spawn_sh("yes x | head -c 200000 1>&2; exit 0");
        let result = poll_until_result(&mut job);
        assert_eq!(result.stderr, kept_stderr(&"x\n".repeat(100_000)));
    }

    #[test]
    fn stdout_is_not_truncated() {
        let mut job = spawn_sh("yes x | head -c 200000; exit 0");
        let result = poll_until_result(&mut job);
        assert_eq!(
            result.stdout.len(),
            200_000,
            "stdout must never be capped, unlike stderr"
        );
    }

    #[test]
    fn concurrent_large_stdout_and_stderr_does_not_deadlock() {
        // Both streams fill well past a pipe buffer at the same time:
        // this is exactly the shape a sequential stdout-then-stderr
        // read would deadlock on.
        let mut job = spawn_sh("(yes x | head -c 200000) & (yes y | head -c 200000 1>&2) & wait");
        let result = poll_until_result(&mut job);
        assert_eq!(result.stdout.len(), 200_000);
        assert_eq!(result.stderr, kept_stderr(&"y\n".repeat(100_000)));
    }

    #[test]
    fn wake_fires_exactly_once() {
        let (wake, woken) = counting_wake();
        let mut job = spawn_sh_with("printf hi", Vec::new(), wake);
        poll_until_result(&mut job);
        assert_eq!(woken.try_iter().count(), 1);
    }

    #[test]
    fn self_signal_kill_reports_no_exit_code() {
        let mut job = spawn_sh("kill -9 $$");
        let result = poll_until_result(&mut job);
        assert_eq!(
            result.status.and_then(|s| s.code()),
            None,
            "a signal-killed child has no exit code"
        );
    }

    #[test]
    fn stdin_input_reaches_the_child() {
        let mut job = spawn_job(
            "cat",
            &[],
            Launch::with_process_path(Path::new(".")),
            b"one\ntwo".to_vec(),
            no_op_wake(),
        )
        .expect("spawn cat");
        let result = poll_until_result(&mut job);
        assert_eq!(result.stdout, b"one\ntwo");
        assert_eq!(result.status.map(|s| s.success()), Some(true));
    }

    #[test]
    fn large_stdin_and_stdout_do_not_deadlock() {
        // Both directions exceed a pipe buffer many times over: writing
        // all of stdin before reading stdout would block forever.
        let input = vec![b'x'; 1024 * 1024];
        let mut job = spawn_job(
            "cat",
            &[],
            Launch::with_process_path(Path::new(".")),
            input.clone(),
            no_op_wake(),
        )
        .expect("spawn cat");
        let result = poll_until_result(&mut job);
        assert_eq!(result.stdout, input);
    }

    #[test]
    fn a_child_that_stops_reading_stdin_still_succeeds() {
        let input = vec![b'y'; 1024 * 1024];
        let mut job = spawn_sh_with("head -c 1", input, no_op_wake());
        let result = poll_until_result(&mut job);
        assert_eq!(result.stdout, b"y");
        assert_eq!(result.status.map(|s| s.success()), Some(true));
    }

    #[test]
    fn stdout_bytes_are_delivered_undecoded() {
        let mut job = spawn_sh(r"printf '\377'");
        let result = poll_until_result(&mut job);
        assert_eq!(result.stdout, [0xff]);
    }

    #[test]
    fn drop_kills_the_child_promptly() {
        let args = vec!["30".to_string()];
        let job = spawn_job(
            "sleep",
            &args,
            Launch::with_process_path(Path::new(".")),
            Vec::new(),
            no_op_wake(),
        )
        .expect("spawn sleep");
        let pid = job.pid();
        let started = Instant::now();
        drop(job);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "Drop must not block for the child's remaining lifetime, took {:?}",
            started.elapsed()
        );
        assert!(
            !is_alive(pid),
            "child must already be dead once Drop has returned"
        );
    }
}
