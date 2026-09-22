//! Sources: registration, what a source is handed, the token rules,
//! answers landing (or being dropped as stale), re-invocation, and how
//! several sources rank together.

use super::*;
use crate::editor::buffer::Buffer;
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_scripting::ScriptingHost;

// ── Invocation ────────────────────────────────────────────────────────────

#[test]
fn ctrl_space_invokes_a_registered_source_and_shows_its_answer() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "second" "sortText" "b")
                 (hash "label" "first" "sortText" "a")
                 (hash "label" "third" "sortText" "c"))"#,
    );
    assert_eq!(
        labels(&ed),
        vec!["first", "second", "third"],
        "with an empty token, items rank by sortText ascending"
    );
}

/// The proc receives the invocation id, the buffer, and the seeded prefix
/// — `'word` hands over the identifier run before the cursor.
#[test]
fn a_word_source_is_handed_the_word_before_the_cursor_as_its_prefix() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("ki-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(register-completion-source! "test"
             (lambda (id bid prefix)
               (log! 'info (string-append "prefix:" prefix "|bid-ok:"
                             (if (equal? bid (current-buffer)) "yes" "no")))
               (completion-emit! id (list (hash "label" "kitty_support"))))
             #:target 'buffer #:token 'word)"#,
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert_eq!(status(&ed), "prefix:ki|bid-ok:yes");
}

#[test]
fn a_cursor_source_is_handed_an_empty_prefix_even_mid_word() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("ki-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(register-completion-source! "test"
             (lambda (id bid prefix)
               (log! 'info (string-append "prefix:[" prefix "]"))
               (completion-emit! id (list (hash "label" "kitty_support"))))
             #:target 'buffer #:token 'cursor)"#,
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert_eq!(status(&ed), "prefix:[]");
}

#[test]
fn ctrl_space_outside_insert_mode_only_reports() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        &completion_source("test", &completion_labels(&["x"]), ""),
    );
    ed.execute_keymap_command("completion-trigger".into(), None, false);
    ed.settle();
    assert!(ed.state.input.completion().is_none());
    assert_eq!(status(&ed), "completion-trigger: only in Insert mode");
}

#[test]
fn ctrl_space_with_no_buffer_source_registered_reports() {
    let mut ed = editor_from("-[a]>bcdef\n");
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert!(ed.state.input.completion().is_none());
    assert_eq!(status(&ed), "no completion sources registered");
}

// ── Token rules: what the filter is seeded from, what accept replaces ─────

/// A prefix already typed before the trigger is filtered on immediately —
/// every candidate does not survive until some later keystroke narrows it.
#[test]
fn a_word_token_seeds_the_filter_from_the_word_before_the_cursor() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("ki-[x]>\n");
    insert_with_source(
        &mut ed,
        tmp.path(),
        &completion_labels(&["kitty_support", "AsMut", "f32"]),
    );
    assert_eq!(
        labels(&ed),
        vec!["kitty_support"],
        "only the \"ki\" subsequence match survives the seeded filter"
    );
}

/// `'cursor` is a first-class choice, not a fallback: no filter at all,
/// even mid-word.
#[test]
fn a_cursor_token_seeds_no_filter_even_mid_word() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("ab-[c]>def\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(register-completion-source! "test"
             (lambda (id bid prefix) (completion-emit! id (list (hash "label" "kitty_support"))))
             #:target 'buffer #:token 'cursor)"#,
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert_eq!(
        labels(&ed),
        vec!["kitty_support"],
        "\"kitty_support\" shares no chars with \"ab\" — a word token would filter it out"
    );
}

/// `'cursor` is also a declaration that accept replaces nothing before the
/// cursor: the "fo" already in the buffer is left in place, duplicated ahead
/// of the inserted text — the documented consequence of choosing it.
#[test]
fn a_cursor_token_replaces_nothing_before_the_cursor_on_accept() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("fo-[ ]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(register-completion-source! "test"
             (lambda (id bid prefix) (completion-emit! id (list (hash "label" "foobar" "insertText" "foobar"))))
             #:target 'buffer #:token 'cursor)"#,
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    ed.feed_key(key_enter());
    assert_eq!(ed.doc().text().to_string(), "fofoobar \n");
}

/// A `'custom` source names its own span, in the coordinates of the
/// snapshot it was handed — here the two chars before the cursor.
#[test]
fn a_custom_token_uses_the_emitted_span_for_filtering_and_accept() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("x fo-[ ]>bar\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(register-completion-source! "test"
             (lambda (id bid prefix)
               (completion-emit! id (list (hash "label" "foobar") (hash "label" "zzz"))
                                 #:span (cons 2 4)))
             #:target 'buffer #:token 'custom)"#,
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert_eq!(
        labels(&ed),
        vec!["foobar"],
        "filtered on the span's own \"fo\""
    );
    ed.feed_key(key_enter());
    assert_eq!(
        ed.doc().text().to_string(),
        "x foobar bar\n",
        "accept replaces exactly the emitted span"
    );
}

#[test]
fn a_custom_token_answer_without_a_span_is_an_error() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("fo-[ ]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(register-completion-source! "test"
             (lambda (id bid prefix) (completion-emit! id (list (hash "label" "foobar"))))
             #:target 'buffer #:token 'custom)"#,
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert!(
        ed.state.input.completion().is_none(),
        "an unusable answer is \"nothing from this source\" — the session closes"
    );
    assert!(
        status(&ed).contains("#:span is required"),
        "got {:?}",
        status(&ed)
    );
}

fn custom_span_source(span: &str) -> String {
    format!(
        r#"(register-completion-source! "test"
             (lambda (id bid prefix)
               (completion-emit! id (list (hash "label" "x")) #:span {span}))
             #:target 'buffer #:token 'custom)"#
    )
}

#[test]
fn a_custom_span_not_containing_the_cursor_is_an_error() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(&mut ed, tmp.path(), &custom_span_source("(cons 3 5)"));
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert!(ed.state.input.completion().is_none());
    assert!(
        status(&ed).contains("does not contain the cursor"),
        "got {:?}",
        status(&ed)
    );
}

#[test]
fn a_custom_span_out_of_range_is_an_error() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(&mut ed, tmp.path(), &custom_span_source("(cons 0 999)"));
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert!(ed.state.input.completion().is_none());
    assert!(
        status(&ed).contains("out of range"),
        "got {:?}",
        status(&ed)
    );
}

/// A completion token never spans a line.
#[test]
fn a_custom_span_across_lines_is_an_error() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("abc\n-[d]>ef\n");
    run(&mut ed, tmp.path(), &custom_span_source("(cons 0 5)"));
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert!(ed.state.input.completion().is_none());
    assert!(
        status(&ed).contains("more than one line"),
        "got {:?}",
        status(&ed)
    );
}

/// A mid-cluster span start — between a base character and its combining
/// mark — snaps outward to the cluster's own start, same as every other
/// buffer-position seam that admits untrusted input. Proven through
/// filtering: an unsnapped start seeds a 1-char filter (the orphaned mark
/// alone), never a *prefix* of the item's filter text — only the correctly
/// snapped 2-char "é" is.
#[test]
fn a_custom_span_starting_mid_cluster_snaps_to_the_cluster_start() {
    let tmp = safe_tempdir();
    // "cafe\u{0301}" — one cluster spans chars [3,5); 4 is inside it.
    let mut ed = editor_from("cafe\u{0301}-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(register-completion-source! "test"
                 (lambda (id bid prefix)
                   (completion-emit! id (list (hash "label" "efoo" "filterText" "e{}foo"))
                                     #:span (cons 4 5)))
                 #:target 'buffer #:token 'custom #:match 'string)"#,
            '\u{0301}'
        ),
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert_eq!(labels(&ed), vec!["efoo"]);
}

// ── Answers ───────────────────────────────────────────────────────────────

#[test]
fn an_empty_answer_closes_the_session_and_reports_no_completions() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    insert_with_source(&mut ed, tmp.path(), "(list)");
    assert!(ed.state.input.completion().is_none());
    assert_eq!(status(&ed), "no completions");
}

/// While a source is pending the session is open (invisibly, with no rows)
/// — the answer, not the trigger, decides whether there is anything to
/// show.
#[test]
fn the_session_is_open_but_empty_until_the_source_answers() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define pending-id #f)
           (register-completion-source! "test"
             (lambda (id bid prefix) (set! pending-id id))
             #:target 'buffer #:token 'word)
           (define-command! "answer" "" (lambda ()
             (completion-emit! pending-id (list (hash "label" "late")))))"#,
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    let session = ed.state.input.completion().expect("open while pending");
    assert!(session.is_pending());
    assert!(session.is_empty());
    ed.execute_keymap_command("answer".into(), None, false);
    assert_eq!(labels(&ed), vec!["late"]);
}

/// An answer for a superseded invocation — the source was called again
/// (a second trigger) before it answered the first — is dropped, never
/// merged into the newer call's slot.
#[test]
fn an_answer_to_a_superseded_invocation_is_dropped() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define calls '())
           (register-completion-source! "test"
             (lambda (id bid prefix) (set! calls (cons id calls)))
             #:target 'buffer #:token 'word)
           (define-command! "answer-first" "" (lambda ()
             (log! 'info (if (completion-emit! (cadr calls) (list (hash "label" "stale")))
                             "applied" "dropped"))))
           (define-command! "answer-second" "" (lambda ()
             (completion-emit! (car calls) (list (hash "label" "fresh")))))"#,
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    trigger(&mut ed);
    ed.execute_keymap_command("answer-first".into(), None, false);
    assert_eq!(status(&ed), "dropped");
    assert!(labels(&ed).is_empty(), "the stale answer must not land");
    ed.execute_keymap_command("answer-second".into(), None, false);
    assert_eq!(labels(&ed), vec!["fresh"]);
}

/// A stale answer changes nothing (`session.contribute` returns `Ok(false)`
/// before touching `filtered`), so the settle path it would otherwise run
/// must not reset the menu's selection either — only an answer that
/// actually re-ranks the list may do that.
#[test]
fn a_dropped_stale_answer_does_not_reset_the_selection() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define calls '())
           (register-completion-source! "test"
             (lambda (id bid prefix)
               (set! calls (cons id calls))
               (completion-emit! id (list (hash "label" "foo") (hash "label" "fox")) #:incomplete #t))
             #:target 'buffer #:token 'word)
           (define-command! "answer-stale" "" (lambda ()
             (log! 'info (if (completion-emit! (car (reverse calls)) (list (hash "label" "OLD")))
                             "applied" "dropped"))))"#,
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert_eq!(labels(&ed), vec!["foo", "fox"], "sanity");

    // Typing narrows the token and, since the last answer was incomplete,
    // reinvokes the source — the first invocation (`calls`'s oldest id) is
    // now stale. This re-rank is a real one, so it legitimately resets the
    // selection; the test only asserts on what happens *after*.
    ed.feed_key(key('f'));
    ed.settle();
    assert_eq!(
        labels(&ed),
        vec!["foo", "fox"],
        "sanity: still both, narrowed to \"f\""
    );

    // The user moves off row 0 on the settled, post-edit list.
    ed.feed_key(key_tab());
    assert_eq!(selected_row(&ed), 1, "sanity");

    // The stale first invocation's answer lands late.
    ed.execute_keymap_command("answer-stale".into(), None, false);
    assert_eq!(status(&ed), "dropped");
    assert_eq!(
        selected_row(&ed),
        1,
        "a dropped answer changed nothing, so the selection must survive"
    );
}

/// A second answer for the *same* still-latest invocation replaces the
/// first — a source may stream.
#[test]
fn a_repeated_answer_for_a_live_invocation_replaces_the_first() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define saved #f)
           (register-completion-source! "test"
             (lambda (id bid prefix)
               (set! saved id)
               (completion-emit! id (list (hash "label" "x") (hash "label" "y"))))
             #:target 'buffer #:token 'word)
           (define-command! "again" "" (lambda ()
             (completion-emit! saved (list (hash "label" "x") (hash "label" "z")))))"#,
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    ed.execute_keymap_command("again".into(), None, false);
    assert_eq!(labels(&ed), vec!["x", "z"], "replaced, not appended");
}

#[test]
fn an_answer_after_the_session_closed_is_dropped() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define saved #f)
           (register-completion-source! "test"
             (lambda (id bid prefix) (set! saved id))
             #:target 'buffer #:token 'word)
           (define-command! "late" "" (lambda ()
             (log! 'info (if (completion-emit! saved (list (hash "label" "x"))) "applied" "dropped"))))"#,
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    ed.feed_key(key_esc());
    assert_eq!(
        ed.state.mode(),
        Mode::Normal,
        "sanity: Insert ended, session gone"
    );
    ed.execute_keymap_command("late".into(), None, false);
    assert_eq!(status(&ed), "dropped");
    assert!(ed.state.input.completion().is_none());
}

/// A malformed item (missing the spec-required `label`) must not take down
/// the whole batch — the well-formed item next to it still survives.
#[test]
fn a_malformed_item_is_skipped_with_a_trace_and_the_rest_survive() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "good") (hash "kind" 1))"#,
    );
    assert_eq!(labels(&ed), vec!["good"]);
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Trace && e.text.contains("skipped malformed item")),
        "must log a Trace entry for the skipped item, got: {:?}",
        ed.state.message_log.entries().collect::<Vec<_>>()
    );
}

#[test]
fn an_all_malformed_answer_behaves_like_an_empty_one() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "kind" 1) (hash "kind" 2))"#,
    );
    assert!(ed.state.input.completion().is_none());
    assert_eq!(status(&ed), "no completions");
}

// ── Re-invocation ─────────────────────────────────────────────────────────

fn counting_source(incomplete: &str) -> String {
    format!(
        r#"(define calls 0)
           (register-completion-source! "test"
             (lambda (id bid prefix)
               (set! calls (+ calls 1))
               (completion-emit! id (list (hash "label" "foobar")) #:incomplete {incomplete}))
             #:target 'buffer #:token 'word)
           (define-command! "report" "" (lambda () (log! 'info (number->string calls))))"#
    )
}

/// A source whose latest answer was `isIncomplete` is called again on every
/// keystroke — the LSP `isIncomplete` flow, with no hook for the plugin to
/// subscribe to.
#[test]
fn an_incomplete_source_is_reinvoked_on_each_keystroke() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(&mut ed, tmp.path(), &counting_source("#t"));
    ed.feed_key(key('i'));
    trigger(&mut ed);
    ed.feed_key(key('f'));
    ed.settle();
    ed.feed_key(key('o'));
    ed.settle();
    ed.execute_keymap_command("report".into(), None, false);
    assert_eq!(status(&ed), "3", "one trigger call plus one per keystroke");
}

/// A complete answer is final — typing re-ranks locally, never re-asks.
#[test]
fn a_complete_source_is_not_reinvoked_on_typing() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(&mut ed, tmp.path(), &counting_source("#f"));
    ed.feed_key(key('i'));
    trigger(&mut ed);
    ed.feed_key(key('f'));
    ed.settle();
    ed.execute_keymap_command("report".into(), None, false);
    assert_eq!(status(&ed), "1");
    assert_eq!(
        labels(&ed),
        vec!["foobar"],
        "re-ranked locally against \"f\""
    );
}

/// The old answer stays ranked (against the new token text) until the
/// re-invocation's own answer lands — the menu never blinks empty.
#[test]
fn a_reinvoked_source_keeps_its_rows_until_the_new_answer_lands() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define calls 0)
           (register-completion-source! "test"
             (lambda (id bid prefix)
               (set! calls (+ calls 1))
               (when (= calls 1)
                 (completion-emit! id (list (hash "label" "foo") (hash "label" "bar")) #:incomplete #t)))
             #:target 'buffer #:token 'word)"#,
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    ed.feed_key(key('f'));
    ed.settle();
    let session = ed.state.input.completion().expect("still open");
    assert!(session.is_pending(), "the second call is in flight");
    assert_eq!(
        labels(&ed),
        vec!["foo"],
        "the first answer, narrowed to \"f\""
    );
}

/// A pending source whose call saw the document *before* an edit is
/// superseded by a fresh call against the document after it.
#[test]
fn a_pending_source_is_reinvoked_after_an_edit_and_its_old_call_goes_stale() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define ids '())
           (register-completion-source! "test"
             (lambda (id bid prefix) (set! ids (cons (cons id prefix) ids)))
             #:target 'buffer #:token 'word)
           (define-command! "answer-old" "" (lambda ()
             (log! 'info (if (completion-emit! (car (cadr ids)) (list (hash "label" "x"))) "applied" "dropped"))))
           (define-command! "prefixes" "" (lambda ()
             (log! 'info (string-join (map cdr (reverse ids)) ","))))"#,
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    ed.feed_key(key('f'));
    ed.settle();
    ed.execute_keymap_command("prefixes".into(), None, false);
    assert_eq!(status(&ed), ",f", "the second call saw the typed \"f\"");
    ed.execute_keymap_command("answer-old".into(), None, false);
    assert_eq!(status(&ed), "dropped");
}

// ── Several sources ───────────────────────────────────────────────────────

fn two_sources(a: &str, b: &str) -> String {
    format!(
        r#"(register-completion-source! "a"
             (lambda (id bid prefix) (completion-emit! id {a}))
             #:target 'buffer #:token 'word)
           (register-completion-source! "b"
             (lambda (id bid prefix) (completion-emit! id {b}))
             #:target 'buffer #:token 'word #:priority 10)"#
    )
}

/// Items from two sources rank together by one score — a contiguous-prefix
/// match from the second outranks a scattered match from the first
/// regardless of arrival order or sortText.
#[test]
fn two_sources_rank_together_by_score() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("rn-[ ]>\n");
    run(
        &mut ed,
        tmp.path(),
        &two_sources(
            r#"(list (hash "label" "random" "sortText" "a"))"#,
            r#"(list (hash "label" "rnorm" "sortText" "z"))"#,
        ),
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert_eq!(labels(&ed), vec!["rnorm", "random"]);
}

/// Priority is a tiebreaker applied *before* sortText: on a score tie the
/// higher-priority source's item ranks first even when its label sorts
/// last.
#[test]
fn priority_breaks_a_score_tie_before_sort_text() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        &two_sources(
            r#"(list (hash "label" "aaa"))"#,
            r#"(list (hash "label" "zzz"))"#,
        ),
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert_eq!(labels(&ed), vec!["zzz", "aaa"]);
}

#[test]
fn top_carries_the_contributing_source_name() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            "{}\n{}",
            completion_source("test-source", &completion_labels(&["x"]), ""),
            r#"(define-command! "src" "" (lambda ()
                 (log! 'info (hash-ref (car (completion-top 1)) "source"))))"#
        ),
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    ed.execute_keymap_command("src".into(), None, false);
    assert_eq!(status(&ed), "test-source");
}

/// Two sources with different token rules each filter against their own
/// token and accept replaces the *selected item's* token — the other
/// source's span never bleeds into it.
#[test]
fn each_source_keeps_its_own_token() {
    let tmp = safe_tempdir();
    // "./fo" — a path-shaped source claims all four chars, a word source
    // only "fo".
    let mut ed = editor_from("./fo-[ ]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(register-completion-source! "word"
             (lambda (id bid prefix) (completion-emit! id (list (hash "label" "foobar"))))
             #:target 'buffer #:token 'word)
           (register-completion-source! "dir"
             (lambda (id bid prefix)
               (completion-emit! id (list (hash "label" "./foo.txt")) #:span (cons 0 4)))
             #:target 'buffer #:token 'custom)"#,
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    let ranked = labels(&ed);
    assert_eq!(
        ranked.len(),
        2,
        "both sources' items match their own token: {ranked:?}"
    );
    // Select the word source's item: accept must replace "fo", not "./fo".
    let row = ranked.iter().position(|l| l == "foobar").unwrap();
    for _ in 0..row {
        ed.feed_key(key_tab());
    }
    ed.feed_key(key_enter());
    assert_eq!(ed.doc().text().to_string(), "./foobar \n");
}

/// A trigger char invokes the source registered under its name (via
/// `register-trigger-chars!`) into the open session — the other source's
/// slot survives, shown again once the typed char is backspaced away.
#[test]
fn a_trigger_char_reinvokes_only_its_own_source_into_the_open_session() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let lang = ed.state.config.languages.intern("rust");
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).language = Some(lang);
    run(
        &mut ed,
        tmp.path(),
        r#"(define dot-calls 0)
           (register-completion-source! "dot"
             (lambda (id bid prefix)
               (set! dot-calls (+ dot-calls 1))
               (completion-emit! id (list (hash "label" (string-append "dot" (number->string dot-calls))))))
             #:target 'buffer #:token 'cursor)
           (register-completion-source! "other"
             (lambda (id bid prefix) (completion-emit! id (list (hash "label" "other"))))
             #:target 'buffer #:token 'cursor)
           (register-trigger-chars! "dot" "rust" (list "."))"#,
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert_eq!(labels(&ed), vec!["dot1", "other"]);

    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(
        labels(&ed),
        vec!["dot2"],
        "the dot source answered afresh at the new cursor; \"other\"'s token now \
         holds \".\" and matches nothing"
    );
    ed.feed_key(key_backspace());
    ed.settle();
    assert_eq!(
        labels(&ed),
        vec!["other"],
        "\"other\"'s slot survived the trigger — its token is empty again; the dot \
         source's token (the cursor after \".\") was crossed and dropped"
    );
}

/// A trigger char with no source registered under its name leaves an open
/// session alone (beyond the ordinary refilter) and opens none.
#[test]
fn a_trigger_char_nobody_registered_for_does_nothing() {
    let mut ed = editor_from("-[a]>bcdef\n");
    let lang = ed.state.config.languages.intern("rust");
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).language = Some(lang);
    ed.feed_key(key('i'));
    ed.feed_key(key('.'));
    ed.settle();
    assert!(ed.state.input.completion().is_none());
    assert_eq!(ed.doc().text().to_string(), ".abcdef\n");
}

/// Ctrl-Space with the menu already up re-invokes every source into the
/// same session — the layer it re-pushes is what keeps the completion key
/// handler's "a bound command dismisses the session" rule from eating it.
#[test]
fn ctrl_space_with_the_menu_up_reinvokes_and_keeps_the_session() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(&mut ed, tmp.path(), &counting_source("#f"));
    ed.feed_key(key('i'));
    trigger(&mut ed);
    ed.feed_key(key_tab());
    trigger(&mut ed);
    assert_eq!(labels(&ed), vec!["foobar"]);
    ed.execute_keymap_command("report".into(), None, false);
    assert_eq!(status(&ed), "2");
    assert_eq!(selected_row(&ed), 0, "a fresh layer starts at row 0");
}

// ── Registration ──────────────────────────────────────────────────────────

#[test]
fn re_registering_a_name_replaces_the_source() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            "{}\n{}",
            completion_source("test", &completion_labels(&["old"]), ""),
            completion_source("test", &completion_labels(&["new"]), ""),
        ),
    );
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert_eq!(labels(&ed), vec!["new"]);
}

/// A Steel `'buffer` source registered under `"path"` — the native
/// `Minibuf` source `:e` completion names — must not silently take the
/// name: the write is refused, an `Error` is reported, and `:e`'s own path
/// completion keeps working.
#[test]
fn a_cross_target_registration_under_a_native_name_is_refused() {
    let dir = safe_tempdir();
    std::fs::write(dir.path().join("hello.txt"), b"").unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        dir.path(),
        r#"(register-completion-source! "path"
             (lambda (id bid prefix) (completion-emit! id '()))
             #:target 'buffer #:token 'word)"#,
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error && e.text.contains("path")),
        "got: {:?}",
        ed.state.message_log.entries().collect::<Vec<_>>()
    );
    ed.handle_key(key(':'));
    type_chars(&mut ed, &format!("e {}/hel", dir.path().display()));
    ed.handle_key(key_tab());
    assert_eq!(
        ed.state
            .minibuf()
            .map(|mb| mb.input.clone())
            .unwrap_or_default(),
        format!("e {}/hello.txt", dir.path().display()),
        "the native path source must still be the one serving `:e`"
    );
}

/// A registration inside an init that then fails is never applied — the
/// `Effect` is dropped with everything else the failed eval queued.
#[test]
fn a_registration_in_a_failed_init_is_never_applied() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let init_path = tmp.path().join("init.scm");
    std::fs::write(
        &init_path,
        format!(
            "{}\n(error \"boom\")",
            completion_source("test", &completion_labels(&["x"]), "")
        ),
    )
    .unwrap();
    let mut host = ScriptingHost::new();
    let err = {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect_err("the init must fail");
    ed.apply_script_effects(err.effects);
    ed.scripting = Some(host);
    assert!(
        ed.state.config.completion_sources.id_of("test").is_none(),
        "the failed init's registration must not have been applied"
    );
}

#[test]
fn reload_config_forgets_a_steel_source() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        &completion_source("test", &completion_labels(&["x"]), ""),
    );
    assert!(ed.state.config.completion_sources.id_of("test").is_some());
    ed.reset_config_state();
    assert!(
        ed.state.config.completion_sources.id_of("test").is_none(),
        "the registry is rebuilt from the natives on reload"
    );
    assert!(
        ed.state.config.completion_sources.id_of("path").is_some(),
        "…and the natives are still there"
    );
}

// ── Sessions that outlive their buffer ────────────────────────────────────

/// The `:` line's own Tab completion must not be affected by any of this —
/// a `Minibuf` session, never a `Buffer` one, opens for `:e <Tab>`.
#[test]
fn minibuffer_tab_completion_opens_a_minibuf_session() {
    let tmp = safe_tempdir();
    std::fs::write(tmp.path().join("hello.rs"), "").unwrap();
    std::fs::write(tmp.path().join("world.rs"), "").unwrap();
    let mut ed = editor_from("-[x]>\n");
    ed.handle_key(key(':'));
    type_chars(&mut ed, &format!("e {}/", tmp.path().display()));
    ed.handle_key(key_tab());
    let session = ed
        .state
        .input
        .completion()
        .expect("2+ path candidates must open a popup");
    assert!(session.minibuf_input().is_some());
    assert!(session.buffer().is_none());
}

#[test]
fn a_buffer_switch_dismisses_the_session_at_settle() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    type_chars(&mut ed, "hello");
    open_completion_session(&mut ed, &["candidate"]);
    assert!(ed.state.input.completion().is_some(), "sanity: open");

    let other = ed.open_buffer(Buffer::new(
        BufferText::from("other\n"),
        SelectionSet::default(),
    ));
    ed.switch_to_buffer_with_jump(other);
    ed.settle();
    assert!(
        ed.state.input.completion().is_none(),
        "dismiss_invalid_completion must dismiss the session once its pane shows \
         a different buffer"
    );
}

// ── A Steel typed command's declared completer ──────────────────────────────

/// `define-typed-command! … #:complete "path"` — a Steel `:` command gets
/// the same argument completion a built-in declares.
#[test]
fn a_steel_typed_command_can_declare_a_native_completer() {
    let tmp = safe_tempdir();
    let dir = safe_tempdir();
    std::fs::write(dir.path().join("hello.txt"), b"").unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "look" "" (lambda (arg) (log! 'info arg)) #:complete "path")"#,
    );
    ed.handle_key(key(':'));
    type_chars(&mut ed, &format!("look {}/hel", dir.path().display()));
    ed.handle_key(key_tab());
    assert_eq!(
        ed.state
            .minibuf()
            .map(|mb| mb.input.clone())
            .unwrap_or_default(),
        format!("look {}/hello.txt", dir.path().display())
    );
}

/// A `'minibuf` Steel source, named by a Steel typed command — the whole
/// `:` completion path with no native code involved.
#[test]
fn a_steel_minibuf_source_completes_a_typed_commands_argument() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(register-completion-source! "names"
             (lambda (id input cursor)
               (completion-emit! id (list (hash "label" "alice") (hash "label" "bob"))))
             #:target 'minibuf #:token 'arg #:match 'string)
           (define-typed-command! "greet" "" (lambda (arg) (log! 'info arg)) #:complete "names")"#,
    );
    ed.handle_key(key(':'));
    type_chars(&mut ed, "greet al");
    ed.handle_key(key_tab());
    ed.settle();
    assert_eq!(
        ed.state
            .minibuf()
            .map(|mb| mb.input.clone())
            .unwrap_or_default(),
        "greet alice",
        "the sole prefix match lands silently once the async source answers"
    );
    assert!(ed.state.input.completion().is_none());
}

/// A completer naming a `'buffer` source can't serve the `:` line — Tab
/// does nothing beyond a Trace line, never a panic or a wrong-target session.
#[test]
fn a_buffer_source_named_as_a_completer_is_ignored_with_a_trace() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            "{}\n{}",
            completion_source("words", &completion_labels(&["x"]), ""),
            r#"(define-typed-command! "greet" "" (lambda (arg) (log! 'info arg)) #:complete "words")"#
        ),
    );
    ed.handle_key(key(':'));
    type_chars(&mut ed, "greet a");
    ed.handle_key(key_tab());
    assert!(ed.state.input.completion().is_none());
    assert_eq!(
        ed.state
            .minibuf()
            .map(|mb| mb.input.clone())
            .unwrap_or_default(),
        "greet a"
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Trace && e.text.contains("serves the buffer")),
        "got: {:?}",
        ed.state.message_log.entries().collect::<Vec<_>>()
    );
}
