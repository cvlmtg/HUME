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
    let result = set_inlay_hints(
        &mut ctx,
        SteelVal::StringV("test".into()),
        BidArg(BufferId::default()),
        empty,
    );
    assert_names_builtin(result, "set-inlay-hints!");
}

#[test]
fn register_sign_source_without_decoration_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = register_sign_source(
        &mut ctx,
        SteelVal::StringV("test".into()),
        SteelVal::IntV(10),
    );
    assert_names_builtin(result, "register-sign-source!");
}

#[test]
fn register_sign_source_rejects_an_empty_name() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = register_sign_source(&mut ctx, SteelVal::StringV("  ".into()), SteelVal::IntV(10));
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("name must not be empty"), "got: {msg}");
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
fn set_signs_rejects_a_control_character_in_the_glyph() {
    // The gutter right-aligns a sign by measuring its text, then writes it
    // with a terminal-buffer writer that silently drops anything it can't
    // draw. A tab measures as several columns and draws as none, so the
    // padding lands wrong and the lane's separator sits past the end of the
    // glyph. Rejected at intake, where the caller can still be told which
    // value was wrong.
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let sign = list(vec![
        SteelVal::IntV(0),
        SteelVal::StringV("\tS".into()),
        SteelVal::StringV("ui.sign".into()),
    ]);
    let result = set_signs(
        &mut ctx,
        SteelVal::StringV("test".into()),
        BidArg(BufferId::default()),
        list(vec![sign]),
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("must not contain a control character"),
        "got: {msg}"
    );
    assert!(msg.contains("set-signs!"), "got: {msg}");
}

#[test]
fn set_signs_accepts_a_multi_codepoint_glyph() {
    // Only *control* characters are rejected — a normal multi-byte glyph
    // (here a combining sequence) must still reach the gutter untouched.
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let sign = list(vec![
        SteelVal::IntV(0),
        SteelVal::StringV("e\u{0301}".into()),
        SteelVal::StringV("ui.sign".into()),
    ]);
    let result = set_signs(
        &mut ctx,
        SteelVal::StringV("test".into()),
        BidArg(BufferId::default()),
        list(vec![sign]),
    );
    // The harness has no decoration host, so validation passing means the
    // call gets as far as the host lookup and fails *there*.
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
fn set_eol_text_without_decoration_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let empty: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let result = set_eol_text(
        &mut ctx,
        SteelVal::StringV("test".into()),
        BidArg(BufferId::default()),
        empty,
    );
    assert_names_builtin(result, "set-eol-text!");
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

#[test]
fn set_statusline_text_without_decoration_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = set_statusline_text(
        &mut ctx,
        SteelVal::StringV("test".into()),
        BidArg(BufferId::default()),
        SteelVal::StringV("main".into()),
    );
    assert_names_builtin(result, "set-statusline-text!");
}

// ── `virtual_line_specs` decoder ─────────────────────────────────────────────
//
// These call the private decoder directly (`NullHost` has no `DecorationHost`,
// so a full `set_virtual_lines` round trip can't reach the store from here) —
// same pattern as `picker_items`'s tests in `builtins/ui.rs`. The decoder only
// checks shape (arity, types) now — segment bounds/ordering/overlap/grapheme
// validation moved to the host boundary (`host_impl.rs`'s
// `virtual_line_segments_to_bytes`), tested there instead.

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
    let specs = virtual_line_specs(list(vec![entry])).unwrap();
    assert_eq!(specs.len(), 1);
    let spec = &specs[0];
    assert_eq!(spec.line, 12);
    assert_eq!(spec.text, "- let x = 5");
    assert!(spec.before, "'anchor 'before must decode to before: true");
    assert_eq!(spec.scope.as_deref(), Some("diff.minus"));
    // Char offsets, passed through verbatim — this layer no longer sorts,
    // bounds-checks, or validates them.
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
    let specs = virtual_line_specs(list(vec![entry])).unwrap();
    let spec = &specs[0];
    assert!(!spec.before, "default anchor must be 'after");
    assert_eq!(spec.scope, None);
    assert!(spec.segments.is_empty());
}

#[test]
fn virtual_line_spec_rejects_positional_list_entry() {
    // The shape `set-virtual-lines!` accepted before this change.
    let old_shape = list(vec![
        SteelVal::IntV(0),
        SteelVal::StringV("text".into()),
        SteelVal::StringV("scope".into()),
    ]);
    let err = virtual_line_specs(list(vec![old_shape]))
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
    let err = virtual_line_specs(list(vec![entry]))
        .unwrap_err()
        .to_string();
    // Not `err.contains("segment")`: the message always also prints the
    // expected-keys list, which contains "segments" — a substring of
    // "segment" — so that assertion would pass regardless of which key was
    // actually blamed. Assert on the phrase naming the offending key instead.
    assert!(err.contains("unknown key 'segment,"), "got: {err}");
}

#[test]
fn virtual_line_spec_rejects_missing_line() {
    let entry = hashmap(vec![("text", SteelVal::StringV("x".into()))]);
    let err = virtual_line_specs(list(vec![entry]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("'line"), "got: {err}");
}

#[test]
fn virtual_line_spec_rejects_missing_text() {
    let entry = hashmap(vec![("line", SteelVal::IntV(0))]);
    let err = virtual_line_specs(list(vec![entry]))
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
    let err = virtual_line_specs(list(vec![entry]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("'before"), "got: {err}");
    assert!(err.contains("'after"), "got: {err}");
}

#[test]
fn virtual_line_spec_rejects_newline_in_text() {
    // A virtual line renders as a single row (`rows.rs`'s
    // `segment_virtual_row`); a raw newline would become one garbled
    // `CellContent::Virtual` cell instead of splitting the row.
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("- deleted a\n- deleted b".into())),
    ]);
    let err = virtual_line_specs(list(vec![entry]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("newline"), "got: {err}");
}

#[test]
fn virtual_line_spec_rejects_carriage_return_in_text() {
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("a\rb".into())),
    ]);
    let err = virtual_line_specs(list(vec![entry]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("newline"), "got: {err}");
}

#[test]
fn virtual_line_spec_keeps_a_literal_tab_in_text() {
    // The engine expands a tab in a virtual row's text to the next tab
    // stop (`hume_engine::rows::segment_virtual_row`) — this builtin no
    // longer expands it, or rejects it, itself.
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("\tx".into())),
    ]);
    let specs = virtual_line_specs(list(vec![entry])).expect("a tab is accepted");
    assert_eq!(specs[0].text, "\tx");
}

#[test]
fn virtual_line_spec_keeps_a_control_character_verbatim() {
    // A control character other than tab is not this builtin's problem to
    // solve: the engine's own placeholder substitution
    // (`hume_rope::width::needs_placeholder`/`placeholder`) is what stands
    // between it and the terminal, the same chokepoint every other text
    // source (buffer content, inline inserts) goes through. Blanking it here
    // instead would be a second, weaker copy of that policy — CLAUDE.md's
    // display-columns invariant says unrenderable text is shown as its
    // codepoint, never as a blank a bidi override could hide behind.
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("a\u{7}b".into())), // BEL
    ]);
    let specs = virtual_line_specs(list(vec![entry])).expect("accepted, not rejected");
    assert_eq!(
        specs[0].text, "a\u{7}b",
        "verbatim — no Steel-side substitution"
    );
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
    let err = virtual_line_specs(list(vec![entry]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("(start end scope)"), "got: {err}");
}
