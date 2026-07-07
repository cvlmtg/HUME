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
    /// Parses and stores the JSON blobs for one registration, keyed by
    /// language. Rejects (loudly, per the hub's OQ default) a second
    /// registration for a language already configured. A blob that fails
    /// to parse rejects the whole registration — fail fast rather than
    /// silently dropping just the bad field.
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

        let init_options = match parse_json_blob(&reg.language, "init-options", reg.init_options, self)
        {
            Ok(v) => v,
            Err(()) => return,
        };
        let settings = match parse_json_blob(&reg.language, "settings", reg.settings, self) {
            Ok(v) => v,
            Err(()) => return,
        };

        self.lsp.configs.insert(
            reg.language,
            LspServerConfig {
                command: reg.command,
                args: reg.args,
                root_markers: reg.root_markers,
                init_options,
                settings,
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
        let key = (language, root.clone());
        if self.lsp.servers_by_key.contains_key(&key) {
            return; // already running (or starting) — C7 handles the didOpen
        }

        match self.lsp.backend.start(&config.command, &config.args, &root) {
            Ok(server_id) => {
                let mut client = hume_lsp::client::LspClient::new(server_id, root);
                client.start_handshake(self.lsp.backend.as_mut());
                self.lsp.clients.insert(server_id, client);
                self.lsp.servers_by_key.insert(key, server_id);
            }
            Err(e) => {
                self.report(
                    Severity::Error,
                    format!("lsp: failed to start '{}': {e}", config.command),
                );
            }
        }
    }
}

/// Parses one optional JSON blob, reporting and returning `Err(())` on a
/// parse failure (the caller aborts the whole registration).
fn parse_json_blob(
    language: &str,
    field: &str,
    raw: Option<String>,
    ed: &mut Editor,
) -> Result<Option<serde_json::Value>, ()> {
    match raw {
        None => Ok(None),
        Some(s) => match serde_json::from_str(&s) {
            Ok(v) => Ok(Some(v)),
            Err(e) => {
                ed.report(
                    Severity::Error,
                    format!("register-lsp-server!: '{language}' {field}: invalid JSON: {e}"),
                );
                Err(())
            }
        },
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
