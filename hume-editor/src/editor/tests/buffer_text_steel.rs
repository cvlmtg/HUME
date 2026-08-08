// End-to-end Steel coverage for `buffer-text` / `buffer-lines` (Phase 4.2,
// docs/GIT-DIFF.md).

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
/// source used `\r\n` — `Text::from`'s CRLF normalization, not a second
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

/// The doc's stated oracle (`docs/GIT-DIFF.md`): `diff-buffer-lines` against
/// a ref must agree with `diff-lines` called on the ref and a `buffer-text`
/// read — the cheaper, buffer-avoiding path and the general-purpose path
/// must produce identical hunks for the same input.
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
