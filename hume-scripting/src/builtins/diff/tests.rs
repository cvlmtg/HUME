use super::*;
use crate::test_support::SteelCtxTestHarness;

// ── Gate (init mode rejection) ────────────────────────────────────────────
//
// Both builtins are `cmd`-gated in `builtins!`'s registration table — the
// gate lives in the registration wrapper closure, not the function body, so
// these test the gate primitive directly rather than calling the builtin
// (its body has no guard to hit).

/// `diff-lines` is blocked in init mode.
///
/// Fail oracle: change `diff-lines`'s table entry from `cmd` to `open` →
/// callable from `init.scm`, where there is no meaningful live state to
/// diff against.
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
/// Fail oracle: hand-roll a `to_string()` coercion instead of `string_arg` —
/// `(diff-lines 1 "x")` would silently diff the literal text `"1"`.
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
/// Fail oracle: hand-roll a `to_string()` coercion instead of `string_arg` —
/// `(diff-words 1 "x")` would silently diff the literal text `"1"`.
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
/// Fail oracle: `ctx.host.diff().map(...).unwrap_or_default()` instead of
/// `require_cap` — a host that cannot diff at all would silently report
/// "no differences" instead of failing.
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

/// `diff-buffer-lines` with a valid-shaped but non-existent buffer id raises
/// an error before ever reaching the diff capability.
///
/// Fail oracle: drop `BidArg::require_live` — the capability-absence error
/// would surface instead of naming the invalid buffer id, and against a
/// real host a closed buffer would return an empty list rather than fail.
#[test]
fn diff_buffer_lines_rejects_an_unknown_buffer_id() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let bid = BidArg(hume_engine::pipeline::BufferId::default());
    let result = diff_buffer_lines(&mut ctx, bid, SteelVal::StringV("a\n".into()));
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("invalid buffer id")
    );
}

/// `diff-words` on a host with no `DiffHost` capability raises an error
/// naming the builtin.
///
/// Fail oracle: `ctx.host.diff().map(...).unwrap_or_default()` instead of
/// `require_cap` — a host that cannot diff at all would silently report
/// "no differences" instead of failing.
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
