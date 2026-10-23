use std::sync::Arc;

use hume_engine::theme::ScopeRegistry;
use hume_treesitter::registry::{GrammarBundle, GrammarSnapshot, LanguageId, LanguageRegistry};
use hume_treesitter::syntax::InjectionLoader;

use crate::editor::{EditorState, Severity};

/// Loads registered grammars on the main thread for the parses that need
/// them: a markup popup's own language, and the language of each injected
/// layer a parse found registered but not loaded. A failed load is queued
/// in the registry and reported by
/// [`EditorState::report_failed_grammar_loads`].
pub(in crate::editor) struct GrammarLoads<'a> {
    languages: &'a mut LanguageRegistry,
    scopes: &'a mut ScopeRegistry,
}

impl<'a> GrammarLoads<'a> {
    pub(in crate::editor) fn new(
        languages: &'a mut LanguageRegistry,
        scopes: &'a mut ScopeRegistry,
    ) -> Self {
        Self { languages, scopes }
    }

    /// `id`'s grammar bundle, loading it if it is registered. `None` when
    /// `id` has no grammar or it fails to load.
    fn grammar(&mut self, id: LanguageId) -> Option<Arc<GrammarBundle>> {
        self.languages.load_grammar(id, self.scopes)
    }

    /// The grammar of the language named `name`, as [`Self::grammar`].
    pub(in crate::editor) fn grammar_named(&mut self, name: &str) -> Option<Arc<GrammarBundle>> {
        let id = self.languages.id_of(name)?;
        self.grammar(id)
    }

    /// The scope registry loads intern highlight captures into.
    pub(in crate::editor) fn scopes(&self) -> &ScopeRegistry {
        self.scopes
    }
}

impl InjectionLoader for GrammarLoads<'_> {
    fn snapshot(&self) -> Arc<GrammarSnapshot> {
        self.languages.grammar_snapshot()
    }

    /// Loads every one of `ids`, so each failure is queued.
    fn load(&mut self, ids: &[LanguageId]) -> bool {
        let mut any_loaded = false;
        for &id in ids {
            any_loaded |= self.grammar(id).is_some();
        }
        any_loaded
    }
}

impl EditorState {
    /// Report each grammar that failed to load since the last call as a
    /// warning. A failed grammar is never loaded again until it is registered
    /// again, so each failure is reported once.
    pub(in crate::editor) fn report_failed_grammar_loads(&mut self) {
        for (id, error) in self.config.languages.take_failed_loads() {
            let name = self.config.languages.name_of(id);
            let text = format!("grammar '{name}': {error}");
            self.report(Severity::Warning, text);
        }
    }
}
