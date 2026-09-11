use super::*;

/// End-to-end: `from_typed` only strips when the server declared
/// `insertTextFormat: Snippet` (2) — a plain-text item's `$` literals
/// must survive untouched.
#[test]
fn from_typed_strips_snippet_insert_text_only_when_format_is_snippet() {
    let v = serde_json::json!({
        "label": "foo",
        "insertText": "${1:foo}(${2:bar})",
        "insertTextFormat": 2,
    });
    let item = StoredCompletionItem::from_json(&v).expect("well-formed item");
    assert_eq!(item.insert_text, "foo(bar)");
    assert_eq!(
        item.raw.get("insertText").and_then(|v| v.as_str()),
        Some("${1:foo}(${2:bar})"),
        "raw must keep the pristine snippet text for on-completion-accept/resolve"
    );
}

#[test]
fn from_typed_leaves_insert_text_untouched_without_snippet_format() {
    let v = serde_json::json!({
        "label": "foo",
        "insertText": "$100 literal",
    });
    let item = StoredCompletionItem::from_json(&v).expect("well-formed item");
    assert_eq!(item.insert_text, "$100 literal");
}

#[test]
fn from_typed_strips_snippet_text_edit_new_text() {
    let v = serde_json::json!({
        "label": "foo",
        "insertTextFormat": 2,
        "textEdit": {
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
            "newText": "${1:foo}",
        },
    });
    let item = StoredCompletionItem::from_json(&v).expect("well-formed item");
    assert_eq!(item.text_edit.unwrap().new_text, "foo");
}

/// `from_json_lenient` (the recovery path for an off-spec item) must
/// strip snippet syntax too — not just the strict `from_typed` path.
#[test]
fn from_json_lenient_also_strips_snippet_insert_text() {
    // A non-numeric `kind` forces the whole item through the lenient
    // fallback (same trick as `string_kind_recovers_via_lenient_fallback`
    // below), while `insertTextFormat`/`insertText` stay well-formed.
    let v = serde_json::json!({
        "label": "foo",
        "kind": "Function",
        "insertTextFormat": 2,
        "insertText": "${1:foo}",
    });
    assert_strict_parse_fails(&v);
    let item = StoredCompletionItem::from_json(&v).expect("label present — must recover");
    assert_eq!(item.insert_text, "foo");
    assert_eq!(
        item.raw.get("insertText").and_then(|v| v.as_str()),
        Some("${1:foo}"),
        "raw must keep the pristine snippet text"
    );
}

/// Independent oracle: strict `lsp_types::CompletionItem` deserialize
/// really does reject `v` on its own — otherwise a test using this
/// wouldn't be exercising `from_json_lenient` at all, just re-testing
/// the strict path.
fn assert_strict_parse_fails(v: &serde_json::Value) {
    assert!(
        serde_json::from_value::<lsp_types::CompletionItem>(v.clone()).is_err(),
        "test input must be strict-parse-rejecting to exercise the lenient fallback: {v}"
    );
}

#[test]
fn well_formed_item_never_touches_the_lenient_path() {
    // Sanity check for the two tests below: a spec-compliant item must
    // NOT need `from_json_lenient` — if this failed, `strict_parse_fails`
    // in those tests wouldn't prove anything.
    let v = serde_json::json!({"label": "ok", "kind": 3});
    assert!(serde_json::from_value::<lsp_types::CompletionItem>(v.clone()).is_ok());
    let item = StoredCompletionItem::from_json(&v).expect("well-formed item");
    assert_eq!(item.label, "ok");
    assert_eq!(item.kind, Some(3));
}

#[test]
fn string_kind_recovers_via_lenient_fallback() {
    // A server sending a human-readable kind string instead of the LSP
    // numeric enum: `CompletionItemKind` is a transparent i32 newtype,
    // so a JSON string for `kind` fails strict deserialize of the whole
    // item, not just that field.
    let v = serde_json::json!({"label": "foo", "kind": "Function"});
    assert_strict_parse_fails(&v);

    let item = StoredCompletionItem::from_json(&v).expect("label present — must recover");
    assert_eq!(item.label, "foo");
    // The lenient reader can't make sense of a non-numeric kind either
    // — dropped, not faked as some default kind.
    assert_eq!(item.kind, None);
    // Undefaulted text fields still fall back to `label`, same as the
    // strict path's `unwrap_or_else(|| label.clone())`.
    assert_eq!(item.sort_text, "foo");
    assert_eq!(item.filter_text, "foo");
    assert_eq!(item.insert_text, "foo");
}

#[test]
fn malformed_text_edit_recovers_the_item_without_the_edit() {
    // `newText` missing fails both `CompletionTextEdit` union variants
    // (`Edit`/`InsertAndReplace`), which fails the whole item's strict
    // parse even though only the edit is broken.
    let v = serde_json::json!({
        "label": "bar",
        "detail": "a detail",
        "textEdit": {
            "range": {
                "start": {"line": 1, "character": 2},
                "end": {"line": 1, "character": 5},
            },
        },
    });
    assert_strict_parse_fails(&v);

    let item = StoredCompletionItem::from_json(&v).expect("label present — must recover");
    assert_eq!(item.label, "bar");
    assert_eq!(item.detail.as_deref(), Some("a detail"));
    assert!(
        item.text_edit.is_none(),
        "a malformed textEdit must be dropped, not the whole item"
    );
}

#[test]
fn missing_label_is_rejected_by_both_strict_and_lenient() {
    let v = serde_json::json!({"kind": 1});
    assert_strict_parse_fails(&v);
    assert!(
        StoredCompletionItem::from_json(&v).is_err(),
        "no label recoverable — item must still be dropped"
    );
}
