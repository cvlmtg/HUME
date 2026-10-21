//! `pipe`'s process side: [`ShellBatch`] running one shell command per
//! input, and the wait for a pipe run. Unix-only because every
//! test runs `sh`.

use super::*;

use std::collections::VecDeque;
use std::io;
use std::num::NonZeroUsize;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::editor::lifecycle::FRAME_GAP;
use crate::editor::pipe_run::{
    Detached, PipeConsole, PipeEnd, PipeKind, PipeProgress, PipeRuns, RunId, WaitEvent,
};
use crate::editor::replay::Answer;
use crate::editor::settings::{SettingText, ShellCommand};
use crate::editor::shell_batch::{BatchProgress, Captured, ShellBatch, ShellFailure};

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

/// A command that logs `+` as it starts and `-` as it ends to `log`.
fn logging_command(log: &Path) -> String {
    format!(
        "echo + >> '{log}'; sleep 0.1; echo - >> '{log}'",
        log = log.display()
    )
}

/// How many processes `logging_command` saw start, and the most that ran
/// at once.
fn starts_and_peak(log: &Path) -> (usize, i32) {
    let events = std::fs::read_to_string(log).expect("log written");
    let mut running = 0i32;
    let mut peak = 0;
    for line in events.lines() {
        running += if line == "+" { 1 } else { -1 };
        peak = peak.max(running);
    }
    (events.lines().filter(|&line| line == "+").count(), peak)
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
    );
    let outputs = finish(&mut b, 2).expect("both succeed");
    assert_eq!(stdouts(&outputs), ["<slow>\n", "<fast>\n"]);
}

#[test]
fn progress_counts_finished_processes() {
    let _path = path_reader();
    let mut b = batch("cat", &["a", "b", "c"]);
    let BatchProgress::Running { done } = poll(&mut b, &mut 1).expect("starts") else {
        panic!("nothing can have finished before the first poll started it");
    };
    assert_eq!(done, 0);
    finish(&mut b, 1).expect("all succeed");
}

#[test]
fn the_limit_bounds_how_many_processes_run_at_once() {
    let _path = path_reader();
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("log");
    let command = logging_command(&log);
    let mut b = batch(&command, &["", "", "", "", "", ""]);
    finish(&mut b, 2).expect("all succeed");

    assert_eq!(starts_and_peak(&log), (6, 2));
}

#[test]
fn batches_sharing_one_budget_never_run_more_processes_together() {
    let _path = path_reader();
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
    let _path = path_reader();
    let mut b = batch(r#"echo boom >&2; exit 3"#, &["x"]);
    let ShellFailure(msg) = finish(&mut b, 1).expect_err("exit 3 is a failure");
    insta::assert_snapshot!(msg, @"`echo boom >&2; exit 3` failed (exit code 3): boom");
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
    let mut b = batch(&command, &["fail\n", "slow\n"]);
    finish(&mut b, 2).expect_err("one input fails");
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
    let mut b = batch(r"printf '\377'", &["x"]);
    let ShellFailure(msg) = finish(&mut b, 1).expect_err("invalid UTF-8");
    insta::assert_snapshot!(msg, @r"`printf '\377'` printed output that is not UTF-8");
}

#[test]
fn a_shell_that_cannot_start_fails_the_batch() {
    let _path = path_reader();
    let shell = ShellCommand::parse("definitely-not-a-real-shell-xyz -c").expect("a word list");
    let mut b = ShellBatch::new(&shell, "true", Path::new("."), vec!["x".to_string()]);
    let ShellFailure(msg) = finish(&mut b, 1).expect_err("no such program");
    assert!(
        msg.contains("definitely-not-a-real-shell-xyz"),
        "got: {msg}"
    );
}

#[test]
fn stderr_of_a_successful_process_is_kept() {
    let _path = path_reader();
    let mut b = batch("echo warn >&2; echo ok", &["x"]);
    let outputs = finish(&mut b, 1).expect("exit 0");
    assert_eq!(outputs[0].stdout, "ok\n");
    assert_eq!(outputs[0].stderr, "warn\n");
}

// ── Waiting for a pipe run ───────────────────────────────────────────────────────

/// A console that waits like a detached run, handing out `events` first
/// once the run has drawn its first frame (the wait has no timeout then),
/// and records the progress each repaint showed and when it drew. It leaves
/// `deferred` to the run loop.
struct ScriptedConsole {
    events: VecDeque<WaitEvent>,
    deferred: Vec<TerminalEvent>,
    detached: Detached,
    repaints: Vec<Option<PipeProgress>>,
    painted_at: Vec<Instant>,
}

impl ScriptedConsole {
    /// Waits on `ed`'s waker.
    fn new(ed: &Editor, events: impl IntoIterator<Item = WaitEvent>) -> Self {
        Self {
            events: events.into_iter().collect(),
            deferred: Vec::new(),
            detached: ed
                .detached
                .clone()
                .expect("a test editor has its own waker"),
            repaints: Vec::new(),
            painted_at: Vec::new(),
        }
    }
}

impl PipeConsole for ScriptedConsole {
    fn next(&mut self, timeout: Option<Duration>) -> io::Result<WaitEvent> {
        match timeout.is_none().then(|| self.events.pop_front()).flatten() {
            Some(event) => Ok(event),
            None => self.detached.next(timeout),
        }
    }

    fn repaint(&mut self, ed: &mut Editor) -> io::Result<()> {
        self.repaints
            .push(ed.state.pipe_runs.progress(ed.focused_buffer_id()));
        self.painted_at.push(Instant::now());
        Ok(())
    }

    fn take_deferred(&mut self) -> Vec<TerminalEvent> {
        std::mem::take(&mut self.deferred)
    }
}

fn start_run(ed: &mut Editor, command: &str, kind: PipeKind) -> RunId {
    let (pid, bid) = (ed.view.focused_pane(), ed.focused_buffer_id());
    ed.state
        .start_pipe_run(pid, bid, command.to_string(), kind, None)
}

fn progress(command: &str, done: usize, total: usize) -> Option<PipeProgress> {
    Some(PipeProgress {
        command: command.to_string(),
        done,
        total,
    })
}

fn run_with(ed: &mut Editor, run: RunId, console: &mut ScriptedConsole) {
    ed.wait_pipe_run(run, console)
        .expect("a scripted console never fails");
    assert!(!ed.doc().is_busy(), "the wait ends the run");
}

#[test]
fn the_output_lands_on_the_selections_the_run_started_from() {
    let _path = path_reader();
    let mut ed = editor_from("-[ab]> cd\n");
    let run = start_run(&mut ed, "tr a-z A-Z", PipeKind::Replace);
    ed.execute_keymap_command("select-next-word".into(), None, false);
    assert_ne!(state(&ed), "-[ab]> cd\n", "setup: the selection moved");
    let mut console = ScriptedConsole::new(&ed, []);
    run_with(&mut ed, run, &mut console);
    assert_eq!(state(&ed), "-[AB]> cd\n");
}

#[test]
fn a_wait_leaves_the_input_it_did_not_handle_to_the_run_loop_in_order() {
    let _path = path_reader();
    let mut ed = editor_from("-[abc]>\n");
    let run = start_run(&mut ed, "cat", PipeKind::Replace);
    let mut console = ScriptedConsole::new(&ed, []);
    console.deferred = vec![TerminalEvent::Key(key('x')), TerminalEvent::Key(key('y'))];
    run_with(&mut ed, run, &mut console);
    assert_eq!(
        ed.input_backlog,
        [TerminalEvent::Key(key('x')), TerminalEvent::Key(key('y'))]
    );
}

#[test]
fn a_cancelled_wait_queues_its_input_behind_the_input_already_waiting() {
    let _path = path_reader();
    let mut ed = editor_from("-[abc]>\n");
    ed.input_backlog.push_back(TerminalEvent::Key(key('a')));
    let run = start_run(&mut ed, "sleep 5; cat", PipeKind::Replace);
    let mut console = ScriptedConsole::new(&ed, [WaitEvent::Interrupt]);
    console.deferred = vec![TerminalEvent::Key(key('x'))];
    run_with(&mut ed, run, &mut console);
    assert_eq!(state(&ed), "-[abc]>\n", "setup: the run was cancelled");
    assert_eq!(
        ed.input_backlog,
        [TerminalEvent::Key(key('a')), TerminalEvent::Key(key('x'))]
    );
}

#[test]
fn a_run_replaces_the_selection_with_its_output() {
    let _path = path_reader();
    let mut ed = editor_from("-[b\na]>\nz\n");
    let run = start_run(&mut ed, "sort", PipeKind::Replace);
    let mut console = ScriptedConsole::new(&ed, []);
    run_with(&mut ed, run, &mut console);
    assert_eq!(state(&ed), "-[a\nb]>\nz\n");
}

#[test]
fn a_run_that_ends_within_the_grace_draws_no_frame() {
    let _path = path_reader();
    let mut ed = editor_from("-[abc]>\n");
    let run = start_run(&mut ed, "cat", PipeKind::Replace);
    let mut console = ScriptedConsole::new(&ed, []);
    run_with(&mut ed, run, &mut console);
    assert_eq!(state(&ed), "-[abc]>\n");
    assert_eq!(console.repaints, []);
}

#[test]
fn progress_repaints_as_processes_finish() {
    let _path = path_reader();
    let mut ed = editor_from("-{fast}> -[mid]> -[slow]>\n");
    let command = r#"read x; case $x in mid) sleep 0.4;; slow) sleep 0.8;; esac; printf '%s' "$x""#;
    let run = start_run(&mut ed, command, PipeKind::Replace);
    let mut console = ScriptedConsole::new(&ed, []);
    run_with(&mut ed, run, &mut console);
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
    let run = start_run(&mut ed, "sleep 0.15; cat", PipeKind::Replace);
    let mut console = ScriptedConsole::new(&ed, []);
    run_with(&mut ed, run, &mut console);
    assert_eq!(state(&ed), "-{a}> -[b]> -[c]> -[d]> -[e]> -[f]>\n");
    let gaps: Vec<Duration> = console
        .painted_at
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect();
    assert!(
        gaps.iter().all(|&gap| gap >= FRAME_GAP),
        "gaps between progress frames: {gaps:?}"
    );
}

#[test]
fn a_resize_during_the_wait_repaints() {
    let _path = path_reader();
    let mut ed = editor_from("-[abc]>\n");
    let run = start_run(&mut ed, "sleep 0.5; cat", PipeKind::Replace);
    let mut console = ScriptedConsole::new(&ed, [WaitEvent::Resized]);
    run_with(&mut ed, run, &mut console);
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
    let run = start_run(&mut ed, "sleep 30; echo late", PipeKind::Replace);
    let started = Instant::now();
    let mut console = ScriptedConsole::new(&ed, [WaitEvent::Interrupt]);
    run_with(&mut ed, run, &mut console);
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
    let run = start_run(&mut ed, "sleep 30; echo late", PipeKind::Replace);
    let started = Instant::now();
    let mut console = ScriptedConsole::new(&ed, []);
    run_with(&mut ed, run, &mut console);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(state(&ed), "-[abc]>\n");
}

// ── `|` and `Ctrl-|` end to end ───────────────────────────────────────────────

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
}

impl PipeConsole for InterruptOnceStarted {
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

/// Replaces the selection with `x`, the change a failed pipe must leave in
/// place.
fn replace_with_x(ed: &mut Editor) {
    ed.feed_key(key('r'));
    ed.feed_key(key('x'));
    assert_eq!(last_command(ed), Some("replace"));
}

#[test]
fn a_failed_pipe_keeps_the_previous_change() {
    let _path = path_reader();
    let mut ed = editor_from("-[a]>\n");
    replace_with_x(&mut ed);
    ed.feed_key(key('|'));
    type_cmd(&mut ed, "exit 3");
    assert!(has_error(&ed, "exit"));
    assert_eq!(last_command(&ed), Some("replace"));
}

#[test]
fn an_interrupted_pipe_keeps_the_previous_change() {
    let _path = path_reader();
    let mut ed = editor_from("-[a]>\n");
    replace_with_x(&mut ed);
    ed.inline_pipe_console = Some(Box::new(ScriptedConsole::new(&ed, [WaitEvent::Interrupt])));
    ed.feed_key(key('|'));
    type_cmd(&mut ed, "sleep 30");
    assert_eq!(status(&ed), "pipe cancelled");
    assert_eq!(last_command(&ed), Some("replace"));
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
    assert_eq!(last_command(&ed), Some("insert-at-selection-start"));
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
    ed.inline_pipe_console = Some(Box::new(ScriptedConsole::new(&ed, [WaitEvent::Interrupt])));
    ed.feed_key(key('q'));
    ed.feed_key(key('q'));
    assert_eq!(state(&ed), "-[b]>\n");
    assert_eq!(status(&ed), "pipe cancelled");
    assert!(!ed.state.macros.has_queued_keys());
}

#[test]
fn an_interrupted_dot_in_a_macro_stops_the_macro() {
    let _path = path_reader();
    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("slow");
    let command = format!(
        "if [ -e '{}' ]; then sleep 30; fi; tr a-z A-Z",
        marker.display()
    );
    let mut ed = editor_from("-[a]>\n");
    ed.feed_key(key('|'));
    type_cmd(&mut ed, &command);
    ed.feed_key(key('Q'));
    ed.feed_key(key('Q'));
    ed.feed_key(key('.'));
    ed.feed_key(key('r'));
    ed.feed_key(key('x'));
    ed.feed_key(key('Q'));
    assert_eq!(state(&ed), "-[x]>\n");

    ed.feed_key(key('|'));
    type_cmd(&mut ed, &command);
    assert_eq!(state(&ed), "-[X]>\n");
    std::fs::write(&marker, "").expect("marker written");
    ed.inline_pipe_console = Some(Box::new(ScriptedConsole::new(&ed, [WaitEvent::Interrupt])));
    ed.feed_key(key('q'));
    ed.feed_key(key('q'));
    assert_eq!(state(&ed), "-[X]>\n");
    assert!(!ed.state.macros.has_queued_keys());
}

#[test]
fn an_interrupt_during_dot_stops_the_rest_of_its_insert_session() {
    let _path = path_reader();
    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("slow");
    let command = format!(
        "if [ -e '{}' ]; then sleep 30; fi; printf v",
        marker.display()
    );
    let mut ed = editor_from_kitty("-[z]>\n");
    ed.feed_key(key('i'));
    type_chars(&mut ed, "a");
    ed.feed_key(key_ctrl('|'));
    type_cmd(&mut ed, &command);
    type_chars(&mut ed, "b");
    ed.feed_key(key_esc());
    assert_eq!(text(&ed), "avbz\n");

    std::fs::write(&marker, "").expect("marker written");
    ed.inline_pipe_console = Some(Box::new(ScriptedConsole::new(&ed, [WaitEvent::Interrupt])));
    ed.feed_key(key('.'));
    assert_eq!(text(&ed), "aavbz\n");
    assert_eq!(ed.state.mode(), Mode::Normal);
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

// ── Background runs ───────────────────────────────────────────────────────────

#[test]
fn bar_runs_in_the_background_and_lands() {
    let _path = path_reader();
    let mut ed = editor_from("-[b\na]>\n");
    let bid = ed.focused_buffer_id();
    type_pipe(&mut ed, "sort");
    assert!(busy(&ed, bid), "the run is in flight after Enter");
    assert_eq!(state(&ed), "-[b\na]>\n");
    drain_until(&mut ed, |ed| !busy(ed, bid));
    assert_eq!(state(&ed), "-[a\nb]>\n");
}

#[test]
fn keys_move_selections_meanwhile_and_the_output_lands_on_the_started_ones() {
    let _path = path_reader();
    let mut ed = editor_from("-[ab]> cd\n");
    let bid = ed.focused_buffer_id();
    type_pipe(&mut ed, "sleep 0.2; tr a-z A-Z");
    ed.handle_key(key('w'));
    assert_ne!(state(&ed), "-[ab]> cd\n", "the selection moved meanwhile");
    drain_until(&mut ed, |ed| !busy(ed, bid));
    assert_eq!(state(&ed), "-[AB]> cd\n");
}

#[test]
fn the_output_lands_in_the_buffer_the_pane_left() {
    let _path = path_reader();
    let mut ed = editor_from("-[ab]>\n");
    let bid = ed.focused_buffer_id();
    type_pipe(&mut ed, "sleep 0.2; tr a-z A-Z");
    open_other_buffer(&mut ed, "x\n");
    drain_until(&mut ed, |ed| !busy(ed, bid));
    assert_eq!(ed.state.buffers.get(bid).text().to_string(), "AB\n");
    assert_eq!(text(&ed), "x\n");
}

#[test]
fn a_failed_background_run_reports_naming_the_buffer_and_keeps_the_previous_change() {
    let _path = path_reader();
    let mut ed = editor_from("-[a]>\n");
    let bid = ed.focused_buffer_id();
    ed.handle_key(key('r'));
    ed.handle_key(key('x'));
    type_pipe(&mut ed, "exit 3");
    drain_until(&mut ed, |ed| !busy(ed, bid));
    let name = ed.doc().display_name();
    assert!(
        error_log(&ed)
            .iter()
            .any(|e| e.starts_with(&format!("pipe in {name}: ")) && e.contains("exit code 3")),
        "got {:?}",
        error_log(&ed)
    );
    assert_eq!(last_command(&ed), Some("replace"));
}

#[test]
fn the_indicator_shows_after_the_grace_for_the_focused_buffer_only() {
    let _path = path_reader();
    let mut ed = editor_from("-[a]>\n");
    let bid = ed.focused_buffer_id();
    type_pipe(&mut ed, "sleep 0.5; cat");
    assert_eq!(ed.state.pipe_runs.progress(bid), None, "within the grace");
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        ed.state.pipe_runs.progress(bid),
        progress("sleep 0.5; cat", 0, 1)
    );
    let other = open_other_buffer(&mut ed, "x\n");
    assert_eq!(ed.state.pipe_runs.progress(other), None);
}

#[test]
fn runs_in_two_buffers_share_one_process_cap() {
    let _path = path_reader();
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("log");
    let mut ed = editor_from("-{a}> -[b]> -[c]>\n");
    ed.state.pipe_runs = PipeRuns::with_limit(NonZeroUsize::new(2).expect("two is not zero"));
    let first = ed.focused_buffer_id();
    type_pipe(&mut ed, &logging_command(&log));
    let second = open_other_buffer(&mut ed, "a b c\n");
    ed.handle_key(key('%'));
    ed.handle_key(key('s'));
    ed.handle_key(key('\\'));
    ed.handle_key(key('w'));
    ed.handle_key(key_enter());
    type_pipe(&mut ed, &logging_command(&log));
    assert!(busy(&ed, first) && busy(&ed, second), "setup: both run");
    drain_until(&mut ed, |ed| !busy(ed, first) && !busy(ed, second));
    assert_eq!(starts_and_peak(&log), (6, 2));
}

#[test]
fn a_waited_run_takes_the_next_free_slot_before_an_older_background_run() {
    let _path = path_reader();
    let mut ed = editor_from("-{a}> -[b]> -[c]> -[d]> -[e]>\n");
    ed.state.pipe_runs = PipeRuns::with_limit(NonZeroUsize::MIN);
    let background = ed.focused_buffer_id();
    type_pipe(&mut ed, "sleep 0.5; cat");
    ed.settle();
    // Past the statusline grace, with the first process still in the slot.
    std::thread::sleep(Duration::from_millis(150));
    open_other_buffer(&mut ed, "x\n");
    ed.feed_key(key('|'));
    type_cmd(&mut ed, "tr a-z A-Z");
    assert_eq!(text(&ed), "X\n");
    let left = ed
        .state
        .pipe_runs
        .progress(background)
        .expect("the background run still has processes to run");
    assert!(left.done < left.total, "{left:?}");
}

#[test]
fn a_background_landing_leaves_a_kill_for_smart_paste() {
    let _path = path_reader();
    let mut ed = editor_from("-[b\na]>\n");
    seed_clipboard(&mut ed, "CLIP");
    let sorted = ed.focused_buffer_id();
    type_pipe(&mut ed, "sort");
    open_other_buffer(&mut ed, "xy\n");
    ed.feed_key(key('d'));
    drain_until(&mut ed, |ed| !busy(ed, sorted));
    assert_eq!(
        ed.state.buffers.get(sorted).text().to_string(),
        "a\nb\n",
        "setup: the run landed"
    );
    ed.feed_key(key('p'));
    assert_eq!(text(&ed), "yx\n");
}

#[test]
fn step_waits_only_for_the_run_its_key_started() {
    let _path = path_reader();
    let mut ed = editor_from("-[a]>\n");
    let first = ed.focused_buffer_id();
    type_pipe(&mut ed, "sleep 1; cat");
    let second = open_other_buffer(&mut ed, "b\n");
    ed.feed_key(key('|'));
    type_cmd(&mut ed, "tr a-z A-Z");
    assert_eq!(text(&ed), "B\n");
    assert!(busy(&ed, first), "the other buffer's run goes on");
    assert!(!busy(&ed, second));
}

/// Type a `|` of a command that writes its pid to a file in `dir`, then
/// sleeps, and return that pid once the run has started its process.
fn start_sleeper(ed: &mut Editor, dir: &Path) -> u32 {
    let pid_file = dir.join("pid");
    type_pipe(
        ed,
        &format!("echo $$ > '{}'; exec sleep 30", pid_file.display()),
    );
    drain_until(ed, |_| {
        std::fs::read_to_string(&pid_file).is_ok_and(|s| s.ends_with('\n'))
    });
    std::fs::read_to_string(&pid_file)
        .expect("pid written")
        .trim()
        .parse()
        .expect("a pid")
}

#[test]
fn ctrl_c_kills_the_background_processes() {
    let _path = path_reader();
    let dir = tempfile::tempdir().expect("tempdir");
    let mut ed = editor_from("-[a]>\n");
    let pid = start_sleeper(&mut ed, dir.path());
    ed.handle_key(key_ctrl('c'));
    assert!(!ed.doc().is_busy());
    assert!(!process_is_alive(pid), "the process is gone after Ctrl-c");
    assert_eq!(state(&ed), "-[a]>\n");
}

#[test]
fn e_bang_stops_the_run_and_reloads() {
    let _path = path_reader();
    let dir = tempfile::tempdir().expect("tempdir");
    let (mut ed, file) = editor_with_file("-[h]>ello\n", "hello\n");
    let pid = start_sleeper(&mut ed, dir.path());
    std::fs::write(&file, "HELLO!!\n").expect("file rewritten");
    type_cmd(&mut ed, ":e!");
    assert!(!ed.doc().is_busy(), "the run stopped");
    assert!(!process_is_alive(pid), "its process is gone");
    assert_eq!(text(&ed), "HELLO!!\n");
}

#[test]
fn close_buffer_from_steel_raises_while_the_run_is_in_flight() {
    let _path = path_reader();
    let dir = tempfile::tempdir().expect("tempdir");
    let mut ed = editor_from("-[a]>\n");
    run(
        &mut ed,
        dir.path(),
        r#"(define-command! "closer" "" (lambda (pane) (close-buffer! pane)))"#,
    );
    let bid = ed.focused_buffer_id();
    let pid = start_sleeper(&mut ed, dir.path());
    ed.execute_keymap_command("closer".into(), None, false);
    assert!(ed.state.buffers.try_get(bid).is_some(), "the buffer stays");
    assert!(process_is_alive(pid), "its run goes on");
    assert!(has_error(
        &ed,
        &format!(
            r#"close-buffer!: {} is busy: a pipe is running. Stop it first with (call! "cancel-pipe" pane)"#,
            ed.doc().display_name()
        )
    ));
}

#[test]
fn calling_cancel_pipe_then_close_buffer_from_steel_closes_it() {
    let _path = path_reader();
    let dir = tempfile::tempdir().expect("tempdir");
    let mut ed = editor_from("-[a]>\n");
    run(
        &mut ed,
        dir.path(),
        r#"(define-command! "closer" "" (lambda (pane)
             (call! "cancel-pipe" pane)
             (close-buffer! pane)))"#,
    );
    let bid = ed.focused_buffer_id();
    let pid = start_sleeper(&mut ed, dir.path());
    ed.execute_keymap_command("closer".into(), None, false);
    assert!(ed.state.buffers.try_get(bid).is_none(), "the buffer closed");
    assert!(!process_is_alive(pid), "its run's process is gone");
}

#[test]
fn calling_cancel_pipe_says_whether_it_stopped_a_run() {
    let _path = path_reader();
    let dir = tempfile::tempdir().expect("tempdir");
    let mut ed = editor_from("-[a]>\n");
    let bid = ed.focused_buffer_id();
    let pid = start_sleeper(&mut ed, dir.path());
    run(
        &mut ed,
        dir.path(),
        r#"(define-command! "stopper" "" (lambda (pane)
             (let* ((first (call! "cancel-pipe" pane))
                    (second (call! "cancel-pipe" pane)))
               (log! 'warn (if (and first (not second)) "stopped once" "wrong")))))"#,
    );
    ed.execute_keymap_command("stopper".into(), None, false);
    assert!(!busy(&ed, bid));
    assert!(!process_is_alive(pid), "its run's process is gone");
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.text == "stopped once"),
        "#t for the run it stopped, then #f"
    );
}

/// Stops `bid`'s run when it draws, as a drawer's render proc calling
/// `(call! "cancel-pipe" pane)` would.
struct CancelsOnRepaint {
    detached: Detached,
    bid: BufferId,
}

impl PipeConsole for CancelsOnRepaint {
    fn next(&mut self, timeout: Option<Duration>) -> io::Result<WaitEvent> {
        self.detached.next(timeout)
    }

    fn repaint(&mut self, ed: &mut Editor) -> io::Result<()> {
        ed.state.cancel_pipe_run(self.bid);
        Ok(())
    }
}

#[test]
fn a_wait_whose_run_is_cancelled_while_it_draws_ends_cancelled() {
    let _path = path_reader();
    let mut ed = editor_from("-[a]>\n");
    let bid = ed.focused_buffer_id();
    let run = start_run(&mut ed, "sleep 30", PipeKind::Replace);
    let mut console = CancelsOnRepaint {
        detached: ed
            .detached
            .clone()
            .expect("a test editor has its own waker"),
        bid,
    };
    let end = ed
        .wait_pipe_run(run, &mut console)
        .expect("the console never fails");
    assert_eq!(end, PipeEnd::Cancelled);
    assert!(!busy(&ed, bid));
}

#[test]
fn an_older_run_that_landed_survives_a_newer_one_withdrawn() {
    let _path = path_reader();
    let mut ed = editor_from("-[a]>\n");
    let first = ed.focused_buffer_id();
    for k in ['r', 'x'] {
        ed.handle_key(key(k));
    }
    type_pipe(&mut ed, "tr a-z A-Z");
    let second = open_other_buffer(&mut ed, "b\n");
    type_pipe(&mut ed, "sleep 30; cat");
    drain_until(&mut ed, |ed| !busy(ed, first));
    assert_eq!(ed.state.buffers.get(first).text().to_string(), "X\n");
    ed.handle_key(key_ctrl('c'));
    assert!(!busy(&ed, second));
    assert_eq!(
        last_command(&ed),
        Some("pipe"),
        "the landed run's change, not the replace before it"
    );
}
