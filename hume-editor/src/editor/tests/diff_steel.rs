// End-to-end Steel coverage for `diff-lines` / `diff-buffer-lines` and
// `diff-words`.

use super::*;
use crate::editor::message_log::Severity;
use hume_platform::dirs::Dirs;
use hume_scripting::ScriptingHost;

/// `diff-lines` returns 0-based hunk hashes, oldest side first.
///
/// This is the one test that pins the shape registered at the Steel
/// boundary. Any change to a key name, the base, or the list encoding
/// stops the probe from firing.
#[test]
fn diff_lines_returns_zero_based_hunk_hashes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(&Dirs::none()),
        tmp.path(),
        r#"(equal? (diff-lines "a\nb\nc\n" "a\nB\nc\n")
                   (list (hash 'old-start 1 'old-count 1 'new-start 1 'new-count 1
                              'old-lines (list "b") 'new-lines (list "B")
                              'words (hash 'old (list) 'new (list)))))"#,
    );
    assert!(fired, "diff-lines must return the expected hunk shape");
}

/// `diff-buffer-lines` diffs `ref-text` (old) against the live buffer (new).
/// This pins the argument order: `old-lines` is the ref's line,
/// `new-lines` is the buffer's.
///
/// Swapping ref and buffer inside `DiffHost::diff_buffer_lines` would invert
/// the hunk's old/new sides and stop the probe from firing. Both builtins
/// build their hunks with `text_hunks`, so this one case stands in for a
/// `diff-buffer-lines` ≡ `diff-lines` matrix.
#[test]
fn diff_buffer_lines_diffs_the_live_buffer_against_the_ref() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>\nb\nc\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(&Dirs::none()),
        tmp.path(),
        r#"(equal? (diff-buffer-lines bid "a\nB\nc\n")
                   (list (hash 'old-start 1 'old-count 1 'new-start 1 'new-count 1
                              'old-lines (list "B") 'new-lines (list "b")
                              'words (hash 'old (list) 'new (list)))))"#,
    );
    assert!(
        fired,
        "diff-buffer-lines must diff the ref against the buffer's live text"
    );
}

/// `diff-buffer-lines` on a stale bid raises "invalid buffer id", not a
/// silent "no differences": `bid`'s liveness is checked at argument-resolve
/// time (`args::LivePane`'s `BuiltinArg::resolve`, in the `builtins!`-
/// registered closure), before `diff_buffer_lines`'s body (and thus
/// `DiffHost::diff_buffer_lines` itself) ever runs. Errors raised inside a
/// `define-command!` body surface as a `Severity::Error` message-log entry
/// prefixed `"steel call error: "` (`scripting_setup.rs`'s `run_call_batch`
/// → `apply_script_result`), not as a Rust panic or a silent no-op. Hence
/// checking the log instead of a `run_probe` boolean.
///
/// If `diff-buffer-lines`'s `builtins!` table entry took `args::ArgPane`
/// instead of `args::LivePane`, only this test would fail. No other test in
/// this file passes a stale bid.
#[test]
fn diff_buffer_lines_on_a_stale_bid_raises_invalid_buffer_id() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>\n");

    let scratch = tmp.path().join("scratch.txt");
    std::fs::write(&scratch, "x\n").unwrap();
    let scratch_str = steel_path(&scratch);

    let mut host = ScriptingHost::new(&Dirs::none());
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

/// `buffer-revision-diff` returns the hunk shape `diff-buffer-lines` returns,
/// with the revision's text as the old side and the live buffer as the new,
/// plus the char columns the edit itself touched under `'words`.
///
/// Deleting the `a` of "abc" makes revision 0 the text with the extra `a`, so
/// the old line is "abc" and its inserted column range is 0..1.
#[test]
fn buffer_revision_diff_returns_hunks_with_word_spans() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "bc\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(&Dirs::none()),
        tmp.path(),
        r#"(equal? (buffer-revision-diff bid 0)
                   (list (hash 'old-start 0 'old-count 1 'new-start 0 'new-count 1
                              'old-lines (list "abc") 'new-lines (list "bc")
                              'words (hash 'old (list (hash 'line 0 'start 0 'end 1))
                                           'new (list)))))"#,
    );
    assert!(
        fired,
        "buffer-revision-diff must return the expected hunk shape"
    );
}

/// A hunk whose changed span covers a whole line still has a `'words` key,
/// its span lists empty, so a renderer never word-diffs a changeset's hunk.
#[test]
fn buffer_revision_diff_keeps_words_when_a_span_covers_its_line() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>\n");
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "\n");

    let verdict = log_probe(
        &mut ed,
        tmp.path(),
        r#"(if (equal? (buffer-revision-diff bid 0)
                       (list (hash 'old-start 0 'old-count 1 'new-start 0 'new-count 1
                                  'old-lines (list "a") 'new-lines (list "")
                                  'words (hash 'old (list) 'new (list)))))
               "match"
               "mismatch")"#,
    );
    assert_eq!(
        verdict, "match",
        "a whole-line span must leave 'words present and empty"
    );
}

/// An id the buffer's history never recorded raises, never an empty diff.
#[test]
fn buffer_revision_diff_on_an_unknown_revision_raises() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    install_source(
        &mut ed,
        ScriptingHost::new(&Dirs::none()),
        r#"(define-typed-command! "probe" "" (lambda (bid) (buffer-revision-diff bid 99)))"#,
        tmp.path(),
    );
    type_cmd(&mut ed, ":probe");

    let entries: Vec<_> = ed.state.message_log.entries().cloned().collect();
    assert!(
        entries
            .iter()
            .any(|e| e.severity == Severity::Error && e.text.contains("no revision 99")),
        "an unknown revision must surface as a Steel error, got: {entries:?}"
    );
}

/// `diff-words` returns `(hash 'hunks … 'deadline-hit …)`, each hunk a hash of
/// char offsets and texts.
///
/// This is the one test that pins the shape registered at the Steel
/// boundary. Any change to a key name, the offset base, or the outer
/// shape stops the probe from firing. Offsets
/// worked out by hand from `split_word_bounds()`'s tokenization of "foo bar"
/// (`"foo"`, `" "`, `"bar"`/`"baz"`: offsets `0,3,4,7`).
#[test]
fn diff_words_returns_a_hunks_and_deadline_hit_hash() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(&Dirs::none()),
        tmp.path(),
        r#"(equal? (diff-words "foo bar" "foo baz")
                   (hash 'hunks (list (hash 'old-start 4 'old-end 7 'new-start 4 'new-end 7
                                        'old-text "bar" 'new-text "baz"))
                         'deadline-hit #f))"#,
    );
    assert!(fired, "diff-words must return the expected hunk/pair shape");
}

/// A replaced hunk with more old lines than new ones marks the edited words
/// on the line that holds them, not on the old line sharing the new line's index.
#[test]
fn diff_lines_words_follow_the_edit_across_uneven_sides() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(&Dirs::none()),
        tmp.path(),
        r##"(equal? (hash-ref (car (diff-lines "# h\n\nkeep one two\n" "keep two\n")) 'words)
                   (hash 'old (list (hash 'line 2 'start 4 'end 8)) 'new (list)))"##,
    );
    assert!(
        fired,
        "diff-lines must return the edit's span on its own line"
    );
}
