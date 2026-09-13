//! `LanguageHost` — moved out of `host_impl.rs`'s per-capability split.

use hume_scripting::GrammarReg;

use super::EditorHostImpl;
use hume_scripting::host::LanguageHost;

impl<'a> LanguageHost for EditorHostImpl<'a> {
    fn attach_grammar(&mut self, reg: &GrammarReg) -> Result<(), String> {
        crate::editor::syntax::attach_grammar_from_reg(self.state, self.view, reg)?;
        Ok(())
    }

    fn has_grammar(&self, language: &str) -> bool {
        self.state.config.languages.has_grammar(language)
    }

    fn register_trigger_chars(&mut self, source: String, language: String, chars: Vec<char>) {
        if chars.is_empty() {
            self.state.config.trigger_chars.remove(&(source, language));
        } else {
            self.state
                .config
                .trigger_chars
                .insert((source, language), chars);
        }
    }
}
