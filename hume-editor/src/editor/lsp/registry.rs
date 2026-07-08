//! Server registration: `register-lsp-server!` config storage, workspace
//! root resolution, and spawn-on-first-open.

use std::path::{Path, PathBuf};

use hume_engine::pipeline::BufferId;

use crate::editor::{Editor, Severity};

/// Config recorded by one `register-lsp-server!` call, keyed by language.
#[derive(Debug, Clone)]
pub(crate) struct LspServerConfig {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) root_markers: Vec<String>,
    /// Sent verbatim in the `initialize` request once C5's handshake gains
    /// an init-options parameter.
    #[allow(dead_code)]
    pub(crate) init_options: Option<serde_json::Value>,
    /// Answered verbatim to `workspace/configuration` requests and sent as
    /// `didChangeConfiguration` after `initialized` (hub OQ default) — needs
    /// a server-id -> language lookup in the C6 dispatch table that doesn't
    /// exist yet; wired when a feature first needs it.
    #[allow(dead_code)]
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
    /// Stores one registration, keyed by language. Rejects (loudly, per the
    /// hub's OQ default) a second registration for a language already
    /// configured. `init_options`/`settings` arrive already decoded to JSON
    /// by `hume_scripting::json::steel_to_json` at the Steel boundary.
    fn apply_pending_lsp_server_reg(&mut self, reg: hume_scripting::PendingLspServerReg) {
        if self.lsp.configs.contains_key(&reg.language) {
            self.report(
                Severity::Error,
                format!(
                    "register-lsp-server!: '{}' is already registered — ignoring duplicate",
                    reg.language
                ),
            );
            return;
        }

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
    }

    /// Drain `host.pending_lsp_server_regs` and apply them. Mirrors
    /// `flush_pending_language_regs` (C8's Rust-side twin).
    pub(in crate::editor) fn flush_pending_lsp_server_regs(&mut self, host: &mut hume_scripting::ScriptingHost) {
        let regs = host.take_pending_lsp_server_regs();
        for reg in regs {
            self.apply_pending_lsp_server_reg(reg);
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
        let Some(language) = buf.language.clone() else {
            return;
        };
        let Some(config) = self.lsp.configs.get(&language).cloned() else {
            return;
        };

        let root = resolve_root(&path, &config.root_markers, &self.state.cwd);
        // Cloned before `key` moves `language` in — needed below regardless
        // of which branch actually runs (the existing-server branch never
        // touches `key` again, but the new-server branch moves it into
        // `servers_by_key`).
        let language_for_hook = language.clone();
        let key = (language, root.clone());

        let server_id = if let Some(&existing) = self.lsp.servers_by_key.get(&key) {
            existing
        } else {
            match self.lsp.backend.start(&config.command, &config.args, &root) {
                Ok(server_id) => {
                    let mut client = hume_lsp::client::LspClient::new(server_id, root);
                    client.start_handshake(self.lsp.backend.as_mut());
                    self.lsp.clients.insert(server_id, client);
                    self.lsp.servers_by_key.insert(key, server_id);
                    self.lsp
                        .server_names
                        .insert(server_id, config.command.clone());
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
            .clients
            .get(&server_id)
            .is_some_and(|c| c.state == hume_lsp::client::ServerState::Running)
        {
            self.fire_hook_lsp_attach(bid, &language_for_hook);
        }
    }

    /// Resolves `:lsp-stop [language]` / `:lsp-restart [language]`'s target
    /// set: every server whose key's language matches, or — with no
    /// argument — just the focused buffer's server (if any).
    fn lsp_targets(&self, language: Option<&str>) -> Vec<(String, PathBuf, hume_lsp::backend::ServerId)> {
        match language {
            Some(lang) => self
                .lsp
                .servers_by_key
                .iter()
                .filter(|((l, _), _)| l == lang)
                .map(|((l, r), &id)| (l.clone(), r.clone(), id))
                .collect(),
            None => {
                let bid = self.focused_buffer_id();
                let Some(server_id) = self.state.buffers.get(bid).lsp_server else {
                    return Vec::new();
                };
                self.lsp
                    .servers_by_key
                    .iter()
                    .find(|&(_, &id)| id == server_id)
                    .map(|((l, r), &id)| vec![(l.clone(), r.clone(), id)])
                    .unwrap_or_default()
            }
        }
    }

    /// Graceful shutdown + full deregistration of one running server:
    /// `begin_shutdown` (shutdown request, then exit — `ServerHandle::drop`
    /// reaps the process regardless), drop the client/key/name/diagnostics
    /// entries, and clear `lsp_server` on every buffer that pointed at it so
    /// a later attach attempt (open or restart) doesn't see it as already
    /// attached. Fires `OnDiagnosticsChanged` for every buffer whose stored
    /// diagnostics were actually cleared, and `OnLspDetach` for every buffer
    /// that was attached — the latter is a plugin's only signal to drop its
    /// own buffer-scoped state derived from this server (e.g. inlay hints),
    /// which nothing here owns well enough to clear on its behalf.
    fn lsp_stop_one(&mut self, language: &str, root: &Path, server_id: hume_lsp::backend::ServerId) {
        if let Some(mut client) = self.lsp.clients.remove(&server_id) {
            client.begin_shutdown(self.lsp.backend.as_mut());
        }
        self.lsp.backend.shutdown(server_id);
        self.lsp
            .servers_by_key
            .remove(&(language.to_string(), root.to_path_buf()));
        self.lsp.server_names.remove(&server_id);
        self.lsp.capabilities_json.remove(&server_id);
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
            .state
            .lsp_completion
            .as_ref()
            .is_some_and(|session| bids.contains(&session.bid()))
        {
            self.state.clear_lsp_completion();
        }
        for bid in diag_touched {
            self.fire_hook_diagnostics_changed(bid);
        }
        for bid in bids {
            self.fire_hook_lsp_detach(bid, language);
        }
    }

    /// `:lsp-stop [language]`. Returns the number of servers stopped.
    pub(in crate::editor) fn lsp_stop(&mut self, language: Option<&str>) -> usize {
        let targets = self.lsp_targets(language);
        let count = targets.len();
        for (lang, root, server_id) in targets {
            self.lsp_stop_one(&lang, &root, server_id);
        }
        count
    }

    /// `:lsp-restart [language]`. Stops each target server, then re-attaches
    /// every buffer that was on it through `lsp_attach_buffer` — the exact
    /// C8 spawn path, not a duplicate. Returns the number of servers
    /// restarted.
    pub(in crate::editor) fn lsp_restart(&mut self, language: Option<&str>) -> usize {
        let targets = self.lsp_targets(language);
        let count = targets.len();
        for (lang, root, server_id) in targets {
            let bids: Vec<BufferId> = self
                .state
                .buffers
                .iter()
                .filter(|(_, buf)| buf.lsp_server == Some(server_id))
                .map(|(bid, _)| bid)
                .collect();
            self.lsp_stop_one(&lang, &root, server_id);
            for bid in bids {
                self.lsp_attach_buffer(bid);
            }
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        std::fs::write(path, b"").unwrap();
    }

    #[test]
    fn marker_in_immediate_parent_wins() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("Cargo.toml"));
        let file = tmp.path().join("src/main.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        touch(&file);

        let root = resolve_root(
            &file,
            &["Cargo.toml".to_string()],
            &PathBuf::from("/should-not-be-used"),
        );
        assert_eq!(root, tmp.path());
    }

    #[test]
    fn marker_in_grandparent_is_found_by_walking_up() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("Cargo.toml"));
        let file = tmp.path().join("src/nested/deep.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        touch(&file);

        let root = resolve_root(&file, &["Cargo.toml".to_string()], &PathBuf::from("/cwd"));
        assert_eq!(root, tmp.path());
    }

    #[test]
    fn no_marker_anywhere_falls_back_to_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("src/main.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        touch(&file);
        let cwd = tmp.path().join("elsewhere");
        std::fs::create_dir_all(&cwd).unwrap();

        let root = resolve_root(&file, &["Cargo.toml".to_string()], &cwd);
        assert_eq!(root, cwd);
    }

    #[test]
    fn directory_marker_like_dot_git_is_matched() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        let file = tmp.path().join("src/main.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        touch(&file);

        let root = resolve_root(&file, &[".git".to_string()], &PathBuf::from("/cwd"));
        assert_eq!(root, tmp.path());
    }
}
