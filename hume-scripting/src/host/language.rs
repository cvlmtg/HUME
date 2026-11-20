//! Grammar attachment and trigger-char registration.

use hume_core::{GrammarSource, HostToken};

use super::Captures;
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

    /// The matches of `query`, compiled against `language`'s grammar, over
    /// every layer of that grammar in the text `parsed` names, as
    /// [`super::SyntaxHost::syntax_captures`] gives them, with char offsets
    /// into that text. `parsed` names a text only while the call it was lent
    /// to runs; `Err` otherwise, when `language` has no grammar, or when
    /// `query` does not compile.
    fn parsed_text_captures(
        &mut self,
        parsed: HostToken,
        language: &str,
        query: &str,
    ) -> Result<Captures, String>;
}
