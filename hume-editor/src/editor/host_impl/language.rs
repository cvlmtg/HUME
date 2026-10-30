//! `EditorHostImpl`'s grammar attachment and trigger-char registration.

use hume_scripting::GrammarReg;

use super::EditorHostImpl;
use hume_scripting::host::LanguageHost;

impl<'a> LanguageHost for EditorHostImpl<'a> {
    fn attach_grammar(&mut self, reg: &GrammarReg) -> Result<bool, String> {
        let attached = self
            .state
            .config
            .languages
            .attach_grammar(&reg.name, &reg.source, reg.origin, &mut self.view.registry)
            .map_err(|e| format!("'{}': {e}", reg.name))?
            .is_some();
        if !attached {
            self.state
                .report_shadowed("register-grammar!", format_args!("'{}' grammar", reg.name));
        }
        Ok(attached)
    }

    fn check_grammar(&self, name: &str, source: &hume_core::GrammarSource) -> Result<(), String> {
        self.state
            .config
            .languages
            .check_grammar(name, source)
            .map_err(|e| format!("'{name}': {e}"))
    }

    fn has_grammar(&self, language: &str) -> bool {
        self.state.config.languages.has_grammar(language)
    }
}
