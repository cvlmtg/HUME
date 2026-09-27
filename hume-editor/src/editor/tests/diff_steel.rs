// End-to-end Steel coverage for `diff-lines` / `diff-buffer-lines` and
// `diff-words`.

use super::*;
use crate::editor::message_log::Severity;
use hume_scripting::ScriptingHost;

/// `diff-lines` returns 0-based hunk tuples, oldest side first.
///
/// This is the one test that pins the shape registered at the Steel
/// boundary. Any change to field order, base, or list-vs-vector encoding
/// stops the probe from firing.
#[test]
fn diff_lines_returns_zero_based_hunk_tuples() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (diff-lines "a\nb\nc\n" "a\nB\nc\n")
                   (list (list 1 1 1 1 (list "b") (list "B"))))"#,
    );
    assert!(fired, "diff-lines must return the expected hunk shape");
}

/// `diff-buffer-lines` diffs `ref-text` (old) against the live buffer (new).
/// This pins the argument order: `old-lines` is the ref's line,
/// `new-lines` is the buffer's.
///
/// Swapping ref and buffer inside `DiffHost::diff_buffer_lines` would invert
/// the hunk's old/new sides and stop the probe from firing. Also stands in
/// for the doc's `diff-buffer-lines` ≡ `diff-lines` equivalence check,
/// since both route through the same `diff_bridge::line_hunks`, this single
/// assertion (same texts fed both ways) is structurally guaranteed rather
/// than needing a matrix.
#[test]
fn diff_buffer_lines_diffs_the_live_buffer_against_the_ref() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\nb\nc\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (diff-buffer-lines bid "a\nB\nc\n")
                   (list (list 1 1 1 1 (list "B") (list "b"))))"#,
    );
    assert!(
        fired,
        "diff-buffer-lines must diff the ref against the buffer's live text"
    );
}

/// `diff-buffer-lines` on a stale bid raises "invalid buffer id", not a
/// silent "no differences": `bid`'s liveness is checked at argument-resolve
/// time (`args::LiveBid`'s `BuiltinArg::resolve`, in the `builtins!`-
/// registered closure), before `diff_buffer_lines`'s body (and thus
/// `DiffHost::diff_buffer_lines` itself) ever runs. Errors raised inside a
/// `define-command!` body surface as a `Severity::Error` message-log entry
/// prefixed `"steel call error: "` (`scripting_setup.rs`'s `run_call_batch`
/// → `apply_script_result`), not as a Rust panic or a silent no-op. Hence
/// checking the log instead of a `run_probe` boolean.
///
/// If `diff-buffer-lines`'s `builtins!` table entry took `args::BidArg`
/// instead of `args::LiveBid`, only this test would fail. No other test in
/// this file passes a stale bid.
#[test]
fn diff_buffer_lines_on_a_stale_bid_raises_invalid_buffer_id() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");

    let scratch = tmp.path().join("scratch.txt");
    std::fs::write(&scratch, "x\n").unwrap();
    let scratch_str = steel_path(&scratch);

    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        &format!(
            r#"(define-typed-command! "probe" "" (lambda ()
                 (define b (open-buffer! {scratch_str}))
                 (close-buffer! b)
                 (diff-buffer-lines b "a\nb\n")))"#
        ),
        tmp.path(),
    );
    ed.scripting = Some(host);
    type_cmd(&mut ed, ":probe");

    let entries: Vec<_> = ed.state.message_log.entries().cloned().collect();
    assert!(
        entries.iter().any(|e| e.severity == Severity::Error
            && e.text.contains("diff-buffer-lines")
            && e.text.contains("invalid buffer id")),
        "a stale bid must surface as a Steel error naming the builtin, got: {entries:?}"
    );
}

/// `diff-words` returns a `(hunks . deadline-hit?)` dotted pair of 6-element
/// char-offset tuples.
///
/// This is the one test that pins the shape registered at the Steel
/// boundary. Any change to field order, offset base, or the dotted-pair
/// outer shape stops the probe from firing. Offsets
/// worked out by hand from `split_word_bounds()`'s tokenization of "foo bar"
/// (`"foo"`, `" "`, `"bar"`/`"baz"`: offsets `0,3,4,7`).
#[test]
fn diff_words_returns_a_hunks_and_deadline_hit_pair() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(equal? (diff-words "foo bar" "foo baz")
                   (cons (list (list 4 7 4 7 "bar" "baz")) #f))"#,
    );
    assert!(fired, "diff-words must return the expected hunk/pair shape");
}
