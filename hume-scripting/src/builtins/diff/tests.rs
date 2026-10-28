use super::*;
use crate::builtins::args::{list_of, string_list, symbol_hash};
use crate::test_support::{SteelCtxTestHarness, hash_field};

// ── Type errors ────────────────────────────────────────────────────────────

/// `diff-lines` rejects a non-string argument.
///
/// A hand-rolled `to_string()` coercion in place of `string_arg` would make
/// `(diff-lines 1 "x")` silently diff the literal text `"1"`.
#[test]
fn diff_lines_rejects_a_non_string_argument() {
    let result = diff_lines(SteelVal::IntV(1), SteelVal::StringV("".into()));
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("expected a string")
    );
}

/// `diff-words` rejects a non-string `old` argument.
///
/// Coercing with `to_string()` instead of `string_arg` would silently diff
/// the text `"1"` for `(diff-words 1 "x")`.
#[test]
fn diff_words_rejects_a_non_string_old_argument() {
    let result = diff_words(SteelVal::IntV(1), SteelVal::StringV("".into()));
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("expected a string")
    );
}

/// `diff-words` rejects a non-string `new` argument.
#[test]
fn diff_words_rejects_a_non_string_new_argument() {
    let result = diff_words(SteelVal::StringV("".into()), SteelVal::IntV(1));
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("expected a string")
    );
}

// ── NullHost: no DiffHost, buffer_exists=false ──────────────────────────────

/// A line diff's result: its `hunks`.
fn lines_result(hunks: SteelVal) -> SteelVal {
    symbol_hash([("hunks", hunks)])
}

fn int(n: isize) -> SteelVal {
    SteelVal::IntV(n)
}

fn word_hunk(old: (isize, isize, &str), new: (isize, isize, &str)) -> SteelVal {
    symbol_hash([
        ("old-start-char", int(old.0)),
        ("old-end-char", int(old.1)),
        ("new-start-char", int(new.0)),
        ("new-end-char", int(new.1)),
        ("old-text", SteelVal::StringV(old.2.into())),
        ("new-text", SteelVal::StringV(new.2.into())),
    ])
}

fn words_result(hunks: Vec<SteelVal>) -> SteelVal {
    symbol_hash([("hunks", list_of(hunks))])
}

fn run_diff_words(old: &str, new: &str) -> SteelVal {
    diff_words(SteelVal::StringV(old.into()), SteelVal::StringV(new.into()))
        .expect("two strings diff")
}

/// `diff-lines` reports one hunk for a changed line, with empty word spans
/// since the whole line changed.
#[test]
fn diff_lines_reports_a_whole_line_change() {
    let result = diff_lines(
        SteelVal::StringV("a\n".into()),
        SteelVal::StringV("b\n".into()),
    )
    .expect("two strings diff");
    let expected = lines_result(list_of([symbol_hash([
        ("old-start-line", int(0)),
        ("old-end-line", int(1)),
        ("new-start-line", int(0)),
        ("new-end-line", int(1)),
        ("old-lines", string_list(vec!["a".to_owned()])),
        ("new-lines", string_list(vec!["b".to_owned()])),
        (
            "words",
            symbol_hash([("old", list_of([])), ("new", list_of([]))]),
        ),
    ])]));
    assert_eq!(result, expected);
}

/// `diff-lines` reads its texts with LF line endings, so a new-side span
/// counts each CRLF of `new-text` as one char: `y` is char 6 of
/// "a\nlet y = 1;\n".
#[test]
fn diff_lines_new_side_offsets_count_a_crlf_as_one_char() {
    let result = diff_lines(
        SteelVal::StringV("a\r\nlet x = 1;\r\n".into()),
        SteelVal::StringV("a\r\nlet y = 1;\r\n".into()),
    )
    .expect("two strings diff");
    let expected = lines_result(list_of([symbol_hash([
        ("old-start-line", int(1)),
        ("old-end-line", int(2)),
        ("new-start-line", int(1)),
        ("new-end-line", int(2)),
        ("old-lines", string_list(vec!["let x = 1;".to_owned()])),
        ("new-lines", string_list(vec!["let y = 1;".to_owned()])),
        (
            "words",
            symbol_hash([
                (
                    "old",
                    list_of([symbol_hash([
                        ("line", int(1)),
                        ("start-char-col", int(4)),
                        ("end-char-col", int(5)),
                    ])]),
                ),
                (
                    "new",
                    list_of([symbol_hash([("start-char", int(6)), ("end-char", int(7))])]),
                ),
            ]),
        ),
    ])]));
    assert_eq!(result, expected);
}

/// A hunk whose word diff was skipped for the budget reads `'words #f`, not
/// the empty spans of a change of whole lines.
#[test]
fn a_hunk_over_the_word_budget_reads_words_false() {
    let hunk = hume_editing::hunk::ChangeHunk {
        old_start: hume_rope::line::ContentLine::new(0),
        new_start: hume_rope::line::ContentLine::new(0),
        old_lines: vec!["a".to_owned()],
        new_lines: vec!["b".to_owned()],
        words: None,
    };
    assert_eq!(
        hash_field(&hunk_to_steel(hunk), "words"),
        SteelVal::BoolV(false)
    );
}

/// A changed word carries its text on both sides, at char offsets.
#[test]
fn diff_words_replace_carries_both_sides() {
    assert_eq!(
        run_diff_words("foo bar", "foo baz"),
        words_result(vec![word_hunk((4, 7, "bar"), (4, 7, "baz"))])
    );
}

/// A pure insert has a zero-width old side with empty text, the anchor a
/// plugin draws its insert marker at.
#[test]
fn diff_words_pure_insert_has_zero_width_old_side() {
    assert_eq!(
        run_diff_words("foo bar", "foo big bar"),
        words_result(vec![word_hunk((4, 4, ""), (4, 8, "big "))])
    );
}

#[test]
fn diff_words_pure_delete_has_zero_width_new_side() {
    assert_eq!(
        run_diff_words("foo big bar", "foo bar"),
        words_result(vec![word_hunk((4, 8, "big "), (4, 4, ""))])
    );
}

#[test]
fn diff_words_drops_equal_runs() {
    assert_eq!(
        run_diff_words("foo bar", "foo bar"),
        words_result(Vec::new())
    );
}

/// `diff-buffer-lines` on a host with no `DiffHost` capability raises. The pane's liveness is checked at resolve time
/// (`LiveBuffer`, in the `builtins!`-registered closure, unreachable from this
/// direct call), so `require_cap` is the first gate this call reaches.
#[test]
fn diff_buffer_lines_reports_an_unsupported_host() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = diff_buffer_lines(
        &mut ctx,
        hume_core::BufferId::default(),
        SteelVal::StringV("a\n".into()),
    );
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("not supported by this host")
    );
}
