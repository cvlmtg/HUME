//! Editor-side glue for the language registry: wiring buffer language changes
//! to hooks, lazy-plugin activation, and tree-sitter syntax setup.
//!
//! The registry itself, grammar attachment, and language detection live in
//! `hume_treesitter::registry` — this module only owns what's specific to a
//! live `Editor` (message log, hooks, plugin activation).

mod parse;

use hume_engine::pipeline::BufferId;
use hume_treesitter::registry::{LanguageId, detect_language};

use super::Editor;
use super::event::EditorEvent;

impl Editor {
    /// Set the language identity for buffer `bid`, via plain detection —
    /// does not mark `language_explicit` (see `set_buffer_language_explicit`
    /// for the user/script write paths).
    pub(super) fn set_buffer_language(&mut self, bid: BufferId, new_lang: Option<LanguageId>) {
        self.set_buffer_language_impl(bid, new_lang, false);
    }

    /// Set the language identity for buffer `bid` from a user or script
    /// assertion (`:set buffer language=`, Steel's `set-buffer-language!`)
    /// rather than detection — stamps `language_explicit` so
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

    /// No-op when the value is unchanged (avoids spurious hook fires) — but
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
        let lang_name = new_lang.map(|id| self.state.config.languages.name_of(id).to_owned());
        self.state.buffers.get_mut(bid).language = new_lang;
        // Activate language-matched plugins after the write so handlers are
        // registered in time for the OnLanguageSet fire below.
        if let Some(name) = lang_name.as_deref() {
            self.activate_lazy_language_plugins(name);
            // A lazy plugin's own body can call `set-buffer-language!` on
            // this same buffer (applied inline via `apply_script_effects`
            // before `activate_lazy_language_plugins` returns) — that nested
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
                &self.state.config.languages,
            )
        };
        self.set_buffer_language(bid, detected);
    }

    /// Detect and set the language for every buffer
    /// `buffer::lifecycle::open_buffer_and_notify` queued onto
    /// `state.config.pending_language_detection` — the disjoint-borrow open
    /// chokepoint can't do this inline (see that function's doc), so every
    /// caller that regains a full `&mut Editor` drains this once: directly
    /// after opening (`Editor::open_buffer`, `apply_edit_request_response`),
    /// or at the tail of `apply_script_effects` for every Steel-eval path.
    ///
    /// Also fires `OnBufferOpen` for each buffer, after its `OnLanguageSet`
    /// (queued by `detect_and_set_language` above) — `open_buffer_and_notify`
    /// itself doesn't fire it, since both hooks share the FIFO `pending_work`
    /// queue and plugins registering both handlers expect `on-language-set`
    /// to run first.
    ///
    /// Takes the queue before iterating, not `while let Some(bid) =
    /// queue.pop()`: detecting a language can activate a lazy plugin, whose
    /// body can open more buffers and re-enter this same drain via a nested
    /// `apply_script_effects` — taking first means that nested call sees (and
    /// drains) only the buffers *it* queued, and returns to find nothing left
    /// for this call to reprocess.
    pub(super) fn detect_pending_languages(&mut self) {
        let pending = std::mem::take(&mut self.state.config.pending_language_detection);
        for bid in pending {
            // The buffer may have been closed by the same batch of work that
            // opened it (e.g. `close-buffer!` in the same eval) before this
            // drain runs — `close-buffer!` mutates synchronously, unlike this
            // deferred detection. Skip rather than hit `BufferStore::get`'s
            // "unseeded BufferId" panic.
            //
            // `open_hook_pending` additionally covers the case where `bid`
            // survives (`try_get` succeeds) but is no longer the buffer that
            // was opened: `close_buffer`'s last-buffer branch reuses `bid`'s
            // slot in place for a fresh scratch buffer, which defaults the
            // flag to `false` — so this still correctly skips rather than
            // detecting a language for (and firing `OnBufferOpen` on behalf
            // of) that unrelated scratch buffer. Skipping the corresponding
            // `OnBufferClose` for a since-closed, never-opened buffer is
            // `close_buffer_and_notify`'s job, not this drain's — see its doc.
            if self
                .state
                .buffers
                .try_get(bid)
                .is_some_and(|b| b.open_hook_pending)
            {
                // A `SetBufferLanguage` effect for this same bid, applied
                // earlier in this same `apply_script_effects` call (e.g.
                // `(define b (open-buffer! path)) (set-buffer-language! b
                // "notes")` in one eval), already stamped `language_explicit`
                // — detection must not clobber it with whatever plain
                // detection finds for the path, the same reasoning as
                // `init_scripting`'s post-reload sweep. `OnBufferOpen` still
                // fires either way: the buffer was genuinely opened.
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
    /// them so a large run — e.g. `languages.scm`'s ~700 `define-language!`
    /// calls — rebuilds the glob matcher once, not once per entry).
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
                    lsp_language_id,
                } => {
                    let exts: Vec<&str> = extensions.iter().map(String::as_str).collect();
                    let shebangs_ref: Vec<&str> = shebangs.iter().map(String::as_str).collect();
                    let mut valid_globs: Vec<globset::Glob> = Vec::with_capacity(globs.len());
                    for g in &globs {
                        match globset::Glob::new(g) {
                            Ok(glob) => valid_globs.push(glob),
                            Err(e) => self.state.message_log.push(
                                super::Severity::Warning,
                                format!("define-language! '{}': invalid glob '{}': {}", name, g, e),
                            ),
                        }
                    }
                    self.state.config.languages.register_identity_no_rebuild(
                        &name,
                        &exts,
                        &valid_globs,
                        &shebangs_ref,
                        lsp_language_id.as_deref(),
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
                    match self.state.config.languages.attach_grammar(
                        &name,
                        &grammar_path,
                        &symbol,
                        &highlights_path,
                        injections_path.as_deref(),
                        &mut self.view.registry,
                    ) {
                        Ok(_) => grammar_sweeps.push(
                            self.state
                                .config
                                .languages
                                .id_of(&name)
                                .expect("attach_grammar interns the name"),
                        ),
                        Err(e) => self.state.message_log.push(
                            super::Severity::Warning,
                            format!("register-grammar! '{}': {}", name, e),
                        ),
                    }
                }
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
}
