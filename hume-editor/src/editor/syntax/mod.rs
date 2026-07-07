//! Editor-side glue for the language registry: wiring buffer language changes
//! to hooks, lazy-plugin activation, and tree-sitter syntax setup.
//!
//! The registry itself, grammar attachment, and language detection live in
//! `hume_treesitter::registry` — this module only owns what's specific to a
//! live `Editor` (message log, hooks, plugin activation).

mod parse;

use hume_engine::pipeline::BufferId;
use hume_treesitter::registry::detect_language;
use steel::rvals::IntoSteelVal as _;

use super::Editor;
use hume_scripting::SteelBufferId;
use hume_scripting::hooks::HookId;

impl Editor {
    /// Set the language identity for buffer `bid`.
    ///
    /// No-op when the value is unchanged (avoids spurious hook fires).
    /// On change: writes `Buffer.language`, fires `OnLanguageSet` with `(bid, name-or-#f)`.
    /// All write paths (detection at open, `:set buffer language=`, Steel API) go
    /// through this function.
    pub(super) fn set_buffer_language(&mut self, bid: BufferId, new_lang: Option<String>) {
        if self.state.buffers.get(bid).language == new_lang {
            return;
        }
        let lang_val = match new_lang.as_deref() {
            Some(name) => name.into_steelval().expect("str into_steelval"),
            None => false.into_steelval().expect("bool into_steelval"),
        };
        // Clone before moving into the buffer so `activate_lazy_language_plugins`
        // can borrow the name after the write — a plugin reading buffer-language
        // during its own activation then sees the new value.
        let activate_name = new_lang.clone();
        self.state.buffers.get_mut(bid).language = new_lang;
        // Activate language-matched plugins after the write so handlers are
        // registered in time for the OnLanguageSet fire below.
        if let Some(name) = activate_name.as_deref() {
            self.activate_lazy_language_plugins(name);
        }
        let bid_val = SteelBufferId::new(bid).into_steel_val();
        self.fire_hook_silent(HookId::OnLanguageSet, &[bid_val, lang_val]);
        // Wire up (or tear down) tree-sitter highlighting for this buffer.
        self.setup_buffer_syntax(bid);
        // Spawn-or-attach an LSP server for this buffer's (possibly new)
        // language. Idempotent — `open_buffer`'s detect-then-fire-OnBufferOpen
        // sequence means this and the open-path hook can both observe the
        // same language-set.
        self.lsp_attach_buffer(bid);
    }

    pub(super) fn detect_and_set_language(&mut self, bid: BufferId) {
        let detected = {
            let buf = self.state.buffers.get(bid);
            let path = buf.path().map(|p| p.to_path_buf());
            let first_line = buf.first_line();
            detect_language(
                path.as_deref(),
                first_line.as_deref(),
                &self.state.languages,
            )
        };
        self.set_buffer_language(bid, detected);
    }

    /// Register languages from a drained `pending_language_regs` vec.
    /// Fail-soft: glob-set build failures are logged as warnings, editor continues.
    pub(super) fn apply_pending_language_regs(
        &mut self,
        regs: Vec<hume_scripting::PendingLanguageReg>,
    ) {
        use hume_scripting::PendingLanguageReg;
        let mut any_identity = false;
        let mut grammar_sweeps: Vec<String> = Vec::new();
        for reg in regs {
            match reg {
                PendingLanguageReg::Identity {
                    name,
                    extensions,
                    globs,
                    shebangs,
                } => {
                    let exts: Vec<&str> = extensions.iter().map(String::as_str).collect();
                    let shebangs_ref: Vec<&str> = shebangs.iter().map(String::as_str).collect();
                    let mut valid_globs: Vec<&str> = Vec::with_capacity(globs.len());
                    for g in &globs {
                        match globset::Glob::new(g) {
                            Ok(_) => valid_globs.push(g.as_str()),
                            Err(e) => self.state.message_log.push(
                                super::Severity::Warning,
                                format!("define-language! '{}': invalid glob '{}': {}", name, g, e),
                            ),
                        }
                    }
                    self.state.languages.register_identity_no_rebuild(
                        &name,
                        &exts,
                        &valid_globs,
                        &shebangs_ref,
                    );
                    any_identity = true;
                }
                PendingLanguageReg::Grammar {
                    name,
                    grammar_path,
                    symbol,
                    highlights_path,
                    injections_path,
                } => {
                    match self.state.languages.attach_grammar(
                        &name,
                        &grammar_path,
                        &symbol,
                        &highlights_path,
                        injections_path.as_deref(),
                        &mut self.view.registry,
                    ) {
                        Ok(_) => grammar_sweeps.push(name),
                        Err(e) => self.state.message_log.push(
                            super::Severity::Warning,
                            format!("register-grammar! '{}': {}", name, e),
                        ),
                    }
                }
            }
        }
        if any_identity && let Err(e) = self.state.languages.rebuild_glob_set() {
            self.state.message_log.push(
                super::Severity::Warning,
                format!("define-language!: glob set build failed: {e}"),
            );
        }
        if !grammar_sweeps.is_empty() {
            self.sweep_buffers_for_grammars(grammar_sweeps);
        }
    }

    /// Drain `host.pending_language_regs` and apply them.
    pub(super) fn flush_pending_language_regs(&mut self, host: &mut hume_scripting::ScriptingHost) {
        let regs = host.take_pending_language_regs();
        self.apply_pending_language_regs(regs);
    }
}
