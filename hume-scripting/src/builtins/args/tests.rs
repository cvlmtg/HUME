use super::*;
use crate::builtins::lsp::LspTargetArg;
use hume_core::PaneHandle;
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
fn chars_arg_rejects_a_string_entry() {
    let err = chars_arg(list_of(&["a"]), "chars").unwrap_err();
    assert!(
        err.to_string().contains("each entry must be a char"),
        "got: {err}"
    );
}

#[test]
fn chars_arg_accepts_char_entries() {
    let chars: SteelVal = vec![SteelVal::CharV('a'), SteelVal::CharV('b')]
        .into_steelval()
        .unwrap();
    assert_eq!(chars_arg(chars, "chars").unwrap(), vec!['a', 'b']);
}

#[test]
fn json_params_rejects_bool() {
    let err = json_params(SteelVal::BoolV(true), "lsp-request! params").unwrap_err();
    assert!(
        err.to_string().contains("lsp-request! params"),
        "got: {err}"
    );
}

// ── hash_entry ───────────────────────────────────────────────────────────

fn symbol_hash(entries: &[(&str, &str)]) -> SteelVal {
    let mut hm = steel::HashMap::new();
    for (k, v) in entries {
        hm.insert(
            SteelVal::SymbolV((*k).into()),
            SteelVal::StringV((*v).into()),
        );
    }
    SteelVal::HashMapV(steel::gc::Gc::new(hm).into())
}

#[test]
fn hash_list_decodes_each_entry_via_row() {
    let entries: SteelVal = vec![
        symbol_hash(&[("a", "1"), ("b", "2")]),
        symbol_hash(&[("a", "3")]),
    ]
    .into_steelval()
    .unwrap();
    let out = hash_list(entries, "f", &["a", "b"], |entry| {
        Ok((
            string_arg(entry.required("a")?, "f")?,
            entry
                .optional("b")
                .map(|v| string_arg(v, "f"))
                .transpose()?,
        ))
    })
    .unwrap();
    assert_eq!(
        out,
        vec![
            ("1".to_string(), Some("2".to_string())),
            ("3".to_string(), None)
        ]
    );
}

#[test]
fn hash_list_rejects_an_unknown_key() {
    let entries: SteelVal = vec![symbol_hash(&[("a", "1"), ("c", "2")])]
        .into_steelval()
        .unwrap();
    let err = hash_list(entries, "f", &["a", "b"], |entry| entry.required("a")).unwrap_err();
    assert!(err.to_string().contains("unknown key 'c,"), "got: {err}");
}

#[test]
fn hash_list_rejects_a_string_key() {
    let mut hm = steel::HashMap::new();
    hm.insert(SteelVal::StringV("a".into()), SteelVal::IntV(1));
    let entries: SteelVal = vec![SteelVal::HashMapV(steel::gc::Gc::new(hm).into())]
        .into_steelval()
        .unwrap();
    let err = hash_list(entries, "f", &["a"], |entry| entry.required("a")).unwrap_err();
    assert!(err.to_string().contains("must be a symbol"), "got: {err}");
}

#[test]
fn hash_list_reports_a_missing_required_key() {
    let entries: SteelVal = vec![symbol_hash(&[("b", "2")])].into_steelval().unwrap();
    let err = hash_list(entries, "f", &["a", "b"], |entry| entry.required("a")).unwrap_err();
    assert!(err.to_string().contains("f: missing 'a"), "got: {err}");
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

// ── ArgPane ───────────────────────────────────────────────────────────────

/// `ArgPane::from_steelval` rejects a non-pane `SteelVal`, preserving
/// the "expected pane" substring every buffer-touching builtin's
/// wrong-type test asserts on.
///
// ── Usize ───────────────────────────────────────────────────────

#[test]
fn usize_newtype_rejects_negative_and_non_integer() {
    assert!(Usize::from_steelval(&SteelVal::IntV(-1)).is_err());
    assert!(Usize::from_steelval(&SteelVal::StringV("x".into())).is_err());
}

#[test]
fn usize_newtype_accepts_non_negative() {
    assert_eq!(Usize::from_steelval(&SteelVal::IntV(4)).unwrap().0, 4);
}

// ── LspTargetArg ─────────────────────────────────────────────────────────

/// `LspTargetArg::from_steelval`: type check only, deciding `Buffer` vs
/// `Name`; the `Buffer` case's liveness half is `BuiltinArg::resolve`'s
/// job (proven through a real `ScriptingHost`, `builtins::tests::
/// pane_builtins_raise_on_a_closed_buffer_through_real_registration`'s
/// `lsp-stop!`/`lsp-restart!` rows).
#[test]
fn lsp_target_arg_accepts_a_pane() {
    use crate::builtins::ids::SteelPane;
    let val = SteelPane::new(PaneHandle::buffer_only(BufferId::default())).into_steel_val();
    assert!(matches!(
        LspTargetArg::from_steelval(&val).unwrap(),
        LspTargetArg::Buffer(_)
    ));
}

#[test]
fn lsp_target_arg_accepts_a_string_or_symbol_server_name() {
    assert!(matches!(
        LspTargetArg::from_steelval(&SteelVal::StringV("rust-analyzer".into())).unwrap(),
        LspTargetArg::Name(name) if name == "rust-analyzer"
    ));
    assert!(matches!(
        LspTargetArg::from_steelval(&SteelVal::SymbolV("rust-analyzer".into())).unwrap(),
        LspTargetArg::Name(name) if name == "rust-analyzer"
    ));
}

/// No fallback left to decode `#f` into: the typed-command wrapper
/// (`core:lsp-install`'s `register.scm`) supplies the focused buffer explicitly instead.
#[test]
fn lsp_target_arg_rejects_false() {
    let err = LspTargetArg::from_steelval(&SteelVal::BoolV(false)).unwrap_err();
    assert!(
        err.to_string()
            .contains("expected a server, a pane or a server name"),
        "got: {err}"
    );
}

// ── Key specs ─────────────────────────────────────────────────────────────

fn spec(s: &str) -> SteelVal {
    SteelVal::StringV(s.into())
}

#[test]
fn single_key_arg_accepts_one_chord() {
    use termina::event::{KeyCode, Modifiers};
    assert_eq!(
        single_key_arg(spec("ctrl-x"), "f").unwrap(),
        KeyEvent::new(KeyCode::Char('x'), Modifiers::CONTROL)
    );
}

#[test]
fn single_key_arg_rejects_a_sequence_naming_ctx() {
    let msg = single_key_arg(spec("g h"), "insert-key!")
        .unwrap_err()
        .to_string();
    assert!(msg.contains("insert-key!"), "got: {msg}");
    assert!(msg.contains("exactly one key"), "got: {msg}");
}

#[test]
fn single_key_arg_rejects_unparseable_naming_ctx() {
    let msg = single_key_arg(spec("not-a-real-key"), "insert-key!")
        .unwrap_err()
        .to_string();
    assert!(msg.contains("insert-key!"), "got: {msg}");
    assert!(msg.contains("invalid key spec"), "got: {msg}");
}

#[test]
fn hash_entry_decodes_one_record() {
    let entry = hash_entry(symbol_hash(&[("a", "1")]), "f", &["a", "b"]).unwrap();
    assert_eq!(string_arg(entry.required("a").unwrap(), "f").unwrap(), "1");
    assert_eq!(entry.optional("b"), None);
}

#[test]
fn hash_entry_rejects_a_non_hashmap() {
    let err = hash_entry(SteelVal::IntV(1), "f", &["a"])
        .err()
        .expect("a non-hashmap must be rejected");
    assert!(
        err.to_string()
            .contains("f: expected a hashmap with keys from 'a"),
        "got: {err}"
    );
}

#[test]
fn hash_entry_rejects_an_unknown_key() {
    let err = hash_entry(symbol_hash(&[("c", "1")]), "f", &["a", "b"])
        .err()
        .expect("an unknown key must be rejected");
    assert!(err.to_string().contains("unknown key 'c,"), "got: {err}");
}

#[test]
fn optional_hash_entry_false_is_none_hash_is_some() {
    assert!(
        optional_hash_entry(SteelVal::BoolV(false), "f", &["a"])
            .unwrap()
            .is_none()
    );
    let entry = optional_hash_entry(symbol_hash(&[("a", "1")]), "f", &["a"])
        .unwrap()
        .expect("a hashmap decodes to Some");
    assert_eq!(string_arg(entry.required("a").unwrap(), "f").unwrap(), "1");
}

// ── cwd_arg ───────────────────────────────────────────────────────────────

#[test]
fn cwd_arg_false_is_the_editor_cwd() {
    let got = cwd_arg(SteelVal::BoolV(false), &AbsolutePath::rooted("/ed"), "f").unwrap();
    assert_eq!(got, AbsolutePath::rooted("/ed").as_path());
}

#[test]
fn cwd_arg_resolves_a_relative_path_against_the_editor_cwd() {
    let ed = tempfile::tempdir().unwrap();
    let got = cwd_arg(
        SteelVal::StringV("sub/x".into()),
        &AbsolutePath::rooted(ed.path()),
        "f",
    )
    .unwrap();
    assert_eq!(got, ed.path().join("sub").join("x"));
}

#[test]
fn cwd_arg_keeps_an_absolute_path() {
    let (ed, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let arg = SteelVal::StringV(other.path().to_str().unwrap().into());
    let got = cwd_arg(arg, &AbsolutePath::rooted(ed.path()), "f").unwrap();
    assert_eq!(got, other.path());
}

#[test]
fn cwd_arg_fails_for_a_path_that_cannot_be_made_absolute() {
    let ed = tempfile::tempdir().unwrap();
    let arg = SteelVal::StringV(if cfg!(windows) { "Q:x" } else { "missing/../x" }.into());
    assert!(cwd_arg(arg, &AbsolutePath::rooted(ed.path()), "f").is_err());
}
