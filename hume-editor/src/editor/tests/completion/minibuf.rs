//! The `:` line's Tab completion: `trigger_minibuf_completion`
//! (`completion/orchestrate.rs`) and `completion_input_minibuf`
//! (`input_stack/completion.rs`): the native sources, the eager single-
//! match policy, cycling, directory descent, and what dismisses.

use super::*;
use crate::editor::buffer::Buffer;
use crate::editor::settings::CommandCompletion;
use hume_editing::text::BufferText;
use pretty_assertions::assert_eq;

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

/// A prefix typed in another case completes to the command's own spelling.
#[test]
fn tab_on_command_prefix_in_another_case_completes_canonical_spelling() {
    let mut ed = editor_from("-[h]>ello\n");
    attach_steel(
        &mut ed,
        r#"(define-typed-command! "FooBar" "" (lambda () #t))"#,
    );
    for (typed, expected) in [("RELO", "reload-config"), ("foob", "FooBar")] {
        command_line(&mut ed, typed);
        ed.handle_key(key_tab());
        assert_eq!(minibuf_input(&ed), expected, "{typed}");
        ed.handle_key(key_esc());
    }
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
    assert_eq!(
        minibuf_candidates(&ed),
        vec!["write", "write-all", "write-quit"]
    );
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
    assert_eq!(
        minibuf_candidates(&ed),
        vec!["write", "write-all", "write-quit"]
    );
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
    let tmp = tempfile::tempdir().unwrap();
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
    assert_eq!(minibuf_candidates(&ed).len(), 2);
}

#[test]
fn the_common_prefix_never_splits_a_grapheme_cluster() {
    let tmp = tempfile::tempdir().unwrap();
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
    let mut ed = greet_answered_by(
        "",
        &[
            (
                "answer-many",
                r#"(list "alice" "bob" "carol")"#,
                "#:more #t",
            ),
            ("answer-pair", r#"(list "bobby" "bobcat")"#, ""),
        ],
    );

    call(&mut ed, "answer-many");
    assert_eq!(minibuf_input(&ed), "greet ");
    assert_eq!(picked_row(&ed), None);

    ed.handle_key(key_tab());
    ed.handle_key(key_tab());
    assert_eq!(picked_row(&ed), Some(1));
    assert_eq!(minibuf_input(&ed), "greet bob");

    call(&mut ed, "answer-pair");
    assert_eq!(picked_row(&ed), None, "the new ranking has no pick");
    assert_eq!(minibuf_candidates(&ed).len(), 2);
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
    let dir = tempfile::tempdir().unwrap();
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
    let dir = tempfile::tempdir().unwrap();
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
    let dir = tempfile::tempdir().unwrap();
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
    let dir = tempfile::tempdir().unwrap();
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
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("alpha")).unwrap();
    std::fs::create_dir(dir.path().join("beta")).unwrap();
    std::fs::write(dir.path().join("alpha/one.txt"), b"").unwrap();
    std::fs::write(dir.path().join("alpha/two.txt"), b"").unwrap();

    let mut ed = first_candidate_editor("-[h]>ello\n");
    command_line(&mut ed, &format!("e {}/", dir.path().display()));
    ed.handle_key(key_tab()); // "alpha/" first (alphabetical)
    assert!(
        minibuf_candidates(&ed)[0].ends_with('/'),
        "sanity: a directory"
    );

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
    let tmp = tempfile::tempdir().unwrap();
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
    let tmp = tempfile::tempdir().unwrap();
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

/// Steel indexes a string by char, so the cursor a source gets counts
/// chars: `greet é` is 7 chars and 8 bytes long.
#[test]
fn a_minibuf_source_gets_the_cursor_as_a_char_offset() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(register-completion-source! "cursor-echo"
             (lambda (id input cursor)
               (completion-emit! id (list (string-append "é" (to-string cursor)))))
             #:target 'minibuf #:match 'delegated)
           (define-typed-command! "greet" "" (lambda (bid arg) (log! 'info arg)) #:complete "cursor-echo")"#,
    );
    ed.handle_key(key(':'));
    type_chars(&mut ed, "greet é");
    ed.handle_key(key_tab());
    ed.settle();
    assert_eq!(minibuf_input(&ed), "greet é7");
}

// ── A source's own span ──────────────────────────────────────────────────────

/// `:greet <typed>` on an open line, after the Scheme `prelude` has run,
/// completed by the `'minibuf` source `(lambda (id input cursor) <body>)`
/// registered with the extra keyword arguments `opts`.
fn greet_source(typed: &str, prelude: &str, body: &str, opts: &str) -> Editor {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"{prelude}
               (register-completion-source! "names"
                 (lambda (id input cursor) {body})
                 #:target 'minibuf {opts})
               (define-typed-command! "greet" "" (lambda (bid arg) (log! 'info arg)) #:complete "names")"#
        ),
    );
    ed.handle_key(key(':'));
    type_chars(&mut ed, &format!("greet {typed}"));
    ed
}

/// Presses Tab and lets the source it calls answer.
fn tab(mut ed: Editor) -> Editor {
    ed.handle_key(key_tab());
    ed.settle();
    ed
}

/// `:greet <typed>` on an open line, completed by a `match_kind` source
/// that answers `items` (a Scheme list) with the extra `completion-emit!`
/// arguments `emit`.
fn greet_line(typed: &str, match_kind: &str, items: &str, emit: &str) -> Editor {
    greet_source(
        typed,
        "",
        &format!("(completion-emit! id {items} {emit})"),
        &format!("#:match '{match_kind}"),
    )
}

/// [`greet_line`] with Tab pressed.
fn greet_tab(typed: &str, match_kind: &str, items: &str, emit: &str) -> Editor {
    tab(greet_line(typed, match_kind, items, emit))
}

/// [`greet_tab`] on `"my no` with a source answering `"my notes"`.
fn greet_with_a_spanning_source(match_kind: &str, emit: &str) -> Editor {
    greet_tab("\"my no", match_kind, r#"(list "\"my notes\"")"#, emit)
}

/// Whether a `completion-emit!` error naming `#:span` was reported.
fn span_refused(ed: &Editor) -> bool {
    ed.state
        .message_log
        .entries()
        .any(|e| e.severity == Severity::Error && e.text.contains("#:span"))
}

/// Nothing asks a `'minibuf` source again while its menu is open, so an
/// answer that asks for it is refused and its items are not shown.
#[test]
fn an_incomplete_answer_from_a_minibuf_source_is_refused() {
    let ed = greet_tab("", "string", r#"(list "alpha" "beta")"#, "#:incomplete #t");
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error
                && e.text.contains("#:incomplete is for a 'buffer source")),
        "expected a raised #:incomplete error in the message log"
    );
    assert_eq!(minibuf_input(&ed), "greet ");
}

#[test]
fn a_delegated_source_completes_the_span_it_names() {
    let ed = greet_with_a_spanning_source("delegated", "#:span (hash 'start-char 6 'end-char 12)");
    assert_eq!(minibuf_input(&ed), "greet \"my notes\"");
}

#[test]
fn a_span_from_a_source_the_editor_matches_for_is_refused() {
    let ed = greet_with_a_spanning_source("string", "#:span (hash 'start-char 6 'end-char 12)");
    assert!(span_refused(&ed));
    assert_eq!(minibuf_input(&ed), "greet \"my no");
}

#[test]
fn a_span_that_leaves_out_the_cursor_is_refused() {
    let ed = greet_with_a_spanning_source("delegated", "#:span (hash 'start-char 0 'end-char 5)");
    assert!(span_refused(&ed));
    assert_eq!(minibuf_input(&ed), "greet \"my no");
}

#[test]
fn a_span_past_the_line_end_is_refused() {
    let ed = greet_with_a_spanning_source("delegated", "#:span (hash 'start-char 6 'end-char 13)");
    assert!(span_refused(&ed));
    assert_eq!(minibuf_input(&ed), "greet \"my no");
}

/// A late answer has no session to check its source against, so a span
/// it names is dropped with it, whatever the source's `#:match`.
#[test]
fn a_span_in_an_answer_after_the_line_closed_is_dropped_unchecked() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define saved #f)
           (register-completion-source! "words"
             (lambda (id input cursor) (set! saved id))
             #:target 'minibuf #:match 'string)
           (define-typed-command! "greet" "" (lambda (bid arg) (log! 'info arg)) #:complete "words")
           (define-command! "late" "" (lambda ()
             (log! 'info (if (completion-emit! saved (list "x")
                                                #:span (hash 'start-char 6 'end-char 6))
                             "applied"
                             "dropped"))))"#,
    );
    ed.handle_key(key(':'));
    type_chars(&mut ed, "greet ");
    ed.handle_key(key_tab());
    ed.settle();
    ed.handle_key(key_esc());
    assert!(
        ed.state.input.minibuf().is_none(),
        "sanity: the line closed"
    );
    ed.execute_keymap_command("late".into(), Count::UNTYPED, false);
    assert_eq!(status(&ed), "dropped");
    assert!(!span_refused(&ed));
}

// ── What the first Tab inserts ───────────────────────────────────────────────

#[test]
fn the_common_prefix_is_taken_after_an_exact_match_drops_out() {
    let ed = greet_tab("foo", "delegated", r#"(list "foo" "foobar" "foobaz")"#, "");
    assert_eq!(minibuf_input(&ed), "greet fooba");
}

/// The insert texts share `'dir/a b`, which does not start with the typed
/// `dir/a`, so only the source can say it extends the word.
#[test]
fn a_delegated_source_names_what_the_first_tab_inserts() {
    let ed = greet_tab(
        "dir/a",
        "delegated",
        r#"(list "'dir/a b1'" "'dir/a b2'")"#,
        r#"#:prefix "'dir/a b""#,
    );
    assert_eq!(minibuf_input(&ed), "greet 'dir/a b");
    assert_eq!(
        ed.state.input.minibuf_completion().map(|s| s.len()),
        Some(2)
    );
}

/// Whether a `completion-emit!` error naming `#:prefix` was reported.
fn prefix_refused(ed: &Editor) -> bool {
    ed.state
        .message_log
        .entries()
        .any(|e| e.severity == Severity::Error && e.text.contains("#:prefix"))
}

#[test]
fn a_prefix_from_a_source_the_editor_matches_for_is_refused() {
    let ed = greet_tab("ab", "string", r#"(list "abc" "abd")"#, r#"#:prefix "abx""#);
    assert!(prefix_refused(&ed));
    assert_eq!(minibuf_input(&ed), "greet ab");
}

#[test]
fn an_empty_prefix_is_refused() {
    let ed = greet_tab("ab", "delegated", r#"(list "abc" "abd")"#, r#"#:prefix """#);
    assert!(prefix_refused(&ed));
    assert_eq!(minibuf_input(&ed), "greet ab");
}

#[test]
fn a_prefix_that_is_not_a_string_is_refused() {
    let ed = greet_tab("ab", "delegated", r#"(list "abc" "abd")"#, "#:prefix 3");
    assert!(prefix_refused(&ed));
    assert_eq!(minibuf_input(&ed), "greet ab");
}

/// The insert texts share `my\`, which ends inside an escape.
#[test]
fn a_false_prefix_proposes_nothing() {
    let ed = greet_tab(
        "my",
        "delegated",
        r#"(list "my\\ file" "my\\(1\\)")"#,
        "#:prefix #f",
    );
    assert_eq!(minibuf_input(&ed), "greet my");
    assert_eq!(
        ed.state.input.minibuf_completion().map(|s| s.len()),
        Some(2)
    );
}

/// The span runs past the cursor to the end of `gixyz`, and the source's
/// prefix was read only up to the cursor.
#[test]
fn a_named_prefix_waits_for_the_cursor_at_the_span_end() {
    let mut ed = greet_line(
        "gixyz",
        "delegated",
        r#"(list "git" "gitk")"#,
        r#"#:span (hash 'start-char 6 'end-char 11) #:prefix "git""#,
    );
    for _ in 0..3 {
        ed.handle_key(key_left());
    }
    ed.handle_key(key_tab());
    ed.settle();
    assert_eq!(minibuf_input(&ed), "greet gixyz");
    assert_eq!(
        ed.state.input.minibuf_completion().map(|s| s.len()),
        Some(2)
    );
}

// ── Buffer-name completion ───────────────────────────────────────────────────

#[test]
fn tab_on_buffer_arg_completes_buffer_names() {
    let dir = tempfile::tempdir().unwrap();
    let path_a = dir.path().join("alpha-notes.rs");
    let path_b = dir.path().join("alpha-utils.rs");
    std::fs::write(&path_a, "a\n").unwrap();
    std::fs::write(&path_b, "b\n").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    for path in [path_a, path_b] {
        let mut buf = Buffer::at_start(BufferText::from("a\n"));
        buf.set_file(crate::editor::buffer::BufferFile::at(path));
        ed.open_buffer(buf);
    }

    command_line(&mut ed, "b alpha");
    ed.handle_key(key_tab());
    let names = minibuf_candidates(&ed);
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
/// still being typed. `scan_command_name` (shared with
/// `parse_typed_command`, what `run_command_line` itself uses) ends a `b`
/// followed only by digits at `b`, matching what Enter would actually run.
#[test]
fn tab_mid_alias_with_no_space_completes_the_declared_arg_not_the_command_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("1-notes.rs");
    std::fs::write(&path, "a\n").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    let mut buf = Buffer::at_start(BufferText::from("a\n"));
    buf.set_file(crate::editor::buffer::BufferFile::at(path));
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
    let names = minibuf_candidates(&ed);
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
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = first_candidate_editor("-[h]>ello\n");
    command_line(&mut ed, "w");
    ed.handle_key(key_tab());
    assert!(ed.state.input.minibuf_completion().is_some(), "sanity");

    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "check" "" (lambda () (completion-accept! 0)))"#,
    );
    ed.execute_keymap_command("check".into(), Count::UNTYPED, false);
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
/// must not leave the popup stuck pending forever. Its call's own raise
/// ends its request, so `settle_minibuf_session` runs its ordinary
/// "nothing landed" policy (dismiss) instead of `is_pending()` staying
/// `true` with no source left that will ever answer.
#[test]
fn a_raising_minibuf_source_does_not_leave_the_popup_stuck_pending() {
    let tmp = tempfile::tempdir().unwrap();
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

/// A `Minibuf` source answering in parts (the async streaming case
/// `docs/COMPLETION-PICKER.md` documents): a wide first part, then later
/// (via a separate command here, standing in for a real async callback) a
/// narrower last one. Between the two, the user Tabs the selection off row
/// 0. `settle_minibuf_session`'s own re-rank must reset that selection, or
/// the splice on the last part reads a selection index the new, shorter
/// list no longer has.
#[test]
fn a_last_part_settles_against_a_reset_selection_not_a_stale_one() {
    let mut ed = greet_answered_by(
        "",
        &[
            (
                "answer-many",
                r#"(list "alice" "bob" "carol")"#,
                "#:more #t",
            ),
            ("answer-one", r#"(list "zzz")"#, ""),
        ],
    );
    ed.state.settings.command_completion = CommandCompletion::FirstCandidate;
    assert!(
        ed.state
            .input
            .minibuf_completion()
            .is_some_and(|s| s.is_pending()),
        "sanity: still pending"
    );

    call(&mut ed, "answer-many");
    assert_eq!(
        minibuf_candidates(&ed).len(),
        3,
        "sanity: first answer landed"
    );
    assert_eq!(minibuf_input(&ed), "greet ", "the eager policy waits");

    ed.handle_key(key_tab());
    ed.handle_key(key_tab());
    ed.handle_key(key_tab());
    assert_eq!(selected_row(&ed), 2, "sanity: selection moved off row 0");
    assert_eq!(minibuf_input(&ed), "greet carol");

    call(&mut ed, "answer-one");
    assert_eq!(
        minibuf_input(&ed),
        "greet zzz",
        "the last, shorter part must settle against a reset selection, \
         not silently fail to splice under the stale row-2 index"
    );
}

// ── An answer in parts ───────────────────────────────────────────────────────

/// `:greet <typed>` with Tab pressed, completed by a `'string` source that
/// answers later: each `(name, items, emit)` defines a command `name` that
/// answers the Tab with the Scheme list `items` and the extra
/// `completion-emit!` arguments `emit`.
fn greet_answered_by(typed: &str, answers: &[(&str, &str, &str)]) -> Editor {
    let commands: String = answers
        .iter()
        .map(|(name, items, emit)| {
            format!(
                r#"(define-command! "{name}" "" (lambda ()
                     (completion-emit! captured-id {items} {emit})))"#
            )
        })
        .collect();
    tab(greet_source(
        typed,
        &format!("(define captured-id #f) {commands}"),
        "(set! captured-id id)",
        "#:match 'string",
    ))
}

#[test]
fn an_answer_with_more_to_come_shows_its_rows_and_defers_the_eager_policy() {
    let mut ed = greet_answered_by(
        "b",
        &[
            ("part", r#"(list "bobby" "bobcat")"#, "#:more #t"),
            ("last", r#"(list "bobby" "bobcat" "bobsled")"#, ""),
        ],
    );
    call(&mut ed, "part");
    assert_eq!(minibuf_candidates(&ed), ["bobby", "bobcat"]);
    assert_eq!(minibuf_input(&ed), "greet b");
    call(&mut ed, "last");
    assert_eq!(minibuf_candidates(&ed).len(), 3);
    assert_eq!(minibuf_input(&ed), "greet bob");
}

#[test]
fn a_single_match_streamed_in_parts_is_applied_only_when_the_answer_ends() {
    let mut ed = greet_answered_by(
        "b",
        &[
            ("part", r#"(list "bobby")"#, "#:more #t"),
            ("last", r#"(list "bobby")"#, ""),
        ],
    );
    call(&mut ed, "part");
    assert_eq!(minibuf_candidates(&ed), ["bobby"]);
    assert_eq!(minibuf_input(&ed), "greet b");
    call(&mut ed, "last");
    assert_eq!(minibuf_input(&ed), "greet bobby");
    assert!(ed.state.input.minibuf_completion().is_none());
}

#[test]
fn a_streamed_answer_that_ends_empty_dismisses() {
    let mut ed = greet_answered_by("b", &[("part", "'()", "#:more #t"), ("last", "'()", "")]);
    call(&mut ed, "part");
    assert!(ed.state.input.minibuf_completion().is_some());
    call(&mut ed, "last");
    assert!(ed.state.input.minibuf_completion().is_none());
    assert_eq!(minibuf_input(&ed), "greet b");
}

/// `:greet b` with Tab pressed, completed by a `'minibuf` source whose
/// call runs `body` with its request `id` bound, and whose `#:cancel` logs
/// `cancelled`.
fn greet_with_cancel(body: &str) -> Editor {
    tab(greet_source(
        "b",
        "",
        body,
        r#"#:match 'string #:cancel (lambda (id) (log! 'info "cancelled"))"#,
    ))
}

#[test]
fn a_source_that_raises_after_a_part_ends_its_answer_there() {
    let ed = greet_with_cancel(r#"(completion-emit! id (list "bobby") #:more #t) (error "boom")"#);
    assert!(has_error(&ed, "boom"));
    assert_eq!(status(&ed), "cancelled");
    assert_eq!(minibuf_input(&ed), "greet bobby");
    assert!(ed.state.input.minibuf_completion().is_none());
}

#[test]
fn an_unrelated_error_leaves_a_request_waiting() {
    let mut ed = tab(greet_source(
        "b",
        r#"(define captured-id #f)
           (define-command! "late" "" (lambda () (completion-emit! captured-id (list "bob"))))"#,
        r#"(set! captured-id id) (after! 0 (lambda () (error "boom")))"#,
        r#"#:match 'string #:cancel (lambda (id) (log! 'info "cancelled"))"#,
    ));
    ed.drain_async_sources();
    ed.settle();
    assert!(has_error(&ed, "boom"));
    assert!(
        ed.state
            .input
            .minibuf_completion()
            .is_some_and(|s| s.is_pending())
    );
    assert_ne!(status(&ed), "cancelled");
    call(&mut ed, "late");
    assert_eq!(minibuf_input(&ed), "greet bob");
}

#[test]
fn a_source_call_the_drain_cap_drops_ends_its_request() {
    let mut ed = greet_source(
        "b",
        "",
        r#"(completion-emit! id (list "bob" "bobby"))"#,
        "#:match 'string",
    );
    ed.handle_key(key_tab());
    // Past `MAX_EVENT_DRAIN` (1000, `event/pump.rs`), so the whole batch,
    // the source's call included, is dropped unrun.
    for _ in 0..1100 {
        ed.state
            .config
            .pending_work
            .push_back(crate::editor::event::PendingWork::Event(
                crate::editor::event::EditorEvent::OnFocusGained,
            ));
    }
    assert!(!ed.drain_pending_work());
    assert!(ed.state.input.minibuf_completion().is_none());
}

#[test]
fn esc_while_an_answer_streams_cancels_its_request() {
    let mut ed = greet_with_cancel(r#"(completion-emit! id (list "bob" "bobby") #:more #t)"#);
    assert_eq!(minibuf_candidates(&ed), ["bob", "bobby"]);
    ed.handle_key(key_esc());
    ed.settle();
    assert_eq!(status(&ed), "cancelled");
}

#[test]
fn a_final_answer_is_never_cancelled() {
    let mut ed = greet_with_cancel(r#"(completion-emit! id (list "bob" "bobby"))"#);
    assert_eq!(minibuf_candidates(&ed), ["bob", "bobby"]);
    ed.handle_key(key_esc());
    ed.settle();
    assert_eq!(status(&ed), "");
}

#[test]
fn a_dropped_call_is_cancelled() {
    let ed = greet_with_cancel(r#"(error "boom")"#);
    assert!(has_error(&ed, "boom"));
    assert!(ed.state.input.minibuf_completion().is_none());
    assert_eq!(status(&ed), "cancelled");
}

#[test]
fn tab_while_an_answer_streams_picks_a_row() {
    let mut ed = greet_answered_by("", &[("part", r#"(list "alice" "bob")"#, "#:more #t")]);
    call(&mut ed, "part");
    assert_eq!(picked_row(&ed), None);
    ed.handle_key(key_tab());
    assert_eq!(picked_row(&ed), Some(0));
    assert_eq!(minibuf_input(&ed), "greet alice");
}

#[test]
fn a_streamed_update_keeps_the_picked_candidate_at_its_new_row() {
    let mut ed = greet_answered_by(
        "",
        &[
            ("part", r#"(list "bob" "carol")"#, "#:more #t"),
            ("more", r#"(list "alice" "bob" "carol")"#, "#:more #t"),
        ],
    );
    call(&mut ed, "part");
    ed.handle_key(key_tab());
    ed.handle_key(key_tab());
    assert_eq!(picked_row(&ed), Some(1));
    call(&mut ed, "more");
    assert_eq!(picked_row(&ed), Some(2));
    assert_eq!(minibuf_candidates(&ed), ["alice", "bob", "carol"]);
    assert_eq!(minibuf_input(&ed), "greet carol");
}

#[test]
fn the_end_of_a_streamed_answer_leaves_a_pick_alone() {
    let mut ed = greet_answered_by(
        "",
        &[
            ("part", r#"(list "bob" "bobcat")"#, "#:more #t"),
            ("last", r#"(list "bob" "bobcat")"#, ""),
        ],
    );
    call(&mut ed, "part");
    ed.handle_key(key_tab());
    call(&mut ed, "last");
    assert_eq!(picked_row(&ed), Some(0));
    assert!(ed.state.input.minibuf_completion().is_some());
    assert_eq!(minibuf_input(&ed), "greet bob");
}

#[test]
fn a_picked_candidate_missing_from_a_streamed_update_drops_the_pick() {
    let mut ed = greet_answered_by(
        "",
        &[
            ("part", r#"(list "bob" "carol")"#, "#:more #t"),
            ("more", r#"(list "alice" "bob")"#, "#:more #t"),
        ],
    );
    call(&mut ed, "part");
    ed.handle_key(key_tab());
    ed.handle_key(key_tab());
    call(&mut ed, "more");
    assert_eq!(picked_row(&ed), None);
    assert_eq!(minibuf_input(&ed), "greet carol");
}
