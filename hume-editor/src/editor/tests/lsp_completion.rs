// B8 (docs/lsp/step-2.md) — completion orchestration: completion-begin!,
// completion-update-filter!, completion-top, completion-accept!,
// completion-dismiss!.

use std::path::Path;

use super::*;
use crate::editor::scripting_setup::make_init_host;
use hume_scripting::ScriptingHost;

fn eval_with_real_host(ed: &mut Editor, host: &mut ScriptingHost, source: &str, tmp: &Path) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    let mut ih = make_init_host(&mut ed.state, &mut ed.view);
    host.eval_init(&init_path, 10_000, &mut ih, Default::default())
        .expect("eval_init");
}

fn run(ed: &mut Editor, tmp: &Path, source: &str) {
    let mut host = ScriptingHost::new();
    eval_with_real_host(ed, &mut host, source, tmp);
    ed.scripting = Some(host);
}

#[test]
fn begin_then_top_returns_items_ranked_by_sort_text_with_no_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "second" "sortText" "b")
                     (hash "label" "first" "sortText" "a")
                     (hash "label" "third" "sortText" "c")))
             (log! 'info (string-join (map (lambda (h) (hash-ref h "label")) (completion-top 10)) ","))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "first,second,third",
        "with no filter, items must rank by sortText ascending"
    );
}

#[test]
fn update_filter_narrows_and_prefix_beats_infix() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "orange")                  ; "rn" matches later (r@1, n@3)
                     ; "rn" matches "random" as a *subsequence* (r@0, n@2) but not
                     ; as a literal prefix (haystack[1] is 'a', not 'n'). sortText
                     ; "a" would win alphabetically if prefix-match didn't matter.
                     (hash "label" "random" "sortText" "a")
                     ; "rn" *is* a literal prefix of "rnorm". sortText "z" would
                     ; lose alphabetically — the only way it can still rank first
                     ; is if prefix-match genuinely outranks the tie-break rule.
                     (hash "label" "rnorm" "sortText" "z")
                     (hash "label" "grape")))                  ; no "r" at all — dropped
             (completion-update-filter! "rn")
             (log! 'info (string-join (map (lambda (h) (hash-ref h "label")) (completion-top 10)) ","))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "rnorm,random,orange",
        "a literal prefix match must rank above a same-position subsequence match \
         regardless of sortText, and both must rank above a later-position match"
    );
}

#[test]
fn accept_with_no_text_edit_inserts_insert_text_at_the_anchor_span() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "foobar" "insertText" "hello")))
             (completion-update-filter! "fo")
             (completion-accept! 0)))"#,
    );
    type_cmd(&mut ed, ":go");
    // anchor = 0 (cursor was on 'a' at begin time), filter "fo" = 2 chars,
    // so the fallback replaces chars [0, 2) ("ab") with "hello".
    assert_eq!(ed.doc().text().to_string(), "hellocdef\n");
}

#[test]
fn accept_with_a_text_edit_applies_the_servers_range_exactly() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "x" "insertText" "ignored-fallback"
                           "textEdit" (hash "range" (hash "start" (hash "line" 0 "character" 1)
                                                        "end" (hash "line" 0 "character" 4))
                                       "newText" "XYZ"))))
             (completion-accept! 0)))"#,
    );
    type_cmd(&mut ed, ":go");
    // textEdit replaces chars [1, 4) ("bcd") with "XYZ" — the server's
    // explicit range, not the anchor..cursor guess (anchor=0, no filter
    // typed, which would have been a zero-width insert at 0).
    assert_eq!(ed.doc().text().to_string(), "aXYZef\n");
}

#[test]
fn accept_is_one_undo_step() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "foobar" "insertText" "hello")))
             (completion-update-filter! "fo")
             (completion-accept! 0)))"#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(ed.doc().text().to_string(), "hellocdef\n");

    ed.handle_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        "abcdef\n",
        "a single 'u' must fully restore the pre-accept text"
    );
}

#[test]
fn dismiss_clears_the_session_so_a_later_accept_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer) (list (hash "label" "x" "insertText" "z")))
             (completion-dismiss!)
             (completion-accept! 0)))"#,
    );
    let before = ed.doc().text().to_string();
    type_cmd(&mut ed, ":go");
    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "dismiss! must clear the session — the later accept! must not apply anything"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no active completion session"),
        "expected a no-active-session error, got {msg:?}"
    );
}

#[test]
fn a_buffer_edit_that_bypasses_update_filter_invalidates_the_session() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "begin" "" (lambda ()
             (completion-begin! (current-buffer) (list (hash "label" "x" "insertText" "z")))))
           (define-command! "finish" "" (lambda ()
             (completion-accept! 0)))"#,
    );
    type_cmd(&mut ed, ":begin");

    // An edit that never goes through completion-update-filter! — a
    // Normal-mode edit (select-line, delete), not Insert-mode typing: U7
    // wires Insert-mode typing to call `completion-update-filter!`
    // automatically whenever a session is open, so raw typing is no longer
    // a valid example of a bypassing edit. Normal mode never touches
    // either of U7's Insert-mode hooks.
    ed.handle_key(key('x'));
    ed.handle_key(key('d'));

    let before_accept = ed.doc().text().to_string();
    type_cmd(&mut ed, ":finish");
    assert_eq!(
        ed.doc().text().to_string(),
        before_accept,
        "accept! must reject — the buffer changed without the session's knowledge"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("error") || msg.to_lowercase().contains("changed"),
        "expected an error message, got {msg:?}"
    );
}

// ── B10c: on-completion-accept / on-completion-refilter ────────────────────

#[test]
fn accept_fires_on_completion_accept_with_the_raw_item_after_the_edit() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "foobar" "insertText" "hello" "extra" "e1")))
             (completion-update-filter! "fo")
             (completion-accept! 0)))
           (register-hook! 'on-completion-accept (lambda (bid item)
             (log! 'info (hash-ref item "extra"))))"#,
    );
    type_cmd(&mut ed, ":go");
    ed.drain_hooks();
    assert_eq!(
        ed.doc().text().to_string(),
        "hellocdef\n",
        "sanity: the main edit must apply before the hook fires"
    );
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "e1",
        "on-completion-accept must receive the accepted item's raw JSON, including \
         fields (\"extra\") that StoredCompletionItem doesn't otherwise parse"
    );
}

#[test]
fn accept_with_no_hook_registered_still_applies_the_edit() {
    // Fail oracle for the hook wiring: if `push` onto `pending_hooks` panicked
    // or the accept path never returned `Ok`, this would fail even with zero
    // handlers registered.
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "foobar" "insertText" "hello")))
             (completion-update-filter! "fo")
             (completion-accept! 0)))"#,
    );
    type_cmd(&mut ed, ":go");
    ed.drain_hooks();
    assert_eq!(ed.doc().text().to_string(), "hellocdef\n");
}

#[test]
fn refilter_fires_on_completion_refilter_only_when_incomplete() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "foobar" "insertText" "hello"))
               #:incomplete #t)))
           (register-hook! 'on-completion-refilter (lambda (bid text)
             (log! 'info (string-append "refilter:" text))))"#,
    );
    type_cmd(&mut ed, ":go");
    ed.feed_key(key('i'));
    ed.feed_key(key('f'));
    ed.drain_hooks();
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "refilter:f",
        "on-completion-refilter must fire with the new filter text while the session's \
         isIncomplete flag is set"
    );
}

#[test]
fn refilter_does_not_fire_when_the_session_is_complete() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "foobar" "insertText" "hello")))))
           (register-hook! 'on-completion-refilter (lambda (bid text)
             (log! 'info "should-not-fire")))"#,
    );
    type_cmd(&mut ed, ":go");
    ed.feed_key(key('i'));
    ed.feed_key(key('f'));
    ed.drain_hooks();
    assert_ne!(
        ed.state.status_msg.clone().unwrap_or_default(),
        "should-not-fire",
        "on-completion-refilter must not fire for a complete (non-isIncomplete) session — \
         it's a bounded window, not an unconditional per-keystroke hook"
    );
}

/// Guardrail regression test (P8): a 1k-item scripted session (begin ->
/// filter -> top -> accept) under a loose release-mode bound. `#[ignore]`
/// by default — run explicitly with `cargo test --release -- --ignored`.
#[test]
#[ignore]
fn scripted_1k_item_session_stays_under_the_p8_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bcdef\n");
    let items: String = (0..1000)
        .map(|i| format!(r#"(hash "label" "item{i}" "sortText" "{i:04}" "insertText" "item{i}")"#))
        .collect::<Vec<_>>()
        .join(" ");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (completion-begin! (current-buffer) (list {items}))
                 (completion-update-filter! "item5")
                 (completion-top 64)
                 (completion-accept! 0)))"#
        ),
    );
    let start = std::time::Instant::now();
    type_cmd(&mut ed, ":go");
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 5,
        "1k-item begin->filter->top->accept took {elapsed:?}, over the 5ms guardrail budget"
    );
}
