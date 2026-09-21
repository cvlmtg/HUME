//! The `:` line's Tab completion — `trigger_minibuf_completion`
//! (`completion/orchestrate.rs`) and `completion_input_minibuf`
//! (`input_stack/completion.rs`): the native sources, the eager single-
//! match policy, cycling, directory descent, and what dismisses.

use super::*;
use crate::editor::buffer::Buffer;
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use pretty_assertions::assert_eq;

fn minibuf_input(ed: &Editor) -> &str {
    ed.state.minibuf().map(|mb| mb.input.as_str()).unwrap_or("")
}

/// Every candidate's `insert_text`, in ranked order.
fn candidates(ed: &Editor) -> Vec<String> {
    let session = ed.state.input.completion().expect("popup open");
    (0..session.len())
        .map(|i| session.selected_item(i).unwrap().insert_text().to_owned())
        .collect()
}

/// `:` then `input`, through `handle_key`.
fn command_line(ed: &mut Editor, input: &str) {
    ed.handle_key(key(':'));
    for ch in input.chars() {
        ed.handle_key(key(ch));
    }
}

// ── Command-name completion ───────────────────────────────────────────────────

#[test]
fn tab_on_command_prefix_single_match_completes_silently() {
    // "reload-config" is the only registered command starting with "relo".
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "relo");
    ed.handle_key(key_tab());
    assert_eq!(minibuf_input(&ed), "reload-config");
    assert!(
        ed.state.input.completion().is_none(),
        "single match: no popup"
    );
}

#[test]
fn tab_no_match_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "zzz");
    ed.handle_key(key_tab());
    assert_eq!(minibuf_input(&ed), "zzz");
    assert!(ed.state.input.completion().is_none());
}

#[test]
fn tab_multiple_matches_opens_popup_with_first_candidate_applied() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    assert!(ed.state.input.completion().is_some(), "popup open");
    assert_eq!(selected_row(&ed), 0);
    let first = candidates(&ed);
    assert!(first.len() >= 2);
    assert_eq!(minibuf_input(&ed), first[0]);
}

#[test]
fn second_tab_cycles_to_next_candidate() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    ed.handle_key(key_tab());
    assert_eq!(selected_row(&ed), 1);
    assert_eq!(minibuf_input(&ed), candidates(&ed)[1]);
}

#[test]
fn shift_tab_cycles_backward() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    ed.handle_key(key_tab());
    ed.handle_key(key_shift_tab());
    assert_eq!(selected_row(&ed), 0);
    assert_eq!(minibuf_input(&ed), candidates(&ed)[0]);
}

#[test]
fn tab_wraps_at_end() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    let n = ed.state.input.completion().unwrap().len();
    for _ in 0..n {
        ed.handle_key(key_tab());
    }
    assert_eq!(selected_row(&ed), 0);
}

#[test]
fn typing_a_char_dismisses_the_popup() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    assert!(ed.state.input.completion().is_some());
    ed.handle_key(key('r'));
    assert!(ed.state.input.completion().is_none());
}

#[test]
fn enter_mid_completion_executes_the_applied_candidate() {
    // ":quit" is a unique completion for "qui".
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "qui");
    ed.handle_key(key_tab());
    ed.handle_key(key_enter());
    assert!(ed.state.should_quit);
    assert!(ed.state.input.completion().is_none());
    assert!(ed.state.minibuf().is_none());
}

#[test]
fn esc_dismisses_minibuf_and_clears_completion() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    ed.handle_key(key_esc());
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(ed.state.minibuf().is_none());
    assert!(ed.state.input.completion().is_none());
}

#[test]
fn shift_tab_with_no_popup_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "wri");
    ed.handle_key(key_shift_tab());
    assert_eq!(minibuf_input(&ed), "wri");
    assert!(ed.state.input.completion().is_none());
}

#[test]
fn tab_in_search_mode_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('/'));
    ed.handle_key(key('e'));
    ed.handle_key(key_tab());
    assert_eq!(minibuf_input(&ed), "e");
    assert!(ed.state.input.completion().is_none());
}

/// Ctrl-w edits the input, so it dismisses the popup like any other key.
#[test]
fn ctrl_w_dismisses_the_open_popup() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    assert!(ed.state.input.completion().is_some(), "sanity");
    ed.handle_key(key_ctrl('w'));
    assert!(ed.state.input.completion().is_none());
}

// ── Mid-token Tab: the replaced span is the token, not up to the cursor ─────

#[test]
fn tab_mid_token_replaces_the_whole_path_not_just_up_to_the_cursor() {
    let dir = safe_tempdir();
    std::fs::write(dir.path().join("hello.txt"), b"").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    command_line(
        &mut ed,
        &format!("e {}/hello.txt extra", dir.path().display()),
    );
    // Cursor back to just after "hel" — mid-token, "lo.txt extra" ahead.
    for _ in 0.."lo.txt extra".len() {
        ed.handle_key(key_left());
    }
    ed.handle_key(key_tab());
    assert_eq!(
        minibuf_input(&ed),
        format!("e {}/hello.txt extra", dir.path().display())
    );
}

#[test]
fn tab_mid_command_name_replaces_the_whole_name() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "reload-config");
    for _ in 0.."ad-config".len() {
        ed.handle_key(key_left());
    }
    ed.handle_key(key_tab());
    assert_eq!(minibuf_input(&ed), "reload-config");
}

#[test]
fn tab_mid_set_key_does_not_swallow_the_equals_value() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "set global theme=x");
    // Cursor back to just after "th" — a space-only forward scan for the
    // span's end would swallow the trailing "=x".
    for _ in 0.."eme=x".len() {
        ed.handle_key(key_left());
    }
    ed.handle_key(key_tab());
    assert_eq!(minibuf_input(&ed), "set global theme=x");
}

// ── Path completion ───────────────────────────────────────────────────────────

#[test]
fn tab_on_edit_arg_completes_path() {
    let dir = safe_tempdir();
    std::fs::write(dir.path().join("hello.txt"), b"").unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, &format!("e {}/hel", dir.path().display()));
    ed.handle_key(key_tab());
    assert_eq!(
        minibuf_input(&ed),
        format!("e {}/hello.txt", dir.path().display())
    );
    assert!(ed.state.input.completion().is_none());
}

#[test]
fn tab_on_write_arg_completes_path() {
    let dir = safe_tempdir();
    std::fs::write(dir.path().join("out.txt"), b"").unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, &format!("w {}/out", dir.path().display()));
    ed.handle_key(key_tab());
    assert_eq!(
        minibuf_input(&ed),
        format!("w {}/out.txt", dir.path().display())
    );
}

#[test]
fn tab_on_cd_arg_completes_dirs_only() {
    let dir = safe_tempdir();
    std::fs::create_dir(dir.path().join("mysubdir")).unwrap();
    std::fs::write(dir.path().join("myfile.txt"), b"").unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, &format!("cd {}/my", dir.path().display()));
    ed.handle_key(key_tab());
    assert_eq!(
        minibuf_input(&ed),
        format!("cd {}/mysubdir/", dir.path().display())
    );
    assert!(
        ed.state.input.completion().is_none(),
        ":cd excludes files, leaving a single dir match"
    );
}

#[test]
fn enter_on_directory_candidate_restarts_completion_inside_it() {
    let dir = safe_tempdir();
    std::fs::create_dir(dir.path().join("alpha")).unwrap();
    std::fs::create_dir(dir.path().join("beta")).unwrap();
    std::fs::write(dir.path().join("alpha/one.txt"), b"").unwrap();
    std::fs::write(dir.path().join("alpha/two.txt"), b"").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, &format!("e {}/", dir.path().display()));
    ed.handle_key(key_tab()); // "alpha/" first (alphabetical)
    assert!(candidates(&ed)[0].ends_with('/'), "sanity: a directory");

    ed.handle_key(key_enter());

    assert!(
        ed.state.minibuf().is_some(),
        "Enter on a dir must not execute"
    );
    assert!(minibuf_input(&ed).contains("/alpha/"));
    assert_eq!(
        ed.state.input.completion().expect("restarted").len(),
        2,
        "the directory's children"
    );
}

// ── Buffer-name completion ───────────────────────────────────────────────────

#[test]
fn tab_on_buffer_arg_completes_buffer_names() {
    let dir = safe_tempdir();
    let path_a = dir.path().join("alpha-notes.rs");
    let path_b = dir.path().join("alpha-utils.rs");
    std::fs::write(&path_a, "a\n").unwrap();
    std::fs::write(&path_b, "b\n").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    for path in [path_a, path_b] {
        let mut buf = Buffer::new(BufferText::from("a\n"), SelectionSet::default());
        buf.set_path(Some(path));
        ed.open_buffer(buf);
    }

    command_line(&mut ed, "b alpha");
    ed.handle_key(key_tab());
    let names = candidates(&ed);
    assert!(
        names.iter().any(|n| n.ends_with("alpha-notes.rs")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("alpha-utils.rs")),
        "{names:?}"
    );
}

// ── :set completion ───────────────────────────────────────────────────────────

#[test]
fn tab_on_set_opens_scope_popup() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "set ");
    ed.handle_key(key_tab());
    let names = candidates(&ed);
    for scope in ["global", "buffer", "pane"] {
        assert!(names.iter().any(|n| n == scope), "{names:?}");
    }
}

#[test]
fn tab_on_set_g_silently_completes_global() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "set g");
    ed.handle_key(key_tab());
    assert_eq!(minibuf_input(&ed), "set global");
    assert!(ed.state.input.completion().is_none());
}

// ── A Buffer-only builtin reaching a Minibuf session ───────────────────────

/// `completion-accept!` while the `:` popup is open — a plugin or async
/// callback firing at the wrong moment — must get an `Err`, not a panic.
#[test]
fn completion_accept_on_a_minibuffer_session_errors_instead_of_aborting() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    assert!(ed.state.input.completion().is_some(), "sanity");

    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "check" "" (lambda () (completion-accept! 0)))"#,
    );
    ed.execute_keymap_command("check".into(), None, false);
    assert!(
        status(&ed).contains("not a buffer-target session"),
        "got {:?}",
        status(&ed)
    );
}
