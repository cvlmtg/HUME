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
    insert_with_script(
        &mut ed,
        tmp.path(),
        r#"(register-completion-source! "test"
             (lambda (id bid prefix)
               (log! 'info (string-append "prefix:" prefix "|bid-ok:"
                             (if (equal? bid (current-buffer)) "yes" "no")))
               (completion-emit! id (list (hash "label" "kitty_support"))))
             #:target 'buffer)"#,
    );
    assert_eq!(status(&ed), "prefix:ki|bid-ok:yes");
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
    assert!(ed.state.input.buffer_completion().is_none());
    assert_eq!(status(&ed), "completion-trigger: only in Insert mode");
}

#[test]
fn ctrl_space_with_no_buffer_source_registered_reports() {
    let mut ed = editor_from("-[a]>bcdef\n");
    ed.feed_key(key('i'));
    trigger(&mut ed);
    assert!(ed.state.input.buffer_completion().is_none());
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

// ── Answers ───────────────────────────────────────────────────────────────

#[test]
fn an_empty_answer_closes_the_session_and_reports_no_completions() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    insert_with_source(&mut ed, tmp.path(), "(list)");
    assert!(ed.state.input.buffer_completion().is_none());
    assert_eq!(status(&ed), "no completions");
}

/// While a source is pending the session is open (invisibly, with no rows)
/// — the answer, not the trigger, decides whether there is anything to
/// show.
#[test]
fn the_session_is_open_but_empty_until_the_source_answers() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    insert_with_script(
        &mut ed,
        tmp.path(),
        r#"(define pending-id #f)
           (register-completion-source! "test"
             (lambda (id bid prefix) (set! pending-id id))
             #:target 'buffer)
           (define-command! "answer" "" (lambda ()
             (completion-emit! pending-id (list (hash "label" "late")))))"#,
    );
    let session = ed
        .state
        .input
        .buffer_completion()
        .expect("open while pending");
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
    insert_with_script(
        &mut ed,
        tmp.path(),
        r#"(define calls '())
           (register-completion-source! "test"
             (lambda (id bid prefix) (set! calls (cons id calls)))
             #:target 'buffer)
           (define-command! "answer-first" "" (lambda ()
             (log! 'info (if (completion-emit! (cadr calls) (list (hash "label" "stale")))
                             "applied" "dropped"))))
           (define-command! "answer-second" "" (lambda ()
             (completion-emit! (car calls) (list (hash "label" "fresh")))))"#,
    );
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
    insert_with_script(
        &mut ed,
        tmp.path(),
        r#"(define calls '())
           (register-completion-source! "test"
             (lambda (id bid prefix)
               (set! calls (cons id calls))
               (completion-emit! id (list (hash "label" "foo") (hash "label" "fox")) #:incomplete #t))
             #:target 'buffer)
           (define-command! "answer-stale" "" (lambda ()
             (log! 'info (if (completion-emit! (car (reverse calls)) (list (hash "label" "OLD")))
                             "applied" "dropped"))))"#,
    );
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

/// The positive twin of the test above: a landing answer that *is* a live,
/// still-latest contribution (not stale, not dropped) resets the selection
/// to row 0 — the row the user had scrolled to on the narrower ranking has
/// no guaranteed meaning against the wider one the new source's items
/// produce.
#[test]
fn a_landing_answer_resets_the_selection_to_row_zero() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    insert_with_script(
        &mut ed,
        tmp.path(),
        r#"(define pending-id #f)
           (register-completion-source! "a"
             (lambda (id bid prefix)
               (completion-emit! id (list (hash "label" "aaa") (hash "label" "abc"))))
             #:target 'buffer)
           (register-completion-source! "b"
             (lambda (id bid prefix) (set! pending-id id))
             #:target 'buffer #:priority 10)
           (define-command! "answer-b" "" (lambda ()
             (completion-emit! pending-id (list (hash "label" "zzz")))))"#,
    );
    assert_eq!(
        labels(&ed),
        vec!["aaa", "abc"],
        "sanity: only source a has answered, b is still pending"
    );

    ed.feed_key(key_tab());
    assert_eq!(selected_row(&ed), 1, "sanity: moved off row 0");

    // b's answer is a fresh, still-latest contribution — not stale.
    ed.execute_keymap_command("answer-b".into(), None, false);
    assert_eq!(
        labels(&ed),
        vec!["zzz", "aaa", "abc"],
        "sanity: b's higher-priority item now ranks first"
    );
    assert_eq!(
        selected_row(&ed),
        0,
        "a live landing answer must reset the selection"
    );
}

/// A second answer for the *same* still-latest invocation replaces the
/// first — a source may stream.
#[test]
fn a_repeated_answer_for_a_live_invocation_replaces_the_first() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    insert_with_script(
        &mut ed,
        tmp.path(),
        r#"(define saved #f)
           (register-completion-source! "test"
             (lambda (id bid prefix)
               (set! saved id)
               (completion-emit! id (list (hash "label" "x") (hash "label" "y"))))
             #:target 'buffer)
           (define-command! "again" "" (lambda ()
             (completion-emit! saved (list (hash "label" "x") (hash "label" "z")))))"#,
    );
    ed.execute_keymap_command("again".into(), None, false);
    assert_eq!(labels(&ed), vec!["x", "z"], "replaced, not appended");
}

#[test]
fn an_answer_after_the_session_closed_is_dropped() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    insert_with_script(
        &mut ed,
        tmp.path(),
        r#"(define saved #f)
           (register-completion-source! "test"
             (lambda (id bid prefix) (set! saved id))
             #:target 'buffer)
           (define-command! "late" "" (lambda ()
             (log! 'info (if (completion-emit! saved (list (hash "label" "x"))) "applied" "dropped"))))"#,
    );
    ed.feed_key(key_esc());
    assert_eq!(
        ed.state.mode(),
        Mode::Normal,
        "sanity: Insert ended, session gone"
    );
    ed.execute_keymap_command("late".into(), None, false);
    assert_eq!(status(&ed), "dropped");
    assert!(ed.state.input.buffer_completion().is_none());
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
    assert!(ed.state.input.buffer_completion().is_none());
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
             #:target 'buffer)
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
    insert_with_script(&mut ed, tmp.path(), &counting_source("#t"));
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
    insert_with_script(&mut ed, tmp.path(), &counting_source("#f"));
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

/// A source that raises instead of ever calling `completion-emit!` must not
/// be retried on every subsequent keystroke — `drop_stalled_invocations`
/// clears its slot once the failed batch is reported, so it drops out of
/// `sources_to_reinvoke` until a fresh explicit trigger, instead of erroring
/// again on every edit. A second, working source registered alongside it
/// stays open and unaffected — `drop_stalled_invocations` only clears a
/// still-`inflight` slot, never a `shown` one.
#[test]
fn a_raising_source_is_not_retried_on_every_keystroke() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    insert_with_script(
        &mut ed,
        tmp.path(),
        r#"(register-completion-source! "ok"
             (lambda (id bid prefix) (completion-emit! id (list (hash "label" "foo"))))
             #:target 'buffer)
           (register-completion-source! "boom"
             (lambda (id bid prefix) (error "boom"))
             #:target 'buffer)"#,
    );
    let error_count = |ed: &Editor| {
        ed.state
            .message_log
            .entries()
            .filter(|e| e.text.contains("boom"))
            .count()
    };
    assert_eq!(
        error_count(&ed),
        1,
        "sanity: the trigger's own batch failed once"
    );
    assert_eq!(
        labels(&ed),
        vec!["foo"],
        "the working source's answer survives"
    );

    for ch in "def".chars() {
        ed.feed_key(key(ch));
        ed.settle();
    }
    assert_eq!(
        error_count(&ed),
        1,
        "a source that will never answer must not be retried on every \
         subsequent keystroke"
    );
}

/// The old answer stays ranked (against the new token text) until the
/// re-invocation's own answer lands — the menu never blinks empty.
#[test]
fn a_reinvoked_source_keeps_its_rows_until_the_new_answer_lands() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    insert_with_script(
        &mut ed,
        tmp.path(),
        r#"(define calls 0)
           (register-completion-source! "test"
             (lambda (id bid prefix)
               (set! calls (+ calls 1))
               (when (= calls 1)
                 (completion-emit! id (list (hash "label" "foo") (hash "label" "bar")) #:incomplete #t)))
             #:target 'buffer)"#,
    );
    ed.feed_key(key('f'));
    ed.settle();
    let session = ed.state.input.buffer_completion().expect("still open");
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
    insert_with_script(
        &mut ed,
        tmp.path(),
        r#"(define ids '())
           (register-completion-source! "test"
             (lambda (id bid prefix) (set! ids (cons (cons id prefix) ids)))
             #:target 'buffer)
           (define-command! "answer-old" "" (lambda ()
             (log! 'info (if (completion-emit! (car (cadr ids)) (list (hash "label" "x"))) "applied" "dropped"))))
           (define-command! "prefixes" "" (lambda ()
             (log! 'info (string-join (map cdr (reverse ids)) ","))))"#,
    );
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
             #:target 'buffer)
           (register-completion-source! "b"
             (lambda (id bid prefix) (completion-emit! id {b}))
             #:target 'buffer #:priority 10)"#
    )
}

/// Items from two sources rank together by one score — a contiguous-prefix
/// match from the second outranks a scattered match from the first
/// regardless of arrival order or sortText.
#[test]
fn two_sources_rank_together_by_score() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("rn-[ ]>\n");
    insert_with_script(
        &mut ed,
        tmp.path(),
        &two_sources(
            r#"(list (hash "label" "random" "sortText" "a"))"#,
            r#"(list (hash "label" "rnorm" "sortText" "z"))"#,
        ),
    );
    assert_eq!(labels(&ed), vec!["rnorm", "random"]);
}

/// Priority is a tiebreaker applied *before* sortText: on a score tie the
/// higher-priority source's item ranks first even when its label sorts
/// last.
#[test]
fn priority_breaks_a_score_tie_before_sort_text() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    insert_with_script(
        &mut ed,
        tmp.path(),
        &two_sources(
            r#"(list (hash "label" "aaa"))"#,
            r#"(list (hash "label" "zzz"))"#,
        ),
    );
    assert_eq!(labels(&ed), vec!["zzz", "aaa"]);
}

#[test]
fn top_carries_the_contributing_source_name() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    insert_with_script(
        &mut ed,
        tmp.path(),
        &format!(
            "{}\n{}",
            completion_source("test-source", &completion_labels(&["x"]), ""),
            r#"(define-command! "src" "" (lambda ()
                 (log! 'info (hash-ref (car (completion-top 1)) "source"))))"#
        ),
    );
    ed.execute_keymap_command("src".into(), None, false);
    assert_eq!(status(&ed), "test-source");
}

/// A trigger char invokes the `Buffer` source registered under its name
/// (via `completion-set-trigger-chars!`) into the open session — the other
/// source's slot survives, shown again once the typed char is backspaced
/// away.
#[test]
fn a_trigger_char_reinvokes_only_its_own_source_into_the_open_session() {
    let tmp = safe_tempdir();
    // A space, not a word char, right before the cursor — both sources'
    // `'word` token starts empty, so neither has a filter yet.
    let mut ed = editor_from("-[ ]>bcdef\n");
    let lang = ed.state.config.languages.intern("rust");
    let bid = ed.focused_buffer_id();
    ed.state.buffers.get_mut(bid).language = Some(lang);
    insert_with_script(
        &mut ed,
        tmp.path(),
        r#"(define dot-calls 0)
           (register-completion-source! "dot"
             (lambda (id bid prefix)
               (set! dot-calls (+ dot-calls 1))
               (completion-emit! id (list (hash "label" (string-append "dot" (number->string dot-calls))))))
             #:target 'buffer)
           (register-completion-source! "other"
             (lambda (id bid prefix) (completion-emit! id (list (hash "label" "other"))))
             #:target 'buffer)
           ;; Same eval as the "dot" registration above — `register-
           ;; completion-source!` only queues an `Effect`, applied after
           ;; this whole eval returns, and `completion-set-trigger-chars!`
           ;; is queued too (`Effect::SetCompletionTriggerChars`), so the
           ;; two apply in emission order rather than racing each other.
           (completion-set-trigger-chars! "dot" "rust" (list "."))"#,
    );
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
    assert!(ed.state.input.buffer_completion().is_none());
    assert_eq!(ed.doc().text().to_string(), ".abcdef\n");
}

/// Ctrl-Space with the menu already up re-invokes every source into the
/// same session — the layer it re-pushes is what keeps the completion key
/// handler's "a bound command dismisses the session" rule from eating it.
#[test]
fn ctrl_space_with_the_menu_up_reinvokes_and_keeps_the_session() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    insert_with_script(&mut ed, tmp.path(), &counting_source("#f"));
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
    insert_with_script(
        &mut ed,
        tmp.path(),
        &format!(
            "{}\n{}",
            completion_source("test", &completion_labels(&["old"]), ""),
            completion_source("test", &completion_labels(&["new"]), ""),
        ),
    );
    assert_eq!(labels(&ed), vec!["new"]);
}

/// A Steel `'buffer` source registered under `"path"` — the same name the
/// native `Minibuf` source `:e` completion uses — lands as a second,
/// independent source rather than a collision: `Buffer` and `Minibuf`
/// sources live in separate namespaces, so registering one never touches
/// the other, and `:e`'s own path completion is unaffected.
#[test]
fn the_same_name_in_both_namespaces_is_two_independent_sources() {
    let dir = safe_tempdir();
    std::fs::write(dir.path().join("hello.txt"), b"").unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        dir.path(),
        r#"(register-completion-source! "path"
             (lambda (id bid prefix) (completion-emit! id '()))
             #:target 'buffer)"#,
    );
    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error),
        "a second namespace's registration under a taken name is not an error: {:?}",
        ed.state.message_log.entries().collect::<Vec<_>>()
    );
    assert!(
        ed.state
            .config
            .completion_sources
            .buffer_id_of("path")
            .is_some(),
        "the buffer-namespace registration landed"
    );
    ed.handle_key(key(':'));
    type_chars(&mut ed, &format!("e {}/hel", dir.path().display()));
    ed.handle_key(key_tab());
    assert_eq!(
        minibuf_input(&ed),
        format!("e {}/hello.txt", dir.path().display()),
        "the native minibuf path source still serves `:e`, untouched by the buffer one"
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
        ed.state
            .config
            .completion_sources
            .buffer_id_of("test")
            .is_none(),
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
    assert!(
        ed.state
            .config
            .completion_sources
            .buffer_id_of("test")
            .is_some()
    );
    ed.reset_config_state();
    assert!(
        ed.state
            .config
            .completion_sources
            .buffer_id_of("test")
            .is_none(),
        "the registry is rebuilt from the natives on reload"
    );
    assert!(
        ed.state
            .config
            .completion_sources
            .minibuf_id_of("path")
            .is_some(),
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
    assert!(
        ed.state.input.minibuf_completion().is_some(),
        "2+ path candidates must open a popup"
    );
    assert!(ed.state.input.buffer_completion().is_none());
}

#[test]
fn a_buffer_switch_dismisses_the_session_at_settle() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.feed_key(key('i'));
    type_chars(&mut ed, "hello");
    open_completion_session(&mut ed, &["candidate"]);
    assert!(ed.state.input.buffer_completion().is_some(), "sanity: open");

    let other = ed.open_buffer(Buffer::new(
        BufferText::from("other\n"),
        SelectionSet::default(),
    ));
    ed.switch_to_buffer_with_jump(other);
    ed.settle();
    assert!(
        ed.state.input.buffer_completion().is_none(),
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
        minibuf_input(&ed),
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
             #:target 'minibuf #:match 'string)
           (define-typed-command! "greet" "" (lambda (arg) (log! 'info arg)) #:complete "names")"#,
    );
    ed.handle_key(key(':'));
    type_chars(&mut ed, "greet al");
    ed.handle_key(key_tab());
    ed.settle();
    assert_eq!(
        minibuf_input(&ed),
        "greet alice",
        "the sole prefix match lands silently once the async source answers"
    );
    assert!(ed.state.input.buffer_completion().is_none());
}

/// A completer naming a `'buffer` source can't serve the `:` line — the
/// name simply doesn't exist in the `Minibuf` namespace `#:complete` looks
/// in, so this is indistinguishable from any other stale/unregistered
/// name: Tab does nothing beyond the same Trace line, never a panic or a
/// wrong-target session.
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
    assert!(ed.state.input.buffer_completion().is_none());
    assert_eq!(minibuf_input(&ed), "greet a");
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Trace && e.text.contains("no completion source named")),
        "got: {:?}",
        ed.state.message_log.entries().collect::<Vec<_>>()
    );
}
