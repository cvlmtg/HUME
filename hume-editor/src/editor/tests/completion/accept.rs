//! What accepting a candidate writes — `CompletionSession::accept`
//! (`completion/session/accept.rs`): the `insertText` fallback over the
//! source's token, a server `textEdit` decoded against the invocation's
//! own snapshot, `additionalTextEdits`, undo grouping, the preconditions
//! that refuse, and every cursor of a multi-cursor session.
//!
//! Most tests drive `completion-accept!` from a Steel command dispatched
//! through `execute_keymap_command` on a raw `push_mode_layer(Insert)` —
//! see `raw_insert_with_source` — so `accept`'s *own* edit-group opening is
//! what's under test, not `begin_insert_session`'s.

use super::*;
use hume_editing::selection::{Selection, SelectionSet};
use hume_rope::offset::CharOffset;

const ACCEPT_0: &str = r#"(define-command! "finish" "" (lambda () (completion-accept! 0)))"#;

fn accept_via_steel(ed: &mut Editor) {
    ed.execute_keymap_command("finish".into(), None, false);
}

// ── The insertText fallback over the token ────────────────────────────────

#[test]
fn accept_with_no_text_edit_replaces_the_token_with_insert_text() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("fo-[ ]>cdef\n");
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "foobar" "insertText" "hello"))"#,
        ACCEPT_0,
    );
    accept_via_steel(&mut ed);
    assert_eq!(ed.doc().text().to_string(), "hello cdef\n");
}

/// A server's `insertText`/`textEdit.newText` isn't guaranteed `\n`-only —
/// accept normalizes it the same way `apply-text-edits!` does, since both
/// converge on the same changeset-building chokepoint.
#[test]
fn accept_normalizes_crlf_in_insert_text() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("fo-[ ]>cdef\n");
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "foobar" "insertText" "hel\r\nlo"))"#,
        ACCEPT_0,
    );
    accept_via_steel(&mut ed);
    assert_eq!(ed.doc().text().to_string(), "hel\nlo cdef\n");
}

/// With '-' configured as a word char, "foo-ba" is one token — the fallback
/// replaces it whole, the same way every other word operation would.
#[test]
fn accept_with_no_text_edit_replaces_the_whole_configured_word_chars_run() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("foo-ba-[ ]>bar\n");
    ed.state.settings.word_chars = "-".into();
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "foo-bar" "insertText" "foo-bar"))"#,
        ACCEPT_0,
    );
    accept_via_steel(&mut ed);
    assert_eq!(ed.doc().text().to_string(), "foo-bar bar\n");
}

// ── A server textEdit, decoded against the invocation's snapshot ──────────

/// The server's range was computed against the document at request time
/// ("fo"); a char typed after ("r") is inside the token, so the range's end
/// follows it — `Assoc::After` on the end, mapped through the observed edit.
#[test]
fn accept_with_a_text_edit_extends_the_range_over_chars_typed_since() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("fo-[ ]>\n");
    insert_with_script(
        &mut ed,
        tmp.path(),
        &completion_source(
            "test",
            r#"(list (hash "label" "format!" "insertText" "ignored-fallback"
                           "textEdit" (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                        "end" (hash "line" 0 "character" 2))
                                       "newText" "format!")))"#,
            "",
        ),
    );
    ed.feed_key(key('r'));
    ed.feed_key(key_enter());
    assert_eq!(ed.doc().text().to_string(), "format! \n");
}

/// A range that doesn't contain the cursor is off-spec (LSP: the completion
/// range always contains the request position) — accept errors with the
/// buffer untouched rather than guessing at the server's intent.
#[test]
fn accept_with_an_off_spec_text_edit_range_errors_and_leaves_the_buffer_untouched() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "x" "insertText" "ignored-fallback"
                       "textEdit" (hash "range" (hash "start" (hash "line" 0 "character" 1)
                                                    "end" (hash "line" 0 "character" 4))
                                   "newText" "XYZ")))"#,
        ACCEPT_0,
    );
    accept_via_steel(&mut ed);
    assert_eq!(ed.doc().text().to_string(), "abcdef\n");
    assert!(
        status(&ed).contains("does not contain the cursor"),
        "got {:?}",
        status(&ed)
    );
}

/// `CompletionTextEdit::InsertAndReplace` applies its narrower `insert`
/// range, not the wider `replace` range.
#[test]
fn insert_replace_text_edit_applies_the_narrower_insert_range() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("a-[b]>cdef\n");
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "xa"
                       "textEdit" (hash "insert" (hash "start" (hash "line" 0 "character" 1)
                                                        "end" (hash "line" 0 "character" 3))
                                        "replace" (hash "start" (hash "line" 0 "character" 1)
                                                         "end" (hash "line" 0 "character" 6))
                                        "newText" "XYZ")))"#,
        ACCEPT_0,
    );
    accept_via_steel(&mut ed);
    assert_eq!(ed.doc().text().to_string(), "aXYZdef\n");
}

// ── Undo grouping ─────────────────────────────────────────────────────────

#[test]
fn accept_is_one_undo_step() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("fo-[ ]>cdef\n");
    insert_with_script(
        &mut ed,
        tmp.path(),
        &completion_source(
            "test",
            r#"(list (hash "label" "foobar" "insertText" "hello"))"#,
            "",
        ),
    );
    ed.feed_key(key_enter());
    assert_eq!(ed.doc().text().to_string(), "hello cdef\n");

    ed.feed_key(key_esc());
    ed.handle_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "fo cdef\n");
}

// ── Preconditions that refuse ─────────────────────────────────────────────

#[test]
fn dismiss_clears_the_session_so_a_later_accept_errors() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "x" "insertText" "z"))"#,
        r#"(define-command! "finish" "" (lambda () (completion-dismiss!) (completion-accept! 0)))"#,
    );
    accept_via_steel(&mut ed);
    assert_eq!(ed.doc().text().to_string(), "abcdef\n");
    assert!(
        status(&ed).contains("no active completion session"),
        "got {:?}",
        status(&ed)
    );
}

/// An edit through a path the session never observed (a raw
/// `apply-text-edits!`) bumps the generation — accept refuses.
#[test]
fn a_buffer_edit_the_session_never_saw_invalidates_it() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "x" "insertText" "z"))"#,
        r#"(define-command! "finish" "" (lambda ()
             (apply-text-edits! (current-buffer) (list (list (cons 0 6) (cons 0 6) "Q")))
             (completion-accept! 0)))"#,
    );
    accept_via_steel(&mut ed);
    assert_eq!(
        ed.doc().text().to_string(),
        "abcdefQ\n",
        "only the raw edit landed"
    );
    assert!(status(&ed).contains("changed"), "got {:?}", status(&ed));
}

#[test]
fn accept_after_the_session_pane_loses_focus_errors_instead_of_writing_at_char_zero() {
    use crate::editor::commands::open_pane_in_layout;

    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let bid_a = ed.focused_buffer_id();
    let pid_a = ed.state.focus.id();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid_a,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "x" "insertText" "z"))"#,
        ACCEPT_0,
    );
    assert!(ed.state.input.buffer_completion().is_some(), "sanity");

    // Nothing dismisses a session on a focused-pane change synchronously —
    // `pane_state::ensure` would otherwise fabricate a cursor at char 0 for
    // pane B. A raw `set_for_test`: `switch_focused_pane`'s Normal-mode
    // precondition doesn't hold here on purpose.
    ed.state.focus.set_for_test(pid_b);

    accept_via_steel(&mut ed);
    assert_eq!(ed.doc().text().to_string(), "abcdef\n");
    assert!(
        status(&ed).contains("no longer focused"),
        "got {:?}",
        status(&ed)
    );
}

#[test]
fn accept_after_the_pane_switched_buffers_errors() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let original = ed.focused_buffer_id();
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "x" "insertText" "z"))"#,
        ACCEPT_0,
    );
    assert!(ed.state.input.buffer_completion().is_some(), "sanity");

    // The window between the switch and the next settle (which is when
    // `dismiss_invalid_completion` runs) is real.
    let scratch = ed.open_buffer(crate::editor::buffer::Buffer::scratch());
    ed.switch_to_buffer_without_jump(scratch);

    accept_via_steel(&mut ed);
    assert_eq!(
        ed.state.buffers.get(original).text().to_string(),
        "abcdef\n"
    );
    assert!(
        status(&ed).contains("no longer shows"),
        "got {:?}",
        status(&ed)
    );
}

/// A real (non-collapsed) selection: typing over one is a different edit
/// than completing at it, and `replace_around_cursors` would force-collapse
/// it — accept refuses instead of silently discarding the selection.
#[test]
fn accept_with_a_non_collapsed_selection_errors_instead_of_force_collapsing_it() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("a-[bc]>def\n");
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "ab" "insertText" "z"))"#,
        ACCEPT_0,
    );
    accept_via_steel(&mut ed);
    assert_eq!(ed.doc().text().to_string(), "abcdef\n");
    assert!(
        status(&ed).contains("must be collapsed"),
        "got {:?}",
        status(&ed)
    );
}

// ── additionalTextEdits ───────────────────────────────────────────────────

#[test]
fn accept_errors_when_additional_text_edits_overlap_the_main_text_edit() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "x" "insertText" "ignored-fallback"
                       "textEdit" (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                    "end" (hash "line" 0 "character" 3))
                                   "newText" "XYZ")
                       "additionalTextEdits"
                         (list (hash "range" (hash "start" (hash "line" 0 "character" 2)
                                                  "end" (hash "line" 0 "character" 4))
                                 "newText" "QQ"))))"#,
        ACCEPT_0,
    );
    accept_via_steel(&mut ed);
    assert_eq!(ed.doc().text().to_string(), "abcdef\n");
    assert!(
        status(&ed).contains("overlaps additionalTextEdits"),
        "got {:?}",
        status(&ed)
    );
}

/// A *zero-width* additional edit exactly at the textEdit's end counts as
/// an overlap: once it lands first, the live head sits after the inserted
/// text, and the cursor edit's `back` chars would delete that text instead
/// of the server's own range.
#[test]
fn accept_errors_when_additional_text_edits_zero_width_inserts_exactly_at_the_text_edit_end() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("ab-[c]>def\n");
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "ab" "insertText" "ignored-fallback"
                       "textEdit" (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                    "end" (hash "line" 0 "character" 2))
                                   "newText" "XY")
                       "additionalTextEdits"
                         (list (hash "range" (hash "start" (hash "line" 0 "character" 2)
                                                  "end" (hash "line" 0 "character" 2))
                                 "newText" "ZZ"))))"#,
        ACCEPT_0,
    );
    accept_via_steel(&mut ed);
    assert_eq!(ed.doc().text().to_string(), "abcdef\n");
    assert!(
        status(&ed).contains("overlaps additionalTextEdits"),
        "got {:?}",
        status(&ed)
    );
}

/// `back`/`forward` retreat a fixed count from the live head, which
/// `commit_char_edits` already shifted across the additional edit — so the
/// completion's own edit lands on the real token regardless of where the
/// import moved it to.
#[test]
fn accept_with_no_text_edit_lands_correctly_past_an_additional_text_edit_shift() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("abc-[ ]>def\n");
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "abc" "insertText" "X"
                       "additionalTextEdits"
                         (list (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                  "end" (hash "line" 0 "character" 0))
                                 "newText" "// "))))"#,
        ACCEPT_0,
    );
    accept_via_steel(&mut ed);
    assert_eq!(ed.doc().text().to_string(), "// X def\n");
}

/// A word-char-ending additional edit ("zz") adjacent to the token: a
/// distance-based retreat never scans into it, unlike a word-chars scan
/// would.
#[test]
fn accept_with_no_text_edit_never_eats_into_word_chars_an_additional_edit_inserted() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("abc-[ ]>def\n");
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "abc" "insertText" "X"
                       "additionalTextEdits"
                         (list (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                  "end" (hash "line" 0 "character" 0))
                                 "newText" "zz"))))"#,
        ACCEPT_0,
    );
    accept_via_steel(&mut ed);
    assert_eq!(ed.doc().text().to_string(), "zzX def\n");
}

/// additionalTextEdits' wire range is computed against the request-time
/// document; a keystroke since shifts everything after it — decoding
/// against the invocation's snapshot and mapping forward still finds
/// "extra" exactly.
#[test]
fn additional_text_edits_track_a_real_edit_observed_since_the_invocation() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("fo-[,]> extra\n");
    insert_with_script(
        &mut ed,
        tmp.path(),
        &completion_source(
            "test",
            r#"(list (hash "label" "foo" "insertText" "func"
                           "additionalTextEdits"
                             (list (hash "range" (hash "start" (hash "line" 0 "character" 4)
                                                      "end" (hash "line" 0 "character" 9))
                                     "newText" "EXTRA"))))"#,
            "",
        ),
    );
    ed.feed_key(key('o'));
    assert_eq!(ed.doc().text().to_string(), "foo, extra\n", "sanity");

    ed.feed_key(key_enter());
    assert_eq!(ed.doc().text().to_string(), "func, EXTRA\n");
}

// ── The accept hook ───────────────────────────────────────────────────────

#[test]
fn accept_fires_on_completion_accept_with_the_raw_item_after_the_edit() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("fo-[ ]>cdef\n");
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "foobar" "insertText" "hello" "extra" "e1"))"#,
        &format!(
            "{ACCEPT_0}\n{}",
            r#"(register-hook! 'on-completion-accept (lambda (bid item)
                 (log! 'info (hash-ref item "extra"))))"#
        ),
    );
    accept_via_steel(&mut ed);
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "hello cdef\n");
    assert_eq!(
        status(&ed),
        "e1",
        "the hook receives the raw item, including fields the store never parses"
    );
}

/// A label-only item (a bare string from `completion-emit!`, no wire
/// payload at all) still gives the hook something useful — `{"label": …}`
/// — rather than `null`.
#[test]
fn accept_fires_on_completion_accept_with_a_synthesized_label_for_a_plain_item() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[ ]>cdef\n");
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list "foobar")"#,
        &format!(
            "{ACCEPT_0}\n{}",
            r#"(register-hook! 'on-completion-accept (lambda (bid item)
                 (log! 'info (hash-ref item "label"))))"#
        ),
    );
    accept_via_steel(&mut ed);
    ed.settle();
    assert_eq!(status(&ed), "foobar");
}

// ── Multi-cursor ──────────────────────────────────────────────────────────
//
// `c` on two selections leaves two collapsed cursors in one Insert session
// — these pin that accepting lands the edit at every one of them, not just
// the primary, and that the session's bookkeeping stays correct regardless
// of which cursor is primary.

#[test]
fn accepting_lands_at_every_cursor_not_just_the_primary() {
    let mut ed = editor_from("-[foo]> -[bar]>\n");
    ed.feed_key(key('c'));
    type_chars(&mut ed, "st");
    open_completion_session(&mut ed, &["std"]);
    ed.feed_key(key_enter());
    assert_eq!(ed.doc().text().to_string(), "std std\n");
}

/// The `insertText` fallback's span is *not* uniform across cursors — each
/// cursor gets its own `word_start_before` scan from its own head, not the
/// primary's own tracked token re-expressed as a shared `(back, forward)`
/// distance: "abc" before the second cursor (with '-' a word char, one
/// contiguous run with its own typed prefix, no separator) is consumed
/// along with it, since it really is part of that cursor's own word —
/// unlike a uniform count, which would only ever eat as many chars as the
/// *primary*'s own prefix happened to be long, regardless of what actually
/// precedes each other cursor.
#[test]
fn accepting_consumes_each_cursors_own_word_run_not_a_uniform_count() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[foo]> abc-[bar]>\n");
    ed.state.settings.word_chars = "-".into();
    ed.feed_key(key('c'));
    type_chars(&mut ed, "x-");
    // `filterText` "x-st": the token is "x-" now and grows to "x-st" once
    // "st" lands below — the item must match both to stay ranked.
    run(
        &mut ed,
        tmp.path(),
        &completion_source(
            "test",
            r#"(list (hash "label" "std" "insertText" "std" "filterText" "x-st"))"#,
            "",
        ),
    );
    trigger(&mut ed);
    type_chars(&mut ed, "st");
    ed.feed_key(key_enter());
    assert_eq!(
        ed.doc().text().to_string(),
        "std std\n",
        "cursor 1 has no preceding word chars (\"x-st\" alone, at the buffer \
         start); cursor 2's own scan continues back through \"abc\" too \
         (contiguous with its \"x-st\", '-' configured as a word char), \
         consuming both"
    );
}

#[test]
fn accepting_a_server_text_edit_also_lands_at_every_cursor() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[foo]> -[bar]>\n");
    ed.feed_key(key('c'));
    type_chars(&mut ed, "st");
    let head = ed.current_selections().primary().head().index();
    // `newText` distinct from `label`/`insertText` — the server's range
    // drove the replacement, not the fallback.
    run(
        &mut ed,
        tmp.path(),
        &completion_source(
            "test",
            &format!(
                r#"(list (hash "label" "std" "insertText" "ignored-fallback"
                               "textEdit" (hash "range" (hash "start" (hash "line" 0 "character" {})
                                                            "end" (hash "line" 0 "character" {head}))
                                           "newText" "STD")))"#,
                head - 2
            ),
            "",
        ),
    );
    trigger(&mut ed);
    ed.feed_key(key_enter());
    assert_eq!(ed.doc().text().to_string(), "STD STD\n");
}

/// A `textEdit` from a source that never declared `#:resolve` decodes
/// `character` as a raw char count, not the buffer's attached-server wire
/// encoding — `completion_source`'s helper registers with no `#:resolve`
/// (defaults `#f`), so this pins the general case every other `textEdit`
/// test here already exercises without noticing, since none of them put a
/// multi-UTF-16-unit character earlier on the line to expose the
/// divergence. "😀" is 1 char but 2 UTF-16 units — a `character` position
/// counted in UTF-16 would land one char short of what this source, having
/// no wire encoding to honor, actually meant.
#[test]
fn a_non_resolve_sources_text_edit_decodes_character_as_a_char_count() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[😀foo]>\n");
    ed.feed_key(key('c'));
    // Cursor now sits right after "😀foo" was deleted, at char 0 — retype
    // "😀" so the line matches the fixture the test's own doc describes,
    // then trigger completion for a `textEdit` covering "foo" (chars 1..4).
    type_chars(&mut ed, "\u{1F600}");
    run(
        &mut ed,
        tmp.path(),
        &completion_source(
            "test",
            r#"(list (hash "label" "std" "insertText" "ignored-fallback"
                           "textEdit" (hash "range" (hash "start" (hash "line" 0 "character" 1)
                                                        "end" (hash "line" 0 "character" 4))
                                       "newText" "bar")))"#,
            "",
        ),
    );
    trigger(&mut ed);
    ed.feed_key(key_enter());
    assert_eq!(
        ed.doc().text().to_string(),
        "\u{1F600}bar\n",
        "character 1..4 must decode as chars 1..4 (\"foo\"), not UTF-16 units \
         1..4 (which would land one char short, on \"😀fo\")"
    );
}

#[test]
fn additional_text_edits_land_once_not_once_per_cursor() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[foo]> -[bar]>\n");
    ed.feed_key(key('c'));
    type_chars(&mut ed, "st");
    run(
        &mut ed,
        tmp.path(),
        &completion_source(
            "test",
            r#"(list (hash "label" "std" "insertText" "std"
                           "additionalTextEdits"
                             (list (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                      "end" (hash "line" 0 "character" 0))
                                     "newText" "// header\n"))))"#,
            "",
        ),
    );
    trigger(&mut ed);
    ed.feed_key(key_enter());
    let text = ed.doc().text().to_string();
    assert_eq!(text, "// header\nstd std\n");
    assert_eq!(text.matches("// header\n").count(), 1);
}

#[test]
fn multi_cursor_accept_is_one_undo_step_in_insert_mode() {
    let mut ed = editor_from("-[foo]> -[bar]>\n");
    ed.feed_key(key('c'));
    type_chars(&mut ed, "st");
    open_completion_session(&mut ed, &["std"]);
    ed.feed_key(key_enter());
    ed.feed_key(key('!'));
    assert_eq!(ed.doc().text().to_string(), "std! std!\n");
    ed.feed_key(key_esc());
    ed.handle_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "foo bar\n");
}

/// Two cursors placed directly, entirely outside a real Insert session —
/// no edit group is open going in, so `accept` opens its own, and one undo
/// still reverts both cursors' edits and the additional edit together.
#[test]
fn multi_cursor_accept_is_one_undo_step_from_steel_outside_insert_mode() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef gh-[i]>jkl\n");
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "std" "insertText" "X"
                       "additionalTextEdits"
                         (list (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                  "end" (hash "line" 0 "character" 0))
                                 "newText" "// header\n"))))"#,
        ACCEPT_0,
    );
    accept_via_steel(&mut ed);
    let text = ed.doc().text().to_string();
    assert_eq!(text.matches('X').count(), 2, "both cursors: {text:?}");
    assert!(text.starts_with("// header\n"));

    // By name rather than `u`: the raw-pushed Insert layer has no session
    // bookkeeping to tear down, so it is never left — `undo` runs as the
    // plain (no open group) command it is.
    ed.execute_keymap_command("undo".into(), None, false);
    assert_eq!(ed.doc().text().to_string(), "abcdef ghijkl\n");
}

/// The header lands between the two cursors: unrelated to either span, but
/// it shifts the second cursor's absolute position — its span still lands
/// on its own (shifted) content, not on text the header pushed its way.
#[test]
fn accepting_with_additional_text_edits_between_cursors_lands_correctly_at_both() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[foo]> abc-[bar]>\n");
    ed.feed_key(key('c'));
    type_chars(&mut ed, "xy");
    // Buffer "xy abcxy\n": cursor1 (primary) at char 2. Per-cursor word
    // scanning (this arm has no server `textEdit`) widens cursor2's own
    // span to [3, 8) — the whole "abcxy" run, not just its own typed "xy"
    // suffix — since 'a'/'b'/'c' are word chars too. Char 3, right at that
    // span's own start, is the one position strictly between the two spans
    // that overlaps neither: safely before cursor2's span (an insertion at
    // a span's own start shifts it uniformly ahead, per the overlap
    // check's own doc) and strictly after cursor1's.
    run(
        &mut ed,
        tmp.path(),
        &completion_source(
            "test",
            r#"(list (hash "label" "std" "insertText" "std" "filterText" "xy"
                           "additionalTextEdits"
                             (list (hash "range" (hash "start" (hash "line" 0 "character" 3)
                                                      "end" (hash "line" 0 "character" 3))
                                     "newText" "H"))))"#,
            "",
        ),
    );
    trigger(&mut ed);
    ed.feed_key(key_enter());
    assert_eq!(ed.doc().text().to_string(), "std Hstd\n");
}

/// A cramped cursor's own preceding word must never retreat across a line
/// boundary into unrelated text on the line above, however long the
/// *primary* cursor's own word happens to be — the finding this whole
/// per-cursor-scan design exists for: a uniform count derived from one
/// cursor's word and blindly applied to another can walk past that
/// cursor's own line start.
#[test]
fn accepting_never_retreats_a_shorter_cursor_across_a_line_boundary() {
    let tmp = safe_tempdir();
    // Primary's own word "ab" (2 chars) is longer than the second cursor's
    // own word "x" (1 char, on the line below) — a uniform 2-char retreat
    // from the second cursor's head would cross its line's own start and
    // eat the preceding newline plus a char of "ab".
    // The marker is a placeholder — real selections are set explicitly
    // below (`editor_from` requires at least one).
    let mut ed = editor_from("let a-[b]>\nx\n");
    ed.set_current_selections(SelectionSet::from_vec(
        vec![
            Selection::collapsed(CharOffset::new(6)), // right after "ab"
            Selection::collapsed(CharOffset::new(8)), // right after "x"
        ],
        0,
    ));
    raw_insert_with_source(
        &mut ed,
        tmp.path(),
        r#"(list (hash "label" "abstd" "insertText" "std"))"#,
        ACCEPT_0,
    );
    accept_via_steel(&mut ed);
    assert_eq!(
        ed.doc().text().to_string(),
        "let std\nstd\n",
        "each cursor's own word is replaced in place — neither line is \
         corrupted or joined with the other"
    );
}

/// Two cursors from one `c` with the SECOND primary: every keystroke at
/// cursor 1 shifts cursor 2's head by more than one char — the token must
/// be remapped through each keystroke so the filter is what was typed at
/// the primary, not drifted text.
#[test]
fn token_remap_keeps_the_filter_correct_when_primary_is_not_the_first_cursor() {
    let mut ed = editor_from("-[foo]> -[bar]>\n");
    ed.feed_key(key('c'));
    let heads: Vec<_> = ed
        .current_selections()
        .iter_sorted()
        .map(|s| s.head())
        .collect();
    ed.set_current_selections(SelectionSet::from_vec(
        heads.iter().map(|&h| Selection::collapsed(h)).collect(),
        1,
    ));
    open_completion_session(&mut ed, &["stable", "xyz"]);
    type_chars(&mut ed, "st");
    assert_eq!(
        labels(&ed),
        vec!["stable"],
        "ranked against exactly what was typed at the primary"
    );
    ed.feed_key(key_enter());
    assert_eq!(ed.doc().text().to_string(), "stable stable\n");
}
