use super::*;
use steel::rvals::IntoSteelVal as _;

fn list_of(items: &[&str]) -> SteelVal {
    items
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .into_steelval()
        .unwrap()
}

// ── Plain decoders ────────────────────────────────────────────────────────

#[test]
fn string_arg_accepts_string_and_symbol() {
    assert_eq!(string_arg(SteelVal::StringV("x".into()), "f").unwrap(), "x");
    assert_eq!(string_arg(SteelVal::SymbolV("x".into()), "f").unwrap(), "x");
}

#[test]
fn string_arg_rejects_wrong_type_naming_the_arg() {
    let err = string_arg(SteelVal::IntV(1), "buffer-name").unwrap_err();
    assert!(err.to_string().contains("buffer-name"), "got: {err}");
    assert!(err.to_string().contains("expected a string"), "got: {err}");
}

#[test]
fn optional_string_arg_false_is_none() {
    assert_eq!(
        optional_string_arg(SteelVal::BoolV(false), "f").unwrap(),
        None
    );
}

#[test]
fn optional_symbol_arg_false_is_none_symbol_is_some_string_is_rejected() {
    assert_eq!(
        optional_symbol_arg(SteelVal::BoolV(false), "f").unwrap(),
        None
    );
    assert_eq!(
        optional_symbol_arg(SteelVal::SymbolV("error".into()), "f").unwrap(),
        Some("error".to_string())
    );
    let err = optional_symbol_arg(SteelVal::StringV("error".into()), "f").unwrap_err();
    assert!(
        err.to_string().contains("expected a symbol or #f"),
        "got: {err}"
    );
}

#[test]
fn usize_arg_rejects_negative() {
    assert!(usize_arg(SteelVal::IntV(-1), "f").is_err());
}

#[test]
fn bool_arg_accepts_both_bools() {
    assert!(bool_arg(SteelVal::BoolV(true), "f").unwrap());
    assert!(!bool_arg(SteelVal::BoolV(false), "f").unwrap());
}

#[test]
fn bool_arg_rejects_wrong_type_naming_the_arg() {
    let err = bool_arg(SteelVal::IntV(1), "dismiss-on-key").unwrap_err();
    assert!(err.to_string().contains("dismiss-on-key"), "got: {err}");
    assert!(err.to_string().contains("expected a bool"), "got: {err}");
}

#[test]
fn optional_usize_arg_false_is_none_some_is_some() {
    assert_eq!(
        optional_usize_arg(SteelVal::BoolV(false), "f").unwrap(),
        None
    );
    assert_eq!(optional_usize_arg(SteelVal::IntV(4), "f").unwrap(), Some(4));
}

#[test]
fn list_to_strings_rejects_non_string_element() {
    let list: SteelVal = vec![SteelVal::StringV("a".into()), SteelVal::IntV(1)]
        .into_steelval()
        .unwrap();
    assert!(list_to_strings(list, "f").is_err());
}

#[test]
fn chars_arg_rejects_multi_char_entry() {
    let err = chars_arg(list_of(&["ab"]), "chars").unwrap_err();
    assert!(
        err.to_string().contains("exactly one character"),
        "got: {err}"
    );
}

#[test]
fn chars_arg_accepts_single_char_entries() {
    assert_eq!(
        chars_arg(list_of(&["a", "b"]), "chars").unwrap(),
        vec!['a', 'b']
    );
}

#[test]
fn json_params_rejects_bool() {
    let err = json_params(SteelVal::BoolV(true), "lsp-request params").unwrap_err();
    assert!(err.to_string().contains("lsp-request params"), "got: {err}");
}

// ── checked_fields / tuple_list ──────────────────────────────────────────

#[test]
fn checked_fields_rejects_wrong_arity() {
    let err = checked_fields(list_of(&["a"]), "f", 2..=2, "(a b)").unwrap_err();
    assert!(
        err.to_string().contains("each entry must be (a b)"),
        "got: {err}"
    );
}

#[test]
fn checked_fields_accepts_arity_within_range() {
    assert_eq!(
        checked_fields(list_of(&["a", "b"]), "f", 2..=3, "(a b [c])")
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        checked_fields(list_of(&["a", "b", "c"]), "f", 2..=3, "(a b [c])")
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn tuple_list_decodes_each_entry_via_row() {
    let entries: SteelVal = vec![list_of(&["a", "b"]), list_of(&["c", "d"])]
        .into_steelval()
        .unwrap();
    let out = tuple_list(entries, "f", 2..=2, "(a b)", |fields| {
        Ok((
            string_arg(fields[0].clone(), "f")?,
            string_arg(fields[1].clone(), "f")?,
        ))
    })
    .unwrap();
    assert_eq!(
        out,
        vec![
            ("a".to_string(), "b".to_string()),
            ("c".to_string(), "d".to_string())
        ]
    );
}

#[test]
fn tuple_list_propagates_a_bad_entry_arity() {
    let entries: SteelVal = vec![list_of(&["a"])].into_steelval().unwrap();
    let result = tuple_list(entries, "f", 2..=2, "(a b)", |fields| {
        string_arg(fields[0].clone(), "f")
    });
    assert!(result.is_err());
}

// ── pair_fields / cons_pair ──────────────────────────────────────────────

#[test]
fn cons_pair_then_pair_fields_round_trips() {
    let pair = cons_pair(SteelVal::IntV(3), SteelVal::IntV(7)).unwrap();
    let (car, cdr) = pair_fields(pair, "f", "(a . b)").unwrap();
    assert_eq!(car, SteelVal::IntV(3));
    assert_eq!(cdr, SteelVal::IntV(7));
}

#[test]
fn pair_fields_rejects_proper_list() {
    let err = pair_fields(list_of(&["a", "b"]), "position", "(line . character)").unwrap_err();
    assert!(err.to_string().contains("(line . character)"), "got: {err}");
}

#[test]
fn pair_fields_rejects_non_pair_scalar() {
    let err = pair_fields(SteelVal::IntV(3), "position", "(line . character)").unwrap_err();
    assert!(err.to_string().contains("(line . character)"), "got: {err}");
}

#[test]
fn optional_pair_fields_false_is_none_pair_is_some() {
    assert_eq!(
        optional_pair_fields(SteelVal::BoolV(false), "f", "(start . end)").unwrap(),
        None
    );
    let pair = cons_pair(SteelVal::IntV(0), SteelVal::IntV(5)).unwrap();
    assert_eq!(
        optional_pair_fields(pair, "f", "(start . end)").unwrap(),
        Some((SteelVal::IntV(0), SteelVal::IntV(5)))
    );
}

// ── ArgPane ───────────────────────────────────────────────────────────────

/// `ArgPane::from_steelval` rejects a non-pane `SteelVal`, preserving
/// the "expected pane" substring every buffer-touching builtin's
/// wrong-type test asserts on.
///
/// Fail oracle: return `Ok` for any value → any argument would silently
/// decode as a pane.
#[test]
fn arg_pane_rejects_non_pane() {
    let err = ArgPane::from_steelval(&SteelVal::StringV("not-a-pane".into())).unwrap_err();
    assert!(err.to_string().contains("expected pane"), "got: {err}");
}

#[test]
fn arg_pane_accepts_a_real_pane() {
    use crate::builtins::ids::SteelPane;
    let val = SteelPane::new(PaneHandle::buffer_only(BufferId::default())).into_steel_val();
    assert_eq!(
        ArgPane::from_steelval(&val).unwrap().0.buffer(),
        BufferId::default()
    );
}

// ── LivePane ─────────────────────────────────────────────────────────────

/// `LivePane::from_steelval` — type check only, same as `ArgPane`'s; the
/// liveness half is `BuiltinArg::resolve`'s job (proven through a real
/// `ScriptingHost`, `builtins::tests::
/// live_pane_builtins_raise_on_a_closed_buffer_through_real_registration` —
/// a direct call here has no `SteelCtx`/host to check liveness against).
#[test]
fn live_pane_rejects_non_pane() {
    let err = LivePane::from_steelval(&SteelVal::StringV("not-a-pane".into())).unwrap_err();
    assert!(err.to_string().contains("expected pane"), "got: {err}");
}

#[test]
fn live_pane_accepts_a_real_pane() {
    use crate::builtins::ids::SteelPane;
    let val = SteelPane::new(PaneHandle::buffer_only(BufferId::default())).into_steel_val();
    assert_eq!(
        LivePane::from_steelval(&val).unwrap().0.buffer(),
        BufferId::default()
    );
}

// ── Usize / OptUsize / OptString ────────────────────────────────────────────

#[test]
fn usize_newtype_rejects_negative_and_non_integer() {
    assert!(Usize::from_steelval(&SteelVal::IntV(-1)).is_err());
    assert!(Usize::from_steelval(&SteelVal::StringV("x".into())).is_err());
}

#[test]
fn usize_newtype_accepts_non_negative() {
    assert_eq!(Usize::from_steelval(&SteelVal::IntV(4)).unwrap().0, 4);
}

#[test]
fn opt_usize_newtype_false_is_none_int_is_some() {
    assert_eq!(
        OptUsize::from_steelval(&SteelVal::BoolV(false)).unwrap().0,
        None
    );
    assert_eq!(
        OptUsize::from_steelval(&SteelVal::IntV(4)).unwrap().0,
        Some(4)
    );
    assert!(OptUsize::from_steelval(&SteelVal::IntV(-1)).is_err());
}

#[test]
fn opt_string_newtype_false_is_none_string_and_symbol_are_some() {
    assert_eq!(
        OptString::from_steelval(&SteelVal::BoolV(false)).unwrap().0,
        None
    );
    assert_eq!(
        OptString::from_steelval(&SteelVal::StringV("rust".into()))
            .unwrap()
            .0,
        Some("rust".to_string())
    );
    assert_eq!(
        OptString::from_steelval(&SteelVal::SymbolV("rust".into()))
            .unwrap()
            .0,
        Some("rust".to_string())
    );
    assert!(OptString::from_steelval(&SteelVal::IntV(1)).is_err());
}

// ── LspTargetArg ─────────────────────────────────────────────────────────

/// `LspTargetArg::from_steelval` — type check only, deciding `Buffer` vs
/// `Language`; the `Buffer` case's liveness half is `BuiltinArg::resolve`'s
/// job (proven through a real `ScriptingHost`, `builtins::tests::
/// live_pane_builtins_raise_on_a_closed_buffer_through_real_registration`'s
/// `lsp-stop!`/`lsp-restart!` rows).
#[test]
fn lsp_target_arg_accepts_a_pane() {
    use crate::builtins::ids::SteelPane;
    let val = SteelPane::new(PaneHandle::buffer_only(BufferId::default())).into_steel_val();
    assert!(matches!(
        LspTargetArg::from_steelval(&val).unwrap(),
        LspTargetArg::Buffer(bid) if bid == BufferId::default()
    ));
}

#[test]
fn lsp_target_arg_accepts_a_string_or_symbol_language() {
    assert!(matches!(
        LspTargetArg::from_steelval(&SteelVal::StringV("rust".into())).unwrap(),
        LspTargetArg::Language(lang) if lang == "rust"
    ));
    assert!(matches!(
        LspTargetArg::from_steelval(&SteelVal::SymbolV("rust".into())).unwrap(),
        LspTargetArg::Language(lang) if lang == "rust"
    ));
}

/// No fallback left to decode `#f` into — the typed-command wrapper
/// (`registration.scm`) supplies the focused buffer explicitly instead.
///
/// Fail oracle: `#f` decoding to a "no target" variant instead of erroring.
#[test]
fn lsp_target_arg_rejects_false() {
    let err = LspTargetArg::from_steelval(&SteelVal::BoolV(false)).unwrap_err();
    assert!(
        err.to_string()
            .contains("expected a buffer-id or a language name"),
        "got: {err}"
    );
}

// ── wire_text_edit_arg ───────────────────────────────────────────────────

/// A `JsonHandle` onto a wire `TextEdit` — an unconverted element straight
/// from a `textDocument/formatting`-shaped response — decodes, and carries
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
/// other scalar) — rejected outright, not silently decoded as if it had no
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
/// way — it decodes as JSON fine, but has no server to read an encoding
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
