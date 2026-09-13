//! Grammar attachment and trigger-char registration — moved out of
//! `host.rs`'s per-capability split.

use crate::types::GrammarReg;

/// Grammar attachment and trigger-char registration — accessed through
/// [`EditorHost::language`](super::EditorHost::language).
pub trait LanguageHost {
    /// Compile and attach a grammar now, for command-mode `register-grammar!`.
    /// Init mode takes the other route — the identical payload queued as
    /// `Effect::LanguageReg(PendingLanguageReg::Grammar(..))` — and both land
    /// on the same implementation behind this trait.
    fn attach_grammar(&mut self, reg: &GrammarReg) -> Result<(), String>;

    fn has_grammar(&self, language: &str) -> bool;

    /// `(register-trigger-chars! source language chars)` — registers `chars`
    /// as `OnTriggerChar`-firing chars for `(source, language)`, replacing
    /// that exact pair's previous set (a plugin's own reload doesn't
    /// accumulate duplicates; a second language attaching under the same
    /// source doesn't clobber the first's). An empty `chars` removes the
    /// entry.
    fn register_trigger_chars(&mut self, source: String, language: String, chars: Vec<char>);
}
