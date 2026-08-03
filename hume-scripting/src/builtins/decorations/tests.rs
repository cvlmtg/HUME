use super::*;
use crate::test_support::SteelCtxTestHarness;
use hume_engine::pipeline::BufferId;
use steel::rvals::IntoSteelVal as _;

/// Every decoration setter on a host with no `DecorationHost` capability
/// (`NullHost`, the harness default) surfaces `require_cap`'s canonical
/// message, naming the builtin — locks the message contract `require_cap`
/// centralizes across `decorations.rs`/`edits.rs`/`completion.rs`/`ui.rs`.
///
/// Fail oracle: revert a setter to `if let Some(decorations) = ... { ... }
/// Ok(Void)` — the write silently no-ops instead of erroring, and this
/// assert fires because `.unwrap_err()` panics on an `Ok`.
fn assert_names_builtin(result: SteelResult, builtin: &str) {
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("not supported by this host"), "got: {msg}");
    assert!(msg.contains(builtin), "got: {msg}");
}

#[test]
fn set_inlay_hints_without_decoration_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let empty: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let result = set_inlay_hints(&mut ctx, BidArg(BufferId::default()), empty);
    assert_names_builtin(result, "set-inlay-hints!");
}

#[test]
fn set_signs_without_decoration_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let empty: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let result = set_signs(
        &mut ctx,
        SteelVal::StringV("test".into()),
        BidArg(BufferId::default()),
        empty,
    );
    assert_names_builtin(result, "set-signs!");
}

#[test]
fn set_virtual_lines_without_decoration_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let empty: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let result = set_virtual_lines(
        &mut ctx,
        SteelVal::StringV("test".into()),
        BidArg(BufferId::default()),
        empty,
    );
    assert_names_builtin(result, "set-virtual-lines!");
}

#[test]
fn set_inline_diagnostics_without_decoration_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let empty: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let result = set_inline_diagnostics(&mut ctx, BidArg(BufferId::default()), empty);
    assert_names_builtin(result, "set-inline-diagnostics!");
}

#[test]
fn set_extra_highlights_without_decoration_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let empty: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let result = set_extra_highlights(
        &mut ctx,
        SteelVal::StringV("test".into()),
        BidArg(BufferId::default()),
        empty,
    );
    assert_names_builtin(result, "set-extra-highlights!");
}
