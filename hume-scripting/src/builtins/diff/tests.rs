use super::*;
use crate::builtins::args::{list_of, string_list, symbol_hash};
use crate::test_support::SteelCtxTestHarness;

// ── Gate (init mode rejection) ────────────────────────────────────────────
//
// All diff builtins are `cmd`-gated in `builtins!`'s registration table:
// the gate lives in the registration wrapper closure, not the function
// body, so these test the gate primitive directly rather than calling the
// builtin (its body has no guard to hit).

/// `diff-lines` is blocked in init mode.
///
/// An `open` table entry would make it callable from `init.scm`, where there
/// is no meaningful live state to diff against.
#[test]
fn diff_lines_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    let result = super::super::errors::require_cmd(&h.ctx_init(), "diff-lines");
    assert!(result.is_err(), "diff-lines must error in init mode");
    assert!(result.unwrap_err().to_string().contains("init"));
}

/// `diff-buffer-lines` is blocked in init mode.
#[test]
fn diff_buffer_lines_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "diff-buffer-lines").is_err());
}

/// `diff-words` is blocked in init mode.
#[test]
fn diff_words_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(super::super::errors::require_cmd(&h.ctx_init(), "diff-words").is_err());
}

// ── Type errors ────────────────────────────────────────────────────────────

/// `diff-lines` rejects a non-string argument.
///
/// A hand-rolled `to_string()` coercion in place of `string_arg` would make
/// `(diff-lines 1 "x")` silently diff the literal text `"1"`.
#[test]
fn diff_lines_rejects_a_non_string_argument() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = diff_lines(&mut ctx, SteelVal::IntV(1), SteelVal::StringV("".into()));
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
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = diff_words(&mut ctx, SteelVal::IntV(1), SteelVal::StringV("".into()));
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
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = diff_words(&mut ctx, SteelVal::StringV("".into()), SteelVal::IntV(1));
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("expected a string")
    );
}

// ── NullHost: no DiffHost, buffer_exists=false ──────────────────────────────

fn int(n: isize) -> SteelVal {
    SteelVal::IntV(n)
}

fn word_hunk(old: (isize, isize, &str), new: (isize, isize, &str)) -> SteelVal {
    symbol_hash([
        ("old-start", int(old.0)),
        ("old-end", int(old.1)),
        ("new-start", int(new.0)),
        ("new-end", int(new.1)),
        ("old-text", SteelVal::StringV(old.2.into())),
        ("new-text", SteelVal::StringV(new.2.into())),
    ])
}

fn words_result(hunks: Vec<SteelVal>) -> SteelVal {
    symbol_hash([
        ("hunks", list_of(hunks)),
        ("deadline-hit", SteelVal::BoolV(false)),
    ])
}

fn run_diff_words(old: &str, new: &str) -> SteelVal {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    diff_words(
        &mut ctx,
        SteelVal::StringV(old.into()),
        SteelVal::StringV(new.into()),
    )
    .expect("diff-words needs no host capability")
}

/// `diff-lines` diffs two strings with no editor state, so a host with no
/// diff capability still answers.
#[test]
fn diff_lines_needs_no_host_capability() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = diff_lines(
        &mut ctx,
        SteelVal::StringV("a\n".into()),
        SteelVal::StringV("b\n".into()),
    )
    .expect("diff-lines needs no host capability");
    let expected = list_of([symbol_hash([
        ("old-start", int(0)),
        ("old-count", int(1)),
        ("new-start", int(0)),
        ("new-count", int(1)),
        ("old-lines", string_list(vec!["a".to_owned()])),
        ("new-lines", string_list(vec!["b".to_owned()])),
        (
            "words",
            symbol_hash([("old", list_of([])), ("new", list_of([]))]),
        ),
    ])]);
    assert_eq!(result, expected);
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

/// `diff-buffer-lines` on a host with no `DiffHost` capability raises an
/// error naming the builtin. The pane's liveness is checked at decode time
/// (`LivePane`, in the `builtins!`-registered closure, unreachable from this
/// direct call), so `require_cap` is the first gate this call reaches.
#[test]
fn diff_buffer_lines_reports_an_unsupported_host() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let pane = crate::types::PaneHandle::buffer_only(hume_engine::pipeline::BufferId::default());
    let result = diff_buffer_lines(&mut ctx, pane, SteelVal::StringV("a\n".into()));
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("not supported by this host")
    );
}
