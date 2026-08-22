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

// ── BidArg ────────────────────────────────────────────────────────────────

/// `BidArg::from_steelval` rejects a non-buffer-id `SteelVal`, preserving
/// the "expected buffer-id" substring every buffer-touching builtin's
/// wrong-type test asserts on.
///
/// Fail oracle: return `Ok` for any value → any argument would silently
/// decode as a buffer-id.
#[test]
fn bid_arg_rejects_non_buffer_id() {
    let err = BidArg::from_steelval(&SteelVal::StringV("not-an-id".into())).unwrap_err();
    assert!(err.to_string().contains("expected buffer-id"), "got: {err}");
}

#[test]
fn bid_arg_accepts_a_real_buffer_id() {
    use crate::builtins::ids::SteelBufferId;
    let val = SteelBufferId(BufferId::default()).into_steelval().unwrap();
    assert_eq!(BidArg::from_steelval(&val).unwrap().0, BufferId::default());
}

/// `BidArg::not_live_err`'s wording matches `require_live`'s own — the two
/// must stay identical since callers whose own host call already does the
/// liveness lookup (e.g. `diff-buffer-lines`) use `not_live_err` directly
/// instead of a second `require_live` check.
///
/// Fail oracle: let the two messages drift (e.g. hand-roll a different
/// string at one of the two call sites) — a plugin matching on this error's
/// text would then behave differently depending on which builtin raised it.
#[test]
fn not_live_err_matches_require_live_wording() {
    let bid = BidArg(BufferId::default());
    let err = bid.not_live_err("diff-buffer-lines");
    assert!(err.to_string().contains("invalid buffer id"), "got: {err}");
    assert!(err.to_string().contains("diff-buffer-lines"), "got: {err}");
}

// ── PosArg ────────────────────────────────────────────────────────────────

fn pos_pair(line: isize, character: isize) -> SteelVal {
    cons_pair(SteelVal::IntV(line), SteelVal::IntV(character)).unwrap()
}

#[test]
fn pos_arg_decodes_line_character_pair() {
    let pos = PosArg::from_steelval(&pos_pair(3, 7)).unwrap();
    assert_eq!((pos.line, pos.character), (3, 7));
}

#[test]
fn pos_arg_rejects_proper_list() {
    let val: SteelVal = vec![SteelVal::IntV(3), SteelVal::IntV(7)]
        .into_steelval()
        .unwrap();
    let err = PosArg::from_steelval(&val).unwrap_err();
    assert!(err.to_string().contains("(line . character)"), "got: {err}");
}

#[test]
fn pos_arg_rejects_non_pair_scalar() {
    assert!(PosArg::from_steelval(&SteelVal::IntV(3)).is_err());
}

// ── TextEditArg ───────────────────────────────────────────────────────────

#[test]
fn text_edit_arg_decodes_start_end_text() {
    let val: SteelVal = vec![
        pos_pair(0, 0),
        pos_pair(0, 3),
        SteelVal::StringV("abc".into()),
    ]
    .into_steelval()
    .unwrap();
    let edit = TextEditArg::from_steelval(&val).unwrap();
    assert_eq!((edit.start.line, edit.start.character), (0, 0));
    assert_eq!((edit.end.line, edit.end.character), (0, 3));
    assert_eq!(edit.text, "abc");
}

#[test]
fn text_edit_arg_rejects_malformed_position() {
    let bad_start: SteelVal = vec![SteelVal::IntV(0), SteelVal::IntV(0)] // proper list, not a pair
        .into_steelval()
        .unwrap();
    let val: SteelVal = vec![bad_start, pos_pair(0, 3), SteelVal::StringV("abc".into())]
        .into_steelval()
        .unwrap();
    let err = TextEditArg::from_steelval(&val).unwrap_err();
    assert!(err.to_string().contains("(line . character)"), "got: {err}");
}
