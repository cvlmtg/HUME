//! Grammar attachment and trigger-char registration.

use hume_core::GrammarSource;

use crate::types::GrammarReg;

/// Grammar attachment and trigger-char registration, accessed through
/// [`EditorHost::language`](super::EditorHost::language).
pub trait LanguageHost {
    /// Register a grammar and load it now, for command-mode
    /// `register-grammar!`, so a grammar that cannot load fails the call.
    /// Init mode takes the other route: the identical payload, queued as
    /// `Effect::LanguageReg(PendingLanguageReg::Grammar(..))`, registers the
    /// grammar without loading it. Both reach the language registry's one
    /// registration path. `false`, opening nothing, when a higher origin's
    /// grammar holds the name.
    fn attach_grammar(&mut self, reg: &GrammarReg) -> Result<bool, String>;

    /// Load `source` as `name`'s grammar would load, registering nothing,
    /// so a grammar that cannot load fails the call in any session and
    /// whatever origin holds `name`.
    fn check_grammar(&self, name: &str, source: &GrammarSource) -> Result<(), String>;

    /// Whether `language` has a grammar, loaded or yet to load. A grammar
    /// whose load failed does not count.
    fn has_grammar(&self, language: &str) -> bool;
}
