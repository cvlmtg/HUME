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

use hume_engine::pipeline::BufferId;
use hume_scripting::ConfigOrigin;
use hume_treesitter::registry::{
    ConfigureOutcome, ConfiguredBy, DetectPaths, LanguageFields, LanguageId, detect_language,
};

use super::Editor;
use super::event::EditorEvent;
use super::host_impl::EditorHostImpl;

impl Editor {
    /// Set the language identity for buffer `bid`, via plain detection:
    /// does not mark `language_explicit` (see `set_buffer_language_explicit`
    /// for the user/script write paths).
    pub(super) fn set_buffer_language(&mut self, bid: BufferId, new_lang: Option<LanguageId>) {
        self.set_buffer_language_impl(bid, new_lang, false);
    }

    /// A user or script `language` option value: `None` clears the language,
    /// and a name no `define-language!` registered is reported but still
    /// applied (a grammar or plugin may register it later). The one parser
    /// `:set buffer language=` and `(set-buffer-option! pane "language" …)`
    /// share.
    pub(super) fn set_buffer_language_by_name(&mut self, bid: BufferId, name: Option<&str>) {
        let new_lang = name.map(|name| {
            if self.state.config.languages.defined_id(name).is_none() {
                self.report(
                    super::Severity::Info,
                    format!("language '{name}' is not registered"),
                );
            }
            self.state.config.languages.intern(name)
        });
        self.set_buffer_language_explicit(bid, new_lang);
    }

    /// Set the language identity for buffer `bid` from a user or script
    /// assertion (`:set buffer language=`, `set-buffer-option!`'s `"language"`)
    /// rather than detection. Stamps `language_explicit` so
    /// `:reload-config`'s reset can restore the assertion across the reload
    /// instead of letting its post-reload re-detect sweep silently pick
    /// something else (see `clear_languages_all`).
    pub(super) fn set_buffer_language_explicit(
        &mut self,
        bid: BufferId,
        new_lang: Option<LanguageId>,
    ) {
        self.set_buffer_language_impl(bid, new_lang, true);
    }

    /// No-op when the value is unchanged (avoids spurious hook fires), but
    /// `language_explicit` is still stamped either way, since it records how
    /// the *current* value arrived, not whether this call changed it.
    /// On change: writes `Buffer.language`, fires `OnLanguageSet` with `(bid, name-or-#f)`.
    fn set_buffer_language_impl(
        &mut self,
        bid: BufferId,
        new_lang: Option<LanguageId>,
        explicit: bool,
    ) {
        self.state.buffers.get_mut(bid).language_explicit = explicit;
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
                self.report(super::Severity::Error, format!("language data: {e}"));
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
        let detected = {
            let buf = self.state.buffers.get(bid);
            let paths = buf.path().map(|resolved| DetectPaths {
                as_opened: buf
                    .typed_path()
                    .expect("a buffer with a path has a typed path (Buffer::set_path)"),
                resolved,
            });
            let first_line = buf.first_line();
            detect_language(paths, first_line.as_deref(), &self.state.config.languages)
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
                // `language_explicit`, so detection must not clobber it with whatever plain
                // detection finds for the path, the same reasoning as
                // `init_scripting`'s post-reload sweep. `OnBufferOpen` still
                // fires either way: the buffer was still opened.
                if !self.state.buffers.get(bid).language_explicit {
                    self.detect_and_set_language(bid);
                }
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
                    extensions,
                    globs,
                    shebangs,
                } => {
                    let shebangs_ref: Vec<&str> = shebangs.iter().map(String::as_str).collect();
                    self.state.config.languages.register_identity_no_rebuild(
                        &name,
                        &extensions,
                        &globs,
                        &shebangs_ref,
                    );
                    any_identity = true;
                }
                PendingLanguageReg::Configure {
                    name,
                    fields,
                    origin,
                } => {
                    let by = match origin {
                        ConfigOrigin::User => ConfiguredBy::User,
                        ConfigOrigin::Plugin => ConfiguredBy::Plugin,
                    };
                    self.configure_language(&name, fields, by);
                }
                PendingLanguageReg::Grammar(reg) => grammar_sweeps.push(
                    self.state
                        .config
                        .languages
                        .register_grammar(&reg.name, reg.source),
                ),
            }
        }
        if any_identity && let Err(e) = self.state.config.languages.rebuild_glob_set() {
            self.state.message_log.push(
                super::Severity::Warning,
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
    /// under a user value is dropped quietly.
    fn configure_language(&mut self, name: &str, fields: LanguageFields, by: ConfiguredBy) {
        let languages = &mut self.state.config.languages;
        let Some(id) = languages.defined_id(name) else {
            self.report(
                super::Severity::Error,
                format!("configure-language!: language '{name}' is not defined"),
            );
            return;
        };
        let ignored: Vec<&str> = languages
            .configure_fields(id, fields, by)
            .into_iter()
            .filter(|&(_, outcome)| outcome == ConfigureOutcome::Ignored)
            .map(|(field, _)| field.key())
            .collect();
        for field in ignored {
            self.report(
                super::Severity::Warning,
                format!(
                    "configure-language! '{name}': {field} is already configured; this value is ignored"
                ),
            );
        }
    }
}
