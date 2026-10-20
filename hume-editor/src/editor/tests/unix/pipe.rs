//! `pipe`'s process side: [`ShellBatch`] running one shell command per
//! input, and the wait that runs a queued pipe. Unix-only because every
//! test runs `sh`.

use super::*;

use std::collections::VecDeque;
use std::io;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hume_platform::process::WakeCallback;

use crate::editor::pipe_run::{
    Detached, PROGRESS_FRAME_GAP, PipeConsole, PipeKind, PipeProgress, QueuedPipe, WaitEvent,
};
use crate::editor::replay::Answer;
use crate::editor::settings::{SettingText, ShellCommand};
use crate::editor::shell_batch::{BatchProgress, Captured, ShellBatch, ShellFailure};

fn no_op_wake() -> WakeCallback {
    Arc::new(|| {})
}

fn batch(command: &str, inputs: &[&str], limit: usize) -> ShellBatch {
    ShellBatch::new(
        &ShellCommand::default(),
        command,
        Path::new("."),
        inputs.iter().map(|s| s.to_string()).collect(),
        NonZeroUsize::new(limit).expect("a test limit is at least 1"),
        no_op_wake(),
    )
}

/// Polls `batch` to the end, with a bound so a stuck child fails the test.
fn finish(batch: &mut ShellBatch) -> Result<Vec<Captured>, ShellFailure> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let BatchProgress::Finished(outputs) = batch.poll()? {
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
    let _path = path_reader();
    // The first input's process finishes last.
    let mut b = batch(
        r#"read x; [ "$x" = slow ] && sleep 0.2; echo "<$x>""#,
        &["slow\n", "fast\n"],
        2,
    );
    let outputs = finish(&mut b).expect("both succeed");
    assert_eq!(stdouts(&outputs), ["<slow>\n", "<fast>\n"]);
}

#[test]
fn progress_counts_finished_processes() {
    let _path = path_reader();
    let mut b = batch("cat", &["a", "b", "c"], 1);
    let BatchProgress::Running { done } = b.poll().expect("starts") else {
        panic!("nothing can have finished before the first poll started it");
    };
    assert_eq!(done, 0);
    finish(&mut b).expect("all succeed");
}

#[test]
fn the_limit_bounds_how_many_processes_run_at_once() {
    let _path = path_reader();
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("log");
    let command = format!(
        "echo + >> '{log}'; sleep 0.1; echo - >> '{log}'",
        log = log.display()
    );
    let mut b = batch(&command, &["", "", "", "", "", ""], 2);
    finish(&mut b).expect("all succeed");

    let events = std::fs::read_to_string(&log).expect("log written");
    let mut running = 0i32;
    let mut peak = 0;
    for line in events.lines() {
        running += if line == "+" { 1 } else { -1 };
        peak = peak.max(running);
    }
    assert_eq!(events.lines().count(), 12);
    assert_eq!(peak, 2);
}

#[test]
fn a_failing_process_fails_the_batch_with_its_exit_code_and_stderr() {
    let _path = path_reader();
    let mut b = batch(r#"echo boom >&2; exit 3"#, &["x"], 1);
    let ShellFailure(msg) = finish(&mut b).expect_err("exit 3 is a failure");
    assert!(msg.contains("exit code 3"), "got: {msg}");
    assert!(msg.contains("boom"), "got: {msg}");
}

#[test]
fn the_first_failure_ends_the_batch_and_dropping_it_kills_the_rest() {
    let _path = path_reader();
    let dir = tempfile::tempdir().expect("tempdir");
    let pid_file = dir.path().join("pid");
    let command = format!(
        r#"read x; if [ "$x" = fail ]; then sleep 0.3; exit 1; fi; echo $$ > '{}'; exec sleep 30"#,
        pid_file.display()
    );
    let mut b = batch(&command, &["fail\n", "slow\n"], 2);
    finish(&mut b).expect_err("one input fails");
    drop(b);

    let pid: u32 = std::fs::read_to_string(&pid_file)
        .expect("the slow process started before the failure")
        .trim()
        .parse()
        .expect("a pid");
    assert!(
        !process_is_alive(pid),
        "the slow process must be gone once the batch is dropped"
    );
}

#[test]
fn output_that_is_not_utf8_fails_the_batch() {
    let _path = path_reader();
    let mut b = batch(r"printf '\377'", &["x"], 1);
    let ShellFailure(msg) = finish(&mut b).expect_err("invalid UTF-8");
    assert!(msg.contains("UTF-8"), "got: {msg}");
}

#[test]
fn a_shell_that_cannot_start_fails_the_batch() {
    let _path = path_reader();
    let shell = ShellCommand::parse("definitely-not-a-real-shell-xyz -c").expect("a word list");
    let mut b = ShellBatch::new(
        &shell,
        "true",
        Path::new("."),
        vec!["x".to_string()],
        NonZeroUsize::MIN,
        no_op_wake(),
    );
    let ShellFailure(msg) = finish(&mut b).expect_err("no such program");
    assert!(
        msg.contains("definitely-not-a-real-shell-xyz"),
        "got: {msg}"
    );
}

#[test]
fn stderr_of_a_successful_process_is_kept() {
    let _path = path_reader();
    let mut b = batch("echo warn >&2; echo ok", &["x"], 1);
    let outputs = finish(&mut b).expect("exit 0");
    assert_eq!(outputs[0].stdout, "ok\n");
    assert_eq!(outputs[0].stderr, "warn\n");
}

// ── Waiting for a queued pipe ─────────────────────────────────────────────────

/// A console that waits like a detached run, handing out `events` first
/// once the run has drawn its first frame (the wait has no timeout then),
/// and records the progress each repaint showed and when it drew.
struct ScriptedConsole {
    events: VecDeque<WaitEvent>,
    detached: Detached,
    repaints: Vec<Option<PipeProgress>>,
    painted_at: Vec<Instant>,
}

impl ScriptedConsole {
    fn new(events: impl IntoIterator<Item = WaitEvent>) -> Self {
        Self {
            events: events.into_iter().collect(),
            detached: Detached::new(),
            repaints: Vec::new(),
            painted_at: Vec::new(),
        }
    }
}

impl PipeConsole for ScriptedConsole {
    fn wake(&self) -> WakeCallback {
        self.detached.wake()
    }

    fn next(&mut self, timeout: Option<Duration>) -> io::Result<WaitEvent> {
        match timeout.is_none().then(|| self.events.pop_front()).flatten() {
            Some(event) => Ok(event),
            None => self.detached.next(timeout),
        }
    }

    fn repaint(&mut self, ed: &mut Editor) -> io::Result<()> {
        self.repaints.push(ed.state.running_pipe.clone());
        self.painted_at.push(Instant::now());
        Ok(())
    }
}

fn queue_pipe(ed: &mut Editor, command: &str, kind: PipeKind) {
    ed.state.queued_pipe = Some(QueuedPipe {
        pane: focused_pane(ed),
        command: command.to_string(),
        kind,
        settles: None,
    });
}

fn progress(command: &str, done: usize, total: usize) -> Option<PipeProgress> {
    Some(PipeProgress {
        command: command.to_string(),
        done,
        total,
    })
}

fn run_with(ed: &mut Editor, console: &mut ScriptedConsole) {
    ed.run_queued_pipe(console)
        .expect("a scripted console never fails");
    assert!(ed.state.queued_pipe.is_none(), "the run takes the queue");
    assert!(
        ed.state.running_pipe.is_none(),
        "the indicator ends with the run"
    );
}

#[test]
fn a_queued_pipe_replaces_the_selection_with_its_output() {
    let _path = path_reader();
    let mut ed = editor_from("-[b\na]>\nz\n");
    queue_pipe(&mut ed, "sort", PipeKind::Replace);
    let mut console = ScriptedConsole::new([]);
    run_with(&mut ed, &mut console);
    assert_eq!(state(&ed), "-[a\nb]>\nz\n");
}

#[test]
fn a_run_that_ends_within_the_grace_draws_no_frame() {
    let _path = path_reader();
    let mut ed = editor_from("-[abc]>\n");
    queue_pipe(&mut ed, "cat", PipeKind::Replace);
    let mut console = ScriptedConsole::new([]);
    run_with(&mut ed, &mut console);
    assert_eq!(state(&ed), "-[abc]>\n");
    assert_eq!(console.repaints, []);
}

#[test]
fn progress_repaints_as_processes_finish() {
    let _path = path_reader();
    let mut ed = editor_from("-{fast}> -[mid]> -[slow]>\n");
    let command = r#"read x; case $x in mid) sleep 0.4;; slow) sleep 0.8;; esac; printf '%s' "$x""#;
    queue_pipe(&mut ed, command, PipeKind::Replace);
    let mut console = ScriptedConsole::new([]);
    run_with(&mut ed, &mut console);
    let done: Vec<usize> = console
        .repaints
        .iter()
        .map(|shown| {
            let shown = shown.as_ref().expect("a repaint shows the run");
            assert_eq!((shown.command.as_str(), shown.total), (command, 3));
            shown.done
        })
        .collect();
    assert!(
        done.is_sorted_by(|a, b| a < b),
        "one frame per change: {done:?}"
    );
    assert_eq!(
        done.last(),
        Some(&2),
        "the last frame before the end: {done:?}"
    );
}

#[test]
fn progress_frames_are_at_least_a_frame_gap_apart() {
    let _path = path_reader();
    let mut ed = editor_from("-{a}> -[b]> -[c]> -[d]> -[e]> -[f]>\n");
    queue_pipe(&mut ed, "sleep 0.15; cat", PipeKind::Replace);
    let mut console = ScriptedConsole::new([]);
    run_with(&mut ed, &mut console);
    assert_eq!(state(&ed), "-{a}> -[b]> -[c]> -[d]> -[e]> -[f]>\n");
    let gaps: Vec<Duration> = console
        .painted_at
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect();
    assert!(
        gaps.iter().all(|&gap| gap >= PROGRESS_FRAME_GAP),
        "gaps between progress frames: {gaps:?}"
    );
}

#[test]
fn a_resize_during_the_wait_repaints() {
    let _path = path_reader();
    let mut ed = editor_from("-[abc]>\n");
    queue_pipe(&mut ed, "sleep 0.5; cat", PipeKind::Replace);
    let mut console = ScriptedConsole::new([WaitEvent::Resized]);
    run_with(&mut ed, &mut console);
    assert_eq!(
        console.repaints,
        [
            progress("sleep 0.5; cat", 0, 1),
            progress("sleep 0.5; cat", 0, 1)
        ]
    );
}

#[test]
fn an_interrupt_cancels_the_run_and_changes_nothing() {
    let _path = path_reader();
    let mut ed = editor_from("-[abc]>\n");
    queue_pipe(&mut ed, "sleep 30; echo late", PipeKind::Replace);
    let started = Instant::now();
    run_with(&mut ed, &mut ScriptedConsole::new([WaitEvent::Interrupt]));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(state(&ed), "-[abc]>\n");
    assert_eq!(status(&ed), "pipe cancelled");
}

#[test]
fn a_termination_request_cancels_the_run() {
    let _path = path_reader();
    let mut ed = editor_from("-[abc]>\n");
    ed.state
        .terminate_exit_code
        .store(143, std::sync::atomic::Ordering::Release);
    queue_pipe(&mut ed, "sleep 30; echo late", PipeKind::Replace);
    let started = Instant::now();
    run_with(&mut ed, &mut ScriptedConsole::new([]));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(state(&ed), "-[abc]>\n");
}

// ── `|` and `Ctrl-|` end to end ───────────────────────────────────────────────

fn text(ed: &Editor) -> String {
    ed.doc().text().to_string()
}

#[test]
fn bar_replaces_each_selection_and_one_undo_restores_them() {
    let _path = path_reader();
    let mut ed = editor_from("-{ab}> -[cd]>\n");
    ed.feed_key(key('|'));
    type_cmd(&mut ed, "tr a-z A-Z");
    assert_eq!(state(&ed), "-{AB}> -[CD]>\n");
    ed.feed_key(key('u'));
    assert_eq!(text(&ed), "ab cd\n");
}

#[test]
fn ctrl_bar_inserts_at_each_cursor_and_joins_the_insert_session() {
    let _path = path_reader();
    let mut ed = editor_from_kitty("-{a}> -[b]>\n");
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('|'));
    type_cmd(&mut ed, "echo x");
    assert_eq!(text(&ed), "xa xb\n");
    assert_eq!(ed.state.mode(), Mode::Insert);
    ed.feed_key(key('y'));
    ed.feed_key(key_esc());
    assert_eq!(text(&ed), "xya xyb\n");
    ed.feed_key(key('u'));
    assert_eq!(text(&ed), "a b\n");
}

#[test]
fn one_failing_process_changes_nothing_and_logs_its_stderr() {
    let _path = path_reader();
    let mut ed = editor_from("-{a}> -[b]>\n");
    ed.feed_key(key('|'));
    type_cmd(
        &mut ed,
        r#"if [ "$(cat)" = b ]; then echo broken >&2; exit 2; fi; echo ok"#,
    );
    assert_eq!(state(&ed), "-{a}> -[b]>\n");
    assert!(has_error(&ed, "broken"));
    assert!(!ed.doc().can_undo());
}

#[test]
fn output_identical_to_the_input_adds_no_undo_step() {
    let _path = path_reader();
    let mut ed = editor_from("-[abc]>\n");
    ed.feed_key(key('|'));
    type_cmd(&mut ed, "cat");
    assert_eq!(state(&ed), "-[abc]>\n");
    assert!(!ed.doc().can_undo());
}

#[test]
fn stderr_of_a_successful_run_is_logged_at_trace() {
    let _path = path_reader();
    let mut ed = editor_from("-[abc]>\n");
    ed.feed_key(key('|'));
    type_cmd(&mut ed, "echo note >&2; cat");
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Trace && e.text.contains("note"))
    );
    assert!(error_log(&ed).is_empty());
}

// ── `.` ───────────────────────────────────────────────────────────────────────

/// A command printing how many times it has run, counted in a file under
/// `dir`.
fn counting_command(dir: &Path) -> String {
    format!(
        r#"n=$(cat '{count}' 2>/dev/null || echo 0); n=$((n + 1)); echo $n > '{count}'; printf 'v%s' $n"#,
        count = dir.join("count").display()
    )
}

#[test]
fn dot_runs_the_command_again() {
    let _path = path_reader();
    let dir = tempfile::tempdir().expect("tempdir");
    let mut ed = editor_from("-[a]>\n");
    ed.feed_key(key('|'));
    type_cmd(&mut ed, &counting_command(dir.path()));
    assert_eq!(state(&ed), "-[v1]>\n");
    ed.feed_key(key('.'));
    assert_eq!(state(&ed), "-[v2]>\n");
}

#[test]
fn dot_replays_an_insert_session_with_its_command_in_order() {
    let _path = path_reader();
    let dir = tempfile::tempdir().expect("tempdir");
    let mut ed = editor_from_kitty("-[z]>\n");
    ed.feed_key(key('i'));
    type_chars(&mut ed, "a");
    ed.feed_key(key_ctrl('|'));
    type_cmd(&mut ed, &counting_command(dir.path()));
    type_chars(&mut ed, "b");
    ed.feed_key(key_esc());
    assert_eq!(text(&ed), "av1bz\n");
    ed.feed_key(key('.'));
    assert_eq!(text(&ed), "av2bav1bz\n");
}

/// Interrupts the wait once the process has written its pid to `pid_file`.
struct InterruptOnceStarted {
    pid_file: std::path::PathBuf,
    detached: Detached,
}

impl PipeConsole for InterruptOnceStarted {
    fn wake(&self) -> WakeCallback {
        self.detached.wake()
    }

    fn next(&mut self, _timeout: Option<Duration>) -> io::Result<WaitEvent> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.pid_file.exists() {
            assert!(Instant::now() < deadline, "the process never started");
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(WaitEvent::Interrupt)
    }
}

#[test]
fn an_interrupt_during_dot_kills_the_run_and_changes_nothing() {
    let _path = path_reader();
    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("slow");
    let pid_file = dir.path().join("pid");
    let command = format!(
        r#"if [ -e '{marker}' ]; then echo $$ > '{pid}.tmp'; mv '{pid}.tmp' '{pid}'; exec sleep 30; fi; tr a-z A-Z"#,
        marker = marker.display(),
        pid = pid_file.display()
    );
    let mut ed = editor_from("-[a]>\n");
    ed.feed_key(key('|'));
    type_cmd(&mut ed, &command);
    assert_eq!(state(&ed), "-[A]>\n");

    std::fs::write(&marker, "").expect("marker written");
    ed.inline_pipe_console = Some(Box::new(InterruptOnceStarted {
        pid_file: pid_file.clone(),
        detached: Detached::new(),
    }));
    ed.feed_key(key('.'));
    assert_eq!(state(&ed), "-[A]>\n");
    assert_eq!(status(&ed), "pipe cancelled");
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .expect("the process started")
        .trim()
        .parse()
        .expect("a pid");
    assert!(!process_is_alive(pid), "the cancelled process must be gone");

    ed.feed_key(key('u'));
    assert_eq!(text(&ed), "a\n");
    assert!(!ed.doc().can_undo());
}

/// The command `.` would repeat now.
fn last_command(ed: &Editor) -> Option<String> {
    ed.state.last_action().map(|a| a.command.to_string())
}

/// Replaces the selection with `x`, the change a failed pipe must leave in
/// place.
fn replace_with_x(ed: &mut Editor) {
    ed.feed_key(key('r'));
    ed.feed_key(key('x'));
    assert_eq!(last_command(ed).as_deref(), Some("replace"));
}

#[test]
fn a_failed_pipe_keeps_the_previous_change() {
    let _path = path_reader();
    let mut ed = editor_from("-[a]>\n");
    replace_with_x(&mut ed);
    ed.feed_key(key('|'));
    type_cmd(&mut ed, "exit 3");
    assert!(has_error(&ed, "exit"));
    assert_eq!(last_command(&ed).as_deref(), Some("replace"));
}

#[test]
fn an_interrupted_pipe_keeps_the_previous_change() {
    let _path = path_reader();
    let mut ed = editor_from("-[a]>\n");
    replace_with_x(&mut ed);
    ed.inline_pipe_console = Some(Box::new(ScriptedConsole::new([WaitEvent::Interrupt])));
    ed.feed_key(key('|'));
    type_cmd(&mut ed, "sleep 30");
    assert_eq!(status(&ed), "pipe cancelled");
    assert_eq!(last_command(&ed).as_deref(), Some("replace"));
}

#[test]
fn a_failed_pipe_with_no_previous_change_leaves_none() {
    let _path = path_reader();
    let mut ed = editor_from("-[a]>\n");
    ed.feed_key(key('|'));
    type_cmd(&mut ed, "exit 3");
    assert_eq!(last_command(&ed), None);
}

#[test]
fn a_pipe_that_ran_replaces_the_previous_change() {
    let _path = path_reader();
    let mut ed = editor_from("-[a]>\n");
    replace_with_x(&mut ed);
    ed.feed_key(key('|'));
    type_cmd(&mut ed, "tr a-z A-Z");
    assert_eq!(state(&ed), "-[X]>\n");
    let action = ed.state.last_action().expect("the pipe is the last change");
    assert_eq!(action.command.as_ref(), "pipe");
    assert!(
        matches!(action.answers.as_slice(), [Answer::Prompt(Some(cmd))] if cmd == "tr a-z A-Z"),
        "answers: {:?}",
        action.answers
    );
}

#[test]
fn a_failed_ctrl_bar_keeps_its_insert_session_change() {
    let _path = path_reader();
    let mut ed = editor_from_kitty("-[z]>\n");
    ed.feed_key(key('i'));
    type_chars(&mut ed, "a");
    ed.feed_key(key_ctrl('|'));
    type_cmd(&mut ed, "exit 1");
    type_chars(&mut ed, "b");
    ed.feed_key(key_esc());
    assert_eq!(text(&ed), "abz\n");
    assert_eq!(
        last_command(&ed).as_deref(),
        Some("insert-at-selection-start")
    );
}

#[test]
fn an_interrupted_pipe_in_a_macro_stops_the_macro() {
    let _path = path_reader();
    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("slow");
    let command = format!(
        "if [ -e '{}' ]; then sleep 30; fi; tr a-z A-Z",
        marker.display()
    );
    let mut ed = editor_from("-[a]>\n");
    ed.feed_key(key('Q'));
    ed.feed_key(key('Q'));
    ed.feed_key(key('|'));
    type_cmd(&mut ed, &command);
    ed.feed_key(key('r'));
    ed.feed_key(key('x'));
    ed.feed_key(key('Q'));
    assert_eq!(state(&ed), "-[x]>\n");

    ed.feed_key(key('r'));
    ed.feed_key(key('b'));
    std::fs::write(&marker, "").expect("marker written");
    ed.inline_pipe_console = Some(Box::new(ScriptedConsole::new([WaitEvent::Interrupt])));
    ed.feed_key(key('q'));
    ed.feed_key(key('q'));
    assert_eq!(state(&ed), "-[b]>\n");
    assert_eq!(status(&ed), "pipe cancelled");
    assert!(!ed.state.macros.has_queued_keys());
}

#[test]
fn a_failed_pipe_in_a_macro_lets_the_macro_go_on() {
    let _path = path_reader();
    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("fail");
    let command = format!("if [ -e '{}' ]; then exit 1; fi; cat", marker.display());
    let mut ed = editor_from("-[a]>\n");
    ed.feed_key(key('Q'));
    ed.feed_key(key('Q'));
    ed.feed_key(key('|'));
    type_cmd(&mut ed, &command);
    ed.feed_key(key('r'));
    ed.feed_key(key('x'));
    ed.feed_key(key('Q'));

    ed.feed_key(key('r'));
    ed.feed_key(key('b'));
    std::fs::write(&marker, "").expect("marker written");
    ed.feed_key(key('q'));
    ed.feed_key(key('q'));
    assert_eq!(state(&ed), "-[x]>\n");
}
