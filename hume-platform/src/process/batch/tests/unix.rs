//! Unix-only tests, gated once at the `mod unix;` declaration in the
//! parent.

use std::path::Path;
use std::time::{Duration, Instant};

use super::super::*;
use crate::process::batch::{logging_command, starts_and_peak};
use crate::process::test_support::is_alive;
use crate::wake::no_op_wake;

fn batch(command: &str, inputs: &[&str]) -> ShellBatch {
    ShellBatch::new(
        &ShellCommand::default(),
        command,
        Path::new("."),
        inputs.iter().map(|s| s.to_string()).collect(),
    )
}

/// Collect `batch`'s finished processes, then start what `free` allows.
fn poll(batch: &mut ShellBatch, free: &mut usize) -> Result<BatchProgress, ShellFailure> {
    batch.collect(free)?;
    batch.fill(free, &no_op_wake())
}

/// Polls `batch` to the end with `limit` process slots of its own, with a
/// bound so a stuck child fails the test.
fn finish(batch: &mut ShellBatch, limit: usize) -> Result<Vec<Captured>, ShellFailure> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut free = limit - batch.running();
        if let BatchProgress::Finished(outputs) = poll(batch, &mut free)? {
            return Ok(outputs);
        }
        assert!(Instant::now() < deadline, "batch never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn stdouts(outputs: &[Captured]) -> Vec<&str> {
    outputs.iter().map(|c| c.stdout.as_str()).collect()
}

#[test]
fn each_input_gets_its_own_process_and_outputs_keep_input_order() {
    // The first input's process finishes last.
    let mut b = batch(
        r#"read x; [ "$x" = slow ] && sleep 0.2; echo "<$x>""#,
        &["slow\n", "fast\n"],
    );
    let outputs = finish(&mut b, 2).expect("both succeed");
    assert_eq!(stdouts(&outputs), ["<slow>\n", "<fast>\n"]);
}

#[test]
fn progress_counts_finished_processes() {
    let mut b = batch("cat", &["a", "b", "c"]);
    let BatchProgress::Running { done } = poll(&mut b, &mut 1).expect("starts") else {
        panic!("nothing can have finished before the first poll started it");
    };
    assert_eq!(done, 0);
    finish(&mut b, 1).expect("all succeed");
}

#[test]
fn the_limit_bounds_how_many_processes_run_at_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("log");
    let command = logging_command(&log);
    let mut b = batch(&command, &["", "", "", "", "", ""]);
    finish(&mut b, 2).expect("all succeed");

    assert_eq!(starts_and_peak(&log), (6, 2));
}

#[test]
fn batches_sharing_one_budget_never_run_more_processes_together() {
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("log");
    let command = logging_command(&log);
    let mut first = batch(&command, &["", "", ""]);
    let mut second = batch(&command, &["", "", ""]);
    let deadline = Instant::now() + Duration::from_secs(5);
    let (mut first_done, mut second_done) = (false, false);
    while !(first_done && second_done) {
        let mut free = 2 - first.running() - second.running();
        first_done = first_done
            || matches!(
                poll(&mut first, &mut free).expect("succeeds"),
                BatchProgress::Finished(_)
            );
        second_done = second_done
            || matches!(
                poll(&mut second, &mut free).expect("succeeds"),
                BatchProgress::Finished(_)
            );
        assert!(Instant::now() < deadline, "batches never finished");
        std::thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(starts_and_peak(&log), (6, 2));
}

#[test]
fn a_failing_process_fails_the_batch_with_its_exit_code_and_stderr() {
    let mut b = batch(r#"echo boom >&2; exit 3"#, &["x"]);
    let ShellFailure(msg) = finish(&mut b, 1).expect_err("exit 3 is a failure");
    insta::assert_snapshot!(msg, @"`echo boom >&2; exit 3` failed (exit code 3): boom");
}

#[test]
fn the_first_failure_ends_the_batch_and_dropping_it_kills_the_rest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pid_file = dir.path().join("pid");
    let command = format!(
        r#"read x; if [ "$x" = fail ]; then sleep 0.3; exit 1; fi; echo $$ > '{}'; exec sleep 30"#,
        pid_file.display()
    );
    let mut b = batch(&command, &["fail\n", "slow\n"]);
    finish(&mut b, 2).expect_err("one input fails");
    drop(b);

    let pid: i32 = std::fs::read_to_string(&pid_file)
        .expect("the slow process started before the failure")
        .trim()
        .parse()
        .expect("a pid");
    assert!(
        !is_alive(pid),
        "the slow process must be gone once the batch is dropped"
    );
}

#[test]
fn output_that_is_not_utf8_fails_the_batch() {
    let mut b = batch(r"printf '\377'", &["x"]);
    let ShellFailure(msg) = finish(&mut b, 1).expect_err("invalid UTF-8");
    insta::assert_snapshot!(msg, @r"`printf '\377'` printed output that is not UTF-8");
}

#[test]
fn a_shell_that_cannot_start_fails_the_batch() {
    let shell = ShellCommand::from_words(vec![
        "definitely-not-a-real-shell-xyz".to_string(),
        "-c".to_string(),
    ])
    .expect("a program");
    let mut b = ShellBatch::new(&shell, "true", Path::new("."), vec!["x".to_string()]);
    let ShellFailure(msg) = finish(&mut b, 1).expect_err("no such program");
    assert!(
        msg.contains("definitely-not-a-real-shell-xyz"),
        "got: {msg}"
    );
}

#[test]
fn stderr_of_a_successful_process_is_kept() {
    let mut b = batch("echo warn >&2; echo ok", &["x"]);
    let outputs = finish(&mut b, 1).expect("exit 0");
    assert_eq!(outputs[0].stdout, "ok\n");
    assert_eq!(outputs[0].stderr, "warn\n");
}
