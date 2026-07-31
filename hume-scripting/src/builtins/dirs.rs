//! Directory state for HUME's scripting layer: raw + display-form data/runtime
//! dirs, and the canonical install-lock root, computed once and shared by
//! every builtin that needs them.
//!
//! Full-trust plugin model (see `user-manual/docs/plugins.md`'s "Filesystem
//! and processes") — this does not enforce a sandbox. What lives here is editor-
//! integration state: the display-form data/runtime dirs (`data-dir`/
//! `runtime-dir`), and the canonical `<data>/servers/` root the cross-process
//! install lock (`acquire-install-lock!`/`release-install-lock!`) needs
//! pre-created and canonicalized at construction time (unlike plugin/grammar
//! install targets, which Steel's own `create-directory!`, or `git clone`'s
//! own recursive parent creation, now create on demand — see
//! `lsp/install-github!`'s `create-directory!` call and the `git clone`
//! behavior it relies on).
//!
//! One [`ScriptDirs`] is built once by [`crate::ScriptingHost::new`] and
//! borrowed into every [`crate::context::SteelCtx`] — no thread-local, no
//! separate init call.

use std::path::{Path, PathBuf};

use steel::rerrs::SteelErr;

// ── Directory state ──────────────────────────────────────────────────────────

pub(crate) struct ScriptDirs {
    /// `$XDG_DATA_HOME/hume/` (or platform equivalent) — where PLUM installs
    /// user/third-party plugins. Raw, uncanonicalized.
    pub(crate) data_dir: Option<PathBuf>,
    /// Where core plugins, themes, and docs live. Raw, uncanonicalized.
    pub(crate) runtime_dir: Option<PathBuf>,
    /// `<data>/hume/` as a *display* (non-UNC) path — what `(data-dir)` returns
    /// to Scheme.  On Windows the canonical form carries a `\\?\` prefix that
    /// the NT object manager does not accept with forward slashes, so we expose
    /// the plain drive-letter form instead (e.g. `C:\Users\…\hume`).
    pub(crate) data_dir_display: Option<PathBuf>,
    /// `<runtime>/` as a display path (same UNC reasoning).
    pub(crate) runtime_dir_display: Option<PathBuf>,
    /// Canonical `<data>/servers/` — the cross-process install lock's root.
    /// `None` when `data_dir` is unavailable or directory creation fails;
    /// lock operations fail closed in that case.
    data_servers: Option<PathBuf>,
}

impl ScriptDirs {
    /// Compute all derived directory state from the raw data/runtime dirs.
    ///
    /// Eagerly creates `<data>/servers/` so the install lock has a canonical
    /// root to work with from the first call. If creation or canonicalization
    /// fails, `data_servers` is `None` and lock operations fail closed rather
    /// than silently permitting writes to a bogus prefix. `new(None, None)`
    /// does no filesystem work at all.
    pub(crate) fn new(data_dir: Option<PathBuf>, runtime_dir: Option<PathBuf>) -> Self {
        // <data>/servers/
        let data_servers = data_dir.as_ref().and_then(|d| {
            let s = d.join("servers");
            hume_platform::fs::create_dir_all(&s).ok()?;
            hume_platform::fs::canonicalize(&s).ok()
        });

        // Canonicalize data_dir for the display form; fall back to raw path
        // when the directory doesn't exist (e.g. sandboxed FS test environments).
        let canonical_data = data_dir
            .clone()
            .map(|d| hume_platform::fs::canonicalize(&d).unwrap_or(d));
        // Display form strips `\\?\` so Scheme can safely concatenate `/`-separated
        // segments on Windows without producing malformed extended-length paths.
        let data_dir_display = canonical_data.map(hume_platform::path::strip_unc_prefix);

        let canonical_runtime = runtime_dir
            .clone()
            .and_then(|rt| hume_platform::fs::canonicalize(&rt).ok());
        let runtime_dir_display = canonical_runtime.map(hume_platform::path::strip_unc_prefix);

        Self {
            data_dir,
            runtime_dir,
            data_dir_display,
            runtime_dir_display,
            data_servers,
        }
    }

    /// The canonical `<data>/servers/` root. Used by `install.rs`'s
    /// cross-process install lock.
    ///
    /// Returns `Err` when no data directory is available (HOME/APPDATA unset),
    /// which fails the lock closed rather than silently permitting it.
    pub(crate) fn servers_dir(&self) -> Result<&Path, SteelErr> {
        match self.data_servers.as_deref() {
            Some(p) => Ok(p),
            None => steel::stop!(Generic =>
                "no data directory — HOME/APPDATA unset; server install operations unavailable"),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
