use super::*;
use crate::test_support::SteelCtxTestHarness;
use steel::rvals::IntoSteelVal;

/// `NullHost`'s default `schedule_timer` returns `None` (no timer wheel
/// to schedule onto) — `after` must surface that as an error, not
/// silently return a meaningless id.
#[test]
fn after_errors_when_the_host_has_no_timer_support() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = after(
        &mut ctx,
        100i64.into_steelval().unwrap(),
        SteelVal::BoolV(true),
    );
    assert!(result.is_err());
}

#[test]
fn after_rejects_a_negative_ms_argument() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = after(
        &mut ctx,
        (-1i64).into_steelval().unwrap(),
        SteelVal::BoolV(true),
    );
    assert!(result.is_err());
}

#[test]
fn cancel_timer_is_a_harmless_no_op_against_nullhost() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = cancel_timer(&mut ctx, 0i64.into_steelval().unwrap());
    assert!(result.is_ok());
}

#[test]
fn after_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    let result = super::super::errors::require_cmd(&h.ctx_init(), "after");
    assert!(result.is_err());
}
