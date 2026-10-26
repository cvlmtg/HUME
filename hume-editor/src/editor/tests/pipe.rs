//! `pipe`'s prompt and command up to the started run. Keys go through
//! `handle_key`, which leaves the run in flight for the main loop to wait
//! for, before any process starts; the runs themselves are in
//! `unix/pipe.rs`.

use std::path::Path;

use super::*;
use crate::editor::input_stack::shell_prompt::ShellPromptLayer;
use crate::editor::keymap::BindMode;
use crate::editor::minibuf::history::HistoryKind;
use crate::editor::pipe_run::{PipeKind, PipeRunning, WaitEvent, wait_event};
use termina::WindowSize;
use termina::event::KeyEventKind;

/// The focused buffer's run's command and kind, taking the run out.
fn take_run(ed: &mut Editor) -> Option<(String, PipeKind)> {
    let bid = ed.focused_buffer_id();
    ed.state
        .pipe_runs
        .take_buffer(bid)
        .map(|launch| (launch.command, launch.kind))
}

fn prompt_label(ed: &Editor) -> Option<String> {
    ed.state.minibuf().map(|mb| mb.prompt.clone())
}

#[test]
fn bar_prompts_for_a_command_and_enter_starts_a_replace() {
    let mut ed = editor_from("-[abc]>\n");
    ed.handle_key(key('|'));
    assert_eq!(prompt_label(&ed).as_deref(), Some("|"));
    assert_eq!(ed.state.mode(), Mode::Normal);
    keys(&mut ed, "sort -u");
    ed.handle_key(key_enter());
    assert!(ed.state.minibuf().is_none());
    assert_eq!(
        take_run(&mut ed),
        Some(("sort -u".to_string(), PipeKind::Replace))
    );
}

#[test]
fn ctrl_bar_prompts_over_insert_and_starts_an_insert() {
    let mut ed = editor_from_kitty("-[abc]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_ctrl('|'));
    assert_eq!(prompt_label(&ed).as_deref(), Some("|"));
    assert_eq!(ed.state.mode(), Mode::Insert);
    keys(&mut ed, "date");
    ed.handle_key(key_enter());
    assert!(ed.state.minibuf().is_none());
    assert_eq!(ed.state.mode(), Mode::Insert);
    assert_eq!(
        take_run(&mut ed),
        Some(("date".to_string(), PipeKind::Insert))
    );
}

#[test]
fn escape_closes_the_prompt_and_the_insert_session_goes_on() {
    let mut ed = editor_from_kitty("-[abc]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_ctrl('|'));
    keys(&mut ed, "date");
    ed.handle_key(key_esc());
    assert!(ed.state.minibuf().is_none());
    assert_eq!(ed.state.mode(), Mode::Insert);
    assert!(!ed.doc().is_busy());
    ed.handle_key(key('x'));
    assert_eq!(text(&ed), "xabc\n");
}

#[test]
fn a_cancelled_shell_prompt_keeps_the_previous_change() {
    let mut ed = editor_from("-[a]>\n");
    ed.handle_key(key('r'));
    ed.handle_key(key('x'));
    ed.handle_key(key('|'));
    keys(&mut ed, "sort");
    ed.handle_key(key_esc());
    assert!(!ed.doc().is_busy());
    assert_eq!(last_command(&ed), Some("replace"));
}

#[test]
fn ctrl_bar_needs_the_kitty_protocol() {
    let mut ed = editor_from("-[abc]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_ctrl('|'));
    assert!(ed.state.minibuf().is_none());
}

#[test]
fn the_prompt_recalls_its_own_history_only() {
    let mut ed = editor_from("-[abc]>\n");
    ed.handle_key(key('|'));
    keys(&mut ed, "sort");
    ed.handle_key(key_enter());
    take_run(&mut ed);

    ed.handle_key(key('|'));
    ed.handle_key(key_up());
    assert_eq!(minibuf_input(&ed), "sort");
    ed.handle_key(key_esc());

    ed.handle_key(key(':'));
    ed.handle_key(key_up());
    assert_eq!(minibuf_input(&ed), "");
    assert!(
        ed.state
            .history
            .get(HistoryKind::Command)
            .entries()
            .is_empty()
    );
}

#[test]
fn a_blank_command_is_not_recorded_in_history() {
    let mut ed = editor_from("-[abc]>\n");
    ed.handle_key(key('|'));
    keys(&mut ed, "sort");
    ed.handle_key(key_enter());
    take_run(&mut ed);

    ed.handle_key(key('|'));
    keys(&mut ed, "   ");
    ed.handle_key(key_enter());
    let entries: Vec<_> = ed
        .state
        .history
        .get(HistoryKind::Pipe)
        .entries()
        .iter()
        .cloned()
        .collect();
    assert_eq!(entries, ["sort"]);
}

#[test]
fn a_blank_command_starts_nothing() {
    let mut ed = editor_from("-[abc]>\n");
    ed.handle_key(key('|'));
    keys(&mut ed, "  ");
    ed.handle_key(key_enter());
    assert!(!ed.doc().is_busy());
    assert_eq!(state(&ed), "-[abc]>\n");
}

#[test]
fn a_read_only_buffer_refuses_before_anything_runs() {
    let mut ed = editor_with_read_only_view("abc\n", "view");
    ed.handle_key(key('|'));
    keys(&mut ed, "sort");
    ed.handle_key(key_enter());
    assert!(!ed.doc().is_busy());
    assert_eq!(status(&ed), read_only_refusal(&ed));
}

#[test]
fn the_prompt_does_not_open_over_another_minibuffer() {
    let mut ed = editor_from("-[abc]>\n");
    let f2 = KeyEvent::new(KeyCode::Function(2), Modifiers::NONE);
    ed.state.config.keymap.bind(
        BindMode::Command,
        &[f2],
        "pipe".into(),
        false,
        hume_scripting::ConfigOrigin::User,
    );
    ed.handle_key(key(':'));
    ed.handle_key(f2);
    assert_eq!(prompt_label(&ed).as_deref(), Some(":"));
    assert!(has_error(&ed, "a minibuffer is already open"));
}

#[test]
fn call_refuses_pipe() {
    let mut ed = editor_from("-[abc]>\n");
    attach_steel(
        &mut ed,
        r#"(define-command! "f2-body" "" (lambda (pane) (call! "pipe" pane "sort")))"#,
    );
    let f2 = KeyEvent::new(KeyCode::Function(2), Modifiers::NONE);
    ed.state.config.keymap.bind(
        BindMode::Normal,
        &[f2],
        "f2-body".into(),
        false,
        hume_scripting::ConfigOrigin::User,
    );
    ed.handle_key(f2);
    assert!(has_error(&ed, "'pipe': runs only from a key"));
    assert!(!ed.doc().is_busy());
}

// ── Tab completion ───────────────────────────────────────────────────────────

/// An editor started over the repo's runtime and working in `dir`, with
/// `typed` on an open `|` line.
pub(super) fn pipe_line_in(dir: &Path, typed: &str) -> Editor {
    let mut ed = editor_from("-[abc]>\n");
    ed.state.dirs = repo_runtime_dirs();
    ed.init_scripting(&mut Default::default());
    ed.state.cwd = AbsolutePath::rooted(dir);
    ed.handle_key(key('|'));
    keys(&mut ed, typed);
    ed
}

/// Presses `key` and lets the completion source it calls answer.
pub(super) fn press(ed: &mut Editor, key: KeyEvent) {
    ed.handle_key(key);
    ed.settle();
}

#[test]
fn tab_completes_a_path_argument() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat no");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat notes.txt");
    assert!(ed.state.input.minibuf_completion().is_none());
}

#[test]
fn tab_completes_only_the_word_under_the_cursor() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("beta.txt"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "diff alpha be");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "diff alpha beta.txt");
}

#[test]
fn tab_applies_the_shared_prefix_then_cycles_and_enter_runs_the_command() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alpha1"), b"").unwrap();
    std::fs::write(dir.path().join("alpha2"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat al");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat alpha");
    assert_eq!(
        ed.state.input.minibuf_completion().map(|s| s.len()),
        Some(2)
    );
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat alpha1");
    ed.handle_key(key_enter());
    assert!(ed.state.minibuf().is_none());
    assert_eq!(
        take_run(&mut ed),
        Some(("cat alpha1".to_string(), PipeKind::Replace))
    );
}

#[test]
fn the_shared_prefix_never_ends_between_a_backslash_and_what_it_escapes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ab c.txt"), b"").unwrap();
    std::fs::write(dir.path().join("ab$d"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat a");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat ab");
}

#[test]
fn escaped_names_sharing_only_the_typed_start_leave_the_line_as_typed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ab c.txt"), b"").unwrap();
    std::fs::write(dir.path().join("ab$d"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat ab");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat ab");
    assert_eq!(
        ed.state.input.minibuf_completion().map(|s| s.len()),
        Some(2)
    );
}

#[test]
fn enter_on_a_directory_candidate_goes_into_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("docs")).unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("docs/one.md"), b"").unwrap();
    std::fs::write(dir.path().join("docs/two.md"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat ");
    press(&mut ed, key_tab());
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat docs/");
    press(&mut ed, key_enter());
    assert_eq!(prompt_label(&ed).as_deref(), Some("|"));
    assert_eq!(minibuf_input(&ed), "cat docs/");
    assert_eq!(
        ed.state.input.minibuf_completion().map(|s| s.len()),
        Some(2)
    );
}

#[test]
fn a_name_with_a_space_is_escaped_and_completes_inside() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("my dir")).unwrap();
    std::fs::write(dir.path().join("my dir/inner.txt"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat my");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), r"cat my\ dir/");
    keys(&mut ed, "in");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), r"cat my\ dir/inner.txt");
}

#[test]
fn a_command_word_with_a_separator_completes_as_a_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("run.sh"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "./ru");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "./run.sh");
}

#[test]
fn an_escaped_directory_is_read_unescaped_and_kept_as_typed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("a b")).unwrap();
    std::fs::write(dir.path().join("a b/c.txt"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), r"cat a\ b/c");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), r"cat a\ b/c.txt");
}

#[test]
fn an_escaped_space_before_the_cursor_stays_in_the_word() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("my notes.txt"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), r"cat my\ n");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), r"cat my\ notes.txt");
}

#[test]
fn an_escaped_tilde_names_a_directory_called_tilde() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("~")).unwrap();
    std::fs::write(dir.path().join("~/inside-tilde"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), r"cat \~/");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), r"cat \~/inside-tilde");
}

#[test]
fn a_quoted_word_completes_inside_its_quote_and_a_file_closes_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("my notes.txt"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat \"my no");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat \"my notes.txt\"");
}

#[test]
fn a_quoted_directory_closes_its_quote_and_the_next_tab_completes_inside_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("my dir")).unwrap();
    std::fs::write(dir.path().join("my dir/inner.txt"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat 'my d");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat 'my dir/'");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat 'my dir/inner.txt'");
}

#[test]
fn tab_after_a_closed_word_with_several_entries_cycles_inside_its_quote() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("my dir")).unwrap();
    std::fs::write(dir.path().join("my dir/a.txt"), b"").unwrap();
    std::fs::write(dir.path().join("my dir/b.txt"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat \"my dir/\"");
    press(&mut ed, key_tab());
    assert_eq!(
        ed.state.input.minibuf_completion().map(|s| s.len()),
        Some(2)
    );
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat \"my dir/a.txt\"");
}

#[test]
fn enter_on_a_quoted_directory_candidate_lists_inside_its_quote() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("my docs")).unwrap();
    std::fs::create_dir(dir.path().join("my dir")).unwrap();
    std::fs::write(dir.path().join("my dir/one.md"), b"").unwrap();
    std::fs::write(dir.path().join("my dir/two.md"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat \"my d");
    press(&mut ed, key_tab());
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat \"my dir/\"");
    press(&mut ed, key_enter());
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat \"my dir/one.md\"");
}

#[test]
fn a_word_that_only_opens_a_quote_completes_inside_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("my notes.txt"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat \"");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat \"my notes.txt\"");
}

#[test]
fn the_shared_prefix_of_a_quoted_word_stays_inside_its_quote() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("my notes.txt"), b"").unwrap();
    std::fs::write(dir.path().join("my nothing"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat \"my no");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat \"my not");
}

#[test]
fn the_shared_prefix_of_escaped_names_is_written_escaped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("my dir")).unwrap();
    std::fs::create_dir(dir.path().join("my docs")).unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat my");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), r"cat my\ d");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), r"cat my\ dir/");
}

/// PowerShell quotes a path from its start, so the shared prefix of
/// `'dir/a b1'` and `'dir/a b2'` is written as an open quote the next Tab
/// reads back as the same path.
#[test]
fn a_shared_prefix_left_in_an_open_quote_completes_on_the_next_tab() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("dir")).unwrap();
    std::fs::write(dir.path().join("dir/a b1"), b"").unwrap();
    std::fs::write(dir.path().join("dir/a b2"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat dir/a");
    ed.state.settings.shell =
        hume_platform::process::batch::ShellCommand::from_words(vec!["pwsh".into(), "-c".into()])
            .unwrap();
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat 'dir/a b");
    keys(&mut ed, "1");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat 'dir/a b1'");
}

#[test]
fn a_quote_opened_after_a_directory_is_continued() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("dir")).unwrap();
    std::fs::write(dir.path().join("dir/my notes.txt"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat dir/\"my");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat dir/\"my notes.txt\"");
}

#[test]
fn a_word_ending_in_an_escape_completes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("my dir")).unwrap();
    let mut ed = pipe_line_in(dir.path(), r"cat my\");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), r"cat my\ dir/");
}

#[test]
fn a_shell_with_no_dialect_completes_nothing_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat no");
    ed.state.settings.shell =
        hume_platform::process::batch::ShellCommand::from_words(vec!["nu".into(), "-c".into()])
            .unwrap();
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat no");
    assert!(ed.state.input.minibuf_completion().is_none());
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Warning && e.text.contains("no shell dialect for nu")),
        "expected a warning naming the shell"
    );
}

#[test]
fn a_shell_with_no_dialect_is_named_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = pipe_line_in(dir.path(), "cat no");
    ed.state.settings.shell =
        hume_platform::process::batch::ShellCommand::from_words(vec!["nu".into(), "-c".into()])
            .unwrap();
    press(&mut ed, key_tab());
    press(&mut ed, key_tab());
    let warnings = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.text.contains("no shell dialect for nu"))
        .count();
    assert_eq!(warnings, 1);
}

#[test]
fn tab_with_no_shell_source_registered_warns() {
    let mut ed = editor_from("-[abc]>\n");
    ed.handle_key(key('|'));
    keys(&mut ed, "cat no");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat no");
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Warning
                && e.text.contains("no completion source named \"shell\"")),
        "expected a warning naming the missing source"
    );
}

#[test]
fn completion_survives_a_config_reload() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), b"").unwrap();
    let mut ed = pipe_line_in(dir.path(), "");
    press(&mut ed, key_esc());
    super::command_mode::submit(&mut ed, "reload-config");
    ed.handle_key(key('|'));
    keys(&mut ed, "cat no");
    press(&mut ed, key_tab());
    assert_eq!(minibuf_input(&ed), "cat notes.txt");
}

#[test]
fn a_run_waits_only_for_ctrl_c_and_resizes() {
    assert!(matches!(
        wait_event(&TerminalEvent::Key(key_ctrl('c'))),
        Some(WaitEvent::Interrupt)
    ));
    let size = WindowSize {
        cols: 80,
        rows: 24,
        pixel_width: None,
        pixel_height: None,
    };
    assert!(matches!(
        wait_event(&TerminalEvent::WindowResized(size)),
        Some(WaitEvent::Resized)
    ));
    let mut release = key_ctrl('c');
    release.kind = KeyEventKind::Release;
    for left_queued in [
        TerminalEvent::Key(release),
        TerminalEvent::Key(key('u')),
        TerminalEvent::FocusIn,
    ] {
        assert!(
            wait_event(&left_queued).is_none(),
            "{left_queued:?} stays queued for after the run"
        );
    }
}

#[test]
fn a_script_prompt_closes_the_shell_prompt() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut ed = editor_from("-[abc]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "arm" "" (lambda (pane)
             (after! 0 (lambda () (prompt! pane "x: " (lambda (s) (void)))))))"#,
    );
    type_cmd(&mut ed, ":arm");
    ed.handle_key(key('|'));
    keys(&mut ed, "sort");
    ed.settle();
    assert_eq!(prompt_label(&ed).as_deref(), Some("x: "));
    assert!(ed.state.input.ref_of::<ShellPromptLayer>().is_none());
    ed.handle_key(key_esc());
    assert!(ed.state.minibuf().is_none());
    assert!(!ed.doc().is_busy());
}

#[test]
fn a_script_entering_insert_closes_the_shell_prompt() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut ed = editor_from("-[abc]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "arm" "" (lambda (pane)
             (after! 0 (lambda () (call! "insert-at-selection-start" pane)))))"#,
    );
    type_cmd(&mut ed, ":arm");
    ed.handle_key(key('|'));
    keys(&mut ed, "sort");
    ed.settle();
    assert_eq!(ed.state.mode(), Mode::Insert);
    assert!(ed.state.input.ref_of::<ShellPromptLayer>().is_none());
    assert!(ed.state.minibuf().is_none());
}

#[test]
fn bar_holds_the_buffer_from_enter_on() {
    let mut ed = editor_from("-[abc]>\n");
    ed.handle_key(key('|'));
    keys(&mut ed, "sort");
    assert!(!ed.doc().is_busy(), "the prompt holds nothing");
    ed.handle_key(key_enter());
    assert!(ed.doc().is_busy());
    take_run(&mut ed);
    assert!(!ed.doc().is_busy(), "ending the run releases the buffer");
}

#[test]
fn bar_starts_a_background_run() {
    let mut ed = editor_from("-[abc]>\n");
    type_pipe(&mut ed, "sort");
    assert!(ed.doc().is_busy());
    assert_eq!(ed.state.pipe_runs.foreground(), None);
}

#[test]
fn ctrl_bar_starts_a_foreground_run() {
    let mut ed = editor_from_kitty("-[abc]>\n");
    ed.handle_key(key('i'));
    ed.handle_key(key_ctrl('|'));
    keys(&mut ed, "date");
    ed.handle_key(key_enter());
    assert!(ed.state.pipe_runs.foreground().is_some());
}

#[test]
fn bar_while_recording_a_macro_starts_a_foreground_run() {
    let mut ed = editor_from("-[abc]>\n");
    keys(&mut ed, "QQ|sort");
    ed.handle_key(key_enter());
    assert!(ed.state.macros.recording_register().is_some());
    assert!(ed.state.pipe_runs.foreground().is_some());
}

/// Starts a background `|sort` on the focused buffer through `handle_key`;
/// nothing settles, so no process starts.
fn start_sort(ed: &mut Editor) {
    type_pipe(ed, "sort");
    assert!(ed.doc().is_busy(), "setup: the run is in flight");
}

fn cancelled(ed: &Editor, bid: BufferId) -> String {
    format!(
        "pipe in {} cancelled",
        ed.state.buffers.get(bid).display_name()
    )
}

#[test]
fn ctrl_c_cancels_the_run_in_this_buffer_and_changes_nothing() {
    let mut ed = editor_from("-[a]>\n");
    let bid = ed.focused_buffer_id();
    keys(&mut ed, "rx");
    start_sort(&mut ed);
    ed.handle_key(key_ctrl('c'));
    assert!(!ed.doc().is_busy());
    assert_eq!(state(&ed), "-[x]>\n");
    assert_eq!(status(&ed), cancelled(&ed, bid));
    assert_eq!(last_command(&ed), Some("replace"));
}

#[test]
fn ctrl_c_in_extend_mode_cancels() {
    let mut ed = editor_from("-[a]>\n");
    start_sort(&mut ed);
    ed.handle_key(key('e'));
    assert_eq!(ed.state.mode(), Mode::Extend, "setup");
    ed.handle_key(key_ctrl('c'));
    assert!(!ed.doc().is_busy());
}

#[test]
fn ctrl_c_without_a_run_says_so() {
    let mut ed = editor_from("-[a]>\n");
    ed.handle_key(key_ctrl('c'));
    assert_eq!(status(&ed), "No pipe is running in this buffer");
    assert_eq!(state(&ed), "-[a]>\n");
}

#[test]
fn closing_another_pane_on_the_buffer_keeps_the_run() {
    let mut ed = editor_from("-[a]>\n");
    let bid = ed.focused_buffer_id();
    start_sort(&mut ed);
    ed.execute_keymap_command("pane-vsplit".into(), None, false);
    ed.execute_keymap_command("pane-close".into(), None, false);
    assert!(busy(&ed, bid));
}

#[test]
fn switching_the_launching_pane_away_keeps_the_run() {
    let mut ed = editor_from("-[a]>\n");
    let bid = ed.focused_buffer_id();
    start_sort(&mut ed);
    open_other_buffer(&mut ed, "x\n");
    assert!(busy(&ed, bid));
}

#[test]
fn closing_a_busy_buffer_is_refused_and_keeps_its_run() {
    let mut ed = editor_from("-[a]>\n");
    let bid = ed.focused_buffer_id();
    start_sort(&mut ed);
    assert_eq!(
        ed.close_buffer(bid),
        Err(crate::editor::buffer::lifecycle::CloseBufferRefused::PipeRunning(PipeRunning(bid)))
    );
    assert!(ed.state.buffers.try_get(bid).is_some());
    assert!(busy(&ed, bid));
}

/// `rx` in one buffer, then a background `|sort` in it and in a second
/// buffer: two runs in flight, each holding the change before it.
fn two_runs_after_a_replace(ed: &mut Editor) -> (BufferId, BufferId) {
    let first = ed.focused_buffer_id();
    keys(ed, "rx");
    start_sort(ed);
    let second = open_other_buffer(ed, "b\n");
    start_sort(ed);
    (first, second)
}

#[test]
fn two_cancelled_runs_leave_the_change_before_both_newest_first() {
    let mut ed = editor_from("-[a]>\n");
    let (first, second) = two_runs_after_a_replace(&mut ed);
    ed.state.cancel_pipe_run(second);
    ed.state.cancel_pipe_run(first);
    assert_eq!(last_command(&ed), Some("replace"));
}

#[test]
fn two_cancelled_runs_leave_the_change_before_both_oldest_first() {
    let mut ed = editor_from("-[a]>\n");
    let (first, second) = two_runs_after_a_replace(&mut ed);
    ed.state.cancel_pipe_run(first);
    assert_eq!(last_command(&ed), Some("pipe"), "the newer run's change");
    ed.state.cancel_pipe_run(second);
    assert_eq!(last_command(&ed), Some("replace"));
}

/// The refusal of a close that would stop `bid`'s run, `bid` in the
/// focused pane, naming `way_out`.
fn pipe_running(ed: &Editor, bid: BufferId, way_out: &str) -> String {
    format!(
        "{} is busy: a pipe is running. Ctrl-c stops it, or use {way_out}",
        ed.state.buffers.get(bid).display_name()
    )
}

/// [`pipe_running`] for a `bid` no focused pane shows, which `Ctrl-c`
/// does not reach, or with `Ctrl-c` not bound to `cancel-pipe`.
fn pipe_running_elsewhere(ed: &Editor, bid: BufferId, way_out: &str) -> String {
    format!(
        "{} is busy: a pipe is running. Use {way_out}",
        ed.state.buffers.get(bid).display_name()
    )
}

#[test]
fn q_refuses_while_a_run_is_in_flight() {
    let mut ed = editor_from("-[a]>\n");
    start_sort(&mut ed);
    type_cmd(&mut ed, ":q");
    assert!(!ed.state.should_quit);
    insta::assert_snapshot!(status(&ed), @"*scratch* is busy: a pipe is running. Ctrl-c stops it, or use :q!");
    assert!(ed.doc().is_busy());
}

#[test]
fn q_that_would_exit_names_the_busy_buffer() {
    let mut ed = editor_from("-[a]>\n");
    let busy = ed.focused_buffer_id();
    start_sort(&mut ed);
    open_other_buffer(&mut ed, "b\n");
    type_cmd(&mut ed, ":q");
    assert!(!ed.state.should_quit);
    assert_eq!(status(&ed), pipe_running_elsewhere(&ed, busy, ":q!"));
}

#[test]
fn q_refuses_to_close_the_busy_buffer() {
    let (mut ed, _file) = editor_with_file("-[a]>\n", "a\n");
    let busy = ed.focused_buffer_id();
    let (other, _other_file) = temp_file("b\n");
    type_cmd(&mut ed, &format!(":e {}", other.display()));
    ed.switch_to_buffer_with_jump(FocusedPane::current(&ed.view), busy);
    start_sort(&mut ed);
    type_cmd(&mut ed, ":q");
    assert_eq!(status(&ed), pipe_running(&ed, busy, ":q!"));
    assert!(ed.state.buffers.try_get(busy).is_some());
    assert!(!ed.state.should_quit);
}

#[test]
fn q_bang_quits() {
    let mut ed = editor_from("-[a]>\n");
    start_sort(&mut ed);
    type_cmd(&mut ed, ":q!");
    assert!(ed.state.should_quit);
}

/// A vertical split whose focused pane started a run in `bid`.
fn split_running_sort() -> (Editor, BufferId) {
    let mut ed = editor_from("-[a]>\n");
    let bid = ed.focused_buffer_id();
    ed.execute_keymap_command("pane-vsplit".into(), None, false);
    start_sort(&mut ed);
    (ed, bid)
}

/// A second tab whose only pane started a run in `bid`.
fn tab_running_sort() -> (Editor, BufferId) {
    let mut ed = editor_from("-[a]>\n");
    ed.execute_keymap_command("tab-new".into(), None, false);
    let bid = ed.focused_buffer_id();
    start_sort(&mut ed);
    (ed, bid)
}

#[test]
fn q_in_a_split_refuses_while_the_panes_run_is_in_flight() {
    let (mut ed, bid) = split_running_sort();
    type_cmd(&mut ed, ":q");
    assert_eq!(status(&ed), pipe_running(&ed, bid, ":q!"));
    assert_eq!(ed.view.active_pane_ids().len(), 2);
    assert!(busy(&ed, bid));
}

#[test]
fn q_bang_in_a_split_closes_the_pane_and_cancels() {
    let (mut ed, bid) = split_running_sort();
    type_cmd(&mut ed, ":q!");
    assert!(!ed.state.should_quit);
    assert_eq!(ed.view.active_pane_ids().len(), 1);
    assert!(!busy(&ed, bid));
}

#[test]
fn q_in_a_split_closes_a_pane_whose_buffer_runs_a_pipe_started_elsewhere() {
    let mut ed = editor_from("-[a]>\n");
    let bid = ed.focused_buffer_id();
    start_sort(&mut ed);
    ed.execute_keymap_command("pane-vsplit".into(), None, false);
    type_cmd(&mut ed, ":q");
    assert_eq!(ed.view.active_pane_ids().len(), 1);
    assert!(busy(&ed, bid), "the run from the pane left open goes on");
}

#[test]
fn q_on_a_tabs_last_pane_refuses_while_its_run_is_in_flight() {
    let (mut ed, bid) = tab_running_sort();
    type_cmd(&mut ed, ":q");
    assert_eq!(status(&ed), pipe_running(&ed, bid, ":q!"));
    assert_eq!(ed.state.tabs.len(), 2);
    assert!(busy(&ed, bid));
}

#[test]
fn q_bang_on_a_tabs_last_pane_closes_the_tab_and_cancels() {
    let (mut ed, bid) = tab_running_sort();
    type_cmd(&mut ed, ":q!");
    assert!(!ed.state.should_quit);
    assert_eq!(ed.state.tabs.len(), 1);
    assert!(!busy(&ed, bid));
}

#[test]
fn tabclose_refuses_while_a_run_from_the_tab_is_in_flight() {
    let (mut ed, bid) = tab_running_sort();
    type_cmd(&mut ed, ":tabclose");
    assert_eq!(status(&ed), pipe_running(&ed, bid, ":tabclose!"));
    assert_eq!(ed.state.tabs.len(), 2);
    assert!(busy(&ed, bid));
}

#[test]
fn tabclose_bang_closes_the_tab_and_cancels() {
    let (mut ed, bid) = tab_running_sort();
    type_cmd(&mut ed, ":tabclose!");
    assert_eq!(ed.state.tabs.len(), 1);
    assert!(!busy(&ed, bid));
}

#[test]
fn wq_in_a_split_writes_then_refuses() {
    let (mut ed, file) = editor_with_file("-[a]>\n", "a\n");
    let bid = ed.focused_buffer_id();
    ed.execute_keymap_command("pane-vsplit".into(), None, false);
    keys(&mut ed, "rx");
    start_sort(&mut ed);
    type_cmd(&mut ed, ":wq");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "x\n");
    assert_eq!(ed.view.active_pane_ids().len(), 2);
    assert_eq!(status(&ed), pipe_running(&ed, bid, ":q!"));
}

#[test]
fn the_pane_close_key_refuses_while_the_panes_run_is_in_flight() {
    let (mut ed, bid) = split_running_sort();
    ed.handle_key(key_ctrl('p'));
    ed.handle_key(key('c'));
    assert_eq!(status(&ed), pipe_running(&ed, bid, ":q!"));
    assert_eq!(ed.view.active_pane_ids().len(), 2);
    assert!(busy(&ed, bid));
}

#[test]
fn e_refuses_while_a_run_is_in_flight() {
    let (mut ed, file) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid = ed.focused_buffer_id();
    std::fs::write(&file, "HELLO!!\n").expect("file rewritten");
    start_sort(&mut ed);
    type_cmd(&mut ed, ":e");
    assert_eq!(status(&ed), pipe_running(&ed, bid, ":e!"));
    assert_eq!(ed.doc().text().to_string(), "hello\n");
    assert!(busy(&ed, bid));
}

#[test]
fn e_bang_on_a_deleted_file_stops_the_run_and_reports_the_read_error() {
    let (mut ed, file) = editor_with_file("-[h]>ello\n", "hello\n");
    let bid = ed.focused_buffer_id();
    std::fs::remove_file(&file).expect("file removed");
    start_sort(&mut ed);
    type_cmd(&mut ed, ":e!");
    let display = ed.doc().display_path().expect("a file buffer").to_string();
    assert!(has_error(&ed, &format!("{display}: ")));
    assert_eq!(ed.doc().text().to_string(), "hello\n");
    assert!(!busy(&ed, bid));
}

#[test]
fn qa_refuses_naming_the_busy_buffer() {
    let mut ed = editor_from("-[a]>\n");
    let bid = ed.focused_buffer_id();
    start_sort(&mut ed);
    open_other_buffer(&mut ed, "b\n");
    type_cmd(&mut ed, ":qa");
    assert!(!ed.state.should_quit);
    assert_eq!(status(&ed), pipe_running_elsewhere(&ed, bid, ":qa!"));
    insta::assert_snapshot!(status(&ed), @"*scratch* is busy: a pipe is running. Use :qa!");
}

#[test]
fn qa_bang_quits() {
    let mut ed = editor_from("-[a]>\n");
    start_sort(&mut ed);
    type_cmd(&mut ed, ":qa!");
    assert!(ed.state.should_quit);
}

#[test]
fn a_refusal_names_ctrl_c_only_while_it_runs_cancel_pipe() {
    let mut ed = editor_from("-[a]>\n");
    let bid = ed.focused_buffer_id();
    ed.state.config.keymap.unbind(
        BindMode::Normal,
        &[key_ctrl('c')],
        hume_scripting::ConfigOrigin::User,
    );
    start_sort(&mut ed);
    type_cmd(&mut ed, ":bd");
    assert_eq!(status(&ed), pipe_running_elsewhere(&ed, bid, ":bd!"));
}

#[test]
fn bd_refuses() {
    let mut ed = editor_from("-[a]>\n");
    let bid = ed.focused_buffer_id();
    start_sort(&mut ed);
    type_cmd(&mut ed, ":bd");
    assert_eq!(status(&ed), pipe_running(&ed, bid, ":bd!"));
    insta::assert_snapshot!(status(&ed), @"*scratch* is busy: a pipe is running. Ctrl-c stops it, or use :bd!");
    assert!(busy(&ed, bid));
}

#[test]
fn bd_bang_cancels_the_buffers_run() {
    let mut ed = editor_from("-[a]>\n");
    let bid = ed.focused_buffer_id();
    start_sort(&mut ed);
    type_cmd(&mut ed, ":bd!");
    assert!(ed.state.buffers.try_get(bid).is_none());
}

#[test]
fn wq_writes_then_refuses() {
    let (mut ed, file) = editor_with_file("-[a]>\n", "a\n");
    let bid = ed.focused_buffer_id();
    keys(&mut ed, "rx");
    start_sort(&mut ed);
    type_cmd(&mut ed, ":wq");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "x\n");
    assert!(!ed.state.should_quit);
    assert_eq!(status(&ed), pipe_running(&ed, bid, ":q!"));
}
