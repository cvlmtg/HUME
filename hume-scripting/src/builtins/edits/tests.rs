use super::*;
use crate::builtins::args::symbol_hash;
use crate::json::JsonHandle;
use crate::test_support::{SteelCtxTestHarness, default_pane, str_val};
use hume_rope::position_encoding::PositionEncoding;
use steel::rvals::IntoSteelVal as _;

/// `insert_key` on a host with no `EditHost` capability surfaces
/// `require_cap`'s canonical message, same contract as every other
/// `EditHost` builtin in this file. Also proves the key spec decoded fine
/// (a bad spec would fail before ever reaching `require_cap`, with a
/// different message; see the next test).
#[test]
fn insert_key_without_edit_host_is_unsupported() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let key: SteelVal = "tab".to_string().into_steelval().unwrap();
    let err = insert_key(&mut ctx, default_pane(), key).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("not supported by this host"), "got: {msg}");
}

/// `paste_text` on a host with no `EditHost` capability surfaces
/// `require_cap`'s message.
#[test]
fn paste_text_without_edit_host_is_unsupported() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let text: SteelVal = "x".to_string().into_steelval().unwrap();
    let msg = paste_text(&mut ctx, default_pane(), text)
        .unwrap_err()
        .to_string();
    assert!(msg.contains("not supported by this host"), "got: {msg}");
}

#[test]
fn paste_text_rejects_a_value_that_is_neither_text_nor_a_list() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let msg = paste_text(&mut ctx, default_pane(), SteelVal::IntV(5))
        .unwrap_err()
        .to_string();
    assert!(
        msg.contains("text: expected a string or a list of strings"),
        "got: {msg}"
    );
}

/// A key spec naming a sequence rather than one chord is rejected before
/// ever reaching the host: `insert-key!` has nowhere to send a second key.
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
/// the harness default) surfaces `require_cap`'s message.
#[test]
fn apply_text_edits_without_edit_host_is_unsupported() {
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
}

/// A server-tagged `JsonHandle` argument must reach `require_cap`'s "no
/// host" error, not an argument-type or encoding error, proving
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
}

/// A hand-built hashmap is not a handle onto a server response, so it is
/// rejected before ever reaching the host, the same discipline
/// `apply-text-edits!` applies to a hand-built tuple.
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
    assert!(
        msg.contains("expected a JSON handle from a server response"),
        "got: {msg}"
    );
    assert!(
        !msg.contains("not supported by this host"),
        "must fail on the missing handle before ever reaching the host; got: {msg}"
    );
}

fn default_from() -> SteelVal {
    super::super::ids::SteelPane::new(default_pane()).into_steel_val()
}

/// A server-tagged `JsonHandle` onto a Location must dispatch through the
/// same path a hashmap does, not the `(hash 'target 'line 'char-col)` record
/// path, proven by reaching "no host" instead of a shape or encoding error.
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
}

/// Neither shape (hashmap/handle vs. list) is silently accepted as the
/// other: an out-of-place scalar still raises the shape error.
#[test]
fn goto_location_rejects_a_bare_scalar() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let msg = goto_location(&mut ctx, default_pane(), SteelVal::IntV(5))
        .unwrap_err()
        .to_string();
    assert!(msg.contains("expected a Location"), "got: {msg}");
}

/// A wire `Location` decodes with its own tagged producing-server encoding.
/// A hand-built (untagged) hashmap has no server to have negotiated one
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

/// The `(hash 'target pane 'char-offset)` and `(hash 'target path 'line
/// 'char-col)` shapes never touch server encoding at all, and reach the host
/// regardless.
#[test]
fn goto_location_char_indexed_shapes_never_touch_encoding() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let pane_target = symbol_hash([
        ("target", default_from()),
        ("char-offset", SteelVal::IntV(0)),
    ]);
    let path_target = symbol_hash([
        ("target", SteelVal::StringV("/tmp/a.txt".into())),
        ("line", SteelVal::IntV(0)),
        ("char-col", SteelVal::IntV(0)),
    ]);
    for target in [pane_target, path_target] {
        let msg = goto_location(&mut ctx, default_pane(), target)
            .unwrap_err()
            .to_string();
        assert!(msg.contains("not supported by this host"), "got: {msg}");
    }
}

/// A pane target is positioned by `'char-offset` alone; the keys of a path
/// target raise rather than being ignored or converted.
#[test]
fn goto_location_pane_target_rejects_line_and_char_col() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    for key in ["line", "char-col"] {
        let target = symbol_hash([
            ("target", default_from()),
            ("char-offset", SteelVal::IntV(0)),
            (key, SteelVal::IntV(0)),
        ]);
        let msg = goto_location(&mut ctx, default_pane(), target)
            .unwrap_err()
            .to_string();
        assert!(
            msg.contains("a pane target takes 'char-offset") && msg.contains(key),
            "got: {msg}"
        );
    }
}

/// A path target is positioned by `'line` and `'char-col`; a char offset
/// into a file that may not be open names nothing.
#[test]
fn goto_location_path_target_rejects_char_offset() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let target = symbol_hash([
        ("target", SteelVal::StringV("/tmp/a.txt".into())),
        ("char-offset", SteelVal::IntV(0)),
    ]);
    let msg = goto_location(&mut ctx, default_pane(), target)
        .unwrap_err()
        .to_string();
    assert!(
        msg.contains("a path target takes 'line and 'char-col"),
        "got: {msg}"
    );
}

fn goto_path_target_error(path: &str) -> String {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let target = symbol_hash([
        ("target", SteelVal::StringV(path.into())),
        ("line", SteelVal::IntV(0)),
        ("char-col", SteelVal::IntV(0)),
    ]);
    goto_location(&mut ctx, default_pane(), target)
        .unwrap_err()
        .to_string()
}

/// A URI string is not a path: opened as a path it would be a new empty
/// buffer named after the URI, so a target with any URI scheme raises before
/// the host is asked.
#[test]
fn goto_location_rejects_a_uri_target() {
    for uri in [
        "file:///tmp/a.txt",
        "FILE:///tmp/a.txt",
        "jdt://contents/a.class",
        "deno:/https/x.ts",
        "Foo+bar.baz-1:x",
    ] {
        let msg = goto_path_target_error(uri);
        assert!(msg.contains("not the URI"), "{uri}: got: {msg}");
        assert!(
            !msg.contains("not supported by this host"),
            "{uri}: must raise before reaching the host; got: {msg}"
        );
    }
}

/// A Windows drive letter and a colon past the first path segment are not a
/// URI scheme, so these targets reach the host.
#[test]
fn goto_location_accepts_paths_that_hold_a_colon() {
    for path in [
        r"C:\tmp\a.txt",
        "c:/tmp/a.txt",
        "./foo:bar/a.txt",
        "/tmp/a:b.txt",
    ] {
        let msg = goto_path_target_error(path);
        assert!(
            msg.contains("not supported by this host"),
            "{path}: must reach the host; got: {msg}"
        );
    }
}

fn edit_entry(entry: Vec<(&'static str, SteelVal)>) -> SteelVal {
    vec![symbol_hash(entry)].into_steelval().unwrap()
}

fn edit_text_error(edits: SteelVal) -> String {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    edit_text(&mut ctx, default_pane(), edits)
        .unwrap_err()
        .to_string()
}

/// A well-formed edit decodes and reaches the host, which this harness
/// doesn't have.
#[test]
fn edit_text_without_edit_host_is_unsupported() {
    let msg = edit_text_error(edit_entry(vec![
        ("start-char", SteelVal::IntV(0)),
        ("end-char", SteelVal::IntV(2)),
        ("text", str_val("ab")),
    ]));
    assert!(msg.contains("not supported by this host"), "got: {msg}");
}

#[test]
fn edit_text_rejects_a_value_that_is_not_a_list() {
    let msg = edit_text_error(SteelVal::IntV(5));
    assert!(msg.contains("edits: expected a list"), "got: {msg}");
    assert!(!msg.contains("not supported by this host"), "got: {msg}");
}

#[test]
fn edit_text_rejects_an_entry_without_text() {
    let msg = edit_text_error(edit_entry(vec![
        ("start-char", SteelVal::IntV(0)),
        ("end-char", SteelVal::IntV(0)),
    ]));
    assert!(msg.contains("edits: missing 'text"), "got: {msg}");
}

#[test]
fn edit_text_rejects_an_unknown_key() {
    let msg = edit_text_error(edit_entry(vec![
        ("start-char", SteelVal::IntV(0)),
        ("end", SteelVal::IntV(0)),
        ("text", str_val("")),
    ]));
    assert!(msg.contains("edits: unknown key 'end"), "got: {msg}");
    assert!(!msg.contains("not supported by this host"), "got: {msg}");
}

#[test]
fn edit_text_rejects_a_negative_offset() {
    let msg = edit_text_error(edit_entry(vec![
        ("start-char", SteelVal::IntV(-1)),
        ("end-char", SteelVal::IntV(0)),
        ("text", str_val("")),
    ]));
    assert!(
        msg.contains("edits 'start-char: expected a non-negative integer"),
        "got: {msg}"
    );
}

#[test]
fn edit_text_rejects_text_that_is_not_a_string() {
    let msg = edit_text_error(edit_entry(vec![
        ("start-char", SteelVal::IntV(0)),
        ("end-char", SteelVal::IntV(0)),
        ("text", SteelVal::IntV(1)),
    ]));
    assert!(msg.contains("edits 'text: expected a string"), "got: {msg}");
}
