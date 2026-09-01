// End-to-end Steel coverage for `buffer-text` / `buffer-lines` /
// `selections-linewise?`.

use super::*;
use crate::editor::message_log::Severity;
use hume_scripting::ScriptingHost;

/// `buffer-text` returns the buffer's live, unsaved content — not a stale
/// on-open snapshot.
///
/// Fail oracle: reading the buffer's content at open time (or from disk)
/// instead of its current rope would still see "abcdef\n", not "Xabcdef\n".
#[test]
fn buffer_text_returns_live_dirty_content() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (buffer-text (current-buffer)) "Xabcdef\n")"#,
    );
    assert!(
        fired,
        "buffer-text must return the live, edited buffer content"
    );
}

/// `buffer-lines` excludes the phantom trailing line ropey counts past a
/// buffer's structural trailing `\n` — the same line the statusline and
/// `:w` never count either.
///
/// Fail oracle: dropping the ghost-line subtraction would return a fourth,
/// empty trailing entry.
#[test]
fn buffer_lines_excludes_the_phantom_trailing_line() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\nb\nc\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (buffer-lines (current-buffer)) (list "a" "b" "c"))"#,
    );
    assert!(
        fired,
        "buffer-lines must return exactly the content lines, no phantom trailing entry"
    );
}

/// `buffer-lines`' `#:start`/`#:end` range is 0-based and end-exclusive.
#[test]
fn buffer_lines_supports_a_start_end_range() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\nb\nc\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (buffer-lines (current-buffer) #:start 1 #:end 3) (list "b" "c"))"#,
    );
    assert!(
        fired,
        "buffer-lines must honor #:start/#:end as a 0-based, end-exclusive range"
    );
}

/// `#:start` alone: `#:end` defaults to the buffer's content line count,
/// not just to whatever `#:end` happened to be passed alongside it above.
#[test]
fn buffer_lines_start_only_defaults_end_to_the_line_count() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\nb\nc\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (buffer-lines (current-buffer) #:start 1) (list "b" "c"))"#,
    );
    assert!(
        fired,
        "buffer-lines with only #:start must default #:end to the line count"
    );
}

/// `buffer-text` always returns `\n` line endings, even for a buffer whose
/// source used `\r\n` — `BufferText::from`'s CRLF normalization, not a second
/// strip pass in the builtin itself.
///
/// Fail oracle: reading the rope's raw content without normalization would
/// still see the `\r` bytes.
#[test]
fn buffer_text_normalizes_crlf_to_lf() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\r\nb\r\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (buffer-text (current-buffer)) "a\nb\n")"#,
    );
    assert!(
        fired,
        "buffer-text must return LF line endings even for a CRLF-origin buffer"
    );
}

/// An `#:end` past the buffer's line count raises rather than silently
/// clamping — fail-fast, matching the project's error-handling convention.
///
/// Fail oracle: silently clamping `end` to the line count would leave
/// `:messages` empty instead of logging this error.
#[test]
fn buffer_lines_out_of_range_end_raises() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\nb\nc\n");

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "probe" "" (lambda ()
             (buffer-lines (current-buffer) #:end 10)))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":probe");

    let entries: Vec<_> = ed.state.message_log.entries().cloned().collect();
    assert!(
        entries.iter().any(|e| e.severity == Severity::Error
            && e.text.contains("buffer-lines")
            && e.text.contains("out of bounds")),
        "an out-of-range #:end must surface as a Steel error, got: {entries:?}"
    );
}

/// A `#:start` past `#:end` raises too — the other half of the `start > end
/// || end > line_count` guard, previously untested (deleting `start > end
/// ||` from the guard would have left the whole suite green).
///
/// Fail oracle: silently treating an inverted range as empty (or as
/// underflowing range math) would leave `:messages` empty instead of
/// logging this error.
#[test]
fn buffer_lines_start_past_end_raises() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\nb\nc\n");

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "probe" "" (lambda ()
             (buffer-lines (current-buffer) #:start 3 #:end 1)))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":probe");

    let entries: Vec<_> = ed.state.message_log.entries().cloned().collect();
    assert!(
        entries.iter().any(|e| e.severity == Severity::Error
            && e.text.contains("buffer-lines")
            && e.text.contains("out of bounds")),
        "a #:start past #:end must surface as a Steel error, got: {entries:?}"
    );
}

/// The user manual's `(viewport-range bid)` + `buffer-lines` recipe
/// (`user-manual/docs/plugins.md`) must not raise on the common case of a
/// buffer shorter than the pane, where the viewport's `end` sits at one past
/// the buffer's last content line — both ranges are 0-based and
/// end-exclusive, so `#:end` takes `(cdr vr)` directly, no `+ 1` needed.
///
/// Fail oracle: `viewport-range` returning one past the ropey phantom-line
/// index (two past the last content line) instead of one past the last
/// content line would make `#:end (cdr vr)` overshoot `buffer-lines`' bounds
/// check and raise instead of returning every content line.
#[test]
fn manual_viewport_range_recipe_reads_every_content_line_without_raising() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\nb\nc\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(let ((vr (viewport-range (current-buffer))))
             (equal? (buffer-lines (current-buffer)
                       #:start (car vr) #:end (cdr vr))
                     (list "a" "b" "c")))"#,
    );
    assert!(
        fired,
        "the documented viewport-range recipe must read every content line without raising"
    );
}

/// A stale bid raises "invalid buffer id" for both `buffer-text` and
/// `buffer-lines` — not an empty string/list.
fn assert_stale_bid_raises(builtin_call: &str, builtin_name: &str) {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");

    let scratch = tmp.path().join("scratch.txt");
    std::fs::write(&scratch, "x\n").unwrap();
    let scratch_str = scratch.to_string_lossy().replace('\\', "/");

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!(
            r#"(define-command! "probe" "" (lambda ()
                 (define b (open-buffer! "{scratch_str}"))
                 (close-buffer! b)
                 ({builtin_call} b)))"#
        ),
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":probe");

    let entries: Vec<_> = ed.state.message_log.entries().cloned().collect();
    assert!(
        entries.iter().any(|e| e.severity == Severity::Error
            && e.text.contains(builtin_name)
            && e.text.contains("invalid buffer id")),
        "a stale bid must surface as a Steel error naming the builtin, got: {entries:?}"
    );
}

/// Same pattern as `diff-buffer-lines`' stale-bid test.
#[test]
fn buffer_text_on_a_stale_bid_raises_invalid_buffer_id() {
    assert_stale_bid_raises("buffer-text", "buffer-text");
}

/// The same `buffer_line_count` lookup that backs the range check is itself
/// the liveness check.
#[test]
fn buffer_lines_on_a_stale_bid_raises_invalid_buffer_id() {
    assert_stale_bid_raises("buffer-lines", "buffer-lines");
}

/// `line->offset` returns the char offset where each content line starts.
///
/// Fail oracle: an off-by-one (e.g. forgetting the previous lines' `\n`
/// separators) would return 1 for line 1 instead of 2.
#[test]
fn line_to_offset_returns_each_lines_start_char_offset() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\nbb\nccc\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(and (= (line->offset (current-buffer) 0) 0)
                 (= (line->offset (current-buffer) 1) 2)
                 (= (line->offset (current-buffer) 2) 5))"#,
    );
    assert!(
        fired,
        "line->offset must return each content line's start char offset"
    );
}

/// `line->offset` counts in chars, not bytes — a line after a multi-byte
/// character must not be offset by its UTF-8 byte width.
///
/// Fail oracle: a byte-offset implementation would return 4 for line 1
/// instead of 2 ("é" is 1 char but 2 UTF-8 bytes, "a\n" contributes 2 chars).
#[test]
fn line_to_offset_counts_chars_not_bytes() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[é]>\nb\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(= (line->offset (current-buffer) 1) 2)"#,
    );
    assert!(
        fired,
        "line->offset must count chars, not UTF-8 bytes, ahead of the target line"
    );
}

/// A `line` at or past the buffer's content line count raises — including
/// the phantom trailing line past the structural `\n`, same convention as
/// `buffer-lines`' `#:end`.
#[test]
fn line_to_offset_out_of_range_line_raises() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\nb\n");

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "probe" "" (lambda ()
             (line->offset (current-buffer) 2)))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":probe");

    let entries: Vec<_> = ed.state.message_log.entries().cloned().collect();
    assert!(
        entries.iter().any(|e| e.severity == Severity::Error
            && e.text.contains("line->offset")
            && e.text.contains("out of range")),
        "an out-of-range line must surface as a Steel error, got: {entries:?}"
    );
}

/// Same pattern as `buffer-lines`' stale-bid test, but not reusing
/// `assert_stale_bid_raises`: that helper appends `b` as the call's *last*
/// arg, which fits `buffer-text`/`buffer-lines` (bid is their only/first
/// arg) but not `line->offset`, whose second arg (`line`) must come after
/// `b`, not before it.
#[test]
fn line_to_offset_on_a_stale_bid_raises_invalid_buffer_id() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");

    let scratch = tmp.path().join("scratch.txt");
    std::fs::write(&scratch, "x\n").unwrap();
    let scratch_str = scratch.to_string_lossy().replace('\\', "/");

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!(
            r#"(define-command! "probe" "" (lambda ()
                 (define b (open-buffer! "{scratch_str}"))
                 (close-buffer! b)
                 (line->offset b 0)))"#
        ),
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":probe");

    let entries: Vec<_> = ed.state.message_log.entries().cloned().collect();
    assert!(
        entries.iter().any(|e| e.severity == Severity::Error
            && e.text.contains("line->offset")
            && e.text.contains("invalid buffer id")),
        "a stale bid must surface as a Steel error naming the builtin, got: {entries:?}"
    );
}

// ── selections-linewise? ─────────────────────────────────────────────────────

#[test]
fn selections_linewise_true_for_a_full_line_selection() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[abc\n]>def\n");
    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(selections-linewise? (current-buffer))"#,
    );
    assert!(fired, "a single full-line selection must be linewise");
}

#[test]
fn selections_linewise_false_for_a_partial_selection() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[ab]>cdef\n");
    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(not (selections-linewise? (current-buffer)))"#,
    );
    assert!(fired, "a partial-line selection must not be linewise");
}

/// The predicate `selection-spans-full-line?` used to back — primary
/// selection only, exactly one line — returned `#f` here, even though the
/// selection spans two whole lines start-to-newline. That mismatch was a
/// live bug in `lsp-fmt`'s range-format gate (`core:lsp/format.scm`), whose
/// own docstring already promised "one or more complete lines".
#[test]
fn selections_linewise_true_for_a_multi_line_whole_line_selection() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[abc\ndef\n]>ghi\n");
    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(selections-linewise? (current-buffer))"#,
    );
    assert!(
        fired,
        "a selection spanning two whole lines must be linewise"
    );
}

#[test]
fn selections_linewise_true_for_several_linewise_selections() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[abc\n]>-[def\n]>ghi\n");
    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(selections-linewise? (current-buffer))"#,
    );
    assert!(
        fired,
        "several selections that are each individually linewise must be linewise"
    );
}

#[test]
fn selections_linewise_false_when_one_of_several_selections_is_partial() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[abc\n]>-[de]>f\n");
    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(not (selections-linewise? (current-buffer)))"#,
    );
    assert!(
        fired,
        "one non-linewise selection among several must make the whole set non-linewise"
    );
}

/// Oracle: `diff-buffer-lines` against a ref must agree with `diff-lines`
/// called on the ref and a `buffer-text` read — the cheaper, buffer-avoiding
/// path and the general-purpose path must produce identical hunks for the
/// same input.
#[test]
fn diff_buffer_lines_agrees_with_diff_lines_over_buffer_text() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\nb\nc\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(let ((ref "a\nB\nc\n"))
             (equal? (diff-buffer-lines (current-buffer) ref)
                     (diff-lines ref (buffer-text (current-buffer)))))"#,
    );
    assert!(
        fired,
        "diff-buffer-lines and diff-lines-over-buffer-text must produce identical hunks"
    );
}
