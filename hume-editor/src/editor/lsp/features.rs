//! The capability each routable [`LspFeature`] needs a server to advertise.

use hume_scripting::LspFeature;

/// Whether `capabilities` (a server's wire `ServerCapabilities`) carries
/// `key` with a value that is neither `false` nor `null`.
pub(in crate::editor) fn has_capability(capabilities: &serde_json::Value, key: &str) -> bool {
    capabilities
        .get(key)
        .is_some_and(|v| !matches!(v, serde_json::Value::Bool(false) | serde_json::Value::Null))
}

const FORMATTING: &str = "documentFormattingProvider";
const RANGE_FORMATTING: &str = "documentRangeFormattingProvider";

/// What a standard request method needs of a server beyond its feature.
pub(in crate::editor) struct Requirement {
    pub(in crate::editor) feature: LspFeature,
    /// The one capability key the method needs, where its feature spans
    /// methods that need different ones.
    pub(in crate::editor) capability: Option<&'static str>,
}

/// The feature `method` belongs to, or `None` for a method not tied to one
/// (a custom method, or one that continues an answer already received, like
/// `workspace/executeCommand`).
pub(in crate::editor) fn requirement(method: &str) -> Option<Requirement> {
    let (feature, capability) = match method {
        "textDocument/formatting" => (LspFeature::Format, Some(FORMATTING)),
        "textDocument/rangeFormatting" | "textDocument/rangesFormatting" => {
            (LspFeature::Format, Some(RANGE_FORMATTING))
        }
        "textDocument/declaration" => (LspFeature::GotoDeclaration, None),
        "textDocument/definition" => (LspFeature::GotoDefinition, None),
        "textDocument/typeDefinition" => (LspFeature::GotoTypeDefinition, None),
        "textDocument/references" => (LspFeature::GotoReference, None),
        "textDocument/implementation" => (LspFeature::GotoImplementation, None),
        "textDocument/signatureHelp" => (LspFeature::SignatureHelp, None),
        "textDocument/hover" => (LspFeature::Hover, None),
        "textDocument/documentHighlight" => (LspFeature::DocumentHighlight, None),
        "textDocument/completion" => (LspFeature::Completion, None),
        "textDocument/codeAction" => (LspFeature::CodeAction, None),
        "textDocument/documentLink" => (LspFeature::DocumentLinks, None),
        "textDocument/documentSymbol" => (LspFeature::DocumentSymbols, None),
        "workspace/symbol" => (LspFeature::WorkspaceSymbols, None),
        "textDocument/diagnostic" => (LspFeature::PullDiagnostics, None),
        "textDocument/rename" => (LspFeature::RenameSymbol, None),
        "textDocument/inlayHint" => (LspFeature::InlayHints, None),
        "textDocument/documentColor" => (LspFeature::DocumentColors, None),
        "textDocument/prepareCallHierarchy" => (LspFeature::CallHierarchy, None),
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
    keys.is_empty() || keys.iter().any(|key| has_capability(capabilities, key))
}

/// The value `capabilities` advertises for `feature`, or for the one
/// capability `method` needs: the first of the feature's keys that is
/// neither `false` nor `null`. `None` when the server advertises none, or
/// when `method` belongs to no feature. Exactly one of `feature` and
/// `method` is given.
pub(in crate::editor) fn provider<'a>(
    capabilities: &'a serde_json::Value,
    feature: Option<LspFeature>,
    method: Option<&str>,
) -> Option<&'a serde_json::Value> {
    let keys: Vec<&str> = match (feature, method) {
        (Some(feature), None) => capability_keys(feature).to_vec(),
        (None, Some(method)) => requirement(method).map_or_else(Vec::new, |r| match r.capability {
            Some(key) => vec![key],
            None => capability_keys(r.feature).to_vec(),
        }),
        _ => Vec::new(),
    };
    keys.into_iter()
        .find(|key| has_capability(capabilities, key))
        .and_then(|key| capabilities.get(key))
}

#[cfg(test)]
mod tests;
