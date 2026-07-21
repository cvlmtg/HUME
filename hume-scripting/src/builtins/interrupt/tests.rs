use super::*;
use crate::test_support::SteelCtxTestHarness;
use std::sync::atomic::Ordering;

/// `hume_yield` returns `Void` when the interrupt flag is clear.
///
/// Fail oracle: always fire the stop! regardless of the flag → this test errors.
#[test]
fn hume_yield_returns_void_when_flag_clear() {
    let mut h = SteelCtxTestHarness::new();
    // Flag starts `false` (clear) — yield must be a transparent no-op.
    let mut ctx = h.ctx();
    let result = hume_yield(&mut ctx);
    assert!(result.is_ok(), "yield with clear flag must return Ok");
    assert!(
        matches!(result.unwrap(), SteelVal::Void),
        "yield must return Void"
    );
}

/// `hume_yield` raises a Steel error when the interrupt flag is set.
///
/// Fail oracle: remove the flag check → the error is never raised → test fails.
#[test]
fn hume_yield_errors_when_flag_set() {
    let mut h = SteelCtxTestHarness::new();
    h.interrupt_flag.store(true, Ordering::Relaxed);
    let mut ctx = h.ctx();
    let result = hume_yield(&mut ctx);
    assert!(result.is_err(), "yield with set flag must return Err");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("interrupted"),
        "error must mention 'interrupted'; got: {msg}"
    );
}
