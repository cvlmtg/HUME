// End-to-end Steel coverage for `diff-lines` / `diff-buffer-lines` and
// `diff-words`.

use super::*;
use hume_core::Severity;
use hume_platform::dirs::Dirs;
use hume_scripting::ScriptingHost;

/// `diff-lines` returns 0-based hunk hashes, oldest side first.
///
/// This is the one test of the whole shape registered at the Steel
/// boundary.
#[test]
fn diff_lines_returns_zero_based_hunk_hashes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(&Dirs::none()),
        tmp.path(),
        r#"(equal? (hash-ref (diff-lines "a\nb\nc\n" "a\nB\nc\n") 'hunks)
                   (list (hash 'old-start-line 1 'old-end-line 2 'new-start-line 1 'new-end-line 2
                              'old-lines (list "b") 'new-lines (list "B")
                              'words (hash 'old (list) 'new (list)))))"#,
    );
    assert!(fired, "diff-lines must return the expected hunk shape");
}

/// `diff-buffer-lines` diffs `ref-text` (old) against the live buffer (new).
/// `old-lines` is the ref's line, `new-lines` is the buffer's.
///
/// Both builtins build their hunks with `text_hunks`, so this one case
/// stands in for a `diff-buffer-lines` ≡ `diff-lines` matrix.
#[test]
fn diff_buffer_lines_diffs_the_live_buffer_against_the_ref() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>\nb\nc\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(&Dirs::none()),
        tmp.path(),
        r#"(equal? (hash-ref (diff-buffer-lines bid "a\nB\nc\n") 'hunks)
                   (list (hash 'old-start-line 1 'old-end-line 2 'new-start-line 1 'new-end-line 2
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
/// time (`args::LiveBuffer`'s `BuiltinArg::resolve`, in the `builtins!`-
/// registered closure), before `diff_buffer_lines`'s body (and thus
/// `DiffHost::diff_buffer_lines` itself) ever runs. Errors raised inside a
/// `define-command!` body surface as a `Severity::Error` message-log entry
/// prefixed `"steel call error: "` (`event/pump.rs`'s `run_call_batch`
/// → `apply_script_result`), not as a Rust panic or a silent no-op. Hence
/// checking the log instead of a `run_probe` boolean.
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
                 (hash-ref (diff-buffer-lines b "a\nb\n") 'hunks)))"#
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
        r#"(equal? (hash-ref (buffer-revision-diff bid 0) 'hunks)
                   (list (hash 'old-start-line 0 'old-end-line 1 'new-start-line 0 'new-end-line 1
                              'old-lines (list "abc") 'new-lines (list "bc")
                              'words (hash 'old (list (hash 'line 0 'start-char-col 0 'end-char-col 1))
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
        r#"(if (equal? (hash-ref (buffer-revision-diff bid 0) 'hunks)
                       (list (hash 'old-start-line 0 'old-end-line 1 'new-start-line 0 'new-end-line 1
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
        r#"(define-typed-command! "probe" "" (lambda (bid) (hash-ref (buffer-revision-diff bid 99) 'hunks)))"#,
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

/// `buffer-changes-since` returns the hunks between the text a
/// `buffer-generation` value was taken at (old side) and the live text (new
/// side), across every edit since.
#[test]
fn buffer_changes_since_returns_the_hunks_since_a_generation() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\nd\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define snap #f)
           (define-typed-command! "snap" "" (lambda (bid) (set! snap (buffer-generation bid))))
           (define-typed-command! "now" "" (lambda (bid)
             (log! 'info (to-string (buffer-changes-since bid (buffer-generation bid))))))"#,
    );
    type_cmd(&mut ed, ":snap");
    ed.feed_key(key('d'));
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "c\nd\n");

    let hunks = log_probe(
        &mut ed,
        tmp.path(),
        r#"(to-string
             (map (lambda (h)
                    (list (hash-ref h 'old-start-line) (hash-ref h 'old-end-line)
                          (hash-ref h 'new-start-line) (hash-ref h 'new-end-line)
                          (hash-ref h 'old-lines) (hash-ref h 'new-lines)))
                  (buffer-changes-since bid snap)))"#,
    );
    assert_eq!(hunks, r#"((0 1 0 1 ("abc") ("c")))"#);

    type_cmd(&mut ed, ":now");
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("()"),
        "nothing separates the live text from itself"
    );
}

/// A generation value names its buffer: equal to itself, and different
/// once the buffer changes.
#[test]
fn a_buffer_generation_compares_equal_until_the_buffer_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define snap #f)
           (define-typed-command! "snap" "" (lambda (bid) (set! snap (buffer-generation bid))))
           (define-typed-command! "same" "" (lambda (bid)
             (log! 'info (if (equal? snap (buffer-generation bid)) "same" "moved"))))"#,
    );
    type_cmd(&mut ed, ":snap");
    type_cmd(&mut ed, ":same");
    assert_eq!(ed.state.status_msg.as_deref(), Some("same"));
    ed.feed_key(key('d'));
    type_cmd(&mut ed, ":same");
    assert_eq!(ed.state.status_msg.as_deref(), Some("moved"));
}

/// A generation of another buffer raises: its number means nothing for this
/// buffer's text.
#[test]
fn buffer_changes_since_with_another_buffers_generation_raises() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define snap #f)
           (define-typed-command! "snap" "" (lambda (bid) (set! snap (buffer-generation bid))))
           (define-typed-command! "since" "" (lambda (bid) (buffer-changes-since bid snap)))"#,
    );
    type_cmd(&mut ed, ":snap");
    open_other_buffer(&mut ed, "other\n");
    type_cmd(&mut ed, ":since");
    assert!(
        has_error(&ed, "a generation of another buffer"),
        "got: {:?}",
        error_log(&ed)
    );
}

/// `diff-words` returns `(hash 'hunks …)`, each hunk a hash of
/// char offsets and texts.
///
/// This is the one test of the whole shape registered at the Steel
/// boundary. Offsets worked out by hand from `split_word_bounds()`'s
/// tokenization of "foo bar" (`"foo"`, `" "`, `"bar"`/`"baz"`: offsets
/// `0,3,4,7`).
#[test]
fn diff_words_returns_a_hunks_hash() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(&Dirs::none()),
        tmp.path(),
        r#"(equal? (diff-words "foo bar" "foo baz")
                   (hash 'hunks (list (hash 'old-start-char 4 'old-end-char 7 'new-start-char 4 'new-end-char 7
                                        'old-text "bar" 'new-text "baz"))))"#,
    );
    assert!(fired, "diff-words must return the expected hunk/pair shape");
}

/// A replaced hunk with more old lines than new ones marks the edited words
/// on the line that holds them, not on the old line sharing the new line's index.
/// Deleting " one" or "one " gives the same text; the diff places an ambiguous
/// change as late as it can.
#[test]
fn diff_lines_words_follow_the_edit_across_uneven_sides() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(&Dirs::none()),
        tmp.path(),
        r##"(equal? (hash-ref (car (hash-ref (diff-lines "# h\n\nkeep one two\n" "keep two\n") 'hunks)) 'words)
                   (hash 'old (list (hash 'line 2 'start-char-col 5 'end-char-col 9)) 'new (list)))"##,
    );
    assert!(
        fired,
        "diff-lines must return the edit's span on its own line"
    );
}

/// Old-side spans name their line in the old text and their columns in it;
/// new-side spans are char offsets into the new text, ready for
/// `set-extra-highlights!`. Both hunks start on line 1, where an absolute
/// line or column differs from a hunk-relative one.
#[test]
fn diff_buffer_lines_words_are_absolute_lines_and_new_text_offsets() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>\nfoo bar\n");

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(&Dirs::none()),
        tmp.path(),
        r#"(equal? (hash-ref (car (hash-ref (diff-buffer-lines bid "a\nfoo baz\n") 'hunks)) 'words)
                   (hash 'old (list (hash 'line 1 'start-char-col 4 'end-char-col 7))
                         'new (list (hash 'start-char 6 'end-char 9))))"#,
    );
    assert!(
        fired,
        "old-side spans carry absolute old lines and new-side spans buffer char offsets"
    );
}
