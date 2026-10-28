//! `(buffer-search pane pattern)` and `(word-search-pattern pane word)`.

use super::*;
use pretty_assertions::assert_eq;

/// Defines `:probe`, which logs `expr` (with `bid` bound to the pane it runs
/// through) through `to-string`, runs it, and returns the logged text.
fn probe(ed: &mut Editor, tmp: &std::path::Path, expr: &str) -> String {
    run(
        ed,
        tmp,
        &format!(
            r#"(define-typed-command! "probe" "" (lambda (bid) (log! 'info (to-string {expr}))))"#
        ),
    );
    type_cmd(ed, ":probe");
    ed.state
        .status_msg
        .clone()
        .expect("the probe logged its value")
}

/// `pattern`'s first match in `ed`'s buffer as `"(start end)"`, or `"#false"`.
fn first_match(initial: &str, pattern: &str) -> String {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from(initial);
    probe(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(let ((m (buffer-search bid {pattern:?})))
                 (and m (list (hash-ref m 'start-char) (hash-ref m 'end-char))))"#
        ),
    )
}

#[test]
fn a_regex_gives_its_first_match_as_a_char_range() {
    assert_eq!(first_match("-[a]>lpha beta beta\n", "b.ta"), "(6 10)");
}

#[test]
fn no_match_gives_false() {
    assert_eq!(first_match("-[a]>lpha\n", "beta"), "#false");
}

/// The pattern reads as at the `/` prompt: an all-lowercase pattern ignores
/// case, one with a capital matches case.
#[test]
fn smart_case_applies() {
    assert_eq!(first_match("-[F]>oo foo\n", "foo"), "(0 3)");
    assert_eq!(first_match("-[F]>oo foo\n", "fOo"), "#false");
}

/// `v/` matches the rest of the pattern literally, as at the `/` prompt.
#[test]
fn the_verbatim_flag_matches_literally() {
    assert_eq!(first_match("-[a]>bc a.c\n", "a.c"), "(0 3)");
    assert_eq!(first_match("-[a]>bc a.c\n", "v/a.c"), "(4 7)");
}

#[test]
fn an_invalid_pattern_raises() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "bad" "" (lambda (bid) (buffer-search bid "(")))
           (define-typed-command! "empty" "" (lambda (bid) (buffer-search bid "")))"#,
    );
    type_cmd(&mut ed, ":bad");
    type_cmd(&mut ed, ":empty");
    let errors = error_log(&ed);
    assert_eq!(errors.len(), 2, "got {errors:?}");
    assert!(
        errors
            .iter()
            .all(|e| e.contains("buffer-search") && e.contains("pattern")),
        "got {errors:?}"
    );
}

/// Searching moves no selection and leaves the current search, its
/// register and its highlights as they were.
#[test]
fn a_search_changes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc abc\n");
    ed.state.registers.set_search_register("bc".to_string());
    let before = state(&ed);

    let found = probe(
        &mut ed,
        tmp.path(),
        r#"(hash-ref (buffer-search bid "abc") 'start-char)"#,
    );

    assert_eq!(found, "0");
    assert_eq!(state(&ed), before);
    assert_eq!(ed.state.registers.search_register(), Some("bc"));
    assert!(ed.doc().search_pattern.is_none());
}

/// A value naming only a buffer searches it: no pane has to show it.
#[test]
fn a_pane_less_value_searches_its_buffer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    let other = open_other_buffer(&mut ed, "xyz needle\n");
    let first = ed
        .state
        .buffers
        .iter()
        .map(|(id, _)| id)
        .find(|&id| id != other);
    ed.switch_to_buffer_with_jump(
        FocusedPane::current(&ed.view),
        first.expect("the first buffer is still open"),
    );
    let found = probe(
        &mut ed,
        tmp.path(),
        r#"(let ((hidden (car (filter (lambda (b) (not (equal? (buffer-key b) (buffer-key bid)))) (buffers)))))
             (hash-ref (buffer-search hidden "needle") 'start-char))"#,
    );
    assert_eq!(found, "4");
}

/// `word-search-pattern` gives what `*` searches for on that word: it finds
/// the word standing alone, not inside a longer one.
#[test]
fn a_word_pattern_finds_the_word_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[f]>oobar foo\n");
    assert_eq!(
        probe(&mut ed, tmp.path(), r#"(word-search-pattern bid "foo")"#),
        r"\\bfoo\\b",
        "to-string writes the string with its backslashes escaped"
    );
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "found" "" (lambda (bid)
             (log! 'info (to-string (hash-ref (buffer-search bid (word-search-pattern bid "foo")) 'start-char)))))"#,
    );
    type_cmd(&mut ed, ":found");
    assert_eq!(ed.state.status_msg.as_deref(), Some("7"));
}

/// An edge character that is a word character only through `word-chars` is
/// left unanchored, as `*` leaves it, so the word is still found after a
/// space.
#[test]
fn a_word_pattern_follows_the_buffers_word_chars() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]> $x\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "found" "" (lambda (bid)
             (set-buffer-option! bid "word-chars" "$")
             (log! 'info (to-string (list (word-search-pattern bid "$x")
                                          (hash-ref (buffer-search bid (word-search-pattern bid "$x")) 'start-char))))))"#,
    );
    type_cmd(&mut ed, ":found");
    assert_eq!(ed.state.status_msg.as_deref(), Some(r#"("\\$x\\b" 2)"#));
}

#[test]
fn an_empty_word_raises() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "empty" "" (lambda (bid) (word-search-pattern bid "")))"#,
    );
    type_cmd(&mut ed, ":empty");
    assert!(
        has_error(&ed, "word-search-pattern"),
        "got {:?}",
        error_log(&ed)
    );
}
