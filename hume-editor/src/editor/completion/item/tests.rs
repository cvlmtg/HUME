use super::*;

#[test]
fn strips_snippet_insert_text_only_when_format_is_snippet() {
    let v = serde_json::json!({
        "label": "foo",
        "insertText": "${1:foo}(${2:bar})",
        "insertTextFormat": 2,
    });
    let item = CompletionItem::from_json(v).expect("well-formed item");
    assert_eq!(item.insert_text, "foo(bar)");
    assert_eq!(
        item.raw.get("insertText").and_then(|v| v.as_str()),
        Some("${1:foo}(${2:bar})"),
        "raw must keep the pristine snippet text for on-completion-accept/resolve"
    );
}

#[test]
fn leaves_insert_text_untouched_without_snippet_format() {
    let v = serde_json::json!({
        "label": "foo",
        "insertText": "$100 literal",
    });
    let item = CompletionItem::from_json(v).expect("well-formed item");
    assert_eq!(item.insert_text, "$100 literal");
}

#[test]
fn strips_snippet_text_edit_new_text() {
    let v = serde_json::json!({
        "label": "foo",
        "insertTextFormat": 2,
        "textEdit": {
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
            "newText": "${1:foo}",
        },
    });
    let item = CompletionItem::from_json(v).expect("well-formed item");
    assert_eq!(item.text_edit.unwrap().new_text, "foo");
}

#[test]
fn a_string_kind_is_dropped_not_faked_as_a_default() {
    // A server sending a human-readable kind string instead of the LSP
    // numeric enum: `CompletionItemKind` is a transparent i32 newtype, so
    // this can't be made sense of — dropped, not defaulted.
    let v = serde_json::json!({"label": "foo", "kind": "Function"});
    let item = CompletionItem::from_json(v).expect("label present");
    assert_eq!(&*item.label, "foo");
    assert_eq!(item.kind, None);
    // Undefaulted text fields fall back to `label`.
    assert_eq!(item.sort_text, "foo");
    assert_eq!(item.filter_text, "foo");
    assert_eq!(item.insert_text, "foo");
}

#[test]
fn a_numeric_kind_decodes_to_its_typed_constant() {
    let v = serde_json::json!({"label": "ok", "kind": 3});
    let item = CompletionItem::from_json(v).expect("well-formed item");
    assert_eq!(&*item.label, "ok");
    assert_eq!(item.kind, Some(lsp_types::CompletionItemKind::FUNCTION));
}

#[test]
fn a_malformed_text_edit_drops_just_the_edit_not_the_item() {
    // `newText` missing fails both wire shapes (`Edit`/`InsertAndReplace`).
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
    let item = CompletionItem::from_json(v).expect("label present");
    assert_eq!(&*item.label, "bar");
    assert_eq!(item.detail.as_deref(), Some("a detail"));
    assert!(
        item.text_edit.is_none(),
        "a malformed textEdit must be dropped, not the whole item"
    );
}

#[test]
fn a_bare_string_decodes_as_a_plain_item_with_that_label() {
    let v = serde_json::json!("foobar");
    let item = CompletionItem::from_json(v).expect("a bare string is a well-formed item");
    assert_eq!(&*item.label, "foobar");
    assert_eq!(item.sort_text, "foobar");
    assert_eq!(item.filter_text, "foobar");
    assert_eq!(item.insert_text, "foobar");
    assert_eq!(item.kind, None);
    assert!(
        item.raw.is_null(),
        "no wire payload to keep — nothing parsed it from JSON"
    );
}

#[test]
fn missing_label_is_rejected() {
    let v = serde_json::json!({"kind": 1});
    assert!(
        CompletionItem::from_json(v).is_none(),
        "no label recoverable — item must be dropped"
    );
}

#[test]
fn non_string_label_is_rejected() {
    let v = serde_json::json!({"label": 42});
    assert!(CompletionItem::from_json(v).is_none());
}

// ── additionalTextEdits presence ─────────────────────────────────────────────
//
// `has_additional_text_edits` distinguishes "the server answered this key"
// from "the key is absent" — only the latter licenses `completionItem/
// resolve` (see the field's own doc). A JSON `null` must count as absent,
// the same as the key being missing entirely, not as "present."

#[test]
fn a_missing_additional_text_edits_key_is_absent() {
    let v = serde_json::json!({"label": "foo"});
    let item = CompletionItem::from_json(v).expect("well-formed item");
    assert!(!item.has_additional_text_edits);
}

#[test]
fn a_null_additional_text_edits_counts_as_absent_not_present() {
    let v = serde_json::json!({"label": "foo", "additionalTextEdits": null});
    let item = CompletionItem::from_json(v).expect("well-formed item");
    assert!(!item.has_additional_text_edits);
}

#[test]
fn an_empty_additional_text_edits_array_is_present() {
    let v = serde_json::json!({"label": "foo", "additionalTextEdits": []});
    let item = CompletionItem::from_json(v).expect("well-formed item");
    assert!(item.has_additional_text_edits);
    assert!(item.additional_text_edits.is_empty());
}

#[test]
fn a_non_empty_additional_text_edits_array_is_present() {
    let v = serde_json::json!({
        "label": "foo",
        "additionalTextEdits": [{
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
            "newText": "use foo;\n",
        }],
    });
    let item = CompletionItem::from_json(v).expect("well-formed item");
    assert!(item.has_additional_text_edits);
    assert_eq!(item.additional_text_edits.len(), 1);
}
