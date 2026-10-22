//! The `initialize` request: the params the client opens a session with,
//! and the capabilities it advertises in them.

use lsp_types::{
    ClientCapabilities, ClientInfo, CodeActionClientCapabilities, CodeActionKind,
    CodeActionKindLiteralSupport, CodeActionLiteralSupport, CompletionClientCapabilities,
    CompletionItemCapability, CompletionListCapability, DiagnosticWorkspaceClientCapabilities,
    DidChangeConfigurationClientCapabilities, DocumentSymbolClientCapabilities,
    FailureHandlingKind, GeneralClientCapabilities, GotoCapability, HoverClientCapabilities,
    InitializeParams, MarkupKind, ParameterInformationSettings, PositionEncodingKind,
    PublishDiagnosticsClientCapabilities, RenameClientCapabilities, ResourceOperationKind,
    SignatureHelpClientCapabilities, SignatureInformationSettings, SymbolKind,
    SymbolKindCapability, TextDocumentClientCapabilities, TextDocumentSyncClientCapabilities,
    WindowClientCapabilities, WorkspaceClientCapabilities, WorkspaceEditClientCapabilities,
    WorkspaceFolder, WorkspaceSymbolClientCapabilities,
};

use crate::uri;

/// The serialized `initialize` params for a session rooted at `root`.
pub(crate) fn initialize_params_json(
    root: &std::path::Path,
    init_options: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut params = serde_json::to_value(build_initialize_params(root, init_options))
        .expect("InitializeParams always serializes");
    advertise_ranges_support(&mut params);
    params
}

/// Sets `capabilities.textDocument.rangeFormatting.rangesSupport = true` on
/// the serialized `initialize` params, advertising LSP 3.18's
/// `textDocument/rangesFormatting`. `lsp_types` 0.97 predates that
/// extension (`DocumentRangeFormattingClientCapabilities` is a bare
/// `DynamicRegistrationClientCapabilities` alias with no `rangesSupport`
/// field), so the flag is unrepresentable in the typed `ClientCapabilities`
/// built by `build_client_capabilities` and must be patched into the
/// already-serialized JSON instead. `range_formatting: Some(Default::
/// default())` there guarantees this pointer resolves to an (empty) object.
fn advertise_ranges_support(params: &mut serde_json::Value) {
    if let Some(range_formatting) = params
        .pointer_mut("/capabilities/textDocument/rangeFormatting")
        .and_then(serde_json::Value::as_object_mut)
    {
        range_formatting.insert("rangesSupport".to_string(), serde_json::Value::Bool(true));
    }
}

#[allow(deprecated)] // root_uri/root_path are deprecated in favor of workspace_folders,
// but rootUri compatibility is deliberate (older servers still read it).
fn build_initialize_params(
    root: &std::path::Path,
    init_options: Option<serde_json::Value>,
) -> InitializeParams {
    let root_uri = uri::path_to_uri(root).ok();
    let workspace_folders = root_uri.clone().map(|u| {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.to_string_lossy().into_owned());
        vec![WorkspaceFolder { uri: u, name }]
    });

    InitializeParams {
        process_id: Some(std::process::id()),
        root_uri,
        workspace_folders,
        capabilities: build_client_capabilities(),
        client_info: Some(ClientInfo {
            name: "hume".to_string(),
            version: None,
        }),
        initialization_options: init_options,
        ..Default::default()
    }
}

/// Every `SymbolKind` of the protocol, so a server never maps one to a
/// default kind for this client.
fn all_symbol_kinds() -> SymbolKindCapability {
    SymbolKindCapability {
        value_set: Some(vec![
            SymbolKind::FILE,
            SymbolKind::MODULE,
            SymbolKind::NAMESPACE,
            SymbolKind::PACKAGE,
            SymbolKind::CLASS,
            SymbolKind::METHOD,
            SymbolKind::PROPERTY,
            SymbolKind::FIELD,
            SymbolKind::CONSTRUCTOR,
            SymbolKind::ENUM,
            SymbolKind::INTERFACE,
            SymbolKind::FUNCTION,
            SymbolKind::VARIABLE,
            SymbolKind::CONSTANT,
            SymbolKind::STRING,
            SymbolKind::NUMBER,
            SymbolKind::BOOLEAN,
            SymbolKind::ARRAY,
            SymbolKind::OBJECT,
            SymbolKind::KEY,
            SymbolKind::NULL,
            SymbolKind::ENUM_MEMBER,
            SymbolKind::STRUCT,
            SymbolKind::EVENT,
            SymbolKind::OPERATOR,
            SymbolKind::TYPE_PARAMETER,
        ]),
    }
}

fn build_client_capabilities() -> ClientCapabilities {
    ClientCapabilities {
        workspace: Some(WorkspaceClientCapabilities {
            apply_edit: Some(true),
            configuration: Some(true),
            // No dynamic registration path exists: the push happens
            // unconditionally right after `initialized`, so this only
            // needs to be present, not negotiated.
            did_change_configuration: Some(DidChangeConfigurationClientCapabilities {
                dynamic_registration: Some(false),
            }),
            workspace_folders: Some(true),
            // Lets a pull-only server ask for a re-pull. The client does not
            // declare `textDocument.diagnostic`: a server that offers both
            // would then stop pushing.
            diagnostic: Some(DiagnosticWorkspaceClientCapabilities {
                refresh_support: Some(true),
            }),
            // Every rename result is a WorkspaceEdit. Some servers
            // (rust-analyzer) refuse textDocument/rename outright without
            // this declared, since they can't otherwise confirm the client
            // can apply one (found via manual smoke testing).
            //
            // `resource_operations` must be present (non-empty) or
            // rust-analyzer refuses *every* rename outright (confirmed
            // live). That covers more than the file-rename-adjacent case
            // below. `workspace_edit::
            // file_edits` has no HUME equivalent for an actual
            // `DocumentChangeOperation::Op` and rejects the whole edit if
            // one ever arrives (a rename whose target shares its name with
            // its containing module/file, which rust-analyzer folds a file
            // rename into) is a real, occasional, safe-by-design rejection,
            // not the common case, and the alternative (never declaring
            // resource_operations) breaks every rename instead.
            workspace_edit: Some(WorkspaceEditClientCapabilities {
                document_changes: Some(true),
                resource_operations: Some(vec![
                    ResourceOperationKind::Create,
                    ResourceOperationKind::Rename,
                    ResourceOperationKind::Delete,
                ]),
                failure_handling: Some(FailureHandlingKind::Abort),
                ..Default::default()
            }),
            // Resolve is not advertised: a server must send each symbol's
            // full location.
            symbol: Some(WorkspaceSymbolClientCapabilities {
                symbol_kind: Some(all_symbol_kinds()),
                ..Default::default()
            }),
            ..Default::default()
        }),
        text_document: Some(TextDocumentClientCapabilities {
            synchronization: Some(TextDocumentSyncClientCapabilities {
                did_save: Some(true),
                ..Default::default()
            }),
            publish_diagnostics: Some(PublishDiagnosticsClientCapabilities::default()),
            hover: Some(HoverClientCapabilities {
                content_format: Some(vec![MarkupKind::PlainText, MarkupKind::Markdown]),
                ..Default::default()
            }),
            completion: Some(CompletionClientCapabilities {
                completion_item: Some(CompletionItemCapability {
                    // v1 strips snippet placeholders to plain text.
                    snippet_support: Some(false),
                    // Markdown first: the documentation box highlights it.
                    documentation_format: Some(vec![MarkupKind::Markdown, MarkupKind::PlainText]),
                    ..Default::default()
                }),
                completion_list: Some(CompletionListCapability {
                    item_defaults: Some(vec!["editRange".to_string()]),
                }),
                ..Default::default()
            }),
            // Without `label_offset_support` a server must describe each
            // parameter by repeating its text, which only *names* the
            // parameter; locating it back inside the signature label means
            // substring-searching for it, and a label like
            // `fn f(a: T, b: T)` has no unique match to find. The offset
            // form says where it is outright, which is what marking the
            // active parameter in place will need. The offsets count code
            // units in the negotiated encoding, so they convert host-side
            // (`lsp-label-offsets->text`): Scheme has no way to know what
            // was negotiated.
            signature_help: Some(SignatureHelpClientCapabilities {
                signature_information: Some(SignatureInformationSettings {
                    parameter_information: Some(ParameterInformationSettings {
                        label_offset_support: Some(true),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            rename: Some(RenameClientCapabilities::default()),
            references: Some(Default::default()),
            definition: Some(GotoCapability::default()),
            declaration: Some(GotoCapability::default()),
            type_definition: Some(GotoCapability::default()),
            implementation: Some(GotoCapability::default()),
            formatting: Some(Default::default()),
            range_formatting: Some(Default::default()),
            // rust-analyzer withholds diagnostic-derived quickfixes
            // entirely without code_action_literal_support declared (the
            // flag saying the client understands CodeAction objects, not
            // just legacy Command[]). Without it, even a byte-perfect
            // request (correct diagnostic round-tripped verbatim, correct
            // overlapping range) comes back empty.
            code_action: Some(CodeActionClientCapabilities {
                code_action_literal_support: Some(CodeActionLiteralSupport {
                    code_action_kind: CodeActionKindLiteralSupport {
                        value_set: vec![
                            CodeActionKind::QUICKFIX.as_str().to_string(),
                            CodeActionKind::REFACTOR.as_str().to_string(),
                            CodeActionKind::REFACTOR_EXTRACT.as_str().to_string(),
                            CodeActionKind::REFACTOR_INLINE.as_str().to_string(),
                            CodeActionKind::REFACTOR_REWRITE.as_str().to_string(),
                            CodeActionKind::SOURCE.as_str().to_string(),
                            CodeActionKind::SOURCE_ORGANIZE_IMPORTS.as_str().to_string(),
                        ],
                    },
                }),
                is_preferred_support: Some(true),
                disabled_support: Some(true),
                ..Default::default()
            }),
            inlay_hint: Some(Default::default()),
            document_symbol: Some(DocumentSymbolClientCapabilities {
                symbol_kind: Some(all_symbol_kinds()),
                hierarchical_document_symbol_support: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        }),
        general: Some(GeneralClientCapabilities {
            position_encodings: Some(vec![
                PositionEncodingKind::UTF8,
                PositionEncodingKind::UTF16,
            ]),
            ..Default::default()
        }),
        // rust-analyzer (and others) gate server-initiated `$/progress`
        // (indexing/loading status) on this flag. Without it, no progress
        // notifications are sent at all, so the editor has no way to show
        // load status beyond the sub-second `initialize` handshake.
        window: Some(WindowClientCapabilities {
            work_done_progress: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests;
