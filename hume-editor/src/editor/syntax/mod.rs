//! Editor-side glue for the language registry: wiring buffer language changes
//! to hooks, lazy-plugin activation, and tree-sitter syntax setup.
//!
//! The registry itself, grammar registration and loading, and language
//! detection live in `hume_treesitter::registry`; this module only owns what's
//! specific to a live `Editor` (message log, hooks, plugin activation, and
//! loading a grammar when a buffer first needs it).

mod loads;
mod parse;

pub(in crate::editor) use loads::GrammarLoads;
pub(in crate::editor) use parse::{SyntaxUse, ensure_syntax_current};

use hume_core::{
    BufferId, BufferIntrinsicOption, ConfigOrigin, LanguageFields, Severity, WriteOutcome,
};
use hume_treesitter::registry::{DetectPaths, LanguageId, detect_language};

use super::Editor;
use super::event::EditorEvent;
use super::host_impl::EditorHostImpl;

impl Editor {
    /// Set the language identity for buffer `bid`, via plain detection:
    /// records no origin. A buffer whose language a user or script asserted
    /// (`language_by`) never gets here (see `set_buffer_language_explicit`
    /// for the user/script write paths).
    pub(super) fn set_buffer_language(&mut self, bid: BufferId, new_lang: Option<LanguageId>) {
        debug_assert!(
            self.state.buffers.get(bid).language_by.by().is_none(),
            "detection must leave an asserted language alone"
        );
        self.set_buffer_language_impl(bid, new_lang);
    }

    /// A user or script `language` option value from `by`: `None` clears the
    /// language, and a name no `define-language!` registered is reported but
    /// still applied (a grammar or plugin may register it later). The one
    /// parser `:set buffer language=` and `(set-buffer-option! pane
    /// "language" …)` share.
    pub(super) fn set_buffer_language_by_name(
        &mut self,
        bid: BufferId,
        name: Option<&str>,
        by: ConfigOrigin,
    ) {
        if !self.claim_language(bid, by) {
            return;
        }
        let new_lang = name.map(|name| {
            if self.state.config.languages.defined_id(name).is_none() {
                self.report(
                    Severity::Info,
                    format!("language '{name}' is not registered"),
                );
            }
            self.state.config.languages.intern(name)
        });
        self.set_buffer_language_impl(bid, new_lang);
    }

    /// Set the language identity for buffer `bid` from a user or script
    /// assertion (`:set buffer language=`, `set-buffer-option!`'s `"language"`)
    /// rather than detection, unless a higher origin asserted the current
    /// one. Stamps `language_by` so `:reload-config`'s reset can restore the
    /// assertion across the reload instead of letting its post-reload
    /// re-detect sweep pick something else (see `clear_languages_all`).
    pub(super) fn set_buffer_language_explicit(
        &mut self,
        bid: BufferId,
        new_lang: Option<LanguageId>,
        by: ConfigOrigin,
    ) {
        if self.claim_language(bid, by) {
            self.set_buffer_language_impl(bid, new_lang);
        }
    }

    /// Records `by` as the origin of `bid`'s language unless a higher origin
    /// asserted it, which is traced. Stamped before the write, so it records
    /// how the current value arrived whether or not the write changes it.
    fn claim_language(&mut self, bid: BufferId, by: ConfigOrigin) -> bool {
        let claimed = self
            .state
            .buffers
            .get_mut(bid)
            .language_by
            .claim(by)
            .is_applied();
        if !claimed {
            self.state.report_shadowed(
                "option",
                format_args!("'{}' value", BufferIntrinsicOption::Language.name()),
            );
        }
        claimed
    }

    /// No-op when the value is unchanged (avoids spurious hook fires).
    /// On change: writes `Buffer.language`, fires `OnLanguageSet` with `(bid, name-or-#f)`.
    fn set_buffer_language_impl(&mut self, bid: BufferId, new_lang: Option<LanguageId>) {
        if self.state.buffers.get(bid).language == new_lang {
            return;
        }
        // Bundled data and lazy plugin activation both run through the host;
        // a change made without one would be recorded without either.
        assert!(
            self.scripting.is_some(),
            "a buffer's language changes only with a scripting host attached"
        );
        let lang_name = new_lang.map(|id| self.state.config.languages.name_of(id).to_owned());
        self.state.buffers.get_mut(bid).language = new_lang;
        if let Some(id) = new_lang {
            self.load_bundled_language(id);
        }
        // Activate language-matched plugins after the write so handlers are
        // registered in time for the OnLanguageSet fire below.
        if let Some(name) = lang_name.as_deref() {
            self.activate_lazy_language_plugins(name);
            // A lazy plugin's own body can call `set-buffer-option!`'s `"language"` on
            // this same buffer (applied inline via `apply_script_effects`
            // before `activate_lazy_language_plugins` returns); that nested
            // call already ran this function to completion for the newer
            // value. Ours is stale: bail out rather than fire a second,
            // out-of-order `OnLanguageSet` and re-derive syntax/LSP state
            // for a language the buffer no longer has.
            if self.state.buffers.get(bid).language != new_lang {
                return;
            }
        }
        self.state.queue_event(EditorEvent::OnLanguageSet {
            buffer: bid,
            language: lang_name,
        });
        // Wire up (or tear down) tree-sitter highlighting for this buffer.
        self.setup_buffer_syntax(bid);
        // Attach this buffer to the servers its (possibly new, possibly
        // absent) language plans, detaching any it no longer does.
        self.state.lsp_reconcile_buffer(&self.view, bid);
    }

    /// Read `id`'s bundled data file (`<runtime>/scheme/languages/<name>.scm`)
    /// the first time a buffer uses the language or a grammar is registered
    /// for it in this configuration, and fill each field configuration left
    /// unset.
    /// A missing file or runtime directory leaves the language without
    /// bundled data; a malformed file is reported and does the same. Either
    /// way the file is not read again until `:reload-config` rebuilds the
    /// registry.
    fn load_bundled_language(&mut self, id: LanguageId) {
        if self.state.config.languages.bundled_loaded(id) {
            return;
        }
        let name = self.state.config.languages.name_of(id).to_owned();
        let scripting = self
            .scripting
            .as_mut()
            .expect("a buffer has a language, and so syntax layers, only with a scripting host");
        let mut host = EditorHostImpl::new(&mut self.state, &mut self.view);
        let lang = match scripting.bundled_language(&name, &mut host) {
            Ok(Some(lang)) => lang,
            Ok(None) => LanguageFields::default(),
            Err(e) => {
                self.report(Severity::Error, format!("language data: {e}"));
                LanguageFields::default()
            }
        };
        self.state.config.languages.fill_bundled(id, lang);
    }

    pub(super) fn detect_and_set_language(&mut self, bid: BufferId) {
        // A view's text is generated, not a file, so nothing in it names a
        // language.
        if self.state.buffers.get(bid).is_synthetic() {
            return;
        }
        // A user or script asserted this buffer's language: detection leaves
        // it as it is.
        if self.state.buffers.get(bid).language_by.by().is_some() {
            return;
        }
        let detected = {
            let buf = self.state.buffers.get(bid);
            let paths = buf.file().map(|file| DetectPaths {
                as_opened: file.typed(),
                resolved: file.resolved(),
            });
            detect_language(paths, buf.text().rope(), &self.state.config.languages)
        };
        self.set_buffer_language(bid, detected);
    }

    /// Detect and set the language for every buffer
    /// `buffer::lifecycle::open_buffer_and_notify` queued onto
    /// `state.config.pending_language_detection`. The disjoint-borrow open
    /// chokepoint can't do this inline (see that function's doc). Every pass
    /// of `drain_pending_work` drains it, so a buffer opened by any path is
    /// announced before the next frame; `apply_script_effects` also drains it
    /// at its tail, so later work in the same batch sees the language. With
    /// no scripting host attached (inside `init_scripting`, before it attaches
    /// the host it built) the queue is left for the drain `init_scripting`
    /// runs after attaching: setting a language needs the host.
    ///
    /// Also fires `OnBufferOpen` for each buffer, after its `OnLanguageSet`
    /// (queued by `detect_and_set_language` above); `open_buffer_and_notify`
    /// itself doesn't fire it, since both hooks share the FIFO `pending_work`
    /// queue and plugins registering both handlers expect `on-language-set`
    /// to run first.
    ///
    /// Takes the queue before iterating, not `while let Some(bid) =
    /// queue.pop()`: detecting a language can activate a lazy plugin, whose
    /// body can open more buffers and re-enter this same drain via a nested
    /// `apply_script_effects`, so taking first means that nested call sees (and
    /// drains) only the buffers *it* queued, and returns to find nothing left
    /// for this call to reprocess.
    pub(super) fn detect_pending_languages(&mut self) {
        if self.scripting.is_none() {
            return;
        }
        let pending = std::mem::take(&mut self.state.config.pending_language_detection);
        for bid in pending {
            // The buffer may have been closed by the same batch of work that
            // opened it (e.g. `close-buffer!` in the same eval) before this
            // drain runs, since `close-buffer!` mutates synchronously, unlike this
            // deferred detection. Skip rather than hit `BufferStore::get`'s
            // "unseeded BufferId" panic. Skipping the corresponding
            // `OnBufferClose` for a since-closed, never-opened buffer is
            // `close_buffer_and_notify`'s job, not this drain's; see its doc.
            if self
                .state
                .buffers
                .try_get(bid)
                .is_some_and(|b| b.open_hook_pending)
            {
                // A `SetBufferLanguage` effect for this same bid, applied
                // earlier in this same `apply_script_effects` call (e.g.
                // `(define b (open-buffer! path)) (set-buffer-option! b
                // "language" "notes")` in one eval), already stamped
                // `language_by`, which detection leaves as it is.
                // `OnBufferOpen` still fires either way: the buffer was
                // still opened.
                self.detect_and_set_language(bid);
                self.state.buffers.get_mut(bid).open_hook_pending = false;
                self.state
                    .queue_event(EditorEvent::OnBufferOpen { buffer: bid });
            }
        }
    }

    /// Register languages from a maximal run of consecutive
    /// `Effect::LanguageReg` entries (`Editor::apply_script_effects` groups
    /// them so a large run (e.g. `languages.scm`'s 342 `define-language!`
    /// calls) rebuilds the glob matcher once, not once per entry).
    /// Fail-soft: glob-set build failures are logged as warnings, editor continues.
    pub(super) fn apply_pending_language_regs(
        &mut self,
        regs: Vec<hume_scripting::PendingLanguageReg>,
    ) {
        use hume_scripting::PendingLanguageReg;
        let mut any_identity = false;
        let mut grammar_sweeps: Vec<LanguageId> = Vec::new();
        for reg in regs {
            match reg {
                PendingLanguageReg::Identity {
                    name,
                    identity,
                    origin,
                } => {
                    let (_, outcome) = self
                        .state
                        .config
                        .languages
                        .register_identity_no_rebuild(&name, identity, origin);
                    match outcome {
                        WriteOutcome::Applied => any_identity = true,
                        WriteOutcome::Ignored => self.report(
                            Severity::Warning,
                            format!(
                                "define-language! '{name}': already defined; this definition is ignored"
                            ),
                        ),
                        WriteOutcome::Shadowed => self
                            .state
                            .report_shadowed("define-language!", format_args!("'{name}' definition")),
                    }
                }
                PendingLanguageReg::Configure {
                    name,
                    fields,
                    origin,
                } => {
                    self.configure_language(&name, fields, origin);
                }
                PendingLanguageReg::Grammar(reg) => {
                    let registered = self
                        .state
                        .config
                        .languages
                        .register_grammar(&reg.name, reg.source, reg.origin);
                    match registered {
                        Some(id) => grammar_sweeps.push(id),
                        None => self.state.report_shadowed(
                            "register-grammar!",
                            format_args!("'{}' grammar", reg.name),
                        ),
                    }
                }
            }
        }
        if any_identity && let Err(e) = self.state.config.languages.rebuild_glob_set() {
            self.state.message_log.push(
                Severity::Warning,
                format!("define-language!: glob set build failed: {e}"),
            );
        }
        if !grammar_sweeps.is_empty() {
            self.sweep_buffers_for_grammars(grammar_sweeps);
        }
    }

    /// Apply one `configure-language!` call made by `by`. A name with no
    /// identity is an error. A field already set by the same tier keeps its
    /// value and the ignored write is logged as a warning; a plugin write
    /// under a user value is dropped and traced.
    fn configure_language(&mut self, name: &str, fields: LanguageFields, by: ConfigOrigin) {
        let languages = &mut self.state.config.languages;
        let Some(id) = languages.defined_id(name) else {
            self.report(
                Severity::Error,
                format!("configure-language!: language '{name}' is not defined"),
            );
            return;
        };
        for (field, outcome) in languages.configure_fields(id, fields, by) {
            let key = field.name();
            match outcome {
                WriteOutcome::Applied => {}
                WriteOutcome::Ignored => self.report(
                    Severity::Warning,
                    format!(
                        "configure-language! '{name}': {key} is already configured; this value is ignored"
                    ),
                ),
                WriteOutcome::Shadowed => self
                    .state
                    .report_shadowed("configure-language!", format_args!("'{name}' {key}")),
            }
        }
    }
}
