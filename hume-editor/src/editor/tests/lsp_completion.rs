// Completion orchestration: completion-begin!,
// completion-update-filter!, completion-top, completion-accept!,
// completion-dismiss!.
//
// Every test drives its command through `execute_keymap_command` rather
// than `type_cmd`'s `:`-typed path: `completion-begin!` gates on the mode
// layer being `Insert`, and typing `:` itself tears that layer down before
// a typed command's body ever runs. A raw
// `push_mode_layer(Insert)` (not a real `i` keypress) satisfies the gate
// without `begin_insert_session`'s side effects — no edit group opened, no
// selection collapsed — which several of these tests depend on staying
// untouched (their whole point is pinning `accept`'s *own*
// group-opening/collapsed-selection logic, the path a real Steel-triggered
// accept outside Insert mode takes).

use super::*;
use crate::editor::input_stack::InsertLayer;

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
                     (hash "label" "third" "sortText" "c")) #:source "test")
             (log! 'info (string-join (map (lambda (h) (hash-ref h "label")) (completion-top 10)) ","))))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "first,second,third",
        "with no filter, items must rank by sortText ascending"
    );
}

#[test]
fn update_filter_narrows_and_fuzzy_score_beats_sort_text() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "orange")                  ; "rn" scattered from offset 1 (r@1, n@3)
                     ; "rn" is a scattered subsequence of "random" (r@0, n@2) —
                     ; contiguous at neither char. sortText "a" would win
                     ; alphabetically if the fuzzy score didn't matter.
                     (hash "label" "random" "sortText" "a")
                     ; "rn" is a contiguous literal prefix of "rnorm" — nucleo's
                     ; boundary/contiguity/prefer-prefix bonuses put this well
                     ; above a scattered match regardless of sortText. sortText
                     ; "z" would lose alphabetically — the only way this can
                     ; still rank first is if the fuzzy score genuinely
                     ; dominates the sortText tie-break.
                     (hash "label" "rnorm" "sortText" "z")
                     (hash "label" "grape")) #:source "test")                  ; no "r" at all — dropped
             (completion-update-filter! "rn")
             (log! 'info (string-join (map (lambda (h) (hash-ref h "label")) (completion-top 10)) ","))))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "rnorm,random,orange",
        "a contiguous/prefix fuzzy match must rank above a scattered match \
         regardless of sortText, and an earlier scattered match above a later one"
    );
}

/// Smart case: a filter holding an uppercase char is case-sensitive, so
/// typing "Vec" excludes "vec_deque" entirely rather than just ranking it
/// lower — matches the picker's existing case behavior (`fuzzy.rs`'s
/// `smart_case_uppercase_query_is_case_sensitive`).
#[test]
fn update_filter_with_uppercase_query_is_case_sensitive() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "Vec") (hash "label" "vec_deque")) #:source "test")
             (completion-update-filter! "Vec")
             (log! 'info (string-join (map (lambda (h) (hash-ref h "label")) (completion-top 10)) ","))))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "Vec",
        "an uppercase-bearing filter must be case-sensitive and drop \
         lowercase-only items entirely, not merely rank them lower"
    );
}

/// A typed space lies outside any legitimate LSP-completion identifier, so
/// the fuzzy query must fail to match anything once it contains one —
/// that's what makes Insert mode's post-edit refilter see zero candidates
/// and treat the space as crossing the completed token's boundary.
/// Regression for `Pattern::new`'s word-splitting: it silently drops the
/// trailing empty atom after "foo ", so the filter scored every item `0`
/// (matching the empty pattern) instead of matching nothing.
#[test]
fn update_filter_with_trailing_space_matches_nothing() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "foobar")) #:source "test")
             (completion-update-filter! "foo ")
             (log! 'info (string-join (map (lambda (h) (hash-ref h "label")) (completion-top 10)) ","))))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "",
        "a filter with a trailing space must match nothing, not fall back to \
         matching every item as if the space were an ignored word separator"
    );
}

/// A bare space as the entire filter is the same bug at its worst: no
/// non-whitespace atoms survive `Pattern::new`'s word-split, so the buggy
/// parsing degenerates to an empty pattern that scores every item `0` —
/// silently resurfacing the full unfiltered list on the one keystroke
/// Insert mode most needs to close the menu.
#[test]
fn update_filter_with_only_a_space_matches_nothing() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "foobar") (hash "label" "foo")) #:source "test")
             (completion-update-filter! " ")
             (log! 'info (string-join (map (lambda (h) (hash-ref h "label")) (completion-top 10)) ","))))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "",
        "a filter that is a single space must match nothing — it must not \
         fall through to an empty pattern that matches every item"
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
               (list (hash "label" "foobar" "insertText" "hello")) #:source "test")
             (completion-update-filter! "fo")
             (completion-accept! 0)))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    // anchor = 0 (cursor was on 'a' at begin time), filter "fo" = 2 chars,
    // so the fallback replaces chars [0, 2) ("ab") with "hello".
    assert_eq!(ed.doc().text().to_string(), "hellocdef\n");
}

/// A server's `insertText`/`textEdit.newText` isn't guaranteed `\n`-only —
/// accept must normalize it the same way `apply-text-edits!` does, since
/// both converge on the same changeset-building chokepoint.
#[test]
fn accept_normalizes_crlf_in_insert_text() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "foobar" "insertText" "hel\r\nlo")) #:source "test")
             (completion-update-filter! "fo")
             (completion-accept! 0)))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(ed.doc().text().to_string(), "hel\nlocdef\n");
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
               (list (hash "label" "foobar" "insertText" "foobar")) #:source "test")
             (completion-accept! 0)))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(ed.doc().text().to_string(), "foobar bar\n");
}

#[test]
fn accept_with_no_text_edit_replaces_the_whole_configured_word_chars_run() {
    let tmp = safe_tempdir();
    // With '-' configured as a word char, "foo-ba" (up to the cursor) is one
    // token — the fallback replace span must cover it whole, the same way
    // every other word operation in this buffer would, not stop at the '-'
    // the way the built-in word rule does.
    let mut ed = editor_from("foo-ba-[ ]>bar\n");
    ed.state.settings.word_chars = "-".into();
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "foo-bar" "insertText" "foo-bar")) #:source "test")
             (completion-accept! 0)))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(ed.doc().text().to_string(), "foo-bar bar\n");
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
                                       "newText" "format!"))) #:source "test")
             (completion-update-filter! "for")
             (completion-accept! 0)))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    // Without the fix, only [0, 2) ("fo") is replaced, leaving the "r"
    // typed after begin sitting untouched next to the insert: "format!r".
    assert_eq!(ed.doc().text().to_string(), "format!\n");
}

#[test]
fn accept_with_an_off_spec_text_edit_range_not_containing_the_cursor_errors_and_leaves_the_buffer_untouched()
 {
    let tmp = safe_tempdir();
    // LSP spec (completion.rs `text_edit` doc, Note 1): a conforming
    // server's completion range always contains the request position — the
    // as-if-typed model `accept` uses (every edit expressed as a char count
    // behind/ahead of the live cursor) depends on that guarantee. This range
    // starts at char 1 while the cursor sits at char 0, deliberately
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
                                       "newText" "XYZ"))) #:source "test")
             (completion-accept! 0)))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    // A delete region that doesn't reach the cursor errors instead of
    // silently clamping to some other span the server never asked for — no
    // data loss, no guessing at a malformed server's intent.
    assert_eq!(
        ed.doc().text().to_string(),
        "abcdef\n",
        "buffer must be untouched when the server's range doesn't contain the cursor"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("does not contain the cursor"),
        "expected a containment error, got {msg:?}"
    );
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
               (list (hash "label" "foobar" "insertText" "hello")) #:source "test")
             (completion-update-filter! "fo")
             (completion-accept! 0)))"#,
    );
    // A real Insert entry, not the raw `push_mode_layer` this file uses
    // elsewhere — this test presses Esc below, and only `begin_insert_
    // session`'s own bookkeeping (an open edit group) makes that safe.
    ed.feed_key(key('i'));
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(ed.doc().text().to_string(), "hellocdef\n");

    // Back to Normal — 'u' is Insert-mode's own literal char otherwise.
    ed.feed_key(key_esc());
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
             (completion-begin! (current-buffer) (list (hash "label" "x" "insertText" "z")) #:source "test")
             (completion-dismiss!)
             (completion-accept! 0)))"#,
    );
    let before = ed.doc().text().to_string();
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
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
             (completion-begin! (current-buffer) (list (hash "label" "x" "insertText" "z")) #:source "test")
             ; An edit that never goes through completion-update-filter! —
             ; a raw text-edit builtin, not Insert-mode typing (which is
             ; wired to call completion-update-filter! automatically
             ; whenever a session is open, so it's no longer a valid
             ; example of a bypassing edit). Appended past "bcdef" so it
             ; doesn't overlap the session's own anchor..cursor span.
             (apply-text-edits! (current-buffer)
               (list (list (cons 0 6) (cons 0 6) "Q")))))
           (define-command! "finish" "" (lambda ()
             (completion-accept! 0)))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("begin".into(), None, false);

    let before_accept = ed.doc().text().to_string();
    ed.execute_keymap_command("finish".into(), None, false);
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

#[test]
fn accept_after_the_session_pane_loses_focus_errors_instead_of_writing_at_char_zero() {
    use crate::editor::commands::open_pane_in_layout;

    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let bid_a = ed.focused_buffer_id();
    let pid_a = ed.state.focus.id();
    // A second pane showing the same buffer, still unfocused.
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid_a,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();

    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "begin" "" (lambda ()
             (completion-begin! (current-buffer) (list (hash "label" "x" "insertText" "z")) #:source "test")))
           (define-command! "finish" "" (lambda ()
             (completion-accept! 0)))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("begin".into(), None, false);
    assert!(
        ed.state.input.completion().is_some(),
        "sanity: session began"
    );

    // Nothing dismisses a session on a focused-pane change — focus moves to
    // the pane the session did *not* begin in. `pane_state::ensure` would
    // otherwise fabricate a fresh cursor at char 0 for pane B (it has never
    // shown this buffer's selections before), landing the completion at the
    // top of the file instead of erroring. A raw `set_for_test` rather than
    // `switch_focused_pane` — that helper's Normal-mode precondition doesn't
    // hold here on purpose (`open_pane_in_layout` already seeded `pid_b`'s
    // pane state through the real pane-creation path, same as its own
    // "a fresh test editor has no session open yet" carve-out just assumes
    // for its own callers).
    ed.state.focus.set_for_test(pid_b);

    let before = ed.doc().text().to_string();
    ed.execute_keymap_command("finish".into(), None, false);
    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "accept! must reject — the session's pane is no longer focused"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no longer focused"),
        "expected a pane-focus error, got {msg:?}"
    );
}

#[test]
fn accept_after_the_pane_switched_buffers_errors() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let original_buf = ed.focused_buffer_id();
    let original_text = ed.doc().text().to_string();
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "begin" "" (lambda ()
             (completion-begin! (current-buffer) (list (hash "label" "x" "insertText" "z")) #:source "test")))
           (define-command! "finish" "" (lambda ()
             (completion-accept! 0)))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("begin".into(), None, false);
    assert!(
        ed.state.input.completion().is_some(),
        "sanity: session began"
    );

    // The session's pane stays focused but is redirected to a different
    // buffer — a pane-buffer switch dismisses nothing synchronously (that's
    // `Editor::dismiss_invalid_completion`'s job, and it only runs at
    // settle), so this window is real between the switch and the next
    // drain. `PaneBufferState`'s own per-(pane, buffer) map can't catch it
    // either — it's retained, not removed, when a pane switches away.
    let scratch = ed.open_buffer(crate::editor::buffer::Buffer::scratch());
    ed.switch_to_buffer_without_jump(scratch);

    ed.execute_keymap_command("finish".into(), None, false);
    assert_eq!(
        ed.state.buffers.get(original_buf).text().to_string(),
        original_text,
        "accept! must reject — the session's pane no longer shows its buffer"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("no longer shows"),
        "expected a pane/buffer-mismatch error, got {msg:?}"
    );
}

#[test]
fn accept_without_a_text_edit_errors_when_the_cursor_left_the_token() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "begin" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "x" "insertText" "z" "filterText" "ab"))
               #:source "test")))
           (define-command! "narrow" "" (lambda ()
             (completion-update-filter! "a")))
           (define-command! "finish" "" (lambda ()
             (completion-accept! 0)))"#,
    );
    // A real `i` (not a raw `push_mode_layer`) — the two keystrokes below
    // need an open edit group, which only a real Insert session provides.
    ed.feed_key(key('i'));
    ed.execute_keymap_command("begin".into(), None, false);
    // Two real keystrokes land in the buffer and are observed by the
    // session (`apply_insert_edit` -> `observe_edit`), moving the primary
    // head two chars past the anchor; the automatic refilter keeps
    // `self.filter` at "ab", matching.
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    // Narrows `self.filter` to one char with no matching real edit — the
    // documented case `accept`'s own comment describes: the session's
    // filter can be set directly by a scripted caller, decoupled from what
    // has actually been typed. `typed` (1) no longer covers the real head
    // (anchor + 2).
    ed.execute_keymap_command("narrow".into(), None, false);

    let before = ed.doc().text().to_string();
    ed.execute_keymap_command("finish".into(), None, false);
    assert_eq!(
        ed.doc().text().to_string(),
        before,
        "accept! must reject — the cursor sits outside the token the narrowed filter describes"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("token"),
        "expected a token-containment error, got {msg:?}"
    );
}

#[test]
fn a_same_length_out_of_band_edit_followed_by_update_filter_still_invalidates_the_session() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "begin" "" (lambda ()
             (completion-begin! (current-buffer) (list (hash "label" "x" "insertText" "z")) #:source "test")))
           (define-command! "corrupt" "" (lambda ()
             ; Same-length replace ("bcdef" -> "BCDEF") — bypasses
             ; observe_edit (only apply_insert_edit calls it) and preserves
             ; length, so it survives observe_edit's own length check on the
             ; next real keystroke too. A subsequent filter update must not
             ; treat this silently-corrupted buffer as caught up.
             (apply-text-edits! (current-buffer)
               (list (list (cons 0 1) (cons 0 6) "BCDEF")))
             (completion-update-filter! "")))
           (define-command! "finish" "" (lambda ()
             (completion-accept! 0)))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("begin".into(), None, false);
    ed.execute_keymap_command("corrupt".into(), None, false);

    let before_accept = ed.doc().text().to_string();
    ed.execute_keymap_command("finish".into(), None, false);
    assert_eq!(
        ed.doc().text().to_string(),
        before_accept,
        "accept! must reject — a same-length out-of-band edit followed by a \
         filter update must not re-baseline the generation guard"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("changed"),
        "expected a buffer-changed error, got {msg:?}"
    );
}

#[test]
fn accept_errors_when_additional_text_edits_overlap_the_main_text_edit() {
    let tmp = safe_tempdir();
    // textEdit replaces chars [0, 3) ("abc"); additionalTextEdits targets
    // [2, 4) ("cd") — the two overlap at chars 2-3. Both edits must land in
    // one batched `ChangeSet` so the overlap is rejected; applying them
    // separately would drop that check.
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "x" "insertText" "ignored-fallback"
                           "textEdit" (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                        "end" (hash "line" 0 "character" 3))
                                       "newText" "XYZ")
                           "additionalTextEdits"
                             (list (hash "range" (hash "start" (hash "line" 0 "character" 2)
                                                      "end" (hash "line" 0 "character" 4))
                                     "newText" "QQ")))) #:source "test")
             (completion-accept! 0)))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(
        ed.doc().text().to_string(),
        "abcdef\n",
        "buffer must be untouched when additionalTextEdits overlaps the main textEdit"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("overlaps additionaltextedits"),
        "expected an overlap error, got {msg:?}"
    );
}

#[test]
fn accept_with_a_non_collapsed_selection_errors_instead_of_force_collapsing_it() {
    let tmp = safe_tempdir();
    // A real (non-collapsed) selection over "bc" — `replace_around_cursors`
    // would otherwise splice text around its head and force-collapse it,
    // silently discarding whatever the user had selected.
    //
    // Label "ab" — `completion-begin!` now seeds the filter from the word
    // before the cursor ("ab", the two chars right before the selection's
    // head), so the item must actually match it to survive into `filtered`
    // and be reachable as index 0.
    let mut ed = editor_from("a-[bc]>def\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer) (list (hash "label" "ab" "insertText" "z")) #:source "test")
             (completion-accept! 0)))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(
        ed.doc().text().to_string(),
        "abcdef\n",
        "buffer must be untouched — accept must reject a non-collapsed selection"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("must be collapsed"),
        "expected a collapsed-selection error, got {msg:?}"
    );
}

#[test]
fn accept_errors_when_additional_text_edits_zero_width_inserts_exactly_at_the_text_edit_end() {
    let tmp = safe_tempdir();
    // textEdit replaces chars [0, 2) ("ab") with "XY"; additionalTextEdits
    // is a *zero-width* insert of "ZZ" exactly at [2, 2) — the cursor
    // position, and thus the textEdit's own `end_now`. The half-open overlap
    // test alone (`s < end_now && start_now < e`) treats a zero-width range
    // at `end_now` as not overlapping (`e == end_now` fails `s < end_now`).
    // But `translate_in_place` maps a live selection head with `Assoc::
    // After` (cursor moves past text inserted at its own position), so once
    // the additional edit lands first, the live head sits *after* "ZZ" —
    // and the cursor edit's `back` chars then delete "ZZ" itself instead of
    // the "ab" the server's range targeted, silently destroying the
    // additional edit and leaving "cd" from the server's own range
    // untouched. Before this test's fix, that ran to completion as
    // "abXYcdef\n" instead of erroring.
    //
    // Label "ab" — matches the filter `completion-begin!` now seeds from
    // the word before the cursor, so the item survives into `filtered`.
    let mut ed = editor_from("ab-[c]>def\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "ab" "insertText" "ignored-fallback"
                           "textEdit" (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                        "end" (hash "line" 0 "character" 2))
                                       "newText" "XY")
                           "additionalTextEdits"
                             (list (hash "range" (hash "start" (hash "line" 0 "character" 2)
                                                      "end" (hash "line" 0 "character" 2))
                                     "newText" "ZZ")))) #:source "test")
             (completion-accept! 0)))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(
        ed.doc().text().to_string(),
        "abcdef\n",
        "buffer must be untouched — a zero-width additionalTextEdit sitting \
         exactly at the textEdit's end must count as an overlap"
    );
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("overlaps additionaltextedits"),
        "expected an overlap error, got {msg:?}"
    );
}

#[test]
fn accept_with_no_text_edit_remaps_the_primary_anchor_through_additional_text_edits() {
    let tmp = safe_tempdir();
    // `primary_head`/`anchor` are captured *before* `additionalTextEdits`
    // lands, in pre-shift buffer coordinates; the live cursor position used
    // to decide "is this the primary cursor" (and, once recognized, to
    // locate its token via `anchor`) is only available *after* it lands.
    // `completion-begin!` already seeds `anchor` at 0 (`word_start_before`
    // scans back through "abc") and `filter` at "abc"; `completion-update-
    // filter!` then narrows the filter to "wxyz" — matching the item's own
    // label (so it still survives filtering) but with no matching buffer
    // edit at all, so `typed` no longer reflects real typed content — the
    // exact divergence `anchor` exists to handle, see `ReplaceSpan::
    // TokenBefore`'s field docs. That pushes `anchor.shift(typed)` to 4,
    // short of `head_now` + the additionalTextEdits shift — so `head -
    // typed` and the correctly anchor-derived start land on genuinely
    // different buffer positions once additionalTextEdits (a 3-char
    // import-like insert at the top of the file) shifts everything after
    // it. Left unmapped, `primary_head` stays stale (3) against the live,
    // shifted head (6) — never recognized as primary — and the `head -
    // typed` fallback (`6.retreat_saturating(4)`) scans from position 2,
    // landing outside "abc" entirely (right after the "// " import) instead
    // of at the real token boundary (3, right after "// ", where "abc"
    // begins).
    let mut ed = editor_from("abc-[ ]>def\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "wxyz" "insertText" "X"
                           "additionalTextEdits"
                             (list (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                      "end" (hash "line" 0 "character" 0))
                                     "newText" "// ")))) #:source "test")
             (completion-update-filter! "wxyz")
             (completion-accept! 0)))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(
        ed.doc().text().to_string(),
        "// Xdef\n",
        "primary_head/anchor must be remapped through additionalTextEdits' \
         own changeset before deciding which cursor is primary and where \
         its token starts"
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
             (completion-begin! (current-buffer) (list) #:source "test")))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert!(
        ed.state.input.completion().is_none(),
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
             (completion-begin! (current-buffer) (list (hash "label" "x" "insertText" "z")) #:source "test")))
           (define-command! "reopen-empty" "" (lambda ()
             (completion-begin! (current-buffer) (list) #:source "test")))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("open".into(), None, false);
    assert!(
        ed.state.input.completion().is_some(),
        "sanity: session opened"
    );

    ed.execute_keymap_command("reopen-empty".into(), None, false);
    assert!(
        ed.state.input.completion().is_none(),
        "an isIncomplete re-request that comes back empty must close the open \
         session, not leave the previous items live"
    );
}

#[test]
fn a_same_length_out_of_band_edit_dismisses_the_session_at_settle() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "begin" "" (lambda ()
             (completion-begin! (current-buffer) (list (hash "label" "x" "insertText" "z")) #:source "test")))
           (define-command! "corrupt" "" (lambda ()
             (apply-text-edits! (current-buffer)
               (list (list (cons 0 1) (cons 0 6) "BCDEF")))))"#,
    );
    ed.feed_key(key('i'));
    ed.execute_keymap_command("begin".into(), None, false);
    assert!(
        ed.state.input.completion().is_some(),
        "sanity: session began"
    );

    // Same-length replace — bypasses `observe_edit` (only `apply_insert_edit`
    // calls it) and survives its length check on the next real keystroke,
    // but `dismiss_invalid_completion` catches the generation mismatch
    // directly, with no filter update or accept attempt needed first.
    ed.execute_keymap_command("corrupt".into(), None, false);
    ed.settle();
    assert!(
        ed.state.input.completion().is_none(),
        "dismiss_invalid_completion must dismiss the session after a \
         same-length out-of-band edit"
    );
}

#[test]
fn empty_items_from_a_stale_response_leaves_the_open_session_alone() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "open" "" (lambda ()
             (completion-begin! (current-buffer) (list (hash "label" "x" "insertText" "z")) #:source "test")))
           (define-command! "reopen-empty" "" (lambda ()
             (completion-begin! (current-buffer) (list) #:source "test")))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("open".into(), None, false);
    assert!(
        ed.state.input.completion().is_some(),
        "sanity: session opened"
    );

    // A modal overlay (`MenuLayer`, default `is_modal() == true`) lands on
    // top of the completion session since the (stale) request went out —
    // constructed directly, bypassing `show-menu!`'s own gate, which
    // requires the mode layer to be `Base` and would refuse to open here
    // while `Insert` is active. `is_settled_for::<CompletionLayer>` must
    // now read `false`, same as a mode-layer change.
    ed.state.push_layer(
        &ed.view,
        crate::editor::input_stack::MenuLayer {
            rows: hume_ui::popup::MenuRows::plain(vec!["x".to_string()]),
            selected: 0,
            callback: steel::rvals::SteelVal::Void,
        },
    );

    ed.execute_keymap_command("reopen-empty".into(), None, false);
    assert!(
        ed.state.input.completion().is_some(),
        "a stale empty response must leave the open session (and whatever \
         landed above it) alone, not dismiss it"
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
               (list (hash "label" "foobar" "insertText" "hello" "extra" "e1")) #:source "test")
             (completion-update-filter! "fo")
             (completion-accept! 0)))
           (register-hook! 'on-completion-accept (lambda (bid item)
             (log! 'info (hash-ref item "extra"))))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    ed.settle();
    assert_eq!(
        ed.doc().text().to_string(),
        "hellocdef\n",
        "sanity: the main edit must apply before the hook fires"
    );
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "e1",
        "on-completion-accept must receive the accepted item's raw JSON, including \
         fields (\"extra\") that CompletionItem doesn't otherwise parse"
    );
}

#[test]
fn accept_with_no_hook_registered_still_applies_the_edit() {
    // Fail oracle for the hook wiring: if `push` onto `pending_work` panicked
    // or the accept path never returned `Ok`, this would fail even with zero
    // handlers registered.
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "foobar" "insertText" "hello")) #:source "test")
             (completion-update-filter! "fo")
             (completion-accept! 0)))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    ed.settle();
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
               #:incomplete #t #:source "test")))
           (register-hook! 'on-completion-refilter (lambda (bid text)
             (log! 'info (string-append "refilter:" text))))"#,
    );
    // A real Insert entry: this test types 'f' for real below, and Insert-
    // mode typing's own edit path (`apply_insert_edit`) requires the edit
    // group `begin_insert_session` opens, unlike a Steel-triggered accept
    // (which opens its own on demand).
    ed.feed_key(key('i'));
    ed.execute_keymap_command("go".into(), None, false);
    ed.feed_key(key('f'));
    ed.settle();
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
               (list (hash "label" "foobar" "insertText" "hello")) #:source "test")))
           (register-hook! 'on-completion-refilter (lambda (bid text)
             (log! 'info "should-not-fire")))"#,
    );
    // Real Insert entry — see the sibling test above for why.
    ed.feed_key(key('i'));
    ed.execute_keymap_command("go".into(), None, false);
    ed.feed_key(key('f'));
    ed.settle();
    assert_ne!(
        ed.state.status_msg.clone().unwrap_or_default(),
        "should-not-fire",
        "on-completion-refilter must not fire for a complete (non-isIncomplete) session — \
         it's a bounded window, not an unconditional per-keystroke hook"
    );
}

// ── Multi-source merge (completion-add-items!) ────────────────────────

/// A stale token — the session was replaced by a new `completion-begin!`
/// since the caller captured it — must be a silent no-op, not an error and
/// not a merge into whatever session happens to be open now.
#[test]
fn add_items_with_a_stale_token_is_a_silent_no_op() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (define tok (completion-begin! (current-buffer) (list (hash "label" "a")) #:source "s1"))
             (completion-add-items! (+ tok 1000) (list (hash "label" "z")) #:source "s2")
             (log! 'info (string-join (map (lambda (h) (hash-ref h "label")) (completion-top 10)) ","))))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "a",
        "an add against a stale token must not merge into the live session"
    );
}

/// Items from two different sources are ranked together by the same fuzzy
/// filter — a contiguous-prefix match from the second source outranks a
/// scattered match from the first, exactly as `update_filter_narrows_and_
/// fuzzy_score_beats_sort_text` proves within one source.
#[test]
fn add_items_merges_and_reranks_across_sources_by_score() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (define tok (completion-begin! (current-buffer)
               (list (hash "label" "random" "sortText" "a")) #:source "a"))
             (completion-add-items! tok
               (list (hash "label" "rnorm" "sortText" "z")) #:source "b")
             (completion-update-filter! "rn")
             (log! 'info (string-join (map (lambda (h) (hash-ref h "label")) (completion-top 10)) ","))))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "rnorm,random",
        "a contiguous-prefix match from one source must outrank a scattered match \
         from another, regardless of arrival order or sortText"
    );
}

/// Re-emitting a source (the `isIncomplete` refilter flow re-invokes the
/// same source on the same session) replaces that source's prior
/// contribution wholesale rather than appending — no duplicates, and other
/// sources' items are untouched.
#[test]
fn add_items_same_source_replaces_rather_than_appends() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (define tok (completion-begin! (current-buffer)
               (list (hash "label" "x") (hash "label" "y")) #:source "s"))
             (completion-add-items! tok (list (hash "label" "x") (hash "label" "z")) #:source "s")
             (log! 'info (string-join (map (lambda (h) (hash-ref h "label")) (completion-top 10)) ","))))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "x,z",
        "re-adding source \"s\" must evict its prior items (x, y), not append \
         alongside them"
    );
}

/// A source arriving late via `completion-add-items!` with `#:incomplete
/// #t` must flip the session-level flag even though the session began
/// complete — `on-completion-refilter` gates on the OR across every
/// source's latest flag, not just the one `begin` saw.
#[test]
fn late_add_flips_incomplete_even_though_the_session_began_complete() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (define tok (completion-begin! (current-buffer)
               (list (hash "label" "foobar" "insertText" "hello")) #:source "a"))
             (completion-add-items! tok (list (hash "label" "other")) #:source "b" #:incomplete #t)))
           (register-hook! 'on-completion-refilter (lambda (bid text)
             (log! 'info (string-append "refilter:" text))))"#,
    );
    // Real Insert entry — see `refilter_fires_on_completion_refilter_only_when_incomplete`
    // above for why a raw mode-layer push isn't enough here.
    ed.feed_key(key('i'));
    ed.execute_keymap_command("go".into(), None, false);
    ed.feed_key(key('f'));
    ed.settle();
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "refilter:f",
        "a late add's #:incomplete #t must flip a session that began complete"
    );
}

/// Merging in a second source must reset the menu selection to row 0 —
/// matching what `refilter_lsp_completion_after_edit` already does on
/// every keystroke, since a re-rank can move whatever row was under the
/// cursor.
#[test]
fn add_items_resets_the_menu_selection_to_row_zero() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define completion-token #f)
           (define-command! "begin" "" (lambda ()
             (set! completion-token (completion-begin! (current-buffer)
               (list (hash "label" "a") (hash "label" "b") (hash "label" "c")) #:source "s"))))
           (define-command! "merge" "" (lambda ()
             (completion-add-items! completion-token (list (hash "label" "d")) #:source "s2")))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("begin".into(), None, false);
    ed.feed_key(key_tab());
    ed.feed_key(key_tab());
    assert_eq!(
        ed.state.input.completion_ui().unwrap().selected,
        2,
        "sanity check: two Tabs move the selection off row 0"
    );
    ed.execute_keymap_command("merge".into(), None, false);
    assert!(
        ed.state.input.completion_ui().is_none(),
        "a merge must reset the selection to row 0 (cleared, same as refilter's own reset)"
    );
}

#[test]
fn update_filter_resets_the_menu_selection_to_row_zero() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "begin" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "a") (hash "label" "b") (hash "label" "c")) #:source "s")))
           (define-command! "narrow" "" (lambda ()
             (completion-update-filter! "")))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("begin".into(), None, false);
    ed.feed_key(key_tab());
    ed.feed_key(key_tab());
    assert_eq!(
        ed.state.input.completion_ui().unwrap().selected,
        2,
        "sanity check: two Tabs move the selection off row 0"
    );
    ed.execute_keymap_command("narrow".into(), None, false);
    assert!(
        ed.state.input.completion_ui().is_none(),
        "completion-update-filter! must reset the selection to row 0, same as add_items/refilter"
    );
}

/// Priority is a tiebreaker applied *before* sortText, not after — a
/// higher-priority source's item must rank first on a score tie even when
/// its label sorts alphabetically last.
#[test]
fn add_items_priority_breaks_a_score_tie_before_sort_text() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (define tok (completion-begin! (current-buffer)
               (list (hash "label" "aaa")) #:source "lo" #:priority 0))
             (completion-add-items! tok (list (hash "label" "zzz")) #:source "hi" #:priority 10)
             (log! 'info (string-join (map (lambda (h) (hash-ref h "label")) (completion-top 10)) ","))))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "zzz,aaa",
        "the higher-priority source's item must rank first despite losing on sortText"
    );
}

#[test]
fn top_json_still_carries_the_contributing_source_name() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer) (list (hash "label" "x")) #:source "test-source")
             (log! 'info (hash-ref (car (completion-top 1)) "source"))))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "test-source",
        "completion-top's items must still carry the contributing source's name"
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
                 (completion-begin! (current-buffer) (list {items}) #:source "test")
                 (completion-update-filter! "item5")
                 (completion-top 64)
                 (completion-accept! 0)))"#
        ),
    );
    let start = std::time::Instant::now();
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 5,
        "1k-item begin->filter->top->accept took {elapsed:?}, over the 5ms guardrail budget"
    );
}

/// `completion-begin!` for a buffer that isn't shown in the focused pane —
/// the normal shape of an async LSP completion response landing after the
/// user switched panes — must be a benign no-op (Trace log, no session
/// created), not an error: an error here would abort the whole `Call` batch
/// this callback was drained in and drop every other queued callback/timer
/// batched alongside it.
#[test]
fn completion_begin_for_a_buffer_not_shown_in_the_focused_pane_is_a_benign_no_op() {
    use crate::editor::commands::open_pane_in_layout;
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::CompletionHost;

    let dir = safe_tempdir();
    let file_b = dir.path().join("b.txt");
    std::fs::write(&file_b, "hello\n").unwrap();

    let mut ed = editor_from("-[a]>bcdef\n");
    let pid_a = ed.state.focus.id();
    let bid_a = ed.focused_buffer_id();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid_a,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();

    // Open a second file only in pane B, then focus back to pane A — bid_b
    // is now only ever recorded in pane B's per-pane state.
    ed.switch_focused_pane(pid_b);
    ed.execute_typed("e", Some(file_b.to_str().unwrap()))
        .unwrap();
    let bid_b = ed.focused_buffer_id();
    assert_ne!(bid_a, bid_b, "must be genuinely different buffers");
    ed.switch_focused_pane(pid_a);
    // The mode/top gate requires Insert to even reach the pane-mismatch
    // check this test is actually pinning — without it, a
    // call from Normal would Trace-drop on the mode check first instead.
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });

    let mut impl_host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let result = impl_host.completion_begin(
        bid_b,
        vec![serde_json::json!({"label": "x"})],
        "test".to_string(),
        0,
        hume_scripting::host::MatchKind::Fuzzy,
        false,
    );
    assert!(
        result.is_ok(),
        "unfocused-pane buffer must be a benign no-op, not an error: {result:?}"
    );

    assert!(
        ed.state.input.completion().is_none(),
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

/// `completion-begin!`'s refresh path (an `isIncomplete` re-request, or a
/// trigger char typed with the menu already up) must still land while a
/// non-modal `Popup` (hover, the `gn`/`gp` diagnostic overlay) has opened
/// above the completion session since — a `Popup` doesn't itself count as
/// "the stack moved" anywhere else, and the refresh must not be the one
/// place that disagrees.
///
/// Fail oracle: before this fix, `is_settled_or_top_is::<CompletionLayer>()`
/// required either every layer above `Insert` to be non-modal (false, since
/// `CompletionLayer` doesn't override `is_modal`) or `top()` to literally be
/// `Completion` (false, since `Popup` sits on top) — the refresh below would
/// Trace-drop and `len()` would stay `1`.
#[test]
fn completion_begin_refreshes_through_a_popup_landed_above_it() {
    use hume_scripting::host::CompletionHost;

    let mut ed = editor_from("-[a]>bcdef\n");
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    let bid = ed.focused_buffer_id();

    let mut host = live_host!(ed);
    host.completion_begin(
        bid,
        vec![serde_json::json!({"label": "x"})],
        "test".to_string(),
        0,
        hume_scripting::host::MatchKind::Fuzzy,
        true,
    )
    .unwrap();
    assert_eq!(ed.state.input.completion().unwrap().len(), 1, "sanity");

    ed.state.push_layer(
        &ed.view,
        crate::editor::input_stack::PopupLayer {
            text: "hover text".to_string(),
            scroll: 0,
            syntax: None,
            layout: hume_ui::popup::PopupLayout::Cursor,
            content: None,
        },
    );
    assert!(
        ed.state.input.popup().is_some(),
        "sanity: popup landed above it"
    );

    let mut host = live_host!(ed);
    host.completion_begin(
        bid,
        vec![
            serde_json::json!({"label": "x"}),
            serde_json::json!({"label": "y"}),
        ],
        "test".to_string(),
        0,
        hume_scripting::host::MatchKind::Fuzzy,
        false,
    )
    .unwrap();

    assert_eq!(
        ed.state.input.completion().unwrap().len(),
        2,
        "the session must have refreshed instead of being dropped as stale"
    );
}

// ── Filter seeded from the word before the cursor ────────────────────────

/// A prefix already typed before the trigger (Ctrl-Space, or a re-request)
/// must be filtered on immediately — the bug this fixes: `completion-begin!`
/// used to seed an empty filter regardless of what already preceded the
/// cursor, so every candidate survived (in `sortText` order) until some
/// later keystroke narrowed the list.
#[test]
fn completion_begin_seeds_the_filter_from_the_word_before_the_cursor() {
    use hume_scripting::host::CompletionHost;

    let mut ed = editor_from("ki-[x]>\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });

    let mut host = live_host!(ed);
    host.completion_begin(
        bid,
        vec![
            serde_json::json!({"label": "kitty_support"}),
            serde_json::json!({"label": "AsMut"}),
            serde_json::json!({"label": "f32"}),
        ],
        "test".to_string(),
        0,
        hume_scripting::host::MatchKind::Fuzzy,
        false,
    )
    .unwrap();

    let labels: Vec<String> = ed
        .state
        .input
        .completion()
        .unwrap()
        .top(10)
        .iter()
        .map(|v| v["label"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        labels,
        vec!["kitty_support"],
        "only the \"ki\" subsequence match must survive begin, not the whole list"
    );
}

/// The LSP `isIncomplete` flow re-`begin!`s the session from scratch on
/// every keystroke (`lsp/request-and-begin-completions`) rather than calling
/// `completion-add-items!` — a fresh `begin_buffer` must recompute the same
/// anchor/filter from the live buffer each time, not wipe out what the user
/// has typed since the first begin.
#[test]
fn completion_begin_reseeds_the_same_filter_across_repeated_begins_isincomplete_style() {
    use hume_scripting::host::CompletionHost;

    let mut ed = editor_from("ki-[x]>\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });

    let items = || {
        vec![
            serde_json::json!({"label": "kitty_support"}),
            serde_json::json!({"label": "AsMut"}),
        ]
    };
    let mut host = live_host!(ed);
    host.completion_begin(
        bid,
        items(),
        "test".to_string(),
        0,
        hume_scripting::host::MatchKind::Fuzzy,
        true, // isIncomplete
    )
    .unwrap();
    assert_eq!(
        ed.state.input.completion().unwrap().len(),
        1,
        "sanity: filtered on first begin"
    );

    // The refresh: a brand-new `completion-begin!` against the *same* live
    // buffer/cursor, exactly what `on-completion-refilter` triggers.
    let mut host = live_host!(ed);
    host.completion_begin(
        bid,
        items(),
        "test".to_string(),
        0,
        hume_scripting::host::MatchKind::Fuzzy,
        true,
    )
    .unwrap();

    let labels: Vec<String> = ed
        .state
        .input
        .completion()
        .unwrap()
        .top(10)
        .iter()
        .map(|v| v["label"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        labels,
        vec!["kitty_support"],
        "the re-begin must recompute the same filter from the live buffer, \
         not reset to an unfiltered list"
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
               (list (hash "label" "good") (hash "kind" 1)) #:source "test")
             (log! 'info (string-join (map (lambda (h) (hash-ref h "label")) (completion-top 10)) ","))))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);

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
             (completion-begin! (current-buffer) (list (hash "kind" 1) (hash "kind" 2)) #:source "test")))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);

    assert!(
        ed.state.input.completion().is_none(),
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
    //
    // Label "xa" — `completion-begin!` seeds the filter from the word
    // before the cursor ("a"), so the item must contain it to survive into
    // `filtered`.
    let mut ed = editor_from("a-[b]>cdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (completion-begin! (current-buffer)
               (list (hash "label" "xa"
                           "textEdit" (hash "insert" (hash "start" (hash "line" 0 "character" 1)
                                                            "end" (hash "line" 0 "character" 3))
                                            "replace" (hash "start" (hash "line" 0 "character" 1)
                                                             "end" (hash "line" 0 "character" 6))
                                            "newText" "XYZ"))) #:source "test")
             (completion-accept! 0)))"#,
    );
    ed.state
        .push_mode_layer(&ed.view, InsertLayer { sticky_popup: None });
    ed.execute_keymap_command("go".into(), None, false);
    // insert = [1,3) ("bc"), replace = [1,6) ("bcdef") — using replace would
    // leave "aXYZ\n"; the narrower insert range must leave "def" behind.
    assert_eq!(ed.doc().text().to_string(), "aXYZdef\n");
}
