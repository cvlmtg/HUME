use super::*;
use crossterm::event::{KeyCode, KeyModifiers};
use pretty_assertions::assert_eq;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn key_tab() -> KeyEvent {
    KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
}

fn key_shift_tab() -> KeyEvent {
    KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)
}

/// Drain the minibuf input for assertions.
fn minibuf_input(ed: &Editor) -> &str {
    ed.minibuf.as_ref().map(|mb| mb.input.as_str()).unwrap_or("")
}

// ── Command-name completion ───────────────────────────────────────────────────

#[test]
fn tab_on_command_prefix_single_match_completes_silently() {
    // ":quit" is the only registered command starting with "qui".
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "qui".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_tab());

    assert_eq!(minibuf_input(&ed), "quit");
    // Single-match: no popup state.
    assert!(ed.completion.is_none());
}

#[test]
fn tab_no_match_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "zzz".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_tab());

    assert_eq!(minibuf_input(&ed), "zzz");
    assert!(ed.completion.is_none());
}

#[test]
fn tab_multiple_matches_opens_popup_with_first_candidate() {
    // "w" matches: write, write-quit, wq, wrap (at minimum).
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_tab());

    // Completion state must be open.
    assert!(ed.completion.is_some(), "popup should be open");
    let state = ed.completion.as_ref().unwrap();
    assert_eq!(state.selected, 0);
    assert!(state.candidates.len() >= 2);
    // Input shows the first candidate.
    let first = state.candidates[0].replacement.clone();
    assert_eq!(minibuf_input(&ed), first);
}

#[test]
fn second_tab_cycles_to_next_candidate() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_tab());
    ed.handle_key(key_tab());

    let state = ed.completion.as_ref().unwrap();
    assert_eq!(state.selected, 1);
    let second = state.candidates[1].replacement.clone();
    assert_eq!(minibuf_input(&ed), second);
}

#[test]
fn shift_tab_cycles_backward() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    // Open popup (first candidate selected).
    ed.handle_key(key_tab());
    // Tab forward to candidate 1.
    ed.handle_key(key_tab());
    // Shift-Tab back to candidate 0.
    ed.handle_key(key_shift_tab());

    let state = ed.completion.as_ref().unwrap();
    assert_eq!(state.selected, 0);
}

#[test]
fn tab_wraps_at_end() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_tab());

    let n = ed.completion.as_ref().unwrap().candidates.len();
    // Tab n times to wrap back to 0.
    for _ in 0..n {
        ed.handle_key(key_tab());
    }
    assert_eq!(ed.completion.as_ref().unwrap().selected, 0);
}

#[test]
fn typing_char_dismisses_popup() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_tab()); // open popup

    assert!(ed.completion.is_some());
    ed.handle_key(key('r')); // type a char → dismiss
    assert!(ed.completion.is_none());
}

#[test]
fn enter_mid_completion_executes_selected_candidate() {
    // ":quit" is a unique completion for "qui".
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "qui".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_tab());

    // Now input = "quit". Enter should quit.
    ed.handle_key(key_enter());
    assert!(ed.should_quit);
    assert!(ed.completion.is_none());
    assert!(ed.minibuf.is_none());
}

#[test]
fn esc_dismisses_minibuf_and_clears_completion() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_tab()); // open popup
    ed.handle_key(key_esc());

    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.minibuf.is_none());
    assert!(ed.completion.is_none());
}

#[test]
fn shift_tab_with_no_popup_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "wri".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_shift_tab()); // no popup yet

    // Nothing should have changed: input stays "wri", no popup.
    assert_eq!(minibuf_input(&ed), "wri");
    assert!(ed.completion.is_none());
}

#[test]
fn tab_in_search_mode_is_noop() {
    // Tab in search mode (`/`) must not trigger completion.
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('/'));
    ed.handle_key(key('e'));
    ed.handle_key(key_tab());

    // Input unchanged; no completion.
    assert_eq!(minibuf_input(&ed), "e");
    assert!(ed.completion.is_none());
}

// ── Path completion ───────────────────────────────────────────────────────────

#[test]
fn tab_on_edit_arg_completes_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), b"").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    // Use an absolute path so we don't depend on cwd (avoids test-parallelism races).
    let prefix = format!("{}/hel", dir.path().display());
    let input = format!("e {prefix}");

    ed.handle_key(key(':'));
    for ch in input.chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_tab());

    // Single match → silent completion, no popup.
    let expected = format!("e {}/hello.txt", dir.path().display());
    assert_eq!(minibuf_input(&ed), expected);
    assert!(ed.completion.is_none());
}

#[test]
fn tab_on_write_arg_completes_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("out.txt"), b"").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    let prefix = format!("{}/out", dir.path().display());
    let input = format!("w {prefix}");

    ed.handle_key(key(':'));
    for ch in input.chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_tab());

    let expected = format!("w {}/out.txt", dir.path().display());
    assert_eq!(minibuf_input(&ed), expected);
}

#[test]
fn tab_on_cd_arg_completes_dirs_only() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("mysubdir")).unwrap();
    std::fs::write(dir.path().join("myfile.txt"), b"").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    // "my" matches both mysubdir/ and myfile.txt if dirs_only=false, but :cd
    // dispatches PathCompleter { dirs_only: true }, leaving only one candidate.
    let prefix = format!("{}/my", dir.path().display());
    let input = format!("cd {prefix}");

    ed.handle_key(key(':'));
    for ch in input.chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_tab());

    // One dir-only match → silent complete with trailing '/'.
    let expected = format!("cd {}/mysubdir/", dir.path().display());
    assert_eq!(minibuf_input(&ed), expected, "cd must complete to the directory");
    assert!(ed.completion.is_none(), ":cd completion must exclude files, leaving a single dir match");
}

// ── Directory descent on Enter ────────────────────────────────────────────────

#[test]
fn enter_on_directory_candidate_restarts_completion() {
    let dir = tempfile::tempdir().unwrap();
    // Two sub-dirs so the path popup has ≥2 candidates (popup opens).
    std::fs::create_dir(dir.path().join("alpha")).unwrap();
    std::fs::create_dir(dir.path().join("beta")).unwrap();
    // Populate one of them with two files so the descend-restart opens a popup.
    std::fs::write(dir.path().join("alpha/one.txt"), b"").unwrap();
    std::fs::write(dir.path().join("alpha/two.txt"), b"").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    let input = format!("e {}/", dir.path().display());
    ed.handle_key(key(':'));
    for ch in input.chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_tab()); // opens popup; "alpha/" selected first (alphabetical).

    let state = ed.completion.as_ref().expect("popup should be open");
    let first = state.candidates[0].replacement.clone();
    assert!(first.ends_with('/'), "expected directory candidate, got {first}");

    ed.handle_key(key_enter());

    // Minibuf stays open — Enter on a dir must not execute the command.
    assert!(ed.minibuf.is_some(), "Enter on dir candidate must keep minibuf open");
    // Input now contains the selected directory.
    let input_now = minibuf_input(&ed);
    assert!(input_now.contains("/alpha/"), "expected descent through alpha/, got {input_now}");
    // Completion re-triggered with the directory's children.
    let restarted = ed.completion.as_ref().expect("completion should restart for dir children");
    assert_eq!(restarted.candidates.len(), 2, "expected 2 files under alpha/");
}

// ── Ctrl-W delete-word in minibuf ─────────────────────────────────────────────

#[test]
fn ctrl_w_deletes_word_in_minibuf() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "e foo bar".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "e foo ");
}

#[test]
fn ctrl_w_skips_trailing_whitespace_first() {
    // Readline behaviour: runs of spaces are consumed before the word.
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "e foo   ".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "e ");
}

#[test]
fn ctrl_w_at_start_is_noop_and_keeps_minibuf_open() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "");
    // Unlike Backspace on empty input (which cancels), Ctrl-W is a no-op.
    assert!(ed.minibuf.is_some(), "Ctrl-W on empty input must not close the minibuf");
}

#[test]
fn ctrl_w_stops_at_slash_for_path_args() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "e /tmp/alpha/one.txt".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "e /tmp/alpha/");
}

#[test]
fn ctrl_w_on_trailing_slash_deletes_dir_component() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "e /tmp/alpha/".chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "e /tmp/");
}

#[test]
fn ctrl_w_dismisses_open_completion_popup() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_tab()); // opens popup for "w"-prefixed commands
    assert!(ed.completion.is_some(), "sanity: popup should be open");

    ed.handle_key(key_ctrl('w'));
    // Edited event clears completion; Ctrl-W consumed the word ("w"-based candidate).
    assert!(ed.completion.is_none(), "Ctrl-W must dismiss the popup");
}

#[test]
fn ctrl_w_works_in_search_minibuf() {
    // Ctrl-W in a `/` search prompt deletes the last word without cancelling.
    let mut ed = editor_from("-[h]>ello world\n");
    ed.handle_key(key('/'));
    assert_eq!(ed.mode, Mode::Search);
    for ch in "foo bar".chars() { ed.handle_key(key(ch)); }
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "foo bar");
    ed.handle_key(key_ctrl('w'));
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "foo ");
    // Search minibuf must still be open.
    assert!(ed.minibuf.is_some(), "Ctrl-W must not close the search minibuf");
    assert_eq!(ed.mode, Mode::Search);
    ed.handle_key(key_esc());
}

#[test]
fn ctrl_w_at_start_of_search_minibuf_is_noop() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.handle_key(key('/'));
    // Nothing typed yet — Ctrl-W on empty input is a no-op.
    ed.handle_key(key_ctrl('w'));
    assert!(ed.minibuf.is_some(), "Ctrl-W on empty search input must not close the minibuf");
    assert_eq!(ed.mode, Mode::Search);
    ed.handle_key(key_esc());
}
