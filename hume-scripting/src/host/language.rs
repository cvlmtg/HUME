//! Grammar attachment and trigger-char registration.

use crate::types::GrammarReg;

/// Grammar attachment and trigger-char registration, accessed through
/// [`EditorHost::language`](super::EditorHost::language).
pub trait LanguageHost {
    /// Register a grammar and load it now, for command-mode
    /// `register-grammar!`, so a grammar that cannot load fails the call.
    /// Init mode takes the other route: the identical payload, queued as
    /// `Effect::LanguageReg(PendingLanguageReg::Grammar(..))`, registers the
    /// grammar without loading it. Both reach the language registry's one
    /// registration path.
    fn attach_grammar(&mut self, reg: &GrammarReg) -> Result<(), String>;

    /// Whether `language` has a grammar, loaded or yet to load. A grammar
    /// whose load failed does not count.
    fn has_grammar(&self, language: &str) -> bool;
}
