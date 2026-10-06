//! Grammar attachment and trigger-char registration.

use crate::types::GrammarReg;

/// Grammar attachment and trigger-char registration, accessed through
/// [`EditorHost::language`](super::EditorHost::language).
pub trait LanguageHost {
    /// Compile and attach a grammar now, for command-mode `register-grammar!`.
    /// Init mode takes the other route (the identical payload queued as
    /// `Effect::LanguageReg(PendingLanguageReg::Grammar(..))`), and both land
    /// on the same implementation behind this trait.
    fn attach_grammar(&mut self, reg: &GrammarReg) -> Result<(), String>;

    fn has_grammar(&self, language: &str) -> bool;
}
