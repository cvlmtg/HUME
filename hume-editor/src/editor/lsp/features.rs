//! The capability each routable [`LspFeature`] needs a server to advertise.

use hume_core::LspFeature;
use hume_scripting::CapabilityQuery;
use lsp_types::request::{
    CallHierarchyPrepare, CodeActionRequest, CodeActionResolveRequest, Completion, DocumentColor,
    DocumentDiagnosticRequest, DocumentHighlightRequest, DocumentLinkRequest,
    DocumentSymbolRequest, Formatting, GotoDeclaration, GotoDefinition, GotoImplementation,
    GotoTypeDefinition, HoverRequest, InlayHintRequest, RangeFormatting, References, Rename,
    Request as _, ResolveCompletionItem, SignatureHelpRequest, WorkspaceSymbolRequest,
};

/// The value at `path` in `capabilities` (a server's wire
/// `ServerCapabilities`), or `None` when a step is missing or the value is
/// `false` or `null`.
pub(in crate::editor) fn capability_at<'a>(
    capabilities: &'a serde_json::Value,
    path: &[&str],
) -> Option<&'a serde_json::Value> {
    let value = path
        .iter()
        .try_fold(capabilities, |value, key| value.get(key))?;
    (!matches!(
        value,
        serde_json::Value::Bool(false) | serde_json::Value::Null
    ))
    .then_some(value)
}

const FORMATTING: &str = "documentFormattingProvider";
const RANGE_FORMATTING: &str = "documentRangeFormattingProvider";

/// What a standard request method needs of a server beyond its feature.
pub(in crate::editor) struct Requirement {
    pub(in crate::editor) feature: LspFeature,
    /// The one capability the method needs, as a path into the server's
    /// capabilities, where its feature spans methods that need different
    /// ones.
    pub(in crate::editor) capability: Option<&'static [&'static str]>,
}

/// The feature `method` belongs to, or `None` for a method not tied to one
/// (a custom method, or one that continues an answer already received, like
/// `workspace/executeCommand`).
pub(in crate::editor) fn requirement(method: &str) -> Option<Requirement> {
    let (feature, capability) = match method {
        Formatting::METHOD => (LspFeature::Format, Some(&[FORMATTING][..])),
        RangeFormatting::METHOD => (LspFeature::Format, Some(&[RANGE_FORMATTING][..])),
        "textDocument/rangesFormatting" => (
            LspFeature::Format,
            Some(&[RANGE_FORMATTING, "rangesSupport"][..]),
        ),
        GotoDeclaration::METHOD => (LspFeature::GotoDeclaration, None),
        GotoDefinition::METHOD => (LspFeature::GotoDefinition, None),
        GotoTypeDefinition::METHOD => (LspFeature::GotoTypeDefinition, None),
        References::METHOD => (LspFeature::GotoReference, None),
        GotoImplementation::METHOD => (LspFeature::GotoImplementation, None),
        SignatureHelpRequest::METHOD => (LspFeature::SignatureHelp, None),
        HoverRequest::METHOD => (LspFeature::Hover, None),
        DocumentHighlightRequest::METHOD => (LspFeature::DocumentHighlight, None),
        Completion::METHOD => (LspFeature::Completion, None),
        ResolveCompletionItem::METHOD => (
            LspFeature::Completion,
            Some(&["completionProvider", "resolveProvider"][..]),
        ),
        CodeActionRequest::METHOD => (LspFeature::CodeAction, None),
        CodeActionResolveRequest::METHOD => (
            LspFeature::CodeAction,
            Some(&["codeActionProvider", "resolveProvider"][..]),
        ),
        DocumentLinkRequest::METHOD => (LspFeature::DocumentLinks, None),
        DocumentSymbolRequest::METHOD => (LspFeature::DocumentSymbols, None),
        WorkspaceSymbolRequest::METHOD => (LspFeature::WorkspaceSymbols, None),
        DocumentDiagnosticRequest::METHOD => (LspFeature::PullDiagnostics, None),
        Rename::METHOD => (LspFeature::RenameSymbol, None),
        InlayHintRequest::METHOD => (LspFeature::InlayHints, None),
        DocumentColor::METHOD => (LspFeature::DocumentColors, None),
        CallHierarchyPrepare::METHOD => (LspFeature::CallHierarchy, None),
        _ => return None,
    };
    Some(Requirement {
        feature,
        capability,
    })
}

/// The `ServerCapabilities` keys that advertise `feature`: any one present
/// suffices. Empty for a feature that needs none; diagnostics are pushed.
fn capability_keys(feature: LspFeature) -> &'static [&'static str] {
    match feature {
        LspFeature::Format => &[FORMATTING, RANGE_FORMATTING],
        LspFeature::GotoDeclaration => &["declarationProvider"],
        LspFeature::GotoDefinition => &["definitionProvider"],
        LspFeature::GotoTypeDefinition => &["typeDefinitionProvider"],
        LspFeature::GotoReference => &["referencesProvider"],
        LspFeature::GotoImplementation => &["implementationProvider"],
        LspFeature::SignatureHelp => &["signatureHelpProvider"],
        LspFeature::Hover => &["hoverProvider"],
        LspFeature::DocumentHighlight => &["documentHighlightProvider"],
        LspFeature::Completion => &["completionProvider"],
        LspFeature::CodeAction => &["codeActionProvider"],
        LspFeature::DocumentLinks => &["documentLinkProvider"],
        LspFeature::WorkspaceCommand => &["executeCommandProvider"],
        LspFeature::DocumentSymbols => &["documentSymbolProvider"],
        LspFeature::WorkspaceSymbols => &["workspaceSymbolProvider"],
        LspFeature::Diagnostics => &[],
        LspFeature::PullDiagnostics => &["diagnosticProvider"],
        LspFeature::RenameSymbol => &["renameProvider"],
        LspFeature::InlayHints => &["inlayHintProvider"],
        LspFeature::DocumentColors => &["colorProvider"],
        LspFeature::CallHierarchy => &["callHierarchyProvider"],
    }
}

/// Whether `capabilities` (a server's wire `ServerCapabilities`) advertises
/// `feature`.
pub(in crate::editor) fn advertises(feature: LspFeature, capabilities: &serde_json::Value) -> bool {
    let keys = capability_keys(feature);
    keys.is_empty()
        || keys
            .iter()
            .any(|key| capability_at(capabilities, &[key]).is_some())
}

/// What `capabilities` advertises for `query`: for a feature, the value of
/// the first of its keys that is neither `false` nor `null`; for a method,
/// the one capability it needs, or its feature's when it needs none in
/// particular. `None` when the server advertises none, or the method
/// belongs to no feature.
pub(in crate::editor) fn provider<'a>(
    capabilities: &'a serde_json::Value,
    query: CapabilityQuery<'_>,
) -> Option<&'a serde_json::Value> {
    let by_feature = |feature| {
        capability_keys(feature)
            .iter()
            .find_map(|key| capability_at(capabilities, &[key]))
    };
    match query {
        CapabilityQuery::Feature(feature) => by_feature(feature),
        CapabilityQuery::Method(method) => {
            let required = requirement(method)?;
            match required.capability {
                Some(path) => capability_at(capabilities, path),
                None => by_feature(required.feature),
            }
        }
    }
}

#[cfg(test)]
mod tests;
