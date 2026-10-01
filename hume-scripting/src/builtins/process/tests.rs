use super::*;
use crate::null_host::RecordingInlineOutputHost;
use crate::test_support::SteelCtxTestHarness;
use std::fs;
use tempfile::TempDir;

// ── run_inline_output (%run-inline-output!) ─────────────────────────────

fn list_val(items: &[&str]) -> SteelVal {
    crate::builtins::args::string_list(items.iter().map(|s| s.to_string()))
}

#[test]
fn run_inline_output_missing_binary_raises() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let err = run_inline_output(
        &mut ctx,
        "definitely-not-a-real-binary-xyz".to_string(),
        list_val(&[]),
        SteelVal::BoolV(false),
        list_val(&[]),
    )
    .unwrap_err();
    assert!(err.to_string().contains("definitely-not-a-real-binary-xyz"));
}

/// The spawned process inherits stdio, so the bracket must open before the
/// spawn attempt, even when the spawn itself then fails.
#[test]
fn run_inline_output_calls_ensure_before_spawn() {
    let mut host = RecordingInlineOutputHost::default();
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_with_host(&mut host);
    let _ = run_inline_output(
        &mut ctx,
        "definitely-not-a-real-binary-xyz".to_string(),
        list_val(&[]),
        SteelVal::BoolV(false),
        list_val(&[]),
    );
    drop(ctx);
    assert_eq!(host.ensure_calls, 1);
}

#[cfg(unix)]
mod unix;
