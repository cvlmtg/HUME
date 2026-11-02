use super::*;
use crate::test_support::{SteelCtxTestHarness, default_bid, default_pane};
use hume_rope::spanned::SpannedText;
use steel::HashMap as SteelHashMap;
use steel::gc::Gc;
use steel::rvals::IntoSteelVal as _;

/// Builds a Steel hashmap `SteelVal` from `(symbol-key, value)` pairs: the
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

/// Builds a `(hash 'start-char 'end-char 'scope)` segment.
fn seg(start: isize, end: isize, scope: &str) -> SteelVal {
    hashmap(vec![
        ("start-char-col", SteelVal::IntV(start)),
        ("end-char-col", SteelVal::IntV(end)),
        ("scope", SteelVal::StringV(scope.into())),
    ])
}

/// Builds a `(hash 'line 'text 'scope)` sign.
fn sign(line: isize, text: &str, scope: &str) -> SteelVal {
    hashmap(vec![
        ("line", SteelVal::IntV(line)),
        ("text", SteelVal::StringV(text.into())),
        ("scope", SteelVal::StringV(scope.into())),
    ])
}

/// Every decoration setter on a host with no `DecorationHost` capability
/// (`NullHost`, the harness default) surfaces `require_cap`'s one message.
///
/// A setter written as `if let Some(decorations) = ... { ... } Ok(Void)`
/// would silently no-op, and `.unwrap_err()` would panic on its `Ok`.
fn assert_unsupported(result: SteelResult) {
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("not supported by this host"), "got: {msg}");
}

/// The diagnostic reads raise on a host with no decorations capability (a
/// drawer row renderer's), where an empty answer would read as "no
/// diagnostics".
#[test]
fn diagnostic_reads_without_decoration_host_error() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    assert_unsupported(diagnostics_for_buffer(
        &mut ctx,
        ArgPane(default_pane()),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
    ));
    assert_unsupported(diagnostic_counts(
        &mut ctx,
        ArgPane(default_pane()),
        SteelVal::BoolV(false),
    ));
}

#[test]
fn diagnostic_glyph_names_each_severity() {
    let glyphs: Vec<String> = ["error", "warning", "info", "hint"]
        .into_iter()
        .map(
            |sev| match diagnostic_glyph(SteelVal::SymbolV(sev.into())).unwrap() {
                SteelVal::StringV(s) => s.to_string(),
                other => panic!("expected a string, got {other:?}"),
            },
        )
        .collect();
    assert_eq!(glyphs, ["✘", "⚠", "ℹ", "·"]);
}

#[test]
fn diagnostic_glyph_raises_on_an_unknown_severity() {
    let msg = diagnostic_glyph(SteelVal::SymbolV("bogus".into()))
        .unwrap_err()
        .to_string();
    assert!(msg.contains("severity"), "got: {msg}");
}

#[test]
fn set_inlay_hints_without_decoration_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let empty: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let result = set_inlay_hints(&mut ctx, default_bid(), "test".to_string(), empty);
    assert_unsupported(result);
}

#[test]
fn register_sign_source_without_decoration_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = register_sign_source(
        &mut ctx,
        default_bid(),
        "test".to_string(),
        SteelVal::IntV(10),
    );
    assert_unsupported(result);
}

#[test]
fn set_signs_without_decoration_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let empty: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let result = set_signs(&mut ctx, default_bid(), "test".to_string(), empty);
    assert_unsupported(result);
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
    let result = set_signs(
        &mut ctx,
        default_bid(),
        "test".to_string(),
        list(vec![sign(0, "\tS", "ui.sign")]),
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("'text must not contain a control character"),
        "got: {msg}"
    );
}

#[test]
fn set_signs_accepts_a_multi_codepoint_glyph() {
    // Only *control* characters are rejected: a normal multi-byte glyph
    // (here a combining sequence) must still reach the gutter untouched.
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = set_signs(
        &mut ctx,
        default_bid(),
        "test".to_string(),
        list(vec![sign(0, "e\u{0301}", "ui.sign")]),
    );
    // The harness has no decoration host, so validation passing means the
    // call gets as far as the host lookup and fails *there*.
    assert_unsupported(result);
}

#[test]
fn set_signs_accepts_an_end_line() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(1)),
        ("end-line", SteelVal::IntV(4)),
        ("text", SteelVal::StringV("!".into())),
        ("scope", SteelVal::StringV("ui.sign".into())),
    ]);
    let result = set_signs(
        &mut ctx,
        default_bid(),
        "test".to_string(),
        list(vec![entry]),
    );
    // Decoding passes, so the call reaches the host lookup and fails there.
    assert_unsupported(result);
}

#[test]
fn set_line_backgrounds_accepts_an_end_line() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("end-line", SteelVal::IntV(2)),
        ("scope", SteelVal::StringV("diff.plus".into())),
    ]);
    let result = set_line_backgrounds(
        &mut ctx,
        default_bid(),
        "test".to_string(),
        list(vec![entry]),
    );
    assert_unsupported(result);
}

#[test]
fn set_signs_rejects_a_positional_list_entry() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let positional = list(vec![
        SteelVal::IntV(0),
        SteelVal::StringV("!".into()),
        SteelVal::StringV("ui.sign".into()),
    ]);
    let msg = set_signs(
        &mut ctx,
        default_bid(),
        "test".to_string(),
        list(vec![positional]),
    )
    .unwrap_err()
    .to_string();
    assert!(msg.contains("signs 0: expected a hashmap"), "got: {msg}");
}

#[test]
fn set_signs_rejects_a_missing_key() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("!".into())),
    ]);
    let msg = set_signs(
        &mut ctx,
        default_bid(),
        "test".to_string(),
        list(vec![entry]),
    )
    .unwrap_err()
    .to_string();
    assert!(msg.contains("missing 'scope"), "got: {msg}");
}

#[test]
fn set_virtual_lines_without_decoration_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let empty: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let result = set_virtual_lines(&mut ctx, default_bid(), "test".to_string(), empty);
    assert_unsupported(result);
}

#[test]
fn set_eol_text_without_decoration_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let empty: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let result = set_eol_text(
        &mut ctx,
        default_bid(),
        "test".to_string(),
        empty,
        SteelVal::BoolV(false),
    );
    assert_unsupported(result);
}

#[test]
fn set_eol_text_rejects_a_non_bool_hide_flag() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let empty: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let result = set_eol_text(
        &mut ctx,
        default_bid(),
        "test".to_string(),
        empty,
        SteelVal::StringV("yes".into()),
    );
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("#:hide-on-insert-line"), "got: {msg}");
}

#[test]
fn set_extra_highlights_without_decoration_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let empty: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let result = set_extra_highlights(&mut ctx, default_bid(), "test".to_string(), empty);
    assert_unsupported(result);
}

#[test]
fn set_extra_highlights_rejects_a_selected_scope() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let span = hashmap(vec![
        ("start-char", SteelVal::IntV(0)),
        ("end-char", SteelVal::IntV(1)),
        ("scope", SteelVal::StringV("keyword".into())),
        ("selected-scope", SteelVal::StringV("markup.bold".into())),
    ]);
    let result = set_extra_highlights(
        &mut ctx,
        default_bid(),
        "test".to_string(),
        list(vec![span]),
    );
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("unknown key 'selected-scope"), "got: {msg}");
}

#[test]
fn set_statusline_text_without_decoration_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = set_statusline_text(
        &mut ctx,
        default_bid(),
        "test".to_string(),
        SteelVal::StringV("main".into()),
    );
    assert_unsupported(result);
}

// ── `virtual_line_specs` decoder ─────────────────────────────────────────────
//
// These call the private decoder directly: `NullHost` has no `DecorationHost`,
// so a full `set_virtual_lines` round trip can't reach the store from here.
// Segment bounds, ordering, overlap and cluster widening are tested on
// `hume_rope::spanned::SpannedText::new`.

#[test]
fn virtual_line_spec_decodes_every_field() {
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(12)),
        ("side", SteelVal::SymbolV("before".into())),
        ("text", SteelVal::StringV("- let x = 5".into())),
        ("scope", SteelVal::StringV("diff.minus".into())),
        (
            "segments",
            list(vec![seg(0, 1, "diff.minus"), seg(2, 5, "keyword")]),
        ),
    ]);
    let specs = virtual_line_specs(list(vec![entry])).unwrap();
    assert_eq!(specs.len(), 1);
    let (line, spec) = &specs[0];
    assert_eq!(*line, 12);
    assert_eq!(spec.side, Side::Before);
    assert_eq!(spec.scope.as_deref(), Some("diff.minus"));
    assert_eq!(
        spec.content,
        SpannedText::new(
            "- let x = 5".to_string(),
            vec![
                (0, 1, "diff.minus".to_string()),
                (2, 5, "keyword".to_string())
            ]
        )
        .unwrap()
    );
}

#[test]
fn virtual_line_spec_rejects_a_segment_that_does_not_fit_naming_its_place() {
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("abcdef".into())),
        ("segments", list(vec![seg(0, 3, "a"), seg(2, 5, "b")])),
    ]);
    let err = virtual_line_specs(list(vec![entry]))
        .expect_err("overlapping segments must not decode")
        .to_string();
    assert!(
        err.contains("lines 0 'segments 1: (2, 5) overlaps (0, 3)"),
        "got: {err}"
    );
}

#[test]
fn a_shape_error_names_each_list_element_on_its_path() {
    let fine = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("abc".into())),
    ]);
    let bad_scope = hashmap(vec![
        ("start-char-col", SteelVal::IntV(0)),
        ("end-char-col", SteelVal::IntV(1)),
        ("scope", SteelVal::IntV(5)),
    ]);
    let bad = hashmap(vec![
        ("line", SteelVal::IntV(1)),
        ("text", SteelVal::StringV("abc".into())),
        ("segments", list(vec![seg(1, 2, "a"), bad_scope])),
    ]);
    let err = virtual_line_specs(list(vec![fine, bad]))
        .expect_err("a non-string scope must not decode")
        .to_string();
    assert!(
        err.contains("lines 1 'segments 1 'scope: expected a string"),
        "got: {err}"
    );
}

#[test]
fn virtual_line_spec_defaults_side_to_after_and_scope_to_none() {
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("x".into())),
    ]);
    let specs = virtual_line_specs(list(vec![entry])).unwrap();
    let (_, spec) = &specs[0];
    assert_eq!(spec.side, Side::After, "default side must be 'after");
    assert_eq!(spec.scope, None);
    assert!(spec.content.spans().is_empty());
}

#[test]
fn virtual_line_spec_rejects_positional_list_entry() {
    // A positional `(line text scope)` entry.
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
    // expected-keys list, which contains "segments" (which contains
    // "segment"), so that assertion would pass regardless of which key was
    // actually blamed. Assert on the phrase naming the offending key instead.
    assert!(err.contains("unknown key 'segment,"), "got: {err}");
}

/// A virtual line names its side with `'side`, the key an inlay hint uses.
#[test]
fn virtual_line_spec_rejects_an_anchor_key() {
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("x".into())),
        ("anchor", SteelVal::SymbolV("before".into())),
    ]);
    let err = virtual_line_specs(list(vec![entry]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown key 'anchor,"), "got: {err}");
}

/// A segment's offsets are char columns in the entry's own text, so it
/// takes the `'start-char-col` pair, not the buffer-offset `'start-char`.
#[test]
fn virtual_line_segment_rejects_buffer_offset_keys() {
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("abc".into())),
        (
            "segments",
            list(vec![hashmap(vec![
                ("start-char", SteelVal::IntV(0)),
                ("end-char-col", SteelVal::IntV(1)),
                ("scope", SteelVal::StringV("x".into())),
            ])]),
        ),
    ]);
    let err = virtual_line_specs(list(vec![entry]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown key 'start-char,"), "got: {err}");
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
fn virtual_line_spec_rejects_bad_side_symbol() {
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("x".into())),
        ("side", SteelVal::SymbolV("above".into())),
    ]);
    let err = virtual_line_specs(list(vec![entry]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("'before"), "got: {err}");
    assert!(err.contains("'after"), "got: {err}");
}

#[test]
fn virtual_line_spec_rejects_newline_in_text() {
    // A virtual line's display lines come from wrapping, never from its
    // text (`format_virtual_lines`); a raw newline would become one garbled
    // `CellContent::Virtual` cell instead of splitting the display line.
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
    // The engine expands a tab in a virtual line's text to the next tab
    // stop (`hume_engine`'s `format_virtual_lines`); this builtin no
    // longer expands it, or rejects it, itself.
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("\tx".into())),
    ]);
    let specs = virtual_line_specs(list(vec![entry])).expect("a tab is accepted");
    assert_eq!(specs[0].1.content.text(), "\tx");
}

#[test]
fn virtual_line_spec_keeps_a_control_character_verbatim() {
    // A control character other than tab is not this builtin's problem to
    // solve: the engine's own placeholder substitution
    // (`hume_rope::width::needs_placeholder`/`placeholder`) is what stands
    // between it and the terminal, the same chokepoint every other text
    // source (buffer content, inline inserts) goes through. Blanking it here
    // instead would be a second, weaker copy of that policy. Unrenderable
    // text is shown as its codepoint, never as a blank a bidi override
    // could hide behind.
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("a\u{7}b".into())), // BEL
    ]);
    let specs = virtual_line_specs(list(vec![entry])).expect("accepted, not rejected");
    assert_eq!(
        specs[0].1.content.text(),
        "a\u{7}b",
        "verbatim, no Steel-side substitution"
    );
}

#[test]
fn virtual_line_spec_rejects_segment_missing_a_key() {
    let entry = hashmap(vec![
        ("line", SteelVal::IntV(0)),
        ("text", SteelVal::StringV("abcdef".into())),
        (
            "segments",
            list(vec![hashmap(vec![
                ("start-char-col", SteelVal::IntV(0)),
                ("end-char-col", SteelVal::IntV(1)),
            ])]),
        ),
    ]);
    let err = virtual_line_specs(list(vec![entry]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("missing 'scope"), "got: {err}");
}
