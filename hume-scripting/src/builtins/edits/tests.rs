use super::*;
use crate::json::JsonHandle;
use crate::test_support::{SteelCtxTestHarness, default_pane};
use hume_rope::position_encoding::PositionEncoding;
use steel::rvals::IntoSteelVal as _;

/// `insert_key` on a host with no `EditHost` capability surfaces
/// `require_cap`'s canonical message — same contract as every other
/// `EditHost` builtin in this file. Also proves the key spec decoded fine
/// (a bad spec would fail before ever reaching `require_cap`, with a
/// different message — see the next test).
#[test]
fn insert_key_without_edit_host_names_the_builtin() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let key: SteelVal = "tab".to_string().into_steelval().unwrap();
    let err = insert_key(&mut ctx, default_pane(), key).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("not supported by this host"), "got: {msg}");
    assert!(msg.contains("insert-key!"), "got: {msg}");
}

/// A key spec naming a sequence rather than one chord is rejected before
/// ever reaching the host — `insert-key!` has nowhere to send a second key.
#[test]
fn insert_key_rejects_a_multi_key_spec() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let key: SteelVal = "g h".to_string().into_steelval().unwrap();
    let msg = insert_key(&mut ctx, default_pane(), key)
        .unwrap_err()
        .to_string();
    assert!(msg.contains("exactly one key"), "got: {msg}");
}

/// `apply_text_edits` on a host with no `EditHost` capability (`NullHost`,
/// the harness default) surfaces `require_cap`'s canonical message,
/// naming the builtin — locks the message contract `require_cap`
/// centralizes across `edits.rs`/`completion.rs`/`ui.rs`.
#[test]
fn apply_text_edits_without_edit_host_names_the_builtin() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let empty_edits: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let err = apply_text_edits(
        &mut ctx,
        default_pane(),
        empty_edits,
        SteelVal::BoolV(false),
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("not supported by this host"), "got: {msg}");
    assert!(msg.contains("apply-text-edits!"), "got: {msg}");
}

/// A server-tagged `JsonHandle` argument must reach `require_cap`'s "no
/// host" error, not an argument-type or encoding error — proving
/// `apply_workspace_edit` decoded it as JSON and resolved its encoding
/// before ever needing the host.
#[test]
fn apply_workspace_edit_accepts_a_tagged_json_handle() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let handle =
        JsonHandle::server_for_test(serde_json::json!({"changes": {}}), PositionEncoding::Utf16)
            .into_steel_val();
    let msg = apply_workspace_edit(&mut ctx, default_pane(), handle, SteelVal::BoolV(false))
        .unwrap_err()
        .to_string();
    assert!(msg.contains("not supported by this host"), "got: {msg}");
    assert!(msg.contains("apply-workspace-edit!"), "got: {msg}");
}

/// A hand-built hashmap has no producing server to have negotiated an
/// encoding with — rejected before ever reaching the host, the same
/// discipline `apply-text-edits!` applies to a hand-built tuple.
#[test]
fn apply_workspace_edit_rejects_a_hand_built_hashmap() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let mut hm = steel::HashMap::new();
    hm.insert(SteelVal::StringV("changes".into()), SteelVal::Void);
    let wsedit = SteelVal::HashMapV(steel::gc::Gc::new(hm).into());
    let msg = apply_workspace_edit(&mut ctx, default_pane(), wsedit, SteelVal::BoolV(false))
        .unwrap_err()
        .to_string();
    assert!(msg.contains("not a value from an LSP server"), "got: {msg}");
    assert!(
        !msg.contains("not supported by this host"),
        "must fail on the missing encoding before ever reaching the host; got: {msg}"
    );
}

fn default_from() -> SteelVal {
    super::super::ids::SteelPane::new(default_pane()).into_steel_val()
}

/// A server-tagged `JsonHandle` onto a Location must dispatch through the
/// same path a hashmap does, not the `(list target line char-col)` tuple
/// path — proven by reaching "no host" instead of a shape or encoding error.
#[test]
fn goto_location_accepts_a_tagged_json_handle() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let handle = JsonHandle::server_for_test(
        serde_json::json!({
            "uri": "file:///a.rs",
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
        }),
        PositionEncoding::Utf16,
    )
    .into_steel_val();
    let msg = goto_location(&mut ctx, default_pane(), handle)
        .unwrap_err()
        .to_string();
    assert!(msg.contains("not supported by this host"), "got: {msg}");
    assert!(msg.contains("goto-location!"), "got: {msg}");
}

/// Neither shape (hashmap/handle vs. list) is silently accepted as the
/// other — an out-of-place scalar still raises the shape error.
#[test]
fn goto_location_rejects_a_bare_scalar() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let msg = goto_location(&mut ctx, default_pane(), SteelVal::IntV(5))
        .unwrap_err()
        .to_string();
    assert!(msg.contains("expected a Location"), "got: {msg}");
}

/// A wire `Location` decodes with its own tagged producing-server encoding
/// — a hand-built (untagged) hashmap has no server to have negotiated one
/// with, so it's rejected before ever reaching the host.
///
/// A default-encoding fallback for untagged handles would let this reach
/// "not supported by this host" instead.
#[test]
fn goto_location_wire_shape_requires_a_tagged_handle() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let handle = JsonHandle::new(serde_json::json!({
        "uri": "file:///a.rs",
        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
    }))
    .into_steel_val();
    let msg = goto_location(&mut ctx, default_pane(), handle)
        .unwrap_err()
        .to_string();
    assert!(msg.contains("not a value from an LSP server"), "got: {msg}");
    assert!(
        !msg.contains("not supported by this host"),
        "must fail on the missing encoding before ever reaching the host; got: {msg}"
    );
}

/// The `(list target line char-col)` shape never touches server encoding at
/// all — reaches the host regardless.
#[test]
fn goto_location_list_shape_never_touches_encoding() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let list = SteelVal::ListV(vec![default_from(), SteelVal::IntV(0), SteelVal::IntV(0)].into());
    let msg = goto_location(&mut ctx, default_pane(), list)
        .unwrap_err()
        .to_string();
    assert!(msg.contains("not supported by this host"), "got: {msg}");
}
