//! The vocabulary of the editor's language-server registry, which scripting
//! reads and writes too: server names, registrations, per-language server
//! lists and the features they filter by.

use crate::Tie;

/// A server registration: a later write from one origin replaces the
/// earlier, whether the editor applies it or a script reads it back before
/// then.
pub const SERVER_REGISTRATION_TIE: Tie = Tie::LaterWins;

/// A language server's registration name: non-empty, no whitespace, so it
/// reads unambiguously as a `:lsp-stop`/`:lsp-restart` argument.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ServerName(std::sync::Arc<str>);

impl ServerName {
    /// `Err` names what is wrong with `name`, for a builtin to prefix with
    /// its own name.
    pub fn parse(name: &str) -> Result<Self, String> {
        if name.is_empty() {
            return Err("server name is empty".to_string());
        }
        if name.chars().any(char::is_whitespace) {
            return Err(format!("server name {name:?} contains whitespace"));
        }
        Ok(Self(std::sync::Arc::from(name)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ServerName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

named_enum! {
    /// A language-server capability requests are routed by: Helix's
    /// `language-servers` feature vocabulary, spelled the same way so a
    /// Helix `only-features`/`except-features` list carries over verbatim.
    /// Features HUME has no command for are still valid names.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum LspFeature {
        Format => "format",
        GotoDeclaration => "goto-declaration",
        GotoDefinition => "goto-definition",
        GotoTypeDefinition => "goto-type-definition",
        GotoReference => "goto-reference",
        GotoImplementation => "goto-implementation",
        SignatureHelp => "signature-help",
        Hover => "hover",
        DocumentHighlight => "document-highlight",
        Completion => "completion",
        CodeAction => "code-action",
        DocumentLinks => "document-links",
        WorkspaceCommand => "workspace-command",
        DocumentSymbols => "document-symbols",
        WorkspaceSymbols => "workspace-symbols",
        Diagnostics => "diagnostics",
        PullDiagnostics => "pull-diagnostics",
        RenameSymbol => "rename-symbol",
        InlayHints => "inlay-hints",
        DocumentColors => "document-colors",
        CallHierarchy => "call-hierarchy",
    }
}

/// A set of [`LspFeature`]s, one bit per variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LspFeatureSet(u32);

impl LspFeatureSet {
    pub fn contains(self, feature: LspFeature) -> bool {
        self.0 & (1 << feature as u32) != 0
    }

    /// Members in [`LspFeature::ALL`] order.
    pub fn iter(self) -> impl Iterator<Item = LspFeature> {
        LspFeature::ALL
            .into_iter()
            .filter(move |f| self.contains(*f))
    }
}

impl FromIterator<LspFeature> for LspFeatureSet {
    fn from_iter<I: IntoIterator<Item = LspFeature>>(iter: I) -> Self {
        Self(
            iter.into_iter()
                .fold(0, |bits, feature| bits | 1 << feature as u32),
        )
    }
}

/// Which features a server may be asked for within one language's server
/// list: Helix's `only-features`/`except-features`, or neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureFilter {
    All,
    Only(LspFeatureSet),
    Except(LspFeatureSet),
}

impl FeatureFilter {
    pub fn admits(self, feature: LspFeature) -> bool {
        match self {
            FeatureFilter::All => true,
            FeatureFilter::Only(set) => set.contains(feature),
            FeatureFilter::Except(set) => !set.contains(feature),
        }
    }
}

/// One entry of a language's ordered server list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    pub name: ServerName,
    pub filter: FeatureFilter,
}

/// Which of a language's server lists a write sets. A default list ranks
/// below every explicit list, whatever the origins of the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListTier {
    Explicit,
    Default,
}

/// The process a `register-lsp-server!` call spawns and configures.
#[derive(Debug, Clone, PartialEq)]
pub struct LspServerConfig {
    pub command: String,
    pub args: Vec<String>,
    /// Sent verbatim as `initializationOptions` in the `initialize` request.
    pub init_options: Option<serde_json::Value>,
    /// Pushed as `workspace/didChangeConfiguration` after `initialized`, and
    /// resolved per item to answer `workspace/configuration` pull requests
    /// from the copy the instance was spawned with.
    pub settings: Option<serde_json::Value>,
    /// `#:env`: applied additively to the spawned process's inherited
    /// environment, never clearing it.
    pub env: Vec<(String, String)>,
}

#[cfg(test)]
mod tests;
