use super::*;
use crate::test_support::SteelCtxTestHarness;
use hume_engine::pipeline::BufferId;
use steel::rvals::IntoSteelVal as _;

/// `apply_text_edits` on a host with no `EditHost` capability (`NullHost`,
/// the harness default) surfaces `require_cap`'s canonical message,
/// naming the builtin — locks the message contract `require_cap`
/// centralizes across `edits.rs`/`completion.rs`/`ui.rs`.
///
/// Fail oracle: `require_cap` drops the `name` interpolation → the
/// second assert fires (message no longer identifies the builtin).
#[test]
fn apply_text_edits_without_edit_host_names_the_builtin() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let empty_edits: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let err = apply_text_edits(
        &mut ctx,
        BidArg(BufferId::default()),
        empty_edits,
        SteelVal::BoolV(false),
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("not supported by this host"), "got: {msg}");
    assert!(msg.contains("apply-text-edits!"), "got: {msg}");
}
