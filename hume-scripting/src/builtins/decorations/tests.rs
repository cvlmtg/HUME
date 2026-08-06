use super::*;
use crate::test_support::SteelCtxTestHarness;
use hume_engine::pipeline::BufferId;
use steel::HashMap as SteelHashMap;
use steel::gc::Gc;
use steel::rvals::IntoSteelVal as _;

/// Builds a Steel hashmap `SteelVal` from `(symbol-key, value)` pairs — the
/// `(hash 'k v ...)` shape `virtual_line_spec` decodes.
fn hashmap(entries: Vec<(&str, SteelVal)>) -> SteelVal {
    let mut hm = SteelHashMap::new();
    for (k, v) in entries {
        hm.insert(SteelVal::SymbolV(k.into()), v);
    }
    SteelVal::HashMapV(Gc::new(hm).into())
}

/// Builds a Steel list `SteelVal` from its elements.
fn list(items: Vec<SteelVal>) -> SteelVal {
    items.into_steelval().unwrap()
}

/// Builds a `(start end scope)` segment 3-list.
fn seg(start: isize, end: isize, scope: &str) -> SteelVal {
    list(vec![
        SteelVal::IntV(start),
        SteelVal::IntV(end),
        SteelVal::StringV(scope.into()),
    ])
}

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

// ── `virtual_line_specs` decoder ─────────────────────────────────────────────
//
// These call the private decoder directly (`NullHost` has no `DecorationHost`,
// so a full `set_virtual_lines` round trip can't reach the store from here) —
// same pattern as `picker_items`'s tests in `builtins/ui.rs`.

#[test]
fn virtual_line_spec_decodes_every_field() {
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(12)),
        ("anchor", SteelVal::SymbolV("before".into())),
        ("text", SteelVal::StringV("- let x = 5".into())),
        ("scope", SteelVal::StringV("diff.minus".into())),
        (
            "segments",
            list(vec![seg(0, 1, "diff.minus"), seg(2, 5, "keyword")]),
        ),
    ]);
    let specs = virtual_line_specs(list(vec![entry]), "test").unwrap();
    assert_eq!(specs.len(), 1);
    let spec = &specs[0];
    assert_eq!(spec.line, 12);
    assert_eq!(spec.text, "- let x = 5");
    assert!(spec.before, "'anchor 'before must decode to before: true");
    assert_eq!(spec.scope.as_deref(), Some("diff.minus"));
    assert_eq!(
        spec.segments,
        vec![
            (0, 1, "diff.minus".to_string()),
            (2, 5, "keyword".to_string())
        ]
    );
}

#[test]
fn virtual_line_spec_defaults_anchor_to_after_and_scope_to_none() {
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("x".into())),
    ]);
    let specs = virtual_line_specs(list(vec![entry]), "test").unwrap();
    let spec = &specs[0];
    assert!(!spec.before, "default anchor must be 'after");
    assert_eq!(spec.scope, None);
    assert!(spec.segments.is_empty());
}

#[test]
fn virtual_line_spec_sorts_segments() {
    // Unsorted input, and the second entry's byte range only fits if the text
    // is long enough — this also locks that the decoder validates against
    // the *decoded* text, not a stale copy.
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("abcdef".into())),
        ("segments", list(vec![seg(4, 6, "b"), seg(0, 2, "a")])),
    ]);
    let specs = virtual_line_specs(list(vec![entry]), "test").unwrap();
    assert_eq!(
        specs[0].segments,
        vec![(0, 2, "a".to_string()), (4, 6, "b".to_string())],
        "segments must come out sorted by start regardless of input order"
    );
}

#[test]
fn virtual_line_spec_rejects_positional_list_entry() {
    // The shape `set-virtual-lines!` accepted before this change.
    let old_shape = list(vec![
        SteelVal::IntV(0),
        SteelVal::StringV("text".into()),
        SteelVal::StringV("scope".into()),
    ]);
    let err = virtual_line_specs(list(vec![old_shape]), "test")
        .unwrap_err()
        .to_string();
    assert!(err.contains("hashmap"), "got: {err}");
}

#[test]
fn virtual_line_spec_rejects_unknown_key() {
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("x".into())),
        ("segment", list(vec![])), // typo for 'segments
    ]);
    let err = virtual_line_specs(list(vec![entry]), "test")
        .unwrap_err()
        .to_string();
    assert!(err.contains("segment"), "got: {err}");
}

#[test]
fn virtual_line_spec_rejects_missing_line() {
    let entry = hashmap(vec![("text", SteelVal::StringV("x".into()))]);
    let err = virtual_line_specs(list(vec![entry]), "test")
        .unwrap_err()
        .to_string();
    assert!(err.contains("'line"), "got: {err}");
}

#[test]
fn virtual_line_spec_rejects_missing_text() {
    let entry = hashmap(vec![("line", SteelVal::IntV(0))]);
    let err = virtual_line_specs(list(vec![entry]), "test")
        .unwrap_err()
        .to_string();
    assert!(err.contains("'text"), "got: {err}");
}

#[test]
fn virtual_line_spec_rejects_bad_anchor_symbol() {
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("x".into())),
        ("anchor", SteelVal::SymbolV("above".into())),
    ]);
    let err = virtual_line_specs(list(vec![entry]), "test")
        .unwrap_err()
        .to_string();
    assert!(err.contains("'before"), "got: {err}");
    assert!(err.contains("'after"), "got: {err}");
}

#[test]
fn virtual_line_spec_rejects_segment_end_past_text_len() {
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("ab".into())),
        ("segments", list(vec![seg(0, 5, "x")])),
    ]);
    let err = virtual_line_specs(list(vec![entry]), "test")
        .unwrap_err()
        .to_string();
    assert!(err.contains("byte length"), "got: {err}");
}

#[test]
fn virtual_line_spec_rejects_zero_length_segment() {
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("abcdef".into())),
        ("segments", list(vec![seg(2, 2, "x")])),
    ]);
    let err = virtual_line_specs(list(vec![entry]), "test")
        .unwrap_err()
        .to_string();
    assert!(err.contains("start < end"), "got: {err}");
}

#[test]
fn virtual_line_spec_rejects_overlapping_segments() {
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("abcdef".into())),
        ("segments", list(vec![seg(0, 3, "a"), seg(2, 5, "b")])),
    ]);
    let err = virtual_line_specs(list(vec![entry]), "test")
        .unwrap_err()
        .to_string();
    assert!(err.contains("overlap"), "got: {err}");
}

#[test]
fn virtual_line_spec_rejects_non_char_boundary_segment() {
    // "é" is 2 bytes (U+00E9 encodes to 0xC3 0xA9); byte offset 1 falls
    // inside it, not on a boundary.
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("é".into())),
        ("segments", list(vec![seg(1, 2, "x")])),
    ]);
    let err = virtual_line_specs(list(vec![entry]), "test")
        .unwrap_err()
        .to_string();
    assert!(err.contains("char boundary"), "got: {err}");
}

#[test]
fn virtual_line_spec_rejects_segment_with_wrong_arity() {
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("abcdef".into())),
        (
            "segments",
            list(vec![list(vec![SteelVal::IntV(0), SteelVal::IntV(1)])]),
        ),
    ]);
    let err = virtual_line_specs(list(vec![entry]), "test")
        .unwrap_err()
        .to_string();
    assert!(err.contains("(start end scope)"), "got: {err}");
}
