use super::*;
use hume_grid::Rect;
use pretty_assertions::assert_eq;
use termina::event::{KeyCode, Modifiers};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn key_tab() -> KeyEvent {
    KeyEvent::new(KeyCode::Tab, Modifiers::NONE)
}

fn key_shift_tab() -> KeyEvent {
    KeyEvent::new(KeyCode::BackTab, Modifiers::SHIFT)
}

/// Drain the minibuf input for assertions.
fn minibuf_input(ed: &Editor) -> &str {
    ed.state.minibuf().map(|mb| mb.input.as_str()).unwrap_or("")
}

/// The completion popup's selected row index — `0` when no `CompletionMenuUi`
/// has been allocated yet, matching `move_completion_selection`'s own
/// `map_or(0, ...)` convention (a session opens with `ui: None` until the
/// first Tab/Down/BackTab/Up moves the selection off its implicit default).
fn selected_row(ed: &Editor) -> usize {
    ed.state.input.completion_ui().map_or(0, |ui| ui.selected)
}

// ── Command-name completion ───────────────────────────────────────────────────

#[test]
fn tab_on_command_prefix_single_match_completes_silently() {
    // "reload-config" is the only registered command starting with "relo".
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "relo".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_tab());

    assert_eq!(minibuf_input(&ed), "reload-config");
    // Single-match: no popup state.
    assert!(ed.state.input.completion().is_none());
}

#[test]
fn tab_no_match_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "zzz".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_tab());

    assert_eq!(minibuf_input(&ed), "zzz");
    assert!(ed.state.input.completion().is_none());
}

#[test]
fn tab_multiple_matches_opens_popup_with_first_candidate() {
    // "w" matches: write, write-quit, wq, wrap (at minimum).
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_tab());

    // Completion state must be open.
    assert!(
        ed.state.input.completion().is_some(),
        "popup should be open"
    );
    assert_eq!(selected_row(&ed), 0);
    let session = ed.state.input.completion().unwrap();
    assert!(session.len() >= 2);
    // Input shows the first candidate.
    let first = session.selected_item(0).unwrap().insert_text().to_owned();
    assert_eq!(minibuf_input(&ed), first);
}

#[test]
fn second_tab_cycles_to_next_candidate() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_tab());
    ed.handle_key(key_tab());

    assert_eq!(selected_row(&ed), 1);
    let session = ed.state.input.completion().unwrap();
    let second = session.selected_item(1).unwrap().insert_text().to_owned();
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

    assert_eq!(selected_row(&ed), 0);
}

#[test]
fn tab_wraps_at_end() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_tab());

    let n = ed.state.input.completion().unwrap().len();
    // Tab n times to wrap back to 0.
    for _ in 0..n {
        ed.handle_key(key_tab());
    }
    assert_eq!(selected_row(&ed), 0);
}

#[test]
fn typing_char_dismisses_popup() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_tab()); // open popup

    assert!(ed.state.input.completion().is_some());
    ed.handle_key(key('r')); // type a char → dismiss
    assert!(ed.state.input.completion().is_none());
}

#[test]
fn enter_mid_completion_executes_selected_candidate() {
    // ":quit" is a unique completion for "qui".
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "qui".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_tab());

    // Now input = "quit". Enter should quit.
    ed.handle_key(key_enter());
    assert!(ed.state.should_quit);
    assert!(ed.state.input.completion().is_none());
    assert!(ed.state.minibuf().is_none());
}

#[test]
fn esc_dismisses_minibuf_and_clears_completion() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_tab()); // open popup
    ed.handle_key(key_esc());

    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(ed.state.minibuf().is_none());
    assert!(ed.state.input.completion().is_none());
}

#[test]
fn shift_tab_with_no_popup_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "wri".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_shift_tab()); // no popup yet

    // Nothing should have changed: input stays "wri", no popup.
    assert_eq!(minibuf_input(&ed), "wri");
    assert!(ed.state.input.completion().is_none());
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
    assert!(ed.state.input.completion().is_none());
}

// ── Path completion ───────────────────────────────────────────────────────────

#[test]
fn tab_on_edit_arg_completes_path() {
    let dir = safe_tempdir();
    std::fs::write(dir.path().join("hello.txt"), b"").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    // Use an absolute path so we don't depend on cwd (avoids test-parallelism races).
    let prefix = format!("{}/hel", dir.path().display());
    let input = format!("e {prefix}");

    ed.handle_key(key(':'));
    for ch in input.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_tab());

    // Single match → silent completion, no popup.
    let expected = format!("e {}/hello.txt", dir.path().display());
    assert_eq!(minibuf_input(&ed), expected);
    assert!(ed.state.input.completion().is_none());
}

#[test]
fn tab_on_write_arg_completes_path() {
    let dir = safe_tempdir();
    std::fs::write(dir.path().join("out.txt"), b"").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    let prefix = format!("{}/out", dir.path().display());
    let input = format!("w {prefix}");

    ed.handle_key(key(':'));
    for ch in input.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_tab());

    let expected = format!("w {}/out.txt", dir.path().display());
    assert_eq!(minibuf_input(&ed), expected);
}

#[test]
fn tab_on_cd_arg_completes_dirs_only() {
    let dir = safe_tempdir();
    std::fs::create_dir(dir.path().join("mysubdir")).unwrap();
    std::fs::write(dir.path().join("myfile.txt"), b"").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    // "my" matches both mysubdir/ and myfile.txt if dirs_only=false, but :cd
    // dispatches complete_path(..., true), leaving only one candidate.
    let prefix = format!("{}/my", dir.path().display());
    let input = format!("cd {prefix}");

    ed.handle_key(key(':'));
    for ch in input.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_tab());

    // One dir-only match → silent complete with trailing '/'.
    let expected = format!("cd {}/mysubdir/", dir.path().display());
    assert_eq!(
        minibuf_input(&ed),
        expected,
        "cd must complete to the directory"
    );
    assert!(
        ed.state.input.completion().is_none(),
        ":cd completion must exclude files, leaving a single dir match"
    );
}

// ── Buffer-name completion ───────────────────────────────────────────────────

/// `:b <Tab>` — `BUFFER_NAME_SOURCE`'s own `complete_buffer_name`, a
/// `MatchKind::String { case_sensitive: true }` source. Previously the only
/// registered source with no end-to-end Tab coverage at all.
#[test]
fn tab_on_buffer_arg_completes_buffer_names() {
    let dir = safe_tempdir();
    let path_a = dir.path().join("alpha-notes.rs");
    let path_b = dir.path().join("alpha-utils.rs");
    std::fs::write(&path_a, "a\n").unwrap();
    std::fs::write(&path_b, "b\n").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    let mut buf_a = Buffer::new(BufferText::from("a\n"), SelectionSet::default());
    buf_a.set_path(Some(path_a));
    ed.open_buffer(buf_a);
    let mut buf_b = Buffer::new(BufferText::from("b\n"), SelectionSet::default());
    buf_b.set_path(Some(path_b));
    ed.open_buffer(buf_b);

    ed.handle_key(key(':'));
    for ch in "b alpha".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_tab());

    let session = ed
        .state
        .input
        .completion()
        .expect(":b <Tab> should open a popup for 2+ matching buffer names");
    let names: Vec<String> = (0..session.len())
        .map(|i| session.selected_item(i).unwrap().insert_text().to_owned())
        .collect();
    assert!(
        names.iter().any(|n| n.ends_with("alpha-notes.rs")),
        "buffer candidate missing: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("alpha-utils.rs")),
        "buffer candidate missing: {names:?}"
    );
}

// ── Directory descent on Enter ────────────────────────────────────────────────

#[test]
fn enter_on_directory_candidate_restarts_completion() {
    let dir = safe_tempdir();
    // Two sub-dirs so the path popup has ≥2 candidates (popup opens).
    std::fs::create_dir(dir.path().join("alpha")).unwrap();
    std::fs::create_dir(dir.path().join("beta")).unwrap();
    // Populate one of them with two files so the descend-restart opens a popup.
    std::fs::write(dir.path().join("alpha/one.txt"), b"").unwrap();
    std::fs::write(dir.path().join("alpha/two.txt"), b"").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    let input = format!("e {}/", dir.path().display());
    ed.handle_key(key(':'));
    for ch in input.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_tab()); // opens popup; "alpha/" selected first (alphabetical).

    let session = ed.state.input.completion().expect("popup should be open");
    let first = session.selected_item(0).unwrap().insert_text().to_owned();
    assert!(
        first.ends_with('/'),
        "expected directory candidate, got {first}"
    );

    ed.handle_key(key_enter());

    // Minibuf stays open — Enter on a dir must not execute the command.
    assert!(
        ed.state.minibuf().is_some(),
        "Enter on dir candidate must keep minibuf open"
    );
    // Input now contains the selected directory.
    let input_now = minibuf_input(&ed);
    assert!(
        input_now.contains("/alpha/"),
        "expected descent through alpha/, got {input_now}"
    );
    // Completion re-triggered with the directory's children.
    let restarted = ed
        .state
        .input
        .completion()
        .expect("completion should restart for dir children");
    assert_eq!(restarted.len(), 2, "expected 2 files under alpha/");
}

// ── Ctrl-w delete-word in minibuf ─────────────────────────────────────────────

#[test]
fn ctrl_w_deletes_word_in_minibuf() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "e foo bar".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "e foo ");
}

#[test]
fn ctrl_w_skips_trailing_whitespace_first() {
    // Readline behaviour: runs of spaces are consumed before the word.
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "e foo   ".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "e ");
}

#[test]
fn ctrl_w_at_start_is_noop_and_keeps_minibuf_open() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "");
    // Unlike Backspace on empty input (which cancels), Ctrl-w is a no-op.
    assert!(
        ed.state.minibuf().is_some(),
        "Ctrl-w on empty input must not close the minibuf"
    );
}

#[test]
fn ctrl_w_stops_at_slash_for_path_args() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "e /tmp/alpha/one.txt".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "e /tmp/alpha/");
}

#[test]
fn ctrl_w_on_trailing_slash_deletes_dir_component() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "e /tmp/alpha/".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "e /tmp/");
}

#[test]
fn ctrl_w_dismisses_open_completion_popup() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_tab()); // opens popup for "w"-prefixed commands
    assert!(
        ed.state.input.completion().is_some(),
        "sanity: popup should be open"
    );

    ed.handle_key(key_ctrl('w'));
    // Edited event clears completion; Ctrl-w consumed the word ("w"-based candidate).
    assert!(
        ed.state.input.completion().is_none(),
        "Ctrl-w must dismiss the popup"
    );
}

#[test]
fn ctrl_w_works_in_search_minibuf() {
    // Ctrl-w in a `/` search prompt deletes the last word without cancelling.
    let mut ed = editor_from("-[h]>ello world\n");
    ed.handle_key(key('/'));
    assert_eq!(ed.state.mode(), Mode::Search);
    for ch in "foo bar".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(ed.state.minibuf().unwrap().input, "foo bar");
    ed.handle_key(key_ctrl('w'));
    assert_eq!(ed.state.minibuf().unwrap().input, "foo ");
    // Search minibuf must still be open.
    assert!(
        ed.state.minibuf().is_some(),
        "Ctrl-w must not close the search minibuf"
    );
    assert_eq!(ed.state.mode(), Mode::Search);
    ed.handle_key(key_esc());
}

#[test]
fn ctrl_w_at_start_of_search_minibuf_is_noop() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.handle_key(key('/'));
    // Nothing typed yet — Ctrl-w on empty input is a no-op.
    ed.handle_key(key_ctrl('w'));
    assert!(
        ed.state.minibuf().is_some(),
        "Ctrl-w on empty search input must not close the minibuf"
    );
    assert_eq!(ed.state.mode(), Mode::Search);
    ed.handle_key(key_esc());
}

// ── :set completion (integration) ─────────────────────────────────────────────

/// `:set ` with no further input opens the scope popup (>=2 candidates), so
/// Tab opens the popup rather than silently completing.
#[test]
fn tab_on_set_opens_scope_popup() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "set".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key(' '));
    ed.handle_key(key_tab());

    let session = ed
        .state
        .input
        .completion()
        .expect(":set <space> should open scope popup");
    let names: Vec<String> = (0..session.len())
        .map(|i| session.selected_item(i).unwrap().insert_text().to_owned())
        .collect();
    assert!(names.iter().any(|n| n == "global"));
    assert!(names.iter().any(|n| n == "buffer"));
    assert!(names.iter().any(|n| n == "pane"));
}

/// `:set g` is a single unique scope match → silent completion, no popup.
#[test]
fn tab_on_set_g_silently_completes_global() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "set g".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_tab());
    assert_eq!(minibuf_input(&ed), "set global");
    assert!(ed.state.input.completion().is_none());
}

// ── Render snapshot ──────────────────────────────────────────────────────────

/// The minibuffer completion popup renders through the same generic
/// `resolve_menu` + `PopupOverlay` every other menu-shaped overlay uses,
/// not a bespoke widget of its own.
#[test]
fn minibuf_completion_popup_renders_above_the_statusline() {
    // `Editor::open`, not `editor_from`: `Editor::for_testing` never goes
    // through `build_pane`, so no overlay providers are registered and the
    // popup would silently not paint.
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::testing::build_snapshot_theme();

    // "w" matches exactly three canonical command names: write, write-all,
    // write-quit. `complete_command` omits aliases (w/wa/wq/wrap) and the
    // exact-prefix match, so this candidate set is stable against new
    // commands being registered elsewhere.
    ed.feed_key(key(':'));
    ed.feed_key(key('w'));
    ed.feed_key(key_tab());

    let snap = render_snapshot::render_to_styled_string(&mut ed, Rect::new(0, 0, 40, 10));
    insta::assert_snapshot!(snap);
}
