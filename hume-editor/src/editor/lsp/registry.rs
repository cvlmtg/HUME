//! Server registration: `register-lsp-server!` config storage, workspace
//! root resolution, and spawn-on-first-open.

use std::path::{Path, PathBuf};

use hume_engine::pipeline::BufferId;

use crate::editor::{Editor, Severity};

/// A `register-lsp-server!`-registered language name — the key
/// `LspState.configs` and every attached `ServerEntry.language` use today.
/// Registration identity is language, one-to-one with a running server (see
/// docs/LSP.md's registry-shape decision row): a future multi-server-per-
/// language design would key `LspState.servers` by a distinct registration
/// name instead, with a `LanguageName -> [registration name]` map alongside
/// it — this alias exists so that future re-key finds every language-keyed
/// signature by type, not by re-reading every `String` in this module.
pub(crate) type LanguageName = String;

/// Config recorded by one `register-lsp-server!` call, keyed by language.
#[derive(Debug, Clone)]
pub(crate) struct LspServerConfig {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) root_markers: Vec<String>,
    /// Sent verbatim as `initializationOptions` in the `initialize` request
    /// (`lsp_attach_buffer`'s spawn branch, via `LspClient::set_init_options`).
    pub(crate) init_options: Option<serde_json::Value>,
    /// Pushed as `workspace/didChangeConfiguration` after `initialized`
    /// (`lsp_attach_buffer`'s spawn branch, via `LspClient::set_settings`),
    /// and resolved per-item to answer `workspace/configuration` pull
    /// requests (`Editor::dispatch_lsp_action`'s `ServerRequest` arm).
    pub(crate) settings: Option<serde_json::Value>,
}

/// Walks up from `file`'s directory to the first ancestor containing any of
/// `markers` (a file or a directory — `.git` included); falls back to `cwd`
/// if none match. `Path::ancestors()` yields `file`'s parent first, then
/// each successively shorter prefix up to (and including) the filesystem
/// root, so the nearest marker wins.
pub(crate) fn resolve_root(file: &Path, markers: &[String], cwd: &Path) -> PathBuf {
    let start = file.parent().unwrap_or(cwd);
    for dir in start.ancestors() {
        if markers.iter().any(|m| dir.join(m).exists()) {
            return dir.to_path_buf();
        }
    }
    cwd.to_path_buf()
}

impl Editor {
    /// Applies one queued op. Called by `Editor::apply_script_effects` for
    /// each `Effect::LspServerOp`, in emission order — the one apply path
    /// for LSP server registration/unregistration regardless of which eval
    /// queued it.
    pub(in crate::editor) fn apply_lsp_server_op(
        &mut self,
        op: hume_scripting::PendingLspServerOp,
    ) {
        match op {
            hume_scripting::PendingLspServerOp::Register(reg) => {
                self.apply_pending_lsp_server_reg(reg);
            }
            hume_scripting::PendingLspServerOp::Unregister { language } => {
                // Idempotent by construction: removing an absent key and
                // stopping a language with no running clients are both
                // no-ops — `:lsp-uninstall` of an orphan or never-spawned
                // server must succeed silently.
                self.lsp.configs.remove(&language);
                self.lsp_stop(Some(&language));
            }
            hume_scripting::PendingLspServerOp::Stop { language } => {
                let n = self.lsp_stop(language.as_deref());
                if n == 0 {
                    self.report(
                        Severity::Info,
                        "lsp: no matching server to stop".to_string(),
                    );
                } else {
                    self.report(Severity::Info, format!("lsp: stopped {n} server(s)"));
                }
            }
            hume_scripting::PendingLspServerOp::Restart { language } => {
                let n = self.lsp_restart(language.as_deref());
                if n == 0 {
                    self.report(
                        Severity::Info,
                        "lsp: no matching server to restart".to_string(),
                    );
                } else {
                    self.report(Severity::Info, format!("lsp: restarted {n} server(s)"));
                }
            }
            hume_scripting::PendingLspServerOp::ShowStatus => {
                let content = self.lsp_status_text();
                self.open_read_only_view("[lsp-status]", &content, 0);
            }
        }
    }

    /// Last-wins insert: replaces any existing registration for the same
    /// language (matching `define-language!`'s semantics) rather than
    /// rejecting the second call. Deliberate, not a missing guard: a user's
    /// `init.scm` loads after every plugin's own registration, and its
    /// `register-lsp-server!` call for a language a plugin already
    /// registered must override that plugin's default — a hard error here
    /// would make user config unable to win over plugin defaults at all.
    /// See `docs/LSP.md`'s "Multiple servers per language" decision row.
    /// Running clients on the *old* config are left alone until their next
    /// spawn — a caller that needs a fresh spawn right away (e.g.
    /// reinstalling a server) unregisters explicitly first, which this does
    /// not do on its own.
    ///
    /// After inserting, sweeps already-open buffers of this language that
    /// aren't yet attached (`lsp_attach_buffer` is idempotent), so
    /// registration always implies "this language's open buffers get an
    /// LSP client" — callers never need a separate attach step.
    ///
    /// `init_options`/`settings` arrive already decoded to JSON by
    /// `hume_scripting::json::steel_to_json` at the Steel boundary.
    fn apply_pending_lsp_server_reg(&mut self, reg: hume_scripting::PendingLspServerReg) {
        let replaced = self.lsp.configs.contains_key(&reg.language);
        let language = reg.language.clone();

        self.lsp.configs.insert(
            reg.language,
            LspServerConfig {
                command: reg.command,
                args: reg.args,
                root_markers: reg.root_markers,
                init_options: reg.init_options,
                settings: reg.settings,
            },
        );

        if replaced {
            self.report(
                Severity::Trace,
                format!("register-lsp-server!: replaced registration for '{language}'"),
            );
        }

        let bids: Vec<BufferId> = self
            .state
            .buffers
            .iter()
            .filter(|(_, buf)| {
                buf.language
                    .is_some_and(|id| self.state.config.languages.name_of(id) == language)
                    && buf.lsp_server.is_none()
            })
            .map(|(bid, _)| bid)
            .collect();
        for bid in bids {
            self.lsp_attach_buffer(bid);
        }
    }

    /// Attaches buffer `bid` to its language's registered server, spawning
    /// it if this is the first buffer under that (language, root) pair.
    /// Idempotent: safe to call from both the open path and
    /// `set_buffer_language` for the same open (detection fires both).
    ///
    /// No-op when the buffer has no path (unnamed buffers never attach) or
    /// no language, or no server is registered for that language.
    pub(in crate::editor) fn lsp_attach_buffer(&mut self, bid: BufferId) {
        let buf = self.state.buffers.get(bid);
        if buf.lsp_server.is_some() {
            return; // already attached — idempotent re-entry
        }
        let Some(path) = buf.path().map(Path::to_path_buf) else {
            return;
        };
        let Some(lang_id) = buf.language else {
            return;
        };
        let language = self.state.config.languages.name_of(lang_id).to_owned();
        let Some(config) = self.lsp.configs.get(&language).cloned() else {
            return;
        };

        let root = resolve_root(&path, &config.root_markers, &self.state.cwd);

        // Scan for an existing *viable* server under this (language, root)
        // pair — `LspState.servers` is the single source of truth, so
        // there's no separate index that could disagree with it. A Crashed
        // entry is excluded: nothing removes it from `servers` on its own
        // (only `:lsp-stop`/`:lsp-restart` do), so without this check every
        // buffer opened after a crash would silently attach to the corpse.
        let existing = self.lsp.servers.iter().find_map(|(&sid, entry)| {
            (entry.language.as_deref() == Some(language.as_str())
                && entry.client.root() == root.as_path()
                && entry.client.state() != hume_lsp::client::ServerState::Crashed)
                .then_some(sid)
        });

        let server_id = if let Some(existing) = existing {
            existing
        } else if self.lsp.servers.values().any(|entry| {
            entry.language.as_deref() == Some(language.as_str())
                && entry.client.root() == root.as_path()
        }) {
            // The only match for this (language, root) is Crashed — refuse
            // to silently attach to it; the buffer stays unattached until
            // an explicit `:lsp-restart`, which re-attaches every buffer
            // that was on the stopped server through this same path.
            self.report(
                Severity::Error,
                format!("lsp: {language} server crashed — :lsp-restart {language}"),
            );
            return;
        } else {
            match self.lsp.backend.start(&config.command, &config.args, &root) {
                Ok(server_id) => {
                    let mut client = hume_lsp::client::LspClient::new(server_id, root);
                    client.set_init_options(config.init_options.clone());
                    client.set_settings(config.settings.clone());
                    client.start_handshake(self.lsp.backend.as_mut());
                    self.lsp.servers.insert(
                        server_id,
                        super::ServerEntry {
                            client,
                            language: Some(language.clone()),
                            name: config.command.clone(),
                            capabilities_json: None,
                            progress: Vec::new(),
                        },
                    );
                    server_id
                }
                Err(e) => {
                    self.report(
                        Severity::Error,
                        format!("lsp: failed to start '{}': {e}", config.command),
                    );
                    return;
                }
            }
        };

        self.state.buffers.get_mut(bid).lsp_server = Some(server_id);
        self.lsp_did_open(bid);
        // A brand-new server is still Starting — its BecameRunning arm
        // fires the attach hook for every buffer attached by then,
        // including this one. Only fire here for the "attach to an
        // already-Running server" case (second+ buffer under the same key).
        if self
            .lsp
            .servers
            .get(&server_id)
            .is_some_and(|e| e.client.state() == hume_lsp::client::ServerState::Running)
        {
            self.fire_hook_lsp_attach(bid, &language);
        }
    }

    /// Resolves `:lsp-stop [language]` / `:lsp-restart [language]`'s target
    /// set: every server whose key's language matches, or — with no
    /// argument — just the focused buffer's server (if any).
    fn lsp_targets(&self, language: Option<&str>) -> Vec<hume_lsp::backend::ServerId> {
        match language {
            Some(lang) => self
                .lsp
                .servers
                .iter()
                .filter(|(_, e)| e.language.as_deref() == Some(lang))
                .map(|(&id, _)| id)
                .collect(),
            None => self
                .state
                .buffers
                .get(self.focused_buffer_id())
                .lsp_server
                .into_iter()
                .collect(),
        }
    }

    /// Graceful shutdown + full deregistration of one running server:
    /// `begin_shutdown` (shutdown request, then exit — `ServerHandle::drop`
    /// reaps the process regardless), drop its `ServerEntry` and diagnostics,
    /// and clear `lsp_server` on every buffer that pointed at it so a later
    /// attach attempt (open or restart) doesn't see it as already attached.
    /// Every request still in flight on this client is dispatched as
    /// `TimedOut` before the client itself is dropped — otherwise a
    /// registered callback (and its `CallbackEntry`) would be orphaned
    /// along with the removed client, never firing and never freed. Fires
    /// `OnDiagnosticsChanged` for every buffer whose stored diagnostics
    /// were actually cleared, and `OnLspDetach` for every buffer that was
    /// attached — the latter is a plugin's only signal to drop its own
    /// buffer-scoped state derived from this server (e.g. inlay hints),
    /// which nothing here owns well enough to clear on its behalf.
    fn lsp_stop_one(&mut self, server_id: hume_lsp::backend::ServerId) {
        let mut language = String::new();
        if let Some(entry) = self.lsp.servers.remove(&server_id) {
            let super::ServerEntry {
                mut client,
                language: entry_language,
                ..
            } = entry;
            language = entry_language.unwrap_or_default();
            client.begin_shutdown(self.lsp.backend.as_mut());
            for (id, meta) in client.drain_pending() {
                self.dispatch_completed(server_id, id, meta, hume_lsp::client::Outcome::TimedOut);
            }
        }
        self.lsp.backend.shutdown(server_id);
        // Belt over `dispatch_completed`'s cleanup above (via `drain_pending`):
        // covers a request whose response arrived but was never drained
        // before the client was dropped, so no id ever leaks past its server.
        self.lsp.supersede.retain(|(sid, _), _| *sid != server_id);
        let diag_touched = self.lsp.diagnostics.remove_server(server_id);

        let bids: Vec<BufferId> = self
            .state
            .buffers
            .iter()
            .filter(|(_, buf)| buf.lsp_server == Some(server_id))
            .map(|(bid, _)| bid)
            .collect();
        for &bid in &bids {
            let buf = self.state.buffers.get_mut(bid);
            buf.lsp_server = None;
            // Any edits queued for the now-detached server must not survive
            // to a future attach — flushed against a new server's didOpen
            // baseline, they'd desync its document state immediately.
            buf.lsp_pending.clear();
        }
        // An open completion session's items are a snapshot already fetched
        // from the server, not a live subscription — but leaving it open
        // would keep showing (and let the user accept) suggestions from a
        // server that's no longer running for this buffer.
        if self
            .lsp
            .completion
            .as_ref()
            .is_some_and(|session| bids.contains(&session.bid()))
        {
            self.clear_completion_menu();
        }
        for bid in diag_touched {
            self.fire_hook_diagnostics_changed(bid);
        }
        for bid in bids {
            self.fire_hook_lsp_detach(bid, &language);
        }
    }

    /// `:lsp-stop [language]`. Returns the number of servers stopped.
    pub(in crate::editor) fn lsp_stop(&mut self, language: Option<&str>) -> usize {
        let targets = self.lsp_targets(language);
        let count = targets.len();
        for server_id in targets {
            self.lsp_stop_one(server_id);
        }
        count
    }

    /// `:lsp-restart [language]`. Stops each target server, then re-attaches
    /// every buffer that was on it through `lsp_attach_buffer` — the exact
    /// registration spawn path, not a duplicate. Returns the number of servers
    /// restarted.
    pub(in crate::editor) fn lsp_restart(&mut self, language: Option<&str>) -> usize {
        let targets = self.lsp_targets(language);
        let count = targets.len();
        for server_id in targets {
            let bids: Vec<BufferId> = self
                .state
                .buffers
                .iter()
                .filter(|(_, buf)| buf.lsp_server == Some(server_id))
                .map(|(bid, _)| bid)
                .collect();
            self.lsp_stop_one(server_id);
            for bid in bids {
                self.lsp_attach_buffer(bid);
            }
        }
        count
    }
}

#[cfg(test)]
mod tests;
