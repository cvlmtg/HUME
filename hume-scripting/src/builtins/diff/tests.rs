use super::*;
use crate::test_support::SteelCtxTestHarness;

// ── Gate (init mode rejection) ────────────────────────────────────────────
//
// All three builtins are `cmd`-gated in `builtins!`'s registration table —
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

// ── Capability / buffer-id errors (NullHost: no DiffHost, buffer_exists=false) ──

/// `diff-lines` on a host with no `DiffHost` capability raises an error
/// naming the builtin.
///
/// Falling back with `ctx.host.diff().map(...).unwrap_or_default()` would
/// have a host that cannot diff at all report "no differences" instead of
/// failing.
#[test]
fn diff_lines_reports_an_unsupported_host() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = diff_lines(
        &mut ctx,
        SteelVal::StringV("a\n".into()),
        SteelVal::StringV("b\n".into()),
    );
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("not supported by this host")
    );
}

/// `diff-buffer-lines` on a host with no `DiffHost` capability raises an
/// error naming the builtin, same as `diff-lines`/`diff-words` — `bid`'s
/// own liveness is already checked at decode time (`LiveBid`, in the
/// `builtins!`-registered closure, unreachable from this direct call), so
/// `require_cap` is the first gate this call actually reaches.
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

/// `diff-words` on a host with no `DiffHost` capability raises an error
/// naming the builtin.
///
/// A silent `unwrap_or_default()` fallback here would report "no
/// differences" on a host that cannot diff.
#[test]
fn diff_words_reports_an_unsupported_host() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = diff_words(
        &mut ctx,
        SteelVal::StringV("foo bar".into()),
        SteelVal::StringV("foo baz".into()),
    );
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("not supported by this host")
    );
}
