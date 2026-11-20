//! `EditorHostImpl`'s grammar attachment and trigger-char registration.

use hume_core::HostToken;
use hume_rope::line::ContentLine;
use hume_rope::offset::ExclusiveRange;
use hume_scripting::GrammarReg;

use super::EditorHostImpl;
use super::syntax::captures_of;
use hume_scripting::host::{Captures, LanguageHost};

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

    fn parsed_text_captures(
        &mut self,
        parsed: HostToken,
        language: &str,
        query: &str,
    ) -> Result<Captures, String> {
        let markup = self
            .lent
            .filter(|&(token, _)| token == parsed)
            .map(|(_, markup)| markup)
            .ok_or_else(|| {
                "the parsed text is only valid during its format-text call".to_owned()
            })?;
        let (_, query) = self.capture_query(language, query)?;
        let mut matches = Vec::new();
        if let Some(layers) = markup.syntax.layers() {
            let text = &markup.text;
            let lines = [ExclusiveRange::new(
                ContentLine::default(),
                text.content_line_count().end_exclusive(),
            )];
            query.collect(layers, text.rope(), &lines, &mut matches);
        }
        Ok(captures_of(&query, matches))
    }
}
