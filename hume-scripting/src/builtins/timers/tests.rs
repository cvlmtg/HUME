use super::*;
use crate::test_support::SteelCtxTestHarness;
use steel::rvals::IntoSteelVal;

/// `NullHost` has no timer capability, so `after!` raises the shared
/// "not supported by this host" error rather than return a meaningless id.
#[test]
fn after_errors_when_the_host_has_no_timer_support() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let msg = after(
        &mut ctx,
        100i64.into_steelval().unwrap(),
        crate::Rooted::new(SteelVal::BoolV(true)),
    )
    .unwrap_err()
    .to_string();
    assert!(msg.contains("not supported by this host"), "got: {msg}");
}

#[test]
fn after_rejects_a_negative_ms_argument() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = after(
        &mut ctx,
        (-1i64).into_steelval().unwrap(),
        crate::Rooted::new(SteelVal::BoolV(true)),
    );
    assert!(result.is_err());
}

/// A host with no timer capability has no timer to cancel either, so
/// `cancel-timer!` raises like `after!`.
#[test]
fn cancel_timer_errors_when_the_host_has_no_timer_support() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let msg = cancel_timer(
        &mut ctx,
        crate::host::TimerHandle::from_raw(0).into_steel_val(),
    )
    .unwrap_err()
    .to_string();
    assert!(msg.contains("not supported by this host"), "got: {msg}");
}
