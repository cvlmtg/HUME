use super::*;
use crate::null_host::RecordingInlineOutputHost;
use crate::test_support::SteelCtxTestHarness;
use std::fs;
use steel::rerrs::SteelErr;
use tempfile::TempDir;

// ── run_inline_output (%run-inline-output!) ─────────────────────────────

fn list_val(items: &[&str]) -> SteelVal {
    crate::builtins::args::string_list(items.iter().map(|s| s.to_string()))
}

/// `run_inline_output` on a host whose inline-output frame is open when
/// `frame_open`, for a program that does not exist.
fn run_missing_program(frame_open: bool) -> (Result<SteelVal, SteelErr>, usize) {
    let mut host = RecordingInlineOutputHost::with_frame_open(frame_open);
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_with_host(&mut host);
    let result = run_inline_output(
        &mut ctx,
        "definitely-not-a-real-binary-xyz".to_string(),
        list_val(&[]),
        SteelVal::BoolV(false),
        list_val(&[]),
    );
    drop(ctx);
    (result, host.claims)
}

#[test]
fn run_inline_output_missing_binary_raises() {
    let (result, _) = run_missing_program(true);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("definitely-not-a-real-binary-xyz")
    );
}

/// The spawned process inherits stdio, so the bracket must open before the
/// spawn attempt, even when the spawn itself then fails.
#[test]
fn run_inline_output_claims_the_terminal_before_spawn() {
    let (_, claims) = run_missing_program(true);
    assert_eq!(claims, 1);
}

/// Outside an `#:inline-output` command the call raises before it spawns
/// anything.
#[test]
fn run_inline_output_raises_outside_an_inline_output_command() {
    let (result, claims) = run_missing_program(false);
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("run-inline-output!: runs only inside an #:inline-output command"),
        "{err}"
    );
    assert_eq!(claims, 0);
}

#[cfg(unix)]
mod unix;

/// `cancel-async!` raises on a host with no async-process capability, which
/// has no job to cancel, like `spawn-async!` does.
#[test]
fn cancel_async_without_async_process_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let msg = cancel_async(
        &mut ctx,
        crate::host::JobHandle::from_raw(1).into_steel_val(),
    )
    .unwrap_err()
    .to_string();
    assert!(
        msg.contains("cancel-async!: not supported by this host"),
        "got: {msg}"
    );
}
