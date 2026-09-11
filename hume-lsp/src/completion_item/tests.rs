use super::*;

#[test]
fn strip_snippet_default_placeholder_becomes_its_default_text() {
    assert_eq!(strip_snippet("${1:foo}"), "foo");
}

#[test]
fn strip_snippet_bare_tabstop_is_dropped() {
    assert_eq!(strip_snippet("before$0after"), "beforeafter");
}

#[test]
fn strip_snippet_multi_digit_tabstop_is_dropped() {
    assert_eq!(strip_snippet("$12"), "");
}

#[test]
fn strip_snippet_empty_default_becomes_empty_string() {
    assert_eq!(strip_snippet("${1:}"), "");
}

#[test]
fn strip_snippet_placeholder_with_no_colon_becomes_empty_string() {
    assert_eq!(strip_snippet("${1}"), "");
}

#[test]
fn strip_snippet_unterminated_placeholder_consumes_to_end_of_string() {
    assert_eq!(strip_snippet("${1:foo"), "foo");
}

#[test]
fn strip_snippet_the_lsp_md_documented_example() {
    assert_eq!(
        strip_snippet("for ${1:x} in ${2:iter} {\n    $0\n}"),
        "for x in iter {\n    \n}"
    );
}

#[test]
fn strip_snippet_leaves_plain_text_untouched() {
    assert_eq!(
        strip_snippet("no snippet syntax here"),
        "no snippet syntax here"
    );
}

#[test]
fn strip_snippet_a_dollar_followed_by_a_digit_is_always_a_tabstop_reference() {
    // "$5" is a bare tabstop ref (dropped) even mid-word — "$5.00" is
    // not special-cased as currency; only the digit run after `$` is
    // consumed.
    assert_eq!(strip_snippet("$5.00"), ".00");
}

#[test]
fn strip_snippet_a_dollar_with_no_following_brace_or_digit_is_copied_literally() {
    assert_eq!(strip_snippet("price: $x"), "price: $x");
}

#[test]
fn text_edit_from_json_lenient_reads_the_edit_shape() {
    let v = serde_json::json!({
        "range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 5}},
        "newText": "foo",
    });
    let te = text_edit_from_json_lenient(&v).expect("well-formed edit");
    assert_eq!(te.new_text, "foo");
    assert_eq!(te.range.start, lsp_types::Position::new(1, 2));
    assert_eq!(te.range.end, lsp_types::Position::new(1, 5));
}

#[test]
fn text_edit_from_json_lenient_prefers_the_narrower_insert_range() {
    // `InsertReplaceEdit` has both an `insert` and a wider `replace` range —
    // only `insert` is read, matching the strict-parse path's own choice
    // (see `StoredCompletionItem::from_typed` in `hume-editor`).
    let v = serde_json::json!({
        "insert": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 2}},
        "replace": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 9}},
        "newText": "foo",
    });
    let te = text_edit_from_json_lenient(&v).expect("well-formed edit");
    assert_eq!(te.range.end, lsp_types::Position::new(0, 2));
}

#[test]
fn text_edit_from_json_lenient_drops_a_malformed_edit() {
    let v = serde_json::json!({"range": {"start": {"line": 0, "character": 0}}});
    assert!(
        text_edit_from_json_lenient(&v).is_none(),
        "missing end/newText must be dropped, not panic or fabricate a value"
    );
}

#[test]
fn parse_additional_text_edits_lenient_collects_well_formed_entries_and_drops_malformed_ones() {
    let v = serde_json::json!({
        "additionalTextEdits": [
            {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}, "newText": "a"},
            {"range": {"start": {"line": 1, "character": 0}}},
        ],
    });
    let edits = parse_additional_text_edits_lenient(&v);
    assert_eq!(edits.len(), 1, "the malformed second entry is dropped");
    assert_eq!(edits[0].new_text, "a");
}

#[test]
fn parse_additional_text_edits_lenient_is_empty_when_the_key_is_absent() {
    let v = serde_json::json!({});
    assert!(parse_additional_text_edits_lenient(&v).is_empty());
}
