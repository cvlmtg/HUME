use super::*;
use std::path::PathBuf;

// Golden-field check on the capability list: a server decides which
// features to offer from it. Assert the exact advertised
// set rather than just "it builds".
#[test]
#[allow(deprecated)] // asserting on the deprecated compat field
fn initialize_params_advertise_the_v1_capability_set() {
    #[cfg(windows)]
    let root = PathBuf::from(r"C:\tmp\proj");
    #[cfg(not(windows))]
    let root = PathBuf::from("/tmp/proj");
    let params = build_initialize_params(&root, None);

    assert_eq!(params.process_id, Some(std::process::id()));
    assert!(params.root_uri.is_some());
    let folders = params.workspace_folders.expect("workspace_folders set");
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].name, "proj");

    let caps = params.capabilities;
    assert_eq!(
        caps.general.unwrap().position_encodings,
        Some(vec![
            PositionEncodingKind::UTF8,
            PositionEncodingKind::UTF16
        ])
    );
    let td = caps.text_document.unwrap();
    let completion = td.completion.unwrap();
    let completion_item = completion.completion_item.unwrap();
    assert_eq!(completion_item.snippet_support, Some(false));
    assert_eq!(
        completion_item.documentation_format,
        Some(vec![MarkupKind::Markdown, MarkupKind::PlainText])
    );
    // Without this a conforming server never sends a list-wide `editRange`.
    assert_eq!(
        completion
            .completion_list
            .expect("completion_list capability must be declared")
            .item_defaults,
        Some(vec!["editRange".to_string()])
    );
    assert_eq!(
        td.hover.unwrap().content_format,
        Some(vec![MarkupKind::PlainText, MarkupKind::Markdown])
    );
    assert!(td.publish_diagnostics.is_some());
    assert!(td.rename.is_some());
    assert!(td.inlay_hint.is_some());
    // Without this, a conforming server sends each parameter as a bare
    // substring of the signature label instead of a `[start, end)` offset
    // pair into it, and `lsp-label-offsets->text` (the only reader of those
    // offsets) would then never see one.
    assert_eq!(
        td.signature_help
            .expect("signature_help capability must be declared")
            .signature_information
            .expect("signature_information must be declared")
            .parameter_information
            .expect("parameter_information must be declared")
            .label_offset_support,
        Some(true)
    );
    let ws = caps.workspace.unwrap();
    assert_eq!(ws.apply_edit, Some(true));
    assert_eq!(ws.configuration, Some(true));
    // Declared unconditionally: the push happens right after
    // `initialized` with no dynamic-registration negotiation.
    assert_eq!(
        ws.did_change_configuration
            .expect("did_change_configuration capability must be declared")
            .dynamic_registration,
        Some(false)
    );
    // Manual smoke testing found rust-analyzer refuses
    // textDocument/rename outright without this declared: every
    // rename result is a WorkspaceEdit, and some servers won't attempt
    // one unless the client has confirmed it can apply it.
    let we = ws
        .workspace_edit
        .expect("workspace_edit capability must be declared");
    assert_eq!(we.document_changes, Some(true));
    // Must be present or rust-analyzer refuses every rename outright
    // (confirmed live). HUME still can't actually apply a resource
    // op if one arrives (edits::collect_edit_entries rejects it by
    // design), but the alternative breaks the common case.
    assert_eq!(
        we.resource_operations,
        Some(vec![
            ResourceOperationKind::Create,
            ResourceOperationKind::Rename,
            ResourceOperationKind::Delete,
        ])
    );
    assert_eq!(we.failure_handling, Some(FailureHandlingKind::Abort));
    // Manual smoke testing found rust-analyzer withholds
    // diagnostic-derived quickfixes entirely without this declared:
    // a byte-perfect codeAction request still came back empty.
    let ca = td
        .code_action
        .expect("code_action capability must be declared");
    let literal = ca
        .code_action_literal_support
        .expect("code_action_literal_support must be declared");
    assert!(
        literal
            .code_action_kind
            .value_set
            .contains(&CodeActionKind::QUICKFIX.as_str().to_string())
    );
    assert_eq!(ca.is_preferred_support, Some(true));
    assert_eq!(ca.disabled_support, Some(true));
}

#[test]
fn diagnostics_refresh_is_declared_and_pull_is_not() {
    let caps = build_client_capabilities();

    assert_eq!(
        caps.workspace
            .unwrap()
            .diagnostic
            .and_then(|d| d.refresh_support),
        Some(true)
    );
    assert!(
        caps.text_document.unwrap().diagnostic.is_none(),
        "declaring pull makes a server that offers both stop pushing"
    );
}

#[test]
fn symbol_requests_declare_every_kind_and_hierarchical_documents() {
    let caps = build_client_capabilities();

    let document = caps.text_document.unwrap().document_symbol.unwrap();
    assert_eq!(document.hierarchical_document_symbol_support, Some(true));
    let workspace = caps.workspace.unwrap().symbol.unwrap();
    assert!(
        workspace.resolve_support.is_none(),
        "resolve is not advertised, so a server must send full locations"
    );
    for kinds in [document.symbol_kind, workspace.symbol_kind] {
        let numbers: Vec<_> = kinds
            .unwrap()
            .value_set
            .unwrap()
            .iter()
            .map(|kind| serde_json::to_value(kind).unwrap())
            .collect();
        let expected: Vec<_> = (1..=26).map(serde_json::Value::from).collect();
        assert_eq!(numbers, expected);
    }
}
