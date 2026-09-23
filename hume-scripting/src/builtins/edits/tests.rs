use super::*;
use crate::json::JsonHandle;
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

/// A `JsonHandle` argument must reach `require_cap`'s "no host" error, not
/// an argument-type error — proving `apply_workspace_edit` decoded it as
/// JSON instead of rejecting the custom type.
#[test]
fn apply_workspace_edit_accepts_a_json_handle() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let handle = JsonHandle::new(serde_json::json!({"changes": {}})).into_steel_val();
    let msg = apply_workspace_edit(&mut ctx, handle)
        .unwrap_err()
        .to_string();
    assert!(msg.contains("not supported by this host"), "got: {msg}");
    assert!(msg.contains("apply-workspace-edit!"), "got: {msg}");
}

/// A hand-built hashmap must keep working (the pre-handle shape).
#[test]
fn apply_workspace_edit_accepts_a_hashmap() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let mut hm = steel::HashMap::new();
    hm.insert(SteelVal::StringV("changes".into()), SteelVal::Void);
    let wsedit = SteelVal::HashMapV(steel::gc::Gc::new(hm).into());
    let msg = apply_workspace_edit(&mut ctx, wsedit)
        .unwrap_err()
        .to_string();
    assert!(msg.contains("not supported by this host"), "got: {msg}");
}

/// A `JsonHandle` onto a Location must dispatch through the same path a
/// hashmap does, not the `(list target line char-col)` tuple path — proven
/// by reaching "no host" instead of a shape error.
#[test]
fn goto_location_accepts_a_json_handle() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let handle = JsonHandle::new(serde_json::json!({
        "uri": "file:///a.rs",
        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
    }))
    .into_steel_val();
    let msg = goto_location(&mut ctx, handle).unwrap_err().to_string();
    assert!(msg.contains("not supported by this host"), "got: {msg}");
    assert!(msg.contains("goto-location!"), "got: {msg}");
}

/// Neither shape (hashmap/handle vs. list) is silently accepted as the
/// other — an out-of-place scalar still raises the shape error.
#[test]
fn goto_location_rejects_a_bare_scalar() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let msg = goto_location(&mut ctx, SteelVal::IntV(5))
        .unwrap_err()
        .to_string();
    assert!(msg.contains("expected a Location"), "got: {msg}");
}
