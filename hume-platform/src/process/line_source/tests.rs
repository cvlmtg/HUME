use super::*;
use crate::wake::no_op_wake;

#[test]
fn splits_multiple_lines_in_one_chunk() {
    let mut s = LineSplitter::new(b'\n');
    assert_eq!(s.push_chunk(b"a\nb\nc\n"), vec!["a", "b", "c"]);
    assert_eq!(s.finish(), None);
}

#[test]
fn carries_a_partial_line_across_chunk_boundary() {
    let mut s = LineSplitter::new(b'\n');
    assert_eq!(s.push_chunk(b"ab"), Vec::<String>::new());
    assert_eq!(s.push_chunk(b"c\nd\n"), vec!["abc", "d"]);
    assert_eq!(s.finish(), None);
}

#[test]
fn carries_across_a_chunk_boundary_that_lands_exactly_on_a_delimiter() {
    let mut s = LineSplitter::new(b'\n');
    assert_eq!(s.push_chunk(b"abc\n"), vec!["abc"]);
    assert_eq!(s.push_chunk(b"def\n"), vec!["def"]);
}

#[test]
fn delimiter_as_first_byte_of_a_later_chunk_still_closes_the_carry() {
    let mut s = LineSplitter::new(b'\n');
    assert_eq!(s.push_chunk(b"abc"), Vec::<String>::new());
    assert_eq!(s.push_chunk(b"\ndef\n"), vec!["abc", "def"]);
}

#[test]
fn nul_mode_preserves_interior_newlines_and_carriage_returns() {
    let mut s = LineSplitter::new(b'\0');
    assert_eq!(s.push_chunk(b"a\r\n\0b\0"), vec!["a\r\n", "b"]);
}

#[test]
fn newline_mode_strips_trailing_carriage_return() {
    let mut s = LineSplitter::new(b'\n');
    assert_eq!(s.push_chunk(b"a\r\nb\r\n"), vec!["a", "b"]);
}

#[test]
fn finish_emits_trailing_unterminated_line() {
    let mut s = LineSplitter::new(b'\n');
    assert_eq!(s.push_chunk(b"a\nb"), vec!["a"]);
    assert_eq!(s.finish(), Some("b".to_string()));
}

#[test]
fn finish_after_a_cleanly_terminated_stream_is_none() {
    let mut s = LineSplitter::new(b'\n');
    assert_eq!(s.push_chunk(b"a\n"), vec!["a"]);
    assert_eq!(s.finish(), None);
}

#[test]
fn interior_empty_lines_are_emitted() {
    let mut s = LineSplitter::new(b'\n');
    assert_eq!(s.push_chunk(b"a\n\nb\n"), vec!["a", "", "b"]);
}

#[test]
fn a_line_past_the_cap_is_truncated_and_the_next_line_is_whole() {
    let mut s = LineSplitter::new(b'\n');
    let half = vec![b'x'; MAX_LINE_BYTES / 2 + 1];
    assert_eq!(s.push_chunk(&half), Vec::<String>::new());
    assert_eq!(s.push_chunk(&half), Vec::<String>::new());
    let lines = s.push_chunk(b"tail\nnext\n");
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].len(), MAX_LINE_BYTES);
    assert!(lines[0].bytes().all(|b| b == b'x'));
    assert_eq!(lines[1], "next");
}

#[test]
fn invalid_utf8_is_lossy_replaced_not_dropped() {
    let mut s = LineSplitter::new(b'\n');
    let lines = s.push_chunk(b"\xffbad\n");
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains('\u{FFFD}'), "got: {:?}", lines[0]);
}

// ── spawn_line_source ─────────────────────────────────────────────────

#[test]
fn spawn_missing_binary_is_io_error() {
    assert!(
        spawn_line_source(
            "definitely-not-a-real-binary-xyz",
            &[],
            Path::new("."),
            b'\n',
            no_op_wake()
        )
        .is_err()
    );
}

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::process::test_support::is_alive;
    use crate::wake::counting_wake;
    use std::time::Instant;

    /// Drains `source` until the reader disconnects, with a generous
    /// bound so a slow CI box can't flake this.
    fn drain_until_disconnected(source: &mut SpawnedLineSource) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut lines = Vec::new();
        loop {
            let (batch, disconnected) = source.try_recv_batches();
            lines.extend(batch);
            if disconnected {
                return lines;
            }
            assert!(Instant::now() < deadline, "reader never disconnected");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn happy_path_streams_lines_and_reports_success() {
        let args = vec!["-c".to_string(), "printf 'a\\nb\\nc'".to_string()];
        let mut source =
            spawn_line_source("sh", &args, Path::new("."), b'\n', no_op_wake()).expect("spawn sh");
        let lines = drain_until_disconnected(&mut source);
        assert_eq!(lines, vec!["a", "b", "c"]);
        let exit = source.finish();
        assert_eq!(exit.status.map(|s| s.success()), Some(true));
        assert_eq!(exit.stderr, "");
    }

    #[test]
    fn nul_delimited_output_splits_on_nul() {
        let args = vec!["-c".to_string(), "printf 'x\\0y\\0z'".to_string()];
        let mut source =
            spawn_line_source("sh", &args, Path::new("."), b'\0', no_op_wake()).expect("spawn sh");
        let lines = drain_until_disconnected(&mut source);
        assert_eq!(lines, vec!["x", "y", "z"]);
    }

    #[test]
    fn wake_is_called_after_a_batch_arrives() {
        let (wake, woken) = counting_wake();
        let args = vec!["-c".to_string(), "printf 'a\\n'".to_string()];
        let mut source =
            spawn_line_source("sh", &args, Path::new("."), b'\n', wake).expect("spawn sh");
        drain_until_disconnected(&mut source);
        assert!(woken.try_iter().count() > 0, "wake must fire at least once");
    }

    #[test]
    fn nonzero_exit_and_stderr_are_captured() {
        let args = vec!["-c".to_string(), "echo oops >&2; exit 3".to_string()];
        let mut source =
            spawn_line_source("sh", &args, Path::new("."), b'\n', no_op_wake()).expect("spawn sh");
        drain_until_disconnected(&mut source);
        let exit = source.finish();
        assert_eq!(exit.status.and_then(|s| s.code()), Some(3));
        assert!(exit.stderr.contains("oops"), "got: {:?}", exit.stderr);
    }

    #[test]
    fn stderr_capture_is_capped() {
        // Emit well over the cap in stderr; the thread must still drain
        // to EOF promptly (not block on the child) and the capture must
        // not exceed the cap.
        let args = vec![
            "-c".to_string(),
            "yes x | head -c 200000 1>&2; exit 0".to_string(),
        ];
        let mut source =
            spawn_line_source("sh", &args, Path::new("."), b'\n', no_op_wake()).expect("spawn sh");
        drain_until_disconnected(&mut source);
        let exit = source.finish();
        assert!(exit.stderr.len() <= STDERR_CAPTURE_CAP);
    }

    #[test]
    fn finish_kills_a_child_that_lingers_after_closing_stdout_without_polling() {
        // Closes stdout immediately (triggering `disconnected`) but keeps
        // running: the exact shape `finish()` must not busy-wait on: it
        // should kill the child right away rather than polling for a
        // grace period before falling back to `kill`. The second `exec`
        // replaces the shell's own process image instead of forking a
        // `sleep` child, so killing the tracked pid actually terminates
        // the process holding stderr open (a plain trailing `sleep 30`
        // would fork, leaving an orphaned `sleep` that `kill` can't
        // reach and that would hold the stderr pipe open for real).
        let args = vec!["-c".to_string(), "exec 1>&-; exec sleep 30".to_string()];
        let mut source =
            spawn_line_source("sh", &args, Path::new("."), b'\n', no_op_wake()).expect("spawn sh");
        drain_until_disconnected(&mut source);
        let pid = source.child.id();

        // 5s, not a tight bound on the kill/reap/stderr-EOF handoff
        // itself (which spans two thread wakeups and is scheduler-
        // sensitive). It only needs to rule out `finish()` waiting the
        // child out instead of killing it, which would take the full 30s
        // the child sleeps for. Matches the margin `drop_kills_the_child_
        // promptly` below uses against the same kind of child.
        let started = Instant::now();
        let exit = source.finish();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "finish() must kill a lingering child immediately, not wait it out, took {:?}",
            started.elapsed()
        );
        assert!(
            exit.status.is_some(),
            "the killed child must still be reaped, not left as unknown"
        );
        assert!(
            !is_alive(pid),
            "child must already be dead once finish() has returned"
        );
    }

    #[test]
    fn drop_kills_the_child_promptly() {
        // `sleep 30` makes a missing `kill()` observable two ways: the
        // signal-liveness check below (a `wait()`-only Drop still reaps
        // it, just 30s later) AND (the check that actually catches
        // that case) `drop()` itself must return promptly, not block
        // for the child's remaining lifetime.
        let args = vec!["30".to_string()];
        let source = spawn_line_source("sleep", &args, Path::new("."), b'\n', no_op_wake())
            .expect("spawn sleep");
        let pid = source.child.id();
        let started = Instant::now();
        drop(source);
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

    #[test]
    fn nonexistent_cwd_is_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bogus = dir.path().join("does-not-exist");
        assert!(spawn_line_source("sh", &[], &bogus, b'\n', no_op_wake()).is_err());
    }
}
