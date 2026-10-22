use steel::rvals::IntoSteelVal as _;

use super::*;
use crate::builtins::args::cons_pair;

/// A `JsonHandle` onto a wire `TextEdit` (an unconverted element straight
/// from a `textDocument/formatting`-shaped response) decodes, and carries
/// its tagged server encoding forward.
#[test]
fn wire_text_edit_arg_decodes_a_json_handle() {
    let val = crate::json::JsonHandle::server_for_test(
        serde_json::json!({
            "range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 5}},
            "newText": "abc",
        }),
        hume_rope::position_encoding::PositionEncoding::Utf8,
    )
    .into_steel_val();
    let edit = wire_text_edit_arg(val).unwrap();
    assert_eq!((edit.range.start.line, edit.range.start.character), (1, 2));
    assert_eq!((edit.range.end.line, edit.range.end.character), (1, 5));
    assert_eq!(edit.new_text, "abc");
    assert_eq!(
        edit.encoding,
        hume_rope::position_encoding::PositionEncoding::Utf8
    );
}

/// A handle missing `range`/`newText` raises naming the missing field.
#[test]
fn wire_text_edit_arg_rejects_a_malformed_handle() {
    let val = crate::json::JsonHandle::server_for_test(
        serde_json::json!({"range": {}}),
        hume_rope::position_encoding::PositionEncoding::Utf16,
    )
    .into_steel_val();
    let err = wire_text_edit_arg(val).unwrap_err();
    assert!(err.to_string().contains("apply-text-edits!"), "got: {err}");
}

/// Not a `JsonHandle` at all (the removed hand-built tuple shape, or any
/// other scalar) is rejected outright, not silently decoded as if it had no
/// encoding.
#[test]
fn wire_text_edit_arg_rejects_a_non_handle() {
    let val: SteelVal = vec![
        cons_pair(SteelVal::IntV(0), SteelVal::IntV(0)).unwrap(),
        cons_pair(SteelVal::IntV(0), SteelVal::IntV(3)).unwrap(),
        SteelVal::StringV("abc".into()),
    ]
    .into_steelval()
    .unwrap();
    let err = wire_text_edit_arg(val).unwrap_err();
    assert!(err.to_string().contains("JSON handle"), "got: {err}");
}

/// A hand-built (`JsonHandle::new`, untagged) handle is rejected the same
/// way: it decodes as JSON fine, but has no server to read an encoding
/// from.
#[test]
fn wire_text_edit_arg_rejects_an_untagged_handle() {
    let val = crate::json::JsonHandle::new(serde_json::json!({
        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
        "newText": "abc",
    }))
    .into_steel_val();
    let err = wire_text_edit_arg(val).unwrap_err();
    assert!(
        err.to_string().contains("not a value from an LSP server"),
        "got: {err}"
    );
}
