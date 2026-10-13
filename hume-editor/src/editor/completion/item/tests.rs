use super::*;

/// Test-only stand-in for `EditorHostImpl::completion_emit`'s real call:
/// wraps `v` as the sole element of a one-item response handle and decodes
/// it through the real `from_json`, so `item.raw` (when present) resolves
/// back to `v` exactly as it would in production.
fn version() -> hume_editing::text::TextVersion {
    hume_editing::text::BufferText::from("\n").version()
}

fn from_json(v: serde_json::Value) -> Option<CompletionItem> {
    from_json_with_default(v, None)
}

/// [`from_json`] for an item of a list whose `itemDefaults.editRange` is
/// `default_range`.
fn from_json_with_default(
    v: serde_json::Value,
    default_range: Option<&lsp_types::Range>,
) -> Option<CompletionItem> {
    let response = hume_scripting::json::JsonHandle::new(serde_json::json!([v]));
    let serde_json::Value::Array(items) = response.value() else {
        unreachable!("just constructed as a one-element array")
    };
    let raw_item = response
        .indexed_child(None, 0)
        .expect("index 0 is within the one-element array just constructed");
    CompletionItem::from_json(&items[0], raw_item, default_range)
}

#[test]
fn strips_snippet_insert_text_only_when_format_is_snippet() {
    let v = serde_json::json!({
        "label": "foo",
        "insertText": "${1:foo}(${2:bar})",
        "insertTextFormat": 2,
    });
    let item = from_json(v).expect("well-formed item");
    assert_eq!(item.insert_text, "foo(bar)");
    assert_eq!(
        item.raw
            .as_ref()
            .and_then(|r| r.value().get("insertText"))
            .and_then(|v| v.as_str()),
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
    let item = from_json(v).expect("well-formed item");
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
    let item = from_json(v).expect("well-formed item");
    assert_eq!(item.text_edit.unwrap().new_text, "foo");
}

#[test]
fn a_string_kind_is_dropped_not_faked_as_a_default() {
    // A server sending a human-readable kind string instead of the LSP
    // numeric enum: `CompletionItemKind` is a transparent i32 newtype, so
    // this can't be made sense of: dropped, not defaulted.
    let v = serde_json::json!({"label": "foo", "kind": "Function"});
    let item = from_json(v).expect("label present");
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
    let item = from_json(v).expect("well-formed item");
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
    let item = from_json(v).expect("label present");
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
    let item = from_json(v).expect("a bare string is a well-formed item");
    assert_eq!(&*item.label, "foobar");
    assert_eq!(item.sort_text, "foobar");
    assert_eq!(item.filter_text, "foobar");
    assert_eq!(item.insert_text, "foobar");
    assert_eq!(item.kind, None);
    assert!(
        item.raw.is_none(),
        "no wire payload to keep: nothing parsed it from JSON"
    );
}

#[test]
fn missing_label_is_rejected() {
    let v = serde_json::json!({"kind": 1});
    assert!(
        from_json(v).is_none(),
        "no label recoverable: item must be dropped"
    );
}

#[test]
fn non_string_label_is_rejected() {
    let v = serde_json::json!({"label": 42});
    assert!(from_json(v).is_none());
}

// ── additionalTextEdits presence ─────────────────────────────────────────────
//
// `has_additional_text_edits` distinguishes "the server answered this key"
// from "the key is absent". Only the latter licenses `completionItem/
// resolve` (see the field's own doc). A JSON `null` must count as absent,
// the same as the key being missing entirely, not as "present."

#[test]
fn a_missing_additional_text_edits_key_is_absent() {
    let v = serde_json::json!({"label": "foo"});
    let item = from_json(v).expect("well-formed item");
    assert!(!item.has_additional_text_edits);
}

#[test]
fn a_null_additional_text_edits_counts_as_absent_not_present() {
    let v = serde_json::json!({"label": "foo", "additionalTextEdits": null});
    let item = from_json(v).expect("well-formed item");
    assert!(!item.has_additional_text_edits);
}

#[test]
fn an_empty_additional_text_edits_array_is_present() {
    let v = serde_json::json!({"label": "foo", "additionalTextEdits": []});
    let item = from_json(v).expect("well-formed item");
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
    let item = from_json(v).expect("well-formed item");
    assert!(item.has_additional_text_edits);
    assert_eq!(item.additional_text_edits.len(), 1);
}

fn default_range() -> lsp_types::Range {
    lsp_types::Range {
        start: lsp_types::Position::new(0, 4),
        end: lsp_types::Position::new(0, 7),
    }
}

/// An item with no `textEdit` of its own takes the list's default range,
/// with `textEditText` (else the label) as its new text.
#[test]
fn an_item_without_a_text_edit_takes_the_default_range_and_its_text_edit_text() {
    let range = default_range();
    let with_text = from_json_with_default(
        serde_json::json!({"label": "foo", "textEditText": "foo_text", "insertText": "ignored"}),
        Some(&range),
    )
    .expect("well-formed item");
    let te = with_text.text_edit.expect("default range applied");
    assert_eq!(te.range, range);
    assert_eq!(te.new_text, "foo_text");

    let label_only = from_json_with_default(serde_json::json!({"label": "bar"}), Some(&range))
        .expect("well-formed item");
    assert_eq!(
        label_only
            .text_edit
            .expect("default range applied")
            .new_text,
        "bar"
    );
}

#[test]
fn an_items_own_text_edit_wins_over_the_default_range() {
    let own = serde_json::json!({
        "label": "foo",
        "textEdit": {
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 2}},
            "newText": "foo",
        },
    });
    let item = from_json_with_default(own, Some(&default_range())).expect("well-formed item");
    assert_eq!(item.text_edit.expect("own edit").range.end.character, 2);
}

#[test]
fn a_snippet_item_strips_the_default_range_text_too() {
    let item = from_json_with_default(
        serde_json::json!({"label": "foo", "insertTextFormat": 2, "textEditText": "${1:foo}"}),
        Some(&default_range()),
    )
    .expect("well-formed item");
    assert_eq!(
        item.text_edit.expect("default range applied").new_text,
        "foo"
    );
}

/// An item whose own `textEdit` is malformed keeps no edit at all: the list's
/// default range is for items with none, and `insertText` stays the fallback.
#[test]
fn a_malformed_own_text_edit_does_not_take_the_default_range() {
    let v = serde_json::json!({
        "label": "foo",
        "insertText": "foo_insert",
        "textEdit": {"newText": "x"},
    });
    let item = from_json_with_default(v, Some(&default_range())).expect("well-formed item");
    assert_eq!(item.text_edit, None);
    assert_eq!(item.insert_text, "foo_insert");
}

// ── documentation ────────────────────────────────────────────────────────────

#[test]
fn a_string_documentation_decodes_as_plaintext() {
    let item = from_json(serde_json::json!({"label": "foo", "documentation": "Does foo."}))
        .expect("well-formed item");
    assert_eq!(
        item.documentation,
        Some(Documentation {
            text: "Does foo.".to_string(),
            markdown: false,
        })
    );
}

#[test]
fn a_markup_documentation_decodes_as_markdown_unless_its_kind_is_plaintext() {
    let markdown = from_json(serde_json::json!({
        "label": "foo",
        "documentation": {"kind": "markdown", "value": "Does `foo`."},
    }))
    .expect("well-formed item");
    assert_eq!(
        markdown.documentation,
        Some(Documentation {
            text: "Does `foo`.".to_string(),
            markdown: true,
        })
    );

    let plaintext = from_json(serde_json::json!({
        "label": "foo",
        "documentation": {"kind": "plaintext", "value": "Does foo."},
    }))
    .expect("well-formed item");
    assert_eq!(plaintext.documentation.map(|d| d.markdown), Some(false));
}

#[test]
fn an_items_docs_are_known_once_both_fields_came_with_it_or_resolved() {
    let mut bare = from_json(serde_json::json!({"label": "foo"})).expect("well-formed item");
    assert_eq!(bare.documentation, None);
    assert!(!bare.docs_known());
    bare.apply_resolved(serde_json::json!({"label": "foo"}), version());
    assert!(bare.docs_known());

    let documented = from_json(serde_json::json!({"label": "foo", "documentation": "Does foo."}))
        .expect("well-formed item");
    assert!(
        !documented.docs_known(),
        "a resolve may still supply the missing detail"
    );

    let complete = from_json(serde_json::json!({
        "label": "foo",
        "detail": "fn foo()",
        "documentation": "Does foo.",
    }))
    .expect("well-formed item");
    assert!(complete.docs_known());
}

#[test]
fn a_malformed_documentation_is_dropped_not_the_item() {
    let item =
        from_json(serde_json::json!({"label": "foo", "documentation": 42})).expect("label present");
    assert_eq!(item.documentation, None);
}

#[test]
fn the_doc_box_prefers_the_items_own_fields_over_the_resolved_ones() {
    let mut item = from_json(serde_json::json!({"label": "foo", "detail": "fn foo()"}))
        .expect("well-formed item");
    item.apply_resolved(
        serde_json::json!({
            "label": "foo",
            "detail": "fn foo() -> u8",
            "documentation": {"kind": "markdown", "value": "Does foo."},
        }),
        version(),
    );
    assert_eq!(
        item.doc_box_text("rust"),
        Some(Documentation {
            text: "```rust\nfn foo()\n```\n\nDoes foo.".to_string(),
            markdown: true,
        })
    );

    let mut bare = from_json(serde_json::json!({"label": "bar"})).expect("well-formed item");
    bare.apply_resolved(
        serde_json::json!({
            "label": "bar",
            "detail": "fn bar()",
            "documentation": "Does bar.",
        }),
        version(),
    );
    assert_eq!(
        bare.doc_box_text("rust"),
        Some(Documentation {
            text: "fn bar()\n\nDoes bar.".to_string(),
            markdown: false,
        })
    );
}

#[test]
fn a_resolved_detail_leaves_the_menu_row_unchanged() {
    let mut item = from_json(serde_json::json!({"label": "foo"})).expect("well-formed item");
    item.apply_resolved(
        serde_json::json!({"label": "foo", "detail": "fn foo() -> u8"}),
        version(),
    );
    assert_eq!(item.menu_row(true).trailing, None);
}

#[test]
fn the_menu_row_shows_detail_only_when_asked() {
    let item = from_json(serde_json::json!({"label": "foo", "detail": "fn foo()"}))
        .expect("well-formed item");
    assert_eq!(item.menu_row(true).trailing.as_deref(), Some("fn foo()"));
    assert_eq!(item.menu_row(false).trailing, None);
}

#[test]
fn a_detail_without_documentation_fills_the_doc_box_as_plain_text() {
    let item = from_json(serde_json::json!({"label": "foo", "detail": "fn foo()"}))
        .expect("well-formed item");
    assert_eq!(
        item.doc_box_text("rust"),
        Some(Documentation {
            text: "fn foo()".to_string(),
            markdown: false,
        })
    );

    let mut bare = from_json(serde_json::json!({"label": "bar"})).expect("well-formed item");
    assert_eq!(bare.doc_box_text("rust"), None);
    bare.apply_resolved(
        serde_json::json!({"label": "bar", "detail": "fn bar() -> u8"}),
        version(),
    );
    assert_eq!(
        bare.doc_box_text("rust"),
        Some(Documentation {
            text: "fn bar() -> u8".to_string(),
            markdown: false,
        })
    );
}

#[test]
fn a_resolution_carries_only_to_a_new_item_with_the_same_label_and_kind() {
    let mut previous = vec![
        from_json(serde_json::json!({"label": "foo", "kind": 3})).expect("well-formed item"),
        from_json(serde_json::json!({"label": "bar", "kind": 3})).expect("well-formed item"),
    ];
    for item in &mut previous {
        item.apply_resolved(serde_json::json!({"documentation": "Docs."}), version());
    }
    let mut items = vec![
        from_json(serde_json::json!({"label": "foo", "kind": 3})).expect("well-formed item"),
        from_json(serde_json::json!({"label": "bar", "kind": 6})).expect("well-formed item"),
    ];
    CompletionItem::carry_resolutions(&mut previous, &mut items);
    assert!(items[0].docs_known());
    assert!(!items[1].docs_known());
}
