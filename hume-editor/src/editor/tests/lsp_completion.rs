// Completion orchestration: completion-begin!,
// completion-update-filter!, completion-top, completion-accept!,
// completion-dismiss!.

use std::path::Path;

use super::*;
use hume_scripting::ScriptingHost;

fn run(ed: &mut Editor, tmp: &Path, source: &str) {
    let mut host = ScriptingHost::new();
    eval_with_real_host(ed, &mut host, source, tmp);
    ed.scripting = Some(host);
}

#[test]
fn begin_then_top_returns_items_ranked_by_sort_text_with_no_filter() {
    let tmp = safe_tempdir();
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
    let tmp = safe_tempdir();
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
    let tmp = safe_tempdir();
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
fn accept_with_no_text_edit_replaces_the_prefix_typed_before_completion_began() {
    let tmp = safe_tempdir();
    // Cursor (anchor at begin time) sits right after an already-typed "fo"
    // prefix — completion invoked manually after typing, not from an empty
    // token. The fallback must replace that whole token, not just
    // [anchor, anchor) (a zero-width insert that would duplicate "fo" ahead
    // of the inserted "foobar").
    let mut ed = editor_from("fo-[ ]>bar\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "foobar" "insertText" "foobar")))
             (completion-accept! 0)))"#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(ed.doc().text().to_string(), "foobar bar\n");
}

#[test]
fn accept_with_a_text_edit_extends_the_range_to_cover_chars_typed_after_begin() {
    let tmp = safe_tempdir();
    // Buffer already holds "for" — standing in for "the user typed one more
    // char ('r') after the completion menu opened, narrowing the filter
    // further." The server's textEdit range (0,0)-(0,2) was computed
    // against "fo", *before* that extra keystroke.
    let mut ed = editor_from("-[f]>or\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "format!" "insertText" "ignored-fallback"
                           "textEdit" (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                        "end" (hash "line" 0 "character" 2))
                                       "newText" "format!"))))
             (completion-update-filter! "for")
             (completion-accept! 0)))"#,
    );
    type_cmd(&mut ed, ":go");
    // Without the fix, only [0, 2) ("fo") is replaced, leaving the "r"
    // typed after begin sitting untouched next to the insert: "format!r".
    assert_eq!(ed.doc().text().to_string(), "format!\n");
}

#[test]
fn accept_with_an_off_spec_text_edit_range_not_containing_the_cursor_clamps_instead_of_panicking() {
    let tmp = safe_tempdir();
    // LSP spec (completion.rs `text_edit` doc, Note 1): a conforming
    // server's completion range always contains the request position — the
    // as-if-typed model `accept` now uses (every edit expressed as a char
    // count behind/ahead of the live cursor) depends on that guarantee. This
    // range starts at char 1 while the cursor sits at char 0, deliberately
    // off-spec and unreachable through real typing.
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
    // A delete region that doesn't reach the cursor clamps to start there
    // instead of panicking or silently doing nothing — no crash, no data
    // loss beyond what the server's own (off-spec) range already implied.
    assert_eq!(ed.doc().text().to_string(), "XYZef\n");
}

#[test]
fn accept_is_one_undo_step() {
    let tmp = safe_tempdir();
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
    let tmp = safe_tempdir();
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
    let tmp = safe_tempdir();
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
    // Normal-mode edit (select-line, delete), not Insert-mode typing:
    // Insert-mode typing is wired to call `completion-update-filter!`
    // automatically whenever a session is open, so raw typing is no longer
    // a valid example of a bypassing edit. Normal mode never touches
    // either of the Insert-mode hooks.
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

// ── Empty items: no session, not an invisible menu ────────────────────

#[test]
fn begin_with_empty_items_creates_no_session_and_reports_info() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer) (list))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert!(
        ed.lsp.completion.is_none(),
        "an empty items response must not open a session"
    );
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "no completions",
        "an empty items response must surface why no menu opened, not silently \
         leave an invisible session trapping Esc"
    );
}

#[test]
fn begin_with_empty_items_clears_an_already_open_session() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "open" "" (lambda ()
             (completion-begin! (current-buffer) (list (hash "label" "x" "insertText" "z")))))
           (define-command! "reopen-empty" "" (lambda ()
             (completion-begin! (current-buffer) (list))))"#,
    );
    type_cmd(&mut ed, ":open");
    assert!(ed.lsp.completion.is_some(), "sanity: session opened");

    type_cmd(&mut ed, ":reopen-empty");
    assert!(
        ed.lsp.completion.is_none(),
        "an isIncomplete re-request that comes back empty must close the open \
         session, not leave the previous items live"
    );
}

// ── on-completion-accept / on-completion-refilter ────────────────────

#[test]
fn accept_fires_on_completion_accept_with_the_raw_item_after_the_edit() {
    let tmp = safe_tempdir();
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
    let tmp = safe_tempdir();
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
    let tmp = safe_tempdir();
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
    let tmp = safe_tempdir();
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

/// Guardrail regression test: a 1k-item scripted session (begin ->
/// filter -> top -> accept) under a loose release-mode bound. `#[ignore]`
/// by default — run explicitly with `cargo test --release -- --ignored`.
#[test]
#[ignore]
fn scripted_1k_item_session_stays_under_the_p8_budget() {
    let tmp = safe_tempdir();
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

/// `completion-begin!` for a buffer that isn't shown in the focused pane —
/// the normal shape of an async LSP completion response landing after the
/// user switched panes — must be a benign no-op (Trace log, no session
/// created), not an error: an error here would abort the whole
/// `drain_pending_steel_calls` batch and drop every other queued
/// callback/timer for the frame.
#[test]
fn completion_begin_for_a_buffer_not_shown_in_the_focused_pane_is_a_benign_no_op() {
    use crate::editor::commands::open_pane;
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::CompletionHost;

    let dir = safe_tempdir();
    let file_b = dir.path().join("b.txt");
    std::fs::write(&file_b, "hello\n").unwrap();

    let mut ed = editor_from("-[a]>bcdef\n");
    let pid_a = ed.state.focused_pane_id;
    let bid_a = ed.focused_buffer_id();
    let pid_b = open_pane(&mut ed.state, &mut ed.view, bid_a);

    // Open a second file only in pane B, then focus back to pane A — bid_b
    // is now only ever recorded in pane B's per-pane state.
    ed.switch_focused_pane(pid_b);
    ed.execute_typed("e", Some(file_b.to_str().unwrap()))
        .unwrap();
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "must be genuinely different buffers");
    ed.switch_focused_pane(pid_a);

    let mut impl_host = EditorHostImpl {
        state: &mut ed.state,
        view: &mut ed.view,
        lsp: None,
        timers: None,
        terminal: None,
    };
    let result = impl_host.completion_begin(bid_b, vec![serde_json::json!({"label": "x"})], false);
    assert!(
        result.is_ok(),
        "unfocused-pane buffer must be a benign no-op, not an error: {result:?}"
    );

    assert!(
        ed.lsp.completion.is_none(),
        "no session must be created for a buffer not shown in the focused pane"
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Trace && e.text.contains("not shown in focused pane")),
        "must log a Trace entry explaining the ignored begin, got: {:?}",
        ed.state.message_log.entries().collect::<Vec<_>>()
    );
}

/// A malformed item (missing the spec-required `label`) must not take down
/// the whole batch — the well-formed item next to it still survives.
#[test]
fn malformed_item_is_skipped_with_a_trace_and_the_rest_survive() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "good") (hash "kind" 1)))
             (log! 'info (string-join (map (lambda (h) (hash-ref h "label")) (completion-top 10)) ","))))"#,
    );
    type_cmd(&mut ed, ":go");

    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "good",
        "the malformed (label-less) item must not appear, but the good one must"
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Trace && e.text.contains("skipped malformed item")),
        "must log a Trace entry for the skipped item, got: {:?}",
        ed.state.message_log.entries().collect::<Vec<_>>()
    );
}

/// Every item malformed must behave exactly like an empty response — no
/// session, "no completions" reported — not a silently-empty open session.
#[test]
fn all_items_malformed_behaves_like_an_empty_response() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer) (list (hash "kind" 1) (hash "kind" 2)))))"#,
    );
    type_cmd(&mut ed, ":go");

    assert!(
        ed.lsp.completion.is_none(),
        "an all-malformed items response must not open a session"
    );
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "no completions",
        "must be reported exactly like an empty items response"
    );
}

/// `CompletionTextEdit::InsertAndReplace` must apply its narrower `insert`
/// range, not the wider `replace` range — pins the union arm the parser
/// needs to handle explicitly.
#[test]
fn insert_replace_text_edit_applies_the_narrower_insert_range() {
    let tmp = safe_tempdir();
    // Cursor at char 1 ('b') — inside the `insert` range below, per the LSP
    // spec's containment guarantee (`completion.rs`'s `text_edit` doc);
    // unlike the two off-spec regression tests, this one isn't testing
    // range-vs-cursor divergence, so the fixture stays spec-conforming.
    let mut ed = editor_from("a-[b]>cdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "x"
                           "textEdit" (hash "insert" (hash "start" (hash "line" 0 "character" 1)
                                                            "end" (hash "line" 0 "character" 3))
                                            "replace" (hash "start" (hash "line" 0 "character" 1)
                                                             "end" (hash "line" 0 "character" 6))
                                            "newText" "XYZ"))))
             (completion-accept! 0)))"#,
    );
    type_cmd(&mut ed, ":go");
    // insert = [1,3) ("bc"), replace = [1,6) ("bcdef") — using replace would
    // leave "aXYZ\n"; the narrower insert range must leave "def" behind.
    assert_eq!(ed.doc().text().to_string(), "aXYZdef\n");
}
