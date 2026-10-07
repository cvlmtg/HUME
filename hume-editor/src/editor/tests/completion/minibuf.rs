//! The `:` line's Tab completion: `trigger_minibuf_completion`
//! (`completion/orchestrate.rs`) and `completion_input_minibuf`
//! (`input_stack/completion.rs`): the native sources, the eager single-
//! match policy, cycling, directory descent, and what dismisses.

use super::*;
use crate::editor::buffer::Buffer;
use crate::editor::settings::CommandCompletion;
use hume_editing::text::BufferText;
use pretty_assertions::assert_eq;

/// Every candidate's `insert_text`, in ranked order.
fn candidates(ed: &Editor) -> Vec<String> {
    let session = ed.state.input.minibuf_completion().expect("popup open");
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

/// An editor whose `:` line applies the first candidate on Tab.
fn first_candidate_editor(input: &str) -> Editor {
    let mut ed = editor_from(input);
    ed.state.settings.command_completion = CommandCompletion::FirstCandidate;
    ed
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
        ed.state.input.minibuf_completion().is_none(),
        "single match: no popup"
    );
}

#[test]
fn tab_no_match_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "zzz");
    ed.handle_key(key_tab());
    assert_eq!(minibuf_input(&ed), "zzz");
    assert!(ed.state.input.minibuf_completion().is_none());
}

// Expected values for the three tests below: "w" matches exactly three
// canonical command names, alphabetically (tied score, sortText-ascending
// tiebreak, since `complete_command`'s own sort_text is the name itself):
// write, write-all, write-quit. `complete_command` omits aliases and the
// exact-prefix match, so this candidate set is stable against new commands
// (see `render.rs`'s own use of the same fact).

#[test]
fn tab_multiple_matches_opens_popup_with_first_candidate_applied() {
    let mut ed = first_candidate_editor("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    assert!(ed.state.input.minibuf_completion().is_some(), "popup open");
    assert_eq!(selected_row(&ed), 0);
    assert_eq!(candidates(&ed), vec!["write", "write-all", "write-quit"]);
    assert_eq!(minibuf_input(&ed), "write");
}

#[test]
fn second_tab_cycles_to_next_candidate() {
    let mut ed = first_candidate_editor("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    ed.handle_key(key_tab());
    assert_eq!(selected_row(&ed), 1);
    assert_eq!(minibuf_input(&ed), "write-all");
}

#[test]
fn shift_tab_cycles_backward() {
    let mut ed = first_candidate_editor("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    ed.handle_key(key_tab());
    ed.handle_key(key_shift_tab());
    assert_eq!(selected_row(&ed), 0);
    assert_eq!(minibuf_input(&ed), "write");
}

#[test]
fn tab_wraps_at_end() {
    let mut ed = first_candidate_editor("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    let n = ed.state.input.minibuf_completion().unwrap().len();
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
    assert!(ed.state.input.minibuf_completion().is_some());
    ed.handle_key(key('r'));
    assert!(ed.state.input.minibuf_completion().is_none());
}

#[test]
fn enter_mid_completion_executes_the_applied_candidate() {
    // ":quit" is a unique completion for "qui".
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "qui");
    ed.handle_key(key_tab());
    ed.handle_key(key_enter());
    assert!(ed.state.should_quit);
    assert!(ed.state.input.minibuf_completion().is_none());
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
    assert!(ed.state.input.minibuf_completion().is_none());
}

#[test]
fn shift_tab_with_no_popup_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "wri");
    ed.handle_key(key_shift_tab());
    assert_eq!(minibuf_input(&ed), "wri");
    assert!(ed.state.input.minibuf_completion().is_none());
}

#[test]
fn tab_in_search_mode_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('/'));
    ed.handle_key(key('e'));
    ed.handle_key(key_tab());
    assert_eq!(minibuf_input(&ed), "e");
    assert!(ed.state.input.minibuf_completion().is_none());
}

/// Ctrl-w edits the input, so it dismisses the popup like any other key.
#[test]
fn ctrl_w_dismisses_the_open_popup() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    assert!(ed.state.input.minibuf_completion().is_some(), "sanity");
    ed.handle_key(key_ctrl('w'));
    assert!(ed.state.input.minibuf_completion().is_none());
}

// ── Common-prefix completion (the default) ───────────────────────────────────

#[test]
fn tab_extends_the_token_to_the_common_prefix_and_picks_nothing() {
    // "write", "write-all", "write-quit" share "write".
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    assert_eq!(minibuf_input(&ed), "write");
    assert_eq!(candidates(&ed), vec!["write", "write-all", "write-quit"]);
    assert_eq!(picked_row(&ed), None);
}

#[test]
fn tab_leaves_the_input_alone_when_the_prefix_adds_nothing() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "set ");
    ed.handle_key(key_tab());
    assert_eq!(minibuf_input(&ed), "set ");
    assert!(ed.state.input.minibuf_completion().is_some(), "popup open");
    assert_eq!(picked_row(&ed), None);
}

#[test]
fn tab_after_the_prefix_picks_the_first_candidate_then_cycles() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    ed.handle_key(key_tab());
    assert_eq!(picked_row(&ed), Some(0));
    assert_eq!(minibuf_input(&ed), "write");
    ed.handle_key(key_tab());
    assert_eq!(picked_row(&ed), Some(1));
    assert_eq!(minibuf_input(&ed), "write-all");
}

#[test]
fn shift_tab_after_the_prefix_picks_the_last_candidate() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    ed.handle_key(key_shift_tab());
    assert_eq!(picked_row(&ed), Some(2));
    assert_eq!(minibuf_input(&ed), "write-quit");
}

#[test]
fn enter_after_a_prefix_only_tab_runs_the_typed_line() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    ed.handle_key(key_enter());
    assert!(ed.state.minibuf().is_none());
    assert!(ed.state.input.minibuf_completion().is_none());
}

#[test]
fn a_fuzzy_prefix_shorter_than_the_typed_text_is_not_applied() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(register-completion-source! "names"
             (lambda (id input cursor)
               (completion-emit! id (list (hash "label" "xab1") (hash "label" "xab2"))))
             #:target 'minibuf #:match 'fuzzy)
           (define-typed-command! "greet" "" (lambda (bid arg) (log! 'info arg)) #:complete "names")"#,
    );
    ed.handle_key(key(':'));
    type_chars(&mut ed, "greet ab");
    ed.handle_key(key_tab());
    ed.settle();
    assert_eq!(minibuf_input(&ed), "greet ab");
    assert_eq!(candidates(&ed).len(), 2);
}

#[test]
fn the_common_prefix_never_splits_a_grapheme_cluster() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        "(register-completion-source! \"names\"
           (lambda (id input cursor)
             (completion-emit! id (list (hash \"label\" \"ae\u{301}x\") (hash \"label\" \"aey\"))))
           #:target 'minibuf #:match 'string)
         (define-typed-command! \"greet\" \"\" (lambda (bid arg) (log! 'info arg)) #:complete \"names\")",
    );
    ed.handle_key(key(':'));
    type_chars(&mut ed, "greet a");
    ed.handle_key(key_tab());
    ed.settle();
    assert_eq!(minibuf_input(&ed), "greet a");
}

#[test]
fn a_later_answer_drops_the_pick_and_proposes_the_common_prefix() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define captured-id #f)
           (register-completion-source! "names"
             (lambda (id input cursor) (set! captured-id id))
             #:target 'minibuf #:match 'string)
           (define-typed-command! "greet" "" (lambda (bid arg) (log! 'info arg)) #:complete "names")
           (define-command! "answer-many" "" (lambda ()
             (completion-emit! captured-id
               (list (hash "label" "alice") (hash "label" "bob") (hash "label" "carol")))))
           (define-command! "answer-pair" "" (lambda ()
             (completion-emit! captured-id
               (list (hash "label" "bobby") (hash "label" "bobcat")))))"#,
    );
    ed.handle_key(key(':'));
    type_chars(&mut ed, "greet ");
    ed.handle_key(key_tab());
    ed.settle();

    ed.execute_keymap_command("answer-many".into(), None, false);
    assert_eq!(minibuf_input(&ed), "greet ");
    assert_eq!(picked_row(&ed), None);

    ed.handle_key(key_tab());
    ed.handle_key(key_tab());
    assert_eq!(picked_row(&ed), Some(1));
    assert_eq!(minibuf_input(&ed), "greet bob");

    ed.execute_keymap_command("answer-pair".into(), None, false);
    assert_eq!(picked_row(&ed), None, "the new ranking has no pick");
    assert_eq!(candidates(&ed).len(), 2);
    assert_eq!(minibuf_input(&ed), "greet bob");
}

#[test]
fn a_mid_token_tab_with_several_candidates_leaves_the_line_alone() {
    let mut ed = editor_from("-[h]>ello\n");
    command_line(&mut ed, "write-all");
    for _ in 0.."rite-all".len() {
        ed.handle_key(key_left());
    }
    ed.handle_key(key_tab());
    assert_eq!(minibuf_input(&ed), "write-all");
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
    // Cursor back to just after "hel": mid-token, "lo.txt extra" ahead.
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
    // Cursor back to just after "th". A space-only forward scan for the
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
    assert!(ed.state.input.minibuf_completion().is_none());
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
        ed.state.input.minibuf_completion().is_none(),
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

    let mut ed = first_candidate_editor("-[h]>ello\n");
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
        ed.state
            .input
            .minibuf_completion()
            .expect("restarted")
            .len(),
        2,
        "the directory's children"
    );
}

/// A non-path source's candidate that happens to end in `/` (a namespaced
/// tag, say) must not be treated as a directory to descend into. Only an
/// item whose own `kind` is `CompletionItemKind::FOLDER` licenses that, and
/// this source's items carry no `kind` at all. Enter runs the command line
/// as normal instead of restarting completion. Two candidates
/// (not one) so the popup stays open after Tab instead of the `:` line's
/// own single-match eager-apply-and-dismiss closing it before Enter is
/// even reachable. `enter_on_directory_candidate_restarts_completion_
/// inside_it`'s own multi-candidate directory hits the same shape.
#[test]
fn enter_on_a_non_path_candidate_ending_in_slash_does_not_restart_completion() {
    let tmp = safe_tempdir();
    let mut ed = first_candidate_editor("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(register-completion-source! "tags"
             (lambda (id input cursor)
               (completion-emit! id (list (hash "label" "ns/") (hash "label" "other"))))
             #:target 'minibuf #:match 'string)
           (define-typed-command! "tag" "" (lambda (bid arg) (log! 'info arg)) #:complete "tags")"#,
    );
    ed.handle_key(key(':'));
    type_chars(&mut ed, "tag ");
    ed.handle_key(key_tab());
    ed.settle();
    assert_eq!(
        minibuf_input(&ed),
        "tag ns/",
        "sanity: popup open, first candidate applied"
    );
    assert!(
        ed.state.input.minibuf_completion().is_some(),
        "sanity: two candidates keep the popup open"
    );

    ed.handle_key(key_enter());

    assert!(
        ed.state.minibuf().is_none(),
        "Enter must run the command, not descend into \"ns/\" as a directory"
    );
    assert_eq!(
        status(&ed),
        "ns/",
        "the typed command ran with its argument"
    );
}

/// `arg_span` (a Steel `'minibuf` source's own token) must be the argument
/// the cursor is *in*, not everything after the command name: a
/// multi-argument typed command's second argument must not drag the first
/// one along into the span/filter. `"bexyz"` only
/// `starts_with` `"be"` (the second argument alone), never `"alpha be"`
/// (what a first-space split would wrongly hand over).
#[test]
fn a_minibuf_source_gets_only_the_argument_the_cursor_is_in() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(register-completion-source! "two-arg"
             (lambda (id input cursor) (completion-emit! id (list (hash "label" "bexyz"))))
             #:target 'minibuf #:match 'string)
           (define-typed-command! "mycmd" "" (lambda (bid arg) (log! 'info arg)) #:complete "two-arg")"#,
    );
    ed.handle_key(key(':'));
    type_chars(&mut ed, "mycmd alpha be");
    ed.handle_key(key_tab());
    ed.settle();
    assert_eq!(
        minibuf_input(&ed),
        "mycmd alpha bexyz",
        "only the second argument (\"be\") should have been replaced"
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
        let mut buf = Buffer::at_start(BufferText::from("a\n"));
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

/// `:b1<Tab>` (the alias `b`, an argument starting with a digit, no space)
/// must complete `1` as `:buffer`'s own argument, not as a command name
/// still being typed. `scan_command_name`'s letter-only name rule (shared
/// with `parse_typed_command`, what `execute_command` itself uses) ends
/// the name at `b`, matching what Enter would actually run.
#[test]
fn tab_mid_alias_with_no_space_completes_the_declared_arg_not_the_command_name() {
    let dir = safe_tempdir();
    let path = dir.path().join("1-notes.rs");
    std::fs::write(&path, "a\n").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    let mut buf = Buffer::at_start(BufferText::from("a\n"));
    buf.set_path(Some(path));
    ed.open_buffer(buf);

    command_line(&mut ed, "b1");
    ed.handle_key(key_tab());
    // A sole candidate applies silently (the `:` line's eager policy):
    // the buffer-name completer's own full-path insert_text lands in place
    // of "1" if (and only if) it's the source that actually answered.
    assert!(
        minibuf_input(&ed).ends_with("1-notes.rs"),
        "the alias's own arg completer (buffer-name) must answer, not the \
         command-name universe (no command starts with \"1\"): {:?}",
        minibuf_input(&ed)
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
    assert!(ed.state.input.minibuf_completion().is_none());
}

// ── A Buffer-only builtin reaching a Minibuf session ───────────────────────

/// `completion-accept!` while the `:` popup is open (a plugin or async
/// callback firing at the wrong moment) must get an `Err`, not a panic.
#[test]
fn completion_accept_on_a_minibuffer_session_errors_instead_of_aborting() {
    let tmp = safe_tempdir();
    let mut ed = first_candidate_editor("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    assert!(ed.state.input.minibuf_completion().is_some(), "sanity");

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
    // The error must be checked *before* the session is torn down: a
    // rejected `completion-accept!` must leave the popup exactly as it
    // was, not destroy it on the way to discovering it was the wrong call.
    assert!(
        ed.state.input.minibuf_completion().is_some(),
        "the session must survive a rejected completion-accept!"
    );
    ed.handle_key(key_tab());
    assert_eq!(
        selected_row(&ed),
        1,
        "the popup must still be usable: a follow-up Tab still cycles"
    );
}

/// A `Minibuf` source that raises instead of ever calling `completion-emit!`
/// must not leave the popup stuck pending forever. `drop_stalled_
/// invocations` clears the slot once the failed call batch is reported, so
/// `settle_minibuf_session` runs its ordinary "nothing landed" policy
/// (dismiss) instead of `is_pending()` staying `true` with no source left
/// that will ever answer.
#[test]
fn a_raising_minibuf_source_does_not_leave_the_popup_stuck_pending() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(register-completion-source! "boom"
             (lambda (id input cursor) (error "boom"))
             #:target 'minibuf #:match 'string)
           (define-typed-command! "mycmd" "" (lambda (bid arg) (log! 'info arg)) #:complete "boom")"#,
    );
    ed.handle_key(key(':'));
    type_chars(&mut ed, "mycmd ");
    ed.handle_key(key_tab());
    ed.settle();
    assert!(
        status(&ed).contains("boom"),
        "sanity: the error was reported: {:?}",
        status(&ed)
    );
    assert!(
        ed.state.input.minibuf_completion().is_none(),
        "a source that will never answer must not leave the popup stuck \
         pending forever"
    );
}

// ── A streaming source re-ranking under a moved selection ──────────────────

/// A `Minibuf` source may answer more than once for the same invocation
/// (the async streaming case `docs/COMPLETION-PICKER.md` documents): once
/// with a wide answer, again later (via a separate command here, standing
/// in for a real async callback) with a narrower one. Between the two, the
/// user Tabs the selection off row 0. `settle_minibuf_session`'s own
/// re-rank must reset that selection the same way `rerank_open_session`
/// does for a `Buffer` session, or the splice on the second answer reads a
/// selection index the new, shorter list no longer has.
#[test]
fn a_second_answer_settles_against_a_reset_selection_not_a_stale_one() {
    let tmp = safe_tempdir();
    let mut ed = first_candidate_editor("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define captured-id #f)
           (register-completion-source! "names"
             (lambda (id input cursor) (set! captured-id id))
             #:target 'minibuf #:match 'string)
           (define-typed-command! "greet" "" (lambda (bid arg) (log! 'info arg)) #:complete "names")
           (define-command! "answer-many" "" (lambda ()
             (completion-emit! captured-id
               (list (hash "label" "alice") (hash "label" "bob") (hash "label" "carol")))))
           (define-command! "answer-one" "" (lambda ()
             (completion-emit! captured-id (list (hash "label" "zzz")))))"#,
    );
    ed.handle_key(key(':'));
    type_chars(&mut ed, "greet ");
    ed.handle_key(key_tab());
    ed.settle();
    assert!(
        ed.state
            .input
            .minibuf_completion()
            .is_some_and(|s| s.is_pending()),
        "sanity: still pending"
    );

    ed.execute_keymap_command("answer-many".into(), None, false);
    assert_eq!(candidates(&ed).len(), 3, "sanity: first answer landed");
    assert_eq!(minibuf_input(&ed), "greet alice");

    ed.handle_key(key_tab());
    ed.handle_key(key_tab());
    assert_eq!(selected_row(&ed), 2, "sanity: selection moved off row 0");
    assert_eq!(minibuf_input(&ed), "greet carol");

    ed.execute_keymap_command("answer-one".into(), None, false);
    assert_eq!(
        minibuf_input(&ed),
        "greet zzz",
        "the second, shorter answer must settle against a reset selection, \
         not silently fail to splice under the stale row-2 index"
    );
}
