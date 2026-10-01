//! Directory state for HUME's scripting layer: raw + display-form data/runtime
//! dirs (`data-dir`/`runtime-dir`), computed once and shared by every builtin
//! that needs them. There is no sandbox here; see `user-manual/docs/plugins.md`'s
//! "Filesystem and processes".
//!
//! One [`ScriptDirs`] is built once by [`crate::ScriptingHost::new`] and
//! borrowed into every [`crate::context::SteelCtx`]: no thread-local, no
//! separate init call.

use std::path::PathBuf;

// ── Directory state ──────────────────────────────────────────────────────────

pub(crate) struct ScriptDirs {
    /// `$XDG_DATA_HOME/hume/` (or platform equivalent), where PLUM installs
    /// user/third-party plugins. Raw, uncanonicalized.
    pub(crate) data_dir: Option<PathBuf>,
    /// Where core plugins, themes, and docs live. Raw, uncanonicalized.
    pub(crate) runtime_dir: Option<PathBuf>,
    /// `<data>/hume/` as a *display* (non-UNC) path: what `(data-dir)` returns
    /// to Scheme.  On Windows the canonical form carries a `\\?\` prefix that
    /// the NT object manager does not accept with forward slashes, so we expose
    /// the plain drive-letter form instead (e.g. `C:\Users\…\hume`).
    pub(crate) data_dir_display: Option<PathBuf>,
    /// `<runtime>/` as a display path (same UNC reasoning).
    pub(crate) runtime_dir_display: Option<PathBuf>,
}

impl ScriptDirs {
    /// Compute all derived directory state from the raw data/runtime dirs.
    /// `new(None, None)` does no filesystem work at all.
    pub(crate) fn new(data_dir: Option<PathBuf>, runtime_dir: Option<PathBuf>) -> Self {
        // Canonicalize data_dir for the display form; fall back to raw path
        // when the directory doesn't exist (e.g. sandboxed FS test environments).
        let canonical_data = data_dir
            .clone()
            .map(|d| std::fs::canonicalize(&d).unwrap_or(d));
        // Display form strips `\\?\` so Scheme can safely concatenate `/`-separated
        // segments on Windows without producing malformed extended-length paths.
        let data_dir_display = canonical_data.map(hume_platform::path::strip_unc_prefix);

        let canonical_runtime = runtime_dir
            .clone()
            .and_then(|rt| std::fs::canonicalize(&rt).ok());
        let runtime_dir_display = canonical_runtime.map(hume_platform::path::strip_unc_prefix);

        Self {
            data_dir,
            runtime_dir,
            data_dir_display,
            runtime_dir_display,
        }
    }
}
